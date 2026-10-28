// Apple's on-device model (Foundation Models, macOS 26+) for Sunday's chat. Reads {"system","prompt"} as JSON on
// stdin and streams the reply as plain text on stdout. `applefm check` prints "ok" or why it can't run.
import Foundation
import FoundationModels

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write(Data((msg + "\n").utf8))
    exit(1)
}

let model = SystemLanguageModel.default
if case .unavailable(let why) = model.availability { fail("unavailable: \(why)") }
if CommandLine.arguments.dropFirst().first == "check" { print("ok"); exit(0) }

guard let input = try? JSONSerialization.jsonObject(with: FileHandle.standardInput.readDataToEndOfFile()) as? [String: String],
      let prompt = input["prompt"] else { fail("bad input") }

let session = LanguageModelSession(model: model, instructions: input["system"] ?? "")
do {
    // streamResponse yields the reply so far each time; print only what is new.
    var sent = ""
    for try await partial in session.streamResponse(to: prompt) {
        let text = partial.content
        if text.hasPrefix(sent) { print(text.dropFirst(sent.count), terminator: "") } else { print(text, terminator: "") }
        sent = text
        fflush(stdout)
    }
} catch {
    fail("\(error)")
}
