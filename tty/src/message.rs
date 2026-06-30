use iced::keyboard::{Key, Modifiers};
use iced::widget::pane_grid;
use iced::Color;

/// Everything the UI can ask `tty` to do.
#[derive(Debug, Clone)]
pub enum Message {
    /// A key press (logical key + modifiers) — an app chord or bytes for the PTY.
    Key(Key, Modifiers),
    /// Keyboard modifiers changed.
    ModifiersChanged(Modifiers),
    /// A pane's terminal widget reports the grid that now fits (pane, cols, rows).
    Resize(pane_grid::Pane, usize, usize),
    /// A pane's selection changed (drag) — cached for ⌘C copy when it's the focused pane.
    Select(pane_grid::Pane, Option<String>),
    /// Bytes a pane's terminal widget produced (mouse reporting) to send to its PTY.
    PtyBytes(pane_grid::Pane, Vec<u8>),
    /// Focus the pane that was clicked. (Keyboard split / focus-move / close are chords
    /// handled directly in `update::handle_key`, like the other ⌘ shortcuts.)
    FocusPane(pane_grid::Pane),
    /// Drag-resize a split divider.
    ResizeSplit(pane_grid::ResizeEvent),
    /// The cursor moved (window-relative) — tracked so a right-click can anchor a menu.
    PointerMoved(iced::Point),
    /// Right-click on a pane — open the split context menu over it.
    PaneRightClick(pane_grid::Pane),
    /// Right-click on a tab — open the split context menu for that tab.
    TabRightClick(usize),
    /// A "Split <dir>" context-menu item was chosen.
    Split(pane_grid::Direction),
    /// The "Close pane" context-menu item was chosen.
    ClosePane,
    /// Dismiss the pane context menu (an item was chosen, or a click landed outside).
    CloseMenu,
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
    /// Pick a named built-in theme (e.g. `"Nord"`).
    SetTheme(String),
    /// Pick the terminal font family (a `FONT_CHOICES` label).
    SetFont(String),
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
    /// The window gained (`true`) or lost (`false`) focus.
    Focused(bool),
    /// Set the unfocused-window terminal opacity (`1.0` = off).
    SetUnfocusedOpacity(f32),
}
