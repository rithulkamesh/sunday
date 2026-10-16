# Sunday

Local-first voice tutor, starting with GATE prep. Everything runs on your machine:
Silero VAD + wake-word spotter + Parakeet (speech in), Ollama (thinking), Kokoro (speech out), SQLite (memory).

## Run

```sh
scripts/fetch-models.sh     # once: speech models into ~/.cache/sunday/models
scripts/build-ocr.sh        # once, and again after updates: screen helper (exact text boxes, window list, cropping); needs Xcode command line tools
ollama pull gemma4          # or: SUNDAY_MODEL=<any ollama model>
bun install && bun run tauri dev
```

Say "sunday" or hold fn. Then talk, or type in the workspace.

## What it does

**Study loop.** "Quiz me", "quiz me on graphs", "next question". One spoken-friendly question; your next answer is graded
against a reference and stored with an error type. Each concept gets a review date (1 day, 3 days, then growing; a miss resets it).
With no topic it picks: what's due, then your weakest concept, then the next unlocked concept on the syllabus.

**Concept graph.** `src-tauri/domains/gate.txt` is the GATE (CS&IT + Data Science) syllabus with prerequisites, one concept per line
(`concept < prereq, prereq`). A concept is *known* after two correct answers in a row; a wrong answer on a concept with shaky
prerequisites points you at the gap. Add your own packs in the same format.

**Notes.** Import PDF / Markdown / text (Knowledge view, drag-drop, or ⌘K). Passages are indexed locally (SQLite FTS5).
Chat answers quote them ("from your notes on …") and show the sources; quizzes on a concept draw on them. PDFs need `brew install poppler`.

**Screen.** "Explain what's on my screen", "what is this question", "point to …": takes one screenshot, a local vision model
explains it, Sunday speaks, and boxes/circles/arrows are drawn on a click-through overlay for 25 s ("clear the screen" removes them).
It knows what's going on, not just what's visible: which app and window is in front and what else is open (from the macOS window list, no extra permission), and "explain this page / window / PDF" looks at just the front window.
Diagrams get a closer look: if the first pass sees a diagram, chart or figure it says so, crops that region at full resolution, reads its labels, and explains how the parts connect, drawing arrows between them ("Client → Load Balancer → Server A").
Pointing at text is exact: macOS's text recogniser (`scripts/ocr.swift`) finds every line's box, the model just picks line numbers. Only for non-text things (diagrams, icons) does it use the model's own coordinates, which are approximate. Only on request; the image is read once, sent to local Ollama only, and deleted. Needs Screen Recording permission. A look takes ~25 s; a diagram with a closer look, up to a minute.

**Memory and the web.** Long-term memory is a store of atomic facts, each with where it came from (you told me / read on a page / inferred from a conversation),
its confidence, and when it refers to; recalled by meaning (local embeddings) and cited in answers. The design follows Devsper's memory service (`devsper/flow/svc/memory`).
"Remember that …" stores what you said. With **web access switched on** (Settings; off by default): "learn this page" reads the tab in your front browser,
"read https://…" reads a link, "research X" searches the web, reads the top results in parallel and answers with sources. Pages are fetched safely (public addresses only,
re-checked on every redirect, size-capped), with Readability for the main text and headless Chrome for JavaScript-only pages. What it reads is extracted into facts in the background.
When you say goodbye, lasting facts about you are saved as unconfirmed guesses. All of it is listed in Memory and deletable per fact or per page.

**Skills.** Versioned, prompt-only capabilities (a template, an output schema, tests). No generated code, no tools.
Lifecycle: proposed → experimental → approved; approval needs a passing test run on that version; any approved version can be rolled back.
Describe one in the Skills view and the model drafts it. Voice: "run skill study.mnemonic on the OSI model".

**Decisions.** Every utterance becomes one typed `Action` (`decision.rs`) before anything runs, and is checked against what's
permitted. `Rules` is the deterministic router; the `Decider` trait is where a smarter router goes.

**⌘K** command palette; **Settings** has the model picker and a diagnostics checklist.

**Your voice.** "Learn my voice" (say it, or Settings / ⌘K) records ~20 s of you, builds a voiceprint (wespeaker ResNet34, on-device) and from then on Sunday ignores other voices: wake word, commands and talking-over-it all have to match you. The fn key always works. Logs show `voice match 0.xx` so the threshold (`speaker::SAME`) can be tuned. "Forget my voice" removes it.

Voice details: "Sunday" or fn wakes it; say "stop", talk over it, or press fn to interrupt; "bye" ends the session and unloads the speech model.

## Models

Defaults: `qwen3:4b-instruct` (text), `qwen3-vl:4b-instruct` (screen, ~10 s per look once warm) and `nomic-embed-text-v2-moe` (memory search). Use the `-instruct` builds: plain `qwen3:4b` and `qwen3-vl:4b` are thinking builds that ignore `think:false` and leak their reasoning into replies. Override with `SUNDAY_MODEL` / `SUNDAY_VISION` / `SUNDAY_EMBED`, or pick in Settings / ⌘K.
The plain `qwen3-vl:4b` is a thinking model and roughly twice as slow; Sunday can read either.

## Not built yet

Held-out evaluation and delayed-retention tests, a GATE question bank with answer keys (grading is LLM-as-judge),
embeddings search, a router model (Laya or otherwise), tool gateway and approvals, sync, the floating-orb polish.
