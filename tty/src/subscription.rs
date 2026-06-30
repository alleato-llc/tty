use iced::event::{self, Event};
use iced::futures::SinkExt;
use iced::{keyboard, Subscription};

use crate::message::Message;
use crate::state::Tty;

pub fn subscription(_state: &Tty) -> Subscription<Message> {
    // Key presses become terminal input. Prefer the committed `text` for printable
    // keys (so Shift/AltGr produce the right character) and fall back to the logical
    // key for Named keys (Enter/arrows) and Ctrl combos, which `phosphor::input::to_bytes`
    // turns into the right escape/control bytes.
    let keys = event::listen_with(|event, _status, _window| match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            text,
            modifiers,
            ..
        }) => {
            let effective = match (&key, &text) {
                (keyboard::Key::Character(_), Some(t)) if !t.is_empty() && !modifiers.control() => {
                    keyboard::Key::Character(t.clone())
                }
                _ => key,
            };
            Some(Message::Key(effective, modifiers))
        }
        Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Message::ModifiersChanged(m)),
        Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.height))
        }
        _ => None,
    });

    // Repaint when shell output arrives (or a shell exits) — output-driven, no idle
    // polling. The `Tick` message also reaps exited tabs.
    let output = Subscription::run(terminal_output);

    Subscription::batch([keys, output])
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
