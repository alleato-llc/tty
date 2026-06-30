//! `tty` — a minimalist, tabbed terminal. The terminal counterpart of `fed`: thin
//! glue over `cathode` (the PTY + screen engine), `phosphor` (the terminal widget,
//! also used by the IDE's terminal panel), and `rime` chrome.

mod detach_drag;
mod message;
mod settings;
mod state;
mod subscription;
mod theme;
mod update;
mod view;

#[cfg(test)]
mod behavior;
#[cfg(test)]
mod snapshot;

use message::Message;
use state::Tty;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tty=debug".parse().unwrap()),
        )
        .init();

    // tty runs on iced's **`daemon`** model (not `application`) so it can open extra OS
    // windows for **detached tabs** (ADR 0003): a daemon's `view`/`title`/`theme` take a
    // `window::Id`, letting each window render different content. A daemon opens no window
    // on its own, so `boot` opens the main window and records its id as `Tty::main_window`.
    iced::daemon(
        || {
            let mut tty = Tty::new();
            // A transparent surface so the optional unfocused-opacity can show the desktop
            // through. With opacity at 1.0 (the default) the background paints fully
            // opaque, so this is invisible until the user dials in some transparency.
            let size = iced::Size::new(900.0, 600.0);
            let (id, open) = iced::window::open(iced::window::Settings {
                size,
                transparent: true,
                ..Default::default()
            });
            tty.main_window = Some(id);
            detach_drag::on_opened(&mut tty, id, size); // seed bounds for drag-to-dock
                                                        // Open the window, then fetch its real on-screen position (the initial
                                                        // placement never arrives as a `Moved` event, so the drag-dock band would
                                                        // otherwise be pinned to (0,0)).
            let task = open.then(move |id| {
                iced::window::position(id).map(move |p| Message::WindowPosition(id, p))
            });
            (tty, task)
        },
        update::update,
        view::root_view,
    )
    .title(view::title)
    .theme(|state: &Tty, _window| state::theme(state))
    .subscription(subscription::subscription)
    .run()
    .map_err(|e| anyhow::anyhow!("UI error: {e}"))
}
