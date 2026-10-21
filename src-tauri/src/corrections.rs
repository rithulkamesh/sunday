//! The router learns from corrections at once, with no retraining. When the user says outright that they are fixing the
//! turn before, right after a turn that went to chat ("Ranked by the best." ... "I said can you sort by the best?"), the first utterance is remembered
//! with the route the second one took; the next time something close to it comes up, that route is used. Examples live
//! in the local `route_examples` table, and a retrained model (issue #56) can use the same rows.
//! It only picks a route, like the rest of the router: permissions and approvals are unchanged.
use crate::db::{now, Db};
use serde::{Deserialize, Serialize};

/// A correction counts only soon after the turn it corrects.
const WINDOW_SECS: i64 = 90;
/// How close an utterance must be to a remembered one to take its route.
const SAME: f32 = 0.85;
/// A new example this close to an old one in the same context replaces it, so one wrong correction can't stick.
const SUPERSEDE: f32 = 0.95;
const MARKERS: [&str; 8] = ["i said", "i meant", "i mean", "no,", "no no", "not that", "that's not what i", "actually"];

#[derive(Serialize, Deserialize)]
struct Turn {
    text: String,
    last: String,
    /// "chat" (plain conversation), "do", "screen", "research", or "other" (teaching, quizzes, notes...).
    route: String,
    at: i64,
}

/// Does `cur` say outright that it is fixing the turn before ("I said ...", "no, ...")? Only an explicit marker counts.
/// Sharing a few words with the previous utterance is not enough: speech recognition garbles things ("sort by the
/// best" heard as "say it by the best"), and a garble that happens to share words would teach the wrong route.
pub fn is_correction(cur: &str) -> bool {
    let c = cur.trim().to_lowercase();
    MARKERS.iter().any(|m| c.starts_with(m))
}

/// The remembered example closest to `emb` in the same context, if it is close enough: (route, similarity).
fn closest(examples: &[(i64, String, String, String, Vec<f32>)], last: &str, emb: &[f32]) -> Option<(String, f32)> {
    examples.iter().filter(|e| e.1 == last).map(|e| (e.3.clone(), crate::memory::sim(&e.4, emb))).filter(|(_, s)| *s >= SAME).max_by(|a, b| a.1.total_cmp(&b.1))
}

fn embedding(text: &str) -> Option<Vec<f32>> {
    crate::memory::embed(&[text.to_string()], true).ok()?.pop()
}

/// Stores `text` (said after `last`) as an example of `route`, replacing any example it contradicts.
fn remember_with(db: &Db, last: &str, text: &str, route: &str, emb: &[f32]) {
    for (id, l, _, _, e) in db.route_examples() {
        if l == last && crate::memory::sim(&e, emb) >= SUPERSEDE {
            db.route_example_delete(id);
        }
    }
    db.route_example_put(last, text, route, emb);
}

/// What the user taught before about something like this, if anything. Costs nothing until the first correction.
pub fn recall(db: &Db, last: &str, text: &str) -> Option<String> {
    let examples = db.route_examples();
    if examples.is_empty() {
        return None;
    }
    closest(&examples, last, &embedding(text)?).map(|(route, s)| {
        eprintln!("[router] remembered correction ({s:.2}) -> {route}");
        route
    })
}

/// Called once per turn with the route it took. Learns when this turn corrects the one before.
pub fn observe(db: &Db, text: &str, last: &str, route: &str) {
    let t = now();
    let prev: Option<Turn> = db.kv_get("last_turn").and_then(|j| serde_json::from_str(&j).ok());
    if let Some(p) = prev.filter(|p| p.route == "chat" && matches!(route, "do" | "screen" | "research") && t - p.at <= WINDOW_SECS && is_correction(text)) {
        if let Some(e) = embedding(&p.text) {
            eprintln!("[router] learned: {:?} after {:?} means {route}", p.text, p.last);
            remember_with(db, &p.last, &p.text, route, &e);
        }
    }
    db.kv_set("last_turn", &serde_json::to_string(&Turn { text: text.into(), last: last.into(), route: route.into(), at: t }).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(name: &str) -> Db {
        Db::open(&std::env::temp_dir().join(format!("sunday-corr-{name}-{}.db", std::process::id()))).unwrap()
    }

    #[test]
    fn a_correction_needs_an_explicit_marker() {
        assert!(is_correction("I said can you sort by the best?"));
        assert!(is_correction("no, pull up the best videos please"));
        assert!(is_correction("Actually open Notes"));
        assert!(!is_correction("open Safari"));
        // Real log: a garbled "sort by the best", then another garble that shares "the best". Not a correction.
        assert!(!is_correction("what is on my screen by the best."));
        assert!(!is_correction("sort by newest first please"), "the same words again are not enough");
    }

    #[test]
    fn only_a_close_example_in_the_same_context_counts() {
        let ex = vec![(1, "do".to_string(), "ranked by the best".to_string(), "do".to_string(), vec![1.0, 0.0])];
        assert_eq!(closest(&ex, "do", &[0.99, 0.1]).map(|c| c.0), Some("do".into()));
        assert!(closest(&ex, "do", &[0.0, 1.0]).is_none(), "something unrelated");
        assert!(closest(&ex, "screen", &[1.0, 0.0]).is_none(), "another context");
    }

    #[test]
    fn a_new_example_replaces_one_it_contradicts() {
        let db = db("supersede");
        remember_with(&db, "do", "best", "do", &[1.0, 0.0]);
        remember_with(&db, "do", "best", "research", &[1.0, 0.01]);
        let ex = db.route_examples();
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].3, "research");
        remember_with(&db, "do", "newest", "do", &[0.0, 1.0]);
        assert_eq!(db.route_examples().len(), 2, "an unrelated example stays");
    }

    #[test]
    fn learns_only_from_a_correction_of_a_chat_turn() {
        let db = db("observe");
        // Plain turns teach nothing.
        observe(&db, "Ranked by the best.", "do", "chat");
        observe(&db, "hello", "chat", "chat");
        assert!(db.route_examples().is_empty());
        // An action that isn't a restatement teaches nothing either (no embedder here, so nothing could be stored anyway).
        observe(&db, "open Notes", "chat", "do");
        assert!(db.route_examples().is_empty());
    }
}
