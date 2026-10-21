"""Asks a local Ollama model for natural utterances per route and appends them to <work dir>/llm_rows.jsonl.
Labels come from the prompt and a keyword filter, so skim the output. Usage: python gen_llm.py [work dir]"""
import json, random, re, urllib.request
import pathlib, sys
from gen_routing import HELD, ctx
from lib import norm
random.seed(11)
WORK = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "laya-work")
WORK.mkdir(parents=True, exist_ok=True)
MODEL = "qwen3:4b-instruct"
BRIEFS = {
 "chat": "just talking to a voice assistant: greetings, thanks, short acknowledgements, questions about facts, science, programming, history, advice, opinions, jokes, definitions, maths, translation, writing help. Nothing that asks the assistant to operate the computer, look at the screen, or fetch today's news",
 "do": "commands to operate the computer by voice: open or switch apps, open websites, show videos or pictures of something (it opens them in the browser), play music, search a site, type text, click buttons, scroll, go back, close or refresh tabs, change zoom. Natural spoken phrasing with polite starters like 'can you' or 'could you' and occasional filler words",
 "screen": "questions about what is visible on the user's screen right now: what is this, what's this error, explain this page or chart or code, read this to me, what does this say, help with the problem on the screen, where is some button on the screen",
 "research": "questions that need fresh information from the internet: today's weather, news, scores, prices, who currently holds a position, upcoming events, reviews, trending topics",
}
ANGLES = ["short and casual", "polite and full sentences", "with speech-recognition style mistakes and filler words like uh and um", "very brief, two to four words", "curious and chatty", "technical", "students studying", "someone cooking or at home", "a developer at work", "mixed topics"]
ACTION = re.compile(r"\b(open|launch|play|pull up|show me|go to|click|type|scroll|switch to|close|refresh|search|navigate|turn (up|down)|zoom|bring up|set a (timer|reminder|alarm))\b", re.I)
FRESH = re.compile(r"\b(weather|news|score|price|latest|current|currently|today|tonight|tomorrow|right now|who won|trending|stock|forecast|this week|next (match|game|event))\b", re.I)
SCREEN = re.compile(r"\b(screen|this|that|here|page|window|image|picture|chart|error|diagram|slide|tab|code|button|text|popup|message)\b", re.I)
KEEP = {
    "chat": lambda t: not ACTION.search(t) and not FRESH.search(t) and not re.search(r"\b(screen|on my)\b", t, re.I),
    "do": lambda t: bool(ACTION.search(t)) and not FRESH.search(t),
    "screen": lambda t: bool(SCREEN.search(t)) and not ACTION.search(t) and not FRESH.search(t),
    "research": lambda t: bool(FRESH.search(t)) and not ACTION.search(t),
}


def ask(label, angle):
    prompt = f"Write 15 different things a person might say out loud to a voice assistant. Each is {BRIEFS[label]}. Style: {angle}. Vary the topics and wording a lot. One per line, no numbering, no quotes, no extra text."
    body = json.dumps({"model": MODEL, "prompt": prompt, "stream": False, "options": {"temperature": 1.0, "num_predict": 450}}).encode()
    for attempt in range(3):
        try:
            r = json.loads(urllib.request.urlopen(urllib.request.Request("http://localhost:11434/api/generate", body, {"Content-Type": "application/json"}), timeout=200).read())
            break
        except Exception as e:
            print("retry", label, angle, e, flush=True)
    else:
        return []
    lines = [re.sub(r"^[-*\d.)\s\"]+|[\"]+$", "", x).strip() for x in r["response"].splitlines()]
    return [x for x in lines if 2 <= len(x) <= 140 and KEEP[label](x)]


out, seen = [], set()
for rep in range(1):
    for label in BRIEFS:
        for angle in ANGLES:
            for t in ask(label, angle):
                if norm(t) in HELD or norm(t) in seen: continue
                seen.add(norm(t)); last, reply = ctx(); row = {"last": last, "reply": reply, "text": t, "label": label}; out.append(row)
                with open(WORK / "llm_rows.jsonl", "a") as f: f.write(json.dumps(row, ensure_ascii=False) + "\n")
        print(rep, label, sum(1 for o in out if o["label"] == label), flush=True)
print(len(out))
