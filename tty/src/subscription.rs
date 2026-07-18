use iced::event::{self, Event};
use iced::futures::SinkExt;
use iced::{keyboard, Subscription};

use crate::message::Message;
use crate::state::Tty;

pub fn subscription(state: &Tty) -> Subscription<Message> {
    // Key presses become terminal input. Prefer the committed `text` for printable
    // keys (so Shift/AltGr produce the right character) and fall back to the logical
    // key for Named keys (Enter/arrows) and Ctrl combos, which `phosphor::input::to_bytes`
    // turns into the right escape/control bytes. Command chords also fall back to the
    // logical key: on macOS `text` is the Option-remapped character (⌥T → "†"), so a
    // ⌥⌘ shortcut must match the base key, not the composed glyph.
    let keys = event::listen_with(|event, status, _window| match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            text,
            modifiers,
            ..
        }) => {
            // A focused widget (the ⌘F find field) consumes its keys → status isn't
            // Ignored → don't also route them to the PTY. ⌘-chords aren't consumed by a
            // text input, so they stay Ignored and still reach the shortcut handler.
            if status != iced::event::Status::Ignored {
                return None;
            }
            let effective = match (&key, &text) {
                (keyboard::Key::Character(_), Some(t))
                    if !t.is_empty() && !modifiers.control() && !modifiers.command() =>
                {
                    keyboard::Key::Character(t.clone())
                }
                _ => key,
            };
            Some(Message::Key(effective, modifiers))
        }
        Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Message::ModifiersChanged(m)),
        // Track the cursor so a right-click can anchor the pane context menu, and a
        // left-button release can complete a tab tear-off drag.
        Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::PointerMoved(position))
        }
        Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(Message::PointerReleased)
        }
        _ => None,
    });

    // Per-window geometry — `listen_with`'s third argument is the originating window id,
    // so focus/resize/move are attributed to the right window. Focus routes the keyboard;
    // resize feeds the status bar's edge band; move drives the drag-to-dock heuristic.
    let window_geom = event::listen_with(|event, _status, window| match event {
        Event::Window(iced::window::Event::Focused) => Some(Message::WindowFocused(window)),
        Event::Window(iced::window::Event::Unfocused) => Some(Message::Focused(false)),
        Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResizedAt(window, size))
        }
        Event::Window(iced::window::Event::Moved(pos)) => Some(Message::WindowMoved(window, pos)),
        _ => None,
    });

    // A window finished closing — exit on the main window, reattach a detached one.
    let window_closed = iced::window::close_events().map(Message::WindowClosed);

    // Repaint when shell output arrives (or a shell exits) — output-driven, no idle
    // polling. The `Tick` message also reaps exited tabs.
    let output = Subscription::run(terminal_output);

    let mut subs = vec![keys, window_geom, window_closed, output];
    // While a detached window is settling after a drag, poll the release debounce to
    // decide whether it docked back. Only armed during a drag, so idle cost stays zero.
    if crate::detach_drag::pending(state) {
        subs.push(
            iced::time::every(std::time::Duration::from_millis(120))
                .map(|_| Message::CheckDragReattach),
        );
    }
    // Machine stats: sample at the configured interval, but only while at least
    // one metric is shown, so idle cost stays zero by default.
    let metrics = state.settings.status_bar_metrics();
    if !metrics.is_empty() {
        let interval =
            std::time::Duration::from_millis(state.settings.status_bar_metrics_interval_ms());
        subs.push(iced::time::every(interval).map(|_| Message::SampleMetrics));
    }
    // A clock cell wants second-level freshness the sample cadence can't give, so
    // tick it once a second (only while one is shown).
    if metrics
        .iter()
        .any(|m| m.kind == crate::settings::MetricKind::Clock)
    {
        subs.push(iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::ClockTick));
    }
    // While a metric-cell press is pending, poll whether it has been held long
    // enough to enter drag-to-reorder edit mode. Only armed during the press, so
    // idle cost stays zero.
    if state.status_metric_press.is_some() {
        subs.push(
            iced::time::every(std::time::Duration::from_millis(120))
                .map(|_| Message::StatusBarEditTick),
        );
    }
    Subscription::batch(subs)
}

/// Stream of redraws fed by `cathode::wake` (the read threads signal on output / shell
/// exit). Coalesces an output burst into a single tick.
fn terminal_output() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(
        16,
        |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            if let Some(mut rx) = cathode::wake::take_receiver() {
                while rx.recv().await.is_some() {
                    while rx.try_recv().is_ok() {} // drain the burst
                    let _ = out.send(Message::Tick).await;
                }
            }
        },
    )
}
