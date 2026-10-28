---
id: getting-started
slug: /
title: Get started
---

# Getting started

## Requirements

- **macOS** (Apple Silicon recommended). Windows and Linux are on the roadmap, see [#21](https://github.com/rithulkamesh/sunday/issues/21).
- [Bun](https://bun.sh), [Rust](https://rustup.rs) (stable), Xcode (the full app, not only the command line tools: it has the Metal compiler that MLX needs; skip MLX with `--no-default-features --features voice`).
- A model provider: [Ollama](https://ollama.com) (default), [LM Studio](https://lmstudio.ai), or MLX on Apple silicon. See [Providers](PROVIDERS.md).
- Optional: `brew install poppler` to import PDFs.

## Install a release

On a Mac with Apple silicon, this installs the latest release, fetches the models and needs no build tools:

```sh
curl -fsSL https://raw.githubusercontent.com/rithulkamesh/sunday/main/scripts/install.sh | bash
```

It checks the download's checksum, clears Gatekeeper's quarantine flag and signs the app ad hoc on your machine (`codesign -s -`), because releases are not notarized with an Apple Developer ID. Models go to `~/.cache/sunday`; `SUNDAY_SKIP_MODELS=1` skips them. You still need a model provider ([Providers](PROVIDERS.md)). To build from source instead:

## Install and run (from source)

```sh
git clone git@github.com:rithulkamesh/sunday.git && cd sunday
scripts/setup.sh             # models, OCR helper, routing model, bun install, and your Ollama models if it is running
bun run tauri dev
```

`setup.sh` is safe to rerun. Add `--vision` to also pull the vision model (diagrams and non-text screens), or `--no-router` to skip the optional routing model. It runs these, which you can also run one by one: `scripts/fetch-models.sh` (Silero VAD, wake word, Parakeet, Kokoro, voiceprint into `~/.cache/sunday/models`), `scripts/build-ocr.sh`, `scripts/fetch-router.sh` ([Router](ROUTER.md#install-it)), `ollama pull qwen3:4b-instruct`, `ollama pull nomic-embed-text-v2-moe`, `ollama pull qwen3-vl:4b-instruct`, `bun install`.

To check your setup, `cd src-tauri && cargo test` needs nothing installed and takes about a second; `bun run test:live` runs the tests that need Ollama and the models ([Testing](TESTING.md)).

Use the `-instruct` builds. Plain `qwen3:4b` and `qwen3-vl:4b` are thinking builds and leak reasoning into replies.
Override models with `SUNDAY_MODEL`, `SUNDAY_VISION`, `SUNDAY_EMBED`, or pick them in Settings / ⌘K.

### Other model providers

Ollama is the default and keeps everything on your machine. To use a hosted model for chat or planning, see [Providers](PROVIDERS.md).

## Permissions (macOS)

Grant these when prompted, or in System Settings → Privacy & Security:

| Permission | Needed for |
|---|---|
| Microphone | everything voice |
| Screen Recording | "explain my screen", "point to…" |
| Accessibility | typing and clicking on your behalf |

Settings has a diagnostics checklist that tells you what's missing. If you ask Sunday to type or click without Accessibility permission, it opens the right page in System Settings and tells you what to turn on. When you run from a terminal with `bun run tauri dev`, macOS asks for it on behalf of that terminal.

## First five minutes

1. Say "sunday" (or hold fn) and ask: *"What's the capital of Australia?"* to check speech in and out.
2. Type or say *"Teach me how to play guitar"*, then *"quiz me"*.
3. Open any page and say *"explain what's on my screen"*.
4. Say *"Remember that my dog is called Pixel"*, end with *"bye"*, start again and ask her name.
5. *"Open Spotify"* or *"go to github.com"* to see the action layer. *"Open Notes and type hello"* opens Notes, starts a new note and types your words without asking; typing anything else, clicking and opening files will ask for a yes. Then try a follow-up like *"sort by the best"* on a YouTube results page.
6. Settings → switch web access on, then try *"research how sourdough starters work"*.

Say "learn my voice" so Sunday ignores everyone else. Talk over it any time to interrupt.

Want a different voice or pace? Start with `SUNDAY_VOICE=8 SUNDAY_VOICE_SPEED=1.1 bun run tauri dev` (ids are listed in [Features](FEATURES.md)).

## Where to start hacking

| You want to… | Start in |
|---|---|
| Change how requests are routed | `src-tauri/src/brain/decision.rs` (`Action`, `Rules`), `brain/laya.rs` and `brain/corrections.rs`; see [Router](ROUTER.md) |
| Add or change something Sunday can *do* | `src-tauri/src/tools/agent.rs` (`TOOLS`, `POLICY`, `check`, `exec`, `verify`), and the OS call in `src-tauri/src/platform/` |
| Change prompts, turn handling, the model call | `src-tauri/src/brain/llm.rs` |
| Memory, facts, embeddings | `src-tauri/src/store/memory.rs`, `store/db.rs` |
| Skills | `src-tauri/src/brain/skills.rs` |
| Screen, OCR, overlay | `src-tauri/src/tools/screen.rs`, `scripts/ocr.swift` |
| UI | `src/*.tsx` |

Full module map: [REPO_MAP.md](REPO_MAP.md). How the pieces behave together: [Architecture](ARCHITECTURE.md).

## Good first contributions

- Pick an issue labelled [`good first issue`](https://github.com/rithulkamesh/sunday/labels/good%20first%20issue).
- Add an eval task ([#2](https://github.com/rithulkamesh/sunday/issues/2)): one JSON line, see [Evals](EVALS.md).
- Port a platform capability to Windows or Linux ([#21](https://github.com/rithulkamesh/sunday/issues/21)).

## Troubleshooting

- **No reply / "model not found"**: is Ollama running (`ollama list`)? Did you pull the `-instruct` model?
- **Screen explain returns nothing**: Screen Recording permission, then restart the app.
- **Typing does nothing**: Accessibility permission (for the terminal you started it from, in development).
- **"I couldn't get this to work"**: the agent tried three times and each failed or could not be verified. Say what you want more directly ("open Notes", "click the Send button"), or check the permission above.
- **Build fails to link with missing `_anon` symbols**: two builds shared `target/` (for example `tauri dev` and `cargo test`). Stop the dev server and `cargo clean -p sunday`.
- **Reasoning text in replies**: you pulled a thinking build; use the `-instruct` one.
- **PDF import fails**: `brew install poppler`.
