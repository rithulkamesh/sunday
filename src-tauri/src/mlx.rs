//! On-device inference with MLX on Apple Silicon: no Ollama, no Python, no daemon. MLX is built from vendored source
//! (the `mlex` crate) and runs the model on the GPU inside this process. Models are MLX checkpoints under
//! `~/.cache/sunday/models/mlx/<owner>--<name>`, put there by `scripts/fetch-mlx-model.sh`; the app never downloads.
//! No constrained decoding and no embeddings here: a schema goes in the prompt (`json_via` retries bad JSON), and
//! memory search keeps using Ollama's embedding model, or plain keywords when that is missing.
use crate::llm::{Res, UserMsg};
use crate::provider::{split, Provider};
use mlex::generate::{GenerateOptions, Session};
use mlex::sampling::SamplingConfig;
use mlex::tokenizer::ChatMessage;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Mlx {
    dir: PathBuf,
    /// The model in memory. Loading takes seconds, so it stays until another one is asked for.
    loaded: Mutex<Option<(String, Arc<Session>)>>,
    /// One generation at a time: chat and the planner share one GPU.
    busy: Mutex<()>,
}

impl Mlx {
    pub fn new() -> Mlx {
        Mlx::at(crate::models::dir().join("mlx"))
    }

    pub fn at(dir: PathBuf) -> Mlx {
        Mlx { dir, loaded: Mutex::new(None), busy: Mutex::new(()) }
    }

    fn session(&self, model: &str) -> Res<Arc<Session>> {
        let mut g = self.loaded.lock().unwrap();
        if let Some((_, s)) = g.as_ref().filter(|(m, _)| m == model) {
            return Ok(s.clone());
        }
        let path = self.dir.join(model.replace('/', "--"));
        if !path.join("config.json").exists() {
            return Err(UserMsg(format!("The model {model} isn't downloaded. Run scripts/fetch-mlx-model.sh {model}.")).into());
        }
        let s = Arc::new(Session::load(&path).map_err(|e| {
            eprintln!("[mlx] {e}");
            UserMsg(format!("I couldn't load {model}. Is it an MLX model of a kind I support, such as Qwen3?"))
        })?);
        *g = Some((model.to_string(), s.clone()));
        Ok(s)
    }

    fn run(&self, model: &str, messages: &[Value], schema: Option<Value>, max_tokens: usize, deadline: Option<Instant>, mut on_token: impl FnMut(&str) -> bool) -> Res<String> {
        let s = self.session(model)?;
        let (system, turns) = split(messages, schema);
        let mut chat = Vec::new();
        if !system.is_empty() {
            chat.push(ChatMessage::system(system));
        }
        for (role, text) in turns {
            chat.push(if role == "assistant" { ChatMessage::assistant(text) } else { ChatMessage::user(text) });
        }
        let _one_at_a_time = self.busy.lock().unwrap();
        let opts = GenerateOptions { max_tokens, sampling: SamplingConfig::default(), enable_thinking: Some(false), ..GenerateOptions::default() };
        let reply = s
            .generate_cached(&chat, None, opts, |t| deadline.is_none_or(|d| Instant::now() < d) && on_token(&t.text))
            .map_err(|e| {
                eprintln!("[mlx] {e}");
                UserMsg("Sorry, the on-device model failed.".into())
            })?;
        Ok(reply.text)
    }
}

/// A model that wraps its JSON in prose or a code fence still has the JSON in it.
fn json_part(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(a), Some(b)) if a < b => &text[a..=b],
        _ => text.trim(),
    }
}

impl Provider for Mlx {
    fn name(&self) -> &'static str {
        "mlx"
    }

    fn complete(&self, model: &str, messages: &[Value], schema: Option<Value>, timeout_s: u64) -> Res<String> {
        let wants_json = schema.is_some();
        let text = self.run(model, messages, schema, 1024, Some(Instant::now() + Duration::from_secs(timeout_s)), |_| true)?;
        Ok(if wants_json { json_part(&text).to_string() } else { text.trim().to_string() })
    }

    fn stream(&self, model: &str, messages: &[Value], on_token: &mut dyn FnMut(&str) -> bool) -> Res<()> {
        self.run(model, messages, None, 1024, None, |t| on_token(t)).map(|_| ())
    }

    fn models(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("config.json").exists())
            .map(|e| e.file_name().to_string_lossy().replacen("--", "/", 1))
            .collect();
        v.sort();
        v
    }

    /// Loads the model ahead of the first question.
    fn warm(&self, model: &str) {
        let _ = self.session(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_cut_out_of_chatter() {
        assert_eq!(json_part("Sure! ```json\n{\"a\": {\"b\": 1}}\n``` done"), "{\"a\": {\"b\": 1}}");
        assert_eq!(json_part("no json here"), "no json here");
    }

    #[test]
    fn models_are_the_folders_that_hold_a_checkpoint() {
        let d = std::env::temp_dir().join(format!("sunday-mlx-{}", std::process::id()));
        std::fs::create_dir_all(d.join("mlx-community--Qwen3-4B-4bit")).unwrap();
        std::fs::write(d.join("mlx-community--Qwen3-4B-4bit/config.json"), "{}").unwrap();
        std::fs::create_dir_all(d.join("half-downloaded")).unwrap();
        let m = Mlx::at(d.clone());
        assert_eq!(m.models(), ["mlx-community/Qwen3-4B-4bit"]);
        let e = m.complete("mlx-community/Nope", &[], None, 5).unwrap_err();
        assert!(e.downcast_ref::<UserMsg>().is_some_and(|u| u.0.contains("fetch-mlx-model.sh mlx-community/Nope")), "{e}");
        let _ = std::fs::remove_dir_all(d);
    }
}
