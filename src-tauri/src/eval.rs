//! Eval harness: runs every task in `evals/*.jsonl` and records pass/fail and time per task.
//! Run with `bun run eval`. Results go to `evals/runs/latest.jsonl` (the run before it is kept as `previous.jsonl`),
//! and the report lists regressions against it. The test fails if any task fails or regresses.
//! Today's suite is the deterministic router (no model needed); model-backed tasks plug into `run_task`.
use crate::decision::{Decider, Rules};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, time::Instant};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../evals")
}

/// Returns what the task produced, as a string to compare with `expect`.
fn run_task(t: &Value) -> String {
    let say = t["say"].as_str().unwrap_or("");
    format!("{:?}", Rules.decide(say, t["pending"].as_bool().unwrap_or(false)))
}

fn load(path: &PathBuf) -> Vec<Value> {
    fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[test]
fn eval_suite() {
    let mut results = Vec::new();
    for entry in fs::read_dir(dir()).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "jsonl")) {
        for t in load(&entry.path()) {
            let (start, got) = (Instant::now(), run_task(&t));
            let pass = t["expect"].as_str() == Some(got.as_str());
            results.push(json!({"id": t["id"], "pass": pass, "got": got, "expect": t["expect"], "ms": start.elapsed().as_millis() as u64}));
        }
    }
    let runs = dir().join("runs");
    fs::create_dir_all(&runs).unwrap();
    let (latest, previous) = (runs.join("latest.jsonl"), runs.join("previous.jsonl"));
    let before = load(&latest);
    if latest.exists() {
        fs::rename(&latest, &previous).unwrap();
    }
    fs::write(&latest, results.iter().map(|r| r.to_string() + "\n").collect::<String>()).unwrap();

    let passed = results.iter().filter(|r| r["pass"] == true).count();
    println!("eval: {passed}/{} passed", results.len());
    let was_passing = |id: &Value| before.iter().any(|b| b["id"] == *id && b["pass"] == true);
    let (mut failed, mut regressed) = (vec![], vec![]);
    for r in results.iter().filter(|r| r["pass"] == false) {
        println!("FAIL {}: expected {} got {}", r["id"], r["expect"], r["got"]);
        failed.push(r["id"].clone());
        if was_passing(&r["id"]) {
            regressed.push(r["id"].clone());
        }
    }
    if !regressed.is_empty() {
        println!("REGRESSIONS vs previous run: {regressed:?}");
    }
    assert!(failed.is_empty(), "{} eval tasks failed", failed.len());
}
