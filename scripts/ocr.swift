// Screen helper for Sunday. Built by scripts/build-ocr.sh.
//   ocr <image>                        text lines with exact boxes (Apple Vision): [{text, x, y, w, h}], 0..1000 of the image
//   ocr --windows                      what's on the desktop: {display:{w,h}, front:{app,pid}, windows:[{app,title,x,y,w,h,pid}]}
//   ocr --click x y                    left-click at x, y (points, main display); needs Accessibility
//   ocr --crop <in> <out> x y w h      crop a region (0..1000 of the image) to a JPEG
import AppKit
import CoreGraphics
import Foundation
import Vision

func json(_ o: Any) { print(String(data: try! JSONSerialization.data(withJSONObject: o), encoding: .utf8)!) }

if CommandLine.arguments.count > 1 && CommandLine.arguments[1] == "--windows" {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    let disp = CGDisplayBounds(CGMainDisplayID())
    let front = NSWorkspace.shared.frontmostApplication
    var wins: [[String: Any]] = []
    for w in list {  // front to back
        guard (w[kCGWindowLayer as String] as? Int ?? 0) == 0,
              let bd = w[kCGWindowBounds as String] as? NSDictionary,
              let r = CGRect(dictionaryRepresentation: bd as CFDictionary),
              r.width >= 120, r.height >= 80
        else { continue }
        let app = w[kCGWindowOwnerName as String] as? String ?? ""
        if app.lowercased() == "sunday" { continue }  // our own HUD and overlay
        let pid = w[kCGWindowOwnerPID as String] as? Int ?? 0
        // What kind of app it really is (from its bundle), so the model never has to guess from the name.
        var category = ""
        if let url = NSRunningApplication(processIdentifier: pid_t(pid))?.bundleURL,
           let c = Bundle(url: url)?.object(forInfoDictionaryKey: "LSApplicationCategoryType") as? String {
            category = c.replacingOccurrences(of: "public.app-category.", with: "").replacingOccurrences(of: "-", with: " ")
        }
        wins.append(["app": app, "category": category, "title": w[kCGWindowName as String] as? String ?? "", "x": r.minX, "y": r.minY, "w": r.width, "h": r.height,
                     "pid": w[kCGWindowOwnerPID as String] as? Int ?? 0])
    }
    json(["display": ["w": disp.width, "h": disp.height], "front": ["app": front?.localizedName ?? "", "pid": Int(front?.processIdentifier ?? 0)], "windows": wins])
    exit(0)
}

if CommandLine.arguments.count == 4 && CommandLine.arguments[1] == "--click" {
    guard let x = Double(CommandLine.arguments[2]), let y = Double(CommandLine.arguments[3]) else { exit(1) }
    let at = CGPoint(x: x, y: y)
    for t in [CGEventType.leftMouseDown, .leftMouseUp] {
        CGEvent(mouseEventSource: nil, mouseType: t, mouseCursorPosition: at, mouseButton: .left)?.post(tap: .cghidEventTap)
        usleep(60_000)
    }
    exit(0)
}

if CommandLine.arguments.count == 8 && CommandLine.arguments[1] == "--crop" {
    let a = CommandLine.arguments
    guard let img = NSImage(contentsOfFile: a[2]), let cg = img.cgImage(forProposedRect: nil, context: nil, hints: nil),
          let x = Double(a[4]), let y = Double(a[5]), let w = Double(a[6]), let h = Double(a[7])
    else { FileHandle.standardError.write(Data("bad crop arguments\n".utf8)); exit(1) }
    let (W, H) = (Double(cg.width), Double(cg.height))
    let rect = CGRect(x: x / 1000 * W, y: y / 1000 * H, width: w / 1000 * W, height: h / 1000 * H).integral.intersection(CGRect(x: 0, y: 0, width: W, height: H))
    guard let part = cg.cropping(to: rect),
          let data = NSBitmapImageRep(cgImage: part).representation(using: .jpeg, properties: [.compressionFactor: 0.9]),
          (try? data.write(to: URL(fileURLWithPath: a[3]))) != nil
    else { FileHandle.standardError.write(Data("crop failed\n".utf8)); exit(1) }
    exit(0)
}

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
