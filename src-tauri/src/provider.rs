//! Where text-model calls go. `Provider` is the seam for other backends. Ollama is the default and the only one
//! that stays on this machine; the others are chosen with SUNDAY_PROVIDER (see `from_env`) and send prompts to a
//! remote API. Embeddings and the vision model (`screen.rs`) still talk to Ollama directly.
use crate::llm::{Res, UserMsg, KEEP_TEXT};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::OnceLock;
use std::time::Duration;

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    /// One whole reply as text. With a schema, the reply is constrained to that JSON shape.
    fn complete(&self, model: &str, messages: &[Value], schema: Option<Value>, timeout_s: u64) -> Res<String>;
    /// Streams the reply token by token; `on_token` returns false to stop early.
    fn stream(&self, model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()>;
    /// Models the backend has; empty if it can't be reached.
    fn models(&self) -> Vec<String>;
    /// Loads `model` ahead of the first question. Best effort.
    fn warm(&self, model: &str);
}

/// The provider in use, fixed for the life of the process.
pub fn current() -> &'static dyn Provider {
    static P: OnceLock<Box<dyn Provider>> = OnceLock::new();
    &**P.get_or_init(|| from_env(|k| std::env::var(k).ok()))
}

/// SUNDAY_PROVIDER = ollama (default) | openai | anthropic | gemini | nova. Anything but ollama sends prompts off
/// this machine, so it is opt-in. Keys come from the usual env vars; OPENAI_BASE_URL points `openai` at OpenRouter,
/// vLLM, LM Studio, a llama.cpp server and the like.
fn from_env(env: impl Fn(&str) -> Option<String>) -> Box<dyn Provider> {
    let get = |k: &str| env(k).unwrap_or_default();
    match env("SUNDAY_PROVIDER").as_deref().unwrap_or("ollama") {
        "ollama" => Box::new(Ollama),
        "openai" => Box::new(OpenAi { base: env("OPENAI_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".into()), key: get("OPENAI_API_KEY") }),
        "gemini" => Box::new(OpenAi { base: "https://generativelanguage.googleapis.com/v1beta/openai".into(), key: get("GEMINI_API_KEY") }),
        "anthropic" => Box::new(Anthropic { base: "https://api.anthropic.com".into(), key: get("ANTHROPIC_API_KEY") }),
        "nova" => Box::new(Nova { base: format!("https://bedrock-runtime.{}.amazonaws.com", env("AWS_REGION").unwrap_or_else(|| "us-east-1".into())), key: get("AWS_BEARER_TOKEN_BEDROCK") }),
        other => {
            eprintln!("[llm] unknown SUNDAY_PROVIDER {other:?}, using ollama");
            Box::new(Ollama)
        }
    }
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
    fn name(&self) -> &'static str {
        "ollama"
    }

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

/// A failed remote call, as a message fit to be spoken.
fn http(e: ureq::Error) -> Box<dyn std::error::Error> {
    eprintln!("[llm] {e}");
    UserMsg(match e {
        ureq::Error::Status(401 | 403, _) => "The provider rejected the API key.".into(),
        ureq::Error::Status(404, _) => "The provider doesn't have that model.".into(),
        ureq::Error::Status(429, _) => "The provider is rate limiting me. Try again in a moment.".into(),
        _ => "Sorry, I couldn't reach the model provider.".to_string(),
    })
    .into()
}

fn need_key(key: &str, var: &str) -> Res<()> {
    if key.is_empty() {
        return Err(UserMsg(format!("Set {var} to use this provider.")).into());
    }
    Ok(())
}

/// Reads `data:` lines of a server-sent-event reply; `pick` pulls the text out of each event.
fn sse(resp: ureq::Response, on_token: &mut dyn FnMut(&str) -> bool, pick: fn(&Value) -> Option<&str>) -> Res<()> {
    for line in BufReader::new(resp.into_reader()).lines() {
        let line = line?;
        let Some(d) = line.strip_prefix("data:").map(str::trim) else { continue };
        if d == "[DONE]" {
            break;
        }
        if let Some(t) = pick(&serde_json::from_str::<Value>(d)?).filter(|t| !t.is_empty()) {
            if !on_token(t) {
                break;
            }
        }
    }
    Ok(())
}

/// System text plus (role, text) turns, for APIs that take the system prompt apart from the messages. These APIs
/// have no schema-constrained output here, so the schema goes in the prompt; `json_via` retries bad JSON.
fn split(messages: &[Value], schema: Option<Value>) -> (String, Vec<(String, String)>) {
    let text = |m: &Value| m["content"].as_str().unwrap_or("").to_string();
    let mut system: Vec<String> = messages.iter().filter(|m| m["role"] == "system").map(text).collect();
    if let Some(s) = schema {
        system.push(format!("Reply with only JSON matching this schema, no prose: {s}"));
    }
    let turns = messages.iter().filter(|m| m["role"] != "system").map(|m| (m["role"].as_str().unwrap_or("user").to_string(), text(m))).collect();
    (system.join("\n\n"), turns)
}

fn model_ids(v: Value) -> Vec<String> {
    v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Any server that speaks OpenAI's chat completions: OpenAI, OpenRouter, vLLM, LM Studio, llama.cpp, and Gemini's
/// compatibility endpoint. The key may be empty for local servers.
pub struct OpenAi {
    base: String,
    key: String,
}

impl OpenAi {
    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let r = ureq::request(method, &format!("{}{path}", self.base));
        if self.key.is_empty() { r } else { r.set("Authorization", &format!("Bearer {}", self.key)) }
    }
}

impl Provider for OpenAi {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn complete(&self, model: &str, messages: &[Value], schema: Option<Value>, timeout_s: u64) -> Res<String> {
        let mut body = json!({"model": model, "messages": messages, "temperature": 0});
        if let Some(s) = schema {
            body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "reply", "schema": s}});
        }
        let v: Value = self.request("POST", "/chat/completions").timeout(Duration::from_secs(timeout_s)).send_json(body).map_err(http)?.into_json()?;
        Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
    }

    fn stream(&self, model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()> {
        let body = json!({"model": model, "messages": messages, "stream": true});
        sse(self.request("POST", "/chat/completions").send_json(body).map_err(http)?, on_token, |v| v["choices"][0]["delta"]["content"].as_str())
    }

    fn models(&self) -> Vec<String> {
        self.request("GET", "/models").timeout(Duration::from_secs(3)).call().ok().and_then(|r| r.into_json().ok()).map(model_ids).unwrap_or_default()
    }

    fn warm(&self, _: &str) {}
}

/// Claude through the Messages API.
pub struct Anthropic {
    base: String,
    key: String,
}

impl Anthropic {
    fn request(&self, method: &str, path: &str) -> ureq::Request {
        ureq::request(method, &format!("{}{path}", self.base)).set("x-api-key", &self.key).set("anthropic-version", "2023-06-01")
    }

    fn body(model: &str, messages: &[Value], schema: Option<Value>, stream: bool) -> Value {
        let (system, turns) = split(messages, schema);
        let turns: Vec<Value> = turns.into_iter().map(|(r, c)| json!({"role": r, "content": c})).collect();
        json!({"model": model, "max_tokens": 4096, "temperature": 0, "system": system, "messages": turns, "stream": stream})
    }
}

impl Provider for Anthropic {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn complete(&self, model: &str, messages: &[Value], schema: Option<Value>, timeout_s: u64) -> Res<String> {
        need_key(&self.key, "ANTHROPIC_API_KEY")?;
        let body = Self::body(model, messages, schema, false);
        let v: Value = self.request("POST", "/v1/messages").timeout(Duration::from_secs(timeout_s)).send_json(body).map_err(http)?.into_json()?;
        Ok(v["content"].as_array().map(|a| a.iter().filter_map(|b| b["text"].as_str()).collect()).unwrap_or_default())
    }

    fn stream(&self, model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()> {
        need_key(&self.key, "ANTHROPIC_API_KEY")?;
        let body = Self::body(model, messages, None, true);
        sse(self.request("POST", "/v1/messages").send_json(body).map_err(http)?, on_token, |v| v["delta"]["text"].as_str())
    }

    fn models(&self) -> Vec<String> {
        self.request("GET", "/v1/models").timeout(Duration::from_secs(3)).call().ok().and_then(|r| r.into_json().ok()).map(model_ids).unwrap_or_default()
    }

    fn warm(&self, _: &str) {}
}

/// Amazon Nova on Bedrock, through the Converse API with a Bedrock API key (AWS_BEARER_TOKEN_BEDROCK).
pub struct Nova {
    base: String,
    key: String,
}

impl Provider for Nova {
    fn name(&self) -> &'static str {
        "nova"
    }

    fn complete(&self, model: &str, messages: &[Value], schema: Option<Value>, timeout_s: u64) -> Res<String> {
        need_key(&self.key, "AWS_BEARER_TOKEN_BEDROCK")?;
        let (system, turns) = split(messages, schema);
        let turns: Vec<Value> = turns.into_iter().map(|(r, c)| json!({"role": r, "content": [{"text": c}]})).collect();
        let body = json!({"system": [{"text": system}], "messages": turns, "inferenceConfig": {"temperature": 0, "maxTokens": 4096}});
        let url = format!("{}/model/{}/converse", self.base, model.replace(':', "%3A"));
        let v: Value = ureq::post(&url).set("Authorization", &format!("Bearer {}", self.key)).timeout(Duration::from_secs(timeout_s)).send_json(body).map_err(http)?.into_json()?;
        Ok(v["output"]["message"]["content"].as_array().map(|a| a.iter().filter_map(|b| b["text"].as_str()).collect()).unwrap_or_default())
    }

    fn stream(&self, model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()> {
        // ponytail: ConverseStream is binary event-stream framing; one token with the whole reply until the first-word delay matters.
        on_token(&self.complete(model, messages, None, 120)?);
        Ok(())
    }

    fn models(&self) -> Vec<String> {
        ["us.amazon.nova-micro-v1:0", "us.amazon.nova-lite-v1:0", "us.amazon.nova-pro-v1:0"].map(String::from).to_vec()
    }

    fn warm(&self, _: &str) {}
}

/// Provider conformance (#39): one set of checks every `Provider` must pass. Each remote provider runs it against a
/// local mock server that answers in that API's wire format, so no key or network is needed. Set SUNDAY_PROVIDER_TEST=1
/// to run it against the real provider chosen by SUNDAY_PROVIDER and SUNDAY_MODEL.
#[cfg(test)]
mod conformance {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serves `reply(path, request_body) -> (content_type, body)` on a free local port; returns its base URL.
    fn mock(reply: fn(&str, &str) -> (&'static str, String)) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut c in l.incoming().flatten() {
                let mut buf = Vec::new();
                let mut b = [0u8; 4096];
                let (head, body_at) = loop {
                    let n = c.read(&mut b).unwrap_or(0);
                    buf.extend_from_slice(&b[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break (String::from_utf8_lossy(&buf[..i]).to_string(), i + 4);
                    }
                    if n == 0 {
                        break (String::new(), 0);
                    }
                };
                let want = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                while buf.len() < body_at + want {
                    let n = c.read(&mut b).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&b[..n]);
                }
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let (ct, body) = reply(&path, &String::from_utf8_lossy(&buf[body_at.min(buf.len())..]));
                let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        base
    }

    const SCHEMA_REPLY: &str = r#"{"ok":true}"#;

    fn check(p: &dyn Provider, dead: &dyn Provider) {
        let msgs = [json!({"role": "system", "content": "be brief"}), json!({"role": "user", "content": "hi"})];
        assert_eq!(p.complete("m1", &msgs, None, 10).unwrap(), "hello");
        let j: Value = serde_json::from_str(&p.complete("m1", &msgs, Some(json!({"type": "object"})), 10).unwrap()).unwrap();
        assert_eq!(j["ok"], true, "schema reply must parse as JSON");
        let mut all = String::new();
        p.stream("m1", &msgs, &mut |t| { all.push_str(t); true }).unwrap();
        assert_eq!(all, "hello");
        let mut n = 0;
        p.stream("m1", &msgs, &mut |_| { n += 1; false }).unwrap();
        assert_eq!(n, 1, "stream must stop when on_token returns false");
        p.models(); // must not panic or hang
        let e = dead.complete("m1", &msgs, None, 2).unwrap_err();
        assert!(e.downcast_ref::<UserMsg>().is_some(), "failures must be spoken messages, got {e}");
        assert!(dead.stream("m1", &msgs, &mut |_| true).is_err());
    }

    fn openai_reply(path: &str, body: &str) -> (&'static str, String) {
        if path.ends_with("/models") {
            return ("application/json", r#"{"data":[{"id":"m1"}]}"#.into());
        }
        if body.contains(r#""stream":true"#) {
            let ev = |t: &str| format!("data: {}\n\n", json!({"choices": [{"delta": {"content": t}}]}));
            return ("text/event-stream", format!("{}{}data: [DONE]\n\n", ev("hel"), ev("lo")));
        }
        let text = if body.contains("json_schema") { SCHEMA_REPLY } else { "hello" };
        ("application/json", json!({"choices": [{"message": {"content": text}}]}).to_string())
    }

    fn anthropic_reply(path: &str, body: &str) -> (&'static str, String) {
        if path.ends_with("/models") {
            return ("application/json", r#"{"data":[{"id":"m1"}]}"#.into());
        }
        if body.contains(r#""stream":true"#) {
            let ev = |t: &str| format!("event: content_block_delta\ndata: {}\n\n", json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": t}}));
            return ("text/event-stream", format!("{}{}", ev("hel"), ev("lo")));
        }
        let text = if body.contains("matching this schema") { SCHEMA_REPLY } else { "hello" };
        ("application/json", json!({"content": [{"type": "text", "text": text}]}).to_string())
    }

    fn nova_reply(path: &str, body: &str) -> (&'static str, String) {
        assert!(path.contains("/model/m1/converse"), "{path}");
        let text = if body.contains("matching this schema") { SCHEMA_REPLY } else { "hello" };
        ("application/json", json!({"output": {"message": {"content": [{"text": text}]}}}).to_string())
    }

    const DEAD: &str = "http://127.0.0.1:1";

    #[test]
    fn openai_compatible() {
        let (p, dead) = (OpenAi { base: mock(openai_reply), key: "k".into() }, OpenAi { base: DEAD.into(), key: String::new() });
        check(&p, &dead);
        assert_eq!(p.models(), ["m1"]);
    }

    #[test]
    fn anthropic() {
        let (p, dead) = (Anthropic { base: mock(anthropic_reply), key: "k".into() }, Anthropic { base: DEAD.into(), key: "k".into() });
        check(&p, &dead);
        assert_eq!(p.models(), ["m1"]);
        let e = Anthropic { base: DEAD.into(), key: String::new() }.complete("m1", &[], None, 2).unwrap_err();
        assert!(e.to_string().contains("ANTHROPIC_API_KEY"), "{e}");
    }

    #[test]
    fn nova() {
        check(&Nova { base: mock(nova_reply), key: "k".into() }, &Nova { base: DEAD.into(), key: "k".into() });
    }

    #[test]
    fn provider_choice() {
        let name = |pairs: &[(&str, &str)]| -> &str {
            let m: std::collections::HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            from_env(|k| m.get(k).cloned()).name()
        };
        assert_eq!(name(&[]), "ollama");
        assert_eq!(name(&[("SUNDAY_PROVIDER", "bogus")]), "ollama");
        assert_eq!(name(&[("SUNDAY_PROVIDER", "openai")]), "openai");
        assert_eq!(name(&[("SUNDAY_PROVIDER", "gemini")]), "openai");
        assert_eq!(name(&[("SUNDAY_PROVIDER", "anthropic")]), "anthropic");
        assert_eq!(name(&[("SUNDAY_PROVIDER", "nova")]), "nova");
    }

    /// Real provider, real network. Costs tokens on remote providers.
    #[test]
    fn live() {
        if std::env::var("SUNDAY_PROVIDER_TEST").is_err() {
            return;
        }
        let model = std::env::var("SUNDAY_MODEL").unwrap_or_else(|_| crate::llm::MODEL.into());
        let msgs = [json!({"role": "user", "content": "Say hello in one word."})];
        let p = current();
        assert!(!p.complete(&model, &msgs, None, 60).unwrap().trim().is_empty());
        let mut got = String::new();
        p.stream(&model, &msgs, &mut |t| { got.push_str(t); true }).unwrap();
        assert!(!got.trim().is_empty());
    }
}
