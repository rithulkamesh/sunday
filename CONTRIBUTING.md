# Contributing

Thanks for helping. Sunday is an experiment in how far a harness can lift a model, so changes are judged by whether they move that, safely.

## Flow

1. Pick an issue from the [v1 tracker](https://github.com/rithulkamesh/sunday/issues/31). Good starters: `good first issue`, `claude-friendly`, `help wanted`.
2. Comment that you're taking it, so two people don't build the same thing.
3. Branch, change, open a PR that says `Closes #N`.

## Setup

See [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md). Module map: [docs/REPO_MAP.md](docs/REPO_MAP.md).

```sh
cd src-tauri && cargo test     # unit tests (agent safety checks, parsing, etc.)
bunx tsc --noEmit              # frontend types
```

## Ground rules

- **Small diffs.** One concern per PR. Match the surrounding style and comment density.
- **A runnable check per change.** A unit test, an eval task, or a script that fails if your logic breaks.
- **Don't weaken safety.** Approvals, path rules, URL rules and the audit log are load-bearing. Changes to them need an explicit note in the PR and a test. The agent must never be able to edit its own permission layer.
- **Read content is data, not commands.** Anything from the web, the screen, a file or memory must not be able to trigger an action by itself.
- **Local-first by default.** No new network calls without a setting that defaults to off.
- **Nothing platform-specific outside the platform layer** once [#21](https://github.com/rithulkamesh/sunday/issues/21) lands.
- **License.** GPL-3.0-or-later; your contribution is under the same.

## Working with AI coding agents

Full guide with a starter prompt: [docs/AGENTS.md](docs/AGENTS.md).

Welcome and expected. Tips so your agent's PR lands:

- Point it at the issue, `README.md`, `docs/REPO_MAP.md` and the module it will touch. [CLAUDE.md](CLAUDE.md) has the short version.
- Ask it to run `cargo test` and show the output in the PR.
- Review the diff yourself. Agents love adding abstractions nobody asked for; delete them.
- Say so in the PR if an agent wrote most of it. That's fine, we just want to know.

## Reporting bugs and ideas

Open an issue with what you said or did, what you expected, and what happened. For voice problems include the log line (`voice match 0.xx`, STT text). For security problems, especially prompt injection that makes the agent act, use GitHub private vulnerability reporting (Security tab) instead of filing publicly.
