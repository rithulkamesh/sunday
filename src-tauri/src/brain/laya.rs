//! Routing by meaning. `Rules` reads words; this asks a small decision model (Laya) what the user wants, and only when
//! `Rules` has no pattern for it (it said "chat"). The model runs in its own process, `tools/laya-router`, because the
//! app already links sherpa-onnx, which embeds an ONNX Runtime of its own, and two copies in one binary clash.
//! Everything is optional: without the sidecar or the model (scripts/build-laya.sh, scripts/laya/README.md) nothing
//! here does anything and `Rules` decides alone. It only picks a route: `permitted()`, the tiers and the approvals
//! apply to whatever the route leads to, exactly as before.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Route {
    Screen,
    Do,
    Research,
}

impl Route {
    pub fn name(self) -> &'static str {
        match self {
            Route::Screen => "screen",
            Route::Do => "do",
            Route::Research => "research",
        }
    }
}

/// The action a route stands for, for an utterance `Rules` had no pattern for.
pub fn action_for(route: &str, text: &str) -> Option<crate::brain::decision::Action> {
    use crate::brain::decision::Action;
    match route {
        "screen" => Some(Action::Screen(text.to_string())),
        "do" => Some(Action::Do(text.to_string())),
        "research" => Some(Action::Research(text.to_string())),
        _ => None,
    }
}

/// How sure the model must be before it overrides "chat". Looking at the screen is the costly mistake (a screenshot
/// nobody asked for), so it needs the most.
const MIN: [(&str, f64); 3] = [("screen", 0.90), ("do", 0.80), ("research", 0.85)];

/// What the model saw, written exactly as in training (`scripts/laya/gen_routing.py`).
pub fn state(last: Option<&str>, reply: &str, text: &str) -> String {
    match last {
        Some(l) => format!("Last thing the assistant did: {l}. It said: {}\nUser: {text}", reply.chars().take(200).collect::<String>().trim()),
        None => format!("User: {text}"),
    }
}

/// The route to take instead of chat, if the model is sure enough.
pub fn pick(choice: &str, probs: &HashMap<String, f64>) -> Option<Route> {
    let route = match choice {
        "screen" => Route::Screen,
        "do" => Route::Do,
        "research" => Route::Research,
        _ => return None,
    };
    let need = MIN.iter().find(|(n, _)| *n == choice)?.1;
    (probs.get(choice).copied().unwrap_or(0.0) >= need).then_some(route)
}

/// One running sidecar: a line in, a line out.
pub struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
}

impl Sidecar {
    pub fn spawn(cmd: &mut Command) -> std::io::Result<Self> {
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
        let (stdin, stdout) = (child.stdin.take().ok_or(std::io::ErrorKind::BrokenPipe)?, child.stdout.take().ok_or(std::io::ErrorKind::BrokenPipe)?);
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, stdin, lines })
    }

    /// Asks one question. None on any failure or a late answer; the caller drops the sidecar then, because a late line
    /// would otherwise be read as the answer to the next question.
    pub fn ask(&mut self, state: &str, wait: Duration) -> Option<(String, HashMap<String, f64>)> {
        writeln!(self.stdin, "{}", json!({"state": state})).and_then(|_| self.stdin.flush()).ok()?;
        let v: Value = serde_json::from_str(&self.lines.recv_timeout(wait).ok()?).ok()?;
        let probs = v["probs"].as_object()?.iter().filter_map(|(k, p)| Some((k.clone(), p.as_f64()?))).collect();
        Some((v["choice"].as_str()?.to_string(), probs))
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static SIDECAR: Mutex<Option<Sidecar>> = Mutex::new(None);

fn command() -> Option<Command> {
    if std::env::var("SUNDAY_ROUTER").as_deref() == Ok("rules") {
        return None;
    }
    let models = crate::voice::models::dir();
    let (bin, model) = (models.parent()?.join("bin/laya-router"), models.join("laya-routing"));
    (bin.exists() && model.join("model.onnx").exists()).then(|| {
        let mut c = Command::new(bin);
        c.arg(model);
        c
    })
}

/// Starts the sidecar in the background so the first question doesn't wait for the model to load.
pub fn warm() {
    std::thread::spawn(|| {
        let Some(mut cmd) = command() else { return };
        let started = std::time::Instant::now();
        if let Ok(mut s) = Sidecar::spawn(&mut cmd) {
            if s.ask(&state(None, "", "hello"), Duration::from_secs(60)).is_some() {
                eprintln!("[laya] router ready in {:?}", started.elapsed());
                *SIDECAR.lock().unwrap() = Some(s);
            }
        }
    });
}

/// Asks the model, if it is ready. Never waits for it to start; a slow or broken sidecar is dropped and `Rules` decides.
pub fn route(last: Option<&str>, reply: &str, text: &str) -> Option<Route> {
    let mut guard = SIDECAR.try_lock().ok()?;
    let side = guard.as_mut()?;
    let t = std::time::Instant::now();
    match side.ask(&state(last, reply, text), Duration::from_secs(2)) {
        Some((choice, probs)) => {
            let r = pick(&choice, &probs);
            eprintln!("[laya] {choice} {:.2} in {:?} -> {}", probs.get(&choice).copied().unwrap_or(0.0), t.elapsed(), r.map_or("chat", Route::name));
            r
        }
        None => {
            eprintln!("[laya] no answer; falling back to rules");
            *guard = None;
            None
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A stand-in sidecar: answers every line with the same JSON.
    fn fake(answer: &str) -> Sidecar {
        let script = format!("while read l; do echo '{answer}'; done");
        Sidecar::spawn(Command::new("sh").arg("-c").arg(script)).unwrap()
    }

    #[test]
    fn state_matches_what_the_model_was_trained_on() {
        assert_eq!(state(None, "", "hello there"), "User: hello there");
        assert_eq!(state(Some("screen"), "You're in a terminal.", "tell me more"), "Last thing the assistant did: screen. It said: You're in a terminal.\nUser: tell me more");
        assert!(state(Some("chat"), &"x".repeat(500), "why").len() < 300, "long replies are cut");
    }

    #[test]
    fn only_a_sure_model_overrides_chat() {
        let p = |c: &str, v: f64| HashMap::from([(c.to_string(), v)]);
        assert_eq!(pick("do", &p("do", 0.95)), Some(Route::Do));
        assert_eq!(pick("do", &p("do", 0.5)), None);
        assert_eq!(pick("screen", &p("screen", 0.85)), None, "a screenshot nobody asked for is the costly mistake");
        assert_eq!(pick("screen", &p("screen", 0.97)), Some(Route::Screen));
        assert_eq!(pick("chat", &p("chat", 0.99)), None, "chat is already what Rules said");
        assert_eq!(pick("fly", &p("fly", 0.99)), None, "an unknown route is ignored");
    }

    #[test]
    fn talks_to_a_sidecar_and_survives_a_bad_one() {
        let mut s = fake(r#"{"choice":"do","probs":{"chat":0.05,"do":0.95}}"#);
        let (c, p) = s.ask("User: open notes", Duration::from_secs(5)).expect("an answer");
        assert_eq!((c.as_str(), p["do"]), ("do", 0.95));
        assert!(s.ask("User: again", Duration::from_secs(5)).is_some(), "it keeps answering");
        assert!(fake(r#"{"error":"boom"}"#).ask("x", Duration::from_secs(5)).is_none());
        assert!(fake("not json").ask("x", Duration::from_secs(5)).is_none());
        let mut silent = Sidecar::spawn(Command::new("sh").arg("-c").arg("sleep 5")).unwrap();
        assert!(silent.ask("x", Duration::from_millis(100)).is_none(), "a sidecar that doesn't answer in time is dropped by the caller");
    }

    /// The real path end to end: `Rules` first, then the model for what Rules left as chat, on the held-out cases in
    /// `evals/router`. Prints the same numbers as `rules_baseline` in decision.rs, with the model added.
    #[test]
    #[ignore = "needs the routing model and sidecar (scripts/laya/export.sh, scripts/build-laya.sh)"]
    fn router_end_to_end_on_the_held_out_sets() {
        use crate::brain::decision::{Action, Decider, Rules};
        let mut side = Sidecar::spawn(&mut command().expect("sidecar or model not installed")).unwrap();
        assert!(side.ask(&state(None, "", "hello"), Duration::from_secs(120)).is_some(), "the sidecar didn't start");
        for name in ["proto", "real"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../evals/router/{name}.jsonl"));
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            let (mut ok, mut n, mut false_actions, mut rules_ok) = (0, 0, 0, 0);
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let v: Value = serde_json::from_str(line).unwrap();
                let (said, want) = (v["text"].as_str().unwrap(), v["label"].as_str().unwrap());
                let mut got = match Rules.decide(said, false) {
                    Action::Do(_) => "do",
                    Action::Screen(_) => "screen",
                    Action::Research(_) => "research",
                    _ => "chat",
                };
                rules_ok += (got == want) as usize;
                if got == "chat" {
                    let (last, reply) = (v["last"].as_str(), v["reply"].as_str().unwrap_or(""));
                    if let Some((choice, probs)) = side.ask(&state(last, reply, said), Duration::from_secs(10)) {
                        got = pick(&choice, &probs).map_or("chat", Route::name);
                    }
                }
                n += 1;
                ok += (got == want) as usize;
                false_actions += (want == "chat" && got != "chat") as usize;
                if got != want {
                    println!("  {name}: said {got}, wanted {want}: {said}");
                }
            }
            println!("rules + model on {name}: {ok}/{n} right (rules alone {rules_ok}), {false_actions} conversations wrongly sent to an action");
        }
    }
}
