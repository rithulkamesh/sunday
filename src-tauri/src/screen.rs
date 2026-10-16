//! Screen reading and drawing. On request only: grab one screenshot, ask the local vision model
//! what matters and where, speak the explanation, and draw marks on a click-through overlay.
//! The image never leaves the machine and its file is deleted as soon as it has been read.
use crate::db::Db;
use crate::llm;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const MAX_MARKS: usize = 6;
/// Longest screenshot edge sent to the model: enough to read text, small enough to be quick.
const MAX_EDGE: &str = "896"; // the OCR carries the fine text, so the image can be small

/// A thing to point at, in 0..1000 coordinates relative to the screenshot (top-left origin).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub label: String,
    /// Which numbered line of on-screen text this points at (-1: none, use x/y/w/h).
    #[serde(default = "no_line")]
    pub line: i64,
}

fn no_line() -> i64 {
    -1
}

/// A line of text found on screen, in the same 0..1000 space as marks.
#[derive(Deserialize, Clone, Debug)]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

struct Seen {
    image: String,
    lines: Vec<Line>,
}

#[derive(Deserialize)]
struct Look {
    explanation: String,
    #[serde(default)]
    marks: Vec<Mark>,
}

const SYSTEM: &str = "You see a screenshot of the user's screen and explain what they ask about, like a patient tutor sitting next to them. Speak to them as \"you\"; never begin with \"The user\". \
`explanation` is at most three short spoken sentences: plain words, no markdown, and refer to things by what they say or look like. \
`marks` point at the specific things you mention, at most four. Point at the content the user is working on, never at window chrome \
(menu bar, tabs, title bars, status bars, notifications) unless they ask. `label` says what the marked thing is about, not where it is on screen. kind is box (a region), circle (a small item), underline (a line of text) or arrow (a pointer to an item). \
label is at most four words. If nothing needs pointing at, return no marks. \
You may be given the screen's text as numbered lines. To point at text, set `line` to that number: the position is then exact, so set x,y,w,h to 0. \
Only for things that are not text (pictures, diagrams, icons) set `line` to -1 and give x,y,w,h as 0 to 1000 relative to the image, origin top-left, x,y the top-left corner.";

fn schema() -> Value {
    json!({"type": "object", "required": ["explanation", "marks"], "properties": {
        "explanation": {"type": "string"},
        "marks": {"type": "array", "items": {"type": "object", "required": ["kind", "line", "x", "y", "w", "h", "label"], "properties": {
            "kind": {"type": "string", "enum": ["box", "circle", "underline", "arrow"]},
            "line": {"type": "integer"},
            "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"},
            "label": {"type": "string"}}}}}})
}

/// Marks that name a text line take that line's exact box from the OCR. The rest are the model's own
/// guess, which can land outside the image or on nothing, so those are clamped; undrawable ones are dropped.
fn sanitize(marks: Vec<Mark>, lines: &[Line]) -> Vec<Mark> {
    marks
        .into_iter()
        .map(|mut m| {
            if let Some(l) = usize::try_from(m.line).ok().and_then(|i| lines.get(i)) {
                (m.x, m.y, m.w, m.h) = (l.x, l.y, l.w, l.h);
                if m.kind == "circle" {
                    m.kind = "box".into(); // a circle round a line of text looks wrong
                }
            } else {
                m.line = -1;
            }
            m
        })
        .filter(|m| matches!(m.kind.as_str(), "box" | "circle" | "underline" | "arrow") && [m.x, m.y, m.w, m.h].iter().all(|v| v.is_finite()))
        .map(|mut m| {
            m.x = m.x.clamp(0.0, 985.0);
            m.y = m.y.clamp(0.0, 985.0);
            m.w = m.w.clamp(15.0, 1000.0 - m.x);
            m.h = m.h.clamp(15.0, 1000.0 - m.y);
            m.label.truncate(40);
            m
        })
        .take(MAX_MARKS)
        .collect()
}

#[cfg(target_os = "macos")]
fn screen_access() -> bool {
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    // SAFETY: plain CoreGraphics queries with no arguments. The request call shows the system prompt once.
    unsafe { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() }
}
#[cfg(not(target_os = "macos"))]
fn screen_access() -> bool {
    true
}

fn ocr_tool() -> Option<std::path::PathBuf> {
    // scripts/build-ocr.sh puts it next to the models directory.
    let p = crate::models::dir().parent()?.join("bin/ocr");
    p.exists().then_some(p)
}

/// Text on screen with exact boxes. Empty (never an error) if the reader isn't built or fails:
/// pointing then falls back to the vision model's own coordinates.
fn read_text(image: &std::path::Path) -> Vec<Line> {
    let Some(tool) = ocr_tool() else {
        eprintln!("[screen] no OCR tool (run scripts/build-ocr.sh): pointing will be approximate");
        return vec![];
    };
    let out = std::process::Command::new(tool).arg(image).output().ok().filter(|o| o.status.success());
    out.and_then(|o| serde_json::from_slice::<Vec<Line>>(&o.stdout).ok()).unwrap_or_default()
}

/// The main display: its text (read at full resolution) and a smaller JPEG for the model. The files are deleted at once.
fn capture() -> Res<Seen> {
    if !screen_access() {
        return Err(llm::UserMsg("I need screen recording permission. Allow it in System Settings, Privacy and Security, Screen Recording, then restart me.".into()).into());
    }
    let dir = std::env::temp_dir();
    let (full, small) = (dir.join(format!("sunday-screen-{}.jpg", std::process::id())), dir.join(format!("sunday-screen-{}-small.jpg", std::process::id())));
    let run = || -> Res<(Vec<u8>, Vec<Line>)> {
        let ok = std::process::Command::new("screencapture").args(["-x", "-m", "-t", "jpg"]).arg(&full).status()?.success();
        if !ok {
            return Err("the screenshot failed".into());
        }
        let lines = read_text(&full);
        // Smaller is faster for the model; falls back to the full image if resizing fails.
        let resized = std::process::Command::new("sips").args(["-Z", MAX_EDGE, "--out"]).arg(&small).arg(&full).output().is_ok_and(|o| o.status.success());
        Ok((std::fs::read(if resized { &small } else { &full })?, lines))
    };
    let res = run();
    let _ = std::fs::remove_file(&full);
    let _ = std::fs::remove_file(&small);
    let (bytes, lines) = res?;
    if bytes.len() < 2_000 {
        return Err("the screenshot came back empty".into());
    }
    eprintln!("[screen] looked once ({} KB, {} text lines), image discarded", bytes.len() / 1024, lines.len());
    Ok(Seen { image: base64::engine::general_purpose::STANDARD.encode(bytes), lines })
}

/// The task plus, when the OCR found text, the numbered lines the model can point at.
fn prompt(task: &str, lines: &[Line]) -> String {
    if lines.is_empty() {
        return task.to_string();
    }
    let list: Vec<String> = lines.iter().take(80).enumerate().map(|(i, l)| format!("[{i}] {}", l.text.chars().take(60).collect::<String>())).collect();
    format!("{task}\n\nText on the screen:\n{}", list.join("\n"))
}

fn look(model: &str, seen: &Seen, task: &str) -> Res<Look> {
    // The line list plus the image need more than Ollama's default 4096-token context.
    let body = json!({"model": model, "stream": false, "think": false, "keep_alive": "2m", "format": schema(), "options": {"num_ctx": 8192},
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": prompt(task, &seen.lines), "images": [seen.image]}]});
    for _ in 0..2 {
        let v: Value = match ureq::post("http://localhost:11434/api/chat").timeout(std::time::Duration::from_secs(240)).send_json(&body) {
            Ok(r) => r.into_json()?,
            Err(ureq::Error::Status(404, _)) => return Err(llm::UserMsg(format!("The vision model {model} isn't installed. Run ollama pull {model}.")).into()),
            Err(e) => return Err(e.into()),
        };
        // Thinking-style models put schema-constrained output in `thinking` and leave `content` empty.
        let m = &v["message"];
        let text = [m["content"].as_str(), m["thinking"].as_str()].into_iter().flatten().find(|t| !t.trim().is_empty()).unwrap_or("");
        if let Ok(l) = serde_json::from_str::<Look>(text) {
            return Ok(l);
        }
    }
    Err("the vision model returned invalid JSON twice".into())
}

/// Window chrome the model keeps pointing at: the menu bar is the top few percent of the main display.
/// Dropped before numbering, so indices stay consistent, unless the task is about it.
fn drop_chrome(mut lines: Vec<Line>, task: &str) -> Vec<Line> {
    let t = task.to_lowercase();
    if !(t.contains("menu bar") || t.contains("menubar") || t.contains("clock") || t.contains("battery")) {
        lines.retain(|l| l.y >= 35.0);
    }
    lines
}

/// Looks at the screen for `task`. Returns the spoken explanation and the marks to draw.
pub fn explain(app: &AppHandle, db: &Db, task: &str) -> Res<(String, Vec<Mark>)> {
    clear(app); // don't photograph our own drawing
    std::thread::sleep(std::time::Duration::from_millis(250));
    let mut seen = capture()?;
    seen.lines = drop_chrome(std::mem::take(&mut seen.lines), task);
    let l = look(&llm::vision_model(db), &seen, task)?;
    Ok((l.explanation.trim().to_string(), sanitize(l.marks, &seen.lines)))
}

/// Bumped on every show/clear so a late auto-hide can tell it was superseded.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Draws `marks` over the main display for `ttl` seconds.
pub fn show(app: &AppHandle, marks: &[Mark], ttl: u64) {
    let Some(win) = app.get_webview_window("overlay") else { return };
    let epoch = EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(Some(m)) = app.primary_monitor() {
        let _ = win.set_size(PhysicalSize::new(m.size().width, m.size().height));
        let _ = win.set_position(PhysicalPosition::new(m.position().x, m.position().y));
    }
    crate::order_front(app, &win);
    let _ = app.emit("overlay", json!({"marks": marks}));
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(ttl));
        if EPOCH.load(Ordering::SeqCst) == epoch {
            clear(&app);
        }
    });
}

pub fn clear(app: &AppHandle) {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    let _ = app.emit("overlay", json!({"marks": []}));
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(kind: &str, x: f64, y: f64, w: f64, h: f64) -> Mark {
        Mark { kind: kind.into(), x, y, w, h, label: "label".repeat(20), line: -1 }
    }

    #[test]
    fn marks_are_clamped_and_filtered() {
        let out = sanitize(vec![
            m("box", 900.0, 950.0, 500.0, 500.0), // runs off the screen
            m("laser", 1.0, 1.0, 10.0, 10.0),      // unknown shape
            m("circle", f64::NAN, 1.0, 10.0, 10.0),
            m("underline", -50.0, 10.0, 0.0, 0.0), // empty region
        ], &[]);
        assert_eq!(out.len(), 2);
        assert!(out[0].x + out[0].w <= 1000.0 && out[0].y + out[0].h <= 1000.0);
        assert_eq!((out[1].x, out[1].w, out[1].h), (0.0, 15.0, 15.0));
        assert!(out[0].label.len() <= 40);
        assert_eq!(sanitize((0..10).map(|i| m("box", i as f64, 0.0, 20.0, 20.0)).collect(), &[]).len(), MAX_MARKS);
    }

    #[test]
    fn menu_bar_text_is_not_offered_unless_asked() {
        let l = |y: f64| Line { text: "x".into(), x: 0.0, y, w: 10.0, h: 10.0 };
        assert_eq!(drop_chrome(vec![l(7.0), l(300.0)], "what's on my screen").len(), 1);
        assert_eq!(drop_chrome(vec![l(7.0), l(300.0)], "what time does the menu bar clock say").len(), 2);
    }

    #[test]
    fn text_marks_snap_to_ocr_lines() {
        let lines = vec![
            Line { text: "first".into(), x: 20.0, y: 100.0, w: 300.0, h: 14.0 },
            Line { text: "second".into(), x: 20.0, y: 140.0, w: 250.0, h: 14.0 },
        ];
        let mut a = m("circle", 700.0, 700.0, 50.0, 50.0); // the model's guess is far off...
        a.line = 1; // ...but it named line 1
        let mut b = m("box", 5.0, 5.0, 40.0, 40.0);
        b.line = 9; // no such line: falls back to its own box
        let out = sanitize(vec![a, b], &lines);
        assert_eq!((out[0].x, out[0].y, out[0].w, out[0].h, out[0].kind.as_str()), (20.0, 140.0, 250.0, 15.0, "box"));
        assert_eq!((out[1].line, out[1].x), (-1, 5.0));
        assert!(prompt("explain", &lines).contains("[1] second"));
        assert_eq!(prompt("explain", &[]), "explain");
    }

    /// Needs the vision model and screen-recording permission; run by hand with SUNDAY_SCREEN_TEST=1.
    #[test]
    fn looks_at_the_real_screen() {
        if std::env::var("SUNDAY_SCREEN_TEST").is_err() {
            return;
        }
        let seen = capture().expect("capture");
        let t = std::time::Instant::now();
        let l = look(&std::env::var("SUNDAY_VISION").unwrap_or_else(|_| llm::VISION.into()), &seen, "What is on my screen? Point at the most important thing.").expect("look");
        println!("{:?} -> {}\n{:?}", t.elapsed(), l.explanation, sanitize(l.marks, &seen.lines));
    }
}
