//! phosphor — the terminal *widget*: a stateful iced widget that renders a
//! [`cathode`] [`TerminalScreen`](cathode::screen::TerminalScreen) (colors, text
//! attributes, scrollback, mouse select/copy), plus the [`input`] translation from
//! iced key presses to the bytes a PTY expects.
//!
//! Where [`cathode`] drives the terminal (parse + screen + pty), phosphor is what
//! glows: the visible grid. It is the embeddable counterpart to fjord's editor
//! widget — the standalone `tty` app and fed-ide's terminal panel share this one
//! implementation.
//!
//! The host supplies a [`TerminalStyle`] (plain colors) so phosphor stays free of any
//! particular theme crate; [`ANSI_DEFAULT`] + [`TerminalStyle::default_dark`] give a
//! sensible starting palette.

pub mod input;
pub mod terminal;

pub use terminal::{terminal, Terminal, TerminalStyle, ANSI_DEFAULT};
