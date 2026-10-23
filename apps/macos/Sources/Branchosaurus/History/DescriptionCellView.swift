// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore

/// The commit summary, after a bar in its lane's color. The first commit of each time group
/// ("yesterday", "3 days ago") shows the group's age on the right.
final class DescriptionCellView: NSTableCellView {
    private static let barWidth: CGFloat = 3

    private let summary = NSTextField(labelWithString: "")
    private let age = NSTextField(labelWithString: "")
    private var color: NSColor = .clear
    private var summaryToAge: NSLayoutConstraint!
    private var summaryToEdge: NSLayoutConstraint!

    override init(frame: NSRect) {
        super.init(frame: frame)
        summary.lineBreakMode = .byTruncatingTail
        summary.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        textField = summary
        age.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        age.textColor = .secondaryLabelColor
        age.setContentCompressionResistancePriority(.required, for: .horizontal)
        age.setContentHuggingPriority(.required, for: .horizontal)
        for field in [summary, age] {
            field.translatesAutoresizingMaskIntoConstraints = false
            addSubview(field)
        }
        summaryToAge = summary.trailingAnchor.constraint(lessThanOrEqualTo: age.leadingAnchor, constant: -8)
        summaryToEdge = summary.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -8)
        NSLayoutConstraint.activate([
            summary.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.barWidth + 9),
            summary.centerYAnchor.constraint(equalTo: centerYAnchor),
            age.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            age.centerYAnchor.constraint(equalTo: centerYAnchor),
            summaryToEdge,
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from nibs")
    }

    override var isFlipped: Bool { true }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet {
            age.textColor =
                backgroundStyle == .emphasized
                ? .alternateSelectedControlTextColor.withAlphaComponent(0.8) : .secondaryLabelColor
            needsDisplay = true
        }
    }

    /// `groupAge` is set on the first commit of a time group.
    func configure(summary text: String, color: NSColor, groupAge: String?) {
        summary.stringValue = text
        self.color = color
        age.stringValue = groupAge ?? ""
        age.isHidden = groupAge == nil
        summaryToAge.isActive = groupAge != nil
        summaryToEdge.isActive = groupAge == nil
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        let height = GraphCellView.avatarDiameter - 4
        (backgroundStyle == .emphasized ? NSColor.white.withAlphaComponent(0.8) : color).setFill()
        NSRect(x: 0, y: (bounds.height - height) / 2, width: Self.barWidth, height: height).fill()
    }
}

/// A commit-list row. The first row of each time group starts with a separator line.
final class HistoryRowView: NSTableRowView {
    var startsGroup = false {
        didSet {
            if startsGroup != oldValue { needsDisplay = true }
        }
    }

    /// Cells draw differently when selected (no band), also when the window isn't key.
    override var isSelected: Bool {
        didSet {
            guard isSelected != oldValue else { return }
            subviews.forEach { $0.needsDisplay = true }
        }
    }

    override func drawBackground(in dirtyRect: NSRect) {
        super.drawBackground(in: dirtyRect)
        guard startsGroup else { return }
        NSColor.separatorColor.setFill()
        NSRect(x: 0, y: 0, width: bounds.width, height: 1).fill()
    }
}
