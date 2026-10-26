//! Where text-model calls go. `Provider` is the seam for other backends. Ollama is the default and the only one
//! that stays on this machine; the others are chosen with SUNDAY_PROVIDER (see `from_env`) and send prompts to a
//! remote API. Embeddings and the vision model (`screen.rs`) still talk to Ollama directly.
use crate::llm::{Res, UserMsg, KEEP_TEXT};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use crate::db::Db;
use std::sync::{Arc, RwLock};
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

/// Which provider answers which job. Chat covers replies, quizzes, grading and skills; the planner picks the next step
/// when Sunday acts on the computer. Vision and embeddings are Ollama only; the router is rules, not a model.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Role {
    Chat,
    Planner,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "chat" => Some(Role::Chat),
            "planner" => Some(Role::Planner),
            _ => None,
        }
    }
}

type Pair = (Arc<dyn Provider>, Arc<dyn Provider>);
static ROLES: RwLock<Option<Pair>> = RwLock::new(None);

fn pair() -> Pair {
    if let Some(p) = ROLES.read().unwrap().clone() {
        return p;
    }
    let p = roles(&|_| None, std::env::var("SUNDAY_PROVIDER").ok(), &|k| std::env::var(k).ok());
    *ROLES.write().unwrap() = Some(p.clone());
    p
}

pub fn current() -> Arc<dyn Provider> {
    pair().0
}

pub fn planner() -> Arc<dyn Provider> {
    pair().1
}

pub fn ollama_models() -> Vec<String> {
    Ollama.models()
}

/// Rebuilds both providers from Settings. Call at startup and after any Settings change.
pub fn configure(db: &Db) {
    crate::memory::set_local(db.kv_get("embed").as_deref() == Some("local"));
    *ROLES.write().unwrap() = Some(roles(&|k| db.kv_get(k), std::env::var("SUNDAY_PROVIDER").ok(), &resolver(db)));
}

/// The planner follows chat until it is set. Settings beat the SUNDAY_PROVIDER variable, which is the default for chat.
fn roles(kv: &dyn Fn(&str) -> Option<String>, env_provider: Option<String>, get: &dyn Fn(&str) -> Option<String>) -> Pair {
    let chat = kv("provider.chat").or(env_provider).unwrap_or_else(|| "ollama".into());
    let planner = kv("provider.planner").unwrap_or_else(|| chat.clone());
    (Arc::from(build(&chat, get)), Arc::from(build(&planner, get)))
}

/// Where a provider setting comes from: an environment variable wins, then the keychain, then Settings.
fn resolver(db: &Db) -> impl Fn(&str) -> Option<String> + '_ {
    move |k| std::env::var(k).ok().filter(|v| !v.is_empty()).or_else(|| secret_get(k)).or_else(|| db.kv_get(&format!("conf.{k}")))
}

/// What Settings may show and store. `local` providers keep prompts on this machine.
pub struct Info {
    pub id: &'static str,
    pub label: &'static str,
    pub local: bool,
    pub key_var: Option<&'static str>,
    /// A non-secret setting: (variable, label, placeholder).
    pub conf: Option<(&'static str, &'static str, &'static str)>,
    pub suggested_model: &'static str,
}

pub const CATALOG: [Info; 9] = [
    Info { id: "ollama", label: "Ollama", local: true, key_var: None, conf: None, suggested_model: crate::llm::MODEL },
    Info { id: "mlx", label: "MLX (on this Mac)", local: true, key_var: None, conf: None, suggested_model: "mlx-community/Qwen3-4B-Instruct-2507-4bit" },
    Info { id: "applefm", label: "Apple Intelligence (on this Mac)", local: true, key_var: None, conf: None, suggested_model: crate::applefm::MODEL },
    Info { id: "openai", label: "OpenAI-compatible", local: false, key_var: Some("OPENAI_API_KEY"), conf: Some(("OPENAI_BASE_URL", "Server address", "https://api.openai.com/v1")), suggested_model: "gpt-4o-mini" },
    Info { id: "lmstudio", label: "LM Studio", local: true, key_var: None, conf: Some(("LMSTUDIO_BASE_URL", "Server address", "http://localhost:1234/v1")), suggested_model: "qwen3-4b-instruct-2507" },
    Info { id: "openrouter", label: "OpenRouter", local: false, key_var: Some("OPENROUTER_API_KEY"), conf: None, suggested_model: "openai/gpt-4o-mini" },
    Info { id: "anthropic", label: "Anthropic", local: false, key_var: Some("ANTHROPIC_API_KEY"), conf: None, suggested_model: "claude-haiku-5-5" },
    Info { id: "gemini", label: "Gemini", local: false, key_var: Some("GEMINI_API_KEY"), conf: None, suggested_model: "gemini-2.5-flash" },
    Info { id: "nova", label: "Amazon Nova (Bedrock)", local: false, key_var: Some("AWS_BEARER_TOKEN_BEDROCK"), conf: Some(("AWS_REGION", "Region", "us-east-1")), suggested_model: "us.amazon.nova-lite-v1:0" },
];

pub fn info(id: &str) -> Option<&'static Info> {
    CATALOG.iter().find(|i| i.id == id)
}

/// Keys live in the macOS Keychain, never in the database. Other platforms use the environment only until their
/// keychain backends land with the platform layer (#21).
#[cfg(target_os = "macos")]
fn entry(var: &str) -> Option<keyring::Entry> {
    keyring::Entry::new("sunday", var).ok()
}

pub fn secret_get(var: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    return entry(var).and_then(|e| e.get_password().ok()).filter(|v| !v.is_empty());
    #[cfg(not(target_os = "macos"))]
    return {
        let _ = var;
        None
    };
}

pub fn secret_set(var: &str, value: &str) -> Result<(), String> {
    if !CATALOG.iter().any(|i| i.key_var == Some(var)) {
        return Err(format!("{var} isn't a provider key"));
    }
    #[cfg(target_os = "macos")]
    return entry(var).ok_or("the Keychain isn't available")?.set_password(value.trim()).map_err(|e| e.to_string());
    #[cfg(not(target_os = "macos"))]
    return Err(format!("Saving keys needs the macOS Keychain for now. Set {var} in the environment instead."));
}

pub fn secret_clear(var: &str) -> Result<(), String> {
    if !CATALOG.iter().any(|i| i.key_var == Some(var)) {
        return Err(format!("{var} isn't a provider key"));
    }
    #[cfg(target_os = "macos")]
    if let Some(e) = entry(var) {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

/// Everything Settings shows. Keys are never sent to the UI, only whether one is set.
pub fn settings(db: &Db) -> Value {
    let chat_model = crate::llm::model_name(db);
    let role = |r: &str, model: String| json!({"provider": db.kv_get(&format!("provider.{r}")), "model": model});
    json!({
        "roles": {
            "chat": {"provider": db.kv_get("provider.chat").or_else(|| std::env::var("SUNDAY_PROVIDER").ok()).unwrap_or_else(|| "ollama".into()), "model": chat_model},
            "planner": role("planner", planner_model(db)),
        },
        "embed": {"provider": if crate::memory::local() { "local" } else { "ollama" }, "model": crate::memory::embed_model(), "local_installed": crate::localembed::installed()},
        "providers": CATALOG.iter().map(|i| json!({
            "id": i.id, "label": i.label, "local": i.local, "suggested_model": i.suggested_model,
            "key_var": i.key_var,
            "key_set": i.key_var.is_some_and(|v| std::env::var(v).is_ok_and(|x| !x.is_empty()) || secret_get(v).is_some()),
            "key_from_env": i.key_var.is_some_and(|v| std::env::var(v).is_ok_and(|x| !x.is_empty())),
            "conf": i.conf.map(|(var, label, placeholder)| json!({"var": var, "label": label, "placeholder": placeholder, "value": resolver(db)(var)})),
        })).collect::<Vec<_>>(),
    })
}

pub fn planner_model(db: &Db) -> String {
    db.kv_get("planner_model").unwrap_or_else(|| crate::llm::model_name(db))
}

/// Saves a role's provider and model, then applies them. The planner's provider is stored only once chosen, so it
/// follows chat until then.
pub fn set_role(db: &Db, role: Role, provider: &str, model: &str) -> Result<(), String> {
    info(provider).ok_or_else(|| format!("unknown provider {provider}"))?;
    let model = model.trim();
    if model.is_empty() {
        return Err("Pick a model first.".into());
    }
    let (p, m) = match role {
        Role::Chat => ("provider.chat", "model"),
        Role::Planner => ("provider.planner", "planner_model"),
    };
    db.kv_set(p, provider);
    db.kv_set(m, model);
    configure(db);
    Ok(())
}

pub fn set_conf(db: &Db, var: &str, value: &str) -> Result<(), String> {
    if !CATALOG.iter().any(|i| i.conf.is_some_and(|c| c.0 == var)) {
        return Err(format!("{var} isn't a provider setting"));
    }
    db.kv_set(&format!("conf.{var}"), value.trim());
    configure(db);
    Ok(())
}

/// Models a provider offers, with the settings and key saved so far. Empty if it can't be reached.
pub fn models_of(db: &Db, id: &str) -> Vec<String> {
    info(id).map(|_| build(id, &resolver(db)).models()).unwrap_or_default()
}

/// Chooses where memory search gets its embeddings: "ollama" or "local" (on this Mac, no Ollama). Tried first, so
/// a missing model is found here and not the next time Sunday tries to remember something.
pub fn set_embedder(db: &Db, id: &str) -> Result<(), String> {
    if !["ollama", "local"].contains(&id) {
        return Err(format!("unknown embedder {id}"));
    }
    crate::memory::embed_via(id == "local", &["hello".to_string()], true).map_err(|e| e.to_string())?;
    db.kv_set("embed", id);
    configure(db);
    Ok(())
}

/// One real request, so the user learns about a wrong key or model here and not mid-sentence.
pub fn test(db: &Db, id: &str, model: &str) -> Result<Value, String> {
    info(id).ok_or_else(|| format!("unknown provider {id}"))?;
    let p = build(id, &resolver(db));
    let t = std::time::Instant::now();
    let reply = p.complete(model, &[json!({"role": "user", "content": "Reply with the single word ok."})], None, 30).map_err(|e| e.to_string())?;
    Ok(json!({"ms": t.elapsed().as_millis() as u64, "reply": reply.trim().chars().take(40).collect::<String>()}))
}

#[cfg(all(feature = "mlx", target_os = "macos"))]
fn mlx() -> Box<dyn Provider> {
    Box::new(crate::mlx::Mlx::new())
}

/// MLX runs on Apple Silicon only, in builds that include it. Elsewhere the choice is offered but says why it can't work.
#[cfg(not(all(feature = "mlx", target_os = "macos")))]
fn mlx() -> Box<dyn Provider> {
    struct NoMlx;
    impl Provider for NoMlx {
        fn name(&self) -> &'static str {
            "mlx"
        }
        fn complete(&self, _: &str, _: &[Value], _: Option<Value>, _: u64) -> Res<String> {
            Err(UserMsg("MLX needs a Mac with Apple Silicon, in a build that includes it.".into()).into())
        }
        fn stream(&self, _: &str, _: &[Value], _: &mut dyn FnMut(&str) -> bool) -> Res<()> {
            Err(UserMsg("MLX needs a Mac with Apple Silicon, in a build that includes it.".into()).into())
        }
        fn models(&self) -> Vec<String> {
            vec![]
        }
        fn warm(&self, _: &str) {}
    }
    Box::new(NoMlx)
}

/// A provider by name. Anything but ollama sends prompts off this machine, so it only happens when chosen.
/// OPENAI_BASE_URL points `openai` at vLLM, a llama.cpp server and the like; LM Studio and OpenRouter have presets.
fn build(name: &str, get: &dyn Fn(&str) -> Option<String>) -> Box<dyn Provider> {
    let key = |k: &str| get(k).unwrap_or_default();
    match name {
        "ollama" => Box::new(Ollama),
        "mlx" => mlx(),
        "applefm" => Box::new(crate::applefm::AppleFm::new()),
        "openai" => Box::new(OpenAi { base: get("OPENAI_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".into()), key: key("OPENAI_API_KEY") }),
        "lmstudio" => Box::new(OpenAi { base: get("LMSTUDIO_BASE_URL").unwrap_or_else(|| "http://localhost:1234/v1".into()), key: String::new() }),
        "openrouter" => Box::new(OpenAi { base: "https://openrouter.ai/api/v1".into(), key: key("OPENROUTER_API_KEY") }),
        "gemini" => Box::new(OpenAi { base: "https://generativelanguage.googleapis.com/v1beta/openai".into(), key: key("GEMINI_API_KEY") }),
        "anthropic" => Box::new(Anthropic { base: "https://api.anthropic.com".into(), key: key("ANTHROPIC_API_KEY") }),
        "nova" => Box::new(Nova { base: format!("https://bedrock-runtime.{}.amazonaws.com", get("AWS_REGION").unwrap_or_else(|| "us-east-1".into())), key: key("AWS_BEARER_TOKEN_BEDROCK") }),
        other => {
            eprintln!("[llm] unknown provider {other:?}, using ollama");
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
pub(crate) fn split(messages: &[Value], schema: Option<Value>) -> (String, Vec<(String, String)>) {
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
        let get = |_: &str| None;
        for (n, want) in [("ollama", "ollama"), ("mlx", "mlx"), ("applefm", "applefm"), ("bogus", "ollama"), ("openai", "openai"), ("gemini", "openai"), ("lmstudio", "openai"), ("openrouter", "openai"), ("anthropic", "anthropic"), ("nova", "nova")] {
            assert_eq!(build(n, &get).name(), want, "{n}");
        }
        // LM Studio is local and keyless; its address is the one setting.
        let lm = info("lmstudio").unwrap();
        assert!(lm.local && lm.key_var.is_none() && lm.conf.is_some_and(|c| c.0 == "LMSTUDIO_BASE_URL"));
    }

    #[test]
    fn roles_follow_settings_then_environment() {
        let names = |kv: &[(&str, &str)], env: Option<&str>| {
            let m: std::collections::HashMap<String, String> = kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let (c, p) = roles(&|k| m.get(k).cloned(), env.map(String::from), &|_| None);
            (c.name(), p.name())
        };
        assert_eq!(names(&[], None), ("ollama", "ollama"));
        assert_eq!(names(&[], Some("anthropic")), ("anthropic", "anthropic"), "the variable sets chat, and the planner follows");
        assert_eq!(names(&[("provider.chat", "ollama"), ("provider.planner", "nova")], Some("anthropic")), ("ollama", "nova"), "Settings beat the variable");
        assert_eq!(names(&[("provider.chat", "openai")], None), ("openai", "openai"), "an unset planner follows chat");
    }

    #[test]
    fn settings_reject_unknown_names_and_hide_nothing_but_keys() {
        let db = Db::open(&std::env::temp_dir().join(format!("sunday-prov-{}.db", std::process::id()))).unwrap();
        assert!(set_role(&db, Role::Chat, "skynet", "m").is_err());
        assert!(set_role(&db, Role::Chat, "ollama", " ").is_err(), "a role needs a model");
        assert!(set_embedder(&db, "skynet").is_err());
        assert!(set_conf(&db, "PATH", "/bin").is_err(), "only provider settings can be written");
        assert!(secret_set("PATH", "x").is_err() && secret_clear("HOME").is_err(), "only provider keys can be stored");
        set_role(&db, Role::Planner, "nova", "us.amazon.nova-lite-v1:0").unwrap();
        let v = settings(&db);
        assert_eq!(v["roles"]["planner"]["provider"], "nova");
        assert!(!v.to_string().contains("sk-"), "keys never reach the UI");
        assert_eq!(planner().name(), "nova");
        assert_eq!(current().name(), "ollama");
    }

    /// Real provider, real network. Costs tokens on remote providers.
    #[test]
    #[ignore = "needs a real provider (SUNDAY_PROVIDER_TEST=1)"]
    fn live() {
        let model = std::env::var("SUNDAY_MODEL").unwrap_or_else(|_| crate::llm::MODEL.into());
        let msgs = [json!({"role": "user", "content": "Say hello in one word."})];
        let p = current();
        assert!(!p.complete(&model, &msgs, None, 60).unwrap().trim().is_empty());
        let mut got = String::new();
        p.stream(&model, &msgs, &mut |t| { got.push_str(t); true }).unwrap();
        assert!(!got.trim().is_empty());
        println!("stream: {got:?}");
        let schema = json!({"type": "object", "required": ["greeting"], "properties": {"greeting": {"type": "string"}}});
        let j = p.complete(&model, &[json!({"role": "user", "content": "Greet me in one word."})], Some(schema), 60).unwrap();
        println!("schema: {j}");
        assert!(serde_json::from_str::<Value>(&j).is_ok_and(|v| v["greeting"].is_string()), "a schema reply must be JSON with the field");
    }
}
