use iced::keyboard::{Key, Modifiers};
use iced::widget::pane_grid;
use iced::Color;

use crate::state::{ArchivedTarget, HistoryRowTarget};

/// The payload of a completed async history start. `history::Started` is
/// neither `Clone` nor `Debug` (it owns the writer thread handle and a key),
/// but `Message` derives both — so the message carries it behind a shared
/// take-once slot instead.
#[derive(Clone)]
pub struct StartedHandle(std::sync::Arc<std::sync::Mutex<Option<crate::history::Started>>>);

impl StartedHandle {
    pub fn new(started: crate::history::Started) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Some(started))))
    }

    /// Take the `Started` out (once) — `None` if it was already taken or the
    /// mutex was poisoned (only possible if a previous taker panicked).
    pub fn take(&self) -> Option<crate::history::Started> {
        self.0.lock().ok()?.take()
    }
}

impl std::fmt::Debug for StartedHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StartedHandle(..)")
    }
}

/// Which user action kicked off an async history start — determines the
/// failure semantics (an *enable* failure reverts the setting to off; a
/// *startup* failure keeps it, matching the long-standing sync behavior) and
/// whether success needs to commit the setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryStartOrigin {
    /// App launch, feature already enabled in settings.
    Startup,
    /// The settings toggle (committed to settings only on success).
    Enable,
    /// The post-Reset restart of a fresh archive.
    Reset,
    /// The passphrase unlock prompt (the feature was enabled but locked).
    Unlock,
}

/// How an async history start ended.
#[derive(Debug, Clone)]
pub enum HistoryStartOutcome {
    Ready(StartedHandle),
    /// The passphrase didn't open the archive (`Error::AuthFailed` — also,
    /// deliberately indistinguishably, a corrupted archive). The prompt
    /// shows it inline for a retry; history stays locked.
    WrongPassphrase,
    Failed,
}

/// What a completed re-auth prompt was unlocking — the prompt is async, so its
/// result message has to say which surface to open on success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReauthFor {
    /// The Scrollback History panel (⌘⇧H / the pane menu).
    ScrollbackPanel,
    /// The archived-commands viewer in the settings History section.
    SettingsHistory,
}

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
    /// A row in the scrollback panel table was right-clicked — opens a context
    /// menu (copy/clear) anchored at the cursor, targeting the resolved command or
    /// output line, live or archived.
    ScrollbackRowRightClick(usize, HistoryRowTarget),
    /// The scrollback row menu's "Copy" item was chosen — write its text to the
    /// clipboard.
    CopyScrollbackTarget(HistoryRowTarget),
    /// The scrollback row menu's "Clear" item was chosen — empty that row's value
    /// in place (the row stays, its text goes blank).
    ClearScrollbackTarget(HistoryRowTarget),
    /// The scrollback row menu's "Delete" item was chosen (command rows only) —
    /// permanently remove that command entry, header row and all.
    DeleteScrollbackTarget(HistoryRowTarget),
    /// The scrollback panel table was scrolled.
    ScrollbackScrolled(f32),
    /// The scrollback panel's "Load older" action — page one more day back
    /// into the encrypted archive.
    ScrollbackPageOlder,
    /// The scrollback panel's "Back to today" action — undo the oldest
    /// paged-in day, back toward the live view.
    ScrollbackPageNewer,
    /// The settings stepper nudged the max-scrollback-lines setting.
    MaxScrollbackStep(i64),
    /// The settings stepper nudged the default-output-lines-per-command setting.
    DefaultOutputLinesStep(i64),
    /// Open a new terminal tab.
    NewTab,
    /// Open a new *untracked* tab (⌘⇧T / the tab menu): its commands never
    /// reach encrypted history — suppressed inside the screen itself, and
    /// marked everywhere the tab shows (strip, title, status bar, panel).
    NewUntrackedTab,
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
    /// Switch the Appearance section's sub-tab to `i` (Theme / Tabs / Status bar
    /// / Terminal / Window).
    AppearanceTab(usize),
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
    /// Toggle the auto-hiding status bar (reveals on near-hover at the bottom).
    SetStatusBarAutohide(bool),
    /// Toggle pinning metric popovers open on a click away (several at once).
    SetStatusBarMetricsPinned(bool),
    /// Append a machine-stat cell to the status bar's ordered list, by metric
    /// key (`"cpu"`, `"mem"`).
    StatusBarMetricAdd(String),
    /// Remove the status-bar metric at this list index.
    StatusBarMetricRemove(usize),
    /// Move the status-bar metric at this index by a delta (-1 = left, +1 =
    /// right), clamped to the ends.
    StatusBarMetricMove(usize, i32),
    /// Set the render style (`"sparkline"`, `"number"`) of the status-bar
    /// metric at this index.
    StatusBarMetricStyle(usize, String),
    /// The periodic machine-stats sample tick (only fires while stats are on).
    SampleMetrics,
    /// Open the detail popover for a status-bar metric (clicked its sparkline),
    /// by metric key (`"cpu"`, `"disk_io"`, …).
    OpenMetricDetail(String),
    /// Close all open metric popovers (a click on the background in the default
    /// one-at-a-time mode, or Escape in any mode).
    CloseMetricDetail,
    /// Close the single metric popover at this index (its "×" button, shown when
    /// popovers are pinned).
    CloseMetricPopover(usize),
    /// Toggle the metric popover at this index between the compact card and the
    /// large one (its "+" / "−" affordance).
    ToggleMetricDetailExpanded(usize),
    /// Begin a drag-resize of the metric popover at this index from one of its
    /// edges/corner. Tracked via `PointerMoved` and ended by `PointerReleased`.
    MetricDetailResizeStart(usize, crate::state::ResizeEdge),
    /// Begin a drag-move of the metric popover at this index (its body was
    /// pressed). Tracked via `PointerMoved` and ended by `PointerReleased`.
    MetricDetailMoveStart(usize),
    /// Toggle encrypted, persisted command history. Turning it on opens the
    /// keychain explainer first (nothing is touched until the user
    /// continues); the start itself is async and commits the setting only on
    /// success. Turning it off stops the writer for this session but never
    /// deletes an existing archive.
    SetEncryptedHistoryEnabled(bool),
    /// The enable dialog's "Continue" (keychain source) — actually begin the
    /// (async) start: keychain key + writer thread. The passphrase source
    /// submits via `SubmitHistoryPassphrase` instead; either way, Cancel is
    /// `CancelHistoryPassphrase` and nothing has been touched before this.
    ConfirmEnableHistory,
    /// An async history start finished (see `HistoryStartOrigin` /
    /// `HistoryStartOutcome` for what "finished" means per origin).
    HistoryStarted(HistoryStartOrigin, HistoryStartOutcome),
    /// Pick the history key source (a `settings::KeySource::as_setting_str`
    /// value). Like the cipher: a before-first-enable choice, fixed once the
    /// archive has data.
    SetHistoryKeySource(String),
    /// Pick the passphrase KDF (a `settings::HistoryKdf::as_setting_str`
    /// value). Chooses the recipe for *new* archives only — an existing
    /// archive keeps the recipe recorded in its KDF sidecar.
    SetHistoryKdf(String),
    /// Pick the fan-out PRF (a `settings::HistoryFanout::as_setting_str`
    /// value). Like the cipher: a before-first-enable choice, fixed once the
    /// archive has data.
    SetHistoryFanout(String),
    /// The locked-history banner's "Unlock…" — reopen the passphrase prompt
    /// dismissed earlier this session.
    OpenHistoryUnlock,
    /// The passphrase prompt's main field changed.
    HistoryPassphraseChanged(String),
    /// The passphrase prompt's confirm field changed (enable flow only).
    HistoryPassphraseConfirmChanged(String),
    /// Submit the passphrase prompt — validates inline, then derives the key
    /// and starts (async; `HistoryStarted` carries the result).
    SubmitHistoryPassphrase,
    /// Dismiss the passphrase prompt. Enable: the setting stays off.
    /// Unlock: history stays locked for the session (reopenable from the
    /// settings History section).
    CancelHistoryPassphrase,
    /// The startup "Record this session's commands?" chooser's answer
    /// (`history_session_start == "ask"`): `true` records (starting the
    /// archive now), `false` makes the whole session untracked.
    SessionStartChoice(bool),
    /// Pick the launch behavior (a `settings::SessionStart::as_setting_str`
    /// value) — record / ask / start untracked.
    SetHistorySessionStart(String),
    /// Pick the history cipher (a `history::crypto::Cipher::as_setting_str`
    /// value). Only takes effect the next time the feature starts fresh
    /// (first enable, or a later relaunch) — it does not retroactively
    /// re-encrypt an archive that already has data in it.
    SetHistoryCipher(String),
    /// The History settings section's "Reset…" button — opens the
    /// confirmation dialog (separate from the enable/disable toggle, which
    /// never deletes anything).
    RequestResetEncryptedHistory,
    /// The reset confirmation dialog's "Cancel" action, or a backdrop click.
    CancelResetEncryptedHistory,
    /// The reset confirmation dialog's "Delete" action — permanently removes
    /// the encrypted history archive.
    ConfirmResetEncryptedHistory,
    /// The settings stepper nudged the re-auth idle interval (minutes; `0` =
    /// only the once-per-session gate). macOS only in effect — see
    /// `history::reauth`.
    HistoryReauthIntervalStep(i64),
    /// The async Touch ID/device-password prompt resolved — `true` unlocks
    /// whichever surface requested it ([`ReauthFor`]), `false` (failed or
    /// cancelled) leaves it closed.
    HistoryReauthResult(ReauthFor, bool),
    /// The settings History section's "View archived commands" button — shows
    /// (through the same re-auth gate as the panel) or hides a read-only,
    /// scrollable list of persisted commands.
    ToggleSettingsHistoryViewer,
    /// The settings archive viewer's "Load older day" — page one more day back.
    SettingsHistoryPageOlder,
    /// The settings archive viewer's table was scrolled.
    SettingsHistoryScrolled(f32),
    /// A row in the settings archive viewer was clicked — selects/highlights it.
    SettingsHistoryRowSelected(usize),
    /// A row in the settings archive viewer was double-clicked — selects it
    /// and copies its command text.
    SettingsHistoryRowActivated(usize, String),
    /// A row in the settings archive viewer was right-clicked — opens a
    /// Copy/Delete menu anchored at the cursor. Carries the full
    /// [`ArchivedTarget`] so Delete can address the entry on disk.
    SettingsHistoryRowRightClick(usize, ArchivedTarget),
    /// The settings archive viewer's row menu's "Delete…" item — opens the
    /// per-row confirmation dialog (unlike the panel's immediate Delete, the
    /// viewer confirms first).
    RequestDeleteSettingsHistoryRow(ArchivedTarget),
    /// The row-delete confirmation dialog's "Cancel" action, or a backdrop
    /// click.
    CancelDeleteSettingsHistoryRow,
    /// The row-delete confirmation dialog's "Delete" action — tombstones the
    /// entry via the background writer and drops it from the viewer.
    ConfirmDeleteSettingsHistoryRow,
    /// A context menu's "Copy" for a plain string (the settings archive
    /// viewer's rows) — close the menu, write the text to the clipboard.
    CopyText(String),

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
