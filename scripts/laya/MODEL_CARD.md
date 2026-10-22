---
license: gpl-3.0
base_model: jhu-clsp/mmBERT-base
library_name: onnx
language: [en]
tags: [routing, intent-classification, onnx, sunday]
---

# Sunday router (Laya fine-tune)

The small decision model [Sunday](https://github.com/rithulkamesh/sunday), a local-first personal agent, uses to decide what an utterance means when its rules have no pattern. It answers one typed question in a single forward pass with calibrated probabilities: which of `chat`, `screen`, `do` or `research` fits, given the utterance, the last thing Sunday did and what it said.

It is a fine-tune of [Laya](https://github.com/NandhaKishorM/laya) (Apache-2.0) on `jhu-clsp/mmBERT-base` (MIT), exported to ONNX. This repo is released under GPL-3.0-or-later, the same license as Sunday; the upstream notices above still apply to their parts.

## Use

Not meant to be loaded by hand. `scripts/fetch-router.sh` in the Sunday repo downloads these files at a pinned revision, checks their SHA-256 and installs the matching sidecar program. Runs on CPU: about 1.2 GB on disk, about 1 GB of memory, about 140 ms per question.

## Results

On cases it was never trained on:

| Held-out set | Rules alone | Rules + this model |
|---|---|---|
| 48 hand-written cases | 37 right (77%) | 47 right (98%) |
| 35 real transcripts | 25 right (71%) | 33 right (94%) |

**Read with care.** The sets are small, the 48 were written by the maintainers while looking at where the rules failed, and the transcripts are mostly variations of "what's on my screen". This shows the approach works, not how often it will be right for you. The base checkpoint scored 56% on the first set before fine-tuning.

Sunday only acts on a route when the model is confident enough (`screen` 0.90, `research` 0.85, `do` 0.80), and a route only picks a path: every action still goes through Sunday's approvals.

## Training

About 2,400 template rows plus optional natural utterances from a local LLM, fine-tuned for 5 epochs on Apple silicon. The generation, training, evaluation and export scripts are in `scripts/laya` in the Sunday repo, so you can reproduce or retrain it. The real-transcript set is private.

## Limits

English only in practice. It knows four routes and nothing else. Known misses: "Ranked by the best" after opening a results page goes to `research`, and a reply to Sunday's own suggestion can stay `chat`.
