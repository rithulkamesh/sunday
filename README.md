# Sunday

**S.U.N.D.A.Y.** — **S**ees · **U**nderstands · **N**otices · **D**ecides · **A**cts · **Y**ours

Tony Stark has F.R.I.D.A.Y.; this is the one that lives on your own computer and answers only to you. A local-first personal AI agent that teaches you
almost anything, looks at your screen and points at things, reads the web, and remembers what matters. Nothing leaves your machine unless you switch the web on:
Silero VAD + a wake-word spotter + Parakeet (speech in), Ollama (thinking), Kokoro (speech out), SQLite (memory).

Honest status: *Sees* (screen, diagrams), *Understands* (teaching, notes), *Notices* (memory, voice) and *Decides* (typed actions, rules router) exist today.
*Acts* has started: opening apps, pages and files and typing, with approvals; clicking and multi-app workflows are next. The long-term goal is a general agent that gets more capable the more it's used.

## Run

```sh
scripts/fetch-models.sh     # once: speech models into ~/.cache/sunday/models
scripts/build-ocr.sh        # once, and again after updates: screen helper (exact text boxes, window list, cropping); needs Xcode command line tools
ollama pull qwen3:4b-instruct && ollama pull nomic-embed-text-v2-moe
ollama pull qwen3-vl:4b-instruct   # only for diagrams and other non-text screens
bun install && bun run tauri dev
```

Say "sunday" or hold fn. Then talk, or type in the workspace.

## What it does

**Teaching anything.** "Teach me how to play guitar", "I want to learn linear algebra", "teach me Spanish". The first time, the model drafts a learning plan
for the subject (6 to 10 concepts in order, each with what it builds on); then it teaches the next concept: a short explanation and a check question. Ask again to continue.
Tried on guitar, linear algebra, sourdough, the Rust borrow checker and conversational Spanish.

**Practice.** "Quiz me", "quiz me on barre chords", "next question". One question you can answer out loud; your next answer is graded against a reference and stored with an error type.
Each concept gets a review date (1 day, 3 days, then growing; a miss resets it). With no topic it picks what's due, then your weakest concept, then the next unlocked one in any plan.

**Learning map.** Every plan joins one graph of concepts and prerequisites. A concept is *known* after two correct answers in a row; a wrong answer on a concept with shaky prerequisites points you at the gap.

**Notes.** Import PDF / Markdown / text (Knowledge view, drag-drop, or ⌘K). Passages are indexed locally (SQLite FTS5).
Chat answers quote them ("from your notes on …") and show the sources; quizzes on a concept draw on them. PDFs need `brew install poppler`.

**Screen.** "Explain what's on my screen", "what is this question", "point to …": takes one screenshot, a local vision model
explains it, Sunday speaks, and boxes/circles/arrows are drawn on a click-through overlay for 25 s ("clear the screen" removes them).
It knows what's going on, not just what's visible: which app and window is in front and what else is open (from the macOS window list, no extra permission), and "explain this page / window / PDF" looks at just the front window.
Pictures and diagrams (screens with a few scattered labels, or visual questions) go to the vision model in one pass; it answers in two short sentences and marks what it mentions, and rough boxes are snapped to the text under them. Ordinary text screens skip the image and answer from the on-screen text, which is much faster.
Pointing at text is exact: macOS's text recogniser (`scripts/ocr.swift`) finds every line's box, the model just picks line numbers. Only for non-text things (diagrams, icons) does it use the model's own coordinates, which are approximate. Only on request; the image is read once, sent to local Ollama only, and deleted. Needs Screen Recording permission. A text screen takes ~8 s, a picture ~10 to 20 s.

**Memory and the web.** Long-term memory is a store of atomic facts, each with where it came from (you told me / read on a page / inferred from a conversation),
its confidence, and when it refers to; recalled by meaning (local embeddings) and cited in answers. The design follows Devsper's memory service (`devsper/flow/svc/memory`).
"Remember that …" stores what you said. With **web access switched on** (Settings; off by default): "learn this page" reads the tab in your front browser,
"read https://…" reads a link, "research X" searches the web, reads the top results in parallel and answers with sources. Pages are fetched safely (public addresses only,
re-checked on every redirect, size-capped), with Readability for the main text and headless Chrome for JavaScript-only pages. What it reads is extracted into facts in the background.
When you say goodbye, lasting facts about you are saved as unconfirmed guesses. All of it is listed in Memory and deletable per fact or per page.

**Skills.** Versioned, prompt-only capabilities (a template, an output schema, tests). No generated code, no tools.
Lifecycle: proposed → experimental → approved; approval needs a passing test run on that version; any approved version can be rolled back.
Describe one in the Skills view and the model drafts it. Voice: "run skill learn.mnemonic on the OSI model".

**Acting.** "Open Spotify", "pull up images of a keyboard", "go to github.com", "type …". The model picks one typed step at a time (`agent.rs`): open an app, open a page (in the browser you already have open), open a file in your home folder, type into the front app, web search, look at the screen, recall. Opening an app or page just happens when you asked for it directly; typing and opening files always ask "…?" and wait for a yes; and once anything read from the web, the screen or memory is in play, everything asks (read content is data, never a command). Hidden folders, non-http addresses and anything outside your home folder are refused. Every step is logged in the `actions` table. Typing needs Accessibility permission.

**Decisions.** Every utterance becomes one typed `Action` (`decision.rs`) before anything runs, and is checked against what's
permitted. `Rules` is the deterministic router; the `Decider` trait is where a smarter router goes.

**⌘K** command palette; **Settings** has the model picker and a diagnostics checklist.

**Your voice.** "Learn my voice" (say it, or Settings / ⌘K) records ~20 s of you, builds a voiceprint (wespeaker ResNet34, on-device) and from then on Sunday ignores other voices: wake word, commands and talking-over-it all have to match you. The fn key always works. Logs show `voice match 0.xx` so the threshold (`speaker::SAME`) can be tuned. "Forget my voice" removes it.

Voice details: "Sunday" or fn wakes it; say "stop", talk over it, or press fn to interrupt; "bye" ends the session and unloads the speech model.

## Models

Defaults: `qwen3:4b-instruct` (text), `qwen3-vl:4b-instruct` (screen, ~10 s per look once warm) and `nomic-embed-text-v2-moe` (memory search). Use the `-instruct` builds: plain `qwen3:4b` and `qwen3-vl:4b` are thinking builds that ignore `think:false` and leak their reasoning into replies. Override with `SUNDAY_MODEL` / `SUNDAY_VISION` / `SUNDAY_EMBED`, or pick in Settings / ⌘K.
The plain `qwen3-vl:4b` is a thinking model and roughly twice as slow; Sunday can read either.

## Not built yet

Held-out evaluation and delayed-retention tests (grading is LLM-as-judge, with no answer keys), a router model (Laya or otherwise),
a tool gateway and approvals, taking actions on your computer, sync, the floating-orb polish.

## License

GPL-3.0-or-later. Forks and derivatives must stay open source under the same license. See [LICENSE](LICENSE).
