//! SQLite: the authoritative record of what was said, what was asked, and when to ask again.
//! Evidence (attempts) is stored raw; the review schedule is derived and can be rebuilt from it.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

const DAY: i64 = 86_400;

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Correct,
    Partial,
    Wrong,
}

/// A question asked and not yet answered; survives restarts so a session can be resumed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub concept: String,
    pub question: String,
    pub answer: String,
}

/// (interval in days, ease, streak) after a graded attempt. Leitner/SM-2 style: explainable, no tuning yet.
fn next(interval: f64, ease: f64, streak: i64, v: Verdict) -> (f64, f64, i64) {
    match v {
        Verdict::Correct => {
            let streak = streak + 1;
            let interval = match streak {
                1 => 1.0,
                2 => 3.0,
                _ => (interval * ease).round(),
            };
            (interval, (ease + 0.05).min(2.8), streak)
        }
        Verdict::Partial => ((interval * 0.5).max(1.0), (ease - 0.05).max(1.3), streak),
        Verdict::Wrong => (1.0, (ease - 0.2).max(1.3), 0),
    }
}

#[derive(Serialize)]
pub struct Due {
    pub concept: String,
    pub overdue_days: i64,
}
#[derive(Serialize)]
pub struct Weak {
    pub concept: String,
    pub wrong: i64,
}
#[derive(Serialize)]
pub struct Recent {
    pub concept: String,
    pub verdict: String,
    pub ts: i64,
}
#[derive(Serialize)]
pub struct Msg {
    pub role: String,
    pub text: String,
}
#[derive(Serialize)]
pub struct State {
    pub due: Vec<Due>,
    pub total: i64,
    pub correct: i64,
    pub weak: Vec<Weak>,
    pub recent: Vec<Recent>,
    pub pending: Option<String>,
}

pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    fn init(c: Connection) -> rusqlite::Result<Self> {
        // ponytail: one schema version; add PRAGMA user_version migrations when the schema first changes.
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY, ts INTEGER, role TEXT, text TEXT);
             CREATE TABLE IF NOT EXISTS attempts(id INTEGER PRIMARY KEY, ts INTEGER, concept TEXT, question TEXT,
                 answer TEXT, verdict TEXT, error_type TEXT, feedback TEXT);
             CREATE TABLE IF NOT EXISTS reviews(concept TEXT PRIMARY KEY, due INTEGER, interval_days REAL,
                 ease REAL, streak INTEGER);
             CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT);",
        )?;
        Ok(Self(Mutex::new(c)))
    }

    pub fn message(&self, role: &str, text: &str) {
        let c = self.0.lock().unwrap();
        let _ = c.execute("INSERT INTO messages(ts, role, text) VALUES (?, ?, ?)", params![now(), role, text]);
    }

    /// Oldest first.
    pub fn history(&self, limit: i64) -> Vec<Msg> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT role, text FROM (SELECT * FROM messages ORDER BY id DESC LIMIT ?) ORDER BY id").unwrap();
        st.query_map([limit], |r| Ok(Msg { role: r.get(0)?, text: r.get(1)? })).unwrap().flatten().collect()
    }

    /// Records the evidence, then updates the concept's review schedule from it.
    pub fn attempt(&self, p: &Pending, said: &str, v: Verdict, error_type: &str, feedback: &str) {
        let c = self.0.lock().unwrap();
        let verdict = serde_json::to_value(v).unwrap();
        let _ = c.execute(
            "INSERT INTO attempts(ts, concept, question, answer, verdict, error_type, feedback) VALUES (?,?,?,?,?,?,?)",
            params![now(), p.concept, p.question, said, verdict.as_str(), error_type, feedback],
        );
        let (i, e, s) = c
            .query_row("SELECT interval_days, ease, streak FROM reviews WHERE concept = ?", [&p.concept], |r| {
                Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?))
            })
            .optional()
            .unwrap()
            .unwrap_or((0.0, 2.5, 0));
        let (i, e, s) = next(i, e, s, v);
        let _ = c.execute(
            "INSERT OR REPLACE INTO reviews(concept, due, interval_days, ease, streak) VALUES (?,?,?,?,?)",
            params![p.concept, now() + (i * DAY as f64) as i64, i, e, s],
        );
    }

    /// Most overdue first.
    pub fn due(&self) -> Vec<Due> {
        let c = self.0.lock().unwrap();
        let t = now();
        let mut st = c.prepare("SELECT concept, due FROM reviews WHERE due <= ? ORDER BY due").unwrap();
        st.query_map([t], |r| Ok(Due { concept: r.get(0)?, overdue_days: (t - r.get::<_, i64>(1)?) / DAY })).unwrap().flatten().collect()
    }

    pub fn weak(&self) -> Vec<Weak> {
        let c = self.0.lock().unwrap();
        let mut st = c
            .prepare("SELECT concept, COUNT(*) n FROM attempts WHERE verdict = 'wrong' GROUP BY concept ORDER BY n DESC LIMIT 5")
            .unwrap();
        st.query_map([], |r| Ok(Weak { concept: r.get(0)?, wrong: r.get(1)? })).unwrap().flatten().collect()
    }

    pub fn recent_questions(&self, concept: &str, n: i64) -> Vec<String> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT question FROM attempts WHERE concept = ? ORDER BY id DESC LIMIT ?").unwrap();
        st.query_map(params![concept, n], |r| r.get(0)).unwrap().flatten().collect()
    }

    pub fn pending(&self) -> Option<Pending> {
        let c = self.0.lock().unwrap();
        let v: Option<String> = c.query_row("SELECT value FROM kv WHERE key = 'pending'", [], |r| r.get(0)).optional().unwrap();
        v.and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn set_pending(&self, p: Option<&Pending>) {
        let c = self.0.lock().unwrap();
        let _ = match p {
            Some(p) => c.execute("INSERT OR REPLACE INTO kv(key, value) VALUES ('pending', ?)", [serde_json::to_string(p).unwrap()]),
            None => c.execute("DELETE FROM kv WHERE key = 'pending'", []),
        };
    }

    pub fn state(&self) -> State {
        let (total, correct) = {
            let c = self.0.lock().unwrap();
            c.query_row("SELECT COUNT(*), COALESCE(SUM(verdict = 'correct'), 0) FROM attempts", [], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap_or((0, 0))
        };
        let recent = {
            let c = self.0.lock().unwrap();
            let mut st = c.prepare("SELECT concept, verdict, ts FROM attempts ORDER BY id DESC LIMIT 8").unwrap();
            st.query_map([], |r| Ok(Recent { concept: r.get(0)?, verdict: r.get(1)?, ts: r.get(2)? })).unwrap().flatten().collect()
        };
        State { due: self.due(), total, correct, weak: self.weak(), recent, pending: self.pending().map(|p| p.question) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(concept: &str) -> Pending {
        Pending { concept: concept.into(), question: format!("What is {concept}?"), answer: "x".into() }
    }

    #[test]
    fn schedule_grows_on_success_and_resets_on_failure() {
        let (i, e, s) = next(0.0, 2.5, 0, Verdict::Correct);
        assert_eq!((i, s), (1.0, 1));
        let (i, e, s) = next(i, e, s, Verdict::Correct);
        assert_eq!((i, s), (3.0, 2));
        let (i, e, s) = next(i, e, s, Verdict::Correct);
        assert!(i >= 7.0 && s == 3, "interval {i}");
        let (i, e2, s) = next(i, e, s, Verdict::Wrong);
        assert_eq!((i, s), (1.0, 0));
        assert!(e2 < e);
    }

    #[test]
    fn attempts_drive_reviews_and_state() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        assert!(db.due().is_empty());
        db.attempt(&p("graphs"), "no idea", Verdict::Wrong, "recall", "It is a set of nodes and edges.");
        db.attempt(&p("sorting"), "n log n", Verdict::Correct, "none", "Yes.");
        // Wrong answers come back after a day; correct ones too (first interval is 1 day), none is due now.
        assert!(db.due().is_empty());
        db.0.lock().unwrap().execute("UPDATE reviews SET due = ? WHERE concept = 'graphs'", [now() - 3 * DAY]).unwrap();
        let due = db.due();
        assert_eq!((due.len(), due[0].concept.as_str(), due[0].overdue_days), (1, "graphs", 3));
        let s = db.state();
        assert_eq!((s.total, s.correct), (2, 1));
        assert_eq!(s.weak[0].concept, "graphs");
        assert_eq!(db.recent_questions("graphs", 5), vec!["What is graphs?"]);
    }

    #[test]
    fn pending_and_history_round_trip() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(db.pending(), None);
        db.set_pending(Some(&p("dp")));
        assert_eq!(db.pending(), Some(p("dp")));
        db.set_pending(None);
        assert_eq!(db.pending(), None);
        for i in 0..5 {
            db.message("user", &format!("m{i}"));
        }
        let h = db.history(3);
        assert_eq!(h.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["m2", "m3", "m4"]);
    }
}
