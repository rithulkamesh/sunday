//! Typed decisions: every utterance becomes exactly one `Action` before anything runs.
//! `Decider` is the seam where a smarter router (Laya, an LLM) plugs in; `Rules` is the
//! deterministic default and also the fallback when a router fails or isn't confident.
use crate::learn;

#[derive(Debug, PartialEq, Clone)]
pub enum Action {
    /// Plain conversation.
    Chat,
    /// Ask a question (optionally on a topic).
    Quiz(Option<String>),
    /// The utterance answers the pending question.
    Answer,
    /// Drop the pending question.
    Skip,
    /// Look at the screen and explain/point (the remaining words are the task).
    Screen(String),
    /// Erase the drawing overlay.
    ClearScreen,
    /// Which of the user's notes mention this?
    Notes(String),
    /// Learn / forget the owner's voice.
    LearnVoice,
    ForgetVoice,
    /// Teach a subject from scratch: build a learning plan and start the first lesson.
    Teach(String),
    /// Read a web page (None: the one open in the front browser) and remember what it says.
    LearnPage(Option<String>),
    /// Search the web, read the best results, answer from them.
    Research(String),
    /// Store something the user said, as they said it.
    Remember(String),
    /// Do something on the computer (open an app, a page, a file; type; look things up). Each change asks first.
    Do(String),
    /// Run an approved skill.
    Skill { id: String, input: String },
}

pub trait Decider {
    fn decide(&self, text: &str, has_pending: bool) -> Action;
}

pub struct Rules;

/// The first http(s) address in the text, without trailing punctuation.
fn first_url(text: &str) -> Option<String> {
    text.split_whitespace().find(|w| w.starts_with("http://") || w.starts_with("https://")).map(|w| w.trim_end_matches(['.', ',', ')', '!', '?', ';']).to_string())
}

fn norm(t: &str) -> String {
    format!(" {} ", t.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' || c == '.' || c == '_' || c == '-' { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "))
}

impl Decider for Rules {
    fn decide(&self, text: &str, has_pending: bool) -> Action {
        let t = norm(text);
        let has = |ks: &[&str]| ks.iter().any(|k| t.contains(k));
        if has_pending && learn::is_skip(text) {
            return Action::Skip;
        }
        if has(&[" learn my voice ", " remember my voice ", " learn my voice. ", " train on my voice ", " recognize my voice ", " recognise my voice "]) {
            return Action::LearnVoice;
        }
        if has(&[" forget my voice ", " reset my voice ", " unlearn my voice "]) {
            return Action::ForgetVoice;
        }
        if let Some(url) = first_url(text).filter(|_| has(&[" read ", " learn ", " save ", " remember ", " summarize ", " summarise ", " add "])) {
            return Action::LearnPage(Some(url));
        }
        if has(&[" read this page ", " learn this page ", " remember this page ", " save this page ", " learn from this page ", " read this tab ", " learn this tab ", " save this tab ",
            " remember this tab ", " read this article ", " learn this article ", " save this article ", " read the page i'm on ", " learn the page i'm on "]) {
            return Action::LearnPage(None);
        }
        for lead in [" research ", " look up ", " search the web for ", " search online for ", " search the internet for ", " find out about ", " find out "] {
            if let Some((_, q)) = t.split_once(lead) {
                return Action::Research(q.trim().to_string());
            }
        }
        // Original wording and casing are kept: "My Piano Exam" shouldn't be lowercased.
        let ascii = text.to_ascii_lowercase();
        for lead in ["remember that ", "note that ", "keep in mind that ", "make a note that "] {
            if let Some(i) = ascii.find(lead) {
                return Action::Remember(text[i + lead.len()..].trim().trim_end_matches(['.', '!']).to_string());
            }
        }
        if has(&[" clear the screen ", " clear my screen ", " clear that ", " erase that ", " hide that ", " clear the drawing ", " remove the drawing "]) {
            return Action::ClearScreen;
        }
        if has(&[" my screen ", " this screen ", " on screen ", " on the screen ", " what i'm looking at ", " what im looking at ", " this page ", " this problem ", " this question ", " explain this ", " what is this ", " what's this ", " point to ", " point at ", " draw on "]) {
            // "sort what is on my screen by the best": the screen is where the command applies, not the question.
            if let Some(task) = computer_task(text).filter(|_| has(&[" on my screen ", " on the screen ", " on screen "])) {
                return Action::Do(task);
            }
            return Action::Screen(fix_heard(text.trim()));
        }
        for lead in [" run skill ", " use skill ", " use the skill "] {
            if let Some(rest) = t.trim_start().strip_prefix(lead.trim_start()).map(str::trim) {
                let mut it = rest.splitn(2, ' ');
                if let Some(id) = it.next().filter(|i| !i.is_empty()) {
                    return Action::Skill { id: id.to_string(), input: it.next().unwrap_or("").trim_start_matches("on ").to_string() };
                }
            }
        }
        for lead in [" search my notes for ", " search my notes about ", " find in my notes ", " what do my notes say about ", " what do my notes say on "] {
            if let Some((_, q)) = t.split_once(lead) {
                return Action::Notes(q.trim().to_string());
            }
        }
        if let Some(topic) = learn::teach_request(text) {
            return Action::Teach(topic);
        }
        if let Some(topic) = learn::quiz_request(text) {
            return Action::Quiz(topic);
        }
        if let Some(task) = computer_task(text) {
            return Action::Do(task);
        }
        if has_pending {
            Action::Answer
        } else {
            Action::Chat
        }
    }
}

/// "open Safari", "can you go to github.com", "type hello": a request to act, not to talk.
/// ponytail: leading-verb rules; a smarter router can decide the rest.
fn computer_task(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches(['.', '!', '?']);
    let mut rest = t.to_lowercase();
    // "Sandey, can you open..." — the wake word is often misheard; a short first word before a comma is addressing, not content.
    if let Some((first, after)) = rest.split_once(", ") {
        if !first.contains(' ') && first.len() <= 8 {
            rest = after.to_string();
        }
    }
    rest = rest.replace(',', "");
    for lead in ["sunday ", "uh ", "um ", "so ", "i said ", "i meant ", "i mean ", "no ", "actually ", "now ", "then ", "and ", "just ", "please ", "can you ", "could you ", "would you ", "i want you to ", "go ahead and "] {
        while let Some(r) = rest.strip_prefix(lead) {
            rest = r.to_string();
        }
    }
    // "show me YouTube videos on...", "play some jazz", "watch a talk on...": a request to open something, not a question.
    // "show me how a mutex works" is still conversation, so "show me / find me / search" need a place or kind of media.
    let media = ["youtube", "video", "image", "picture", "photo", "website", "google", "spotify"];
    let show = ["show me ", "find me ", "search "].iter().any(|v| rest.starts_with(v)) && media.iter().any(|m| rest.contains(m));
    (show || ["open ", "click ", "select ", "launch ", "go to ", "type ", "navigate to ", "pull up ", "bring up ", "play ", "watch ", "sort ", "filter ", "scroll ", "switch to ", "go back", "refresh", "reload", "pause "].iter().any(|v| rest.starts_with(v))).then(|| t.to_string())
}

/// Refuses actions that can't run right now; the caller falls back to `Chat` with an explanation.
pub fn permitted(a: &Action, skill_exists: impl Fn(&str) -> bool, web_on: bool) -> Result<(), String> {
    match a {
        Action::LearnPage(_) | Action::Research(_) if !web_on => Err("Web access is off. Turn it on in Settings, then ask again.".into()),
        Action::Teach(t) if t.trim().len() < 2 => Err("What would you like to learn?".into()),
        Action::Research(q) if q.trim().is_empty() => Err("What should I look up?".into()),
        Action::Remember(t) if t.trim().len() < 3 => Err("What should I remember?".into()),
        Action::Skill { id, .. } if !skill_exists(id) => Err(format!("There's no approved skill called {id}.")),
        Action::Skill { input, .. } if input.trim().is_empty() => Err("What should the skill work on?".into()),
        Action::Notes(q) if q.trim().is_empty() => Err("What should I search your notes for?".into()),
        _ => Ok(()),
    }
}

/// Lookahead correction for speech-to-text: a word that sounds like "what is" right before an
/// unmistakable anchor ("on my screen") is almost certainly "what is" ("water on my screen").
/// ponytail: one anchor family, consonant-skeleton match; add anchors/heads as mishearings show up.
fn fix_heard(text: &str) -> String {
    let skel = |w: &str| -> String { w.to_lowercase().chars().filter(|c| c.is_alphabetic() && !"aeiouh".contains(*c)).collect() };
    let dist = |a: &str, b: &str| -> usize {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut prev = row[0];
            row[0] = i;
            for j in 1..=b.len() {
                let cur = row[j];
                row[j] = (prev + (a[i - 1] != b[j - 1]) as usize).min(row[j] + 1).min(row[j - 1] + 1);
                prev = cur;
            }
        }
        row[b.len()]
    };
    let mut words: Vec<String> = text.split_whitespace().map(String::from).collect();
    for i in 0..words.len().saturating_sub(2) {
        let anchor = ["on", "in"].contains(&words[i + 1].to_lowercase().as_str())
            && ["my", "the"].contains(&words[i + 2].to_lowercase().as_str())
            && words.get(i + 3).is_some_and(|w| w.to_lowercase().starts_with("screen"));
        let w = words[i].trim_matches(|c: char| !c.is_alphabetic());
        if anchor && w.len() >= 3 && !["what", "what's", "whats", "that", "this", "anything"].contains(&w.to_lowercase().as_str()) && dist(&skel(w), "wts") <= 1 {
            words[i] = if w.chars().next().is_some_and(char::is_uppercase) { "What is".into() } else { "what is".into() };
        }
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    fn d(t: &str, pending: bool) -> Action {
        Rules.decide(t, pending)
    }

    #[test]
    fn routes_utterances() {
        assert_eq!(d("hello there", false), Chat);
        assert_eq!(d("Sunday, teach me how to play guitar", false), Teach("how to play guitar".into()));
        assert_eq!(d("I want to learn linear algebra", false), Teach("linear algebra".into()));
        assert_eq!(d("learn this page", false), LearnPage(None)); // reading a page isn't a lesson
        assert_eq!(d("quiz me on graphs", false), Quiz(Some("graphs".into())));
        assert_eq!(d("it needs four conditions", true), Answer);
        assert_eq!(d("skip", true), Skip);
        assert_eq!(d("skip", false), Chat);
        assert!(matches!(d("Can you explain what's on my screen?", false), Screen(_)));
        assert!(matches!(d("explain this question", true), Screen(_))); // wins over answering
        assert_eq!(d("Water on my screen", false), Screen("What is on my screen".into()));
        assert_eq!(fix_heard("explain water on my screen"), "explain what is on my screen");
        assert_eq!(fix_heard("what's on my screen"), "what's on my screen");
        assert_eq!(fix_heard("look at the writing on my screen"), "look at the writing on my screen");
        assert_eq!(d("clear the screen", false), ClearScreen);
        assert_eq!(d("Can you open Safari?", false), Do("Can you open Safari".into()));
        assert_eq!(d("open question: what is a mutex", false), Do("open question: what is a mutex".into())); // ponytail: verb rule, the model's plan will just finish
        assert_eq!(d("what does open mean", false), Chat);
        assert!(matches!(d("Sunday can you show me YouTube videos on machine learning?", false), Do(_)));
        assert!(matches!(d("play some jazz on spotify", false), Do(_)));
        assert!(matches!(d("Uh can you sort by the best videos?", false), Do(_)), "a follow-up on what is on screen is an action");
        assert!(matches!(d("scroll down a bit", false), Do(_)));
        assert!(matches!(d("Sandhi, can you sort what is on my screen by the best?", false), Do(_)), "a command applied to what is on screen acts");
        assert!(matches!(d("click the button on my screen", false), Do(_)));
        assert!(matches!(d("explain what is on my screen", false), Screen(_)), "no operating verb: still a question about the screen");
        assert!(matches!(d("I said can you sort by the best?", false), Do(_)), "a repeated command is still a command");
        assert!(matches!(d("no, open Notes", false), Do(_)));
        assert!(matches!(d("sunday, um, go back", false), Do(_)));
        assert_eq!(d("what does sort mean", false), Chat);
        assert!(matches!(d("search youtube for rust tutorials", false), Do(_)));
        assert_eq!(d("show me how a mutex works", false), Chat);
        assert_eq!(d("find me a better word for happy", false), Chat);
        assert!(matches!(d("Sandey, can you open one with three colours?", false), Do(_)));
        assert!(matches!(d("click on the blue one", false), Do(_)));
        assert_eq!(d("Sunday, remember that my driving test is on 8 February.", false), Remember("my driving test is on 8 February".into()));
        assert_eq!(d("learn this page", false), LearnPage(None));
        assert_eq!(d("explain this page", false), Screen("explain this page".into())); // looking at it is not saving it
        assert_eq!(d("please read https://example.com/os-notes.", false), LearnPage(Some("https://example.com/os-notes".into())));
        assert_eq!(d("https://example.com is down", false), Chat); // a link without a verb is just conversation
        assert_eq!(d("research the banker's algorithm", false), Research("the banker's algorithm".into()));
        assert_eq!(d("look up how tcp congestion control works", false), Research("how tcp congestion control works".into()));
        assert_eq!(d("Sunday, learn my voice", false), LearnVoice);
        assert_eq!(d("forget my voice", false), ForgetVoice);
        assert_eq!(d("what do my notes say about deadlock", false), Notes("deadlock".into()));
        assert_eq!(d("run skill learn.mnemonic on osi model", false), Skill { id: "learn.mnemonic".into(), input: "osi model".into() });
    }

    #[test]
    fn validates_before_running() {
        let known = |id: &str| id == "learn.mnemonic";
        assert!(permitted(&Skill { id: "learn.mnemonic".into(), input: "x".into() }, known, false).is_ok());
        assert!(permitted(&Skill { id: "nope".into(), input: "x".into() }, known, false).is_err());
        assert!(permitted(&Skill { id: "learn.mnemonic".into(), input: " ".into() }, known, false).is_err());
        assert!(permitted(&Chat, known, false).is_ok());
        // The web is opt-in: nothing leaves the machine until it's switched on.
        assert!(permitted(&Research("deadlock".into()), known, false).unwrap_err().contains("Web access is off"));
        assert!(permitted(&LearnPage(None), known, false).is_err());
        assert!(permitted(&Research("deadlock".into()), known, true).is_ok());
        assert!(permitted(&Remember("x".into()), known, false).is_err());
    }
}

#[cfg(test)]
mod held_out {
    use super::*;

    /// How `Rules` alone does on the router's held-out cases (`evals/router`), the number a learned router has to beat.
    /// Rules sees only the words, so a follow-up that depends on the last turn is out of its reach by design.
    /// `real.jsonl` is the maintainer's own transcripts and isn't in git; without it only the shared cases run.
    #[test]
    fn rules_baseline() {
        let route = |a: Action| match a {
            Action::Do(_) => "do",
            Action::Screen(_) => "screen",
            Action::Research(_) => "research",
            _ => "chat",
        };
        for name in ["proto", "real"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../evals/router/{name}.jsonl"));
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            let (mut ok, mut n, mut false_actions) = (0, 0, 0);
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let v: serde_json::Value = serde_json::from_str(line).unwrap();
                let (said, want) = (v["text"].as_str().unwrap(), v["label"].as_str().unwrap());
                let got = route(Rules.decide(said, false));
                n += 1;
                ok += (got == want) as usize;
                false_actions += (want == "chat" && got != "chat") as usize;
                if got != want {
                    println!("  {name}: rules said {got}, wanted {want}: {said}");
                }
            }
            println!("rules on {name}: {ok}/{n} right, {false_actions} conversations wrongly sent to an action");
            assert_eq!(false_actions, 0, "{name}: Rules must never turn conversation into an action");
        }
    }
}
