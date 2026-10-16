//! Typed decisions: every utterance becomes exactly one `Action` before anything runs.
//! `Decider` is the seam where a smarter router (Laya, an LLM) plugs in; `Rules` is the
//! deterministic default and also the fallback when a router fails or isn't confident.
use crate::study;

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
    /// Run an approved skill.
    Skill { id: String, input: String },
}

pub trait Decider {
    fn decide(&self, text: &str, has_pending: bool) -> Action;
}

pub struct Rules;

fn norm(t: &str) -> String {
    format!(" {} ", t.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' || c == '.' || c == '_' || c == '-' { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "))
}

impl Decider for Rules {
    fn decide(&self, text: &str, has_pending: bool) -> Action {
        let t = norm(text);
        let has = |ks: &[&str]| ks.iter().any(|k| t.contains(k));
        if has_pending && study::is_skip(text) {
            return Action::Skip;
        }
        if has(&[" learn my voice ", " remember my voice ", " learn my voice. ", " train on my voice ", " recognize my voice ", " recognise my voice "]) {
            return Action::LearnVoice;
        }
        if has(&[" forget my voice ", " reset my voice ", " unlearn my voice "]) {
            return Action::ForgetVoice;
        }
        if has(&[" clear the screen ", " clear my screen ", " clear that ", " erase that ", " hide that ", " clear the drawing ", " remove the drawing "]) {
            return Action::ClearScreen;
        }
        if has(&[" my screen ", " this screen ", " on screen ", " on the screen ", " what i'm looking at ", " what im looking at ", " this page ", " this problem ", " this question ", " explain this ", " what is this ", " what's this ", " point to ", " point at ", " draw on "]) {
            return Action::Screen(text.trim().to_string());
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
        if let Some(topic) = study::quiz_request(text) {
            return Action::Quiz(topic);
        }
        if has_pending {
            Action::Answer
        } else {
            Action::Chat
        }
    }
}

/// Refuses actions that can't run right now; the caller falls back to `Chat` with an explanation.
pub fn permitted(a: &Action, skill_exists: impl Fn(&str) -> bool) -> Result<(), String> {
    match a {
        Action::Skill { id, .. } if !skill_exists(id) => Err(format!("There's no approved skill called {id}.")),
        Action::Skill { input, .. } if input.trim().is_empty() => Err("What should the skill work on?".into()),
        Action::Notes(q) if q.trim().is_empty() => Err("What should I search your notes for?".into()),
        _ => Ok(()),
    }
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
        assert_eq!(d("quiz me on graphs", false), Quiz(Some("graphs".into())));
        assert_eq!(d("it needs four conditions", true), Answer);
        assert_eq!(d("skip", true), Skip);
        assert_eq!(d("skip", false), Chat);
        assert!(matches!(d("Can you explain what's on my screen?", false), Screen(_)));
        assert!(matches!(d("explain this question", true), Screen(_))); // wins over answering
        assert_eq!(d("clear the screen", false), ClearScreen);
        assert_eq!(d("Sunday, learn my voice", false), LearnVoice);
        assert_eq!(d("forget my voice", false), ForgetVoice);
        assert_eq!(d("what do my notes say about deadlock", false), Notes("deadlock".into()));
        assert_eq!(d("run skill study.mnemonic on osi model", false), Skill { id: "study.mnemonic".into(), input: "osi model".into() });
    }

    #[test]
    fn validates_before_running() {
        let known = |id: &str| id == "study.mnemonic";
        assert!(permitted(&Skill { id: "study.mnemonic".into(), input: "x".into() }, known).is_ok());
        assert!(permitted(&Skill { id: "nope".into(), input: "x".into() }, known).is_err());
        assert!(permitted(&Skill { id: "study.mnemonic".into(), input: " ".into() }, known).is_err());
        assert!(permitted(&Chat, known).is_ok());
    }
}
