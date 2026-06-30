//! tty's persisted preferences — a small `tty.settings.json` in the user config dir.
//! The terminal counterpart of fed's `patina::Settings`, scoped to what a terminal
//! actually needs: dark/light, font family, font size.

use std::path::PathBuf;

use rime::theme::ThemeChoice;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// `"dark"` or `"light"`; absent reads as dark.
    #[serde(default)]
    pub theme: Option<String>,
    /// Monospace font family override; absent uses the platform monospace.
    #[serde(default)]
    pub font_family: Option<String>,
    /// Font size in points; absent uses the app default.
    #[serde(default)]
    pub font_size: Option<f32>,
}

impl Settings {
    /// Load `tty.settings.json`, or defaults if it's missing or malformed.
    pub fn load() -> Self {
        match std::fs::read_to_string(path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// The dark/light choice this file selects (default dark).
    pub fn theme_choice(&self) -> ThemeChoice {
        match self.theme.as_deref() {
            Some("light") => ThemeChoice::Light,
            _ => ThemeChoice::Dark,
        }
    }
}

/// `~/.config/tty/tty.settings.json` (or the platform equivalent).
fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tty")
        .join("tty.settings.json")
}
