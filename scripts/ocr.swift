// Text on the screen with exact boxes, via Apple's Vision framework. Usage: ocr <image> -> JSON lines.
// Boxes are 0..1000 of the image, origin top-left: {text, x, y, w, h}. Built by scripts/build-ocr.sh.
import AppKit
import Foundation
import Vision

guard CommandLine.arguments.count > 1,
      let img = NSImage(contentsOfFile: CommandLine.arguments[1]),
      let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil)
else { FileHandle.standardError.write(Data("cannot read image\n".utf8)); exit(1) }

let req = VNRecognizeTextRequest()
req.recognitionLevel = .accurate
req.usesLanguageCorrection = false  // terminals and code: don't "fix" what is actually there
try VNImageRequestHandler(cgImage: cg, options: [:]).perform([req])

struct Line { let text: String; let x: Double; let y: Double; let w: Double; let h: Double }
var lines: [Line] = []
for o in req.results ?? [] {
    guard let c = o.topCandidates(1).first else { continue }
    let b = o.boundingBox  // normalized, origin bottom-left
    lines.append(Line(text: c.string, x: Double(b.minX) * 1000, y: Double(1 - b.maxY) * 1000, w: Double(b.width) * 1000, h: Double(b.height) * 1000))
}
lines.sort { ($0.y, $0.x) < ($1.y, $1.x) }
let rows = lines.map { ["text": $0.text, "x": $0.x, "y": $0.y, "w": $0.w, "h": $0.h] as [String: Any] }
print(String(data: try JSONSerialization.data(withJSONObject: rows), encoding: .utf8)!)
