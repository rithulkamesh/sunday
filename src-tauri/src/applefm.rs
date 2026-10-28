//! Chat on Apple's on-device model (Foundation Models, macOS 26+ with Apple Intelligence): no Ollama, no download,
//! prompts stay on this Mac. The model is reached through `bin/applefm`, a small Swift program that
//! scripts/build-applefm.sh compiles (the framework is Swift only). Each call starts it, writes
//! `{"system","prompt"}` on stdin and reads the reply from stdout. A schema goes in the prompt and `json_part` cuts
//! the JSON out, as with MLX.
use crate::llm::{Res, UserMsg};
use crate::provider::{split, Provider};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// The only model there is; the name exists so Settings has something to show.
pub const MODEL: &str = "apple-on-device";

pub struct AppleFm {
    bin: PathBuf,
}

impl AppleFm {
    pub fn new() -> AppleFm {
        AppleFm::at(crate::models::dir().join("../bin/applefm"))
    }

    pub fn at(bin: PathBuf) -> AppleFm {
        AppleFm { bin }
    }

    fn spawn(&self, arg: Option<&str>) -> Res<Child> {
        if !self.bin.exists() {
            return Err(UserMsg("The Apple model helper isn't built. Run scripts/build-applefm.sh (needs macOS 26 and Xcode).".into()).into());
        }
        Ok(Command::new(&self.bin).args(arg).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?)
    }

    fn run(&self, messages: &[Value], schema: Option<Value>, on_token: &mut dyn FnMut(&str) -> bool) -> Res<String> {
        let (system, turns) = split(messages, schema);
        let mut child = self.spawn(None)?;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(json!({"system": system, "prompt": fold(&turns)}).to_string().as_bytes())?;
        drop(stdin);
        let mut out = child.stdout.take().unwrap();
        let (mut all, mut pending, mut buf, mut cut) = (String::new(), Vec::new(), [0u8; 512], false);
        loop {
            let n = out.read(&mut buf)?;
            if n == 0 {
                break;
            }
            pending.extend_from_slice(&buf[..n]);
            // A character can arrive in two reads: send the whole characters and keep the rest.
            let whole = std::str::from_utf8(&pending).map_or_else(|e| e.valid_up_to(), |_| pending.len());
            if whole == 0 {
                continue;
            }
            let text = String::from_utf8_lossy(&pending[..whole]).into_owned();
            pending.drain(..whole);
            all.push_str(&text);
            if !on_token(&text) {
                cut = true;
                let _ = child.kill();
                break;
            }
        }
        let status = child.wait()?;
        if !cut && !status.success() {
            let mut err = String::new();
            let _ = child.stderr.take().map(|mut e| e.read_to_string(&mut err));
            eprintln!("[applefm] {err}");
            return Err(UserMsg(unavailable(&err)).into());
        }
        Ok(all)
    }
}

/// The model sees one prompt: earlier turns as a transcript, then the question.
fn fold(turns: &[(String, String)]) -> String {
    let [earlier @ .., (_, last)] = turns else { return String::new() };
    if earlier.is_empty() {
        return last.clone();
    }
    let label = |r: &str| if r == "assistant" { "Assistant" } else { "User" };
    let mut s: String = earlier.iter().map(|(r, t)| format!("{}: {t}\n", label(r))).collect();
    s.push_str(&format!("\nContinue the conversation as the assistant. The user now says:\n{last}"));
    s
}

/// Why the helper refused, in words fit to be spoken.
fn unavailable(stderr: &str) -> String {
    if stderr.contains("appleIntelligenceNotEnabled") {
        "Turn on Apple Intelligence in System Settings to use the Apple model.".into()
    } else if stderr.contains("deviceNotEligible") {
        "This Mac can't run the Apple model.".into()
    } else if stderr.contains("modelNotReady") {
        "The Apple model is still downloading. Try again in a few minutes.".into()
    } else {
        "Sorry, the Apple model failed.".into()
    }
}

/// A model that wraps its JSON in prose or a code fence still has the JSON in it.
fn json_part(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(a), Some(b)) if a < b => &text[a..=b],
        _ => text.trim(),
    }
}

impl Provider for AppleFm {
    fn name(&self) -> &'static str {
        "applefm"
    }

    fn complete(&self, _model: &str, messages: &[Value], schema: Option<Value>, _timeout_s: u64) -> Res<String> {
        let wants_json = schema.is_some();
        let text = self.run(messages, schema, &mut |_| true)?;
        Ok(if wants_json { json_part(&text).to_string() } else { text.trim().to_string() })
    }

    fn stream(&self, _model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()> {
        self.run(messages, None, on_token).map(|_| ())
    }

    /// One model, offered only when the helper says this Mac can run it.
    fn models(&self) -> Vec<String> {
        let ok = self.spawn(Some("check")).and_then(|mut c| Ok(c.wait()?.success())).unwrap_or(false);
        if ok { vec![MODEL.into()] } else { vec![] }
    }

    fn warm(&self, _model: &str) {}
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in helper: a shell script that gets the request on stdin like the real one.
    fn fake(name: &str, script: &str) -> AppleFm {
        let p = std::env::temp_dir().join(format!("sunday-applefm-{}-{name}", std::process::id()));
        std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        AppleFm::at(p)
    }

    #[test]
    fn history_is_folded_into_one_prompt() {
        let t = |r: &str, s: &str| (r.to_string(), s.to_string());
        assert_eq!(fold(&[t("user", "hi")]), "hi");
        let p = fold(&[t("user", "hi"), t("assistant", "hello"), t("user", "and you?")]);
        assert!(p.starts_with("User: hi\nAssistant: hello\n") && p.ends_with("and you?"), "{p}");
    }

    #[test]
    fn a_reply_is_read_and_json_is_cut_out_of_chatter() {
        let a = fake("ok", r#"if grep -q 'Reply with only JSON'; then printf 'Sure {"a":1} done'; else printf 'plain é'; fi"#);
        let msgs = [json!({"role": "user", "content": "x"})];
        assert_eq!(a.complete(MODEL, &msgs, Some(json!({"type": "object"})), 5).unwrap(), r#"{"a":1}"#);
        assert_eq!(a.complete(MODEL, &msgs, None, 5).unwrap(), "plain é");
        let mut got = String::new();
        a.stream(MODEL, &msgs, &mut |t| { got.push_str(t); true }).unwrap();
        assert_eq!(got.trim(), "plain é");
        assert_eq!(a.models(), [MODEL]);
    }

    #[test]
    fn a_refusal_becomes_a_spoken_reason_and_a_missing_helper_says_how_to_build_it() {
        let a = fake("no", "echo 'unavailable: appleIntelligenceNotEnabled' >&2; exit 1");
        let e = a.complete(MODEL, &[json!({"role": "user", "content": "x"})], None, 5).unwrap_err();
        assert!(e.downcast_ref::<UserMsg>().is_some_and(|u| u.0.contains("Apple Intelligence")), "{e}");
        assert!(a.models().is_empty());
        let e = AppleFm::at("/nonexistent/applefm".into()).complete(MODEL, &[], None, 5).unwrap_err();
        assert!(e.downcast_ref::<UserMsg>().is_some_and(|u| u.0.contains("build-applefm.sh")), "{e}");
    }

    #[test]
    #[ignore = "needs macOS 26 with Apple Intelligence and bin/applefm from scripts/build-applefm.sh"]
    fn apple_model_answers() {
        let a = AppleFm::new();
        assert_eq!(a.models(), [MODEL]);
        let r = a.complete(MODEL, &[json!({"role": "user", "content": "Reply with the single word ok."})], None, 30).unwrap();
        assert!(r.to_lowercase().contains("ok"), "{r}");
    }
}
