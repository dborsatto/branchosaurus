// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore
import SwiftUI

/// What a diff pane is showing.
enum DiffState: Equatable {
    case idle
    case loading
    case loaded(FileDiff)
    case failed(String)
}

/// Hunk and line actions a diff offers: none for history, stage/discard for unstaged changes,
/// unstage for staged ones.
struct DiffActions {
    enum Mode { case unstaged, staged }

    let mode: Mode
    let perform: @MainActor (LineAction, LineSelection) -> Void
}

/// Shows a file diff, or explains why there is none.
struct DiffPane: View {
    let state: DiffState
    var actions: DiffActions?
    /// The changed lines selected in the diff, when it has actions.
    var selectedLines: Binding<LineSelection>?

    var body: some View {
        switch state {
        case .idle:
            ContentUnavailableView("No File Selected", systemImage: "doc.text")
        case .loading:
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case let .failed(message):
            ContentUnavailableView {
                Label("Can’t Show Diff", systemImage: "exclamationmark.triangle")
            } description: {
                Text(message)
            }
        case let .loaded(diff):
            switch diff.content {
            case let .text(hunks, _, _) where hunks.isEmpty:
                ContentUnavailableView(
                    "No Content Changes", systemImage: "equal.circle",
                    description: Text(modeChange(diff) ?? "Only the file’s name or location changed."))
            case .text:
                DiffTableView(
                    content: DiffRows(diff: diff), actions: actions,
                    onSelectLines: { selectedLines?.wrappedValue = $0 })
            case let .binary(oldSize, newSize):
                ContentUnavailableView(
                    "Binary File", systemImage: "doc.zipper",
                    description: Text(sizeChange(oldSize, newSize)))
            case let .tooLarge(oldSize, newSize):
                ContentUnavailableView(
                    "File Too Large to Diff", systemImage: "doc.badge.ellipsis",
                    description: Text(sizeChange(oldSize, newSize)))
            }
        }
    }

    private func modeChange(_ diff: FileDiff) -> String? {
        guard let old = diff.oldMode, let new = diff.newMode, old != new else { return nil }
        return "Mode changed from \(String(old, radix: 8)) to \(String(new, radix: 8))."
    }

    private func sizeChange(_ old: UInt64?, _ new: UInt64?) -> String {
        let format = { (size: UInt64) in
            ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file)
        }
        switch (old, new) {
        case let (old?, new?): return "\(format(old)) → \(format(new))"
        case let (nil, new?): return "Added, \(format(new))"
        case let (old?, nil): return "Deleted, \(format(old))"
        case (nil, nil): return ""
        }
    }
}

/// One visible row of a diff: a hunk header or a line.
enum DiffRow {
    case hunk(index: Int, header: String)
    /// `highlights` are UTF-16 ranges of `text`.
    case line(
        kind: LineKind, old: UInt32?, new: UInt32?, text: String, highlights: [Highlight],
        missingNewline: Bool)

    /// The widest the text can be, in columns: tabs count as a full tab stop.
    var textLength: Int {
        switch self {
        case let .hunk(_, header): header.count
        case let .line(_, _, _, text, _, _):
            text.count + text.utf8.reduce(0) { $1 == 0x09 ? $0 + DiffLineView.tabWidth - 1 : $0 }
        }
    }

    var isChange: Bool {
        if case let .line(kind, _, _, _, _, _) = self { return kind != .context }
        return false
    }
}

/// A diff flattened into table rows, with each hunk's changed lines for hunk actions.
struct DiffRows {
    /// Longest line shown in full, in UTF-16 units; longer ones (minified files) are cut for
    /// drawing speed.
    static let maxDisplayedLength = 2_000

    let file: Data
    let rows: [DiffRow]
    let hunkSelections: [LineSelection]

    init(diff: FileDiff) {
        file = diff.path.bytes
        guard case let .text(hunks, _, _) = diff.content else {
            rows = []
            hunkSelections = []
            return
        }
        var rows: [DiffRow] = []
        rows.reserveCapacity(hunks.reduce(0) { $0 + $1.lines.count + 1 })
        var selections: [LineSelection] = []
        for (index, hunk) in hunks.enumerated() {
            rows.append(.hunk(index: index, header: hunk.header))
            var selection = LineSelection(removed: [], added: [])
            for line in hunk.lines {
                switch line.kind {
                case .removed: line.oldLine.map { selection.removed.append($0) }
                case .added: line.newLine.map { selection.added.append($0) }
                case .context: break
                }
                // Tabs stay in the text, so highlight ranges stay valid; they're drawn at tab stops.
                var text = line.text
                let utf16 = text as NSString
                if utf16.length > Self.maxDisplayedLength {
                    let cut = utf16.rangeOfComposedCharacterSequence(at: Self.maxDisplayedLength)
                    text = utf16.substring(to: cut.location) + "…"
                }
                rows.append(
                    .line(
                        kind: line.kind, old: line.oldLine, new: line.newLine, text: text,
                        highlights: line.highlights, missingNewline: line.missingNewline))
            }
            selections.append(selection)
        }
        self.rows = rows
        hunkSelections = selections
    }

    /// The changed lines among `rows`.
    func selection(of indexes: IndexSet) -> LineSelection {
        var selection = LineSelection(removed: [], added: [])
        for index in indexes where index < rows.count {
            guard case let .line(kind, old, new, _, _, _) = rows[index] else { continue }
            switch kind {
            case .removed: old.map { selection.removed.append($0) }
            case .added: new.map { selection.added.append($0) }
            case .context: break
            }
        }
        return selection
    }
}

/// A virtualized, monospaced diff: one table row per line. With actions, hunk headers get
/// buttons and changed lines can be selected.
struct DiffTableView: NSViewRepresentable {
    let content: DiffRows
    var actions: DiffActions?
    var onSelectLines: (LineSelection) -> Void = { _ in }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> DiffScrollView {
        let table = NSTableView()
        table.headerView = nil
        table.style = .plain
        table.rowHeight = DiffLineView.rowHeight
        table.intercellSpacing = .zero
        table.columnAutoresizingStyle = .noColumnAutoresizing
        table.gridStyleMask = []
        table.backgroundColor = .textBackgroundColor
        table.allowsMultipleSelection = true
        table.allowsEmptySelection = true
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("line"))
        table.addTableColumn(column)
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        context.coordinator.table = table

        let scroll = DiffScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        scroll.backgroundColor = .textBackgroundColor
        return scroll
    }

    func updateNSView(_ scroll: DiffScrollView, context: Context) {
        let longest = content.rows.lazy.map(\.textLength).max() ?? 0
        scroll.contentWidth = DiffLineView.width(forCharacters: longest)
        let coordinator = context.coordinator
        coordinator.actions = actions
        coordinator.onSelectLines = onSelectLines
        coordinator.show(content)
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        weak var table: NSTableView?
        var actions: DiffActions?
        var onSelectLines: (LineSelection) -> Void = { _ in }
        private var content = DiffRows(rows: [])

        func show(_ content: DiffRows) {
            guard let table else { return }
            let sameFile = content.file == self.content.file
            self.content = content
            table.selectionHighlightStyle = actions == nil ? .none : .regular
            table.deselectAll(nil)
            table.reloadData()
            // Keep the place after staging part of the same file; start at the top of a new one.
            if !sameFile {
                table.scrollRowToVisible(0)
            }
        }

        func numberOfRows(in tableView: NSTableView) -> Int { content.rows.count }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            switch content.rows[row] {
            case let .hunk(index, header):
                let view = reuse(tableView, "hunk") { HunkHeaderView() }
                view.configure(
                    header: header, mode: actions?.mode,
                    perform: { [weak self] action in
                        guard let self, let actions = self.actions, index < self.content.hunkSelections.count
                        else { return }
                        actions.perform(action, self.content.hunkSelections[index])
                    })
                return view
            case .line:
                let view = reuse(tableView, "line") { DiffLineView() }
                view.row = content.rows[row]
                return view
            }
        }

        /// Only changed lines can be selected, and only when there is something to do with them.
        func tableView(
            _ tableView: NSTableView, selectionIndexesForProposedSelection proposed: IndexSet
        ) -> IndexSet {
            guard actions != nil else { return [] }
            return proposed.filteredIndexSet { $0 < content.rows.count && content.rows[$0].isChange }
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard let table else { return }
            onSelectLines(content.selection(of: table.selectedRowIndexes))
        }

        private func reuse<View: NSView>(_ tableView: NSTableView, _ identifier: String, make: () -> View)
            -> View
        {
            let identifier = NSUserInterfaceItemIdentifier(identifier)
            if let view = tableView.makeView(withIdentifier: identifier, owner: nil) as? View {
                return view
            }
            let view = make()
            view.identifier = identifier
            return view
        }
    }
}

extension DiffRows {
    fileprivate init(rows: [DiffRow]) {
        file = Data()
        self.rows = rows
        hunkSelections = []
    }
}

/// Keeps the diff column at least as wide as the visible area, so line backgrounds reach the
/// edge, and wider when lines are longer, so they scroll horizontally.
final class DiffScrollView: NSScrollView {
    var contentWidth: CGFloat = 0 {
        didSet { needsLayout = true }
    }

    override func tile() {
        super.tile()
        if let table = documentView as? NSTableView, let column = table.tableColumns.first {
            let width = max(contentWidth, contentSize.width)
            if column.width != width {
                column.width = width
            }
        }
    }
}

/// A hunk's `@@ -a,b +c,d @@` line, with its actions.
final class HunkHeaderView: NSTableCellView {
    private let label = NSTextField(labelWithString: "")
    private let buttons = NSStackView()
    private var perform: (LineAction) -> Void = { _ in }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        label.font = DiffLineView.font
        label.textColor = .secondaryLabelColor
        buttons.orientation = .horizontal
        buttons.spacing = 6
        let row = NSStackView(views: [label, NSView(), buttons])
        row.orientation = .horizontal
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: DiffLineView.gutterWidth),
            row.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -8),
            row.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from nibs")
    }

    override var wantsUpdateLayer: Bool { true }

    override func updateLayer() {
        layer?.backgroundColor = NSColor.quaternarySystemFill.cgColor
    }

    func configure(header: String, mode: DiffActions.Mode?, perform: @escaping (LineAction) -> Void) {
        label.stringValue = header
        self.perform = perform
        buttons.arrangedSubviews.forEach { $0.removeFromSuperview() }
        let actions: [(String, LineAction)] =
            switch mode {
            case .unstaged: [("Discard Hunk", .discard), ("Stage Hunk", .stage)]
            case .staged: [("Unstage Hunk", .unstage)]
            case nil: []
            }
        for (title, action) in actions {
            let button = HunkButton(title: title, action: action) { [weak self] in self?.perform($0) }
            buttons.addArrangedSubview(button)
        }
    }
}

/// A small button that reports which hunk action it stands for.
private final class HunkButton: NSButton {
    private let lineAction: LineAction
    private let handler: (LineAction) -> Void

    init(title: String, action: LineAction, handler: @escaping (LineAction) -> Void) {
        lineAction = action
        self.handler = handler
        super.init(frame: .zero)
        self.title = title
        bezelStyle = .accessoryBarAction
        controlSize = .small
        font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        target = self
        self.action = #selector(clicked)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from nibs")
    }

    @objc private func clicked() {
        handler(lineAction)
    }
}

/// Draws one diff line: old and new line numbers, a +/− marker and the text.
final class DiffLineView: NSTableCellView {
    static let rowHeight: CGFloat = 18
    static let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    private static let numberFont = NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .regular)
    private static let characterWidth = ("M" as NSString).size(withAttributes: [.font: font]).width
    private static let numberColumnWidth: CGFloat = 46
    private static let markerWidth: CGFloat = 18
    /// Columns per tab stop.
    nonisolated static let tabWidth = 4
    private static let paragraphStyle: NSParagraphStyle = {
        let style = NSMutableParagraphStyle()
        style.tabStops = []
        style.defaultTabInterval = characterWidth * CGFloat(tabWidth)
        return style
    }()

    static var gutterWidth: CGFloat { numberColumnWidth * 2 + markerWidth }

    static func width(forCharacters count: Int) -> CGFloat {
        gutterWidth + CGFloat(count) * characterWidth + 24
    }

    var row: DiffRow? {
        didSet {
            needsDisplay = true
            updateAccessibility()
        }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from nibs")
    }

    override var isFlipped: Bool { true }

    /// Changes when the row is selected or deselected.
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { needsDisplay = true }
    }

    private var isSelected: Bool {
        (superview as? NSTableRowView)?.isSelected ?? false
    }

    override func draw(_ dirtyRect: NSRect) {
        guard case let .line(kind, old, new, text, highlights, missingNewline) = row else { return }
        let textY = (bounds.height - Self.font.ascender + Self.font.descender) / 2 - 1
        let (background, marker): (NSColor?, String) =
            switch kind {
            case .added: (.systemGreen.withAlphaComponent(0.16), "+")
            case .removed: (.systemRed.withAlphaComponent(0.16), "−")
            case .context: (nil, "")
            }
        if let background {
            background.setFill()
            bounds.fill()
        }
        if isSelected {
            NSColor.selectedContentBackgroundColor.withAlphaComponent(0.35).setFill()
            bounds.fill()
        }
        let numberAttributes: [NSAttributedString.Key: Any] = [
            .font: Self.numberFont, .foregroundColor: NSColor.tertiaryLabelColor,
        ]
        drawNumber(old, rightEdge: Self.numberColumnWidth - 6, attributes: numberAttributes)
        drawNumber(new, rightEdge: Self.numberColumnWidth * 2 - 6, attributes: numberAttributes)
        (marker as NSString).draw(
            at: NSPoint(x: Self.numberColumnWidth * 2 + 4, y: textY),
            withAttributes: [.font: Self.font, .foregroundColor: NSColor.secondaryLabelColor])
        let attributed = NSMutableAttributedString(
            string: text,
            attributes: [
                .font: Self.font, .foregroundColor: NSColor.labelColor, .paragraphStyle: Self.paragraphStyle,
            ])
        let length = attributed.length
        for highlight in highlights {
            guard let color = highlight.kind.color else { continue }
            let start = min(Int(highlight.start), length)
            let end = min(Int(highlight.end), length)
            if start < end {
                attributed.addAttribute(.foregroundColor, value: color, range: NSRange(start..<end))
            }
        }
        if missingNewline {
            attributed.append(
                NSAttributedString(
                    string: "  ⏎̸ no newline at end of file",
                    attributes: [.font: Self.numberFont, .foregroundColor: NSColor.tertiaryLabelColor]
                ))
        }
        attributed.draw(at: NSPoint(x: Self.gutterWidth, y: textY))
    }

    private func drawNumber(_ number: UInt32?, rightEdge: CGFloat, attributes: [NSAttributedString.Key: Any]) {
        guard let number else { return }
        let string = String(number) as NSString
        let size = string.size(withAttributes: attributes)
        string.draw(
            at: NSPoint(x: rightEdge - size.width, y: (bounds.height - size.height) / 2),
            withAttributes: attributes)
    }

    private func updateAccessibility() {
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        guard case let .line(kind, _, _, text, _, _) = row else {
            setAccessibilityLabel(nil)
            return
        }
        let prefix =
            switch kind {
            case .added: "Added: "
            case .removed: "Removed: "
            case .context: ""
            }
        setAccessibilityLabel(prefix + text)
    }
}

extension HighlightKind {
    /// System colors, so highlighting follows the light and dark appearances and Increase
    /// Contrast. Operators and properties keep the text color, to keep lines calm.
    fileprivate var color: NSColor? {
        switch self {
        case .keyword: .systemPink
        case .string: .systemRed
        case .comment: .secondaryLabelColor
        case .number: .systemIndigo
        case .constant: .systemPurple
        case .type: .systemTeal
        case .function: .systemBlue
        case .attribute: .systemOrange
        case .tag: .systemPink
        case .property, .operator: nil
        }
    }
}
