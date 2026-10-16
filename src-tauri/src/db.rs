//! SQLite: the authoritative record of what was said, what was asked, and when to ask again.
//! Evidence (attempts) is stored raw; the review schedule is derived and can be rebuilt from it.
use rusqlite::{params, Connection, OptionalExtension};
use crate::skills::Manifest as Skill;
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
pub struct Node {
    pub name: String,
    pub subject: String,
    pub status: &'static str,
    pub needs: Vec<String>,
}
#[derive(Serialize)]
pub struct Doc {
    pub id: i64,
    pub title: String,
    pub chunks: i64,
}
/// A passage of the user's notes, with where it came from.
#[derive(Clone, Serialize)]
pub struct Hit {
    pub title: String,
    pub n: i64,
    pub text: String,
}
/// One remembered fact and where it came from. `kind`: "user" (you told me), "web" (read from a page),
/// "chat" (learned from a conversation, so only a guess until you confirm it).
#[derive(Clone, Serialize)]
pub struct Fact {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub source: String,
    pub title: String,
    pub text: String,
    pub confidence: f64,
    pub event_date: String,
    pub score: f32,
}
pub struct NewFact<'a> {
    pub kind: &'a str,
    pub source: &'a str,
    pub title: &'a str,
    pub text: &'a str,
    pub confidence: f64,
    pub event_date: &'a str,
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
    /// Concepts known / in the learning map.
    pub coverage: (usize, usize),
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
             CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE IF NOT EXISTS concepts(name TEXT PRIMARY KEY, subject TEXT, ord INTEGER);
             CREATE TABLE IF NOT EXISTS prereqs(concept TEXT, requires TEXT, PRIMARY KEY(concept, requires));
             CREATE TABLE IF NOT EXISTS documents(id INTEGER PRIMARY KEY, ts INTEGER, title TEXT, path TEXT, hash TEXT UNIQUE);
             CREATE TABLE IF NOT EXISTS skills(id TEXT, version INTEGER, manifest TEXT, status TEXT, PRIMARY KEY(id, version));
             CREATE TABLE IF NOT EXISTS skill_runs(id INTEGER PRIMARY KEY, ts INTEGER, skill TEXT, version INTEGER, input TEXT, output TEXT, ms INTEGER, error TEXT);
             CREATE TABLE IF NOT EXISTS facts(id INTEGER PRIMARY KEY, ts INTEGER, kind TEXT, source TEXT, title TEXT, text TEXT,
                 confidence REAL, event_date TEXT, embedding BLOB);
             CREATE TABLE IF NOT EXISTS actions(id INTEGER PRIMARY KEY, ts INTEGER, task TEXT, tool TEXT, args TEXT, outcome TEXT);
             CREATE VIRTUAL TABLE IF NOT EXISTS chunks USING fts5(text, doc UNINDEXED, n UNINDEXED);",
        )?;
        let db = Self(Mutex::new(c));
        for m in crate::skills::builtins() {
            if db.skill(&m.id, Some(m.version)).is_none() {
                db.skill_put(&m);
            }
        }
        Ok(db)
    }

    /// Loads a text pack ("# Subject" headers, "concept < prereq, prereq" lines). Only tests use it: plans are built on demand.
    #[cfg(test)]
    /// Loads a domain pack: "# Subject" headers, then "concept < prereq, prereq" lines. Safe to re-run.
    pub fn seed(&self, pack: &str) {
        let c = self.0.lock().unwrap();
        let (mut subject, mut n) = (String::new(), 0);
        for line in pack.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(h) = line.strip_prefix('#') {
                subject = h.trim().to_string();
                continue;
            }
            let (name, needs) = line.split_once('<').unwrap_or((line, ""));
            let name = name.trim().to_lowercase();
            n += 1;
            let _ = c.execute("INSERT OR REPLACE INTO concepts(name, subject, ord) VALUES (?,?,?)", params![name, subject, n]);
            for r in needs.split(',').map(|r| r.trim().to_lowercase()).filter(|r| !r.is_empty()) {
                let _ = c.execute("INSERT OR IGNORE INTO prereqs(concept, requires) VALUES (?,?)", params![name, r]);
            }
        }
    }

    /// Stores a learning plan: `concepts` in learning order, each with the earlier ones it builds on. Safe to repeat.
    pub fn add_plan(&self, subject: &str, concepts: &[(String, Vec<String>)]) {
        let c = self.0.lock().unwrap();
        let mut n: i64 = c.query_row("SELECT COALESCE(MAX(ord), 0) FROM concepts", [], |r| r.get(0)).unwrap_or(0);
        for (name, needs) in concepts {
            n += 1;
            let _ = c.execute("INSERT OR IGNORE INTO concepts(name, subject, ord) VALUES (?,?,?)", params![name, subject, n]);
            for r in needs {
                let _ = c.execute("INSERT OR IGNORE INTO prereqs(concept, requires) VALUES (?,?)", params![name, r]);
            }
        }
    }

    pub fn has_subject(&self, subject: &str) -> bool {
        self.0.lock().unwrap().query_row("SELECT 1 FROM concepts WHERE subject = ? LIMIT 1", [subject], |_| Ok(())).optional().unwrap().is_some()
    }

    /// The next concept of `subject` to teach: not introduced yet, with every prerequisite at least introduced
    /// (teaching moves on as the learner is introduced to things; quizzes are what turn "learning" into "known").
    pub fn next_to_teach(&self, subject: &str) -> Option<String> {
        let g = self.graph();
        let introduced = |n: &str| g.iter().any(|x| x.name == n && x.status != "new");
        g.iter().find(|n| n.subject == subject && n.status == "new" && n.needs.iter().all(|r| introduced(r))).map(|n| n.name.clone())
    }

    /// Marks a concept as introduced: it joins the review schedule (first review tomorrow) as "learning".
    pub fn introduce(&self, concept: &str) {
        let _ = self.0.lock().unwrap().execute(
            "INSERT OR IGNORE INTO reviews(concept, due, interval_days, ease, streak) VALUES (?,?,?,?,?)",
            params![concept, now() + DAY, 1.0, 2.5, 0],
        );
    }

    /// The learning map with the learner's status on each concept: new, learning, or known (two correct reviews in a row).
    pub fn graph(&self) -> Vec<Node> {
        let c = self.0.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT c.name, c.subject, COALESCE(r.streak, -1), (SELECT group_concat(requires, '|') FROM prereqs WHERE concept = c.name)
                 FROM concepts c LEFT JOIN reviews r ON r.concept = c.name ORDER BY c.ord",
            )
            .unwrap();
        st.query_map([], |r| {
            let streak: i64 = r.get(2)?;
            let needs: Option<String> = r.get(3)?;
            Ok(Node {
                name: r.get(0)?,
                subject: r.get(1)?,
                status: if streak >= 2 { "known" } else if streak >= 0 { "learning" } else { "new" },
                needs: needs.map(|n| n.split('|').map(String::from).collect()).unwrap_or_default(),
            })
        })
        .unwrap()
        .flatten()
        .collect()
    }

    /// The next concept to practise: not started, with every prerequisite known (plan order).
    pub fn frontier(&self) -> Option<String> {
        let g = self.graph();
        let known = |n: &str| g.iter().any(|x| x.name == n && x.status == "known");
        g.iter().find(|n| n.status == "new" && n.needs.iter().all(|r| known(r))).map(|n| n.name.clone())
    }

    /// Prerequisites of `concept` the learner hasn't shown they know.
    pub fn gaps(&self, concept: &str) -> Vec<String> {
        let g = self.graph();
        let Some(n) = g.iter().find(|n| n.name == concept) else { return vec![] };
        n.needs.iter().filter(|r| !g.iter().any(|x| &x.name == *r && x.status == "known")).cloned().collect()
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

    /// Indexes a document unless the same content is already there. Returns the chunk count, or None if a duplicate.
    pub fn add_document(&self, title: &str, path: &str, hash: &str, chunks: &[String]) -> Option<usize> {
        let c = self.0.lock().unwrap();
        c.execute("INSERT INTO documents(ts, title, path, hash) VALUES (?,?,?,?)", params![now(), title, path, hash]).ok()?;
        let id = c.last_insert_rowid();
        for (n, t) in chunks.iter().enumerate() {
            let _ = c.execute("INSERT INTO chunks(text, doc, n) VALUES (?,?,?)", params![t, id, n as i64 + 1]);
        }
        Some(chunks.len())
    }

    pub fn docs(&self) -> Vec<Doc> {
        let c = self.0.lock().unwrap();
        let mut st = c
            .prepare("SELECT d.id, d.title, (SELECT COUNT(*) FROM chunks WHERE doc = d.id) FROM documents d ORDER BY d.id DESC")
            .unwrap();
        st.query_map([], |r| Ok(Doc { id: r.get(0)?, title: r.get(1)?, chunks: r.get(2)? })).unwrap().flatten().collect()
    }

    pub fn remove_document(&self, id: i64) {
        let c = self.0.lock().unwrap();
        let _ = c.execute("DELETE FROM chunks WHERE doc = ?", [id]);
        let _ = c.execute("DELETE FROM documents WHERE id = ?", [id]);
    }

    /// Best passages for a question: BM25 candidates, kept only if they contain at least half
    /// of the query terms, so a stray common word never counts as a source.
    pub fn search(&self, q: &str, k: usize) -> Vec<Hit> {
        let terms = crate::notes::terms(q);
        if terms.is_empty() {
            return vec![];
        }
        let c = self.0.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT d.title, ch.n, ch.text FROM chunks ch JOIN documents d ON d.id = ch.doc
                 WHERE chunks MATCH ? ORDER BY bm25(chunks) LIMIT 20",
            )
            .unwrap();
        let need = terms.len().div_ceil(2);
        st.query_map([crate::notes::fts_query(&terms)], |r| Ok(Hit { title: r.get(0)?, n: r.get(1)?, text: r.get(2)? }))
            .unwrap()
            .flatten()
            .filter(|h| {
                let t = h.text.to_lowercase();
                terms.iter().filter(|w| t.contains(w.as_str())).count() >= need
            })
            .take(k)
            .collect()
    }

    pub fn kv_get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().query_row("SELECT value FROM kv WHERE key = ?", [key], |r| r.get(0)).optional().unwrap()
    }

    /// The audit trail of everything Sunday did (or was told not to do) on the computer.
    pub fn act_log(&self, task: &str, tool: &str, args: &str, outcome: &str) {
        let _ = self.0.lock().unwrap().execute("INSERT INTO actions(ts, task, tool, args, outcome) VALUES (?,?,?,?,?)", params![now(), task, tool, args, outcome]);
    }

    pub fn kv_set(&self, key: &str, value: &str) {
        let _ = self.0.lock().unwrap().execute("INSERT OR REPLACE INTO kv(key, value) VALUES (?, ?)", params![key, value]);
    }

    pub fn skill_put(&self, m: &Skill) {
        let _ = self.0.lock().unwrap().execute(
            "INSERT OR REPLACE INTO skills(id, version, manifest, status) VALUES (?,?,?,?)",
            params![m.id, m.version, serde_json::to_string(m).unwrap(), m.status],
        );
    }

    /// A specific version, or the newest approved one.
    pub fn skill(&self, id: &str, version: Option<i64>) -> Option<Skill> {
        let c = self.0.lock().unwrap();
        let row: Option<(String, String)> = match version {
            Some(v) => c.query_row("SELECT manifest, status FROM skills WHERE id = ? AND version = ?", params![id, v], |r| Ok((r.get(0)?, r.get(1)?))).optional().unwrap(),
            None => c
                .query_row("SELECT manifest, status FROM skills WHERE id = ? AND status = 'approved' ORDER BY version DESC LIMIT 1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .unwrap(),
        };
        row.and_then(|(m, st)| serde_json::from_str::<Skill>(&m).ok().map(|mut m| { m.status = st; m }))
    }

    pub fn skills(&self) -> Vec<Skill> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT manifest, status FROM skills ORDER BY id, version DESC").unwrap();
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .flatten()
            .filter_map(|(m, st)| serde_json::from_str::<Skill>(&m).ok().map(|mut m| { m.status = st; m }))
            .collect()
    }

    pub fn skill_set_status(&self, id: &str, version: i64, status: &str) {
        let _ = self.0.lock().unwrap().execute("UPDATE skills SET status = ? WHERE id = ? AND version = ?", params![status, id, version]);
    }

    pub fn skill_run_logged(&self, skill: &str, version: i64, input: &str, output: &str, ms: i64, error: &str) {
        let _ = self.0.lock().unwrap().execute(
            "INSERT INTO skill_runs(ts, skill, version, input, output, ms, error) VALUES (?,?,?,?,?,?,?)",
            params![now(), skill, version, input, output, ms, error],
        );
    }

    pub fn fact_add(&self, f: &NewFact, embedding: Option<&[f32]>) -> i64 {
        let c = self.0.lock().unwrap();
        let blob: Option<Vec<u8>> = embedding.map(|e| e.iter().flat_map(|x| x.to_le_bytes()).collect());
        let _ = c.execute(
            "INSERT INTO facts(ts, kind, source, title, text, confidence, event_date, embedding) VALUES (?,?,?,?,?,?,?,?)",
            params![now(), f.kind, f.source, f.title, f.text, f.confidence, f.event_date, blob],
        );
        c.last_insert_rowid()
    }

    fn fact_row(r: &rusqlite::Row) -> rusqlite::Result<Fact> {
        Ok(Fact { id: r.get(0)?, ts: r.get(1)?, kind: r.get(2)?, source: r.get(3)?, title: r.get(4)?, text: r.get(5)?, confidence: r.get(6)?, event_date: r.get(7)?, score: 0.0 })
    }

    /// Newest first.
    pub fn facts(&self, limit: i64) -> Vec<Fact> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT id, ts, kind, source, title, text, confidence, event_date FROM facts ORDER BY id DESC LIMIT ?").unwrap();
        st.query_map([limit], Self::fact_row).unwrap().flatten().collect()
    }

    pub fn fact(&self, id: i64) -> Option<Fact> {
        let c = self.0.lock().unwrap();
        c.query_row("SELECT id, ts, kind, source, title, text, confidence, event_date FROM facts WHERE id = ?", [id], Self::fact_row).optional().unwrap()
    }

    /// (id, embedding) of every fact that has one, for similarity search.
    pub fn fact_embeddings(&self) -> Vec<(i64, Vec<f32>)> {
        let c = self.0.lock().unwrap();
        let mut st = c.prepare("SELECT id, embedding FROM facts WHERE embedding IS NOT NULL").unwrap();
        st.query_map([], |r| {
            let b: Vec<u8> = r.get(1)?;
            Ok((r.get(0)?, b.chunks_exact(4).map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]])).collect()))
        })
        .unwrap()
        .flatten()
        .collect()
    }

    pub fn fact_delete(&self, id: i64) {
        let _ = self.0.lock().unwrap().execute("DELETE FROM facts WHERE id = ?", [id]);
    }

    /// Forget everything learned from one source (a page, or "chat"). Returns how many.
    pub fn facts_delete_source(&self, source: &str) -> usize {
        self.0.lock().unwrap().execute("DELETE FROM facts WHERE source = ?", [source]).unwrap_or(0)
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
        let g = self.graph();
        let coverage = (g.iter().filter(|n| n.status == "known").count(), g.len());
        State { due: self.due(), total, correct, weak: self.weak(), recent, pending: self.pending().map(|p| p.question), coverage }
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

    const SAMPLE: &str = "# Logic\nprepositions\nconnectives < prepositions\nquantifiers < connectives\n# Sets\nsets\nrelations < sets\n";

    #[test]
    fn graph_frontier_and_gaps() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        assert!(db.graph().is_empty() && db.frontier().is_none(), "nothing is built in: plans are made on demand");
        db.seed(SAMPLE);
        assert_eq!(db.graph().len(), 5);
        assert_eq!(db.frontier().as_deref(), Some("prepositions")); // first concept with no prerequisites
        assert_eq!(db.gaps("connectives"), vec!["prepositions"]);
        // Two correct answers in a row make a concept "known", which unlocks what depends on it.
        let a = p("prepositions");
        db.attempt(&a, "x", Verdict::Correct, "none", "");
        db.attempt(&a, "x", Verdict::Correct, "none", "");
        assert!(db.gaps("connectives").is_empty());
        assert_eq!(db.frontier().as_deref(), Some("connectives"));
        assert_eq!(db.state().coverage, (1, 5));
    }

    #[test]
    fn plans_are_taught_in_order_for_any_subject() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        assert!(!db.has_subject("guitar"));
        let plan = vec![("open chords".to_string(), vec![]), ("strumming".to_string(), vec!["open chords".to_string()]), ("barre chords".to_string(), vec!["open chords".to_string(), "strumming".to_string()])];
        db.add_plan("guitar", &plan);
        db.add_plan("guitar", &plan); // repeating changes nothing
        assert!(db.has_subject("guitar") && db.graph().len() == 3);
        // Teaching moves on once a concept is introduced, without waiting for quiz mastery.
        assert_eq!(db.next_to_teach("guitar").as_deref(), Some("open chords"));
        db.introduce("open chords");
        assert_eq!(db.graph()[0].status, "learning");
        assert_eq!(db.next_to_teach("guitar").as_deref(), Some("strumming"));
        db.introduce("strumming");
        db.introduce("barre chords");
        assert_eq!(db.next_to_teach("guitar"), None);
        assert_eq!(db.next_to_teach("piano"), None);
        // A second subject sits beside the first.
        db.add_plan("baking", &[("measuring".to_string(), vec![])]);
        assert_eq!(db.next_to_teach("baking").as_deref(), Some("measuring"));
        assert!(db.due().is_empty(), "introduced concepts are due tomorrow, not now");
    }

    #[test]
    fn notes_are_searchable_with_sources() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        let os = vec!["A deadlock needs mutual exclusion, hold and wait, no preemption and circular wait.".to_string()];
        let net = vec!["TCP uses slow start and congestion avoidance to adapt its window.".to_string()];
        assert_eq!(db.add_document("os notes", "/x/os.md", "h1", &os), Some(1));
        assert_eq!(db.add_document("os notes again", "/x/os2.md", "h1", &os), None); // same content: skipped
        db.add_document("networks", "/x/net.md", "h2", &net);
        let hits = db.search("what are the conditions for deadlock?", 3);
        assert_eq!((hits.len(), hits[0].title.as_str(), hits[0].n), (1, "os notes", 1));
        assert!(db.search("explain quicksort pivots", 3).is_empty());
        assert!(db.search("the and for", 3).is_empty());
        assert_eq!(db.docs().len(), 2);
        db.remove_document(hits_doc_id(&db, "networks"));
        assert!(db.search("tcp congestion window", 3).is_empty());
    }

    fn hits_doc_id(db: &Db, title: &str) -> i64 {
        db.docs().iter().find(|d| d.title == title).unwrap().id
    }

    #[test]
    fn skills_resolve_to_the_newest_approved_version() {
        let db = Db::init(Connection::open_in_memory().unwrap()).unwrap();
        assert!(db.skill("learn.mnemonic", None).is_none()); // built-ins start experimental
        assert_eq!(db.skill("learn.mnemonic", Some(1)).unwrap().status, "experimental");
        db.skill_set_status("learn.mnemonic", 1, "approved");
        let mut v2 = db.skill("learn.mnemonic", Some(1)).unwrap();
        v2.version = 2;
        db.skill_put(&v2);
        db.skill_set_status("learn.mnemonic", 2, "approved");
        assert_eq!(db.skill("learn.mnemonic", None).unwrap().version, 2);
        db.skill_set_status("learn.mnemonic", 2, "deprecated"); // rollback
        assert_eq!(db.skill("learn.mnemonic", None).unwrap().version, 1);
        db.kv_set("model", "qwen3:4b");
        assert_eq!(db.kv_get("model").as_deref(), Some("qwen3:4b"));
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
