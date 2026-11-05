// Draws the app icon at 1024 pt and saves it as a PNG. scripts/make_icon.sh turns it into an .icns.
// Usage: swift scripts/make_icon.swift <output.png>
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
    CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.displayP3), colors: colors as CFArray, locations: locations)!
}

// Apple's icon grid: an 824 pt rounded square centred in the canvas.
let inset: CGFloat = 100
let plate = CGRect(x: inset, y: inset, width: side - inset * 2, height: side - inset * 2)
let platePath = CGPath(roundedRect: plate, cornerWidth: 185, cornerHeight: 185, transform: nil)

context.saveGState()
context.addPath(platePath)
context.clip()
context.drawLinearGradient(
    gradient([color(0x1B2233), color(0x0B0E16)], [0, 1]),
    start: CGPoint(x: side / 2, y: side - inset), end: CGPoint(x: side / 2, y: inset), options: [])
context.drawRadialGradient(
    gradient([color(0x2DD4BF, 0.20), color(0x2DD4BF, 0)], [0, 1]),
    startCenter: CGPoint(x: side / 2, y: side / 2), startRadius: 0,
    endCenter: CGPoint(x: side / 2, y: side / 2), endRadius: 400, options: [])
context.restoreGState()

// Hairline along the plate's edge, brighter at the top.
context.saveGState()
context.addPath(platePath)
context.setLineWidth(4)
context.replacePathWithStrokedPath()
context.clip()
context.drawLinearGradient(
    gradient([color(0xFFFFFF, 0.22), color(0xFFFFFF, 0.04)], [0, 1]),
    start: CGPoint(x: side / 2, y: side - inset), end: CGPoint(x: side / 2, y: inset), options: [])
context.restoreGState()

let center = CGPoint(x: side / 2, y: side / 2)
let radius: CGFloat = 268
let width: CGFloat = 58

// Ring track.
context.setLineWidth(width)
context.setStrokeColor(color(0xFFFFFF, 0.07))
context.addArc(center: center, radius: radius, startAngle: 0, endAngle: .pi * 2, clockwise: false)
context.strokePath()

// Health ring, most of the way round, sweeping teal into violet.
let start = CGFloat.pi / 2
let end = start - CGFloat.pi * 2 * 0.82
let ring = CGMutablePath()
ring.addArc(center: center, radius: radius, startAngle: start, endAngle: end, clockwise: true)
context.saveGState()
context.addPath(ring)
context.setLineWidth(width)
context.setLineCap(.round)
context.replacePathWithStrokedPath()
context.clip()
context.drawLinearGradient(
    gradient([color(0x5EEAD4), color(0x2DD4BF), color(0xA78BFA)], [0, 0.45, 1]),
    start: CGPoint(x: center.x + radius, y: center.y + radius),
    end: CGPoint(x: center.x - radius, y: center.y - radius), options: [])
context.restoreGState()

// Pulse line across the middle.
let pulse = CGMutablePath()
let points: [(CGFloat, CGFloat)] = [
    (-178, 0), (-82, 0), (-44, 62), (6, -112), (54, 42), (84, 0), (178, 0),
]
pulse.move(to: CGPoint(x: center.x + points[0].0, y: center.y + points[0].1))
for point in points.dropFirst() {
    pulse.addLine(to: CGPoint(x: center.x + point.0, y: center.y + point.1))
}
context.setShadow(offset: .zero, blur: 28, color: color(0x5EEAD4, 0.55))
context.addPath(pulse)
context.setLineWidth(34)
context.setLineCap(.round)
context.setLineJoin(.round)
context.setStrokeColor(color(0xF4FFFD))
context.strokePath()

guard let image = context.makeImage(),
    let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
else { fatalError("no PNG") }
try png.write(to: URL(fileURLWithPath: output))
