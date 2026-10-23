// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore

/// Draws one row of the commit graph. The core decides lanes and lines; this only draws them.
///
/// Commits are their author's avatar, ringed in the lane's color; merges are small rings. Lines
/// change lanes with rounded corners, and a tinted band runs from the commit to its message.
///
/// When the graph is wider than the column, lines fade out at its right edge, and commits in
/// lanes past it are pinned to the last lane that fits, with a shadow and a chevron, so every row
/// still shows its commit.
final class GraphCellView: NSTableCellView {
    static let laneWidth: CGFloat = 22
    static let padding: CGFloat = 8
    static let avatarDiameter: CGFloat = 22
    private static let ringWidth: CGFloat = 2
    private static let mergeDiameter: CGFloat = 10
    private static let lineWidth: CGFloat = 2
    private static let cornerRadius: CGFloat = 9

    static func width(forLanes lanes: CGFloat) -> CGFloat {
        lanes * laneWidth + padding * 2
    }

    private var graph: GraphRow?
    private var author: (name: String, email: String) = ("", "")
    private var hasRefs = false

    func configure(graph: GraphRow, authorName: String, authorEmail: String, hasRefs: Bool) {
        self.graph = graph
        author = (authorName, authorEmail)
        self.hasRefs = hasRefs
        needsDisplay = true
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        // Since macOS 14 views draw outside their bounds unless told not to.
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not used from nibs")
    }

    override var isFlipped: Bool { true }

    // Resizing the column changes which lanes fit.
    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        needsDisplay = true
    }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { needsDisplay = true }
    }

    private var isSelected: Bool {
        (superview as? NSTableRowView)?.isSelected ?? false
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let graph else { return }
        let height = bounds.height
        let middle = height / 2
        func x(_ lane: UInt16) -> CGFloat {
            Self.padding + (CGFloat(lane) + 0.5) * Self.laneWidth
        }
        // The last lane that fits whole in the column.
        let lastLane = UInt16(max(0, Int((bounds.width - Self.padding * 2) / Self.laneWidth) - 1))
        let overflows = graph.laneCount > lastLane + 1
        let isPinned = graph.nodeLane > lastLane
        let nodeX = x(min(graph.nodeLane, lastLane))
        let laneColor = GraphPalette.color(graph.nodeColor)

        // The band behind the row, from the commit to its message.
        if !isSelected {
            laneColor.withAlphaComponent(0.16).setFill()
            let band = Self.avatarDiameter - 4
            NSRect(x: nodeX, y: middle - band / 2, width: bounds.maxX - nodeX, height: band).fill()
        }
        // The line from the branch labels on the left to the commit.
        if hasRefs {
            let connector = NSBezierPath()
            connector.move(to: NSPoint(x: 0, y: middle))
            connector.line(to: NSPoint(x: nodeX, y: middle))
            connector.lineWidth = 1
            laneColor.withAlphaComponent(0.7).setStroke()
            connector.stroke()
        }

        // Lines go on their own layer, so the fade at the edge makes them transparent whatever is
        // behind them: the band, the selection, the window.
        let context = NSGraphicsContext.current?.cgContext
        if overflows {
            context?.beginTransparencyLayer(auxiliaryInfo: nil)
        }
        for edge in graph.edges {
            let from = x(edge.fromLane)
            let to = x(edge.toLane)
            let path = NSBezierPath()
            path.lineWidth = Self.lineWidth
            path.lineCapStyle = .round
            path.lineJoinStyle = .round
            switch edge.kind {
            case .through:
                path.move(to: NSPoint(x: from, y: 0))
                path.line(to: NSPoint(x: from, y: height))
            case .intoNode:
                // Down from the top, then sideways into the commit.
                path.move(to: NSPoint(x: from, y: 0))
                if from != to {
                    path.appendArc(
                        from: NSPoint(x: from, y: middle), to: NSPoint(x: to, y: middle),
                        radius: Self.cornerRadius)
                }
                path.line(to: NSPoint(x: to, y: middle))
            case .outOfNode:
                // Sideways out of the commit, then down to the bottom.
                path.move(to: NSPoint(x: from, y: middle))
                if from != to {
                    path.appendArc(
                        from: NSPoint(x: to, y: middle), to: NSPoint(x: to, y: height),
                        radius: Self.cornerRadius)
                }
                path.line(to: NSPoint(x: to, y: height))
            }
            GraphPalette.color(edge.color).setStroke()
            path.stroke()
        }
        if overflows, let context {
            fadeRightEdge(of: context, from: x(lastLane) + Self.lineWidth)
            context.endTransparencyLayer()
        }

        // A pinned commit floats above the lines it covers.
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        if isPinned {
            let shadow = NSShadow()
            shadow.shadowColor = NSColor.black.withAlphaComponent(0.35)
            shadow.shadowBlurRadius = 3
            shadow.shadowOffset = NSSize(width: 0, height: -1)
            shadow.set()
        }

        let isMerge = graph.edges.filter { $0.kind == .outOfNode }.count > 1
        if isMerge {
            let radius = Self.mergeDiameter / 2
            let ring = NSBezierPath(
                ovalIn: NSRect(x: nodeX - radius, y: middle - radius, width: radius * 2, height: radius * 2))
            NSColor.textBackgroundColor.setFill()
            ring.fill()
            ring.lineWidth = Self.ringWidth
            laneColor.setStroke()
            ring.stroke()
        } else {
            let radius = Self.avatarDiameter / 2
            drawAvatar(
                in: NSRect(x: nodeX - radius, y: middle - radius, width: radius * 2, height: radius * 2),
                ring: laneColor)
        }
        if isPinned {
            NSShadow().set()
            drawChevron(after: nodeX + (isMerge ? Self.mergeDiameter : Self.avatarDiameter) / 2, middle: middle)
        }
    }

    /// Erases the layer's content progressively from `start` to the right edge.
    private func fadeRightEdge(of context: CGContext, from start: CGFloat) {
        guard start < bounds.maxX, let sRGB = CGColorSpace(name: CGColorSpace.sRGB),
            let gradient = CGGradient(
                colorSpace: sRGB, colorComponents: [0, 0, 0, 0, 0, 0, 0, 1] as [CGFloat],
                locations: [0, 1], count: 2)
        else { return }
        context.saveGState()
        context.setBlendMode(.destinationOut)
        context.clip(to: CGRect(x: start, y: 0, width: bounds.maxX - start, height: bounds.height))
        context.drawLinearGradient(
            gradient, start: CGPoint(x: start, y: 0), end: CGPoint(x: bounds.maxX, y: 0), options: [])
        context.restoreGState()
    }

    /// A small "›" after a pinned commit: it really is further right.
    private func drawChevron(after x: CGFloat, middle: CGFloat) {
        let chevron = NSBezierPath()
        chevron.move(to: NSPoint(x: x + 2.5, y: middle - 3))
        chevron.line(to: NSPoint(x: x + 5.5, y: middle))
        chevron.line(to: NSPoint(x: x + 2.5, y: middle + 3))
        chevron.lineWidth = 1.5
        chevron.lineCapStyle = .round
        chevron.lineJoinStyle = .round
        NSColor.secondaryLabelColor.setStroke()
        chevron.stroke()
    }

    private func drawAvatar(in rect: NSRect, ring: NSColor) {
        ring.setFill()
        NSBezierPath(ovalIn: rect).fill()
        let face = rect.insetBy(dx: Self.ringWidth, dy: Self.ringWidth)
        let author = AvatarStore.shared.author(name: author.name, email: author.email)
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSBezierPath(ovalIn: face).addClip()
        if let image = author.image {
            image.draw(
                in: face, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true,
                hints: [.interpolation: NSImageInterpolation.high.rawValue])
            return
        }
        AvatarPalette.color(author.color).setFill()
        face.fill()
        let font = NSFont.systemFont(ofSize: face.height * (author.initials.count > 1 ? 0.4 : 0.48), weight: .semibold)
        let text = author.initials as NSString
        let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.white]
        let size = text.size(withAttributes: attributes)
        text.draw(
            at: NSPoint(x: face.midX - size.width / 2, y: face.midY - size.height / 2),
            withAttributes: attributes)
    }
}

enum GraphPalette {
    /// System colors, so the graph follows light and dark mode and the accessibility contrast
    /// settings.
    @MainActor private static let colors: [NSColor] = [
        .systemBlue, .systemOrange, .systemGreen, .systemPink, .systemPurple, .systemTeal,
        .systemYellow, .systemRed, .systemIndigo, .systemBrown, .systemMint, .systemCyan,
    ]

    @MainActor static func color(_ index: UInt16) -> NSColor {
        colors[Int(index) % colors.count]
    }
}

/// Backgrounds for authors' initials: darker system colors, so white letters stay readable.
enum AvatarPalette {
    @MainActor private static let colors: [NSColor] = [
        .systemBlue, .systemIndigo, .systemPurple, .systemPink, .systemRed, .systemOrange,
        .systemBrown, .systemTeal, .systemGreen, .systemGray,
    ]

    @MainActor static func color(_ index: UInt16) -> NSColor {
        let color = colors[Int(index) % colors.count]
        return color.blended(withFraction: 0.25, of: .black) ?? color
    }
}
