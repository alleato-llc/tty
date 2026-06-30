//! Experimental drag-to-reattach for detached tab windows (ADR 0003).
//!
//! iced exposes no cross-window drag-and-drop and no "window dropped here" event, so
//! this is a best-effort heuristic: track each window's outer bounds, and when a
//! detached window stops moving (a debounced drag-release) with its title over the main
//! window's tab-bar band, dock its tab back. The reliable paths — the **Reattach**
//! button and reattach-on-close — stand on their own, so this whole module (plus the
//! `window_bounds`/`last_detached_move` fields it drives) can be deleted as a unit if
//! the heuristic proves too fiddly. Ported from fed-ide's `detach_drag.rs`.

use std::time::{Duration, Instant};

use iced::{Point, Rectangle, Size};

use crate::state::Tty;

/// Treat a drag as released after this idle gap with no further `Moved` event.
const SETTLE: Duration = Duration::from_millis(300);

/// Height of the reattach drop zone — the main window's "tab-bar band", measured from
/// the window's top. Generously covers the OS title bar (~28px) plus the tab strip so
/// dropping a detached window's title onto the strip docks it, like browser tabs.
const DROP_BAND: f32 = 96.0;

/// Seed a freshly-opened window's bounds (position is unknown until its first `Moved`,
/// which is fine — the heuristic only fires after a drag).
pub fn on_opened(state: &mut Tty, window: iced::window::Id, size: Size) {
    state.window_bounds.insert(
        window,
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: size.width,
            height: size.height,
        },
    );
}

/// Record a window moving. For a detached window this (re)arms the release debounce.
pub fn on_moved(state: &mut Tty, window: iced::window::Id, pos: Point) {
    let b = state.window_bounds.entry(window).or_insert(Rectangle {
        x: pos.x,
        y: pos.y,
        width: 0.0,
        height: 0.0,
    });
    b.x = pos.x;
    b.y = pos.y;
    if state.detached.contains_key(&window) {
        state.last_detached_move = Some((window, Instant::now()));
    }
}

/// Set a window's known on-screen position (from a `window::position` fetch), without
/// arming the drag-release debounce — used to learn the *initial* placement that
/// `Moved` never reports.
pub fn set_position(state: &mut Tty, window: iced::window::Id, pos: Point) {
    let b = state.window_bounds.entry(window).or_insert(Rectangle {
        x: pos.x,
        y: pos.y,
        width: 0.0,
        height: 0.0,
    });
    b.x = pos.x;
    b.y = pos.y;
}

/// Record a window's new size.
pub fn on_resized(state: &mut Tty, window: iced::window::Id, size: Size) {
    let b = state.window_bounds.entry(window).or_insert(Rectangle {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    });
    b.width = size.width;
    b.height = size.height;
}

/// Whether a drag-release debounce is armed (drives the subscription poll).
pub fn pending(state: &Tty) -> bool {
    state.last_detached_move.is_some()
}

/// The outcome of polling a detached window's drag-release debounce.
pub enum Settle {
    /// Still moving (or nothing dragging) — keep polling.
    Pending,
    /// Dropped onto the main window's tab-bar band — reattach this window.
    Reattach(iced::window::Id),
    /// Settled elsewhere — the window just moved (nothing to persist; tty sessions are
    /// ephemeral, so unlike fed this is a no-op for the caller).
    Repositioned,
}

/// Poll the drag-release debounce: once a detached window has stopped moving for
/// [`SETTLE`], decide whether it docked or merely repositioned. The "drop point" is the
/// detached window's **top-center** (the title bar you drag), tested against the main
/// window's [`DROP_BAND`] (full width, from the top) — so docking anywhere along the
/// strip works, like dragging a browser tab back.
pub fn poll_settle(state: &mut Tty) -> Settle {
    let Some((window, at)) = state.last_detached_move else {
        return Settle::Pending;
    };
    if Instant::now().duration_since(at) < SETTLE {
        return Settle::Pending;
    }
    state.last_detached_move = None;
    let over_main = (|| {
        let main = *state.window_bounds.get(&state.main_window?)?;
        let d = *state.window_bounds.get(&window)?;
        let drop = Point::new(d.x + d.width / 2.0, d.y);
        let band = Rectangle {
            x: main.x,
            y: main.y,
            width: main.width,
            height: DROP_BAND,
        };
        Some(band.contains(drop))
    })()
    .unwrap_or(false);
    if over_main {
        Settle::Reattach(window)
    } else {
        Settle::Repositioned
    }
}
