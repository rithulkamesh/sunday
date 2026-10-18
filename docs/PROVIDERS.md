---
id: providers
slug: /providers
title: Providers
---

# Providers

A provider is where a model call goes. Sunday runs on **Ollama** by default, so nothing leaves your machine. You can send some roles to a hosted model and keep the rest local.

## Roles

| Role | What it does | Providers |
|---|---|---|
| Chat | Replies, quizzes, grading, skills | any |
| Planner | Picks the next step when Sunday acts on your computer | any, follows Chat until you set it |
| Vision | Reads diagrams and non-text screens | Ollama only |
| Embeddings | Memory and note search | Ollama only |
| Router | Decides what an utterance means | built-in rules, no model |

## Set one up

Open **Settings → Models** and press **Change** on Chat or Planner. Choose a provider, paste its key if it needs one, pick a model, then press **Test and use**. Nothing is saved until the test passes, so a wrong key or model shows up there, not mid-sentence. Switching the planner to a hosted model while chat stays on Ollama takes one pass through that form.

**Keys live in your operating system's keychain**, never in the database, the repo or the logs. Sunday doesn't show a saved key again; you can replace or remove it.

Prefer environment variables? They win over saved settings, which is handy for CI and scripts.

| Provider | Key variable | Other settings |
|---|---|---|
| `ollama` | none | runs on `localhost:11434` |
| `openai` (OpenAI, OpenRouter, vLLM, LM Studio, llama.cpp server) | `OPENAI_API_KEY`, optional for local servers | `OPENAI_BASE_URL` |
| `anthropic` | `ANTHROPIC_API_KEY` | |
| `gemini` | `GEMINI_API_KEY` | uses Google's OpenAI-compatible endpoint |
| `nova` (Amazon Nova on Bedrock) | `AWS_BEARER_TOKEN_BEDROCK` | `AWS_REGION`, default `us-east-1`; replies arrive in one piece, not word by word |

`SUNDAY_PROVIDER` and `SUNDAY_MODEL` still pick the provider and model for chat when nothing is saved in Settings.

## Privacy

Anything but Ollama sends the prompt to that provider. A prompt can include your notes and memories that matched the question, and the text of web pages or the screen when you asked Sunday to look at them. Settings shows a warning next to every role that leaves your machine. Embeddings and vision stay on Ollama, so notes and screenshots are never sent for indexing.

## Cost

Sunday sends short prompts with a few retrieved notes, and the planner makes up to six calls per task. Hosted prices change, so check your provider's page. The cheapest way to try a hosted model is the planner role alone.

## Test a provider

```sh
SUNDAY_PROVIDER=openai OPENAI_BASE_URL=http://localhost:11434/v1 SUNDAY_PROVIDER_TEST=1 \
  cargo test conformance::live -- --nocapture
```

This runs a real request and a real stream against the chosen provider. The same checks run in CI for every provider against a local mock server.

## Add a provider

1. Implement `Provider` in `src-tauri/src/provider.rs`: `complete`, `stream`, `models`, `warm`, `name`.
2. Add it to `build` and to the provider list in Settings.
3. Add a mock that answers in its wire format and a test that calls `check`, as the existing providers do.
4. Keep the safety rule: a new provider is opt-in and its calls only happen after the user picks it.
