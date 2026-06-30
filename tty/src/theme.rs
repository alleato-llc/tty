//! tty's theming — deliberately tiny. rime's built-in [`ThemeChoice`] (Dracula dark /
//! GitHub light) supplies the chrome palette; the terminal palette is either the
//! conventional dark/light default or a custom one from the settings panel / a base16
//! scheme (see [`base16`]).

use iced::Color;
use rime::theme::{parse_color, Palette, ThemeChoice};

use phosphor::TerminalStyle;

use crate::settings::Settings;

/// The active look: a rime chrome [`Palette`] (status bar, tab strip), the
/// [`TerminalStyle`] the terminal renders with, and whether the retro overlay is on.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub choice: ThemeChoice,
    pub palette: Palette,
    pub terminal: TerminalStyle,
    /// Retro CRT overlay intensities (`0.0`..=`1.0`): the refresh lines and the
    /// glass-curve vignette. Both `0.0` means the overlay is off.
    pub scanlines: f32,
    pub vignette: f32,
}

impl Theme {
    /// Build the theme for a dark/light choice with the default terminal palette.
    pub fn new(choice: ThemeChoice) -> Self {
        Self {
            choice,
            palette: choice.palette(),
            terminal: terminal_style(choice),
            scanlines: 0.0,
            vignette: 0.0,
        }
    }

    /// Build from persisted settings: chrome from the dark/light choice, terminal from
    /// the custom palette if set (else the default), plus the retro intensities.
    pub fn from_settings(s: &Settings) -> Self {
        let choice = s.theme_choice();
        Self {
            choice,
            palette: choice.palette(),
            terminal: s.custom_style().unwrap_or_else(|| terminal_style(choice)),
            scanlines: s.scanlines(),
            vignette: s.vignette(),
        }
    }

    /// The iced theme for built-in widgets (scrollbars, etc.).
    pub fn iced(&self) -> iced::Theme {
        self.choice.theme()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemeChoice::default())
    }
}

/// The terminal palette paired with a chrome choice: the conventional dark palette for
/// dark, a light-background variant for light.
fn terminal_style(choice: ThemeChoice) -> TerminalStyle {
    match choice {
        ThemeChoice::Dark => TerminalStyle::default_dark(),
        ThemeChoice::Light => {
            let mut s = TerminalStyle::default_dark();
            s.bg = Color::from_rgb8(0xff, 0xff, 0xff);
            s.fg = Color::from_rgb8(0x24, 0x29, 0x2e);
            s.cursor = Color::from_rgb8(0x24, 0x29, 0x2e);
            s.selection = Color::from_rgba8(0x03, 0x66, 0xd6, 0.25);
            s
        }
    }
}

/// base16 scheme support. A scheme is 16 colors `base00`..`base0F`; we map them to the
/// terminal's 16 ANSI slots + fg/bg/cursor with the conventional base16 ↔ ansi mapping.
pub mod base16 {
    use super::*;

    /// Parse 16 hex colors (whitespace/comma/newline separated, `#` optional) into a
    /// [`TerminalStyle`]. Returns `None` unless exactly 16 parse.
    pub fn parse(input: &str) -> Option<TerminalStyle> {
        let mut cols = Vec::new();
        for tok in input.split(|c: char| c.is_whitespace() || c == ',') {
            if tok.is_empty() {
                continue;
            }
            let hex = if tok.starts_with('#') {
                tok.to_string()
            } else {
                format!("#{tok}")
            };
            cols.push(parse_color(&hex)?);
        }
        if cols.len() != 16 {
            return None;
        }
        Some(from_base16(&cols))
    }

    /// Map `base00`..`base0F` to a terminal style (the standard base16 shell template).
    pub fn from_base16(b: &[Color]) -> TerminalStyle {
        let ansi = [
            b[0x0], b[0x8], b[0xB], b[0xA], b[0xD], b[0xE], b[0xC], b[0x5], // 0–7
            b[0x3], b[0x8], b[0xB], b[0xA], b[0xD], b[0xE], b[0xC], b[0x7], // 8–15 bright
        ];
        TerminalStyle {
            ansi,
            fg: b[0x5],
            bg: b[0x0],
            cursor: b[0x5],
            selection: Color { a: 0.4, ..b[0xD] },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    /// 16 distinct greys, base00 (#000000) … base0F (#f0f0f0).
    fn sixteen() -> String {
        (0..16)
            .map(|i| format!("#{0:02x}{0:02x}{0:02x}", i * 16))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn base16_maps_bg_to_base00_and_fg_to_base05() {
        let style = base16::parse(&sixteen()).expect("16 colors parse");
        // bg = base00 (black), fg = base05.
        assert_eq!(style.bg, Color::from_rgb8(0x00, 0x00, 0x00));
        assert_eq!(style.fg, Color::from_rgb8(0x50, 0x50, 0x50));
        // ansi[0] is base00, ansi[7] is base05 (the standard template).
        assert_eq!(style.ansi[0], style.bg);
        assert_eq!(style.ansi[7], style.fg);
    }

    #[test]
    fn base16_rejects_wrong_count() {
        assert!(base16::parse("#111111 #222222").is_none());
        assert!(base16::parse("not hex at all").is_none());
    }

    #[test]
    fn from_settings_carries_retro_and_custom_palette() {
        let mut s = Settings {
            scanlines: Some(0.7),
            vignette: Some(0.4),
            ..Settings::default()
        };
        s.set_palette(&base16::parse(&sixteen()).unwrap());
        let theme = Theme::from_settings(&s);
        assert_eq!(theme.scanlines, 0.7, "scanline intensity flows through");
        assert_eq!(theme.vignette, 0.4, "vignette intensity flows through");
        assert_eq!(theme.terminal.bg, Color::from_rgb8(0x00, 0x00, 0x00));
    }
}
