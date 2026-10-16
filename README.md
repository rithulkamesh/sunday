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

## Study loop

- "Quiz me" / "quiz me on graphs" / "next question": asks one spoken-friendly question (due reviews first, then weak spots).
- Your next answer is graded against a reference answer; the attempt is stored with an error type.
- Each concept gets a review date (1 day, 3 days, then growing; a miss resets it). The workspace shows what's due.
- "Skip" drops the question. "Bye" ends the conversation and frees the speech model.
- Chat, history and any unanswered question survive restarts (`sunday.db` in the app data dir).

## Not built yet (see the spec)

Concept graph and prerequisites, imported notes with citations, held-out evaluation, skills, tools,
GATE syllabus pack, floating-orb polish. Grading is LLM-as-judge; there is no answer key yet.
