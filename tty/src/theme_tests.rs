use super::*;
use crate::settings::Settings;

/// 16 distinct greys, base00 (#000000) … base0F (#f0f0f0).
fn sixteen() -> String {
    (0..16)
        .map(|i| format!("#{0:02x}{0:02x}{0:02x}", i * 16))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn base16_maps_bg_to_base00_and_fg_to_base05() {
    let style = base16::parse(&sixteen()).expect("16 colors parse");
    assert_eq!(style.bg, Color::from_rgb8(0x00, 0x00, 0x00));
    assert_eq!(style.fg, Color::from_rgb8(0x50, 0x50, 0x50));
    assert_eq!(style.ansi[0], style.bg);
    assert_eq!(style.ansi[7], style.fg);
}

#[test]
fn base16_rejects_wrong_count() {
    assert!(base16::parse("#111111 #222222").is_none());
    assert!(base16::parse("not hex at all").is_none());
}

#[test]
fn named_theme_resolves_chrome_and_terminal() {
    let nord = Theme::named("Nord");
    assert_eq!(nord.terminal.bg, Color::from_rgb8(0x2e, 0x34, 0x40));
    assert_eq!(nord.palette.bg, rime::theme::NORD.bg);
    // Unknown name falls back to Dracula.
    assert_eq!(Theme::named("nope").palette.bg, rime::theme::DRACULA.bg);
}

#[test]
fn custom_palette_rethemes_chrome_and_terminal() {
    let mut s = Settings::default();
    s.set_palette(&base16::parse(&sixteen()).unwrap());
    let theme = Theme::from_settings(&s);
    // Terminal bg is base00 (black) and the chrome bg follows it.
    assert_eq!(theme.terminal.bg, Color::from_rgb8(0x00, 0x00, 0x00));
    assert_eq!(theme.palette.bg, theme.terminal.bg);
}
