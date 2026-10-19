---
id: architecture
slug: /architecture
title: Architecture
---

# Architecture

How Sunday fits together, from a sound in the room to a click on the screen. The diagrams are the source of truth for the shape of the system; the [repo map](REPO_MAP.md) says which file each box lives in.

## The whole thing

Everything runs on your machine. The model behind each role is swappable, and the only network calls are to a model you chose and, if you switch web access on, the pages you ask Sunday to read.

```mermaid
flowchart TB
    in["Voice, typing and the screen<br/>mic, VAD, wake word, speech to text, OCR"] --> route["Router<br/>one typed Action"]
    route --> talk["Talk<br/>chat, teaching, research,<br/>memory, notes, skills"]
    route --> look["Screen help<br/>OCR, vision, overlay"]
    route --> agent["Agent loop<br/>plan, act, verify"]
    talk --> models["Swappable models<br/>chat and planner: Ollama, MLX or hosted<br/>embeddings and vision: on this Mac"]
    look --> models
    agent --> models
    agent --> os["Platform layer<br/>open, type, click"]
    talk --> out["Spoken by Kokoro<br/>and shown in the HUD"]
    look --> out
    agent --> out
```

Every utterance becomes exactly one `Action` before anything runs. `Rules` is the deterministic router and always the fallback; a smarter router plugs in behind the same `Decider` trait (see [Router](ROUTER.md)).

## A voice turn

What happens between "Sunday, open Notes" and the reply, including the part where you talk over it.

```mermaid
sequenceDiagram
    actor You
    participant Audio as Mic, VAD, wake word
    participant STT as Speech to text
    participant Turn as Turn loop (llm.rs)
    participant Router as Router
    participant Agent as Agent loop
    participant TTS as Text to speech
    You->>Audio: "Sunday, open Notes"
    Audio->>Audio: keyword and voiceprint match
    Audio->>STT: audio of the request
    STT->>Turn: text
    Turn->>Router: decide(text, context)
    Router-->>Turn: Do("open Notes")
    Turn->>Agent: start(task)
    Agent-->>Turn: Done("Done.")
    Turn->>TTS: sentence, stamped with the epoch
    TTS-->>You: speaks
    You->>Audio: talks over Sunday
    Audio->>TTS: interrupt (epoch + 1)
    Note over TTS: audio queued under the old epoch is dropped,<br/>including a sentence that was still being made
```

Barge-in works without echo cancellation: while Sunday talks, the mic hears Sunday, so the detector learns the echo level and only fires when you are clearly louder than it and your voice matches yours. Each spoken sentence carries the epoch it was requested under, and an interrupt bumps the epoch, so anything queued earlier is discarded. The check and the queueing happen under the same locks, so an interrupt cannot slip in between them and leave stale audio playing.

## The agent loop

`agent.rs` runs one typed step at a time. The model only ever picks a tool name and one argument; code decides what happens next.

```mermaid
flowchart TD
    start(["Task from the router"]) --> limits{"Within budget?<br/>12 steps, 120 s,<br/>fewer than 3 failures in a row"}
    limits -- "no" --> report["Stop and say what went wrong"]
    limits -- "yes" --> plan["Planner picks the next step<br/>tool, arg, say"]
    plan --> fin{"finish?"}
    fin -- "yes" --> done(["Say the result"])
    fin -- "no" --> dup{"Same call as one<br/>already done?"}
    dup -- "yes" --> done
    dup -- "no" --> check["check(): arguments are safe<br/>path, URL, app name, keys"]
    check -- "rejected" --> fail["Log the failure<br/>for the planner"]
    check -- "ok" --> perm{"Typing or clicking and<br/>no Accessibility permission?"}
    perm -- "yes" --> openset["Open System Settings,<br/>stop and say so"]
    perm -- "no" --> ask{"Needs a yes?<br/>see approvals"}
    ask -- "yes" --> wait["Save state and ask<br/>the user"]
    wait --> reply{"Reply"}
    reply -- "yes" --> exec
    reply -- "no" --> cancel(["Cancelled"])
    ask -- "no" --> exec["exec(): run the step"]
    exec --> verify{"verify(): did it really<br/>happen?"}
    verify -- "yes" --> ok["Log it as verified,<br/>reset the failure count"]
    verify -- "no" --> bad["Log it as NOT verified,<br/>allow a retry, count a failure"]
    ok --> limits
    bad --> limits
    fail --> limits
```

A failed or unverified step is not "done": the planner reads `failed or NOT verified` in the step log and can try again differently. Every log line carries the tool and what it was given (`open_url(https://…) → done (verified)`), so the model can tell what was already done. Three failures in a row end the task with an honest message instead of trying forever, and a missing permission ends it at once because retrying cannot fix it.

### Approvals

Every tool has a tier in one table (`POLICY`). The model can't reach it; it only picks a name.

```mermaid
flowchart TD
    s["A step"] --> t{"Tier"}
    t -- "Read<br/>web search, look at screen, recall" --> run["Runs, and marks the task<br/>as having read untrusted content"]
    t -- "Reversible<br/>open app, open page" --> tr{"Task already read<br/>untrusted content?"}
    tr -- "no" --> run
    tr -- "yes" --> ask["Asks for a yes"]
    t -- "Confirm<br/>type, click, shortcuts, open file" --> c{"Safe by construction?"}
    c -- "type_text: the words are in the user's own request,<br/>nothing untrusted read, short, into the app just opened" --> run
    c -- "press_keys: cmd+n, t, s, f or l,<br/>nothing untrusted read" --> run
    c -- "anything else" --> ask
    t -- "not in the table" --> ask
```

"Tainted" means the task has read the web, the screen or memory. It is set by code, never by the model, and never cleared, so content that was read can never talk its way into an action that runs without asking. The typing rule compares the text with the user's own request, which the model cannot change.

### Verifiers

After a step runs, `verify()` checks real state instead of trusting that it worked.

| Step | What is checked | If it can't be checked |
|---|---|---|
| `open_app` | the app comes to the front (polled for about 3 s) | |
| `open_url` | the front browser's address bar is on the same host | unverified |
| `open_path` | the path exists | |
| `type_text` | the first 30 letters and digits appear on screen (screenshot and OCR) | unverified |
| `web_search`, `look_at_screen` | the result is not empty | |
| `press_keys`, `click`, `recall` | nothing yet | unverified |

Unverified steps are logged without the `(verified)` mark, so the planner and the audit log can tell the difference.

## Routing

```mermaid
flowchart TD
    u["Utterance<br/>plus last action and reply"] --> r["Rules<br/>exact patterns"]
    r -- "a clear match" --> a["Action"]
    r -- "no pattern: chat" --> m["Corrections memory<br/>what you taught it"]
    m -- "a close match in the same context" --> a
    m -- "nothing close" --> d["Routing model (Laya)<br/>optional sidecar"]
    d -- "sure enough" --> a
    d -- "unsure, slow or not installed" --> chat["Chat"]
    a --> p{"permitted()?"}
    p -- "no" --> why["Say why"]
    p -- "yes" --> go["Run it"]
    a -. "this turn restated the last one" .-> learn[("route_examples<br/>local table")]
    learn -.-> m
```

`Rules` never goes away: it decides first, and it is the answer whenever the other two are missing, slow or unsure. The corrections memory and the routing model only choose where an utterance that matched no pattern goes, and `permitted()`, the tiers and the approvals run exactly as before, so a wrong route can waste a turn but cannot widen what Sunday may do. The model needs more confidence to send you to the screen (a screenshot nobody asked for is the costly mistake) than to an action. Details and measurements: [Router](ROUTER.md).

Chat can't claim computer actions either. A chat reply that says it opened, clicked, played or typed something is cut off and replaced with an honest line, because in conversation nothing was done on the computer.

## Platform layer

All OS-specific code lives behind one trait in `src-tauri/src/platform/`. Nothing else may call `osascript`, `screencapture`, `open` and the like.

```mermaid
flowchart LR
    agent["agent.rs"] --> P
    screen["screen.rs"] --> P
    llm["llm.rs"] --> P
    subgraph P["trait Platform"]
        direction TB
        a["open_app, open_url, open_path"]
        b["type_text, press_keys, click"]
        c["screenshot, ocr, crop, resize"]
        d["desk, frontmost_app, browser_url"]
        e["screen_access, input_access"]
    end
    P --> mac["macos.rs<br/>open, AppleScript, CoreGraphics, Swift helper"]
    P --> uns["unsupported.rs<br/>fails with a spoken message"]
    P --> rec["Recorder<br/>tests only: records, does nothing"]
```

In tests and evals the `Recorder` stands in, so a test can say "this would have opened Notes" without touching the machine.

## Models and providers

```mermaid
flowchart LR
    chat["Chat role<br/>replies, quizzes, grading, skills"] --> prov
    planner["Planner role<br/>next step of a task"] --> prov
    prov{"Provider trait"} --> ollama["Ollama"]
    prov --> mlx["MLX<br/>on this Mac"]
    prov --> oai["OpenAI-compatible<br/>OpenAI, OpenRouter, vLLM, LM Studio"]
    prov --> ant["Anthropic"]
    prov --> gem["Gemini"]
    prov --> nova["Amazon Nova<br/>Bedrock"]
    emb["Embeddings"] --> e1["Ollama"]
    emb --> e2["On device<br/>MiniLM"]
    vis["Vision"] --> ollama
```

The planner follows chat until you set it separately. Embeddings and vision always run on this Mac, so notes and screenshots are never sent out for indexing. Setup and privacy: [Providers](PROVIDERS.md).

## Memory and data

One SQLite file holds everything.

```mermaid
erDiagram
    messages {
        int id PK
        int ts
        text role
        text text
    }
    facts {
        int id PK
        text kind
        text source
        text text
        real confidence
        blob embedding
    }
    documents ||--o{ chunks : "split into"
    documents {
        int id PK
        text title
        text path
        text hash
    }
    chunks {
        text text
        int doc
    }
    concepts ||--o{ prereqs : "needs"
    concepts ||--o{ attempts : "practised in"
    concepts ||--o| reviews : "scheduled by"
    concepts {
        text name PK
        text subject
        int ord
    }
    prereqs {
        text concept
        text requires
    }
    attempts {
        int id PK
        text concept
        text verdict
        text error_type
    }
    reviews {
        text concept PK
        int due
        real interval_days
        int streak
    }
    skills ||--o{ skill_runs : "run as"
    skills {
        text id PK
        int version PK
        text status
    }
    skill_runs {
        int id PK
        text skill
        int version
        text output
    }
    actions {
        int id PK
        text task
        text tool
        text args
        text outcome
    }
    kv {
        text key PK
        text value
    }
```

`actions` is the audit log of everything the agent did, with the tier and whether it ran on its own or you approved it. `kv` holds settings and the saved state of a task waiting for a yes.

## Tests and CI

```mermaid
flowchart LR
    pr["Pull request"] --> ci["CI on macOS<br/>cargo test"]
    pr --> fe["Frontend<br/>tsc"]
    pr --> dc["Docs build<br/>fails on broken links"]
    ci --> unit["Unit tests"]
    ci --> ev["Eval suite<br/>routing, agent, injection"]
    local["Your machine"] --> live["bun run test:live<br/>Ollama, models, screen, web"]
```

The default tests need nothing but the code. Anything that needs Ollama, models, the screen or the web is `#[ignore]`d and runs locally. More in [Testing](TESTING.md).
