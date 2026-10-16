//! Sunday: hold fn or say "sunday" -> HUD pill at the bottom of the screen.
//! A normal dock app: closing the main window hides it, only Quit (dock/tray/cmd+q) exits.

mod bargein;
mod mic;
#[cfg(feature = "voice")]
mod db;
#[cfg(feature = "voice")]
mod decision;
#[cfg(feature = "voice")]
mod screen;
#[cfg(feature = "voice")]
mod skills;
#[cfg(feature = "voice")]
mod llm;
#[cfg(feature = "voice")]
mod notes;
#[cfg(feature = "voice")]
mod models;
#[cfg(feature = "voice")]
mod study;
#[cfg(feature = "voice")]
mod stt;
#[cfg(feature = "voice")]
mod tts;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, WebviewWindow, WindowEvent};

const WAKE: &str = "sunday";
const EXIT_ANIM: Duration = Duration::from_millis(220); // keep in sync with Hud.tsx
const WAKE_HIDE_AFTER: Duration = Duration::from_secs(10); // matches stt::IDLE: talk on without the wake word
const BOTTOM_MARGIN: f64 = 16.0; // logical px (~1em) above the bottom of the usable screen

/// Shows a window without activating the app (activation drags the main window forward too).
/// AppKit calls must run on the main thread, or the process aborts.
pub(crate) fn order_front(app: &AppHandle, win: &WebviewWindow) {
    let win = win.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        if let Ok(ns) = win.ns_window() {
            // SAFETY: ns_window is a live NSWindow owned by this webview window.
            unsafe {
                let w = ns as *mut objc2::runtime::AnyObject;
                let _: () = objc2::msg_send![w, orderFrontRegardless];
            }
            return;
        }
        let _ = win.show();
    });
}

/// Never takes clicks or focus, and floats over every Space (incl. fullscreen apps), so showing it
/// doesn't switch Spaces or raise other windows. Call from the main thread (setup).
fn float_everywhere(win: &WebviewWindow) {
    let _ = win.set_ignore_cursor_events(true);
    #[cfg(target_os = "macos")]
    if let Ok(ns) = win.ns_window() {
        const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
        const FULLSCREEN_AUXILIARY: usize = 1 << 8;
        // SAFETY: ns is the window's live NSWindow, and this runs on the main thread.
        unsafe {
            let w = ns as *mut objc2::runtime::AnyObject;
            let _: () = objc2::msg_send![w, setCollectionBehavior: CAN_JOIN_ALL_SPACES | FULLSCREEN_AUXILIARY];
        }
    }
}

/// The HUD pill. `wake_until` keeps it up after the wake word; holding fn keeps it up too.
#[derive(Clone)]
struct Hud {
    app: AppHandle,
    win: WebviewWindow,
    wake_until: Arc<Mutex<Option<Instant>>>,
    /// Bumped on every show/hide so a late hide can tell it was superseded.
    epoch: Arc<AtomicU64>,
}

impl Hud {
    fn show(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if let (Ok(Some(m)), Ok(size)) = (self.app.primary_monitor(), self.win.outer_size()) {
            // work_area excludes the dock and menu bar.
            let wa = m.work_area();
            let (mp, ms) = (wa.position, wa.size);
            let margin = (BOTTOM_MARGIN * m.scale_factor()) as i32;
            let x = mp.x + (ms.width as i32 - size.width as i32) / 2;
            let y = mp.y + ms.height as i32 - size.height as i32 - margin;
            let _ = self.win.set_position(PhysicalPosition::new(x, y));
        }
        self.show_without_activating();
        let _ = self.app.emit("state", "listening");
    }

    /// `show()` activates the app, which drags the main window to the front too.
    /// AppKit calls must run on the main thread, or the process aborts.
    fn show_without_activating(&self) {
        order_front(&self.app, &self.win);
    }

    fn hide(&self) {
        *self.wake_until.lock().unwrap() = None;
        // Tell the page to animate out, then actually hide once that's done.
        let _ = self.app.emit("state", "idle");
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let (win, cur) = (self.win.clone(), self.epoch.clone());
        std::thread::spawn(move || {
            std::thread::sleep(EXIT_ANIM);
            if cur.load(Ordering::SeqCst) == epoch {
                let _ = win.hide();
            }
        });
    }

    fn wake(&self) {
        *self.wake_until.lock().unwrap() = Some(Instant::now() + WAKE_HIDE_AFTER);
        self.show();
    }

    fn woken(&self) -> bool {
        self.wake_until.lock().unwrap().is_some()
    }

    /// Extend the wake window while the conversation continues.
    fn touch(&self) {
        let mut w = self.wake_until.lock().unwrap();
        if w.is_some() {
            *w = Some(Instant::now() + WAKE_HIDE_AFTER);
        }
    }
}

/// fn state via CGEventSourceFlagsState: no event tap, so no Input Monitoring prompt and no
/// key-translation crash (rdev's listener aborts the process on recent macOS).
#[cfg(target_os = "macos")]
fn fn_down() -> bool {
    extern "C" {
        fn CGEventSourceFlagsState(state: i32) -> u64;
    }
    const COMBINED_SESSION: i32 = 0;
    const FN_FLAG: u64 = 1 << 23;
    // ponytail: arrow/F-keys also set this flag; fine for a hold-to-talk key.
    unsafe { CGEventSourceFlagsState(COMBINED_SESSION) & FN_FLAG != 0 }
}
#[cfg(not(target_os = "macos"))]
fn fn_down() -> bool {
    false
}

fn watch_fn(hud: Hud) {
    std::thread::spawn(move || {
        let mut held = false;
        loop {
            std::thread::sleep(Duration::from_millis(30));
            let now = fn_down();
            if now != held {
                held = now;
                if held { hud.show() } else if !hud.woken() { hud.hide() }
            }
            let expired = hud.wake_until.lock().unwrap().is_some_and(|t| Instant::now() > t);
            if expired && !held {
                hud.hide();
            }
        }
    });
}

/// What a piece of speech means for the HUD: the wake word shows it, later speech is shown.
/// Emits "transcript" (live, for the HUD) and, for finished utterances, "utterance" (the log).
fn on_speech(hud: &Hud, text: &str, is_final: bool) -> Option<String> {
    let lower = text.to_lowercase();
    let shown = if let Some(i) = lower.find(WAKE) {
        if !hud.woken() {
            hud.wake();
        }
        text[i + WAKE.len()..].trim_start_matches([',', '.', ' ', '!', '?'])
    } else if hud.woken() || fn_down() {
        text
    } else {
        if is_final {
            eprintln!("[stt] ignored, not awake: {text:?}");
        }
        return None;
    };
    hud.touch();
    let _ = hud.app.emit("transcript", shown);
    if is_final && !shown.is_empty() {
        let _ = hud.app.emit("utterance", shown);
        return Some(shown.to_string());
    }
    None
}

fn listen(hud: Hud) {
    std::thread::spawn(move || {
        let app = hud.app.clone();
        let Ok((_stream, rx)) = mic::open() else { return eprintln!("mic unavailable") };
        #[cfg(feature = "voice")]
        let mut stt = stt::Stt::new();
        #[cfg(feature = "voice")]
        let (mut barge, mut mic_level) = (bargein::BargeIn::default(), 0.0f32);
        #[cfg(feature = "voice")]
        let reply = app.state::<Reply>().0.lock().unwrap().clone();
        for chunk in rx {
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len().max(1) as f32).sqrt();
            let _ = app.emit("level", rms * 8.0);
            #[cfg(feature = "voice")]
            if let Some(s) = stt.as_mut() {
                let fn_held = fn_down();
                let mut muted = tts::speaking(); // our own voice: only the keyword spotter listens
                if muted && fn_held {
                    tts::interrupt(); // holding fn barges in
                    muted = false;
                }
                if muted {
                    // Talking over Sunday: mic well above the learned echo of our own playback.
                    let ms = chunk.len() as f32 / 16.0;
                    mic_level += 0.5 * (rms - mic_level);
                    if barge.over(mic_level, tts::playback_level(), ms, s.voiced()) {
                        eprintln!("[stt] barge-in (voice) echo ratio {:.2}", barge.ratio());
                        tts::interrupt();
                        hud.wake();
                        muted = false;
                    }
                } else {
                    barge.reset();
                }
                if fn_held || muted || tts::busy() {
                    s.warm(); // hold-to-talk, or mid-conversation: keep the model loaded
                }
                for heard in s.push(&chunk, muted) {
                    match heard {
                        stt::Heard::Interim(t) => {
                            if !tts::is_echo(&t) {
                                drop(on_speech(&hud, &t, false));
                            }
                        }
                        stt::Heard::Final(t) => {
                            eprintln!("[stt] {t:?}");
                            if tts::is_echo(&t) {
                                eprintln!("[stt] ignored: our own voice coming back");
                                continue;
                            }
                            if hud.woken() && stt::is_goodbye(&t) {
                                tts::interrupt();
                                hud.hide();
                                s.cool();
                                continue;
                            }
                            if let Some(q) = on_speech(&hud, &t, true) {
                                tts::interrupt(); // a new question cancels a reply still in flight
                                let _ = reply.send(llm::Turn { text: q, voice: true });
                            }
                        }
                        stt::Heard::Keyword(k) => {
                            eprintln!("[stt] keyword {k:?} muted={muted}");
                            if muted {
                                tts::interrupt();
                            }
                            if k == "sunday" {
                                hud.wake(); // show the HUD now, before the model has loaded
                            }
                        }
                    }
                }
            }
        }
    });
}

#[cfg(feature = "voice")]
struct Reply(Mutex<std::sync::mpsc::Sender<llm::Turn>>);

/// Typed input from the workspace takes the same path as speech.
#[cfg(feature = "voice")]
#[tauri::command]
fn ask(text: String, voice: bool, reply: tauri::State<Reply>) {
    let _ = reply.0.lock().unwrap().send(llm::Turn { text, voice });
}

#[cfg(feature = "voice")]
#[tauri::command]
fn study_state(db: tauri::State<Arc<db::Db>>) -> db::State {
    db.state()
}

/// One status line per file: what was indexed, or why not.
#[cfg(feature = "voice")]
#[tauri::command(async)]
fn import_notes(paths: Vec<String>, db: tauri::State<Arc<db::Db>>) -> Vec<String> {
    paths
        .iter()
        .map(|p| {
            let path = std::path::Path::new(p);
            let name = path.file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned());
            match notes::extract(path) {
                Err(e) => format!("{name}: {e}"),
                Ok((title, text)) => match db.add_document(&title, p, &notes::hash(&text), &notes::chunk(&text)) {
                    Some(n) => format!("{name}: indexed {n} passages"),
                    None => format!("{name}: already imported"),
                },
            }
        })
        .collect()
}

#[cfg(feature = "voice")]
#[tauri::command]
fn notes(db: tauri::State<Arc<db::Db>>) -> Vec<db::Doc> {
    db.docs()
}

#[cfg(feature = "voice")]
#[tauri::command]
fn remove_note(id: i64, db: tauri::State<Arc<db::Db>>) {
    db.remove_document(id);
}

#[cfg(feature = "voice")]
#[tauri::command]
fn skills(db: tauri::State<Arc<db::Db>>) -> Vec<skills::Manifest> {
    db.skills()
}

#[cfg(feature = "voice")]
#[tauri::command(async)]
fn skill_run(id: String, version: Option<i64>, input: String, db: tauri::State<Arc<db::Db>>) -> Result<serde_json::Value, String> {
    llm::run_skill(&db, &llm::model_name(&db), &id, version, &input).map(|(v, _)| v).map_err(|e| e.to_string())
}

/// Runs every test of one version; the result is remembered, and approval needs it to be all-pass.
#[cfg(feature = "voice")]
#[tauri::command(async)]
fn skill_test(id: String, version: i64, db: tauri::State<Arc<db::Db>>) -> Result<Vec<skills::TestResult>, String> {
    let m = db.skill(&id, Some(version)).ok_or("no such skill")?;
    let model = llm::model_name(&db);
    let results: Vec<_> = m
        .tests
        .iter()
        .map(|t| match llm::run_skill(&db, &model, &id, Some(version), &t.input) {
            Ok((out, _)) => skills::check(t, &out),
            Err(e) => skills::TestResult { input: t.input.clone(), pass: false, detail: e.to_string() },
        })
        .collect();
    let pass = !results.is_empty() && results.iter().all(|r| r.pass);
    db.kv_set(&format!("skilltest:{id}:{version}"), if pass { "pass" } else { "fail" });
    Ok(results)
}

#[cfg(feature = "voice")]
#[tauri::command]
fn skill_set_status(id: String, version: i64, status: String, db: tauri::State<Arc<db::Db>>) -> Result<(), String> {
    let m = db.skill(&id, Some(version)).ok_or("no such skill")?;
    let passed = db.kv_get(&format!("skilltest:{id}:{version}")).as_deref() == Some("pass");
    skills::transition(&m.status, &status, passed)?;
    db.skill_set_status(&id, version, &status);
    Ok(())
}

/// Drafts a prompt-only skill from a description. It is stored as `proposed` and can't run until reviewed, tested and approved.
#[cfg(feature = "voice")]
#[tauri::command(async)]
fn propose_skill(description: String, db: tauri::State<Arc<db::Db>>) -> Result<skills::Manifest, String> {
    let draft: skills::Draft = llm::json(&llm::model_name(&db), skills::DRAFT_SYSTEM, &description, skills::draft_schema(), 120).map_err(|e| e.to_string())?;
    let mut m = skills::from_draft(&draft);
    while db.skill(&m.id, Some(m.version)).is_some() {
        m.version += 1;
    }
    db.skill_put(&m);
    Ok(m)
}

#[cfg(feature = "voice")]
#[tauri::command]
fn model(db: tauri::State<Arc<db::Db>>) -> String {
    llm::model_name(&db)
}

#[cfg(feature = "voice")]
#[tauri::command]
fn set_model(name: String, db: tauri::State<Arc<db::Db>>) {
    db.kv_set("model", name.trim());
}

/// Installed Ollama models (empty if Ollama isn't running).
#[cfg(feature = "voice")]
#[tauri::command(async)]
fn models() -> Vec<String> {
    llm::installed_models()
}

#[cfg(feature = "voice")]
#[tauri::command(async)]
fn diagnostics(db: tauri::State<Arc<db::Db>>) -> serde_json::Value {
    let installed = llm::installed_models();
    let (m, v) = (llm::model_name(&db), llm::vision_model(&db));
    serde_json::json!({
        "ollama": !installed.is_empty(),
        "model": m, "model_installed": installed.contains(&m),
        "vision_model": v, "vision_installed": installed.contains(&v),
        "concepts": db.graph().len(), "notes": db.docs().len(), "skills": db.skills().len(),
        "stt_models": std::path::Path::new(&models::path("silero_vad.onnx")).exists(),
    })
}

#[cfg(feature = "voice")]
#[tauri::command]
fn graph(db: tauri::State<Arc<db::Db>>) -> Vec<db::Node> {
    db.graph()
}

#[cfg(feature = "voice")]
#[tauri::command]
fn history(limit: i64, db: tauri::State<Arc<db::Db>>) -> Vec<db::Msg> {
    db.history(limit)
}

#[cfg(feature = "voice")]
struct Tts(Mutex<std::sync::mpsc::Sender<tts::Cmd>>);

#[cfg(feature = "voice")]
#[tauri::command]
fn speak(text: String, tts: tauri::State<Tts>) {
    tts::say(&tts.0.lock().unwrap(), text, tts::epoch());
}

#[cfg(feature = "voice")]
#[tauri::command]
fn stop_speaking() {
    tts::interrupt();
}

fn open_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn main() {
    tauri::Builder::default()
        // Must be first: a second launch just surfaces the first.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| open_main(app)))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            #[cfg(feature = "voice")]
            speak,
            #[cfg(feature = "voice")]
            stop_speaking,
            #[cfg(feature = "voice")]
            ask,
            #[cfg(feature = "voice")]
            study_state,
            #[cfg(feature = "voice")]
            graph,
            #[cfg(feature = "voice")]
            skills,
            #[cfg(feature = "voice")]
            skill_run,
            #[cfg(feature = "voice")]
            skill_test,
            #[cfg(feature = "voice")]
            skill_set_status,
            #[cfg(feature = "voice")]
            propose_skill,
            #[cfg(feature = "voice")]
            model,
            #[cfg(feature = "voice")]
            set_model,
            #[cfg(feature = "voice")]
            models,
            #[cfg(feature = "voice")]
            diagnostics,
            #[cfg(feature = "voice")]
            import_notes,
            #[cfg(feature = "voice")]
            notes,
            #[cfg(feature = "voice")]
            remove_note,
            #[cfg(feature = "voice")]
            history
        ])
        .setup(|app| {
            #[cfg(feature = "voice")]
            app.manage(Tts(Mutex::new(tts::spawn(app.handle().clone()))));

            let hud_win = app.get_webview_window("hud").expect("hud window");
            float_everywhere(&hud_win);
            if let Some(overlay) = app.get_webview_window("overlay") {
                float_everywhere(&overlay);
            }

            let hud = Hud {
                app: app.handle().clone(),
                win: hud_win,
                wake_until: Arc::default(),
                epoch: Arc::default(),
            };
            #[cfg(feature = "voice")]
            {
                let db = Arc::new(db::Db::open(&app.path().app_data_dir()?.join("sunday.db"))?);
                let tts_tx = app.state::<Tts>().0.lock().unwrap().clone();
                let reply = llm::spawn(hud.clone(), tts_tx, db.clone());
                app.manage(db);
                app.manage(Reply(Mutex::new(reply)));
            }
            watch_fn(hud.clone());
            listen(hud);

            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, "open", "Open Sunday", true, None::<&str>)?,
                    &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
                ],
            )?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().expect("icon").clone())
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => open_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|t, e| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        open_main(t.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|win, e| {
            if let (WindowEvent::CloseRequested { api, .. }, "main") = (e, win.label()) {
                api.prevent_close();
                let _ = win.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("tauri failed")
        .run(|app, e| match e {
            // Hiding every window must not end the app; only an explicit quit (code Some) does.
            RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
            // Dock icon clicked while the main window is hidden.
            RunEvent::Reopen { .. } => open_main(app),
            _ => {}
        });
}
