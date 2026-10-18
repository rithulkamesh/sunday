# Getting started

## Requirements

- **macOS** (Apple Silicon recommended). Windows and Linux are on the roadmap, see [#21](https://github.com/rithulkamesh/sunday/issues/21).
- [Bun](https://bun.sh), [Rust](https://rustup.rs) (stable), Xcode command line tools (`xcode-select --install`).
- [Ollama](https://ollama.com), running locally.
- Optional: `brew install poppler` to import PDFs.

## Install and run

```sh
git clone git@github.com:rithulkamesh/sunday.git && cd sunday
scripts/fetch-models.sh      # Silero VAD, wake word, Parakeet, Kokoro, voiceprint -> ~/.cache/sunday/models
scripts/build-ocr.sh         # compiles scripts/ocr.swift (rerun after updates)
ollama pull qwen3:4b-instruct
ollama pull nomic-embed-text-v2-moe
ollama pull qwen3-vl:4b-instruct   # only for diagrams and non-text screens
bun install
bun run tauri dev
```

Use the `-instruct` builds. Plain `qwen3:4b` and `qwen3-vl:4b` are thinking builds and leak reasoning into replies.
Override models with `SUNDAY_MODEL`, `SUNDAY_VISION`, `SUNDAY_EMBED`, or pick them in Settings / ⌘K.

### Other model providers

Ollama is the default and keeps everything on your machine. To use a hosted model for text and chat, set `SUNDAY_PROVIDER` and `SUNDAY_MODEL` before launching. **Your prompts, including any notes or memory pulled into them, are then sent to that provider.** Embeddings and the vision model still use Ollama.

| `SUNDAY_PROVIDER` | Key | Notes |
|---|---|---|
| `openai` | `OPENAI_API_KEY` (optional for local servers) | `OPENAI_BASE_URL` for OpenRouter, vLLM, LM Studio, llama.cpp server |
| `anthropic` | `ANTHROPIC_API_KEY` | e.g. `SUNDAY_MODEL=claude-sonnet-5-5` |
| `gemini` | `GEMINI_API_KEY` | via Google's OpenAI-compatible endpoint |
| `nova` | `AWS_BEARER_TOKEN_BEDROCK` | Bedrock API key; `AWS_REGION` (default `us-east-1`); no token streaming yet |

Check a provider end to end with `SUNDAY_PROVIDER_TEST=1 cargo test conformance::live` (spends tokens).

## Permissions (macOS)

Grant these when prompted, or in System Settings → Privacy & Security:

| Permission | Needed for |
|---|---|
| Microphone | everything voice |
| Screen Recording | "explain my screen", "point to…" |
| Accessibility | typing and clicking on your behalf |

Settings has a diagnostics checklist that tells you what's missing.

## First five minutes

1. Say "sunday" (or hold fn) and ask: *"What's the capital of Australia?"* to check speech in and out.
2. Type or say *"Teach me how to play guitar"*, then *"quiz me"*.
3. Open any page and say *"explain what's on my screen"*.
4. Say *"Remember that my dog is called Pixel"*, end with *"bye"*, start again and ask her name.
5. *"Open Spotify"* or *"go to github.com"* to see the action layer. Typing and file opens will ask for a yes.
6. Settings → switch web access on, then try *"research how sourdough starters work"*.

Say "learn my voice" so Sunday ignores everyone else.

## Where to start hacking

| You want to… | Start in |
|---|---|
| Change how requests are routed | `src-tauri/src/decision.rs` (`Action`, `Rules`, `Decider`) |
| Add or change something Sunday can *do* | `src-tauri/src/agent.rs` (`TOOLS`, `Step`, `check`, `exec`) |
| Change prompts, turn handling, the model call | `src-tauri/src/llm.rs` |
| Memory, facts, embeddings | `src-tauri/src/memory.rs`, `db.rs` |
| Skills | `src-tauri/src/skills.rs` |
| Screen, OCR, overlay | `src-tauri/src/screen.rs`, `scripts/ocr.swift` |
| UI | `src/*.tsx` |

Full module map: [REPO_MAP.md](REPO_MAP.md).

## Good first contributions

- Pick an issue labelled [`good first issue`](https://github.com/rithulkamesh/sunday/labels/good%20first%20issue).
- Add an eval task once the harness lands ([#2](https://github.com/rithulkamesh/sunday/issues/2)).
- Port a platform capability to Windows or Linux ([#21](https://github.com/rithulkamesh/sunday/issues/21)).

## Troubleshooting

- **No reply / "model not found"**: is Ollama running (`ollama list`)? Did you pull the `-instruct` model?
- **Screen explain returns nothing**: Screen Recording permission, then restart the app.
- **Typing does nothing**: Accessibility permission.
- **Reasoning text in replies**: you pulled a thinking build; use the `-instruct` one.
- **PDF import fails**: `brew install poppler`.
