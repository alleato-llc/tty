//! tty's theming — deliberately tiny. Where fed's `patina` carries a full editor +
//! syntax + terminal theme with TOML user themes, a terminal only needs chrome colors
//! and a terminal palette, so tty leans on rime's built-in [`ThemeChoice`] (Dracula
//! dark / GitHub light) and pairs each with a [`phosphor::TerminalStyle`].

use rime::theme::{Palette, ThemeChoice};

use phosphor::TerminalStyle;

/// The active look: a rime chrome [`Palette`] (status bar, tab strip) plus the
/// [`TerminalStyle`] the terminal widget renders with.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub choice: ThemeChoice,
    pub palette: Palette,
    pub terminal: TerminalStyle,
}

impl Theme {
    /// Build the theme for a dark/light choice.
    pub fn new(choice: ThemeChoice) -> Self {
        Self {
            choice,
            palette: choice.palette(),
            terminal: terminal_style(choice),
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
            s.bg = iced::Color::from_rgb8(0xff, 0xff, 0xff);
            s.fg = iced::Color::from_rgb8(0x24, 0x29, 0x2e);
            s.cursor = iced::Color::from_rgb8(0x24, 0x29, 0x2e);
            s.selection = iced::Color::from_rgba8(0x03, 0x66, 0xd6, 0.25);
            s
        }
    }
}
