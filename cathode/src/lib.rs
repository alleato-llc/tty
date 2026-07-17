//! cathode — the terminal *engine*: a VT/ANSI [`parser`], the [`screen`] grid model
//! it writes into, a [`pty`] session driving a real shell, and a [`wake`] signal that
//! lets a background reader nudge the UI to repaint.
//!
//! The retro-CRT image: the cathode drives the electron beam; [`phosphor`](../phosphor)
//! is the coating it lights up (the iced widget that renders this screen). cathode is
//! iced-free — it's pure terminal emulation, embeddable by any front-end.

pub mod commands;
pub mod history;
pub mod parser;
pub mod pty;
pub mod screen;
pub mod wake;
