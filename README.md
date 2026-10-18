# Sunday

**S.U.N.D.A.Y.** — **S**ees · **U**nderstands · **N**otices · **D**ecides · **A**cts · **Y**ours

> **Can any model go AGI, given the right harness?**

Sunday is an open experiment to find out. It is a local-first agent harness: a voice and screen front end, a memory, a skill library and an action layer wrapped around a swappable model. The bet is that a lot of what looks like "intelligence" comes from the loop around the model (planning, verifying, remembering, learning from failure) and not only from the weights. If that is true, a small local model with a good harness should keep getting more capable, and a bigger model should get further still.

We don't claim to have AGI. We claim a measurable target, and the repo is organised to chase it:

> **v1 bar:** Sunday completes open-ended, multi-day goals on your own machine with no hand-holding, scoring **>= 70% on a fixed 50-task eval suite**, with the pass rate rising week over week while the model stays constant.

If the number moves, the harness is doing real work. If it doesn't, we learn that too. Either way it is a number, not a vibe.

Track progress in the [v1 master issue](https://github.com/rithulkamesh/sunday/issues/31). Docs: [rithulkamesh.github.io/sunday](https://rithulkamesh.github.io/sunday/).

## Where it is today

Honest status. This is early software, macOS only, and runs on a 4B local model by default.

| | |
|---|---|
| **Sees** | Screenshot, local OCR with exact boxes, vision model for diagrams, click-through overlay that points at things |
| **Understands** | Teaches any subject (plans, concepts, quizzes, spaced review), imports your notes |
| **Notices** | Wake word, voice activation, your voiceprint, long-term memory with provenance |
| **Decides** | Every utterance becomes one typed `Action`; deterministic rules router with a seam for a smarter one |
| **Acts** | Open apps, pages and files, type, click with a preview, all behind approvals and an audit log |
| **Yours** | Local models via Ollama, SQLite on disk, web access off unless you switch it on |

What each of these does in detail: [docs/FEATURES.md](docs/FEATURES.md).

## What's next (v1)

1. **Measure**: an eval harness and a 50-task suite. Nothing else counts until this exists.
2. **Agent loop**: persistent tasks, plan → act → observe → verify → replan.
3. **Memory that compounds**: successful runs become reusable skills, gated by the evals.
4. **Any model**: Ollama today; Amazon Nova / Bedrock, Anthropic, OpenAI-compatible, Gemini and llama.cpp next, behind one provider trait.
5. **Extensible**: MCP tools, a plugin/tool SDK, custom skills and routers.
6. **Everywhere**: Windows and Linux (macOS only today).

Everything is a labelled issue. Browse [`roadmap`](https://github.com/rithulkamesh/sunday/labels/roadmap).

## Quick start

```sh
scripts/fetch-models.sh     # speech models into ~/.cache/sunday/models
scripts/build-ocr.sh        # screen helper; needs Xcode command line tools
ollama pull qwen3:4b-instruct && ollama pull nomic-embed-text-v2-moe
ollama pull qwen3-vl:4b-instruct   # only for diagrams and non-text screens
bun install && bun run tauri dev
```

Say "sunday" or hold fn, then talk, or type in the workspace. Full walkthrough and troubleshooting: [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md).

## Docs

- [Getting started](docs/GETTING_STARTED.md): install, first run, things to try, where to begin hacking
- [What's in the repo](docs/REPO_MAP.md): every module, the data flow, the database tables
- [Features](docs/FEATURES.md): what exists and how it behaves
- [Contributing](CONTRIBUTING.md): the issue-to-PR flow, safety rules, working with AI coding agents

## Help build it

Pointing your own Claude (or any coding agent) at this repo is welcome. Start from issues labelled
[`claude-friendly`](https://github.com/rithulkamesh/sunday/labels/claude-friendly),
[`good first issue`](https://github.com/rithulkamesh/sunday/labels/good%20first%20issue) or
[`help wanted`](https://github.com/rithulkamesh/sunday/labels/help%20wanted). Windows and Linux ports need people on those machines.

## License

GPL-3.0-or-later. Forks and derivatives must stay open source under the same license. See [LICENSE](LICENSE).
