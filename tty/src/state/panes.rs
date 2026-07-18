//! `impl Tty` methods for tabs, panes, and terminal I/O: the active-tab / pane
//! accessors, new-tab / split / focus / resize / close, paste and PTY-output
//! draining, dead-pane reaping, zoom, and the detach / reattach multi-window
//! flows. Split out of `state.rs`; these are just more methods on the same `Tty`.

use iced::widget::pane_grid;

use super::{
    drain_pane, reap_tab_panes, spawn_term, MenuKind, Pane, PaneTabDrag, Tab, Term, Tty,
    DEFAULT_FONT_SIZE, MAX_FONT_SIZE, MIN_FONT_SIZE, TAB_TEAR_THRESHOLD,
};
use crate::message::Message;

impl Tty {
    /// The active tab's focused pane terminal.
    pub fn active_term(&self) -> Option<&Term> {
        self.tabs.get(self.active).and_then(Tab::focused)
    }

    /// The `Tab` a window hosts: the main window shows the active tab; a detached window
    /// shows its own tab. The linchpin for routing window-tagged pane messages.
    pub fn tab_for(&self, window: iced::window::Id) -> Option<&Tab> {
        if self.main_window == Some(window) {
            self.tabs.get(self.active)
        } else {
            self.detached.get(&window)
        }
    }

    /// Mutable [`tab_for`](Self::tab_for).
    pub fn tab_for_mut(&mut self, window: iced::window::Id) -> Option<&mut Tab> {
        if self.main_window == Some(window) {
            self.tabs.get_mut(self.active)
        } else {
            self.detached.get_mut(&window)
        }
    }

    /// The window the keyboard should act on: the focused window, else the main window.
    pub fn keyboard_window(&self) -> Option<iced::window::Id> {
        self.focused_window.or(self.main_window)
    }

    /// The DEC application-cursor-keys mode of `window`'s focused pane (affects arrow
    /// bytes).
    pub fn app_cursor_for(&self, window: iced::window::Id) -> bool {
        self.tab_for(window)
            .and_then(Tab::focused)
            .map(|t| t.screen.lock().app_cursor_keys)
            .unwrap_or(false)
    }

    /// Make tab `idx` active and clear the unseen-activity dot on all its panes (the
    /// whole tab — every pane — becomes visible).
    pub fn activate(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.get_mut(idx) {
            self.active = idx;
            for term in tab.terms_mut() {
                term.activity = false;
            }
        }
    }

    /// Spawn a shell in a new tab and make it active. The new shell starts in the
    /// active pane's reported working directory (OSC 7) when known.
    pub fn new_tab(&mut self) {
        self.new_tab_with(false);
    }

    /// [`Self::new_tab`], optionally untracked — the tab's commands then never
    /// reach encrypted history (suppressed inside the screen itself). In an
    /// untracked *session* every tab is untracked regardless of what the
    /// caller asked for.
    pub fn new_tab_with(&mut self, untracked: bool) {
        let untracked = untracked || self.session_untracked;
        let cwd = self.active_term().and_then(|t| t.screen.lock().cwd.clone());
        let pane_tag = format!("Tab {}", self.tabs.len() + 1);
        if let Some(term) = spawn_term(
            80,
            24,
            cwd.as_deref(),
            self.settings.max_scrollback(),
            &pane_tag,
            self.history_id_floor,
            untracked,
            self.settings.shell_integration(),
            &self.settings.env,
        ) {
            let mut tab = Tab::new(term);
            tab.untracked = untracked;
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
    }

    /// Split `window`'s focused pane toward `dir`, spawning a fresh shell there (seeded
    /// with the focused pane's cwd) and focusing it. Left/Right split the column (vertical
    /// divider); Up/Down split the row (horizontal divider).
    pub fn split_focused(&mut self, window: iced::window::Id, dir: pane_grid::Direction) {
        let tab = self.tab_for(window);
        let cwd = tab
            .and_then(Tab::focused)
            .and_then(|t| t.screen.lock().cwd.clone());
        let pane_tag = tab.map(Tab::label).unwrap_or_else(|| "Tab".to_string());
        // A pane split off an untracked tab is untracked too — the promise is
        // per-tab, not per-pane.
        let untracked = tab.is_some_and(|t| t.untracked);
        if let Some(term) = spawn_term(
            80,
            24,
            cwd.as_deref(),
            self.settings.max_scrollback(),
            &pane_tag,
            self.history_id_floor,
            untracked,
            self.settings.shell_integration(),
            &self.settings.env,
        ) {
            self.split_with(window, dir, Pane::single(term));
        }
    }

    /// Graduate a metric drill-in into a real pane: split `window`'s focused pane
    /// toward `dir` and put a live [`Pane::Metric`] there. Unlike a terminal split
    /// there's no shell to spawn — the metric reads the shared `Metrics`.
    pub fn promote_metric_to_pane(
        &mut self,
        window: iced::window::Id,
        dir: pane_grid::Direction,
        kind: crate::settings::MetricKind,
    ) {
        self.split_with(window, dir, Pane::Metric(kind));
    }

    /// Add a new shell tab to `pane`'s tab group (a single-terminal pane becomes a two-tab
    /// group), make it active, and focus the pane. New shell starts in the pane's active
    /// terminal's cwd, like a split.
    pub fn new_pane_tab(&mut self, window: iced::window::Id, pane: pane_grid::Pane) {
        let tab = self.tab_for(window);
        let cwd = tab
            .and_then(|t| t.panes.get(pane))
            .and_then(Pane::as_term)
            .and_then(|t| t.screen.lock().cwd.clone());
        let pane_tag = tab.map(Tab::label).unwrap_or_else(|| "Tab".to_string());
        let untracked = tab.is_some_and(|t| t.untracked);
        if let Some(term) = spawn_term(
            80,
            24,
            cwd.as_deref(),
            self.settings.max_scrollback(),
            &pane_tag,
            self.history_id_floor,
            untracked,
            self.settings.shell_integration(),
            &self.settings.env,
        ) {
            if let Some(t) = self.tab_for_mut(window) {
                if let Some(g) = t.panes.get_mut(pane).and_then(Pane::group_mut) {
                    g.tabs.push(term);
                    g.active = g.tabs.len() - 1;
                }
                t.focus = pane;
            }
        }
    }

    /// Select tab `idx` in `pane`'s group and focus that pane. Fired on the tab *press*, so
    /// it also arms a drag (mirroring the window strip's tab press): a release without the
    /// pointer crossing another tab is just a plain select.
    pub fn select_pane_tab(&mut self, window: iced::window::Id, pane: pane_grid::Pane, idx: usize) {
        if let Some(t) = self.tab_for_mut(window) {
            if let Some(g) = t.panes.get_mut(pane).and_then(Pane::group_mut) {
                if idx < g.tabs.len() {
                    g.active = idx;
                }
            }
            t.focus = pane;
        }
        let mut reorder = rime::widgets::Reorder::default();
        reorder.begin(idx);
        self.pane_tab_drag = Some(PaneTabDrag {
            window,
            pane,
            reorder,
        });
    }

    /// Pointer moved onto pane-tab `idx` of `pane` (or off the strip, `None`). Tracks hover
    /// for the close affordance and, while a drag is armed, reorders within the group or
    /// moves the dragged tab into this group.
    pub fn hover_pane_tab(
        &mut self,
        window: iced::window::Id,
        pane: pane_grid::Pane,
        idx: Option<usize>,
    ) {
        match idx {
            Some(i) => {
                self.pane_tab_hover = Some((window, pane, i));
                self.reorder_or_move_pane_tab(window, pane, i);
            }
            None => {
                if matches!(self.pane_tab_hover, Some((w, p, _)) if w == window && p == pane) {
                    self.pane_tab_hover = None;
                }
            }
        }
    }

    /// While a pane-tab drag is armed and the pointer entered pane-tab `target` of
    /// `dst_pane`: reorder within the same group, or move the dragged tab across groups
    /// (same window only — a cross-window drag would be a detach, not handled here).
    fn reorder_or_move_pane_tab(
        &mut self,
        window: iced::window::Id,
        dst_pane: pane_grid::Pane,
        target: usize,
    ) {
        let Some(mut drag) = self.pane_tab_drag else {
            return;
        };
        if drag.window != window {
            return;
        }
        if drag.pane == dst_pane {
            if let Some((from, to)) = drag.reorder.drag_to(target) {
                if let Some(g) = self
                    .tab_for_mut(window)
                    .and_then(|t| t.panes.get_mut(dst_pane))
                    .and_then(Pane::group_mut)
                {
                    rime::widgets::reorder_slice(&mut g.tabs, from, to);
                    g.active = to;
                }
            }
        } else {
            self.move_pane_tab_across(&mut drag, dst_pane, target);
        }
        self.pane_tab_drag = Some(drag);
    }

    /// Move the dragged tab out of its source group and into `dst_pane`'s group at
    /// `target`, closing the source pane if it empties. Updates `drag` to follow the tab
    /// into its new home.
    fn move_pane_tab_across(
        &mut self,
        drag: &mut PaneTabDrag,
        dst_pane: pane_grid::Pane,
        target: usize,
    ) {
        let window = drag.window;
        let src_pane = drag.pane;
        let Some(from) = drag.reorder.anchor() else {
            return;
        };
        let Some(t) = self.tab_for_mut(window) else {
            return;
        };
        // Take the terminal out of the source group.
        let taken = t
            .panes
            .get_mut(src_pane)
            .and_then(Pane::group_mut)
            .and_then(|g| {
                (from < g.tabs.len()).then(|| {
                    let term = g.tabs.remove(from);
                    g.active = g.active.min(g.tabs.len().saturating_sub(1));
                    term
                })
            });
        let Some(term) = taken else {
            return;
        };
        // Insert into the destination group (or, if it isn't a terminal group, put it
        // back where it came from — a metric pane can't hold shell tabs).
        let Some(g) = t.panes.get_mut(dst_pane).and_then(Pane::group_mut) else {
            if let Some(g) = t.panes.get_mut(src_pane).and_then(Pane::group_mut) {
                let at = from.min(g.tabs.len());
                g.tabs.insert(at, term);
            }
            return;
        };
        let at = target.min(g.tabs.len());
        g.tabs.insert(at, term);
        g.active = at;
        t.focus = dst_pane;
        // A source group that emptied leaves a stale pane behind — close it.
        if t.panes
            .get(src_pane)
            .and_then(Pane::group)
            .is_some_and(|g| g.tabs.is_empty())
        {
            t.panes.close(src_pane);
        }
        drag.pane = dst_pane;
        let mut reorder = rime::widgets::Reorder::default();
        reorder.begin(at);
        drag.reorder = reorder;
    }

    /// Open the pane-tab context menu (new / rename / close): select the tab, then anchor
    /// at the cursor. Right-click isn't a drag, so the drag armed by the select is cleared.
    pub fn open_pane_tab_menu(
        &mut self,
        window: iced::window::Id,
        pane: pane_grid::Pane,
        idx: usize,
    ) {
        self.select_pane_tab(window, pane, idx);
        self.pane_tab_drag = None;
        self.menu = Some((MenuKind::PaneTab { window, pane, idx }, self.pointer));
    }

    /// Add a shell tab to the *focused* pane (the `⌥⌘T` chord).
    pub fn new_focused_pane_tab(&mut self, window: iced::window::Id) {
        if let Some(pane) = self.tab_for(window).map(|t| t.focus) {
            self.new_pane_tab(window, pane);
        }
    }

    /// Close the focused pane's active tab — but only when the pane holds more than one
    /// (so `⌥⌘W` doesn't step on `⌘W`, which closes the whole pane).
    pub fn close_focused_pane_tab(&mut self, window: iced::window::Id) {
        let target = self.tab_for(window).and_then(|t| {
            let pane = t.focus;
            t.panes
                .get(pane)
                .and_then(Pane::group)
                .filter(|g| g.tabs.len() > 1)
                .map(|g| (pane, g.active_idx()))
        });
        if let Some((pane, idx)) = target {
            self.close_pane_tab(window, pane, idx);
        }
    }

    /// Cycle the focused pane's tab group by `delta` (wrapping). No-op for a single-tab pane.
    pub fn cycle_pane_tab(&mut self, window: iced::window::Id, delta: isize) {
        if let Some(t) = self.tab_for_mut(window) {
            let focus = t.focus;
            if let Some(g) = t.panes.get_mut(focus).and_then(Pane::group_mut) {
                let n = g.tabs.len();
                if n > 1 {
                    let cur = g.active_idx() as isize;
                    g.active = (cur + delta).rem_euclid(n as isize) as usize;
                }
            }
        }
    }

    /// Close tab `idx` in `pane`'s group (dropping its terminal + PTY). If it was the last
    /// tab, close the whole pane and focus a sibling.
    pub fn close_pane_tab(&mut self, window: iced::window::Id, pane: pane_grid::Pane, idx: usize) {
        if let Some(t) = self.tab_for_mut(window) {
            let empty = if let Some(g) = t.panes.get_mut(pane).and_then(Pane::group_mut) {
                if idx < g.tabs.len() {
                    g.tabs.remove(idx); // drops the Term (and its PtySession)
                    if g.active > idx {
                        g.active -= 1;
                    }
                    g.active = g.active.min(g.tabs.len().saturating_sub(1));
                }
                g.tabs.is_empty()
            } else {
                false
            };
            if empty {
                if let Some((_removed, sibling)) = t.panes.close(pane) {
                    t.focus = sibling;
                }
            } else {
                t.focus = pane;
            }
        }
    }

    /// Place `content` as a new pane split off `window`'s focused pane toward `dir`,
    /// and focus it. (The spawn-free core of [`split_focused`], so a metric pane or a
    /// pty-less test pane can be inserted directly.)
    pub fn split_with(
        &mut self,
        window: iced::window::Id,
        dir: pane_grid::Direction,
        content: Pane,
    ) {
        use pane_grid::{Axis, Direction};
        let axis = match dir {
            Direction::Left | Direction::Right => Axis::Vertical,
            Direction::Up | Direction::Down => Axis::Horizontal,
        };
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some((new_pane, _split)) = tab.panes.split(axis, tab.focus, content) {
                // `split` always places the newcomer after the target (right/below); for
                // Left/Up, swap so the new pane lands on the requested side.
                if matches!(dir, Direction::Left | Direction::Up) {
                    tab.panes.swap(tab.focus, new_pane);
                }
                tab.focus = new_pane;
            }
        }
    }

    /// Toggle whether `window`'s focused pane fills the whole grid (maximize /
    /// restore). iced's `pane_grid` tracks a single maximized pane per tab.
    pub fn toggle_maximize_pane(&mut self, window: iced::window::Id) {
        if let Some(tab) = self.tab_for_mut(window) {
            if tab.panes.maximized().is_some() {
                tab.panes.restore();
            } else {
                tab.panes.maximize(tab.focus);
            }
        }
    }

    /// Close a specific pane in `window`'s tab (its own × control), re-pointing
    /// focus to a survivor. Returns `false` if it was the tab's last pane (the
    /// caller decides whether to close the tab).
    pub fn close_pane(&mut self, window: iced::window::Id, pane: pane_grid::Pane) -> bool {
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some((_content, sibling)) = tab.panes.close(pane) {
                tab.focus = sibling;
                return true;
            }
        }
        false
    }

    /// Move focus to the neighbouring pane in `dir` within `window`'s tab (no-op at the
    /// edge).
    pub fn focus_dir(&mut self, window: iced::window::Id, dir: pane_grid::Direction) {
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some(p) = tab.panes.adjacent(tab.focus, dir) {
                tab.focus = p;
            }
        }
    }

    /// Focus a specific pane in `window`'s tab (a click landed on it).
    pub fn focus_pane(&mut self, window: iced::window::Id, pane: pane_grid::Pane) {
        if let Some(tab) = self.tab_for_mut(window) {
            tab.focus = pane;
        }
    }

    /// Drag-resize the divider at `split` to `ratio` (0..=1) in `window`'s tab.
    pub fn resize_split(&mut self, window: iced::window::Id, split: pane_grid::Split, ratio: f32) {
        if let Some(tab) = self.tab_for_mut(window) {
            tab.panes.resize(split, ratio);
        }
    }

    /// Close the active tab's focused pane. Closing the last pane in a tab closes the
    /// tab; closing the last tab returns `false` (the caller exits). The dropped `Term`
    /// drops its `PtySession`, ending that shell.
    pub fn close_focused_pane(&mut self) -> bool {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if let Some((_term, sibling)) = tab.panes.close(tab.focus) {
                tab.focus = sibling;
                return true;
            }
        }
        // It was the tab's only pane → close the whole tab.
        self.close_tab(self.active)
    }

    /// Paste `text` into `window`'s focused shell, wrapping it in bracketed-paste markers
    /// when the app enabled mode 2004 (so a compliant shell holds embedded newlines as
    /// literal text in one edit buffer instead of auto-executing each line).
    ///
    /// Without that (`bracketed` false), the destination can't tell paste apart from
    /// typing — every embedded newline runs immediately as its own command, exactly
    /// like a real Enter, just not through one. So each complete pasted line queues its
    /// own Scrollback History boundary *before* any of it is sent, using the
    /// already-known line text (there's nothing to read off the terminal grid yet — see
    /// `TerminalScreen::mark_command_boundary_with`). A final line with no trailing
    /// newline is left alone: it hasn't been "entered," the same as normal typing.
    pub fn paste(&mut self, window: iced::window::Id, text: &str) {
        let bracketed = self
            .tab_for(window)
            .and_then(Tab::focused)
            .map(|t| t.screen.lock().bracketed_paste)
            .unwrap_or(false);

        if !bracketed {
            if let Some(term) = self.tab_for(window).and_then(Tab::focused) {
                let mut screen = term.screen.lock();
                let mut lines: Vec<&str> = text.split('\n').collect();
                if !text.ends_with('\n') {
                    lines.pop(); // an unterminated final fragment — nothing to mark yet
                }
                for line in lines {
                    let line = line.strip_suffix('\r').unwrap_or(line);
                    if line.is_empty() {
                        continue;
                    }
                    let cap = self.settings.resolve_output_cap(line);
                    screen.mark_command_boundary_with(line.to_string(), cap);
                }
            }
        }

        let mut bytes = Vec::new();
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        self.write_focused(window, &bytes);
    }

    /// Per-redraw housekeeping: light activity dots on background tabs that produced
    /// output or rang the bell, clear the active tab's dot, and surface any OSC 52
    /// clipboard-write request for the host to put on the system clipboard. Walks both
    /// the main strip and the detached windows (a detached tab is always on-screen in its
    /// own window, so it never carries a dot).
    pub fn drain_effects(&mut self) -> Option<String> {
        let active = self.active;
        let writer = self.history_writer.as_ref();
        // Notify only for commands that finished while the window is unfocused and ran
        // past the threshold — the "I walked away, tell me when it's done" case.
        let si = self.settings.shell_integration();
        let notify = si.notify && !self.focused;
        let threshold = std::time::Duration::from_secs(si.notify_min_seconds as u64);
        let mut clip = None;
        let mut completions = Vec::new();
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            for term in tab.terms_mut() {
                let (signal, requested, done) = drain_pane(term, writer);
                completions.extend(done);
                if let Some(c) = requested {
                    clip = Some(c);
                }
                // Every pane of the active tab is on screen, so it never carries a dot;
                // a background tab's panes light one on output or a bell.
                if i == active {
                    term.activity = false;
                } else if signal {
                    term.activity = true;
                }
            }
        }
        for tab in self.detached.values_mut() {
            for term in tab.terms_mut() {
                let (_signal, requested, done) = drain_pane(term, writer);
                completions.extend(done);
                if let Some(c) = requested {
                    clip = Some(c);
                }
                term.activity = false;
            }
        }
        if notify {
            for c in completions {
                if c.duration >= threshold {
                    crate::notify::command_finished(&c);
                }
            }
        }
        clip
    }

    /// Close tab `idx`. Returns `false` when the last tab closes (the caller exits).
    pub fn close_tab(&mut self, idx: usize) -> bool {
        if idx < self.tabs.len() {
            self.tabs.remove(idx);
        }
        if self.tabs.is_empty() {
            return false;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        true
    }

    /// Forward `bytes` to a specific pane's shell in `window`'s tab (mouse reporting
    /// targets the pane under the cursor).
    pub fn write_pane(&mut self, window: iced::window::Id, pane: pane_grid::Pane, bytes: &[u8]) {
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some(term) = tab.panes.get_mut(pane).and_then(Pane::as_term_mut) {
                if let Some(pty) = term.pty.as_mut() {
                    if let Err(e) = pty.write_bytes(bytes) {
                        tracing::warn!("PTY write failed: {e}");
                    }
                }
            }
        }
    }

    /// Forward `bytes` to `window`'s focused pane (keyboard / paste).
    pub fn write_focused(&mut self, window: iced::window::Id, bytes: &[u8]) {
        if let Some(focus) = self.tab_for(window).map(|t| t.focus) {
            self.write_pane(window, focus, bytes);
        }
    }

    /// Resize one pane's grid + PTY (SIGWINCH) to what its widget reports fits, in
    /// `window`'s tab.
    pub fn resize_pane(
        &mut self,
        window: iced::window::Id,
        pane: pane_grid::Pane,
        cols: usize,
        rows: usize,
    ) {
        if let Some(tab) = self.tab_for(window) {
            if let Some(term) = tab.panes.get(pane).and_then(Pane::as_term) {
                term.screen.lock().resize(cols, rows);
                if let Some(pty) = term.pty.as_ref() {
                    let _ = pty.resize(cols as u16, rows as u16);
                }
            }
        }
    }

    /// Adjust the font size (⌘+/⌘−/⌘0), clamped. The widget re-measures the grid on
    /// the next event and resizes the PTY.
    pub fn zoom(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    }

    pub fn reset_zoom(&mut self) {
        self.font_size = DEFAULT_FONT_SIZE;
    }

    /// Drop panes whose shell has exited, then any tab left with no live pane — across
    /// both the main strip and the detached windows. Returns `(any_tabs_remain, windows
    /// to close)`: the caller closes each dead detached window's OS window, and exits when
    /// no tabs remain anywhere. Keeps focus + active valid.
    pub fn reap_dead(&mut self) -> (bool, Vec<iced::window::Id>) {
        let active_alive = self.tabs.get(self.active).is_some_and(Tab::has_live_pane);
        for tab in self.tabs.iter_mut() {
            reap_tab_panes(tab);
        }
        self.tabs.retain(Tab::has_live_pane);

        // Detached tabs: reap their panes, then collect windows whose tab fully died.
        let mut dead_windows = Vec::new();
        for (win, tab) in self.detached.iter_mut() {
            reap_tab_panes(tab);
            if !tab.has_live_pane() {
                dead_windows.push(*win);
            }
        }
        for win in &dead_windows {
            self.detached.remove(win);
            self.detach_origin.remove(win);
            self.window_bounds.remove(win);
        }

        let any = !self.tabs.is_empty() || !self.detached.is_empty();
        // If the active tab died, fall back to the last; otherwise just clamp.
        if !self.tabs.is_empty() && (!active_alive || self.active >= self.tabs.len()) {
            self.active = self.tabs.len() - 1;
        }
        (any, dead_windows)
    }

    // ---- detach / reattach (ADR 0003) ----

    /// Detach the main strip's tab `idx` into its own OS window. The owned `Tab` moves
    /// into `detached`; if that would empty the main strip, a fresh shell tab is spawned
    /// so the main window is never empty. Returns the task that opens the window (and
    /// fetches both windows' positions to align the drag-dock band).
    pub fn detach_tab(&mut self, idx: usize) -> Option<iced::Task<Message>> {
        self.menu = None;
        if idx >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.new_tab();
        } else {
            self.active = self.active.min(self.tabs.len() - 1);
        }
        Some(self.open_detached_window(tab, idx))
    }

    /// Open a new OS window hosting `tab`, recording `origin` as the main-strip index it
    /// docks back to on reattach. Returns the window-open + position-sync task (both
    /// windows' positions feed the drag-dock band). The single detach primitive behind
    /// both the top-level [`detach_tab`](Self::detach_tab) and a pane-tab tear-off.
    fn open_detached_window(&mut self, tab: Tab, origin: usize) -> iced::Task<Message> {
        let size = iced::Size::new(720.0, 600.0);
        let (id, open) = iced::window::open(iced::window::Settings {
            size,
            // A detached window inherits the app's always-on-top setting.
            level: self.window_level(),
            ..Default::default()
        });
        self.detached.insert(id, tab);
        self.detach_origin.insert(id, origin);
        crate::detach_drag::on_opened(self, id, size);
        let open = open.then(move |id| {
            iced::window::position(id).map(move |p| Message::WindowPosition(id, p))
        });
        match self.main_window {
            Some(main) => iced::Task::batch([
                open,
                iced::window::position(main).map(move |p| Message::WindowPosition(main, p)),
            ]),
            None => open,
        }
    }

    /// Detach terminal tab `idx` out of `pane`'s group into its own OS window (as a
    /// one-pane tab). An emptied source pane closes; the untracked promise follows the
    /// terminal. Docks onto the end of the main strip on reattach.
    pub fn detach_pane_tab(
        &mut self,
        window: iced::window::Id,
        pane: pane_grid::Pane,
        idx: usize,
    ) -> Option<iced::Task<Message>> {
        self.menu = None;
        self.pane_tab_drag = None;
        let untracked = self.tab_for(window).is_some_and(|t| t.untracked);
        let t = self.tab_for_mut(window)?;
        let term = t
            .panes
            .get_mut(pane)
            .and_then(Pane::group_mut)
            .and_then(|g| {
                (idx < g.tabs.len()).then(|| {
                    let term = g.tabs.remove(idx);
                    g.active = g.active.min(g.tabs.len().saturating_sub(1));
                    term
                })
            })?;
        // Close the source pane if its group emptied.
        if t.panes
            .get(pane)
            .and_then(Pane::group)
            .is_some_and(|g| g.tabs.is_empty())
        {
            if let Some((_removed, sibling)) = t.panes.close(pane) {
                t.focus = sibling;
            }
        }
        let mut tab = Tab::new(term);
        tab.untracked = untracked;
        let origin = self.tabs.len();
        Some(self.open_detached_window(tab, origin))
    }

    /// Dock a detached window's tab back into the main strip at its origin index.
    pub fn reattach_window(&mut self, window: iced::window::Id) {
        if let Some(tab) = self.detached.remove(&window) {
            let at = self
                .detach_origin
                .remove(&window)
                .unwrap_or(usize::MAX)
                .min(self.tabs.len());
            self.tabs.insert(at, tab);
            self.active = at;
            self.window_bounds.remove(&window);
        }
    }

    /// While a tab tear-off is armed, dragging the pointer over a *different* tab
    /// live-reorders the dragged tab to that slot (browser-style). The drag anchor
    /// follows so successive crossings keep moving it; no-op when not dragging.
    pub fn reorder_dragged_tab(&mut self, target: usize) {
        let Some((from, start)) = self.tab_drag else {
            return;
        };
        if from == target || from >= self.tabs.len() || target >= self.tabs.len() {
            return;
        }
        rime::widgets::reorder_slice(&mut self.tabs, from, target);
        self.active = target;
        self.tab_drag = Some((target, start));
    }

    /// Complete an armed tab tear-off on pointer release: a drag past
    /// [`TAB_TEAR_THRESHOLD`] detaches the pressed tab; a short drag is just a click.
    pub fn finish_tab_drag(&mut self) -> Option<iced::Task<Message>> {
        let (idx, start) = self.tab_drag.take()?;
        if self.pointer.y - start.y > TAB_TEAR_THRESHOLD {
            self.detach_tab(idx)
        } else {
            None
        }
    }

    /// Record which window has the keyboard (chords/typing route to its tab).
    pub fn focus_window(&mut self, window: iced::window::Id) {
        self.focused_window = Some(window);
    }

    /// Close the focused pane of a detached `window`. Returns `Some(window)` to close
    /// when its last pane went — the tab is removed from `detached` *first*, so the
    /// ensuing `WindowClosed` no-ops instead of reattaching (⌘W through the last pane
    /// kills the window; an OS-close reattaches). `None` means the pane closed in place.
    pub fn close_detached_focused_pane(
        &mut self,
        window: iced::window::Id,
    ) -> Option<iced::window::Id> {
        let tab = self.detached.get_mut(&window)?;
        if let Some((_term, sibling)) = tab.panes.close(tab.focus) {
            tab.focus = sibling;
            return None;
        }
        self.detached.remove(&window);
        self.detach_origin.remove(&window);
        self.window_bounds.remove(&window);
        Some(window)
    }
}
