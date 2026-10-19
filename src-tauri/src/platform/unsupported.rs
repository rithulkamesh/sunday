//! Every OS without a real implementation yet. Nothing here acts; each call says so in words that can be spoken.
use super::Platform;
use crate::llm::{Res, UserMsg};
use crate::screen::{Desk, Line, Region};
use std::path::Path;
use tauri::WebviewWindow;

pub struct Unsupported;
pub const OS: Unsupported = Unsupported;

const NO: &str = "That isn't supported on this OS yet.";

fn no<T>() -> Result<T, String> {
    Err(NO.into())
}

impl Platform for Unsupported {
    fn open_app(&self, _: &str) -> Result<(), String> {
        no()
    }
    fn open_url(&self, _: &str, _: Option<&str>) -> Result<(), String> {
        no()
    }
    fn open_path(&self, _: &Path) -> Result<(), String> {
        no()
    }
    fn type_text(&self, _: &str) -> Result<(), String> {
        no()
    }
    fn click(&self, _: f64, _: f64) -> Result<(), String> {
        no()
    }
    fn press_keys(&self, _: &str) -> Result<(), String> {
        no()
    }
    fn screenshot(&self, _: &Path) -> Res<()> {
        Err(UserMsg(NO.into()).into())
    }
    fn screen_access(&self) -> bool {
        true
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
        no()
    }
}

/// No hold-to-talk key until the OS has a backend.
pub fn fn_down() -> bool {
    false
}

pub fn order_front(_: &tauri::AppHandle, win: &WebviewWindow) {
    let _ = win.show();
}

pub fn float_everywhere(win: &WebviewWindow) {
    let _ = win.set_ignore_cursor_events(true);
}
