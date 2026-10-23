// SPDX-License-Identifier: GPL-3.0-or-later
//
// Draws the app icon, a stylized brachiosaurus, and writes Resources/AppIcon.icns. Run from
// apps/macos:
//
//     swift Scripts/make-icon.swift              # writes Resources/AppIcon.icns
//     swift Scripts/make-icon.swift preview.png  # writes a 1024 px preview instead
//
// Coordinates are on Apple's 1024 px icon grid, with the origin at the top left. Before drawing,
// the script checks that the commit graph on the animal's side stays inside its body.

import AppKit

func color(_ hex: UInt32, alpha: CGFloat = 1) -> NSColor {
    NSColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
}

let skyTop = color(0x3D8DEB)
let skyBottom = color(0xAADDFF)
let sun = color(0xFFD23F)
let sunGlow = color(0xFFF4C2)
let hillFar = color(0x6CC46A)
let hill = color(0x3E9446)
let skin = color(0xD28AD0)
let skinShade = color(0xAE6CB4)
let ink = color(0x2A1230)
let graphNode = color(0x2F2F2F)
let graphLine = color(0x5E5E5E)

extension NSBezierPath {
    func curve(_ x: CGFloat, _ y: CGFloat, _ c1x: CGFloat, _ c1y: CGFloat, _ c2x: CGFloat, _ c2y: CGFloat) {
        curve(to: NSPoint(x: x, y: y), controlPoint1: NSPoint(x: c1x, y: c1y), controlPoint2: NSPoint(x: c2x, y: c2y))
    }
}

// MARK: Shapes

/// Body, tail, neck and head as one silhouette.
func bodyPath() -> NSBezierPath {
    let body = NSBezierPath()
    body.move(to: NSPoint(x: 150, y: 668))  // tail tip
    body.curve(340, 528, 215, 640, 260, 545)  // top of the tail up to the hips
    body.curve(585, 478, 420, 470, 520, 452)  // back, rising towards the shoulders
    body.curve(700, 250, 640, 455, 655, 300)  // back of the neck
    body.curve(746, 186, 706, 220, 718, 188)  // back of the head, up to the dome
    body.curve(784, 208, 766, 184, 776, 198)  // over the dome to the brow
    body.curve(822, 230, 792, 214, 814, 214)  // snout
    body.curve(788, 258, 828, 248, 808, 258)  // jaw
    body.curve(742, 270, 772, 257, 756, 262)  // chin to throat
    body.curve(668, 560, 730, 330, 690, 470)  // front of the neck down to the chest
    body.curve(470, 650, 650, 630, 560, 660)  // chest and belly
    body.curve(318, 632, 400, 645, 355, 642)  // belly to the base of the tail
    body.curve(150, 668, 270, 640, 200, 672)  // underside of the tail
    body.close()
    return body
}

/// A column-like leg: wide at the top, narrowing to the ankle, with a slightly flared foot.
func leg(x: CGFloat, top: CGFloat, width: CGFloat, bottom: CGFloat) -> NSBezierPath {
    let ankle = bottom - 34
    let path = NSBezierPath()
    path.move(to: NSPoint(x: x - 6, y: top))
    path.line(to: NSPoint(x: x + width + 6, y: top))
    path.curve(x + width - 6, ankle, x + width + 2, top + 90, x + width - 8, ankle - 60)
    path.curve(x + width - 4, bottom, x + width - 4, ankle + 14, x + width + 2, bottom - 4)
    path.line(to: NSPoint(x: x + 2, y: bottom))
    path.curve(x + 8, ankle, x - 2, bottom - 4, x + 6, ankle + 14)
    path.curve(x - 6, top, x + 6, ankle - 60, x - 4, top + 90)
    path.close()
    return path
}

// MARK: Commit graph

/// The spots on the animal's side, drawn as a little commit graph. It starts on the tail and forks
/// at the second commit: a short branch rises to one commit on the back, and a longer one runs along
/// the body and up the neck.
struct Node {
    let x: CGFloat
    let y: CGFloat
    let radius: CGFloat
    var point: NSPoint { NSPoint(x: x, y: y) }
}

/// A line between two nodes: straight, or a cubic curve through two control points.
struct Edge {
    let from: Int
    let to: Int
    var controls: (NSPoint, NSPoint)? = nil

    func point(at t: CGFloat) -> NSPoint {
        let p0 = nodes[from].point
        let p3 = nodes[to].point
        guard let (p1, p2) = controls else {
            return NSPoint(x: p0.x + (p3.x - p0.x) * t, y: p0.y + (p3.y - p0.y) * t)
        }
        let u = 1 - t
        let a = u * u * u, b = 3 * u * u * t, c = 3 * u * t * t, d = t * t * t
        return NSPoint(
            x: a * p0.x + b * p1.x + c * p2.x + d * p3.x, y: a * p0.y + b * p1.y + c * p2.y + d * p3.y)
    }
}

let nodes = [
    Node(x: 252, y: 616, radius: 12),  // 0: on the tail
    Node(x: 345, y: 580, radius: 14),  // 1: by the hips, where the graph forks
    Node(x: 455, y: 512, radius: 14),  // 2: the short branch, on the back
    Node(x: 455, y: 590, radius: 14),  // 3: the long branch, along the body…
    Node(x: 560, y: 585, radius: 14),  // 4
    Node(x: 664, y: 432, radius: 13),  // 5: …and up the neck
    Node(x: 695, y: 330, radius: 12),  // 6
]
let edges = [
    Edge(from: 0, to: 1),
    Edge(from: 1, to: 2, controls: (NSPoint(x: 398, y: 580), NSPoint(x: 410, y: 512))),
    Edge(from: 1, to: 3),
    Edge(from: 3, to: 4),
    Edge(from: 4, to: 5),
    Edge(from: 5, to: 6),
]
let graphLineWidth: CGFloat = 8
/// How far every node and line must stay inside the body's outline.
let graphMargin: CGFloat = 6

/// Every place where a node or a line comes closer to the outline than `graphMargin`.
func graphProblems(inside body: NSBezierPath) -> [String] {
    var problems: [String] = []
    for node in nodes {
        let reach = node.radius + graphMargin
        for step in 0..<72 {
            let angle = CGFloat(step) * .pi / 36
            let point = NSPoint(x: node.x + reach * cos(angle), y: node.y + reach * sin(angle))
            if !body.contains(point) {
                problems.append("node at (\(Int(node.x)), \(Int(node.y))) reaches outside near (\(Int(point.x)), \(Int(point.y)))")
                break
            }
        }
    }
    // Walk each line in short steps, checking its width plus the margin on both sides.
    let reach = graphLineWidth / 2 + graphMargin
    for edge in edges {
        let steps = 60
        for step in 0..<steps {
            let start = edge.point(at: CGFloat(step) / CGFloat(steps))
            let end = edge.point(at: CGFloat(step + 1) / CGFloat(steps))
            let length = max(hypot(end.x - start.x, end.y - start.y), 0.001)
            let normal = NSPoint(x: -(end.y - start.y) / length, y: (end.x - start.x) / length)
            for side in [-1.0, 1.0] {
                let point = NSPoint(x: start.x + normal.x * reach * side, y: start.y + normal.y * reach * side)
                if !body.contains(point) {
                    problems.append("line from node \(edge.from) to \(edge.to) reaches outside at (\(Int(point.x)), \(Int(point.y)))")
                }
            }
        }
    }
    return problems
}

func drawGraph() {
    let path = NSBezierPath()
    path.lineWidth = graphLineWidth
    path.lineCapStyle = .round
    path.lineJoinStyle = .round
    for edge in edges {
        path.move(to: nodes[edge.from].point)
        if let (c1, c2) = edge.controls {
            path.curve(to: nodes[edge.to].point, controlPoint1: c1, controlPoint2: c2)
        } else {
            path.line(to: nodes[edge.to].point)
        }
    }
    graphLine.setStroke()
    path.stroke()
    graphNode.setFill()
    for node in nodes {
        NSBezierPath(
            ovalIn: NSRect(
                x: node.x - node.radius, y: node.y - node.radius, width: node.radius * 2,
                height: node.radius * 2)
        ).fill()
    }
}

// MARK: Drawing

func drawIcon() {
    // The rounded square every macOS app icon sits in: 824 px with ~22.5% corners.
    let tile = NSRect(x: 100, y: 100, width: 824, height: 824)
    let tilePath = NSBezierPath(roundedRect: tile, xRadius: 185, yRadius: 185)

    NSGraphicsContext.saveGraphicsState()
    tilePath.addClip()

    // Sky, deeper blue overhead and paler towards the horizon.
    NSGradient(starting: skyTop, ending: skyBottom)?
        .draw(from: NSPoint(x: 0, y: 100), to: NSPoint(x: 0, y: 760), options: [.drawsAfterEndingLocation])

    // The sun behind the head, with a soft glow that fades into the sky.
    NSGradient(colors: [sunGlow.withAlphaComponent(0.9), sunGlow.withAlphaComponent(0)])?
        .draw(fromCenter: NSPoint(x: 685, y: 275), radius: 100, toCenter: NSPoint(x: 685, y: 275), radius: 190, options: [])
    sun.setFill()
    NSBezierPath(ovalIn: NSRect(x: 575, y: 165, width: 220, height: 220)).fill()

    // Rolling hills.
    let farHill = NSBezierPath()
    farHill.move(to: NSPoint(x: 100, y: 700))
    farHill.curve(924, 660, 380, 610, 700, 600)
    farHill.line(to: NSPoint(x: 924, y: 924))
    farHill.line(to: NSPoint(x: 100, y: 924))
    farHill.close()
    hillFar.setFill()
    farHill.fill()

    let nearHill = NSBezierPath()
    nearHill.move(to: NSPoint(x: 100, y: 790))
    nearHill.curve(924, 770, 400, 740, 650, 745)
    nearHill.line(to: NSPoint(x: 924, y: 924))
    nearHill.line(to: NSPoint(x: 100, y: 924))
    nearHill.close()
    hill.setFill()
    nearHill.fill()

    // Ground shadow.
    color(0x000000, alpha: 0.22).setFill()
    NSBezierPath(ovalIn: NSRect(x: 240, y: 772, width: 470, height: 36)).fill()

    drawBrachiosaurus()
    NSGraphicsContext.restoreGraphicsState()
}

func drawBrachiosaurus() {
    let body = bodyPath()

    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = color(0x000000, alpha: 0.18)
    shadow.shadowBlurRadius = 18
    shadow.shadowOffset = NSSize(width: 0, height: -6)
    shadow.set()
    // One layer for the whole animal, so the shadow falls around its outline, not around each
    // overlapping part.
    let cg = NSGraphicsContext.current!.cgContext
    cg.beginTransparencyLayer(auxiliaryInfo: nil)

    // Far-side legs, a shade darker, behind the body.
    skinShade.setFill()
    leg(x: 304, top: 590, width: 62, bottom: 776).fill()
    leg(x: 540, top: 500, width: 62, bottom: 772).fill()

    skin.setFill()
    body.fill()

    // Near-side legs.
    leg(x: 340, top: 540, width: 72, bottom: 790).fill()
    leg(x: 588, top: 500, width: 72, bottom: 786).fill()
    cg.endTransparencyLayer()
    NSGraphicsContext.restoreGraphicsState()

    // Belly shading, without the drop shadow.
    NSGraphicsContext.saveGraphicsState()
    body.addClip()
    let belly = NSBezierPath()
    belly.move(to: NSPoint(x: 300, y: 600))
    belly.curve(690, 540, 420, 640, 600, 640)
    belly.line(to: NSPoint(x: 690, y: 700))
    belly.line(to: NSPoint(x: 300, y: 700))
    belly.close()
    skinShade.withAlphaComponent(0.6).setFill()
    belly.fill()
    NSGraphicsContext.restoreGraphicsState()

    drawGraph()

    // Eye and smile.
    ink.setFill()
    NSBezierPath(ovalIn: NSRect(x: 764, y: 216, width: 15, height: 15)).fill()
    let smile = NSBezierPath()
    smile.lineWidth = 5
    smile.lineCapStyle = .round
    smile.move(to: NSPoint(x: 786, y: 244))
    smile.curve(814, 238, 796, 250, 808, 246)
    ink.withAlphaComponent(0.7).setStroke()
    smile.stroke()
}

func png(size: Int) -> Data {
    let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    let context = NSGraphicsContext(bitmapImageRep: bitmap)!
    NSGraphicsContext.current = context
    context.imageInterpolation = .high
    // Scale the 1024 grid to `size`, with the origin at the top left.
    let transform = NSAffineTransform()
    transform.translateX(by: 0, yBy: CGFloat(size))
    transform.scaleX(by: CGFloat(size) / 1024, yBy: -CGFloat(size) / 1024)
    transform.concat()
    drawIcon()
    NSGraphicsContext.restoreGraphicsState()
    return bitmap.representation(using: .png, properties: [:])!
}

// MARK: Main

let problems = graphProblems(inside: bodyPath())
if !problems.isEmpty {
    print("The commit graph doesn't fit inside the body (margin \(Int(graphMargin)) px):")
    problems.forEach { print("  \($0)") }
    exit(1)
}

let arguments = CommandLine.arguments.dropFirst()
if let preview = arguments.first {
    try png(size: 1024).write(to: URL(fileURLWithPath: preview))
    print("Wrote \(preview)")
    exit(0)
}

let iconset = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("AppIcon.iconset")
try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let suffix = scale == 1 ? "" : "@2x"
        try png(size: points * scale).write(to: iconset.appendingPathComponent("icon_\(points)x\(points)\(suffix).png"))
    }
}
let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["--convert", "icns", "--output", "Resources/AppIcon.icns", iconset.path]
try iconutil.run()
iconutil.waitUntilExit()
print(iconutil.terminationStatus == 0 ? "Wrote Resources/AppIcon.icns" : "iconutil failed")
