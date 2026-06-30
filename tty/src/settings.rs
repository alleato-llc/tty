//! tty's persisted preferences — a small `tty.settings.json` in the user config dir.
//! The terminal counterpart of fed's `patina::Settings`, scoped to what a terminal
//! actually needs: dark/light, font, and an optional custom palette (the 16 ANSI
//! colors + fg/bg/cursor) edited in the settings panel or imported from a base16
//! scheme.

use std::path::PathBuf;

use iced::Color;
use rime::theme::{color_hex, parse_color};

use phosphor::TerminalStyle;

/// A custom terminal palette as hex strings (so it round-trips through JSON cleanly).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Palette {
    /// The 16 ANSI colors (0–7 normal, 8–15 bright).
    pub ansi: Vec<String>,
    pub fg: String,
    pub bg: String,
    pub cursor: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// The built-in theme name (e.g. `"Dracula"`, `"Nord"`); absent reads as Dracula.
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
    /// A custom palette overriding the built-in dark/light terminal colors.
    #[serde(default)]
    pub palette: Option<Palette>,
}

impl Settings {
    /// Load `tty.settings.json`, or defaults if it's missing or malformed.
    pub fn load() -> Self {
        match std::fs::read_to_string(path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist to `tty.settings.json` (best-effort; a write failure isn't fatal).
    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, json);
        }
    }

    /// The custom [`TerminalStyle`] this file describes, if any (the panel/base16 edits).
    pub fn custom_style(&self) -> Option<TerminalStyle> {
        let p = self.palette.as_ref()?;
        if p.ansi.len() != 16 {
            return None;
        }
        let mut ansi = [Color::BLACK; 16];
        for (i, hex) in p.ansi.iter().enumerate() {
            ansi[i] = parse_color(hex)?;
        }
        Some(TerminalStyle {
            ansi,
            fg: parse_color(&p.fg)?,
            bg: parse_color(&p.bg)?,
            cursor: parse_color(&p.cursor)?,
            // Keep a readable selection tint derived from the chosen blue.
            selection: Color { a: 0.4, ..ansi[4] },
        })
    }

    /// Replace the custom palette from a fully-resolved [`TerminalStyle`] (panel edits +
    /// base16 import both funnel through here).
    pub fn set_palette(&mut self, style: &TerminalStyle) {
        self.palette = Some(Palette {
            ansi: style.ansi.iter().map(|c| color_hex(*c)).collect(),
            fg: color_hex(style.fg),
            bg: color_hex(style.bg),
            cursor: color_hex(style.cursor),
        });
    }
}

/// `~/.config/tty/tty.settings.json` (or the platform equivalent).
fn path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tty")
        .join("tty.settings.json")
}
