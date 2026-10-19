"""Shared by the routing-model scripts: the question, the state text, and the held-out cases."""
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[2]
# The one question the model is trained and asked. The Rust sidecar (tools/laya-router) embeds the same file.
QUESTION = json.load(open(ROOT / "tools/laya-router/question.json"))
ROUTES = list(QUESTION["route"]["criteria"])


def state(last, reply, text):
    """What the model reads. Must match `state()` in src-tauri/src/laya.rs."""
    ctx = f"Last thing the assistant did: {last}. It said: {reply[:200].strip()}\n" if last else ""
    return f"{ctx}User: {text}"


def norm(t):
    return re.sub(r"[^a-z0-9 ]+", "", t.lower().replace("'", "")).strip()


def load_cases(path):
    """Rows of {last, reply, text, label}; a missing file is an empty list."""
    p = pathlib.Path(path)
    return [json.loads(l) for l in p.read_text().splitlines() if l.strip()] if p.exists() else []


def to_rows(cases):
    """Cases in Laya's training/eval format."""
    return [{"state": state(c["last"], c["reply"], c["text"]), "questions": QUESTION, "expected": {"route": c["label"]}} for c in cases]


def write_jsonl(path, rows):
    with open(path, "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
