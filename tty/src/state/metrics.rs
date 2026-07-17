//! `impl Tty` methods for the status-bar metrics: the cell editor
//! (add/remove/reorder/style/thresholds/hold), the press-hold reorder
//! interaction, the Processes sort/scroll/detail, opening drill-ins, and the
//! graduate- / replace-a-pane flows. Split out of `state.rs`; these are just more
//! methods on the same `Tty`.

use iced::widget::pane_grid;

use super::{MetricPopover, Pane, ProcSortColumn, Tab, Tty};

impl Tty {
    /// Add a metric to the status bar's ordered list (persisted). The first
    /// metric added takes a sample immediately so numbers appear without waiting
    /// for the next tick (that sample is the CPU% baseline; a real percentage
    /// lands one tick later).
    pub fn add_status_bar_metric(&mut self, metric: &str) {
        let was_empty = self.settings.status_bar_metrics().is_empty();
        if self.settings.add_status_bar_metric(metric) {
            self.settings.save();
            if was_empty {
                self.metrics.sample();
            }
        }
    }

    /// Remove the status-bar metric at `idx` (persisted). Emptying the list
    /// resets the sampler so its history starts clean if re-enabled.
    pub fn remove_status_bar_metric(&mut self, idx: usize) {
        self.settings.remove_status_bar_metric(idx);
        self.settings.save();
        if self.settings.status_bar_metrics().is_empty() {
            self.metrics = crate::metrics::Metrics::default();
        }
    }

    /// Reorder the status-bar metric at `idx` by `delta` (persisted).
    pub fn move_status_bar_metric(&mut self, idx: usize, delta: i32) {
        self.settings.move_status_bar_metric(idx, delta);
        self.settings.save();
    }

    /// Set the render style of the status-bar metric at `idx` (persisted).
    pub fn set_status_bar_metric_style(&mut self, idx: usize, style: &str) {
        self.settings.set_status_bar_metric_style(idx, style);
        self.settings.save();
    }

    /// Nudge a graded metric's caution/alarm threshold (persisted).
    pub fn step_status_bar_metric_threshold(&mut self, idx: usize, warn: bool, delta: f64) {
        self.settings
            .step_status_bar_metric_threshold(idx, warn, delta);
        self.settings.save();
    }

    /// Nudge the edit-mode long-press hold duration by `delta` seconds
    /// (persisted, clamped to the allowed range).
    pub fn step_status_bar_edit_hold(&mut self, delta: f32) {
        let next = (self.settings.status_bar_edit_hold_secs() + delta).clamp(
            crate::settings::MIN_EDIT_HOLD_SECS,
            crate::settings::MAX_EDIT_HOLD_SECS,
        );
        self.settings.status_bar_edit_hold_secs = Some(next);
        self.settings.save();
    }

    /// A metric cell was pressed (config index `idx`). Already editing → begin
    /// dragging it at once; otherwise arm a pending press that either opens the
    /// cell's drill-in on a quick release or, held past the configured duration,
    /// enters edit mode and starts a drag (see [`Self::check_status_metric_hold`]).
    pub fn press_status_metric(&mut self, idx: usize) {
        if self.status_bar_edit {
            self.status_metric_drag = Some(idx);
            self.status_metric_drop = Some(idx);
        } else {
            self.status_metric_press = Some((idx, std::time::Instant::now()));
        }
    }

    /// While a press is pending, enter edit mode + start dragging once it has been
    /// held for the configured duration. Called from the periodic tick.
    pub fn check_status_metric_hold(&mut self) {
        if let Some((idx, started)) = self.status_metric_press {
            if started.elapsed().as_secs_f32() >= self.settings.status_bar_edit_hold_secs() {
                self.status_bar_edit = true;
                self.status_metric_drag = Some(idx);
                self.status_metric_drop = Some(idx);
                self.status_metric_press = None;
            }
        }
    }

    /// While dragging, the pointer entered the cell at config index `target`:
    /// mark it the drop position (where the insertion bar shows). The reorder is
    /// committed on release, not live.
    pub fn drag_status_metric_over(&mut self, target: usize) {
        if self.status_metric_drag.is_some() {
            self.status_metric_drop = Some(target);
        }
    }

    /// Pointer release: commit a metric drag's reorder, or — if it was a quick tap
    /// rather than a hold — return the config index whose drill-in should open.
    pub fn release_status_metric(&mut self) -> Option<usize> {
        if let Some(from) = self.status_metric_drag.take() {
            let target = self.status_metric_drop.take().unwrap_or(from);
            let list = &mut self.settings.status_bar_metrics;
            if from != target && from < list.len() && target < list.len() {
                let item = list.remove(from);
                list.insert(target, item);
                self.settings.save();
            }
            None
        } else {
            self.status_metric_press.take().map(|(idx, _)| idx)
        }
    }

    /// Re-sort the Processes drill-in by `column`: clicking a new column sorts by
    /// it (descending for the numeric columns, ascending for the name); clicking
    /// the active column flips the direction.
    pub fn set_proc_sort(&mut self, column: ProcSortColumn) {
        if self.proc_sort.0 == column {
            self.proc_sort.1 = !self.proc_sort.1;
        } else {
            let descending = column != ProcSortColumn::Name;
            self.proc_sort = (column, descending);
        }
        self.proc_table_scroll = 0.0;
    }

    /// Set the Processes table's scroll offset (px), clamped non-negative.
    pub fn set_proc_scroll(&mut self, offset: f32) {
        self.proc_table_scroll = offset.max(0.0);
    }

    /// Open the per-process detail view for `pid` within the Processes drill-in.
    /// The live sample is refreshed by the update loop; here we just flag which
    /// process to show and reset the (now-hidden) table scroll.
    pub fn open_proc_detail(&mut self, pid: i32) {
        self.proc_detail_pid = Some(pid);
    }

    /// Return the Processes drill-in from a per-process detail back to the list,
    /// dropping the retained per-process CPU history.
    pub fn close_proc_detail(&mut self) {
        self.proc_detail_pid = None;
        self.metrics.proc_detail = None;
    }

    /// Leave drag-to-reorder edit mode (Escape or a press on empty bar space).
    pub fn exit_status_bar_edit(&mut self) {
        self.status_bar_edit = false;
        self.status_metric_press = None;
        self.status_metric_drag = None;
        self.status_metric_drop = None;
    }

    /// Open the drill-in popover for the metric named `metric` (a `MetricKind`
    /// setting string). Pinned mode accumulates open popovers (deduped by kind);
    /// otherwise it replaces whatever was open. Called on a status-bar cell tap.
    pub fn open_metric_detail(&mut self, metric: &str) {
        let Some(kind) = crate::settings::MetricKind::from_setting_str(metric) else {
            return;
        };
        // Don't open a drill-in for a metric that's already on screen — as another
        // popover, or graduated into a pane. A click on a *different* metric's cell
        // still opens (in one-at-a-time mode it replaces the current popover).
        if self.metric_is_shown(kind) {
            return;
        }
        let pop = MetricPopover::new(kind);
        if self.settings.status_bar_metrics_pinned() {
            self.metric_details.push(pop);
        } else {
            self.metric_details = vec![pop];
        }
    }

    /// Whether `kind` is already visible — an open popover, or a metric pane in the
    /// active tab or any detached window.
    fn metric_is_shown(&self, kind: crate::settings::MetricKind) -> bool {
        if self.metric_details.iter().any(|p| p.kind == kind) {
            return true;
        }
        let in_tab = |tab: &Tab| {
            tab.panes
                .iter()
                .any(|(_, p)| matches!(p, Pane::Metric(k) if *k == kind))
        };
        self.tabs.get(self.active).is_some_and(in_tab) || self.detached.values().any(in_tab)
    }

    /// Enter "replace a pane" pick mode for `kind`: the next pane click replaces
    /// that pane with this metric (see [`crate::view`] for the dimmed overlay).
    /// Closes any open popover so the panes are unobstructed.
    pub fn start_pane_replace(&mut self, kind: crate::settings::MetricKind) {
        self.metric_details.clear();
        self.pane_replace_pending = Some(kind);
    }

    /// A pane was clicked while in replace-pick mode: if it holds a live shell,
    /// stage a confirm dialog (replacing ends the shell); otherwise replace it now.
    pub fn request_pane_replace(&mut self, window: iced::window::Id, pane: pane_grid::Pane) {
        let Some(kind) = self.pane_replace_pending.take() else {
            return;
        };
        // A terminal pane has a shell + scrollback to lose, so confirm first; a
        // metric pane has nothing to terminate, so replace it outright.
        let is_terminal = self
            .tab_for(window)
            .and_then(|t| t.panes.get(pane))
            .is_some_and(|p| matches!(p, Pane::Term(_)));
        if is_terminal {
            self.pane_replace_confirm = Some((window, pane, kind));
        } else {
            self.replace_pane(window, pane, kind);
        }
    }

    /// Swap a pane's content for a metric view in place. The old content is
    /// dropped — a terminal's `PtySession` goes with it, ending that shell.
    pub fn replace_pane(
        &mut self,
        window: iced::window::Id,
        pane: pane_grid::Pane,
        kind: crate::settings::MetricKind,
    ) {
        if let Some(tab) = self.tab_for_mut(window) {
            if let Some(slot) = tab.panes.get_mut(pane) {
                *slot = Pane::Metric(kind);
                tab.focus = pane;
            }
        }
    }

    /// Leave replace-pick mode / dismiss the confirm without replacing.
    pub fn cancel_pane_replace(&mut self) {
        self.pane_replace_pending = None;
        self.pane_replace_confirm = None;
    }
}
