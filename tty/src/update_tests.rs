use super::*;

/// The four arrow keys map to their pane-grid directions; everything else is `None`
/// (so a non-arrow chord never triggers a split/focus move).
#[test]
fn arrow_keys_map_to_directions_others_none() {
    use iced::keyboard::key::Named;
    use iced::widget::pane_grid::Direction;

    assert_eq!(arrow_direction(Named::ArrowLeft), Some(Direction::Left));
    assert_eq!(arrow_direction(Named::ArrowRight), Some(Direction::Right));
    assert_eq!(arrow_direction(Named::ArrowUp), Some(Direction::Up));
    assert_eq!(arrow_direction(Named::ArrowDown), Some(Direction::Down));
    assert_eq!(arrow_direction(Named::Enter), None);
    assert_eq!(arrow_direction(Named::Escape), None);
}
