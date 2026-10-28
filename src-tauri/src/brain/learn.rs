//! Teaching and practice, for any subject: the prompts, what the model must return, a learning plan built on demand,
//! and what the user's words mean.
use crate::store::db::Verdict;
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

/// Sunday's standing instructions: a general agent that happens to be a very good teacher.
pub const IDENTITY: &str = "You are Sunday, a personal AI agent that lives on the user's computer and works with them. You can teach any subject, answer questions, \
look at their screen and point at things, read web pages, and remember what matters. Everything you say is spoken aloud: short plain sentences, no markdown, lists, code blocks or emoji. \
When someone is learning something, explain it simply and concretely in two or three sentences, then check understanding with one quick question. For anything else, just help directly. \
If they want practice, tell them to say \"quiz me\". You can open apps, pages and files, type, and click when asked directly; in plain conversation you have not done anything on their computer, so never claim to have found, opened or clicked something. \
Never say you can't access YouTube, the web, apps or the screen: you can. If a request needs something done on the computer, tell them to ask for it directly, like \"open YouTube\" or \"click the filter button\". \
If they say something you did was wrong or a bug, own it in one short sentence and ask what should have happened; never say you are not a real person.";

pub const QUIZ: &str = "You write one oral question for someone who is learning a subject. It must be answerable out loud in one or two sentences: no tables, code or heavy formulas. \
`answer` is the short reference answer. `concept` is a short lowercase topic name. Test understanding (why, how, compare, apply), \
not just recall of a term, and never put the answer in the question.";

pub const GRADE: &str = "You grade a learner's spoken answer against a reference answer. \
verdict is correct only if the key idea is right, partial only if the answer states part of the key idea correctly, wrong otherwise. \
\"I don't know\", \"no idea\", silence and unrelated answers are always wrong. \
feedback is at most two spoken sentences with no markdown, addressed to the learner as \"you\": if the answer is not correct, say what was missing and state the right answer. \
error_type is one of conceptual, calculation, recall, careless, none. The transcript may contain speech-recognition errors; be lenient about wording.";

/// Asks for a learning path for any topic: concepts in order, each with the earlier concepts it builds on.
pub const PLAN: &str = "You design a learning path for someone new to a topic, whatever it is (a school subject, a skill, a language, a tool, a craft). \
List 6 to 10 concepts from the foundations up (fewer is better than padding), in the order to learn them, covering the topic broadly but practically. Each concept has a short lowercase name \
(2 to 5 words) and `needs`: the names of earlier concepts in your list that it directly builds on (empty for the foundations).";

#[derive(Deserialize, Debug)]
pub struct PlanConcept {
    pub name: String,
    #[serde(default)]
    pub needs: Vec<String>,
}
#[derive(Deserialize, Debug)]
pub struct Plan {
    pub concepts: Vec<PlanConcept>,
}

pub fn plan_schema() -> Value {
    json!({"type": "object", "required": ["concepts"], "properties": {"concepts": {"type": "array", "items": {"type": "object",
        "required": ["name", "needs"], "properties": {"name": {"type": "string"}, "needs": {"type": "array", "items": {"type": "string"}}}}}}})
}

/// A usable plan from what the model returned: names tidied and unique, at most 14, and prerequisites only ever
/// point at earlier concepts (so there are no cycles and the order is always a valid order to learn in).
pub fn plan_from(plan: Plan) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for c in plan.concepts {
        let name = c.name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        if !(2..=60).contains(&name.len()) || out.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let mut needs: Vec<String> = c.needs.iter().map(|n| n.trim().to_lowercase()).filter(|n| out.iter().any(|(m, _)| m == n)).collect();
        needs.dedup();
        out.push((name, needs));
        if out.len() == 14 {
            break;
        }
    }
    out
}

/// "teach me how to play guitar" -> Some("how to play guitar"); also "I want to learn rust", "help me learn calculus".
pub fn teach_request(text: &str) -> Option<String> {
    let t = format!(" {} ", norm(text));
    const LEADS: [&str; 7] = [" teach me ", " i want to learn ", " i'd like to learn ", " i wanna learn ", " help me learn ", " show me how to ", " learn about "];
    let (lead, i) = LEADS.iter().find_map(|l| t.find(l).map(|i| (*l, i)))?;
    let topic = t[i + lead.len()..].trim().trim_start_matches("about ").trim();
    (!topic.is_empty()).then(|| if lead == " show me how to " { format!("how to {topic}") } else { topic.to_string() })
}

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
    fn plans_are_valid_orders_to_learn_in() {
        let c = |name: &str, needs: &[&str]| PlanConcept { name: name.into(), needs: needs.iter().map(|s| s.to_string()).collect() };
        let plan = plan_from(Plan { concepts: vec![
            c("  Basic   Chords ", &[]),
            c("strumming", &["basic chords"]),
            c("barre chords", &["Basic Chords", "strumming", "fingerpicking", "barre chords"]), // later + self refs dropped
            c("basic chords", &[]),                                                              // duplicate dropped
            c("x", &[]),                                                                         // too short
            c("fingerpicking", &["basic chords"]),
        ] });
        assert_eq!(plan.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["basic chords", "strumming", "barre chords", "fingerpicking"]);
        assert_eq!(plan[2].1, vec!["basic chords", "strumming"]);
        let many: Vec<PlanConcept> = (0..30).map(|i| c(&format!("concept {i}"), &[])).collect();
        assert_eq!(plan_from(Plan { concepts: many }).len(), 14);
    }

    #[test]
    fn understands_teaching_requests() {
        assert_eq!(teach_request("Teach me how to play the guitar").as_deref(), Some("how to play the guitar"));
        assert_eq!(teach_request("teach me about graph theory.").as_deref(), Some("graph theory"));
        assert_eq!(teach_request("I want to learn Rust").as_deref(), Some("rust"));
        assert_eq!(teach_request("show me how to bake bread").as_deref(), Some("how to bake bread"));
        assert_eq!(teach_request("help me learn spanish please").as_deref(), Some("spanish please"));
        assert_eq!(teach_request("teach me"), None);
        assert_eq!(teach_request("what is a deadlock"), None);
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
