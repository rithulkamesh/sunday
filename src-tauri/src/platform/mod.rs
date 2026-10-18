//! Everything that talks to the operating system on Sunday's behalf: open apps, pages and files, type, click,
//! screenshot, the window list, reading text on screen, the front browser's address, permissions, and the HUD's window
//! behaviour. Callers use `current()` and never name a tool. macOS is the only real implementation today
//! (`macos.rs`); every other OS gets `unsupported.rs`, whose calls fail with a message fit to be spoken. Adding
//! Windows or Linux means writing one file that implements `Platform`.
use crate::llm::Res;
use crate::screen::{Desk, Line, Region};
use std::path::Path;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
use unsupported as imp;
/// Compiled on every OS in tests, so the stub can't rot while macOS is the only OS in CI.
#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
#[path = "unsupported.rs"]
mod unsupported_check;

pub use imp::{float_everywhere, fn_down, order_front};

pub trait Platform: Send + Sync {
    fn open_app(&self, name: &str) -> Result<(), String>;
    /// `browser`: the browser to use, if one is already open; otherwise the system default.
    fn open_url(&self, url: &str, browser: Option<&str>) -> Result<(), String>;
    fn open_path(&self, path: &Path) -> Result<(), String>;
    /// Types into the front app. The text is never run as a script.
    fn type_text(&self, text: &str) -> Result<(), String>;
    /// Left-click at a point of the main display, 0..1000 each way.
    fn click(&self, x: f64, y: f64) -> Result<(), String>;
    /// A full-resolution JPEG of the main display at `path`. Fails with a spoken message if permission is missing.
    fn screenshot(&self, path: &Path) -> Res<()>;
    /// Whether screen capture is allowed (asks once where the OS has a prompt).
    fn screen_access(&self) -> bool;
    /// Display size, the front app, and the windows open (front to back). None if the OS can't say.
    fn desk(&self) -> Option<Desk>;
    fn frontmost_app(&self) -> Option<String> {
        self.desk().map(|d| d.front_app())
    }
    /// Text in an image with exact boxes. Empty if reading text isn't available.
    fn ocr(&self, image: &Path) -> Vec<Line>;
    /// Crops `region` of `src` into `dst`; false if it can't.
    fn crop(&self, src: &Path, dst: &Path, region: Region) -> bool;
    /// Shrinks `src` so its longest edge is `edge` pixels, into `dst`; false if it can't.
    fn resize(&self, src: &Path, dst: &Path, edge: &str) -> bool;
    /// A Chromium-family browser installed here, to drive headlessly.
    fn chrome_path(&self) -> Option<std::path::PathBuf>;
    /// The address of the page open in the front browser.
    fn browser_url(&self, front_app: &str) -> Result<String, String>;
}

pub fn current() -> &'static dyn Platform {
    #[cfg(test)]
    if crate::agent::sandbox::on() {
        return &Recorder;
    }
    &imp::OS
}

/// In eval sandboxes nothing touches the machine: each call is noted as the command it would have been.
#[cfg(test)]
struct Recorder;

#[cfg(test)]
impl Platform for Recorder {
    fn open_app(&self, name: &str) -> Result<(), String> {
        note(format!("open -a {name}"))
    }
    fn open_url(&self, url: &str, browser: Option<&str>) -> Result<(), String> {
        note(match browser {
            Some(b) => format!("open -a {b} {url}"),
            None => format!("open {url}"),
        })
    }
    fn open_path(&self, path: &Path) -> Result<(), String> {
        note(format!("open {}", path.display()))
    }
    fn type_text(&self, text: &str) -> Result<(), String> {
        note(format!("type {text}"))
    }
    fn click(&self, x: f64, y: f64) -> Result<(), String> {
        note(format!("click {x:.0},{y:.0}"))
    }
    fn screenshot(&self, _: &Path) -> Res<()> {
        Err("no screen in the sandbox".into())
    }
    fn screen_access(&self) -> bool {
        false
    }
    fn desk(&self) -> Option<Desk> {
        None
    }
    fn ocr(&self, _: &Path) -> Vec<Line> {
        vec![]
    }
    fn crop(&self, _: &Path, _: &Path, _: Region) -> bool {
        false
    }
    fn resize(&self, _: &Path, _: &Path, _: &str) -> bool {
        false
    }
    fn chrome_path(&self) -> Option<std::path::PathBuf> {
        None
    }
    fn browser_url(&self, _: &str) -> Result<String, String> {
        Err("no browser in the sandbox".into())
    }
}

#[cfg(test)]
fn note(effect: String) -> Result<(), String> {
    crate::agent::sandbox::record(effect);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stub for every other OS: nothing panics, and every action says plainly that it can't be done.
    #[test]
    fn unsupported_os_fails_with_a_spoken_message() {
        let p = super::unsupported_check::OS;
        for r in [p.open_app("Notes"), p.open_url("https://example.com", None), p.open_path(Path::new("/tmp")), p.type_text("hi"), p.click(500.0, 500.0), p.browser_url("Safari").map(|_| ())] {
            assert!(r.unwrap_err().contains("isn't supported on this OS yet"));
        }
        assert!(p.screenshot(Path::new("/tmp/x.jpg")).unwrap_err().to_string().contains("isn't supported on this OS yet"));
        assert!(p.desk().is_none() && p.frontmost_app().is_none() && p.chrome_path().is_none());
        assert!(p.ocr(Path::new("/tmp/x.jpg")).is_empty());
        assert!(!p.crop(Path::new("a"), Path::new("b"), Region { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }) && !p.resize(Path::new("a"), Path::new("b"), "9"));
        assert!(!super::unsupported_check::fn_down());
    }

    #[test]
    fn the_sandbox_records_instead_of_acting() {
        let ((), sb) = crate::agent::sandbox::run(crate::agent::sandbox::Sandbox::default(), || {
            current().open_app("Safari").unwrap();
            current().click(500.0, 250.0).unwrap();
        });
        assert_eq!(sb.effects, ["open -a Safari", "click 500,250"]);
    }
}
