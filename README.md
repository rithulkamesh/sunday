# Sunday

Local-first voice tutor, starting with GATE prep. Everything runs on your machine:
Silero VAD + wake-word spotter + Parakeet (speech in), Ollama (thinking), Kokoro (speech out), SQLite (memory).

## Run

```sh
scripts/fetch-models.sh     # once: speech models into ~/.cache/sunday/models
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
Only on request; the image is read once, sent to local Ollama only, and deleted. Needs Screen Recording permission.

**Skills.** Versioned, prompt-only capabilities (a template, an output schema, tests). No generated code, no tools.
Lifecycle: proposed → experimental → approved; approval needs a passing test run on that version; any approved version can be rolled back.
Describe one in the Skills view and the model drafts it. Voice: "run skill study.mnemonic on the OSI model".

**Decisions.** Every utterance becomes one typed `Action` (`decision.rs`) before anything runs, and is checked against what's
permitted. `Rules` is the deterministic router; the `Decider` trait is where a smarter router goes.

**⌘K** command palette; **Settings** has the model picker and a diagnostics checklist.

Voice details: "Sunday" or fn wakes it; say "stop", talk over it, or press fn to interrupt; "bye" ends the session and unloads the speech model.

## Models

Defaults: `qwen3:4b` (text) and `qwen3-vl:4b-instruct` (screen, ~10 s per look once warm). Override with `SUNDAY_MODEL` / `SUNDAY_VISION`, or pick in Settings / ⌘K.
The plain `qwen3-vl:4b` is a thinking model and roughly twice as slow; Sunday can read either.

## Not built yet

Held-out evaluation and delayed-retention tests, a GATE question bank with answer keys (grading is LLM-as-judge),
embeddings search, a router model (Laya or otherwise), tool gateway and approvals, sync, the floating-orb polish.
