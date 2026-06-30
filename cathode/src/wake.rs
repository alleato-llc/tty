//! A process-global "terminal output arrived" signal, so the UI repaints when a shell
//! produces output instead of on a fixed timer.
//!
//! The PTY read/parse threads call [`signal`] after updating a screen (and once more
//! when the shell exits). The host app's subscription takes the receiver once via
//! [`take_receiver`] and awaits it, emitting a redraw message per output burst. A
//! global is acceptable here: there's one app process, and iced subscription builders
//! are non-capturing `fn` pointers, so a captured channel can't be threaded in.

use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

struct Wake {
    tx: UnboundedSender<()>,
    rx: Mutex<Option<UnboundedReceiver<()>>>,
}

static WAKE: OnceLock<Wake> = OnceLock::new();

fn wake() -> &'static Wake {
    WAKE.get_or_init(|| {
        let (tx, rx) = unbounded_channel();
        Wake {
            tx,
            rx: Mutex::new(Some(rx)),
        }
    })
}

/// Signal that terminal output arrived — call after updating a screen so the UI
/// repaints. Cheap and non-blocking; safe from any thread.
pub fn signal() {
    let _ = wake().tx.send(());
}

/// Take the output receiver (once). The host's subscription awaits it; subsequent
/// calls return `None`.
pub fn take_receiver() -> Option<UnboundedReceiver<()>> {
    wake().rx.lock().ok().and_then(|mut g| g.take())
}
