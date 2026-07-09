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

#[test]
fn plain_character_sends_its_utf8_bytes() {
    let none = Modifiers::default();
    assert_eq!(
        to_bytes(&Key::Character("a".into()), none, false).unwrap(),
        b"a"
    );
    // Non-ASCII passes through as its UTF-8 encoding, not a control code.
    assert_eq!(
        to_bytes(&Key::Character("é".into()), none, false).unwrap(),
        "é".as_bytes()
    );
}

#[test]
fn ctrl_letter_sends_the_control_code() {
    let ctrl = |s: &str| to_bytes(&Key::Character(s.into()), Modifiers::CTRL, false);
    assert_eq!(ctrl("a").unwrap(), vec![0x01], "Ctrl+a");
    assert_eq!(ctrl("z").unwrap(), vec![0x1a], "Ctrl+z");
    // Uppercase is lowercased before mapping — Ctrl+A is the same code as Ctrl+a.
    assert_eq!(ctrl("A").unwrap(), vec![0x01]);
}

#[test]
fn ctrl_non_letter_character_passes_through_unmodified() {
    // Not ASCII-lowercase-able, so it falls through to the literal bytes branch.
    assert_eq!(
        to_bytes(&Key::Character("[".into()), Modifiers::CTRL, false).unwrap(),
        b"["
    );
}

#[test]
fn ctrl_empty_character_produces_no_input() {
    assert_eq!(
        to_bytes(&Key::Character("".into()), Modifiers::CTRL, false),
        None
    );
}

#[test]
fn named_enter_tab_escape_space() {
    let key = |n| to_bytes(&Key::Named(n), Modifiers::default(), false).unwrap();
    assert_eq!(key(Named::Enter), b"\r");
    assert_eq!(key(Named::Tab), b"\t");
    assert_eq!(key(Named::Escape), b"\x1b");
    assert_eq!(key(Named::Space), b" ");
}

#[test]
fn named_navigation_keys_send_their_csi_sequences() {
    let key = |n| to_bytes(&Key::Named(n), Modifiers::default(), false).unwrap();
    assert_eq!(key(Named::ArrowUp), b"\x1b[A");
    assert_eq!(key(Named::ArrowDown), b"\x1b[B");
    assert_eq!(key(Named::Home), b"\x1b[H");
    assert_eq!(key(Named::End), b"\x1b[F");
    assert_eq!(key(Named::PageUp), b"\x1b[5~");
    assert_eq!(key(Named::PageDown), b"\x1b[6~");
    assert_eq!(key(Named::Delete), b"\x1b[3~");
}

#[test]
fn unhandled_named_key_produces_no_input() {
    assert_eq!(
        to_bytes(&Key::Named(Named::F1), Modifiers::default(), false),
        None
    );
}

#[test]
fn unidentified_key_produces_no_input() {
    assert_eq!(
        to_bytes(&Key::Unidentified, Modifiers::default(), false),
        None
    );
}

mod mouse {
    use super::*;
    use cathode::screen::{MouseState, MouseTracking};

    fn state(tracking: MouseTracking, sgr: bool) -> MouseState {
        MouseState { tracking, sgr }
    }

    #[test]
    fn no_report_when_tracking_is_off() {
        assert_eq!(
            mouse_report(
                state(MouseTracking::Off, true),
                MouseEvent::Press(MouseButton::Left),
                0,
                0,
                Modifiers::default(),
            ),
            None
        );
    }

    #[test]
    fn no_report_without_sgr() {
        assert_eq!(
            mouse_report(
                state(MouseTracking::Normal, false),
                MouseEvent::Press(MouseButton::Left),
                0,
                0,
                Modifiers::default(),
            ),
            None
        );
    }

    #[test]
    fn press_and_release_encode_sgr_with_one_based_coords() {
        let s = state(MouseTracking::Normal, true);
        assert_eq!(
            mouse_report(
                s,
                MouseEvent::Press(MouseButton::Left),
                0,
                0,
                Modifiers::default()
            )
            .unwrap(),
            b"\x1b[<0;1;1M".to_vec()
        );
        assert_eq!(
            mouse_report(
                s,
                MouseEvent::Release(MouseButton::Left),
                4,
                9,
                Modifiers::default()
            )
            .unwrap(),
            b"\x1b[<0;5;10m".to_vec()
        );
    }

    #[test]
    fn button_codes_map_left_middle_right() {
        let s = state(MouseTracking::Normal, true);
        let press = |b| mouse_report(s, MouseEvent::Press(b), 0, 0, Modifiers::default()).unwrap();
        assert_eq!(press(MouseButton::Left), b"\x1b[<0;1;1M".to_vec());
        assert_eq!(press(MouseButton::Middle), b"\x1b[<1;1;1M".to_vec());
        assert_eq!(press(MouseButton::Right), b"\x1b[<2;1;1M".to_vec());
    }

    #[test]
    fn drag_is_reported_only_in_button_drag_or_any_motion_modes() {
        let drag = MouseEvent::Drag(MouseButton::Left);
        assert_eq!(
            mouse_report(
                state(MouseTracking::Normal, true),
                drag,
                0,
                0,
                Modifiers::default()
            ),
            None,
            "mode 1000 doesn't report motion"
        );
        assert_eq!(
            mouse_report(
                state(MouseTracking::ButtonDrag, true),
                drag,
                0,
                0,
                Modifiers::default()
            ),
            Some(b"\x1b[<32;1;1M".to_vec()),
            "mode 1002 reports drag; +32 motion bit on button 0"
        );
        assert_eq!(
            mouse_report(
                state(MouseTracking::AnyMotion, true),
                drag,
                0,
                0,
                Modifiers::default()
            ),
            Some(b"\x1b[<32;1;1M".to_vec()),
        );
    }

    #[test]
    fn free_motion_is_reported_only_in_any_motion_mode() {
        let mv = MouseEvent::Move;
        assert_eq!(
            mouse_report(
                state(MouseTracking::ButtonDrag, true),
                mv,
                0,
                0,
                Modifiers::default()
            ),
            None,
            "mode 1002 doesn't report button-less motion"
        );
        assert_eq!(
            mouse_report(
                state(MouseTracking::AnyMotion, true),
                mv,
                0,
                0,
                Modifiers::default()
            ),
            Some(b"\x1b[<35;1;1M".to_vec()),
            "\"no button\" (3) + motion (32) = 35",
        );
    }

    #[test]
    fn wheel_encodes_64_and_65() {
        let s = state(MouseTracking::Normal, true);
        assert_eq!(
            mouse_report(s, MouseEvent::WheelUp, 0, 0, Modifiers::default()).unwrap(),
            b"\x1b[<64;1;1M".to_vec()
        );
        assert_eq!(
            mouse_report(s, MouseEvent::WheelDown, 0, 0, Modifiers::default()).unwrap(),
            b"\x1b[<65;1;1M".to_vec()
        );
    }

    #[test]
    fn modifier_bits_accumulate_on_top_of_the_button_code() {
        let s = state(MouseTracking::Normal, true);
        let press =
            |mods| mouse_report(s, MouseEvent::Press(MouseButton::Left), 0, 0, mods).unwrap();
        assert_eq!(press(Modifiers::SHIFT), b"\x1b[<4;1;1M".to_vec());
        assert_eq!(press(Modifiers::ALT), b"\x1b[<8;1;1M".to_vec());
        assert_eq!(press(Modifiers::CTRL), b"\x1b[<16;1;1M".to_vec());
        assert_eq!(
            press(Modifiers::SHIFT | Modifiers::CTRL),
            b"\x1b[<20;1;1M".to_vec(),
            "shift (4) + control (16) = 20"
        );
    }
}
