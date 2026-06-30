//! Translate Iced key presses into the byte sequences a PTY expects, so a focused
//! terminal behaves like a real terminal (control codes, arrow escapes). Shared by
//! the IDE's terminal panel and the standalone `tty` app.

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};

/// The bytes to send to the PTY for a key press, or `None` if it produces no input
/// (e.g. a bare modifier).
pub fn to_bytes(key: &Key, mods: Modifiers) -> Option<Vec<u8>> {
    match key {
        Key::Character(s) => {
            if mods.control() {
                // Ctrl+letter → control code (Ctrl+A = 0x01 … Ctrl+Z = 0x1a).
                let mut out = Vec::new();
                for c in s.chars() {
                    let lc = c.to_ascii_lowercase();
                    if lc.is_ascii_lowercase() {
                        out.push((lc as u8 - b'a') + 1);
                    } else {
                        out.extend_from_slice(c.to_string().as_bytes());
                    }
                }
                (!out.is_empty()).then_some(out)
            } else {
                Some(s.as_bytes().to_vec())
            }
        }
        Key::Named(n) => {
            let bytes: &[u8] = match n {
                Named::Enter => b"\r",
                Named::Backspace => b"\x7f",
                Named::Tab => b"\t",
                Named::Escape => b"\x1b",
                Named::Space => b" ",
                Named::ArrowUp => b"\x1b[A",
                Named::ArrowDown => b"\x1b[B",
                Named::ArrowRight => b"\x1b[C",
                Named::ArrowLeft => b"\x1b[D",
                Named::Home => b"\x1b[H",
                Named::End => b"\x1b[F",
                Named::PageUp => b"\x1b[5~",
                Named::PageDown => b"\x1b[6~",
                Named::Delete => b"\x1b[3~",
                _ => return None,
            };
            Some(bytes.to_vec())
        }
        _ => None,
    }
}
