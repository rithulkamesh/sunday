//! The study loop's pure parts: what the model must return, and what the user's words mean.
use crate::db::Verdict;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize, Debug)]
pub struct Quiz {
    pub concept: String,
    pub question: String,
    pub answer: String,
}

#[derive(Deserialize, Debug)]
pub struct Grade {
    pub verdict: Verdict,
    pub feedback: String,
    #[serde(default)]
    pub error_type: String,
}

pub const TUTOR: &str = "You are Sunday, a voice tutor for a student preparing for GATE (CS & IT and Data Science & AI). \
Explain simply and concretely in two or three short spoken sentences, then check understanding with one quick question. \
Plain words only: no markdown, lists, code or emoji. If they want practice, tell them to say \"quiz me\".";

pub const QUIZ: &str = "You write one oral exam question for a GATE (CS & IT / Data Science & AI) student. \
It must be answerable out loud in one or two sentences: no tables, code or heavy formulas. \
`answer` is the short reference answer. `concept` is a short lowercase topic name. Test understanding (why, how, compare, apply), \
not just recall of a term, and never put the answer in the question.";

pub const GRADE: &str = "You grade a student's spoken answer against a reference answer. \
verdict is correct only if the key idea is right, partial only if the answer states part of the key idea correctly, wrong otherwise. \
\"I don't know\", \"no idea\", silence and unrelated answers are always wrong. \
feedback is at most two spoken sentences with no markdown, addressed to the student as \"you\": if the answer is not correct, say what was missing and state the right answer. \
error_type is one of conceptual, calculation, recall, careless, none. The transcript may contain speech-recognition errors; be lenient about wording.";

pub fn quiz_schema() -> Value {
    json!({"type": "object", "required": ["concept", "question", "answer"],
        "properties": {"concept": {"type": "string"}, "question": {"type": "string"}, "answer": {"type": "string"}}})
}

pub fn grade_schema() -> Value {
    json!({"type": "object", "required": ["verdict", "feedback", "error_type"],
        "properties": {"verdict": {"type": "string", "enum": ["correct", "partial", "wrong"]},
            "feedback": {"type": "string"},
            "error_type": {"type": "string", "enum": ["conceptual", "calculation", "recall", "careless", "none"]}}})
}

fn norm(text: &str) -> String {
    text.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' }).collect::<Vec<_>>().iter().collect::<String>()
        .split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "quiz me on graphs" -> Some(Some("graphs")); "quiz me" / "next question" -> Some(None); otherwise None.
pub fn quiz_request(text: &str) -> Option<Option<String>> {
    let t = format!(" {} ", norm(text));
    const TRIGGERS: [&str; 8] =
        [" quiz me ", " test me ", " start a quiz ", " ask me a question ", " practice ", " next question ", " another question ", " another one "];
    TRIGGERS.iter().find(|k| t.contains(**k))?;
    let topic = [" on ", " about ", " for "].iter().find_map(|k| t.split_once(k).map(|(_, rest)| rest.trim().to_string()));
    Some(topic.filter(|s| !s.is_empty()))
}

/// Drops the pending question without grading.
pub fn is_skip(text: &str) -> bool {
    matches!(norm(text).as_str(), "skip" | "skip it" | "skip this" | "never mind" | "cancel" | "forget it" | "pass")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn understands_quiz_requests() {
        assert_eq!(quiz_request("Quiz me."), Some(None));
        assert_eq!(quiz_request("Sunday, quiz me on binary search trees!"), Some(Some("binary search trees".into())));
        assert_eq!(quiz_request("can we practice conditional probability"), Some(None)); // no "on/about": model picks
        assert_eq!(quiz_request("Next question please"), Some(None));
        assert_eq!(quiz_request("what is a deadlock"), None);
        assert!(is_skip("Skip.") && is_skip("never mind") && !is_skip("skip lists are linked lists"));
    }

    #[test]
    fn parses_model_json() {
        let g: Grade = serde_json::from_str(r#"{"verdict":"partial","feedback":"Close.","error_type":"recall"}"#).unwrap();
        assert_eq!(g.verdict, Verdict::Partial);
        assert!(serde_json::from_str::<Grade>(r#"{"verdict":"maybe","feedback":"x"}"#).is_err()); // invalid verdict rejected
        let q: Quiz = serde_json::from_str(r#"{"concept":"dp","question":"q","answer":"a"}"#).unwrap();
        assert_eq!(q.concept, "dp");
    }
}
