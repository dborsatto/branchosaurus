// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore
import SwiftUI

/// The commit list. An `NSTableView` because it only asks for the rows on screen, which keeps
/// repositories with millions of commits as fast as small ones.
struct HistoryTableView: NSViewRepresentable {
    let history: History
    @Binding var selection: String?
    /// Runs a command from the row context menu on a commit id.
    var onCommand: (HistoryCommand, String) -> Void = { _, _ in }

    func makeCoordinator() -> Coordinator {
        Coordinator(selection: $selection)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let table = NSTableView()
        table.style = .fullWidth
        table.allowsMultipleSelection = false
        table.allowsEmptySelection = true
        table.rowHeight = 28
        // No gaps: the branch connector, the graph's band and the message's bar touch.
        table.intercellSpacing = .zero
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.autosaveName = "HistoryColumns"
        table.autosaveTableColumns = true

        for column in Column.allCases {
            let tableColumn = NSTableColumn(identifier: column.identifier)
            tableColumn.title = column.title
            tableColumn.width = column.width
            tableColumn.minWidth = column.minWidth
            tableColumn.resizingMask = column == .description ? .autoresizingMask : .userResizingMask
            table.addTableColumn(tableColumn)
        }

        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.menu = context.coordinator.makeMenu()
        context.coordinator.table = table

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let coordinator = context.coordinator
        coordinator.selection = $selection
        coordinator.onCommand = onCommand
        coordinator.show(history)
        coordinator.reveal(selection)
    }

    /// Branches on the left, then the graph and the message. Authors are the graph's avatars;
    /// dates are the time groups.
    enum Column: String, CaseIterable {
        case refs, graph, description

        var identifier: NSUserInterfaceItemIdentifier { NSUserInterfaceItemIdentifier(rawValue) }

        var title: String {
            switch self {
            case .refs: "Branch / Tag"
            case .graph: "Graph"
            case .description: "Message"
            }
        }

        var width: CGFloat {
            switch self {
            case .refs: 150
            case .graph: 120
            case .description: 480
            }
        }

        var minWidth: CGFloat {
            switch self {
            case .refs: 48
            case .graph: 30
            case .description: 160
            }
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
        var selection: Binding<String?>
        var onCommand: (HistoryCommand, String) -> Void = { _, _ in }
        weak var table: NSTableView?
        private var history: History?
        private var cache: HistoryRowCache?
        /// The last id the table itself selected, to tell user clicks from external changes.
        private var selectedID: String?
        /// Commit ages are relative to this; it moves on every minute.
        private var now = Date()
        private let ages: RelativeDateTimeFormatter = {
            let formatter = RelativeDateTimeFormatter()
            formatter.dateTimeStyle = .named
            formatter.unitsStyle = .full
            return formatter
        }()

        init(selection: Binding<String?>) {
            self.selection = selection
            super.init()
            NotificationCenter.default.addObserver(
                self, selector: #selector(avatarsChanged), name: AvatarStore.didChange, object: nil)
            Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] timer in
                guard let self else { return timer.invalidate() }
                MainActor.assumeIsolated {
                    self.now = Date()
                    self.refreshAges()
                }
            }
        }

        func show(_ history: History) {
            guard history !== self.history, let table else { return }
            self.history = history
            cache = HistoryRowCache(history: history)
            selectedID = nil
            table.reloadData()
            // Wide enough for typical graphs; wider ones are clipped until the column is resized.
            now = Date()
            if let column = table.tableColumn(withIdentifier: Column.graph.identifier) {
                let lanes = CGFloat(min(Int(history.maxLanes()), 12))
                column.width = max(column.width, GraphCellView.width(forLanes: lanes))
            }
        }

        func reveal(_ id: String?) {
            guard let table, let history, id != selectedID else { return }
            selectedID = id
            guard let id, let row = history.rowOf(id: id) else {
                table.deselectAll(nil)
                return
            }
            table.selectRowIndexes(IndexSet(integer: Int(row)), byExtendingSelection: false)
            table.scrollRowToVisible(Int(row))
        }

        func numberOfRows(in tableView: NSTableView) -> Int {
            Int(history?.rowCount() ?? 0)
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let tableColumn, let column = Column(rawValue: tableColumn.identifier.rawValue),
                let item = cache?.row(at: row)
            else { return nil }

            let color = GraphPalette.color(item.graph.nodeColor)
            switch column {
            case .refs:
                let cell = reuse(tableView, column) { RefsCellView() }
                cell.configure(refs: item.refs, color: color)
                return cell
            case .graph:
                let cell = reuse(tableView, column) { GraphCellView() }
                cell.configure(
                    graph: item.graph, authorName: item.authorName, authorEmail: item.authorEmail,
                    hasRefs: !item.refs.isEmpty)
                cell.toolTip =
                    "\(item.authorName) <\(item.authorEmail)>\n"
                    + item.authorDate.formatted(date: .complete, time: .shortened)
                return cell
            case .description:
                let cell = reuse(tableView, column) { DescriptionCellView() }
                cell.configure(summary: item.summary, color: color, groupAge: groupAge(row))
                return cell
            }
        }

        func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
            let view = HistoryRowView()
            view.startsGroup = row > 0 && groupAge(row) != nil
            return view
        }

        // MARK: Time groups

        /// "3 days ago", by commit time, which history is sorted by.
        private func age(_ row: Int) -> String? {
            cache?.row(at: row).map { ages.localizedString(for: $0.commitDate, relativeTo: now) }
        }

        /// The age of the group a row starts, if it starts one.
        private func groupAge(_ row: Int) -> String? {
            guard let age = age(row) else { return nil }
            return row == 0 || age != self.age(row - 1) ? age : nil
        }

        private func refreshAges() {
            guard let table else { return }
            let visible = table.rows(in: table.visibleRect)
            guard visible.length > 0 else { return }
            let rows = IndexSet(integersIn: visible.location..<visible.location + visible.length)
            table.reloadData(
                forRowIndexes: rows,
                columnIndexes: IndexSet(integer: table.column(withIdentifier: Column.description.identifier)))
            for row in rows {
                (table.rowView(atRow: row, makeIfNecessary: false) as? HistoryRowView)?.startsGroup =
                    row > 0 && groupAge(row) != nil
            }
        }

        @objc private func avatarsChanged() {
            table?.enumerateAvailableRowViews { rowView, _ in
                for column in 0..<rowView.numberOfColumns {
                    (rowView.view(atColumn: column) as? GraphCellView)?.needsDisplay = true
                }
            }
        }

        func makeMenu() -> NSMenu {
            let menu = NSMenu()
            menu.delegate = self
            for command in HistoryCommand.allCases {
                if command == .copyID {
                    menu.addItem(.separator())
                }
                let item = NSMenuItem(title: command.title, action: #selector(runCommand(_:)), keyEquivalent: "")
                item.target = self
                item.representedObject = command.rawValue
                menu.addItem(item)
            }
            return menu
        }

        func menuNeedsUpdate(_ menu: NSMenu) {
            let hasRow = (table?.clickedRow ?? -1) >= 0
            menu.items.forEach { $0.isEnabled = hasRow }
        }

        @objc private func runCommand(_ sender: NSMenuItem) {
            guard let table, table.clickedRow >= 0,
                let raw = sender.representedObject as? String,
                let command = HistoryCommand(rawValue: raw),
                let id = cache?.row(at: table.clickedRow)?.id
            else { return }
            onCommand(command, id)
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard let table else { return }
            let id = table.selectedRow >= 0 ? cache?.row(at: table.selectedRow)?.id : nil
            selectedID = id
            if selection.wrappedValue != id {
                selection.wrappedValue = id
            }
        }

        private func reuse<View: NSView>(
            _ tableView: NSTableView, _ column: Column, make: () -> View
        ) -> View {
            if let view = tableView.makeView(withIdentifier: column.identifier, owner: nil) as? View {
                return view
            }
            let view = make()
            view.identifier = column.identifier
            return view
        }
    }
}

/// Commands in the commit list's context menu.
enum HistoryCommand: String, CaseIterable {
    case checkout, newBranch, copyID

    var title: String {
        switch self {
        case .checkout: "Check Out (Detached HEAD)"
        case .newBranch: "New Branch from Here…"
        case .copyID: "Copy Commit ID"
        }
    }
}

/// Fetches rows from the core in blocks, so scrolling asks the core once per block, not per cell.
@MainActor
final class HistoryRowCache {
    private static let blockSize: UInt32 = 256
    private static let maxBlocks = 64

    private let history: History
    private var blocks: [UInt32: [HistoryRow]] = [:]
    private var recentBlocks: [UInt32] = []

    init(history: History) {
        self.history = history
    }

    func row(at index: Int) -> HistoryRow? {
        guard index >= 0 else { return nil }
        let block = UInt32(index) / Self.blockSize
        let offset = Int(UInt32(index) % Self.blockSize)
        if let rows = blocks[block] {
            return offset < rows.count ? rows[offset] : nil
        }
        guard let rows = try? history.rows(start: block * Self.blockSize, count: Self.blockSize)
        else { return nil }
        blocks[block] = rows
        recentBlocks.append(block)
        if recentBlocks.count > Self.maxBlocks {
            blocks[recentBlocks.removeFirst()] = nil
        }
        return offset < rows.count ? rows[offset] : nil
    }
}
