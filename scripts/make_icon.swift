// Draws the app icon at 1024 pt and saves it as a PNG. scripts/make_icon.sh turns it into an .icns.
// Usage: swift scripts/make_icon.swift <output.png>
//
// Follows docs/DESIGN.md: a neutral graphite plate, and colour only where it reports state. The
// ring is the health score, drawn in the same system green the app uses for "fine"; the pulse line
// is the Mac. Flat strokes, no glow, no gradient on the marks: it has to read at 16 px in a list.
import AppKit

let side: CGFloat = 1024
let output = CommandLine.arguments[1]

guard
    let context = CGContext(
        data: nil, width: Int(side), height: Int(side), bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.displayP3)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
else { fatalError("no CGContext") }

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255, alpha: alpha)
}

func gradient(_ colors: [CGColor], _ locations: [CGFloat]) -> CGGradient {
    CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.displayP3), colors: colors as CFArray,
        locations: locations)!
}

// Apple's icon grid: an 824 pt rounded square centred in the canvas.
let inset: CGFloat = 100
let plate = CGRect(x: inset, y: inset, width: side - inset * 2, height: side - inset * 2)
let platePath = CGPath(roundedRect: plate, cornerWidth: 185, cornerHeight: 185, transform: nil)

// The plate sits on the soft, neutral shadow macOS icons carry.
context.saveGState()
context.setShadow(offset: CGSize(width: 0, height: -14), blur: 36, color: color(0x000000, 0.32))
context.addPath(platePath)
context.setFillColor(color(0x1C1C1E))
context.fillPath()
context.restoreGState()

// Graphite, a shade lighter at the top, like the app's dark sidebar under a light from above.
context.saveGState()
context.addPath(platePath)
context.clip()
context.drawLinearGradient(
    gradient([color(0x3A3A3C), color(0x1C1C1E)], [0, 1]),
    start: CGPoint(x: side / 2, y: side - inset), end: CGPoint(x: side / 2, y: inset), options: [])
context.restoreGState()

// Hairline along the edge, brighter at the top, so the plate holds its shape on a dark Dock.
context.saveGState()
context.addPath(platePath)
context.setLineWidth(4)
context.replacePathWithStrokedPath()
context.clip()
context.drawLinearGradient(
    gradient([color(0xFFFFFF, 0.20), color(0xFFFFFF, 0.04)], [0, 1]),
    start: CGPoint(x: side / 2, y: side - inset), end: CGPoint(x: side / 2, y: inset), options: [])
context.restoreGState()

let center = CGPoint(x: side / 2, y: side / 2)
let radius: CGFloat = 262
let width: CGFloat = 56

// Ring track.
context.setLineWidth(width)
context.setStrokeColor(color(0xFFFFFF, 0.10))
context.addArc(center: center, radius: radius, startAngle: 0, endAngle: .pi * 2, clockwise: false)
context.strokePath()

// The score: three quarters of the way round, clockwise from the top, in system green.
let start = CGFloat.pi / 2
let end = start - CGFloat.pi * 2 * 0.77
let ring = CGMutablePath()
ring.addArc(center: center, radius: radius, startAngle: start, endAngle: end, clockwise: true)
context.addPath(ring)
context.setLineWidth(width)
context.setLineCap(.round)
context.setStrokeColor(color(0x30D158))
context.strokePath()

// Pulse line across the middle.
let pulse = CGMutablePath()
let points: [(CGFloat, CGFloat)] = [
    (-170, 0), (-80, 0), (-44, 58), (4, -108), (50, 40), (80, 0), (170, 0),
]
pulse.move(to: CGPoint(x: center.x + points[0].0, y: center.y + points[0].1))
for point in points.dropFirst() {
    pulse.addLine(to: CGPoint(x: center.x + point.0, y: center.y + point.1))
}
context.addPath(pulse)
context.setLineWidth(34)
context.setLineCap(.round)
context.setLineJoin(.round)
context.setStrokeColor(color(0xF2F2F7))
context.strokePath()

guard let image = context.makeImage(),
    let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
else { fatalError("no PNG") }
try png.write(to: URL(fileURLWithPath: output))
