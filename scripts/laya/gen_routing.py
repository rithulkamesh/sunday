"""Builds the routing model's training data from templates (and, if present, model-written utterances), and writes the
held-out sets in Laya's format. Usage: python gen_routing.py [work dir]   (default ./laya-work)

Held out, never trained on: evals/router/proto.jsonl (hand-written cases) and evals/router/real.jsonl (your own labelled
transcripts, local only). Any training utterance that matches one of them is dropped.
"""
import json
import pathlib
import random
import sys
from collections import Counter

from lib import ROOT, ROUTES, load_cases, norm, state, to_rows, write_jsonl

random.seed(7)
PROTO = load_cases(ROOT / "evals/router/proto.jsonl")
REAL = load_cases(ROOT / "evals/router/real.jsonl")
HELD = {norm(c["text"]) for c in PROTO + REAL}

# ---------------------------------------------------------------- templates
APPS = ["Notes", "Safari", "Spotify", "Mail", "Calendar", "Slack", "Messages", "Finder", "Terminal", "Chrome", "Photos", "Music", "Maps",
        "Reminders", "Preview", "Xcode", "VS Code", "Zoom", "Discord", "Obsidian", "Figma", "Pages", "Numbers", "Settings"]
SITES = ["YouTube", "GitHub", "Reddit", "Hacker News", "Wikipedia", "Gmail", "Twitter", "LinkedIn", "Amazon", "Netflix", "Google Maps", "Stack Overflow", "Notion", "arXiv"]
TOPICS = ["machine learning", "jazz piano", "the French revolution", "rust async", "sourdough bread", "linear algebra", "chess openings", "black holes",
          "home workouts", "photography tips", "operating systems", "guitar chords", "quantum computing", "lo-fi beats", "cooking pasta", "neural networks",
          "world history", "calculus", "japanese anime", "space launches", "keyboard shortcuts", "watercolor painting", "docker basics", "yoga for beginners"]
THINGS = ["a keyboard", "a red bike", "a mountain", "a cat", "a laptop stand", "a desk lamp", "a sunset", "a wedding dress", "a coffee machine", "a guitar"]
FILL = ["", "", "", "uh ", "um ", "okay ", "okay, ", "so ", "hey ", "please ", "can you ", "could you ", "would you ", "sunday ", "sunday, ", "sandey, ", "sandy, ",
        "I want you to ", "go ahead and ", "can you please ", "uh can you ", "hey sunday, can you "]
TAIL = ["", "", "", "", " please", " for me", " now", " real quick", " if you can"]


IMP = {"open", "launch", "switch", "pull", "go", "bring", "show", "play", "search", "find", "watch", "type", "click", "scroll", "refresh", "reload", "close", "make", "zoom",
       "save", "pause", "turn", "minimize", "explain", "read", "help", "point", "summarize", "look", "research", "check", "sort", "filter", "try", "describe", "start", "tell"}
LIGHT = ["", "", "", "", "uh ", "um ", "okay ", "okay, ", "so ", "hey ", "sunday ", "sunday, ", "sandey, ", "sandy, ", "hey sunday, "]


def dress(core):
    first = core.split()[0].lower()
    if first in IMP:
        t = random.choice(FILL) + core + random.choice(TAIL)
    else:
        t = random.choice(LIGHT) + core
    t = t.strip()
    if random.random() < 0.55:
        q = first in {"what", "what's", "who", "when", "where", "how", "is", "are", "do", "does", "can", "could", "would", "why"} or t.lower().startswith(("can", "could", "would"))
        t = t[0].upper() + t[1:] + random.choice(["?", "?", ""] if q else [".", "", "!"])
    return t


def pick(xs):
    return random.choice(xs)


DO_CORE = [
    lambda: f"open {pick(APPS)}", lambda: f"launch {pick(APPS)}", lambda: f"switch to {pick(APPS)}", lambda: f"pull up {pick(SITES)}", lambda: f"go to {pick(SITES)}",
    lambda: f"bring up {pick(APPS)}", lambda: f"open {pick(APPS)} and type {pick(['hello', 'my notes for today', 'buy milk', 'meeting at three', 'hi there'])}",
    lambda: f"show me {pick(SITES)} videos on {pick(TOPICS)}", lambda: f"show me videos about {pick(TOPICS)}", lambda: f"pull up images of {pick(THINGS)}",
    lambda: f"show me pictures of {pick(THINGS)}", lambda: f"play {pick(TOPICS)} on {pick(['Spotify', 'YouTube', 'Music'])}", lambda: f"play some {pick(['jazz', 'lo-fi', 'rock', 'classical', 'piano music'])}",
    lambda: f"search {pick(SITES)} for {pick(TOPICS)}", lambda: f"search for {pick(TOPICS)} on {pick(SITES)}", lambda: f"find {pick(TOPICS)} videos on YouTube",
    lambda: f"watch a video about {pick(TOPICS)}", lambda: f"type {pick(['hello world', 'thanks for the update', 'see you tomorrow', 'my email address', 'good morning'])}",
    lambda: f"click the {pick(['blue', 'red', 'green', 'send', 'submit', 'login', 'save', 'cancel'])} button", lambda: f"click on {pick(['the first link', 'the search box', 'the menu', 'the logo', 'that icon'])}",
    lambda: f"scroll {pick(['down', 'up', 'down a bit', 'to the bottom', 'to the top'])}", lambda: "go back", lambda: "go forward", lambda: "refresh the page", lambda: "reload it",
    lambda: "close this tab", lambda: "open a new tab", lambda: "make the text bigger", lambda: "zoom in", lambda: "zoom out", lambda: "save this file", lambda: "pause the video", lambda: "play it",
    lambda: "turn the volume down", lambda: "open a new window", lambda: "minimize this", lambda: f"open the {pick(['first', 'second', 'third', 'last'])} result", lambda: "start a new note",
]
DO_FOLLOW = [  # only after the assistant just did something on the computer
    lambda: f"sort by {pick(['the best', 'best', 'most viewed', 'newest', 'oldest', 'top rated', 'date', 'relevance', 'price'])}", lambda: f"{pick(['best', 'newest', 'most viewed', 'top rated', 'ranked by the best', 'cheapest first'])}",
    lambda: f"open {pick(['the first one', 'the second one', 'number three', 'the last one', 'that one', 'the one with three colours', 'the top result'])}", lambda: f"click {pick(['the first one', 'that', 'the second one', 'the blue one'])}",
    lambda: pick(["scroll down", "go back", "next page", "previous page", "do that again", "try the next one", "play it", "pause it", "close it", "open it", "no, the other one", "no, open the second result"]),
    lambda: f"filter by {pick(['this week', 'last month', 'free', 'videos', 'images', 'under ten dollars'])}", lambda: f"now {pick(['type hello', 'click the first one', 'scroll down', 'open Notes', 'go back'])}",
    lambda: f"I said {pick(['sort by the best', 'open the second one', 'scroll down', 'click that', 'play it'])}",
]
SCREEN_CORE = [
    lambda: pick(["what's on my screen", "what is on my screen", "what's on the screen", "what am I looking at", "what is this on my screen", "describe my screen", "tell me what you see",
                  "what do you see on my screen", "look at my screen", "can you see my screen", "what's open right now", "what is this", "what's this", "what's that on the left"]),
    lambda: f"explain {pick(['this page', 'this image', 'this chart', 'this diagram', 'this error', 'this code', 'this slide', 'this paragraph', 'this problem', 'this question', 'this table'])}",
    lambda: f"what does {pick(['this', 'that', 'this error', 'the image', 'this message', 'this button', 'this graph', 'the red text', 'this popup'])} {pick(['say', 'mean', 'show', 'explain'])}",
    lambda: f"what is {pick(['this error', 'this window', 'this page', 'this chart', 'this popup', 'this website', 'that icon'])}", lambda: f"read {pick(['this', 'this page', 'what is on the screen', 'this error', 'the text on screen'])} to me",
    lambda: f"help me with {pick(['this problem', 'this question', 'this error', 'what is on my screen', 'this code'])}", lambda: f"point at {pick(['the important part', 'the error', 'the button I need', 'the search box', 'the date'])}",
    lambda: f"summarize {pick(['this page', 'what is on my screen', 'this article', 'this slide', 'this email'])}", lambda: f"look at {pick(['my screen', 'this page', 'this chart'])} and tell me {pick(['what is wrong', 'what it means', 'what to do next', 'if it looks right'])}",
    lambda: pick(["water on my screen", "what's on my screen now", "whats on my screen", "my screen", "what is on screen", "explain what's on screen", "uh what's on my screen", "what are all these windows"]),
    lambda: f"where is {pick(['the save button', 'the settings', 'the search bar', 'the error', 'the close button'])} on this screen",
]
SCREEN_FOLLOW = [  # only after the assistant just described the screen
    lambda: pick(["tell me more", "go into more detail", "could you go more in detail", "can you elaborate", "more detail please", "say more about that", "what else", "and what else is there",
                  "what does that mean", "explain that", "explain that more", "why is that", "and the bottom part", "what about the top", "what about the second paragraph", "and what's that on the right",
                  "what's the error say exactly", "read it out", "zoom in on that", "and the other window", "is that a problem", "how do I fix that", "what should I do about it", "keep going", "go on"]),
]
CHAT_FOLLOW = [  # same words as the screen follow-ups, but after ordinary conversation: stays conversation
    lambda: pick(["tell me more", "go into more detail", "could you go more in detail", "can you elaborate", "more detail please", "say more about that", "what else", "what does that mean",
                  "explain that", "explain that more", "why is that", "why", "how come", "can you give an example", "are you sure", "keep going", "go on", "and then what", "what about the other one", "interesting"]),
]
RESEARCH_CORE = [
    lambda: f"what's the weather {pick(['in Paris', 'in Mumbai', 'in London', 'today', 'tomorrow', 'in Tokyo', 'in New York this weekend', 'like outside'])}", lambda: f"who won {pick(['the match last night', 'the game yesterday', 'the election', 'the cricket match', 'the grand prix', 'the final'])}",
    lambda: f"latest news {pick(['about Rust', 'on the economy', 'about OpenAI', 'from India', 'about the stock market', 'in tech', 'about the new iPhone'])}", lambda: f"what's the price of {pick(['bitcoin', 'gold', 'a PS5', 'Tesla stock', 'the new MacBook', 'oil', 'ethereum'])} right now",
    lambda: f"look up {pick(TOPICS)}", lambda: f"search the web for {pick(TOPICS)}", lambda: f"research {pick(TOPICS)}", lambda: f"find out about {pick(TOPICS)}", lambda: f"what's trending {pick(['on Twitter', 'today', 'in tech', 'right now', 'in music'])}",
    lambda: f"what time is it in {pick(['Tokyo', 'London', 'New York', 'Sydney'])}", lambda: f"who is the current {pick(['president of France', 'CEO of Apple', 'prime minister of the UK', 'champion', 'coach of the team'])}",
    lambda: f"when is the next {pick(['solar eclipse', 'Apple event', 'full moon', 'world cup match', 'rocket launch', 'holiday'])}", lambda: f"how is {pick(['the stock market', 'the Nasdaq', 'the dollar', 'the rupee'])} doing today",
    lambda: f"what happened {pick(['in the news today', 'at the keynote', 'in the match', 'with the strike', 'at the summit yesterday'])}", lambda: f"is {pick(['the new Rust release', 'the store', 'the bridge', 'the library'])} out yet",
    lambda: f"check the score of {pick(['the Lakers game', 'the match', 'the football game', 'the cricket match'])}", lambda: f"find me reviews of {pick(['the new MacBook', 'that phone', 'the Sony headphones', 'the movie'])}",
]
CHAT_CORE = [
    lambda: pick(["hello", "hello there", "hi", "hey", "hey sunday", "good morning", "good evening", "how are you", "how's it going", "what's up", "thanks", "thank you", "thanks, that helps", "great, thanks",
                  "okay", "okay cool", "got it", "nice", "awesome", "never mind", "that makes sense", "I see", "right", "hmm", "uh huh", "yes", "yeah", "yep", "no", "nope", "sure", "alright", "okay, uh", "mm", "yes please",
                  "what can you do", "who are you", "what's your name", "are you there", "can you hear me", "good night", "see you later", "that's a bug", "no no no", "stop", "you're wrong", "that was wrong", "that didn't work"]),
    lambda: f"what is {pick(['a mutex', 'a semaphore', 'a compiler', 'a closure', 'recursion', 'a deadlock', 'entropy', 'a monad', 'TCP', 'a hash table', 'inflation', 'photosynthesis', 'gravity', 'DNA', 'a black hole'])}",
    lambda: f"explain {pick(['how a mutex works', 'the banker algorithm', 'how TCP congestion control works', 'quicksort', 'how neural networks learn', 'recursion', 'big O notation', 'what a compiler does', 'how vaccines work', 'the CAP theorem', 'how rainbows form'])}",
    lambda: f"show me how {pick(['a mutex works', 'a for loop works', 'binary search works', 'a compiler works', 'sorting works'])}", lambda: f"what does {pick(['sort', 'open', 'deadlock', 'idempotent', 'latency', 'refactor', 'ephemeral'])} mean",
    lambda: f"how do I {pick(['center a div', 'reverse a string in python', 'undo a git commit', 'cook rice', 'write a cover letter', 'learn guitar faster', 'stay focused', 'make a good password'])}",
    lambda: f"do you think {pick(['rust is better than go', 'tabs are better than spaces', 'AI will take my job', 'cats are better than dogs', 'it will rain tomorrow'])}", lambda: pick(["tell me a joke", "tell me something interesting", "give me a fun fact", "say something funny", "recommend a good book", "write me a short poem", "what should I eat tonight", "give me a study tip"]),
    lambda: f"find me a better word for {pick(['happy', 'big', 'said', 'good', 'sad'])}", lambda: f"what's the difference between {pick(['a process and a thread', 'TCP and UDP', 'stack and heap', 'a list and a tuple', 'weather and climate'])}",
    lambda: f"how many {pick(['ounces are in a pound', 'days are in a leap year', 'bits are in a byte', 'minutes are in a day'])}", lambda: f"translate {pick(['good morning', 'thank you', 'where is the station', 'see you soon'])} to {pick(['French', 'Spanish', 'Hindi', 'German', 'Japanese'])}",
    lambda: f"what is {pick(['12 times 13', 'the square root of 144', '15 percent of 80', 'two to the tenth'])}", lambda: f"remind me what we {pick(['talked about', 'said earlier', 'covered yesterday'])}", lambda: "I think three blue one brown should be good",
    lambda: f"I think {pick(['that sounds good', 'that is right', 'I get it now', 'that is confusing', 'I will try that'])}", lambda: f"can you {pick(['explain that again', 'say that slower', 'repeat that', 'be more concise', 'speak up'])}",
]

REPLIES = {
    "screen": ["You're looking at a code editor with a failing test.", "You're in Safari on a page about operating systems.", "There's a PDF open with a diagram of a scheduler.",
               "You're in a terminal window with a build log and an error near the bottom.", "It looks like a spreadsheet with sales figures by month.", "You're reading an email about a meeting tomorrow."],
    "do": ["Done.", "Done.", "All set.", "Done, it's open.", "Opened YouTube.", "Okay.", "There you go."],
    "chat": ["A mutex is a lock that only one thread can hold at a time.", "Recursion is when a function calls itself on a smaller version of the problem.", "TCP slows down when it detects packet loss.",
             "Sure. Which part would you like me to go over?", "Would you like me to tell you about one of those?", "I think so, yes.", "Photosynthesis turns light, water and carbon dioxide into sugar and oxygen."],
    "research": ["It's 18 degrees and cloudy in Paris.", "The match ended two to one.", "Bitcoin is trading near its weekly average.", "Here's what I found about that."],
}
LAST = [None] * 6 + ["chat"] * 2 + ["do"] * 2 + ["screen"] * 2 + ["research"]


def ctx(label_hint=None):
    last = pick(LAST)
    return last, (pick(REPLIES[last]) if last else "")


def build():
    rows = []

    def add(last, reply, text, label):
        if norm(text) in HELD or not text.strip():
            return
        rows.append((last, reply, text, label))

    for _ in range(520):
        add(*ctx(), dress(pick(DO_CORE)()), "do")
    for _ in range(330):
        add("do", pick(REPLIES["do"]), dress(pick(DO_FOLLOW)()), "do")
    for _ in range(90):
        add("chat", pick(REPLIES["chat"]), dress(pick(DO_CORE)()), "do")
    for _ in range(340):
        add(*ctx(), dress(pick(SCREEN_CORE)()), "screen")
    for _ in range(300):
        add("screen", pick(REPLIES["screen"]), dress(pick(SCREEN_FOLLOW)()), "screen")
    for _ in range(300):
        add(*ctx(), dress(pick(RESEARCH_CORE)()), "research")
    for _ in range(640):
        add(*ctx(), dress(pick(CHAT_CORE)()), "chat")
    for _ in range(300):
        last = pick(["chat", "chat", "research"])
        add(last, pick(REPLIES[last]), dress(pick(CHAT_FOLLOW)()), "chat")
    for _ in range(80):  # "yes"/"ok" after the assistant did something: nothing left to do
        add("do", pick(REPLIES["do"]), pick(["yes", "yeah", "okay", "thanks", "great", "perfect", "nice", "cool", "that's good", "thank you"]), "chat")
    return rows


if __name__ == "__main__":
    work = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "laya-work")
    work.mkdir(parents=True, exist_ok=True)
    rows = build()
    for c in load_cases(work / "llm_rows.jsonl"):  # from gen_llm.py, if you ran it
        if norm(c["text"]) not in HELD:
            rows.append((c["last"], c["reply"], c["text"], c["label"]))
    seen, out = set(), []
    for last, reply, text, label in rows:
        k = (last, norm(text))
        if k not in seen:
            seen.add(k)
            out.append({"last": last, "reply": reply, "text": text, "label": label})
    random.shuffle(out)
    write_jsonl(work / "train.jsonl", to_rows(out))
    write_jsonl(work / "eval_proto.jsonl", to_rows(PROTO))
    write_jsonl(work / "eval_real.jsonl", to_rows(REAL))
    print(len(out), "train rows", dict(Counter(c["label"] for c in out)), "| held out:", len(PROTO), "proto,", len(REAL), "real")
