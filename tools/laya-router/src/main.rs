//! `laya-router <model dir>`: reads one JSON object per line on stdin, `{"state": "..."}`, and answers one per line on
//! stdout, `{"choice": "do", "probs": {"chat": 0.02, "screen": 0.01, "do": 0.96, "research": 0.01}}`.
//! The question is `question.json`, the same file the training scripts use, so the two cannot drift apart.
use laya::{LayaEngine, Question, QuestionSet};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const QUESTION: &str = include_str!("../question.json");

fn questions() -> Result<QuestionSet, String> {
    let q: Value = serde_json::from_str(QUESTION).map_err(|e| e.to_string())?;
    let q = &q["route"];
    let options: Vec<(String, String)> = q["criteria"].as_object().ok_or("criteria")?.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();
    let question = Question::choice(q["instructions"].as_str().ok_or("instructions")?, options).map_err(|e| e.to_string())?;
    QuestionSet::new().with("route", question).map_err(|e| e.to_string())
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: laya-router <model dir>");
        std::process::exit(2);
    });
    let (engine, questions) = match (LayaEngine::from_directory(&dir), questions()) {
        (Ok(e), Ok(q)) => (e, q),
        (Err(e), _) => return eprintln!("laya-router: can't load {dir}: {e}"),
        (_, Err(e)) => return eprintln!("laya-router: bad question: {e}"),
    };
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let reply = match serde_json::from_str::<Value>(&line).ok().and_then(|v| v["state"].as_str().map(str::to_string)) {
            None => json!({"error": "expected {\"state\": \"...\"}"}),
            Some(state) => match engine.predict(json!(state), &questions) {
                Ok(r) => match r.get("route").and_then(|a| a.as_choice()) {
                    Some(c) => json!({"choice": c.choice, "probs": c.probabilities.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>()}),
                    None => json!({"error": "no answer"}),
                },
                Err(e) => json!({"error": e.to_string()}),
            },
        };
        let mut out = stdout.lock();
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            return;
        }
    }
}
