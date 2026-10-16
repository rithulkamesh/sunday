//! Replies via a local Ollama model: tokens stream to the HUD, each finished sentence goes to TTS.
use crate::tts::{self, Cmd};
use crate::Hud;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::mpsc::{channel, Sender};
use tauri::Emitter;

const URL: &str = "http://localhost:11434/api/chat";
const MODEL: &str = "gemma4"; // override with SUNDAY_MODEL
const SYSTEM: &str = "You are Sunday, a voice assistant. Reply in one or two short spoken \
sentences. Plain words only: no markdown, lists, code or emoji.";
const KEEP: usize = 12; // history messages kept besides the system prompt

/// Send it what the user said; it answers on the HUD and out loud.
pub fn spawn(hud: Hud, speak: Sender<Cmd>) -> Sender<String> {
    let (tx, rx) = channel::<String>();
    std::thread::spawn(move || {
        let model = std::env::var("SUNDAY_MODEL").unwrap_or_else(|_| MODEL.into());
        let mut history = vec![json!({"role": "system", "content": SYSTEM})];
        for q in rx {
            history.push(json!({"role": "user", "content": q}));
            let _ = hud.app.emit("state", "thinking");
            let reply = chat(&hud, &speak, &model, &history).unwrap_or_else(|e| {
                eprintln!("[llm] {e}");
                String::new()
            });
            if reply.is_empty() {
                history.pop();
            } else {
                history.push(json!({"role": "assistant", "content": reply}));
            }
            let excess = history.len().saturating_sub(KEEP + 1);
            history.drain(1..1 + excess);
            // Keep the HUD up until the last sentence has been spoken.
            while tts::speaking() {
                hud.touch();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = hud.app.emit("state", "listening");
        }
    });
    tx
}

fn chat(hud: &Hud, speak: &Sender<Cmd>, model: &str, history: &[Value]) -> Result<String, Box<dyn std::error::Error>> {
    // think:false is ~5x faster to first word on reasoning models; keep_alive avoids reloading weights.
    let body = json!({"model": model, "messages": history, "stream": true, "think": false, "keep_alive": "30m"});
    let resp = ureq::post(URL).send_json(body)?;
    let (mut full, mut pending) = (String::new(), String::new());
    for line in BufReader::new(resp.into_reader()).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        let Some(tok) = v["message"]["content"].as_str().filter(|t| !t.is_empty()) else { continue };
        full.push_str(tok);
        pending.push_str(tok);
        hud.touch();
        let _ = hud.app.emit("transcript", full.trim());
        while let Some(s) = take_sentence(&mut pending) {
            let _ = speak.send(Cmd::Speak(s));
        }
    }
    if !pending.trim().is_empty() {
        let _ = speak.send(Cmd::Speak(pending));
    }
    Ok(full.trim().to_string())
}

/// Pops the first complete sentence. A stop mark only counts once whitespace follows it,
/// so "3.5" survives and a mark at the end of the buffer waits for more tokens.
fn take_sentence(buf: &mut String) -> Option<String> {
    let mut chars = buf.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let end = c == '\n' || (".!?".contains(c) && chars.peek().is_some_and(|(_, n)| n.is_whitespace()));
        if end {
            let s: String = buf.drain(..i + c.len_utf8()).collect();
            return Some(s).filter(|s| !s.trim().is_empty()).or_else(|| take_sentence(buf));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sentences() {
        let mut b = "It is 3.5 now. Next one".to_string();
        assert_eq!(take_sentence(&mut b).as_deref(), Some("It is 3.5 now."));
        assert_eq!(take_sentence(&mut b), None); // "Next one" is unfinished
        b.push_str("! Done.");
        assert_eq!(take_sentence(&mut b).as_deref(), Some(" Next one!"));
        assert_eq!(take_sentence(&mut b), None); // trailing "." has nothing after it yet
    }
}
