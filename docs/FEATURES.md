---
id: features
slug: /features
title: Features
---

# Features (what exists today)

Everything below is implemented and macOS-only for now. What is planned is on the [roadmap](ROADMAP.md).


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

**Acting.** "Open Spotify", "pull up images of a keyboard", "show me YouTube videos on machine learning", "open Notes and type hello", "go to github.com". The model picks one typed step at a time (`agent.rs`): open an app, open a page (in the browser you already have open), open a file in your home folder, type into the front app, press a shortcut like cmd+n, click, search the web, look at the screen, recall. It can chain several apps in one request, and a follow-up like "sort by the best" or "scroll down" continues what is on screen. [The loop](ARCHITECTURE.md#the-agent-loop):

- **It checks its own work.** After each step Sunday looks at the real state: did the app come to the front, is the browser on that site, is the typed text on screen. A step that did not work is not treated as done: the planner is told it failed and tries something different. Three failures in a row, 12 steps or two minutes end the task with an honest message instead of a false "Done".
- **Approvals by tier** (one table, `POLICY`). Reading (web search, look at screen, recall) runs freely. Opening an app or page is reversible and just happens when you asked for it directly. Typing, clicking, shortcuts and opening files ask "…?" and wait for a yes, with two exceptions that are safe by construction: typing the words you said in your own request into the app the task just opened, and the shortcuts new note, save, new tab, find and address bar (cmd+n, s, t, f, l). Once the task has read anything from the web, the screen or memory, even opening an app asks (read content is data, never a command). A tool that isn't in the table always asks.
- **Guard rails.** Hidden folders, non-http addresses and anything outside your home folder are refused. Every step is logged in the `actions` table with its tier and whether it ran on its own or you approved it.
- **Permissions on first use.** Typing and clicking need Accessibility. If it is missing, Sunday opens the right page in System Settings and says what to turn on, instead of retrying.
- **Honest chat.** A conversational reply never claims it opened, clicked, played or typed something: only the agent acts, and a reply that says otherwise is replaced with "I haven't done that. Ask me directly."

**Decisions.** Every utterance becomes one typed `Action` (`decision.rs`) before anything runs, and is checked against what's permitted. `Rules` is the deterministic router and always the fallback; short corrections and fillers ("uh", "I said…", "no, …") don't hide a command, and and with the optional on-device [routing model](ROUTER.md) installed, a follow-up such as "tell me more" after a screen answer, or "what's the weather in Paris", is routed by meaning instead of falling through to chat. Restating a command ("I said sort by the best") teaches the router at once.

**⌘K** command palette; **Settings** has the model picker and a diagnostics checklist.

**Your voice.** "Learn my voice" (say it, or Settings / ⌘K) records ~20 s of you, builds a voiceprint (wespeaker ResNet34, on-device) and from then on Sunday ignores other voices: wake word, commands and talking-over-it all have to match you. The fn key always works. Logs show `voice match 0.xx` so the threshold (`speaker::SAME`) can be tuned. "Forget my voice" removes it.

Voice details: "Sunday" or fn wakes it; say "stop", talk over it, or press fn to interrupt; "bye" ends the session and unloads the speech model. Talking over Sunday cancels everything it had queued, including a sentence that was still being made. Replies are short and spoken the way a person would say them: "Done." after a command, more only when there is something to tell.

**The voice.** Kokoro, on this Mac. The default is the American voice 0 at normal pace. Try others without a rebuild with `SUNDAY_VOICE` (a speaker id from `kokoro-en-v0_19`: 0 to 4 American women, 5 and 6 American men, 7 and 8 British women, 9 and 10 British men) and `SUNDAY_VOICE_SPEED` (1.0 is normal). `cargo test speaks_aloud -- --ignored --nocapture` plays the same sentence in several voices so you can pick by ear.

**Models.** Ollama by default, or MLX on this Mac with no Ollama, or a hosted model (OpenAI-compatible, Anthropic, Gemini, Amazon Nova) for chat or for planning only. Memory search can run on the device too. See [Providers](PROVIDERS.md).

