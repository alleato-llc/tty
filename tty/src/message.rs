use iced::keyboard::{Key, Modifiers};
use iced::Color;

/// Everything the UI can ask `tty` to do.
#[derive(Debug, Clone)]
pub enum Message {
    /// A key press (logical key + modifiers) — an app chord or bytes for the PTY.
    Key(Key, Modifiers),
    /// Keyboard modifiers changed.
    ModifiersChanged(Modifiers),
    /// The terminal widget reports the grid that now fits (cols, rows).
    Resize(usize, usize),
    /// The terminal selection changed (drag) — cached for ⌘C copy.
    Select(Option<String>),
    /// Bytes the terminal widget produced (mouse reporting) to send to the PTY.
    PtyBytes(Vec<u8>),
    /// A clipboard read for ⌘V resolved — paste into the active shell.
    Pasted(Option<String>),
    /// The `⌘F` find query changed.
    SearchChanged(String),
    /// The find bar was submitted (Enter) — close it, keep the cursor in the terminal.
    SearchSubmit,
    /// Open a new terminal tab.
    NewTab,
    /// Close tab `i` (the window closes when the last tab goes).
    CloseTab(usize),
    /// Make tab `i` active.
    ActivateTab(usize),
    /// The pointer entered tab `i` (`Some`) or left the strip (`None`).
    HoverTab(Option<usize>),
    /// Periodic redraw, so PTY output appears.
    Tick,
    /// The window was resized to this height (for the status bar's edge band).
    WindowResized(f32),
    /// Open/close the settings panel (`⌘,`).
    ToggleSettings,
    /// Switch the settings panel to section `i` (Appearance / Palette).
    SettingsSection(usize),
    /// Pick the dark/light chrome theme (`"dark"` / `"light"`).
    SetTheme(String),
    /// Toggle the retro CRT overlay.
    ToggleRetro,
    /// Nudge the font size by `±1` from the settings stepper.
    FontSizeStep(f32),
    /// The base16 paste box changed.
    Base16Changed(String),
    /// Import the base16 colors in the paste box as the terminal palette.
    ApplyBase16,
    /// Drop the custom palette, back to the built-in dark/light colors.
    ResetPalette,
    /// Edit one palette slot (`0..16` = ANSI, `16`=fg, `17`=bg, `18`=cursor).
    EditColor(usize, Color),
}
