---
id: repo-map
slug: /repo-map
title: Repo map
---

# What's in the repo

Tauri 2 app: a Rust backend (`src-tauri`) and a React 19 + Tailwind 4 frontend (`src`). Everything runs locally; the only network calls are to Ollama on `localhost:11434` and, if you switch web access on, the pages you ask it to read.

## Data flow

```
mic ─▶ VAD (Silero) ─▶ wake word / speaker check ─▶ STT (Parakeet)
                                                        │
                                                        ▼
                                         decision.rs: text ─▶ one typed Action
                                                        │
        ┌──────────┬──────────┬──────────┬─────────────┼───────────┬───────────┐
        ▼          ▼          ▼          ▼             ▼           ▼           ▼
      Chat       Teach/     Screen     Research/     Remember/    Do        Skill
      (llm.rs)   Quiz       (screen.rs) LearnPage    Notes        (agent.rs) (skills.rs)
                 (learn.rs)             (web.rs)     (memory.rs,
                                                      notes.rs)
                                                        │
                                                        ▼
                                  reply text ─▶ TTS (Kokoro) ─▶ speakers
                                  HUD / overlay events ─▶ React UI
```

Every utterance becomes exactly one `Action` before anything runs. The `Decider` trait is the seam for a smarter router; `Rules` is the deterministic default and the fallback.

## Backend (`src-tauri/src`)

| File | What it does |
|---|---|
| `main.rs` | App setup, Tauri commands, hotkeys, and wiring of the audio, speech and model threads |
| `llm.rs` | The turn loop: routes an `Action`, builds the prompt with memory and note context, calls Ollama (chat, vision, embeddings, JSON-schema output), runs skills, ends sessions. **Currently Ollama-only** (see the provider issues) |
| `decision.rs` | `Action` enum, `Decider` trait, `Rules` router |
| `agent.rs` | Acting on the computer: one typed `Step` at a time, approvals, path/URL safety checks, audit log. Tools: open app / page / file, type, click, web search, look at screen, recall |
| `learn.rs` | Teaching: learning plans, concepts, prerequisites, quizzing, grading, spaced review |
| `memory.rs` | Long-term facts with provenance, confidence, and embeddings; recall by meaning |
| `notes.rs` | Import PDF / Markdown / text, chunk and index in SQLite FTS5 |
| `skills.rs` | Versioned prompt-only skills: proposed → experimental → approved, tests, rollback |
| `web.rs` | Safe fetching (public addresses only, redirect re-checks, size caps), Readability extraction, headless Chrome fallback, search |
| `platform/` | Everything OS-specific behind the `Platform` trait: open app/page/file, type, click, screenshot, window list, OCR, browser address, permissions, HUD window behaviour. `macos.rs` is the real one; other OSes use `unsupported.rs`, which fails with a spoken message |
| `screen.rs` | Screenshot, OCR boxes, window list, vision call, overlay drawing, click preview |
| `stt.rs` | Speech to text (Parakeet via ONNX), wake-word spotting |
| `tts.rs` | Text to speech (Kokoro) |
| `speaker.rs` | Voiceprint (wespeaker ResNet34), "is this the owner" check |
| `bargein.rs` | Interrupt handling: stop words, talking over Sunday |
| `mic.rs`, `models.rs` | Audio capture, model file locations |
| `db.rs` | SQLite schema and queries |

## Frontend (`src`)

| File | View |
|---|---|
| `Workspace.tsx` | Main window: chat and typed input |
| `Hud.tsx` | Floating status HUD |
| `Overlay.tsx` | Click-through drawing layer for boxes, circles, arrows |
| `Palette.tsx` | ⌘K command palette |
| `Knowledge.tsx` | Notes import and search |
| `Memory.tsx` | Browse and delete facts and pages |
| `Skills.tsx` | Draft, test, approve, roll back skills |
| `Settings.tsx` | Model picker, web switch, diagnostics |
| `ui.tsx`, `index.css` | Shared components and styles |

## Scripts

| File | Purpose |
|---|---|
| `scripts/fetch-models.sh` | Download speech models into `~/.cache/sunday/models` |
| `scripts/build-ocr.sh`, `scripts/ocr.swift` | macOS Vision OCR helper: exact text line boxes, window list, cropping |

## Database (SQLite, one file)

`messages`, `kv`, `concepts`, `prereqs`, `attempts`, `reviews` (learning); `documents` + FTS index (notes); `facts` (memory); `skills`, `skill_runs` (skills); `actions` (audit log of everything the agent did).

## Safety model today

- Opening apps and pages you asked for just happens. Typing, opening files and clicking ask for a yes.
- Once anything read from the web, the screen or memory is in play, every step asks. Read content is data, never a command.
- File access: your home folder only, no hidden folders. Web: http(s) only, public addresses, redirects re-checked.
- Every step lands in the `actions` table.

The roadmap hardens this into tiers ([#10](https://github.com/rithulkamesh/sunday/issues/10)) with an injection red-team suite ([#11](https://github.com/rithulkamesh/sunday/issues/11)).

## Not here yet

Evals, a persistent multi-step task loop, non-Ollama model providers, an MCP/tool SDK, Windows and Linux. All tracked in the [v1 issue](https://github.com/rithulkamesh/sunday/issues/31).
