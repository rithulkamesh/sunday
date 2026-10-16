//! Screen reading and drawing. On request only: grab one screenshot, work out what's going on (which app
//! and window, the text on screen, and for diagrams a closer second look), speak an explanation, and draw
//! marks on a click-through overlay. The image never leaves the machine and its files are deleted at once.
use crate::db::Db;
use crate::llm;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const MAX_MARKS: usize = 8;
/// Longest screenshot edge sent to the model: the OCR carries the fine text, so the image can be small.
const MAX_EDGE: &str = "896";
/// A closer look at a figure gets more pixels: diagrams carry their meaning in small details.
const ZOOM_EDGE: &str = "1280";

/// A thing to point at, in 0..1000 coordinates relative to the image it was found in (top-left origin).
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
    /// For a connector arrow ("this feeds into that"): the line it ends at (-1: none)...
    #[serde(default = "no_line")]
    pub to_line: i64,
    /// ...and that line's box, filled in from the OCR so the overlay can draw the arrow between the two.
    #[serde(default, skip_deserializing)]
    pub to: Option<Region>,
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

/// A rectangle of the main display, 0..1000 each way (origin top-left). Looks at windows and figures
/// are regions; their marks are mapped back so the overlay can always draw in display coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Region {
    const WHOLE: Region = Region { x: 0.0, y: 0.0, w: 1000.0, h: 1000.0 };

    /// `r`, given relative to this region (0..1000 of it), as display coordinates.
    fn sub(&self, r: Region) -> Region {
        Region { x: self.x + r.x * self.w / 1000.0, y: self.y + r.y * self.h / 1000.0, w: r.w * self.w / 1000.0, h: r.h * self.h / 1000.0 }
    }

    fn to_display(&self, m: &Mark) -> Mark {
        let r = self.sub(Region { x: m.x, y: m.y, w: m.w, h: m.h });
        Mark { x: r.x, y: r.y, w: r.w, h: r.h, to: m.to.map(|t| self.sub(t)), ..m.clone() }
    }

    fn clamped(self) -> Region {
        let (x, y) = (self.x.clamp(0.0, 999.0), self.y.clamp(0.0, 999.0));
        Region { x, y, w: self.w.clamp(1.0, 1000.0 - x), h: self.h.clamp(1.0, 1000.0 - y) }
    }

    /// Grown by `f` of its size on every side: a model's idea of "the diagram" is usually a bit too tight.
    fn padded(self, f: f64) -> Region {
        let (px, py) = (self.w * f, self.h * f);
        Region { x: self.x - px, y: self.y - py, w: self.w + 2.0 * px, h: self.h + 2.0 * py }
    }

    /// Fraction of the display covered.
    fn area(&self) -> f64 {
        self.w * self.h / 1_000_000.0
    }
}

/// What's open on the desktop, from the window list (front to back).
#[derive(Deserialize, Clone, Debug)]
struct Win {
    app: String,
    #[serde(default)]
    category: String,
    title: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    pid: i64,
}
#[derive(Deserialize)]
struct Desk {
    display: Size,
    front: Front,
    windows: Vec<Win>,
}
#[derive(Deserialize)]
struct Size {
    w: f64,
    h: f64,
}
#[derive(Deserialize)]
struct Front {
    app: String,
    pid: i64,
}

impl Desk {
    /// The frontmost app's top window, as a region of the display.
    fn front_window(&self) -> Option<Region> {
        let w = self.windows.iter().find(|w| w.pid == self.front.pid)?;
        Some(Region { x: w.x / self.display.w * 1000.0, y: w.y / self.display.h * 1000.0, w: w.w / self.display.w * 1000.0, h: w.h / self.display.h * 1000.0 }.clamped())
    }

    /// One short paragraph for the model: what the user is in, and what else is open.
    fn context(&self) -> String {
        let title = |w: &Win| {
            let mut facts = vec![];
            if !w.category.is_empty() {
                facts.push(format!("a {} app", w.category));
            }
            if !w.title.trim().is_empty() {
                facts.push(format!("window \"{}\"", w.title.chars().take(60).collect::<String>()));
            }
            if facts.is_empty() { w.app.clone() } else { format!("{} ({})", w.app, facts.join(", ")) }
        };
        let front = self.windows.iter().find(|w| w.pid == self.front.pid).map_or_else(|| self.front.app.clone(), title);
        let others: Vec<String> = self.windows.iter().filter(|w| w.pid != self.front.pid).take(6).map(title).collect();
        let mut s = format!("Desktop context. In front: {front}.");
        if !others.is_empty() {
            s.push_str(&format!(" Also open: {}.", others.join("; ")));
        }
        s
    }
}

/// A region this full of text is a page, not a figure.
const MAX_FIGURE_LINES: usize = 30;

/// Terminal output, code and prose share one left edge; a diagram's labels are scattered.
/// True if at least half of 4+ lines start at the same x (within 2% of the width).
fn left_aligned_block(lines: &[&Line]) -> bool {
    if lines.len() < 4 {
        return false;
    }
    let mut buckets = std::collections::HashMap::new();
    for l in lines {
        *buckets.entry((l.x / 20.0) as i64).or_insert(0usize) += 1;
    }
    buckets.values().max().copied().unwrap_or(0) * 2 >= lines.len()
}

fn text_block(r: Region, lines: &[Line]) -> bool {
    let inside: Vec<&Line> = lines.iter().filter(|l| l.x + l.w / 2.0 >= r.x && l.x + l.w / 2.0 <= r.x + r.w && l.y + l.h / 2.0 >= r.y && l.y + l.h / 2.0 <= r.y + r.h).collect();
    inside.len() > MAX_FIGURE_LINES || left_aligned_block(&inside)
}

/// "this window", "this page"...: look at the front window only, not the whole display.
fn wants_window(task: &str) -> bool {
    let t = task.to_lowercase();
    ["this window", "this page", "this tab", "this document", "this pdf", "this paper", "this article", "this app", "this slide"].iter().any(|k| t.contains(k))
}

struct Seen {
    image: String,
    lines: Vec<Line>,
}

#[derive(Deserialize)]
struct Zoom {
    #[serde(default)]
    needed: bool,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    w: f64,
    #[serde(default)]
    h: f64,
}

#[derive(Deserialize)]
struct Look {
    explanation: String,
    #[serde(default)]
    marks: Vec<Mark>,
    #[serde(default)]
    zoom: Option<Zoom>,
}

const POINTING: &str = "`marks` point at the specific things you mention. kind is box (a region), circle (a small item), underline (a line of text) or arrow (a pointer to an item). \
label is at most four words and says what the marked thing is about, not where it is on screen. If nothing needs pointing at, return no marks. \
To show a connection or flow between two pieces of text (this feeds into that, A calls B), use kind arrow with `line` where it starts and `to_line` where it ends; otherwise to_line is -1. \
You may be given the screen's text as numbered lines. To point at text, set `line` to that number: the position is then exact, so set x,y,w,h to 0. \
Only for things that are not text (pictures, diagram parts, icons) set `line` to -1 and give x,y,w,h as 0 to 1000 relative to the image, origin top-left, x,y the top-left corner.";

fn system() -> String {
    format!("You see a screenshot of the user's screen and explain what they ask about, like a patient tutor sitting next to them. Speak to them as \"you\"; never begin with \"The user\". \
You are also told which app and window is in front and what else is open. That only says where they are: say what they are doing and what the content is about \
(for example: you're reading a terminal session about building a voice assistant). Never explain what an app is or does, and never guess that from its name. \
`explanation` is at most three short spoken sentences: plain words, no markdown, and refer to things by what they say or look like. \
At most four marks. Point at the content the user is working on, never at window chrome (menu bar, tabs, title bars, status bars, notifications) unless they ask. \
If the screen has a diagram, chart, graph, figure, flowchart, circuit or other small detailed picture that needs a closer look to explain properly, set zoom.needed true and give its region \
(0..1000 of the image, x,y top-left, w,h the size, slightly generous). Pages of text, code and terminals are not diagrams. Otherwise zoom.needed is false and x,y,w,h are 0. {POINTING}")
}

fn system_figure() -> String {
    format!("You see a zoomed-in figure from the user's screen: a diagram, chart, graph, circuit, flowchart, table or picture. Explain it like a patient tutor: what kind of figure it is, \
its main parts, and how they connect or what flows from what (for a chart: the axes and the trend). At most four short spoken sentences, plain words, no markdown. Speak to them as \"you\". \
At most five marks, one per part you mention. {POINTING}")
}

fn schema(zoom: bool) -> Value {
    let mut s = json!({"type": "object", "required": ["explanation", "marks"], "properties": {
        "explanation": {"type": "string"},
        "marks": {"type": "array", "items": {"type": "object", "required": ["kind", "line", "to_line", "x", "y", "w", "h", "label"], "properties": {
            "kind": {"type": "string", "enum": ["box", "circle", "underline", "arrow"]},
            "line": {"type": "integer"}, "to_line": {"type": "integer"},
            "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"},
            "label": {"type": "string"}}}}}});
    if zoom {
        s["required"] = json!(["explanation", "marks", "zoom"]);
        s["properties"]["zoom"] = json!({"type": "object", "required": ["needed", "x", "y", "w", "h"],
            "properties": {"needed": {"type": "boolean"}, "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"}}});
    }
    s
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
            // A connector needs both ends to be real text lines.
            m.to = usize::try_from(m.to_line).ok().and_then(|i| lines.get(i)).filter(|_| m.line >= 0 && m.line != m.to_line && m.kind == "arrow").map(|l| Region { x: l.x, y: l.y, w: l.w, h: l.h });
            if m.to.is_none() {
                m.to_line = -1;
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

/// scripts/build-ocr.sh puts the helper (text reader, window list, cropper) next to the models directory.
fn tool() -> Option<PathBuf> {
    let p = crate::models::dir().parent()?.join("bin/ocr");
    p.exists().then_some(p)
}

/// The app in front right now.
pub fn front_app() -> Option<String> {
    desk().map(|d| d.front.app)
}

fn desk() -> Option<Desk> {
    let out = std::process::Command::new(tool()?).arg("--windows").output().ok().filter(|o| o.status.success())?;
    serde_json::from_slice(&out.stdout).ok()
}

/// Text in an image with exact boxes. Empty (never an error) if the helper isn't built or fails:
/// pointing then falls back to the vision model's own coordinates.
fn read_text(image: &Path) -> Vec<Line> {
    let Some(tool) = tool() else {
        eprintln!("[screen] no helper tool (run scripts/build-ocr.sh): pointing will be approximate");
        return vec![];
    };
    let out = std::process::Command::new(tool).arg(image).output().ok().filter(|o| o.status.success());
    out.and_then(|o| serde_json::from_slice::<Vec<Line>>(&o.stdout).ok()).unwrap_or_default()
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sunday-{}-{name}.jpg", std::process::id()))
}

/// A full-resolution screenshot of the main display, in a temp file the caller must delete.
fn grab() -> Res<PathBuf> {
    if !screen_access() {
        return Err(llm::UserMsg("I need screen recording permission. Allow it in System Settings, Privacy and Security, Screen Recording, then restart me.".into()).into());
    }
    let full = tmp("screen");
    if !std::process::Command::new("screencapture").args(["-x", "-m", "-t", "jpg"]).arg(&full).status()?.success() {
        return Err("the screenshot failed".into());
    }
    Ok(full)
}

/// `region` of the screenshot: its text read at full resolution, and a smaller JPEG (longest edge `edge`) for the model.
/// Cropping needs the helper tool; without it the whole screenshot is used.
fn view(full: &Path, region: Region, edge: &str) -> Res<Seen> {
    let (crop, small) = (tmp("crop"), tmp("small"));
    let run = || -> Res<(Vec<u8>, Vec<Line>)> {
        let cropped = region != Region::WHOLE
            && tool().is_some_and(|t| {
                std::process::Command::new(t)
                    .arg("--crop").arg(full).arg(&crop)
                    .args([region.x, region.y, region.w, region.h].map(|v| format!("{v:.1}")))
                    .status().is_ok_and(|s| s.success())
            });
        let src = if cropped { &crop } else { full };
        let lines = read_text(src);
        // Smaller is faster for the model; falls back to the full image if resizing fails.
        let resized = std::process::Command::new("sips").args(["-Z", edge, "--out"]).arg(&small).arg(src).output().is_ok_and(|o| o.status.success());
        Ok((std::fs::read(if resized { &small } else { src })?, lines))
    };
    let res = run();
    let _ = std::fs::remove_file(&crop);
    let _ = std::fs::remove_file(&small);
    let (bytes, lines) = res?;
    if bytes.len() < 1_000 {
        return Err("the screenshot came back empty".into());
    }
    eprintln!("[screen] looked ({} KB, {} text lines), image discarded", bytes.len() / 1024, lines.len());
    Ok(Seen { image: base64::engine::general_purpose::STANDARD.encode(bytes), lines })
}

/// The task, what's open, and (when text was found) the numbered lines the model can point at.
fn prompt(task: &str, context: &str, lines: &[Line]) -> String {
    let mut p = task.to_string();
    if !context.is_empty() {
        p.push_str(&format!("\n\n{context}"));
    }
    if !lines.is_empty() {
        let list: Vec<String> = lines.iter().take(80).enumerate().map(|(i, l)| format!("[{i}] {}", l.text.chars().take(60).collect::<String>())).collect();
        p.push_str(&format!("\n\nText on the screen:\n{}", list.join("\n")));
    }
    p
}

fn look(model: &str, seen: &Seen, system: &str, zoom: bool, user: &str) -> Res<Look> {
    // The line list plus the image need more than Ollama's default 4096-token context.
    let body = json!({"model": model, "stream": false, "think": false, "keep_alive": "2m", "format": schema(zoom), "options": {"num_ctx": 8192},
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user, "images": [seen.image]}]});
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

/// The whole pipeline on one screenshot file. Marks come back in display coordinates.
/// A diagram gets a second, closer look at full resolution; `progress` says so while that runs.
fn analyse(full: &Path, context: &str, region: Region, task: &str, model: &str, progress: &dyn Fn(&str)) -> Res<(String, Vec<Mark>)> {
    let mut seen = view(full, region, MAX_EDGE)?;
    if region == Region::WHOLE {
        seen.lines = drop_chrome(std::mem::take(&mut seen.lines), task);
    }
    let l = look(model, &seen, &system(), true, &prompt(task, context, &seen.lines))?;
    let marks: Vec<Mark> = sanitize(l.marks, &seen.lines).iter().map(|m| region.to_display(m)).collect();
    let first = (l.explanation.trim().to_string(), marks);

    if let Some(z) = l.zoom.filter(|z| z.needed) {
        let zr = region.sub(Region { x: z.x, y: z.y, w: z.w, h: z.h }).padded(0.15).clamped();
        // Lines are relative to what was viewed; zoom coordinates likewise (before mapping to the display).
        let crowded = text_block(Region { x: z.x, y: z.y, w: z.w, h: z.h }, &seen.lines);
        if crowded {
            eprintln!("[screen] zoom ignored: that region is mostly text, not a figure");
        }
        if !crowded && (0.02..0.9).contains(&zr.area()) && tool().is_some() {
            progress("That looks like a diagram. Let me look closer.");
            let closer = view(full, zr, ZOOM_EDGE).and_then(|s2| {
                let user = prompt(&format!("{task} (Explain this figure in detail.)"), context, &s2.lines);
                look(model, &s2, &system_figure(), false, &user).map(|l2| (s2, l2))
            });
            match closer {
                Ok((s2, l2)) => {
                    let marks = sanitize(l2.marks, &s2.lines).iter().map(|m| zr.to_display(m)).collect();
                    return Ok((l2.explanation.trim().to_string(), marks));
                }
                Err(e) => eprintln!("[screen] closer look failed ({e}); using the first look"),
            }
        }
    }
    Ok(first)
}

/// Looks at the screen for `task`. Returns the spoken explanation and the marks to draw.
pub fn explain(app: &AppHandle, db: &Db, task: &str, progress: &dyn Fn(&str)) -> Res<(String, Vec<Mark>)> {
    clear(app); // don't photograph our own drawing
    std::thread::sleep(std::time::Duration::from_millis(250));
    let desk = desk();
    let context = desk.as_ref().map(Desk::context).unwrap_or_default();
    // "this window / this page": just the front window, without everything around it.
    let region = desk.as_ref().filter(|_| wants_window(task)).and_then(Desk::front_window).filter(|r| r.area() > 0.05).unwrap_or(Region::WHOLE);
    let full = grab()?;
    let res = analyse(&full, &context, region, task, &llm::vision_model(db), progress);
    let _ = std::fs::remove_file(&full);
    res
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
        Mark { kind: kind.into(), x, y, w, h, label: "label".repeat(20), line: -1, to_line: -1, to: None }
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
        assert!(prompt("explain", "", &lines).contains("[1] second"));
        assert_eq!(prompt("explain", "", &[]), "explain");
        assert!(prompt("explain", "Desktop context. In front: Safari.", &[]).contains("In front: Safari"));
    }

    #[test]
    fn connectors_need_two_real_lines() {
        let lines = vec![
            Line { text: "client".into(), x: 100.0, y: 100.0, w: 60.0, h: 15.0 },
            Line { text: "server".into(), x: 100.0, y: 400.0, w: 60.0, h: 15.0 },
        ];
        let arrow = |line: i64, to: i64| Mark { kind: "arrow".into(), line, to_line: to, ..m("arrow", 0.0, 0.0, 0.0, 0.0) };
        let out = sanitize(vec![arrow(0, 1), arrow(0, 0), arrow(0, 9), arrow(-1, 1)], &lines);
        assert_eq!(out[0].to, Some(Region { x: 100.0, y: 400.0, w: 60.0, h: 15.0 }));
        assert!(out[1..].iter().all(|m| m.to.is_none() && m.to_line == -1), "self-loops, missing lines and unanchored starts are plain arrows");
    }

    #[test]
    fn regions_map_marks_back_to_the_display() {
        // A window covering the right half; a mark in the middle of it is at 750 on the display.
        let win = Region { x: 500.0, y: 0.0, w: 500.0, h: 1000.0 };
        let mark = Mark { kind: "box".into(), x: 500.0, y: 250.0, w: 100.0, h: 100.0, label: String::new(), line: -1, to_line: -1, to: Some(Region { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }) };
        let d = win.to_display(&mark);
        assert_eq!((d.x, d.y, d.w, d.h), (750.0, 250.0, 50.0, 100.0));
        assert_eq!(d.to, Some(Region { x: 500.0, y: 0.0, w: 50.0, h: 100.0 })); // connector ends map too
        // A zoom given relative to that window.
        let z = win.sub(Region { x: 0.0, y: 0.0, w: 500.0, h: 500.0 });
        assert_eq!((z.x, z.w, z.h), (500.0, 250.0, 500.0));
        assert!(Region { x: 990.0, y: 990.0, w: 500.0, h: 500.0 }.clamped().w <= 10.0);
        let p = Region { x: 400.0, y: 400.0, w: 200.0, h: 100.0 }.padded(0.15);
        assert_eq!((p.x, p.y, p.w, p.h), (370.0, 385.0, 260.0, 130.0));
    }

    const DESK: &str = r#"{"display":{"w":1680,"h":1050},"front":{"app":"Preview","pid":7},"windows":[
        {"app":"Preview","category":"productivity","title":"os-notes.pdf","x":420,"y":30,"w":840,"h":1020,"pid":7},
        {"app":"Ghostty","category":"developer tools","title":"build","x":0,"y":30,"w":1680,"h":1020,"pid":3},
        {"app":"Spotify","title":"","x":0,"y":30,"w":1680,"h":1020,"pid":9}]}"#;

    #[test]
    fn text_blocks_are_not_figures() {
        let l = |x: f64, y: f64| Line { text: "t".into(), x, y, w: 40.0, h: 10.0 };
        let whole = Region::WHOLE;
        // 60 lines: a page. 5 lines on one left edge: terminal output or code.
        assert!(text_block(whole, &(0..60).map(|i| l(100.0 + (i % 7) as f64 * 90.0, i as f64 * 15.0)).collect::<Vec<_>>()));
        assert!(text_block(whole, &(0..5).map(|i| l(100.0, 100.0 + i as f64 * 20.0)).collect::<Vec<_>>()));
        // The architecture diagram's labels are scattered.
        let diagram = [l(562.0, 160.0), l(540.0, 302.0), l(463.0, 466.0), l(642.0, 467.0), l(469.0, 634.0), l(639.0, 635.0)];
        assert!(!text_block(whole, &diagram));
        assert!(!text_block(whole, &diagram[..3]), "too few lines to call it text");
    }

    #[test]
    fn knows_what_is_in_front() {
        let d: Desk = serde_json::from_str(DESK).unwrap();
        let r = d.front_window().unwrap();
        assert_eq!((r.x, r.w), (250.0, 500.0)); // 420/1680, 840/1680
        let c = d.context();
        assert!(c.contains("In front: Preview (a productivity app, window \"os-notes.pdf\")") && c.contains("Ghostty (a developer tools app, window \"build\")") && c.contains("Spotify"), "{c}");
        assert!(!c.contains("Spotify ("), "no facts, no parentheses: {c}");
        assert!(wants_window("explain this page") && wants_window("what is on this PDF") && !wants_window("what is on my screen"));
    }

    /// Needs the vision model and screen-recording permission; run by hand with SUNDAY_SCREEN_TEST=1.
    /// With SUNDAY_TEST_IMAGE=<file> it analyses that picture instead of the screen (e.g. a diagram).
    #[test]
    fn looks_at_the_real_screen() {
        if std::env::var("SUNDAY_SCREEN_TEST").is_err() {
            return;
        }
        let model = std::env::var("SUNDAY_VISION").unwrap_or_else(|_| llm::VISION.into());
        let task = std::env::var("SUNDAY_TEST_TASK").unwrap_or_else(|_| "What is on my screen? Point at the most important thing.".into());
        let t = std::time::Instant::now();
        let (text, marks) = match std::env::var("SUNDAY_TEST_IMAGE") {
            Ok(img) => analyse(Path::new(&img), "", Region::WHOLE, &task, &model, &|m| println!("[progress] {m}")).expect("analyse"),
            Err(_) => {
                let d = desk();
                let ctx = d.as_ref().map(Desk::context).unwrap_or_default();
                let full = grab().expect("grab");
                let r = analyse(&full, &ctx, Region::WHOLE, &task, &model, &|m| println!("[progress] {m}"));
                let _ = std::fs::remove_file(&full);
                r.expect("analyse")
            }
        };
        println!("{:?} -> {text}\n{marks:?}", t.elapsed());
    }
}
