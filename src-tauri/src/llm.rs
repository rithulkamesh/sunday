//! The tutor. Routes what the user says (chat, a quiz request, or the answer to a pending
//! question) to the local Ollama model, stores the evidence, and delivers the reply to the
//! workspace, the HUD and TTS.
use crate::db::{Db, Pending};
use crate::study::{self, Grade, Quiz};
use crate::tts::{self, Cmd};
use crate::Hud;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use tauri::Emitter;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const URL: &str = "http://localhost:11434/api/chat";
const MODEL: &str = "gemma4"; // override with SUNDAY_MODEL
const KEEP: usize = 12; // history messages kept besides the system prompt

/// One thing the user said. `voice` replies also go to the HUD and speakers; typed ones stay on screen.
pub struct Turn {
    pub text: String,
    pub voice: bool,
}

struct Ctx<'a> {
    hud: &'a Hud,
    speak: &'a Sender<Cmd>,
    db: &'a Db,
    model: &'a str,
    ep: u64,
    voice: bool,
}

pub fn spawn(hud: Hud, speak: Sender<Cmd>, db: Arc<Db>) -> Sender<Turn> {
    let (tx, rx) = channel::<Turn>();
    std::thread::spawn(move || {
        let model = std::env::var("SUNDAY_MODEL").unwrap_or_else(|_| MODEL.into());
        let mut history = vec![json!({"role": "system", "content": study::TUTOR})];
        // Pick the conversation back up after a restart.
        history.extend(db.history(6).into_iter().map(|m| json!({"role": m.role, "content": m.text})));
        for Turn { text: q, voice } in rx {
            db.message("user", &q);
            let _ = hud.app.emit("turn", json!({"role": "user", "text": q}));
            history.push(json!({"role": "user", "content": q}));
            tts::set_replying(true);
            let ep = tts::epoch();
            let waiting = Arc::new(AtomicBool::new(voice));
            if voice {
                let _ = hud.app.emit("state", "thinking");
                let _ = speak.send(Cmd::Chime);
                // Thinking can outlast the wake window; keep the HUD awake until the reply arrives.
                let (h, w) = (hud.clone(), waiting.clone());
                std::thread::spawn(move || {
                    while w.load(Ordering::Relaxed) {
                        h.touch();
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                });
            }
            let c = Ctx { hud: &hud, speak: &speak, db: &db, model: &model, ep, voice };
            let reply = route(&c, &mut history, &q);
            waiting.store(false, Ordering::Relaxed);
            let reply = match reply {
                Ok(r) => r,
                // Interrupted replies fail quietly; a real failure is worth saying.
                Err(_) if tts::epoch() != ep => String::new(),
                Err(e) => {
                    eprintln!("[llm] {e}");
                    say(&c, "Sorry, I couldn't reach the model. Is Ollama running?")
                }
            };
            if reply.is_empty() {
                history.pop();
            } else {
                db.message("assistant", &reply);
                let _ = hud.app.emit("turn", json!({"role": "assistant", "text": reply}));
                history.push(json!({"role": "assistant", "content": reply}));
            }
            let excess = history.len().saturating_sub(KEEP + 1);
            history.drain(1..1 + excess);
            if voice {
                // Keep the HUD up until the last sentence has been spoken.
                while tts::talking() {
                    hud.touch();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                hud.touch(); // the idle window runs from the end of Sunday's speech
                let _ = hud.app.emit("state", "listening");
            }
            tts::set_replying(false);
        }
    });
    tx
}

/// An unanswered question takes the next utterance as its answer, unless it's a new request.
fn route(c: &Ctx, history: &mut Vec<Value>, q: &str) -> Res<String> {
    if let Some(p) = c.db.pending() {
        if study::is_skip(q) {
            c.db.set_pending(None);
            return Ok(say(c, "Okay, skipped. Say quiz me when you're ready."));
        }
        if study::quiz_request(q).is_none() {
            return grade(c, &p, q);
        }
    }
    match study::quiz_request(q) {
        Some(topic) => quiz(c, topic),
        None => chat(c, history),
    }
}

/// Structured output: the schema constrains the model, serde validates, one retry, then give up.
fn json<T: DeserializeOwned>(model: &str, system: &str, user: &str, schema: Value) -> Res<T> {
    let body = json!({"model": model, "stream": false, "think": false, "keep_alive": "30m", "format": schema,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]});
    for _ in 0..2 {
        let v: Value = ureq::post(URL).send_json(&body)?.into_json()?;
        if let Ok(t) = serde_json::from_str(v["message"]["content"].as_str().unwrap_or("")) {
            return Ok(t);
        }
    }
    Err("the model returned invalid JSON twice".into())
}

fn quiz(c: &Ctx, topic: Option<String>) -> Res<String> {
    // No topic: what's due first, then the weakest concept, else let the model choose.
    let concept = topic
        .or_else(|| c.db.due().first().map(|d| d.concept.clone()))
        .or_else(|| c.db.weak().first().map(|w| w.concept.clone()));
    let ask = match &concept {
        Some(k) => format!("Concept: {k}. Do not repeat these questions: {:?}", c.db.recent_questions(k, 5)),
        None => "Pick any core GATE concept to start with.".into(),
    };
    let z: Quiz = json(c.model, study::QUIZ, &ask, study::quiz_schema())?;
    let p = Pending {
        concept: concept.unwrap_or(z.concept).trim().to_lowercase(),
        question: z.question.trim().into(),
        answer: z.answer,
    };
    c.db.set_pending(Some(&p));
    Ok(say(c, &format!("Here's one on {}. {}", p.concept, p.question)))
}

fn grade(c: &Ctx, p: &Pending, said: &str) -> Res<String> {
    let ask = format!("Question: {}\nReference answer: {}\nStudent answer: {said}", p.question, p.answer);
    let g: Grade = json(c.model, study::GRADE, &ask, study::grade_schema())?;
    c.db.attempt(p, said, g.verdict, &g.error_type, &g.feedback);
    c.db.set_pending(None);
    let lead = match g.verdict {
        crate::db::Verdict::Correct => "Correct.",
        crate::db::Verdict::Partial => "Partly right.",
        crate::db::Verdict::Wrong => "Not quite.",
    };
    Ok(say(c, &format!("{lead} {} Say next question when you're ready.", g.feedback.trim())))
}

/// Speaks `text` sentence by sentence (voice turns only); returns it for the log.
fn say(c: &Ctx, text: &str) -> String {
    if c.voice {
        let (mut buf, mut first) = (text.to_string(), true);
        while let Some(s) = take_sentence(&mut buf, first) {
            first = false;
            tts::say(c.speak, s, c.ep);
        }
        if !buf.trim().is_empty() {
            tts::say(c.speak, buf, c.ep);
        }
    }
    text.trim().to_string()
}

/// Free-form tutoring. Empty if interrupted (`tts::interrupt` bumps the epoch).
fn chat(c: &Ctx, history: &[Value]) -> Res<String> {
    // think:false is ~5x faster to first word on reasoning models; keep_alive avoids reloading weights.
    let body = json!({"model": c.model, "messages": history, "stream": true, "think": false, "keep_alive": "30m"});
    let resp = ureq::post(URL).send_json(body)?;
    let (mut full, mut pending) = (String::new(), String::new());
    let mut first = true; // the first chunk is cut at a comma so audio starts sooner
    for line in BufReader::new(resp.into_reader()).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        if tts::epoch() != c.ep {
            return Ok(String::new()); // dropping the response closes the connection; Ollama stops
        }
        let Some(tok) = v["message"]["content"].as_str().filter(|t| !t.is_empty()) else { continue };
        full.push_str(tok);
        pending.push_str(tok);
        c.hud.touch();
        while let Some(s) = take_sentence(&mut pending, first) {
            first = false;
            if c.voice {
                tts::say(c.speak, s, c.ep);
            }
        }
    }
    if tts::epoch() != c.ep {
        return Ok(String::new());
    }
    if c.voice && !pending.trim().is_empty() {
        tts::say(c.speak, pending, c.ep);
    }
    Ok(full.trim().to_string())
}

/// Pops the first complete sentence. A stop mark only counts once whitespace follows it,
/// so "3.5" survives and a mark at the end of the buffer waits for more tokens.
/// `eager` also cuts at a comma after 12+ chars: TTS runs faster than real time, so only the
/// first chunk's length matters for time-to-first-audio.
fn take_sentence(buf: &mut String, eager: bool) -> Option<String> {
    let mut chars = buf.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let stop = ".!?".contains(c) || (eager && i >= 12 && ",;:".contains(c));
        let end = c == '\n' || (stop && chars.peek().is_some_and(|(_, n)| n.is_whitespace()));
        if end {
            let s: String = buf.drain(..i + c.len_utf8()).collect();
            return Some(s).filter(|s| !s.trim().is_empty()).or_else(|| take_sentence(buf, eager));
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
        assert_eq!(take_sentence(&mut b, false).as_deref(), Some("It is 3.5 now."));
        assert_eq!(take_sentence(&mut b, false), None); // "Next one" is unfinished
        b.push_str("! Done.");
        assert_eq!(take_sentence(&mut b, false).as_deref(), Some(" Next one!"));
        assert_eq!(take_sentence(&mut b, false), None); // trailing "." has nothing after it yet

        let mut b = "Well, I can answer questions, and more. Ok".to_string();
        assert_eq!(take_sentence(&mut b, true).as_deref(), Some("Well, I can answer questions,")); // short comma skipped
        assert_eq!(take_sentence(&mut b, false).as_deref(), Some(" and more."));
    }

    /// Needs Ollama running with the model pulled: the quiz and grading JSON must parse.
    #[test]
    fn model_returns_valid_study_json() {
        let m = std::env::var("SUNDAY_MODEL").unwrap_or_else(|_| MODEL.into());
        let t = std::time::Instant::now();
        let z: Quiz = json(&m, study::QUIZ, "Concept: deadlock. Do not repeat these questions: []", study::quiz_schema()).unwrap();
        println!("quiz in {:?}: {z:?}", t.elapsed());
        let t = std::time::Instant::now();
        let ask = format!("Question: {}\nReference answer: {}\nStudent answer: I have no idea", z.question, z.answer);
        let g: Grade = json(&m, study::GRADE, &ask, study::grade_schema()).unwrap();
        println!("grade in {:?}: {g:?}", t.elapsed());
        assert_eq!(g.verdict, crate::db::Verdict::Wrong);
    }
}
