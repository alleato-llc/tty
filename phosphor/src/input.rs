//! Translate input into the byte sequences a PTY expects, so a focused terminal
//! behaves like a real terminal: [`to_bytes`] for key presses (control codes, arrow
//! escapes, honoring DEC application-cursor-keys mode), and [`mouse_report`] for the
//! SGR mouse encoding apps enable via DEC modes 1000/1002/1003 + 1006.

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};

use cathode::screen::{MouseState, MouseTracking};

/// The bytes to send to the PTY for a key press, or `None` if it produces no input
/// (e.g. a bare modifier). When `app_cursor` is set (DECCKM), arrow/Home/End keys send
/// the `ESC O _` application form instead of `ESC [ _`.
pub fn to_bytes(key: &Key, mods: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
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
            // Cursor keys flip between `ESC [` (normal) and `ESC O` (application) forms.
            let ss3 = if app_cursor { b'O' } else { b'[' };
            let cursor = |last: u8| Some(vec![0x1b, ss3, last]);
            match n {
                Named::Enter => Some(b"\r".to_vec()),
                Named::Backspace => Some(b"\x7f".to_vec()),
                Named::Tab => Some(b"\t".to_vec()),
                Named::Escape => Some(b"\x1b".to_vec()),
                Named::Space => Some(b" ".to_vec()),
                Named::ArrowUp => cursor(b'A'),
                Named::ArrowDown => cursor(b'B'),
                Named::ArrowRight => cursor(b'C'),
                Named::ArrowLeft => cursor(b'D'),
                Named::Home => cursor(b'H'),
                Named::End => cursor(b'F'),
                Named::PageUp => Some(b"\x1b[5~".to_vec()),
                Named::PageDown => Some(b"\x1b[6~".to_vec()),
                Named::Delete => Some(b"\x1b[3~".to_vec()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A pointer interaction to report to the running application.
#[derive(Debug, Clone, Copy)]
pub enum MouseEvent {
    Press(MouseButton),
    Release(MouseButton),
    /// Motion while a button is held.
    Drag(MouseButton),
    /// Free motion (no button) — only reported in any-motion mode (1003).
    Move,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// Encode a pointer event as the bytes the app expects, or `None` if this `mouse`
/// mode doesn't report it. Only the SGR (1006) encoding is emitted — the universal
/// modern form, free of the legacy 223-column limit. `col`/`row` are 0-based grid cells.
pub fn mouse_report(
    mouse: MouseState,
    event: MouseEvent,
    col: usize,
    row: usize,
    mods: Modifiers,
) -> Option<Vec<u8>> {
    if !mouse.reports() || !mouse.sgr {
        return None;
    }
    // Motion is only reported in the drag/any modes.
    match event {
        MouseEvent::Drag(_) if mouse.tracking == MouseTracking::Normal => return None,
        MouseEvent::Move if !matches!(mouse.tracking, MouseTracking::AnyMotion) => return None,
        _ => {}
    }

    // Low 2 bits: button (wheel uses 64/65); +32 motion; + modifier bits.
    let (mut cb, release) = match event {
        MouseEvent::Press(b) | MouseEvent::Drag(b) => (button_code(b), false),
        MouseEvent::Release(b) => (button_code(b), true),
        MouseEvent::Move => (3, false), // "no button" + motion
        MouseEvent::WheelUp => (64, false),
        MouseEvent::WheelDown => (65, false),
    };
    if matches!(event, MouseEvent::Drag(_) | MouseEvent::Move) {
        cb += 32;
    }
    if mods.shift() {
        cb += 4;
    }
    if mods.alt() {
        cb += 8;
    }
    if mods.control() {
        cb += 16;
    }
    let x = col + 1;
    let y = row + 1;
    let final_byte = if release { 'm' } else { 'M' };
    Some(format!("\x1b[<{cb};{x};{y}{final_byte}").into_bytes())
}

fn button_code(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}
