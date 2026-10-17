//! Where text-model calls go. `Provider` is the seam for other backends (see the `providers` issues);
//! Ollama is the only one so far. Embeddings and the vision model (`screen.rs`) still talk to Ollama directly.
use crate::llm::{Res, UserMsg, KEEP_TEXT};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::time::Duration;

pub trait Provider: Send + Sync {
    /// One whole reply as text. With a schema, the reply is constrained to that JSON shape.
    fn complete(&self, model: &str, messages: &[Value], schema: Option<Value>, timeout_s: u64) -> Res<String>;
    /// Streams the reply token by token; `on_token` returns false to stop early.
    fn stream(&self, model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()>;
    /// Models the backend has; empty if it can't be reached.
    fn models(&self) -> Vec<String>;
    /// Loads `model` ahead of the first question. Best effort.
    fn warm(&self, model: &str);
}

/// The provider in use. Always Ollama until others exist.
pub fn current() -> &'static dyn Provider {
    &Ollama
}

pub const OLLAMA: &str = "http://localhost:11434";
pub const OLLAMA_CHAT: &str = "http://localhost:11434/api/chat";

pub struct Ollama;

/// Posts to /api/chat. Failures become messages fit to be spoken.
fn chat(model: &str, body: &Value, timeout_s: Option<u64>) -> Res<ureq::Response> {
    let mut req = ureq::post(OLLAMA_CHAT);
    if let Some(s) = timeout_s {
        req = req.timeout(Duration::from_secs(s));
    }
    req.send_json(body).map_err(|e| {
        eprintln!("[llm] {e}");
        let msg = match e {
            ureq::Error::Status(404, _) => format!("The model {model} isn't installed. Run ollama pull {model}."),
            _ => "Sorry, I couldn't reach the model. Is Ollama running?".into(),
        };
        UserMsg(msg).into()
    })
}

impl Provider for Ollama {
    fn complete(&self, model: &str, messages: &[Value], schema: Option<Value>, timeout_s: u64) -> Res<String> {
        let mut body = json!({"model": model, "stream": false, "think": false, "keep_alive": KEEP_TEXT, "options": {"temperature": 0}, "messages": messages});
        if let Some(s) = schema {
            body["format"] = s;
        }
        let v: Value = chat(model, &body, Some(timeout_s))?.into_json()?;
        Ok(v["message"]["content"].as_str().unwrap_or("").to_string())
    }

    fn stream(&self, model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()> {
        // think:false is ~5x faster to first word on reasoning models; keep_alive avoids reloading weights.
        let body = json!({"model": model, "messages": messages, "stream": true, "think": false, "keep_alive": KEEP_TEXT});
        let resp = chat(model, &body, None)?;
        for line in BufReader::new(resp.into_reader()).lines() {
            let v: Value = serde_json::from_str(&line?)?;
            if let Some(tok) = v["message"]["content"].as_str().filter(|t| !t.is_empty()) {
                if !on_token(tok) {
                    break; // dropping the response closes the connection; Ollama stops
                }
            }
        }
        Ok(())
    }

    fn models(&self) -> Vec<String> {
        ureq::get(&format!("{OLLAMA}/api/tags"))
            .timeout(Duration::from_secs(3))
            .call()
            .ok()
            .and_then(|r| r.into_json::<Value>().ok())
            .map(|v| v["models"].as_array().map(|a| a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()).unwrap_or_default())
            .unwrap_or_default()
    }

    fn warm(&self, model: &str) {
        let _ = ureq::post(&format!("{OLLAMA}/api/generate"))
            .timeout(Duration::from_secs(120))
            .send_json(json!({"model": model, "prompt": "", "keep_alive": KEEP_TEXT}));
    }
}
