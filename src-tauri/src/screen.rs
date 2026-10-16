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
const MAX_EDGE: &str = "1280";

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
}

#[derive(Deserialize)]
struct Look {
    explanation: String,
    #[serde(default)]
    marks: Vec<Mark>,
}

const SYSTEM: &str = "You see a screenshot of the user's screen and explain what they ask about, like a patient tutor sitting next to them. \
`explanation` is at most three short spoken sentences: plain words, no markdown, and refer to things by what they say or look like. \
`marks` point at the specific things you mention, at most four. Coordinates are 0 to 1000 relative to the image, origin top-left: \
x,y is the top-left corner and w,h the size. kind is box (a region), circle (a small item), underline (a line of text) or arrow (a pointer to an item). \
label is at most four words. If nothing needs pointing at, return no marks.";

fn schema() -> Value {
    json!({"type": "object", "required": ["explanation", "marks"], "properties": {
        "explanation": {"type": "string"},
        "marks": {"type": "array", "items": {"type": "object", "required": ["kind", "x", "y", "w", "h", "label"], "properties": {
            "kind": {"type": "string", "enum": ["box", "circle", "underline", "arrow"]},
            "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"},
            "label": {"type": "string"}}}}}})
}

/// Models sometimes return coordinates outside the image or empty regions; keep only drawable marks.
fn sanitize(marks: Vec<Mark>) -> Vec<Mark> {
    marks
        .into_iter()
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

/// The main display as base64 JPEG, via the OS screenshot tool (no extra dependency, no stored file).
fn capture() -> Res<String> {
    if !screen_access() {
        return Err(llm::UserMsg("I need screen recording permission. Allow it in System Settings, Privacy and Security, Screen Recording, then restart me.".into()).into());
    }
    let path = std::env::temp_dir().join(format!("sunday-screen-{}.jpg", std::process::id()));
    let run = || -> Res<Vec<u8>> {
        let ok = std::process::Command::new("screencapture").args(["-x", "-m", "-t", "jpg"]).arg(&path).status()?.success();
        if !ok {
            return Err("the screenshot failed".into());
        }
        let _ = std::process::Command::new("sips").args(["-Z", MAX_EDGE]).arg(&path).output(); // best effort: smaller is faster
        Ok(std::fs::read(&path)?)
    };
    let bytes = run();
    let _ = std::fs::remove_file(&path);
    let bytes = bytes?;
    if bytes.len() < 2_000 {
        return Err("the screenshot came back empty".into());
    }
    eprintln!("[screen] looked once ({} KB), image discarded", bytes.len() / 1024);
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn look(model: &str, image: &str, task: &str) -> Res<Look> {
    let body = json!({"model": model, "stream": false, "think": false, "keep_alive": "2m", "format": schema(),
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": task, "images": [image]}]});
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

/// Looks at the screen for `task`. Returns the spoken explanation and the marks to draw.
pub fn explain(app: &AppHandle, db: &Db, task: &str) -> Res<(String, Vec<Mark>)> {
    clear(app); // don't photograph our own drawing
    std::thread::sleep(std::time::Duration::from_millis(250));
    let image = capture()?;
    let l = look(&llm::vision_model(db), &image, task)?;
    Ok((l.explanation.trim().to_string(), sanitize(l.marks)))
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
        Mark { kind: kind.into(), x, y, w, h, label: "label".repeat(20) }
    }

    #[test]
    fn marks_are_clamped_and_filtered() {
        let out = sanitize(vec![
            m("box", 900.0, 950.0, 500.0, 500.0), // runs off the screen
            m("laser", 1.0, 1.0, 10.0, 10.0),      // unknown shape
            m("circle", f64::NAN, 1.0, 10.0, 10.0),
            m("underline", -50.0, 10.0, 0.0, 0.0), // empty region
        ]);
        assert_eq!(out.len(), 2);
        assert!(out[0].x + out[0].w <= 1000.0 && out[0].y + out[0].h <= 1000.0);
        assert_eq!((out[1].x, out[1].w, out[1].h), (0.0, 15.0, 15.0));
        assert!(out[0].label.len() <= 40);
        assert_eq!(sanitize((0..10).map(|i| m("box", i as f64, 0.0, 20.0, 20.0)).collect()).len(), MAX_MARKS);
    }

    /// Needs the vision model and screen-recording permission; run by hand with SUNDAY_SCREEN_TEST=1.
    #[test]
    fn looks_at_the_real_screen() {
        if std::env::var("SUNDAY_SCREEN_TEST").is_err() {
            return;
        }
        let image = capture().expect("capture");
        let t = std::time::Instant::now();
        let l = look(&std::env::var("SUNDAY_VISION").unwrap_or_else(|_| llm::VISION.into()), &image, "What is on my screen? Point at the most important thing.").expect("look");
        println!("{:?} -> {}\n{:?}", t.elapsed(), l.explanation, sanitize(l.marks));
    }
}
