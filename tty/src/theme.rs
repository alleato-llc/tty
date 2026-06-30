//! tty's theming. The chrome palette (tabs, status bar, settings panel) comes from
//! rime's shared named-theme set — the same palettes fed's `patina` builds on — so the
//! two products stay in visual step. The terminal surface gets a coordinated 16-color
//! ANSI palette per theme (rime is terminal-free, so those live here, expressed as
//! [`base16`] schemes). A custom palette from the panel / a base16 import overrides the
//! terminal *and* re-themes the chrome, so the whole window moves together.

use iced::Color;
use rime::theme::{builtin_themes, parse_color, Palette, DRACULA};

use phosphor::TerminalStyle;

use crate::settings::Settings;

/// The active look: a rime chrome [`Palette`] (status bar, tab strip) and the
/// [`TerminalStyle`] the terminal renders with.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub palette: Palette,
    pub terminal: TerminalStyle,
}

impl Theme {
    /// The default theme (Dracula).
    pub fn dracula() -> Self {
        Self::named("Dracula")
    }

    /// Build a named built-in theme: chrome from rime, terminal from its base16 scheme.
    /// An unknown name falls back to Dracula.
    pub fn named(name: &str) -> Self {
        let palette = builtin_themes()
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, p, _)| *p)
            .unwrap_or(DRACULA);
        Self {
            palette,
            terminal: terminal_for(name),
        }
    }

    /// Build from persisted settings. A custom palette (panel edit / base16 import)
    /// takes over: it styles the terminal and the chrome is derived from it so the whole
    /// window re-themes. Otherwise it's the named built-in theme.
    pub fn from_settings(s: &Settings) -> Self {
        if let Some(terminal) = s.custom_style() {
            return Self {
                palette: chrome_from_terminal(&terminal),
                terminal,
            };
        }
        Self::named(s.theme.as_deref().unwrap_or("Dracula"))
    }

    /// The iced theme for built-in widgets (scrollbars, etc.), from the chrome palette.
    pub fn iced(&self) -> iced::Theme {
        self.palette.iced_theme("tty")
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dracula()
    }
}

/// The display-ordered built-in theme names, for the settings picker.
pub fn theme_names() -> Vec<String> {
    builtin_themes()
        .iter()
        .map(|(n, _, _)| n.to_string())
        .collect()
}

/// The terminal ANSI palette for a named theme (its canonical base16 scheme). Unknown
/// names fall back to the conventional dark palette.
fn terminal_for(name: &str) -> TerminalStyle {
    let hexes = match name.to_lowercase().as_str() {
        "nord" => &NORD16,
        "gruvbox dark" => &GRUVBOX16,
        "solarized dark" => &SOLARIZED_DARK16,
        "solarized light" => &SOLARIZED_LIGHT16,
        "github" => &GITHUB16,
        "dracula" => &DRACULA16,
        _ => return TerminalStyle::default_dark(),
    };
    base16_style(hexes)
}

/// Build a [`TerminalStyle`] from 16 base16 hex strings (`base00`..`base0F`).
fn base16_style(hexes: &[&str; 16]) -> TerminalStyle {
    let cols: Vec<Color> = hexes.iter().filter_map(|h| parse_color(h)).collect();
    if cols.len() == 16 {
        base16::from_base16(&cols)
    } else {
        TerminalStyle::default_dark()
    }
}

/// Derive a coherent chrome [`Palette`] from a terminal style, so a custom/base16
/// palette re-themes the tabs + status bar + panel, not just the grid.
fn chrome_from_terminal(s: &TerminalStyle) -> Palette {
    Palette {
        bg: s.bg,
        surface: mix(s.bg, s.fg, 0.08),
        ink: s.fg,
        muted: mix(s.fg, s.bg, 0.45),
        hairline: mix(s.bg, s.fg, 0.18),
        accent: s.ansi[4],  // blue
        success: s.ansi[2], // green
        warn: s.ansi[3],    // yellow
        danger: s.ansi[1],  // red
    }
}

/// Linear per-channel blend, `t` of the way from `a` to `b`.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t;
    Color::from_rgb(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b))
}

// Canonical base16 schemes (base00..base0F) for each built-in theme's terminal palette.
#[rustfmt::skip]
const DRACULA16: [&str; 16] = [
    "#282a36","#3a3c4e","#44475a","#6272a4","#9ea8c7","#f8f8f2","#f0f1f4","#ffffff",
    "#ff5555","#ffb86c","#f1fa8c","#50fa7b","#8be9fd","#bd93f9","#ff79c6","#ff5555",
];
#[rustfmt::skip]
const NORD16: [&str; 16] = [
    "#2e3440","#3b4252","#434c5e","#4c566a","#d8dee9","#e5e9f0","#eceff4","#8fbcbb",
    "#bf616a","#d08770","#ebcb8b","#a3be8c","#88c0d0","#81a1c1","#b48ead","#5e81ac",
];
#[rustfmt::skip]
const GRUVBOX16: [&str; 16] = [
    "#282828","#3c3836","#504945","#665c54","#bdae93","#d5c4a1","#ebdbb2","#fbf1c7",
    "#fb4934","#fe8019","#fabd2f","#b8bb26","#8ec07c","#83a598","#d3869b","#d65d0e",
];
#[rustfmt::skip]
const SOLARIZED_DARK16: [&str; 16] = [
    "#002b36","#073642","#586e75","#657b83","#839496","#93a1a1","#eee8d5","#fdf6e3",
    "#dc322f","#cb4b16","#b58900","#859900","#2aa198","#268bd2","#6c71c4","#d33682",
];
#[rustfmt::skip]
const SOLARIZED_LIGHT16: [&str; 16] = [
    "#fdf6e3","#eee8d5","#93a1a1","#839496","#657b83","#586e75","#073642","#002b36",
    "#dc322f","#cb4b16","#b58900","#859900","#2aa198","#268bd2","#6c71c4","#d33682",
];
#[rustfmt::skip]
const GITHUB16: [&str; 16] = [
    "#ffffff","#f6f8fa","#eaeef2","#6e7781","#57606a","#24292f","#1f2328","#1f2328",
    "#cf222e","#bc4c00","#9a6700","#116329","#1b7c83","#0969da","#8250df","#cf222e",
];

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
        assert_eq!(style.bg, Color::from_rgb8(0x00, 0x00, 0x00));
        assert_eq!(style.fg, Color::from_rgb8(0x50, 0x50, 0x50));
        assert_eq!(style.ansi[0], style.bg);
        assert_eq!(style.ansi[7], style.fg);
    }

    #[test]
    fn base16_rejects_wrong_count() {
        assert!(base16::parse("#111111 #222222").is_none());
        assert!(base16::parse("not hex at all").is_none());
    }

    #[test]
    fn named_theme_resolves_chrome_and_terminal() {
        let nord = Theme::named("Nord");
        assert_eq!(nord.terminal.bg, Color::from_rgb8(0x2e, 0x34, 0x40));
        assert_eq!(nord.palette.bg, rime::theme::NORD.bg);
        // Unknown name falls back to Dracula.
        assert_eq!(Theme::named("nope").palette.bg, rime::theme::DRACULA.bg);
    }

    #[test]
    fn custom_palette_rethemes_chrome_and_terminal() {
        let mut s = Settings::default();
        s.set_palette(&base16::parse(&sixteen()).unwrap());
        let theme = Theme::from_settings(&s);
        // Terminal bg is base00 (black) and the chrome bg follows it.
        assert_eq!(theme.terminal.bg, Color::from_rgb8(0x00, 0x00, 0x00));
        assert_eq!(theme.palette.bg, theme.terminal.bg);
    }
}
