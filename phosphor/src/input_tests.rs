use super::*;

fn arrow(named: Named, mods: Modifiers) -> Vec<u8> {
    to_bytes(&Key::Named(named), mods, false).unwrap()
}

#[test]
fn plain_arrows_send_cursor_sequences() {
    let none = Modifiers::default();
    assert_eq!(arrow(Named::ArrowLeft, none), b"\x1b[D");
    assert_eq!(arrow(Named::ArrowRight, none), b"\x1b[C");
    // Application-cursor mode (DECCKM) switches `ESC [` → `ESC O`.
    assert_eq!(
        to_bytes(&Key::Named(Named::ArrowLeft), none, true).unwrap(),
        b"\x1bOD"
    );
}

#[test]
fn option_or_ctrl_arrow_moves_by_word() {
    // Meta-b / Meta-f are the readline/zsh backward-word / forward-word bindings.
    for mods in [Modifiers::ALT, Modifiers::CTRL] {
        assert_eq!(arrow(Named::ArrowLeft, mods), b"\x1bb", "{mods:?} ←");
        assert_eq!(arrow(Named::ArrowRight, mods), b"\x1bf", "{mods:?} →");
    }
}

#[test]
fn cmd_arrow_jumps_to_line_start_and_end() {
    // Cmd (logo) → beginning-of-line (Ctrl-A) / end-of-line (Ctrl-E).
    assert_eq!(arrow(Named::ArrowLeft, Modifiers::LOGO), b"\x01");
    assert_eq!(arrow(Named::ArrowRight, Modifiers::LOGO), b"\x05");
}

#[test]
fn modified_backspace_deletes_word_or_line() {
    let del = |mods| to_bytes(&Key::Named(Named::Backspace), mods, false).unwrap();
    assert_eq!(del(Modifiers::default()), b"\x7f", "plain ⌫");
    assert_eq!(del(Modifiers::ALT), b"\x1b\x7f", "⌥⌫ deletes a word");
    assert_eq!(del(Modifiers::LOGO), b"\x15", "⌘⌫ deletes to line start");
}
