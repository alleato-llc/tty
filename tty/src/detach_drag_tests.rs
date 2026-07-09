use super::*;

use crate::behavior::{headless, screen_term};
use crate::state::Tab;

#[test]
fn on_opened_seeds_bounds_at_the_origin() {
    let mut tty = headless(0);
    let win = iced::window::Id::unique();
    on_opened(&mut tty, win, Size::new(720.0, 600.0));
    let b = tty.window_bounds[&win];
    assert_eq!((b.x, b.y, b.width, b.height), (0.0, 0.0, 720.0, 600.0));
}

#[test]
fn on_moved_updates_position_and_leaves_size_alone() {
    let mut tty = headless(0);
    let win = iced::window::Id::unique();
    on_opened(&mut tty, win, Size::new(720.0, 600.0));
    on_moved(&mut tty, win, Point::new(50.0, 60.0));
    let b = tty.window_bounds[&win];
    assert_eq!((b.x, b.y, b.width, b.height), (50.0, 60.0, 720.0, 600.0));
}

#[test]
fn on_moved_arms_the_debounce_only_for_a_detached_window() {
    let mut tty = headless(0);
    let plain = iced::window::Id::unique();
    on_moved(&mut tty, plain, Point::new(1.0, 1.0));
    assert!(
        tty.last_detached_move.is_none(),
        "a non-detached window moving isn't a drag-to-dock gesture"
    );

    let detached_win = iced::window::Id::unique();
    tty.detached
        .insert(detached_win, Tab::new(screen_term("d")));
    on_moved(&mut tty, detached_win, Point::new(1.0, 1.0));
    assert!(
        matches!(tty.last_detached_move, Some((w, _)) if w == detached_win),
        "moving a detached window arms the debounce"
    );
}

#[test]
fn set_position_updates_bounds_without_arming_the_debounce() {
    let mut tty = headless(0);
    let detached_win = iced::window::Id::unique();
    tty.detached
        .insert(detached_win, Tab::new(screen_term("d")));
    set_position(&mut tty, detached_win, Point::new(5.0, 6.0));
    let b = tty.window_bounds[&detached_win];
    assert_eq!((b.x, b.y), (5.0, 6.0));
    assert!(
        tty.last_detached_move.is_none(),
        "set_position learns the initial placement — it's not a drag"
    );
}

#[test]
fn on_resized_updates_size_and_leaves_position_alone() {
    let mut tty = headless(0);
    let win = iced::window::Id::unique();
    on_moved(&mut tty, win, Point::new(10.0, 20.0));
    on_resized(&mut tty, win, Size::new(400.0, 300.0));
    let b = tty.window_bounds[&win];
    assert_eq!((b.x, b.y, b.width, b.height), (10.0, 20.0, 400.0, 300.0));
}

#[test]
fn pending_reflects_whether_the_debounce_is_armed() {
    let mut tty = headless(0);
    assert!(!pending(&tty));
    tty.last_detached_move = Some((iced::window::Id::unique(), Instant::now()));
    assert!(pending(&tty));
}

#[test]
fn poll_settle_is_pending_with_no_armed_debounce() {
    let mut tty = headless(0);
    assert!(matches!(poll_settle(&mut tty), Settle::Pending));
}

#[test]
fn poll_settle_stays_pending_before_the_settle_window_elapses() {
    let mut tty = headless(0);
    let win = iced::window::Id::unique();
    tty.last_detached_move = Some((win, Instant::now()));
    assert!(
        matches!(poll_settle(&mut tty), Settle::Pending),
        "just moved — hasn't settled yet"
    );
    // Still armed: poll_settle doesn't clear it while pending.
    assert!(tty.last_detached_move.is_some());
}

#[test]
fn poll_settle_reattaches_when_dropped_on_the_drop_band() {
    let mut tty = headless(0);
    let main = iced::window::Id::unique();
    tty.main_window = Some(main);
    tty.window_bounds.insert(
        main,
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 900.0,
            height: 600.0,
        },
    );
    let win = iced::window::Id::unique();
    tty.window_bounds.insert(
        win,
        Rectangle {
            x: 100.0,
            y: 10.0,
            width: 400.0,
            height: 300.0,
        },
    );
    tty.last_detached_move = Some((win, Instant::now() - Duration::from_secs(1)));
    assert!(matches!(poll_settle(&mut tty), Settle::Reattach(w) if w == win));
    assert!(
        tty.last_detached_move.is_none(),
        "the debounce is consumed once settled"
    );
}

#[test]
fn poll_settle_is_a_no_op_reposition_when_dropped_elsewhere() {
    let mut tty = headless(0);
    let main = iced::window::Id::unique();
    tty.main_window = Some(main);
    tty.window_bounds.insert(
        main,
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 900.0,
            height: 600.0,
        },
    );
    let win = iced::window::Id::unique();
    tty.window_bounds.insert(
        win,
        Rectangle {
            x: 100.0,
            y: 400.0,
            width: 400.0,
            height: 300.0,
        },
    );
    tty.last_detached_move = Some((win, Instant::now() - Duration::from_secs(1)));
    assert!(matches!(poll_settle(&mut tty), Settle::Repositioned));
}

#[test]
fn poll_settle_with_no_main_window_never_reattaches() {
    let mut tty = headless(0);
    let win = iced::window::Id::unique();
    tty.window_bounds.insert(
        win,
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 300.0,
        },
    );
    tty.last_detached_move = Some((win, Instant::now() - Duration::from_secs(1)));
    assert!(matches!(poll_settle(&mut tty), Settle::Repositioned));
}
