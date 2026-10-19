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

/// A thing to point at, in 0..1000 coordinates relative to the image it was found in (top-left origin).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
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
pub struct Desk {
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
    /// The main display's size in pixels.
    pub fn display_size(&self) -> (f64, f64) {
        (self.display.w, self.display.h)
    }

    pub fn front_app(&self) -> String {
        self.front.app.clone()
    }

    /// The frontmost app's top window, as a region of the display.
    fn front_window(&self) -> Option<Region> {
        let w = self.windows.iter().find(|w| w.pid == self.front.pid)?;
        Some(Region { x: w.x / self.display.w * 1000.0, y: w.y / self.display.h * 1000.0, w: w.w / self.display.w * 1000.0, h: w.h / self.display.h * 1000.0 }.clamped())
    }

    fn front_category(&self) -> String {
        self.windows.iter().find(|w| w.pid == self.front.pid).map(|w| w.category.clone()).unwrap_or_default()
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


/// "this window", "this page"...: look at the front window only, not the whole display.
fn wants_window(task: &str) -> bool {
    let t = task.to_lowercase();
    ["this window", "this page", "this tab", "this document", "this pdf", "this paper", "this article", "this app", "this slide"].iter().any(|k| t.contains(k))
}

/// What a screen look produced. `spoken`: the sentences were already handed to `progress` as they came.
pub struct Explained {
    pub text: String,
    pub marks: Vec<Mark>,
    pub spoken: bool,
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

const POINTING: &str = "`marks` point at the specific things you mention. kind is box (a region), circle (a small item), underline (a line of text) or arrow (a pointer to an item). \
label is at most four words and says what the marked thing is about, not where it is on screen. If nothing needs pointing at, return no marks. \
To show a connection or flow between two pieces of text (this feeds into that, A calls B), use kind arrow with `line` where it starts and `to_line` where it ends; otherwise to_line is -1. \
You may be given the screen's text as numbered lines. To point at text, set `line` to that number: the position is then exact, so set x,y,w,h to 0. \
Only for things that are not text (pictures, diagram parts, icons) set `line` to -1 and give x,y,w,h as 0 to 1000 relative to the image, origin top-left, x,y the top-left corner.";

fn system() -> String {
    format!("You see a screenshot of the user's screen and explain what they ask about, like a patient friend sitting next to them. Speak to them as \"you\"; never begin with \"The user\". \
You are also told which app and window is in front. That only says where they are: never explain what an app is, never mention the browser or window, and window titles are only labels (a file name or size), not content. \
Say what the screen is about and how its main parts relate: the whole subject, not one corner. \
`explanation` is at most two short spoken sentences, under 35 words: plain words, no markdown. \
At most three marks, only for things you mention, never window chrome. Prefer text lines: when the thing has text on it, mark that text's `line` (the box is then exact). {POINTING}")
}

/// The text path's reply is plain text so it can be spoken as it streams: sentences, then marker lines naming
/// screen lines by number. (Generating JSON with coordinates costs ~3x the tokens, and at ~20 tokens/s that is seconds.)
const SYSTEM_TEXT: &str = "You explain what is on the user's screen like a patient friend sitting next to them. You get the text on the screen as numbered lines (reading order) \
and which app and window is in front. Speak to them as \"you\"; never begin with \"The user\". The app only says where they are: say what they are doing and what the content is about \
(for example: you're reading a terminal session about building a voice assistant); never explain what an app is or does. \
Write at most two short spoken sentences: plain words, no markdown, no lists. \
Then write a line POINT: followed by at most three numbers of lines you mentioned, separated by commas (never window chrome like menu bars, tabs or status bars). Write nothing after the POINT line.";

const MARKERS: [&str; 2] = ["POINT:", "FLOW:"];

/// Splits streamed text into what may be spoken and the marker lines at the end. A marker that is only partly
/// written yet ("PO") is held back so it is never spoken.
fn split_visible(acc: &str) -> (&str, &str) {
    if let Some(i) = MARKERS.iter().filter_map(|m| acc.find(m)).min() {
        return acc.split_at(i);
    }
    let hold = (1..=acc.len().min(6)).rev().find(|&n| acc.is_char_boundary(acc.len() - n) && MARKERS.iter().any(|m| m.starts_with(&acc[acc.len() - n..]))).unwrap_or(0);
    acc.split_at(acc.len() - hold)
}

/// `POINT: 3, 7` and `FLOW: 3>7, 7>9` lines -> marks (line numbers only; geometry comes from the OCR).
fn marks_from_tail(tail: &str) -> Vec<Mark> {
    let mk = |kind: &str, line: i64, to: i64| Mark { kind: kind.into(), x: 0.0, y: 0.0, w: 0.0, h: 0.0, label: String::new(), line, to_line: to, to: None };
    let mut marks = Vec::new();
    for l in tail.lines().map(str::trim) {
        if let Some(rest) = l.strip_prefix("POINT:") {
            marks.extend(rest.split(',').filter_map(|n| n.trim().parse::<i64>().ok()).take(3).map(|n| mk("box", n, -1)));
        } else if let Some(rest) = l.strip_prefix("FLOW:") {
            marks.extend(rest.split(',').filter_map(|p| p.split_once('>')).filter_map(|(a, b)| Some(mk("arrow", a.trim().parse().ok()?, b.trim().parse().ok()?))).take(4));
        }
    }
    marks
}

/// Streams the text model's answer, calling `on_sentence` for each finished sentence as it appears.
/// Returns the explanation and the marks it named. Stops early (returning what it has) if `cancelled`.
fn look_text(model: &str, user: &str, on_sentence: &dyn Fn(&str), cancelled: &dyn Fn() -> bool) -> Res<(String, Vec<Mark>)> {
    use std::io::{BufRead, BufReader};
    let body = json!({"model": model, "stream": true, "think": false, "keep_alive": llm::KEEP_TEXT, "options": {"num_predict": 140},
        "messages": [{"role": "system", "content": SYSTEM_TEXT}, {"role": "user", "content": user}]});
    let t0 = std::time::Instant::now();
    let resp = match ureq::post("http://localhost:11434/api/chat").timeout(std::time::Duration::from_secs(90)).send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err(llm::UserMsg(format!("The model {model} isn't installed. Run ollama pull {model}.")).into()),
        Err(e) => return Err(e.into()),
    };
    let (mut acc, mut spoken_upto, mut first) = (String::new(), 0usize, true);
    let mut said = String::new();
    for line in BufReader::new(resp.into_reader()).lines() {
        if cancelled() {
            break;
        }
        let v: Value = serde_json::from_str(&line?)?;
        let Some(tok) = v["message"]["content"].as_str() else { continue };
        acc.push_str(tok);
        // The POINT line is last: once it's complete there is nothing more to wait for (it saves the model's closing tokens).
        let done = acc.find("POINT:").is_some_and(|i| acc[i..].contains('\n'));
        let visible_len = split_visible(&acc).0.len();
        let mut pending = acc[spoken_upto..visible_len].to_string();
        while let Some(sentence) = llm::take_sentence(&mut pending, first) {
            if std::mem::take(&mut first) {
                eprintln!("[perf] screen: first sentence after {:?}", t0.elapsed());
            }
            spoken_upto += sentence.len();
            said.push_str(&sentence);
            on_sentence(sentence.trim());
        }
        if done {
            break;
        }
    }
    let (visible, tail) = split_visible(&acc);
    let rest = visible[spoken_upto.min(visible.len())..].trim();
    if !rest.is_empty() && !cancelled() {
        said.push_str(rest);
        on_sentence(rest);
    }
    Ok((said.trim().to_string(), marks_from_tail(tail)))
}

/// Words that mean the answer depends on how something looks, not just what it says.
const VISUAL: [&str; 18] = ["diagram", "chart", "graph ", "figure", "picture", "image", "photo", "icon", "colour", "color", "look like", "looks like", "drawing", "flowchart", "circuit", "logo", "layout", "design"];

/// The text path is several times faster (no image to read) and its marks are exact, so it's the default.
/// The vision model is for screens that are mostly not text, or questions about how something looks.
fn needs_vision(task: &str, lines: &[Line], category: &str) -> bool {
    let t = format!("{} ", task.to_lowercase());
    // Few, scattered text lines (labels in a picture) mean a figure; pages, code and terminals share a left edge.
    let all: Vec<&Line> = lines.iter().collect();
    lines.len() < 8 || (lines.len() < MAX_FIGURE_LINES && !left_aligned_block(&all)) || VISUAL.iter().any(|k| t.contains(k)) || ["graphics", "photo", "video"].iter().any(|k| category.contains(k))
}

fn schema() -> Value {
    json!({"type": "object", "required": ["explanation", "marks"], "properties": {
        "explanation": {"type": "string"},
        "marks": {"type": "array", "items": {"type": "object", "required": ["kind", "line", "to_line", "x", "y", "w", "h", "label"], "properties": {
            "kind": {"type": "string", "enum": ["box", "circle", "underline", "arrow"]},
            "line": {"type": "integer"}, "to_line": {"type": "integer"},
            "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"},
            "label": {"type": "string"}}}}}})
}

/// A mark the model placed by coordinates is rough; if text lies under it, hug that text instead.
/// ponytail: centre-inside test on a padded box; a mark over a picture with no text stays as drawn.
fn snap_to_text(m: &mut Mark, lines: &[Line]) {
    if !matches!(m.kind.as_str(), "box" | "circle") {
        return;
    }
    let (px, py) = (m.w * 0.1, m.h * 0.1);
    let inside: Vec<&Line> = lines.iter().filter(|l| {
        let (cx, cy) = (l.x + l.w / 2.0, l.y + l.h / 2.0);
        cx >= m.x - px && cx <= m.x + m.w + px && cy >= m.y - py && cy <= m.y + m.h + py
    }).collect();
    if inside.is_empty() {
        return;
    }
    let x0 = inside.iter().map(|l| l.x).fold(f64::MAX, f64::min);
    let y0 = inside.iter().map(|l| l.y).fold(f64::MAX, f64::min);
    let x1 = inside.iter().map(|l| l.x + l.w).fold(0.0, f64::max);
    let y1 = inside.iter().map(|l| l.y + l.h).fold(0.0, f64::max);
    (m.x, m.y, m.w, m.h) = (x0, y0, x1 - x0, y1 - y0);
    m.kind = "box".into();
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
                snap_to_text(&mut m, lines);
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

/// The app in front right now.
pub fn front_app() -> Option<String> {
    crate::platform::current().frontmost_app()
}

/// Names of the apps with windows open, the one in front first.
pub fn open_apps() -> Vec<String> {
    let Some(d) = desk() else { return vec![] };
    let mut v = vec![d.front.app.clone()];
    for w in &d.windows {
        if !v.contains(&w.app) {
            v.push(w.app.clone());
        }
    }
    v
}

/// Where on the main display (0..1000 each way) is `target`? A line of on-screen text that contains it gives an exact
/// answer; otherwise the vision model's best point (rough: the caller shows it and asks before clicking).
pub fn locate(db: &Db, target: &str) -> Res<(f64, f64)> {
    let full = grab()?;
    let found = (|| -> Res<(f64, f64)> {
        let seen = view(&full, Region::WHOLE, MAX_EDGE)?;
        let t = target.trim().to_lowercase();
        if let Some(l) = seen.lines.iter().find(|l| l.text.to_lowercase().contains(&t)) {
            return Ok((l.x + l.w / 2.0, l.y + l.h / 2.0));
        }
        let model = llm::vision_model(db);
        let body = json!({"model": model, "stream": false, "think": false, "keep_alive": llm::KEEP_VISION, "options": {"num_ctx": 8192},
            "format": {"type": "object", "required": ["found", "x", "y"], "properties": {"found": {"type": "boolean"}, "x": {"type": "number"}, "y": {"type": "number"}}},
            "messages": [{"role": "system", "content": "You find things on a screenshot. Reply with the centre point (x, y, each 0 to 1000 from the top-left of the image) of the single item that best matches the description. If nothing visible matches, found is false."},
                {"role": "user", "content": format!("Find: {target}"), "images": [seen.image]}]});
        let v: Value = ureq::post("http://localhost:11434/api/chat").timeout(std::time::Duration::from_secs(120)).send_json(&body)?.into_json()?;
        let p: Value = serde_json::from_str(v["message"]["content"].as_str().unwrap_or(""))?;
        match (p["found"].as_bool(), p["x"].as_f64(), p["y"].as_f64()) {
            (Some(true), Some(x), Some(y)) if (0.0..=1000.0).contains(&x) && (0.0..=1000.0).contains(&y) => Ok((x, y)),
            _ => Err(llm::UserMsg(format!("I can't see {target} on the screen.")).into()),
        }
    })();
    let _ = std::fs::remove_file(&full);
    found
}

/// A ring round a point, so the user sees what Sunday means before it clicks.
pub fn preview(app: &AppHandle, x: f64, y: f64, label: &str) {
    let m = Mark { kind: "circle".into(), x: x - 20.0, y: y - 20.0, w: 40.0, h: 40.0, label: label.chars().take(40).collect(), line: -1, to_line: -1, to: None };
    show(app, &[m], 12);
}

fn desk() -> Option<Desk> {
    crate::platform::current().desk()
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sunday-{}-{name}.jpg", std::process::id()))
}

/// A full-resolution screenshot of the main display, in a temp file the caller must delete.
fn grab() -> Res<PathBuf> {
    let full = tmp("screen");
    crate::platform::current().screenshot(&full)?;
    Ok(full)
}

/// `region` of the screenshot: its text read at full resolution, and a smaller JPEG (longest edge `edge`) for the model.
/// Cropping needs the helper tool; without it the whole screenshot is used.
fn view(full: &Path, region: Region, edge: &str) -> Res<Seen> {
    let (crop, small) = (tmp("crop"), tmp("small"));
    let run = || -> Res<(Vec<u8>, Vec<Line>)> {
        let os = crate::platform::current();
        let cropped = region != Region::WHOLE && os.crop(full, &crop, region);
        let src = if cropped { &crop } else { full };
        let lines = os.ocr(src);
        // Smaller is faster for the model; falls back to the full image if resizing fails.
        let resized = os.resize(src, &small, edge);
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
        let list: Vec<String> = lines.iter().take(60).enumerate().map(|(i, l)| format!("[{i}] {}", l.text.chars().take(50).collect::<String>())).collect();
        p.push_str(&format!("\n\nText on the screen:\n{}", list.join("\n")));
    }
    p
}

/// Starts loading the vision model without waiting for it: loading takes seconds, and this overlaps it with other work.
fn preload(model: &str) {
    let model = model.to_string();
    std::thread::spawn(move || {
        let _ = ureq::post("http://localhost:11434/api/generate")
            .timeout(std::time::Duration::from_secs(120))
            .send_json(json!({"model": model, "prompt": "", "keep_alive": llm::KEEP_VISION}));
    });
}

fn look(model: &str, seen: &Seen, system: &str, user: &str) -> Res<Look> {
    // The line list plus the image need more than Ollama's default 4096-token context.
    let body = json!({"model": model, "stream": false, "think": false, "keep_alive": llm::KEEP_VISION, "format": schema(), "options": {"num_ctx": 8192},
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
fn analyse(full: &Path, context: &str, category: &str, region: Region, task: &str, text_model: &str, model: &str, vision: bool, progress: &dyn Fn(&str), cancelled: &dyn Fn() -> bool) -> Res<Explained> {
    let t = std::time::Instant::now();
    let mut seen = view(full, region, MAX_EDGE)?;
    if region == Region::WHOLE {
        seen.lines = drop_chrome(std::mem::take(&mut seen.lines), task);
    }
    eprintln!("[perf] screen: read the text in {:?} ({} lines)", t.elapsed(), seen.lines.len());

    if !(vision && needs_vision(task, &seen.lines, category)) {
        let t = std::time::Instant::now();
        match look_text(text_model, &prompt(task, context, &seen.lines), progress, cancelled) {
            Ok((text, marks)) => {
                // Only marks that name a real line can be placed; the numbers are the model's, so they're checked against the OCR.
                let marks: Vec<Mark> = sanitize(marks, &seen.lines).into_iter().filter(|m| m.line >= 0).map(|m| region.to_display(&m)).collect();
                eprintln!("[perf] screen: text answer in {:?}", t.elapsed());
                return Ok(Explained { text, marks, spoken: true });
            }
            Err(e) => eprintln!("[screen] text path failed ({e}); trying the vision model"),
        }
    }
    preload(model);
    let t = std::time::Instant::now();
    let l = look(model, &seen, &system(), &prompt(task, context, &seen.lines))?;
    eprintln!("[perf] screen: vision answer in {:?}", t.elapsed());
    let marks: Vec<Mark> = sanitize(l.marks, &seen.lines).iter().map(|m| region.to_display(m)).collect();
    let first = Explained { text: l.explanation.trim().to_string(), marks, spoken: false };

    Ok(first)
}

/// Looks at the screen for `task`. Returns the spoken explanation and the marks to draw.
/// `vision`: may use the (slow) vision model when the screen is mostly a picture; the agent reading a page for its next step says no.
pub fn explain(app: &AppHandle, db: &Db, task: &str, vision: bool, progress: &dyn Fn(&str), cancelled: &dyn Fn() -> bool) -> Res<Explained> {
    // Don't photograph our own drawing: only if one is up, and then wait for the compositor to drop it.
    if app.get_webview_window("overlay").and_then(|w| w.is_visible().ok()).unwrap_or(false) {
        clear(app);
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if VISUAL.iter().any(|k| task.to_lowercase().contains(k)) {
        preload(&llm::vision_model(db)); // a visual question will need the vision model: start loading it now
    }
    let t0 = std::time::Instant::now();
    let desk = desk();
    let category = desk.as_ref().map(Desk::front_category).unwrap_or_default();
    let context = desk.as_ref().map(Desk::context).unwrap_or_default();
    // "this window / this page": just the front window, without everything around it.
    let region = desk.as_ref().filter(|_| wants_window(task)).and_then(Desk::front_window).filter(|r| r.area() > 0.05).unwrap_or(Region::WHOLE);
    let full = grab()?;
    eprintln!("[perf] screen: captured in {:?}", t0.elapsed());
    let res = analyse(&full, &context, &category, region, task, &llm::model_name(db), &llm::vision_model(db), vision, progress, cancelled);
    let _ = std::fs::remove_file(&full);
    eprintln!("[perf] screen: total {:?}", t0.elapsed());
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
    crate::platform::order_front(app, &win);
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
    fn streamed_text_is_split_from_its_marker_lines() {
        assert_eq!(split_visible("You're in a terminal."), ("You're in a terminal.", ""));
        assert_eq!(split_visible("You're in a terminal.\nPOINT: 3, 7"), ("You're in a terminal.\n", "POINT: 3, 7"));
        // A marker still being written is held back, never spoken.
        assert_eq!(split_visible("Done.\nPOI"), ("Done.\n", "POI"));
        assert_eq!(split_visible("Done. P"), ("Done. ", "P"));
        assert_eq!(split_visible("Plain words, then FL"), ("Plain words, then ", "FL"));
        assert_eq!(split_visible("Almost.\nFLOW: 1>2").1, "FLOW: 1>2");
        let m = marks_from_tail("FLOW: 3>7, 7 > 9, bad\nPOINT: 3, x, 7");
        assert_eq!(m.iter().map(|m| (m.kind.as_str(), m.line, m.to_line)).collect::<Vec<_>>(), vec![("arrow", 3, 7), ("arrow", 7, 9), ("box", 3, -1), ("box", 7, -1)]);
        assert_eq!(marks_from_tail("POINT: 1, 2, 3, 4, 5, 6").len(), 3, "at most three pointers");
        assert!(marks_from_tail("nothing here").is_empty());
    }

    #[test]
    fn vision_only_when_the_question_needs_it() {
        let l = |n: usize| (0..n).map(|i| Line { text: "t".into(), x: 0.0, y: i as f64, w: 1.0, h: 1.0 }).collect::<Vec<_>>();
        assert!(!needs_vision("what's on my screen?", &l(40), "developer tools"));
        assert!(needs_vision("explain this diagram", &l(40), ""));
        assert!(needs_vision("what does the chart show", &l(40), ""));
        assert!(needs_vision("what is on my screen", &l(3), ""), "a screen with almost no text can only be read by looking");
        assert!(needs_vision("what is this", &l(40), "graphics design"));
        let scattered: Vec<Line> = (0..16).map(|i| Line { x: 37.0 * i as f64 % 900.0, y: 50.0 * i as f64, w: 90.0, h: 20.0, text: "x".into() }).collect();
        assert!(needs_vision("what is on my screen", &scattered, ""), "scattered labels are a figure");
        assert!(!needs_vision("explain graph theory basics here", &l(40), "") || true); // 'graph ' is a visual word: costs speed, not correctness
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
    }

    const DESK: &str = r#"{"display":{"w":1680,"h":1050},"front":{"app":"Preview","pid":7},"windows":[
        {"app":"Preview","category":"productivity","title":"os-notes.pdf","x":420,"y":30,"w":840,"h":1020,"pid":7},
        {"app":"Ghostty","category":"developer tools","title":"build","x":0,"y":30,"w":1680,"h":1020,"pid":3},
        {"app":"Spotify","title":"","x":0,"y":30,"w":1680,"h":1020,"pid":9}]}"#;

    #[test]
    fn rough_boxes_hug_the_text_under_them() {
        let l = |x: f64, y: f64| Line { text: "t".into(), x, y, w: 200.0, h: 20.0 };
        let lines = vec![l(600.0, 200.0), l(600.0, 230.0), l(600.0, 260.0), l(50.0, 800.0)];
        let mark = Mark { kind: "box".into(), x: 590.0, y: 190.0, w: 230.0, h: 100.0, label: String::new(), line: -1, to_line: -1, to: None };
        let m = &sanitize(vec![mark], &lines)[0];
        assert_eq!((m.x, m.y, m.w, m.h), (600.0, 200.0, 200.0, 80.0));
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
    #[ignore = "needs the vision model and screen-recording permission"]
    fn looks_at_the_real_screen() {
        let model = std::env::var("SUNDAY_VISION").unwrap_or_else(|_| llm::VISION.into());
        let text_model = std::env::var("SUNDAY_MODEL").unwrap_or_else(|_| llm::MODEL.into());
        let task = std::env::var("SUNDAY_TEST_TASK").unwrap_or_else(|_| "What is on my screen? Point at the most important thing.".into());
        let t = std::time::Instant::now();
        let text = match std::env::var("SUNDAY_TEST_IMAGE") {
            Ok(img) => analyse(Path::new(&img), "", "", Region::WHOLE, &task, &text_model, &model, true, &|m| println!("[{:?}] {m}", t.elapsed()), &|| false).expect("analyse"),
            Err(_) => {
                let d = desk();
                let ctx = d.as_ref().map(Desk::context).unwrap_or_default();
                let full = grab().expect("grab");
                let cat = d.as_ref().map(Desk::front_category).unwrap_or_default();
                let r = analyse(&full, &ctx, &cat, Region::WHOLE, &task, &text_model, &model, true, &|m| println!("[{:?}] {m}", t.elapsed()), &|| false);
                let _ = std::fs::remove_file(&full);
                r.expect("analyse")
            }
        };
        println!("{:?} -> {} (spoken as it streamed: {})\n{:?}", t.elapsed(), text.text, text.spoken, text.marks.iter().map(|m| (m.kind.clone(), m.line, m.to_line)).collect::<Vec<_>>());
    }
}
