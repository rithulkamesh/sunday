//! macOS: `open`, AppleScript, `screencapture`, `sips`, CoreGraphics, and the Swift helper that scripts/build-ocr.sh
//! builds (text reader, window list, cropper, clicker). This is the only file that names those tools.
use super::Platform;
use crate::llm::{Res, UserMsg};
use crate::screen::{Desk, Line, Region};
use std::path::{Path, PathBuf};
use tauri::WebviewWindow;

pub struct Mac;
pub const OS: Mac = Mac;

/// scripts/build-ocr.sh puts the helper (text reader, window list, cropper) next to the models directory.
fn tool() -> Option<PathBuf> {
    let p = crate::models::dir().parent()?.join("bin/ocr");
    p.exists().then_some(p)
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let o = std::process::Command::new(cmd).args(args).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().chars().take(200).collect()) }
}

impl Platform for Mac {
    fn open_app(&self, name: &str) -> Result<(), String> {
        run("open", &["-a", name])
    }

    // In the browser the user already has open, not whichever one is the system default.
    fn open_url(&self, url: &str, browser: Option<&str>) -> Result<(), String> {
        match browser {
            Some(b) => run("open", &["-a", b, url]),
            None => run("open", &[url]),
        }
    }

    fn open_path(&self, path: &Path) -> Result<(), String> {
        run("open", &[path.to_string_lossy().as_ref()])
    }

    // The text travels as an argument, never inside the script, so it can't be executed as one.
    fn type_text(&self, text: &str) -> Result<(), String> {
        run("osascript", &["-e", "on run argv\ntell application \"System Events\" to keystroke (item 1 of argv)\nend run", text])
            .map_err(|e| format!("typing failed ({e}); Sunday may need Accessibility permission"))
    }

    fn click(&self, x: f64, y: f64) -> Result<(), String> {
        let (d, t) = (self.desk().ok_or("I can't read the display.")?, tool().ok_or("the screen helper isn't built (run scripts/build-ocr.sh)")?);
        let (w, h) = d.display_size();
        let ok = std::process::Command::new(t).arg("--click").args([format!("{:.0}", x / 1000.0 * w), format!("{:.0}", y / 1000.0 * h)]).status().is_ok_and(|s| s.success());
        if ok { Ok(()) } else { Err("the click failed; Sunday may need Accessibility permission".into()) }
    }

    fn screenshot(&self, path: &Path) -> Res<()> {
        if !self.screen_access() {
            return Err(UserMsg("I need screen recording permission. Allow it in System Settings, Privacy and Security, Screen Recording, then restart me.".into()).into());
        }
        if !std::process::Command::new("screencapture").args(["-x", "-m", "-t", "jpg"]).arg(path).status()?.success() {
            return Err("the screenshot failed".into());
        }
        Ok(())
    }

    fn screen_access(&self) -> bool {
        extern "C" {
            fn CGPreflightScreenCaptureAccess() -> bool;
            fn CGRequestScreenCaptureAccess() -> bool;
        }
        // SAFETY: plain CoreGraphics queries with no arguments. The request call shows the system prompt once.
        unsafe { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() }
    }

    fn desk(&self) -> Option<Desk> {
        let out = std::process::Command::new(tool()?).arg("--windows").output().ok().filter(|o| o.status.success())?;
        serde_json::from_slice(&out.stdout).ok()
    }

    /// Empty (never an error) if the helper isn't built or fails: pointing then falls back to the vision model's own coordinates.
    fn ocr(&self, image: &Path) -> Vec<Line> {
        let Some(tool) = tool() else {
            eprintln!("[screen] no helper tool (run scripts/build-ocr.sh): pointing will be approximate");
            return vec![];
        };
        let out = std::process::Command::new(tool).arg(image).output().ok().filter(|o| o.status.success());
        out.and_then(|o| serde_json::from_slice::<Vec<Line>>(&o.stdout).ok()).unwrap_or_default()
    }

    fn crop(&self, src: &Path, dst: &Path, r: Region) -> bool {
        tool().is_some_and(|t| {
            std::process::Command::new(t)
                .arg("--crop").arg(src).arg(dst)
                .args([r.x, r.y, r.w, r.h].map(|v| format!("{v:.1}")))
                .status().is_ok_and(|s| s.success())
        })
    }

    fn resize(&self, src: &Path, dst: &Path, edge: &str) -> bool {
        std::process::Command::new("sips").args(["-Z", edge, "--out"]).arg(dst).arg(src).output().is_ok_and(|o| o.status.success())
    }

    fn chrome_path(&self) -> Option<PathBuf> {
        const APPS: [&str; 6] = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
            "/usr/bin/chromium",
        ];
        APPS.iter().map(PathBuf::from).find(|p| p.exists())
    }

    /// The address of the page open in the front browser. The first use asks for Automation permission.
    fn browser_url(&self, front_app: &str) -> Result<String, String> {
        const CHROMIUM: [&str; 7] = ["Google Chrome", "Brave Browser", "Microsoft Edge", "Chromium", "Vivaldi", "Arc", "Dia"];
        let script = match front_app {
            "Safari" => r#"tell application "Safari" to get URL of front document"#.to_string(),
            a if CHROMIUM.contains(&a) => format!(r#"tell application "{a}" to get URL of active tab of front window"#),
            a if a.eq_ignore_ascii_case("sunday") => return Err("Switch to the browser tab first and ask by voice, or paste the link".into()),
            a => return Err(format!("{a} isn't a browser I can read an address from")),
        };
        let out = std::process::Command::new("osascript").arg("-e").arg(&script).output().map_err(|e| e.to_string())?;
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && url.starts_with("http") {
            Ok(url)
        } else {
            Err(format!("I couldn't read the address from {front_app}. Allow this app to control it in System Settings, Privacy and Security, Automation"))
        }
    }
}

/// fn state via CGEventSourceFlagsState: no event tap, so no Input Monitoring prompt and no
/// key-translation crash (rdev's listener aborts the process on recent macOS).
pub fn fn_down() -> bool {
    extern "C" {
        fn CGEventSourceFlagsState(state: i32) -> u64;
    }
    const COMBINED_SESSION: i32 = 0;
    const FN_FLAG: u64 = 1 << 23;
    // ponytail: arrow/F-keys also set this flag; fine for a hold-to-talk key.
    unsafe { CGEventSourceFlagsState(COMBINED_SESSION) & FN_FLAG != 0 }
}

/// Shows a window without activating the app (activation drags the main window forward too).
/// AppKit calls must run on the main thread, or the process aborts.
pub fn order_front(app: &tauri::AppHandle, win: &WebviewWindow) {
    let win = win.clone();
    let _ = app.run_on_main_thread(move || {
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
pub fn float_everywhere(win: &WebviewWindow) {
    let _ = win.set_ignore_cursor_events(true);
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
