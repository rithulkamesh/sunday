//! Long-term memory: atomic facts with provenance. The model follows Devsper's memory service
//! (svc/memory): each fact is one self-contained sentence with where it came from, how confident the
//! extraction was and when it refers to, stored with an embedding and searched by similarity.
//! Here it runs locally: SQLite for storage, Ollama for embeddings and extraction.
use crate::db::{Db, Fact, NewFact};
use crate::speaker::cosine;
use serde::Deserialize;
use serde_json::{json, Value};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const EMBED_MODEL: &str = "nomic-embed-text-v2-moe"; // override with SUNDAY_EMBED
/// A new fact this similar to an existing one is the same fact.
const DUPLICATE: f32 = 0.93;
/// Recall only returns facts at least this relevant to the question. Measured with nomic-embed-text-v2-moe:
/// questions about a fact score 0.36-0.55, unrelated ones at most 0.29 (see `similarity_calibration`).
pub const RELEVANT: f32 = 0.33;
const WINDOW: usize = 3500;
const MAX_WINDOWS: usize = 8;

pub fn embed_model() -> String {
    std::env::var("SUNDAY_EMBED").unwrap_or_else(|_| EMBED_MODEL.into())
}

/// Embeddings from Ollama. nomic wants a task prefix: documents and queries are embedded differently.
pub fn embed(texts: &[String], query: bool) -> Res<Vec<Vec<f32>>> {
    let prefix = if query { "search_query: " } else { "search_document: " };
    let input: Vec<String> = texts.iter().map(|t| format!("{prefix}{t}")).collect();
    let v: Value = ureq::post("http://localhost:11434/api/embed")
        .timeout(std::time::Duration::from_secs(60))
        .send_json(json!({"model": embed_model(), "input": input, "keep_alive": crate::llm::KEEP_TEXT}))?
        .into_json()?;
    let out: Vec<Vec<f32>> = v["embeddings"]
        .as_array()
        .ok_or("no embeddings in the reply")?
        .iter()
        .map(|e| e.as_array().map(|a| a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect()).unwrap_or_default())
        .collect();
    if out.len() != texts.len() || out.iter().any(Vec::is_empty) {
        return Err("the embedding model returned the wrong number of vectors".into());
    }
    Ok(out)
}

/// Sentence-aligned windows for extraction: long pages are read a piece at a time.
pub fn windows(text: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    for sentence in text.split_inclusive(['.', '!', '?', '\n']) {
        if cur.len() + sentence.len() > WINDOW && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        // A "sentence" longer than a window (a table, minified text) is cut at a char boundary.
        let mut rest = sentence;
        while rest.len() > WINDOW {
            let mut cut = WINDOW;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push(rest[..cut].to_string());
            rest = &rest[cut..];
        }
        cur.push_str(rest);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.truncate(MAX_WINDOWS);
    out
}

#[derive(Deserialize, Debug)]
pub struct Extracted {
    pub text: String,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub event_date: String,
}
#[derive(Deserialize)]
struct Extraction {
    facts: Vec<Extracted>,
}

const EXTRACT: &str = "You extract atomic facts from text so they can be remembered and searched later. Each fact is one self-contained sentence \
that makes sense on its own: name the subject instead of using pronouns, and keep numbers and units. Only facts the text actually states: no opinions \
of yours, and skip navigation, ads and boilerplate. confidence is 0 to 1: how clearly the text states it. event_date is the date the fact refers to \
(YYYY-MM-DD, YYYY-MM or YYYY) if the text gives one, otherwise an empty string. At most 12 facts.";

const EXTRACT_CHAT: &str = "You read a conversation between a user and their assistant and note lasting facts about the USER: goals, plans and dates, what they are learning \
or working on, preferences, people and projects they mention, recurring difficulties. Only what the user said or clearly showed; never what the assistant said. \
Skip small talk and one-off questions. Each fact is one self-contained sentence about the user. confidence is 0 to 1. event_date is a date it refers to if stated, \
else an empty string. At most 6 facts, and none if nothing lasting was said.";

fn schema() -> Value {
    json!({"type": "object", "required": ["facts"], "properties": {"facts": {"type": "array", "items": {"type": "object",
        "required": ["text", "confidence", "event_date"],
        "properties": {"text": {"type": "string"}, "confidence": {"type": "number"}, "event_date": {"type": "string"}}}}}})
}

/// Facts the model finds in `text`; `system` says what to look for.
fn extract_with(model: &str, system: &str, text: &str) -> Res<Vec<Extracted>> {
    let e: Extraction = crate::llm::json(model, system, text, schema(), 180)?;
    Ok(clean(e.facts))
}

/// Drops empties and absurd lengths, clamps confidence, and removes repeats within one batch.
fn clean(facts: Vec<Extracted>) -> Vec<Extracted> {
    let mut seen = std::collections::HashSet::new();
    facts
        .into_iter()
        .map(|mut f| {
            f.text = f.text.split_whitespace().collect::<Vec<_>>().join(" ");
            f.confidence = f.confidence.clamp(0.0, 1.0);
            f.event_date.truncate(10);
            f
        })
        .filter(|f| (12..=400).contains(&f.text.len()) && seen.insert(f.text.to_lowercase()))
        .collect()
}

/// Stores facts unless an equivalent one is already remembered. Returns (added, duplicates).
/// Without embeddings (the model isn't pulled) facts are still stored, and found by keyword instead.
pub fn store(db: &Db, kind: &str, source: &str, title: &str, facts: &[Extracted]) -> (usize, usize) {
    if facts.is_empty() {
        return (0, 0);
    }
    let texts: Vec<String> = facts.iter().map(|f| f.text.clone()).collect();
    let embs = embed(&texts, false).ok();
    let mut known = db.fact_embeddings();
    let (mut added, mut dup) = (0, 0);
    for (i, f) in facts.iter().enumerate() {
        let e = embs.as_ref().map(|v| &v[i]);
        if e.is_some_and(|e| known.iter().any(|(_, k)| cosine(e, k) >= DUPLICATE)) {
            dup += 1;
            continue;
        }
        let id = db.fact_add(&NewFact { kind, source, title, text: &f.text, confidence: f.confidence, event_date: &f.event_date }, e.map(Vec::as_slice));
        if let Some(e) = e {
            known.push((id, e.clone()));
        }
        added += 1;
    }
    (added, dup)
}

/// Reads a page's text a window at a time and remembers what it says. Returns (added, duplicates).
pub fn learn_page(db: &Db, model: &str, source: &str, title: &str, text: &str) -> (usize, usize) {
    let (mut added, mut dup) = (0, 0);
    for w in windows(text) {
        match extract_with(model, EXTRACT, &w) {
            Ok(facts) => {
                let (a, d) = store(db, "web", source, title, &facts);
                (added, dup) = (added + a, dup + d);
            }
            Err(e) => eprintln!("[memory] extraction failed on one part of {source}: {e}"),
        }
    }
    (added, dup)
}

/// Lasting facts about the user from a conversation (`transcript` is "User: ...\nAssistant: ..." lines).
/// Stored as kind "chat": a guess you can review, never presented as something you said.
pub fn learn_from_chat(db: &Db, model: &str, transcript: &str) -> (usize, usize) {
    match extract_with(model, EXTRACT_CHAT, transcript) {
        Ok(facts) => store(db, "chat", "chat", "a conversation", &facts),
        Err(e) => {
            eprintln!("[memory] couldn't learn from the conversation: {e}");
            (0, 0)
        }
    }
}

/// "remember that my driving test is on 8 February": stored as you said it, with full confidence.
pub fn remember(db: &Db, text: &str) -> bool {
    let f = Extracted { text: text.split_whitespace().collect::<Vec<_>>().join(" "), confidence: 1.0, event_date: String::new() };
    if f.text.len() < 3 {
        return false;
    }
    let (added, _) = store(db, "user", "you", "something you told me", &[f]);
    added > 0
}

/// The facts most relevant to `q`, best first, each with its similarity in `score`.
/// Falls back to keyword matching if the embedding model isn't available.
pub fn recall(db: &Db, q: &str, k: usize) -> Vec<Fact> {
    let mut scored: Vec<(i64, f32)> = match embed(&[q.to_string()], true) {
        Ok(v) => db.fact_embeddings().into_iter().map(|(id, e)| (id, cosine(&v[0], &e))).filter(|(_, s)| *s >= RELEVANT).collect(),
        Err(_) => keyword_scores(db, q),
    };
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().take(k).filter_map(|(id, s)| db.fact(id).map(|f| Fact { score: s, ..f })).collect()
}

fn keyword_scores(db: &Db, q: &str) -> Vec<(i64, f32)> {
    let terms = crate::notes::terms(q);
    if terms.is_empty() {
        return vec![];
    }
    db.facts(2000)
        .into_iter()
        .filter_map(|f| {
            let t = f.text.to_lowercase();
            let hit = terms.iter().filter(|w| t.contains(w.as_str())).count();
            (hit * 2 >= terms.len()).then(|| (f.id, hit as f32 / terms.len() as f32))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_long_text_into_sentence_windows() {
        let text = "Processes need resources. ".repeat(400); // ~10 KB
        let w = windows(&text);
        assert!(w.len() >= 3 && w.iter().all(|x| x.len() <= WINDOW + 30), "{:?}", w.iter().map(String::len).collect::<Vec<_>>());
        assert!(w.iter().all(|x| x.trim_end().ends_with('.')), "windows end on sentence boundaries");
        assert_eq!(windows("short.").len(), 1);
        assert!(windows(&"x".repeat(50_000)).len() <= MAX_WINDOWS);
        assert!(windows("").is_empty());
    }

    #[test]
    fn cleans_extracted_facts() {
        let f = |t: &str, c: f64| Extracted { text: t.into(), confidence: c, event_date: "2025-03-01 and more".into() };
        let out = clean(vec![
            f("  Deadlock needs   four conditions. ", 1.7),
            f("deadlock needs four conditions.", 0.5), // same fact again
            f("too short", 0.9),
            f(&"x".repeat(500), 0.9),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].text.as_str(), out[0].confidence, out[0].event_date.as_str()), ("Deadlock needs four conditions.", 1.0, "2025-03-01"));
    }

    #[test]
    fn keyword_fallback_finds_facts_without_embeddings() {
        let db = Db::open(&std::env::temp_dir().join(format!("sunday-mem-{}-a.db", std::process::id()))).unwrap();
        db.fact_add(&NewFact { kind: "user", source: "you", title: "", text: "My driving test is on 8 February.", confidence: 1.0, event_date: "" }, None);
        db.fact_add(&NewFact { kind: "web", source: "https://x.dev", title: "x", text: "TCP uses slow start.", confidence: 0.9, event_date: "" }, None);
        let hits: Vec<_> = keyword_scores(&db, "when is my driving test").into_iter().map(|(id, _)| db.fact(id).unwrap().text).collect();
        assert_eq!(hits, vec!["My driving test is on 8 February."]);
        assert!(keyword_scores(&db, "the and for").is_empty());
    }

    /// Needs Ollama with the embedding and text models. Remembers, extracts from text, deduplicates, recalls by meaning.
    #[test]
    fn remembers_and_recalls_by_meaning() {
        let db = Db::open(&std::env::temp_dir().join(format!("sunday-mem-{}-b.db", std::process::id()))).unwrap();
        assert!(remember(&db, "my driving test is on the 8th of February"));
        assert!(!remember(&db, "my driving test is on the 8th of February"), "the same fact twice is stored once");
        let page = "The Banker's algorithm, devised by Edsger Dijkstra, avoids deadlock by granting a resource request only if the system stays in a safe state. \
            A state is safe if some ordering of the processes lets each one finish with the resources that are available. \
            The algorithm needs each process to declare its maximum resource demand in advance. It runs in O(m n^2) time for m resource types and n processes.";
        let model = crate::llm::model_name(&db);
        let t = std::time::Instant::now();
        let (added, dup) = learn_page(&db, &model, "https://example.com/banker", "Banker's algorithm", page);
        println!("learned {added} facts ({dup} duplicates) in {:?}", t.elapsed());
        for f in db.facts(20) {
            println!("  [{}] {} (conf {:.2}, {})", f.kind, f.text, f.confidence, f.source);
        }
        assert!(added >= 2);
        let by_meaning = recall(&db, "when do I take my test?", 3);
        println!("recall exam: {:?}", by_meaning.iter().map(|f| (f.score, &f.text)).collect::<Vec<_>>());
        assert!(by_meaning.first().is_some_and(|f| f.kind == "user"), "found by meaning, not keywords");
        let dl = recall(&db, "how do you avoid deadlock safely?", 3);
        println!("recall deadlock: {:?}", dl.iter().map(|f| (f.score, &f.text)).collect::<Vec<_>>());
        assert!(dl.first().is_some_and(|f| f.kind == "web"));
        assert!(recall(&db, "what is the capital of Peru", 3).is_empty(), "unrelated questions recall nothing");
        assert_eq!(db.facts_delete_source("https://example.com/banker") > 0, true);
    }

    /// Prints similarities so RELEVANT can be chosen from data (run with --nocapture).
    #[test]
    fn similarity_calibration() {
        if std::env::var("SUNDAY_MEM_CALIBRATE").is_err() {
            return;
        }
        let facts = ["my driving test is on the 8th of February", "The Banker's algorithm avoids deadlock by granting a resource request only if the system stays in a safe state",
            "TCP uses slow start and congestion avoidance to adapt its window", "I prefer studying in the mornings"];
        let qs = [("when do I take my test?", 0), ("when is my driving test", 0), ("how do you avoid deadlock safely?", 1), ("what is the banker's algorithm", 1),
            ("how does congestion control work", 2), ("when do I like to study", 3), ("what is the capital of Peru", 9), ("who won the world cup", 9), ("explain virtual memory", 9)];
        let f = embed(&facts.iter().map(|s| s.to_string()).collect::<Vec<_>>(), false).unwrap();
        for (q, want) in qs {
            let v = &embed(&[q.to_string()], true).unwrap()[0];
            let sims: Vec<String> = f.iter().map(|e| format!("{:.2}", cosine(v, e))).collect();
            println!("{q:40} expects #{want}: {sims:?}");
        }
    }
}
