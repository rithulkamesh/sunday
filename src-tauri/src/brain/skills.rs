//! Skills: versioned, declarative capabilities. A skill is a prompt template plus an output
//! schema and tests. There is no generated code and no tool access, so a skill can't do more
//! than ask the local model a question; promotion to `approved` needs passing tests and an explicit yes.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Test {
    pub input: String,
    /// output key -> text that must appear in it (case-insensitive)
    pub expect: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Manifest {
    pub id: String,
    pub version: i64,
    pub purpose: String,
    /// Uses `{input}` for what the user passed in.
    pub prompt: String,
    /// Names of the string fields the skill returns.
    pub output: Vec<String>,
    pub timeout_s: u64,
    pub tests: Vec<Test>,
    /// proposed -> experimental -> approved; deprecated and blocked are terminal.
    pub status: String,
}

impl Manifest {
    pub fn schema(&self) -> Value {
        let props: serde_json::Map<String, Value> = self.output.iter().map(|k| (k.clone(), json!({"type": "string"}))).collect();
        json!({"type": "object", "required": self.output, "properties": props})
    }

    /// Checks the output has every declared field as non-empty text.
    pub fn validate(&self, out: &Value) -> Result<(), String> {
        for k in &self.output {
            match out.get(k).and_then(Value::as_str) {
                Some(s) if !s.trim().is_empty() => {}
                _ => return Err(format!("output is missing '{k}'")),
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub struct TestResult {
    pub input: String,
    pub pass: bool,
    pub detail: String,
}

pub fn check(test: &Test, out: &Value) -> TestResult {
    let miss: Vec<String> = test
        .expect
        .iter()
        .filter(|(k, want)| !out.get(k.as_str()).and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(&want.to_lowercase())))
        .map(|(k, want)| format!("{k} should mention '{want}'"))
        .collect();
    TestResult { input: test.input.clone(), pass: miss.is_empty(), detail: miss.join("; ") }
}

pub fn render(m: &Manifest, input: &str) -> String {
    m.prompt.replace("{input}", input)
}

/// Turns a model-drafted spec into a stored, unapproved manifest. Ids are namespaced so a draft can never shadow a built-in.
pub fn from_draft(d: &Draft) -> Manifest {
    let slug: String = d.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').to_string();
    Manifest {
        id: format!("user.{slug}"),
        version: 1,
        purpose: d.purpose.clone(),
        prompt: if d.prompt.contains("{input}") { d.prompt.clone() } else { format!("{}\n\nInput: {{input}}", d.prompt) },
        output: d.output.iter().map(|o| o.to_lowercase().replace(' ', "_")).collect(),
        timeout_s: 60,
        tests: d.tests.iter().map(|t| Test { input: t.input.clone(), expect: [(t.key.to_lowercase().replace(' ', "_"), t.contains.clone())].into() }).collect(),
        status: "proposed".into(),
    }
}

/// The promotion gate: which status changes are allowed. Approval needs a passing test run on that exact version.
pub fn transition(from: &str, to: &str, tests_passed: bool) -> Result<(), String> {
    match (from, to) {
        ("proposed", "experimental") | ("experimental", "proposed") => Ok(()),
        ("experimental", "approved") if tests_passed => Ok(()),
        ("experimental", "approved") => Err("run the tests first: a skill needs a passing test run before approval".into()),
        ("approved", "experimental") => Ok(()),
        (_, "deprecated" | "blocked") => Ok(()),
        ("proposed", "approved") => Err("a proposed skill must be reviewed and tested before approval".into()),
        (f, t) => Err(format!("can't go from {f} to {t}")),
    }
}

#[derive(Deserialize, Debug)]
pub struct Draft {
    pub name: String,
    pub purpose: String,
    pub prompt: String,
    pub output: Vec<String>,
    pub tests: Vec<DraftTest>,
}
#[derive(Deserialize, Debug)]
pub struct DraftTest {
    pub input: String,
    pub key: String,
    pub contains: String,
}

pub const DRAFT_SYSTEM: &str = "You design small prompt-only skills for a personal assistant. A skill takes one text input and returns \
a few short text fields. `name` is short (2-3 words). `prompt` instructs a model and must contain {input}. `output` lists 1-3 \
field names. Give 2 `tests`: an example input, one output field `key`, and a word or phrase `contains` that any correct answer must include.";

pub fn draft_schema() -> Value {
    json!({"type": "object", "required": ["name", "purpose", "prompt", "output", "tests"], "properties": {
        "name": {"type": "string"}, "purpose": {"type": "string"}, "prompt": {"type": "string"},
        "output": {"type": "array", "items": {"type": "string"}},
        "tests": {"type": "array", "items": {"type": "object", "required": ["input", "key", "contains"],
            "properties": {"input": {"type": "string"}, "key": {"type": "string"}, "contains": {"type": "string"}}}}}})
}

/// Skills that ship with the app, as experimental so the test -> approve flow is visible.
pub fn builtins() -> Vec<Manifest> {
    let t = |input: &str, key: &str, want: &str| Test { input: input.into(), expect: [(key.to_string(), want.to_string())].into() };
    vec![
        Manifest {
            id: "learn.mnemonic".into(),
            version: 1,
            purpose: "Make a memorable mnemonic for a list or concept.".into(),
            prompt: "Create a short, vivid mnemonic to remember: {input}. Also say in one sentence how it maps to the material.".into(),
            output: vec!["mnemonic".into(), "mapping".into()],
            timeout_s: 60,
            tests: vec![t("the seven layers of the OSI model", "mapping", "layer"), t("deadlock conditions", "mnemonic", "")],
            status: "experimental".into(),
        },
        Manifest {
            id: "learn.summarize".into(),
            version: 1,
            purpose: "Summarize a passage into a one-line gist and a quick recall prompt.".into(),
            prompt: "Summarize this text in one sentence, then write one question that checks recall of it:\n{input}".into(),
            output: vec!["gist".into(), "recall_question".into()],
            timeout_s: 60,
            tests: vec![t("A process is a program in execution; a thread is a lightweight unit of execution inside a process.", "gist", "process")],
            status: "experimental".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_outputs_and_tests() {
        let m = &builtins()[0];
        assert!(m.validate(&json!({"mnemonic": "x", "mapping": "y"})).is_ok());
        assert!(m.validate(&json!({"mnemonic": "x"})).is_err());
        assert!(m.validate(&json!({"mnemonic": "x", "mapping": " "})).is_err());
        let r = check(&m.tests[0], &json!({"mnemonic": "x", "mapping": "Each Layer is a word"}));
        assert!(r.pass, "{}", r.detail);
        assert!(!check(&m.tests[0], &json!({"mapping": "nothing"})).pass);
        assert_eq!(render(m, "OSI"), m.prompt.replace("{input}", "OSI"));
    }

    #[test]
    fn promotion_gate() {
        assert!(transition("experimental", "approved", true).is_ok());
        assert!(transition("experimental", "approved", false).is_err());
        assert!(transition("proposed", "approved", true).is_err()); // can't skip review
        assert!(transition("approved", "deprecated", false).is_ok()); // rollback is always allowed
        assert!(transition("deprecated", "approved", true).is_err());
    }

    #[test]
    fn drafts_become_namespaced_unapproved_skills() {
        let d: Draft = serde_json::from_value(json!({"name": "Big-O Coach!", "purpose": "p", "prompt": "Explain {input}", "output": ["Tip"],
            "tests": [{"input": "sorting", "key": "Tip", "contains": "n log n"}]})).unwrap();
        let m = from_draft(&d);
        assert_eq!((m.id.as_str(), m.status.as_str()), ("user.big-o-coach", "proposed"));
        assert_eq!(m.output, vec!["tip"]);
        assert!(m.tests[0].expect.contains_key("tip"));
        let d2: Draft = serde_json::from_value(json!({"name": "x", "purpose": "p", "prompt": "no placeholder", "output": ["a"], "tests": []})).unwrap();
        assert!(from_draft(&d2).prompt.contains("{input}"));
    }
}
