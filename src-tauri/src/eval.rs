//! Eval harness: runs every task in `evals/*.jsonl` and records pass/fail and time per task.
//! Run with `bun run eval`. Results go to `evals/runs/latest.jsonl` (the run before it is kept as `previous.jsonl`),
//! and the report lists regressions against it. The test fails if any task fails or regresses.
//! Tasks are data, see `evals/README.md`. Router tasks (`say`) and agent tasks (`plan`) need no model; model-backed
//! tasks plug into `run_task`. Agent tasks run in `agent::sandbox`: a fresh fake home folder, a canned web, scripted
//! steps in place of the model, and nothing that would touch the real machine; every task runs twice and must agree.
use crate::agent::{self, sandbox, Out, Step};
use crate::db::Db;
use crate::decision::{Decider, Rules};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, time::Instant};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../evals")
}

/// Returns what the task produced, as a string to compare with `expect`.
fn run_task(t: &Value) -> String {
    if t["plan"].is_array() {
        return run_agent(t);
    }
    let say = t["say"].as_str().unwrap_or("");
    format!("{:?}", Rules.decide(say, t["pending"].as_bool().unwrap_or(false)))
}

/// The agent runs the task's scripted steps (step N on its Nth turn; then finish) against a fake home. The result is
/// the outcome, then what the machine would have been asked to do, with the fake home shown as ~.
fn run_agent(t: &Value) -> String {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    static N: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!("sunday-eval-{}-{}", std::process::id(), N.fetch_add(1, Relaxed)));
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let home = home.canonicalize().unwrap();
    for (rel, text) in t["files"].as_object().into_iter().flatten() {
        let f = home.join(rel);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, text.as_str().unwrap_or("")).unwrap();
    }
    #[cfg(unix)]
    for (rel, target) in t["links"].as_object().into_iter().flatten() {
        std::os::unix::fs::symlink(target.as_str().unwrap_or(""), home.join(rel)).unwrap();
    }
    let db = Db::open(&root.join("sunday.db")).unwrap();
    let steps: Vec<Step> = t["plan"].as_array().unwrap().iter().map(|s| serde_json::from_value(s.clone()).expect("bad step in task")).collect();
    let plan = |st: &agent::State| Ok(steps.get(st.steps as usize - 1).cloned().unwrap_or(Step { tool: "finish".into(), say: "Done.".into(), ..Step::default() }));
    let screen = t["screen"].as_str().unwrap_or("").to_string();
    let look = |_: &str| Ok(screen.clone());
    let locate = |_: &str| Ok((500.0, 500.0));
    let eyes = agent::Eyes { look: &look, locate: &locate };
    let sb = sandbox::Sandbox { home: home.clone(), web: t["web"].as_str().unwrap_or("").to_string(), ..Default::default() };
    let (out, sb) = sandbox::run(sb, || {
        let mut out = agent::start(&db, t["task"].as_str().unwrap_or(""), &plan, &eyes);
        if t["approve"] == true {
            if let (Ok(Out::Ask(_)), Some(st)) = (&out, agent::waiting(&db)) {
                out = agent::approve(&db, st, &plan, &eyes);
            }
        }
        out
    });
    let _ = fs::remove_dir_all(&root);
    let out = match out {
        Ok(Out::Done(s)) => format!("Done({s:?})"),
        Ok(Out::Ask(q)) => format!("Ask({q:?})"),
        Err(e) => format!("Err({e})"),
    };
    let effects = sb.effects.join("; ").replace(home.to_string_lossy().as_ref(), "~");
    format!("{out} | {}", if effects.is_empty() { "none" } else { &effects })
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
            assert_eq!(got, run_task(&t), "task {} is not repeatable", t["id"]);
            let pass = t["expect"].as_str() == Some(got.as_str());
            results.push(json!({"id": t["id"], "pass": pass, "got": got, "expect": t["expect"], "xfail": t["xfail"], "ms": start.elapsed().as_millis() as u64}));
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
    // A task with "xfail": "<issue url>" is a known bug: reported, but it doesn't fail the run until it is fixed.
    for r in results.iter().filter(|r| !r["xfail"].is_null()) {
        println!("{} {} ({})", if r["pass"] == true { "XPASS (fixed? drop the xfail)" } else { "XFAIL" }, r["id"], r["xfail"]);
    }
    results.retain(|r| r["xfail"].is_null() || r["pass"] == true);
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
