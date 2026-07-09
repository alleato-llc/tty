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
    /// A pane's terminal widget reports the grid that now fits (window, pane, cols, rows).
    /// The window id disambiguates panes across tabs/windows (`pane_grid::Pane` ids are
    /// only unique within one `pane_grid::State`).
    Resize(iced::window::Id, pane_grid::Pane, usize, usize),
    /// A pane's selection changed (drag) — cached for ⌘C copy when it's the focused pane
    /// of the reporting window's tab.
    Select(iced::window::Id, pane_grid::Pane, Option<String>),
    /// Bytes a pane's terminal widget produced (mouse reporting) to send to its PTY.
    PtyBytes(iced::window::Id, pane_grid::Pane, Vec<u8>),
    /// Focus the pane that was clicked. (Keyboard split / focus-move / close are chords
    /// handled directly in `update::handle_key`, like the other ⌘ shortcuts.)
    FocusPane(iced::window::Id, pane_grid::Pane),
    /// Drag-resize a split divider.
    ResizeSplit(iced::window::Id, pane_grid::ResizeEvent),
    /// The cursor moved (window-relative) — tracked so a right-click can anchor a menu.
    PointerMoved(iced::Point),
    /// Right-click on a pane — open the split context menu over it.
    PaneRightClick(pane_grid::Pane),
    /// Right-click on a tab — open the split context menu for that tab.
    TabRightClick(usize),
    /// A right-click landed on a detected URL in a pane — open the link menu over it.
    LinkClick(String),
    /// The "Open Link" menu item was chosen — open the URL in the default browser.
    OpenLink(String),
    /// The "Copy Link" menu item was chosen — write the URL to the clipboard.
    CopyLink(String),
    /// A "Split <dir>" context-menu item was chosen.
    Split(pane_grid::Direction),
    /// The "Close pane" context-menu item was chosen.
    ClosePane,
    /// Dismiss the pane context menu (an item was chosen, or a click landed outside).
    CloseMenu,
    /// Begin renaming tab `i` (the "Rename tab…" menu item).
    StartRename(usize),
    /// The rename field's text changed.
    RenameChanged(String),
    /// The rename field was submitted (Enter) — commit the new name.
    RenameSubmit,
    /// A clipboard read for ⌘V resolved — paste into the active shell.
    Pasted(Option<String>),
    /// The `⌘F` find query changed.
    SearchChanged(String),
    /// The find bar was submitted (Enter) — jump to the next match, or the previous
    /// one if ⇧ is held (checked against the live `modifiers`, since a text field's
    /// `on_submit` doesn't carry them). The bar stays open; Esc closes it.
    SearchSubmit,
    /// ⌘K — clear the active pane's buffered scrollback.
    ClearScrollback,
    /// ⌘⇧H — open/close the scrollback history panel.
    ToggleScrollbackPanel,
    /// The scrollback panel's own filter query changed.
    ScrollbackQueryChanged(String),
    /// An output row in the scrollback panel table was clicked — selects/highlights it.
    ScrollbackRowSelected(usize),
    /// A row in the scrollback panel table was double-clicked — selects it and
    /// copies its text to the clipboard.
    ScrollbackRowActivated(usize, String),
    /// A command's header row was clicked — toggles whether its output is shown.
    ScrollbackToggleExpand(usize),
    /// The scrollback panel table was scrolled.
    ScrollbackScrolled(f32),
    /// The settings stepper nudged the max-scrollback-lines setting.
    MaxScrollbackStep(i64),
    /// The settings stepper nudged the default-output-lines-per-command setting.
    DefaultOutputLinesStep(i64),
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
    /// Toggle inking the active tab with the accent color.
    SetTabHighlight(bool),

    // ---- multi-window: detachable tabs ----
    /// Detach the main strip's tab `idx` into its own OS window.
    DetachTab(usize),
    /// A detached window's "Reattach" button — dock its tab back into the main strip.
    ReattachTab(iced::window::Id),
    /// A window gained focus — route the keyboard to that window's tab.
    WindowFocused(iced::window::Id),
    /// A window finished closing — exit on the main window, reattach a detached one.
    WindowClosed(iced::window::Id),
    /// A window moved (drives the drag-to-dock bounds + release debounce).
    WindowMoved(iced::window::Id, iced::Point),
    /// A window was resized (the main window's height feeds the status bar's edge band).
    WindowResizedAt(iced::window::Id, iced::Size),
    /// A fetched on-screen window position (the initial placement `Moved` never reports).
    WindowPosition(iced::window::Id, Option<iced::Point>),
    /// Poll the drag-release debounce for drag-to-dock (only armed while a detached
    /// window is settling).
    CheckDragReattach,
    /// Global left-button release — completes a tab tear-off drag, if one is armed.
    PointerReleased,
}
