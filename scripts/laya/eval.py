"""Scores a checkpoint on the held-out sets. Usage: python eval.py <checkpoint dir | base> [work dir]
Prints raw accuracy, and accuracy of the policy the app uses: the model overrides "chat" only when it is sure enough
(the thresholds in src-tauri/src/brain/laya.rs), otherwise the answer stays chat."""
import pathlib
import sys

import laya

from lib import ROOT, ROUTES, load_cases, norm, state, QUESTION

# Keep in step with MIN in src-tauri/src/brain/laya.rs.
MIN = {"screen": 0.90, "do": 0.80, "research": 0.85}


def predict(agent, c):
    a = agent.predict(state(c["last"], c["reply"], c["text"]), QUESTION)["answers"]["route"]
    return a["choice"], a["probabilities"] if "probabilities" in a else a.get("probs", {})


def gated(choice, probs):
    return choice if choice in MIN and probs.get(choice, 0) >= MIN[choice] else "chat"


if __name__ == "__main__":
    which = sys.argv[1]
    work = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else "laya-work")
    agent = laya.load(str(work / "base") if which == "base" else which)
    for name in ("proto", "real"):
        cases = load_cases(ROOT / f"evals/router/{name}.jsonl")
        if not cases:
            continue
        raw = gate = 0
        misses = []
        for c in cases:
            choice, probs = predict(agent, c)
            g = gated(choice, probs)
            raw += choice == c["label"]
            gate += g == c["label"]
            if g != c["label"]:
                misses.append((c["label"], choice, round(probs.get(choice, 0), 2), c["last"], c["text"]))
        print(f"{name}: raw {raw}/{len(cases)}, with the app's confidence gate {gate}/{len(cases)}")
        for m in misses:
            print("   MISS want=%-8s got=%-8s p=%-5s last=%-8s %s" % m)
