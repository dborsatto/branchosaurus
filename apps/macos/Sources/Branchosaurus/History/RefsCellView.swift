// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore

/// The branch and tag column: the commit's first label as a pill in its lane's color, "+N" for
/// the others, and a line to the commit in the graph.
///
/// The pill shows a check on the checked-out branch, a laptop when the branch is local and a
/// cloud when it is on a remote, so `main` and `origin/main` on the same commit share one pill.
final class RefsCellView: NSTableCellView {
    private static let pillHeight: CGFloat = 18
    private static let inset: CGFloat = 6
    private static let iconSize: CGFloat = 12
    private static let gap: CGFloat = 3

    private var refs: [RefLabel] = []
    private var color: NSColor = .systemBlue

    func configure(refs: [RefLabel], color: NSColor) {
        self.refs = refs
        self.color = color
        toolTip = refs.isEmpty ? nil : refs.map(\.longDescription).joined(separator: "\n")
        setAccessibilityElement(!refs.isEmpty)
        setAccessibilityRole(.staticText)
        setAccessibilityLabel(refs.map(\.longDescription).joined(separator: ", "))
        needsDisplay = true
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

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { needsDisplay = true }
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let first = refs.first else { return }
        let emphasized = backgroundStyle == .emphasized
        let textColor: NSColor = emphasized ? .alternateSelectedControlTextColor : .labelColor
        let fill = emphasized ? NSColor.white.withAlphaComponent(0.25) : color.withAlphaComponent(first.isHead ? 0.4 : 0.22)
        let middle = bounds.midY

        // "+N" for the other labels, kept visible however narrow the column is.
        let extra = refs.count - 1
        let extraAttributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 10, weight: .semibold), .foregroundColor: textColor,
        ]
        let extraText = "+\(extra)" as NSString
        let extraWidth = extra > 0 ? extraText.size(withAttributes: extraAttributes).width + 10 : 0

        let leading = first.leadingSymbol
        let trailing = first.trailingSymbols
        let fixed =
            Self.inset * 2 + (leading == nil ? 0 : Self.iconSize + Self.gap)
            + CGFloat(trailing.count) * (Self.iconSize + Self.gap)
        let font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize + 1, weight: first.isHead ? .semibold : .medium)
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byTruncatingTail
        let name = NSAttributedString(
            string: first.name,
            attributes: [.font: font, .foregroundColor: textColor, .paragraphStyle: paragraph])
        let available = bounds.width - Self.inset - extraWidth - (extra > 0 ? 4 : 0) - 8
        let textWidth = max(0, min(ceil(name.size().width), available - fixed))
        let pill = NSRect(
            x: Self.inset, y: middle - Self.pillHeight / 2, width: fixed + textWidth, height: Self.pillHeight)

        // The connector first, so the pill covers its start.
        let connector = NSBezierPath()
        connector.move(to: NSPoint(x: pill.maxX, y: middle))
        connector.line(to: NSPoint(x: bounds.maxX, y: middle))
        connector.lineWidth = 1
        (emphasized ? NSColor.white.withAlphaComponent(0.7) : color.withAlphaComponent(0.7)).setStroke()
        connector.stroke()

        fill.setFill()
        NSBezierPath(roundedRect: pill, xRadius: 4, yRadius: 4).fill()
        var x = pill.minX + Self.inset
        if let leading {
            drawSymbol(leading, at: x, middle: middle, color: textColor)
            x += Self.iconSize + Self.gap
        }
        let textHeight = ceil(name.size().height)
        name.draw(
            with: NSRect(x: x, y: middle - textHeight / 2, width: textWidth, height: textHeight),
            options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        x += textWidth + Self.gap
        for symbol in trailing {
            drawSymbol(symbol, at: x, middle: middle, color: textColor.withAlphaComponent(0.75))
            x += Self.iconSize + Self.gap
        }

        if extra > 0 {
            let badge = NSRect(x: pill.maxX + 4, y: middle - 7, width: extraWidth, height: 14)
            // Opaque, so the connector doesn't show through.
            (emphasized ? NSColor.white.withAlphaComponent(0.25) : NSColor.textBackgroundColor).setFill()
            NSBezierPath(roundedRect: badge, xRadius: 7, yRadius: 7).fill()
            fill.setFill()
            NSBezierPath(roundedRect: badge, xRadius: 7, yRadius: 7).fill()
            let size = extraText.size(withAttributes: extraAttributes)
            extraText.draw(
                at: NSPoint(x: badge.midX - size.width / 2, y: badge.midY - size.height / 2),
                withAttributes: extraAttributes)
        }
    }

    private func drawSymbol(_ name: String, at x: CGFloat, middle: CGFloat, color: NSColor) {
        let configuration = NSImage.SymbolConfiguration(pointSize: Self.iconSize - 1, weight: .semibold)
            .applying(NSImage.SymbolConfiguration(paletteColors: [color]))
        guard
            let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
                .withSymbolConfiguration(configuration)
        else { return }
        let size = image.size
        let scale = min(1, Self.iconSize / max(size.width, size.height))
        let rect = NSRect(
            x: x + (Self.iconSize - size.width * scale) / 2, y: middle - size.height * scale / 2,
            width: size.width * scale, height: size.height * scale)
        image.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
    }
}

extension RefLabel {
    fileprivate var leadingSymbol: String? {
        if isHead { return "checkmark" }
        return kind == .tag ? "tag" : nil
    }

    /// Where the branch is: on this computer, on a remote, or both.
    fileprivate var trailingSymbols: [String] {
        guard kind == .branch else { return [] }
        return (local ? ["laptopcomputer"] : []) + (remotes.isEmpty ? [] : ["cloud"])
    }

    /// "Branch main, checked out, also on origin", for tooltips and VoiceOver.
    var longDescription: String {
        switch kind {
        case .head:
            return "Detached HEAD"
        case .tag:
            return "Tag \(name)"
        case .branch:
            guard local else {
                return "Remote branch " + remotes.map { "\($0)/\(name)" }.joined(separator: ", ")
            }
            var description = "Branch \(name)"
            if isHead { description += ", checked out" }
            if !remotes.isEmpty { description += ", also on \(remotes.joined(separator: ", "))" }
            return description
        }
    }
}
