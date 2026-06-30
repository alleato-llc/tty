use iced::keyboard::{Key, Modifiers};

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
}
