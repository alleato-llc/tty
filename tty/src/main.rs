//! `tty` — a minimalist, tabbed terminal. The terminal counterpart of `fed`: thin
//! glue over `cathode` (the PTY + screen engine), `phosphor` (the terminal widget,
//! also used by the IDE's terminal panel), and `rime` chrome.

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

use state::Tty;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tty=debug".parse().unwrap()),
        )
        .init();

    iced::application(Tty::new, update::update, view::view)
        .title("tty")
        .theme(|state: &Tty| state::theme(state))
        .subscription(subscription::subscription)
        .window_size(iced::Size::new(900.0, 600.0))
        .run()
        .map_err(|e| anyhow::anyhow!("UI error: {e}"))
}
