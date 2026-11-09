use egui::{Pos2, Rangef, Rect, Vec2, vec2};

/// Pixels around the point where the middle button went down in which middle-button scrolling stands still.
pub const MIDDLE_DEAD_ZONE: f32 = 8.0;
/// Pixels from where the middle button went down, past the dead zone, at which middle-button scrolling goes at its configured speed. It goes proportionally faster or slower elsewhere.
const MIDDLE_REFERENCE_DISTANCE: f32 = 100.0;
/// Rows past a pane's edge beyond which a selection drag scrolls no faster.
const MAX_DRAG_OVERSHOOT_ROWS: f32 = 10.0;


/// How far `pointer` is outside `range`: negative before its start, positive past its end, zero inside.
pub fn overshoot(pointer: f32, range: Rangef) -> f32 {
    if !pointer.is_finite() || range.min > range.max {
        return 0.0;
    }
    if pointer < range.min {
        pointer - range.min
    } else if pointer > range.max {
        pointer - range.max
    } else {
        0.0
    }
}


/// How fast a selection drag scrolls, in pixels per second, with the pointer `overshoot` pixels past a pane's edge: `lines_per_second` just past it, one more time as fast for every row farther out, up to a limit.
pub fn drag_velocity(overshoot: f32, row_height: f32, lines_per_second: f32) -> f32 {
    if overshoot == 0.0 || !overshoot.is_finite() || !(row_height > 0.0) || !(lines_per_second > 0.0) {
        return 0.0;
    }
    let rows = (overshoot.abs() / row_height).min(MAX_DRAG_OVERSHOOT_ROWS);
    overshoot.signum() * lines_per_second * row_height * (1.0 + rows)
}


/// Where a pointer held past the edges of `rect` scrolls a selection drag, in pixels per second along each axis.
pub fn drag_velocity_2d(pointer: Pos2, rect: Rect, row_height: f32, lines_per_second: f32) -> Vec2 {
    vec2(
        drag_velocity(overshoot(pointer.x, rect.x_range()), row_height.clone(), lines_per_second.clone()),
        drag_velocity(overshoot(pointer.y, rect.y_range()), row_height, lines_per_second),
    )
}


/// Middle-button scrolling: the view scrolls toward the pointer, faster the farther it is from where the button went down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiddleScroll {
    /// Where the middle button went down.
    pub origin: Pos2,
    /// The button was clicked without moving: scrolling goes on after the release until the next press or Escape.
    pub sticky: bool,
    /// The pointer left the dead zone while the button was held, so the release ends scrolling.
    pub moved: bool,
}


/// What the mouse and keyboard did in a frame, as middle-button scrolling sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MiddleInput {
    pub pointer: Option<Pos2>,
    /// The middle button went down over the view.
    pub middle_pressed_here: bool,
    pub middle_pressed: bool,
    pub middle_released: bool,
    /// Another mouse button went down anywhere.
    pub other_pressed: bool,
    pub escape: bool,
}


impl MiddleScroll {
    /// The middle-button scrolling after a frame of `input`, starting from `scroll`.
    pub fn next(scroll: Option<MiddleScroll>, input: MiddleInput) -> Option<MiddleScroll> {
        let Some(mut scroll) = scroll else {
            return match (input.middle_pressed_here, input.pointer) {
                (true, Some(origin)) => Some(MiddleScroll { origin, sticky: false, moved: false }),
                _ => None,
            };
        };
        if input.escape || input.other_pressed || (scroll.sticky && input.middle_pressed) {
            return None;
        }
        if !scroll.sticky
            && let Some(pointer) = input.pointer
            && outside_dead_zone(pointer - scroll.origin)
        {
            scroll.moved = true;
        }
        if input.middle_released && !scroll.sticky {
            if scroll.moved {
                return None;
            }
            scroll.sticky = true;
        }
        Some(scroll)
    }


    /// How fast the view scrolls with the pointer at `pointer`, in pixels per second along each axis: `lines_per_second` at the reference distance past the dead zone.
    pub fn velocity(&self, pointer: Pos2, row_height: f32, lines_per_second: f32) -> Vec2 {
        let along = |distance: f32| {
            let beyond = distance.abs() - MIDDLE_DEAD_ZONE;
            if !(beyond > 0.0) || !(row_height > 0.0) || !(lines_per_second > 0.0) {
                return 0.0;
            }
            distance.signum() * beyond / MIDDLE_REFERENCE_DISTANCE * lines_per_second * row_height
        };
        let offset = pointer - self.origin;
        vec2(along(offset.x), along(offset.y))
    }
}


/// Whether a pointer `offset` from where the middle button went down is out of the dead zone along either axis.
pub fn outside_dead_zone(offset: Vec2) -> bool {
    offset.x.abs() > MIDDLE_DEAD_ZONE || offset.y.abs() > MIDDLE_DEAD_ZONE
}


/// Moves a scroll offset by `velocity` pixels per second for a frame of `dt` seconds, kept between 0 and `max`.
pub fn advance(offset: f32, velocity: f32, dt: f32, max: f32) -> f32 {
    let moved = offset + velocity * dt.max(0.0);
    if !moved.is_finite() {
        return offset;
    }
    moved.min(max).max(0.0)
}


#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn overshoot_is_zero_inside_and_signed_outside() {
        let range = Rangef::new(10.0, 50.0);
        assert_eq!(overshoot(30.0, range), 0.0);
        assert_eq!(overshoot(10.0, range), 0.0);
        assert_eq!(overshoot(4.0, range), -6.0);
        assert_eq!(overshoot(57.0, range), 7.0);
        assert_eq!(overshoot(f32::NAN, range), 0.0);
    }


    #[test]
    fn drag_scrolls_at_the_configured_speed_just_past_the_edge() {
        assert_eq!(drag_velocity(0.0, 20.0, 10.0), 0.0);
        let just_past = drag_velocity(1.0, 20.0, 10.0);
        assert!((just_past - 210.0).abs() < 0.01, "{just_past}");
        assert!(drag_velocity(-1.0, 20.0, 10.0) < 0.0);
    }


    #[test]
    fn drag_scrolls_faster_farther_out_up_to_a_limit() {
        let near = drag_velocity(20.0, 20.0, 10.0);
        let far = drag_velocity(100.0, 20.0, 10.0);
        assert!(far > near);
        assert_eq!(drag_velocity(10_000.0, 20.0, 10.0), drag_velocity(20.0 * MAX_DRAG_OVERSHOOT_ROWS, 20.0, 10.0));
    }


    #[test]
    fn drag_speed_does_not_depend_on_the_frame_rate() {
        let velocity = drag_velocity(30.0, 20.0, 10.0);
        let mut slow = 0.0;
        let mut fast = 0.0;
        for _ in 0..30 {
            slow = advance(slow, velocity, 1.0 / 30.0, 1e6);
        }
        for _ in 0..144 {
            fast = advance(fast, velocity, 1.0 / 144.0, 1e6);
        }
        assert!((slow - fast).abs() < 0.5, "{slow} vs {fast}");
    }


    #[test]
    fn degenerate_drag_inputs_do_not_scroll() {
        assert_eq!(drag_velocity(5.0, 0.0, 10.0), 0.0);
        assert_eq!(drag_velocity(5.0, 20.0, 0.0), 0.0);
        assert_eq!(drag_velocity(f32::INFINITY, 20.0, 10.0), 0.0);
    }


    #[test]
    fn drag_velocity_2d_follows_each_edge() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        assert_eq!(drag_velocity_2d(pos2(50.0, 50.0), rect, 20.0, 10.0), Vec2::ZERO);
        let velocity = drag_velocity_2d(pos2(-5.0, 120.0), rect, 20.0, 10.0);
        assert!(velocity.x < 0.0 && velocity.y > 0.0);
    }


    #[test]
    fn advance_stays_in_range() {
        assert_eq!(advance(10.0, -1000.0, 1.0, 50.0), 0.0);
        assert_eq!(advance(10.0, 1000.0, 1.0, 50.0), 50.0);
        assert_eq!(advance(10.0, f32::NAN, 1.0, 50.0), 10.0);
        assert_eq!(advance(10.0, 100.0, -1.0, 50.0), 10.0);
    }


    fn press_at(origin: Pos2) -> Option<MiddleScroll> {
        MiddleScroll::next(None, MiddleInput { pointer: Some(origin), middle_pressed_here: true, middle_pressed: true, ..MiddleInput::default() })
    }


    #[test]
    fn middle_press_over_the_view_starts_scrolling() {
        assert_eq!(press_at(pos2(5.0, 6.0)), Some(MiddleScroll { origin: pos2(5.0, 6.0), sticky: false, moved: false }));
        assert_eq!(MiddleScroll::next(None, MiddleInput { pointer: Some(pos2(5.0, 6.0)), middle_pressed: true, ..MiddleInput::default() }), None);
    }


    #[test]
    fn hold_move_and_release_ends_scrolling() {
        let held = press_at(pos2(0.0, 0.0));
        let moved = MiddleScroll::next(held, MiddleInput { pointer: Some(pos2(0.0, 40.0)), ..MiddleInput::default() });
        assert!(moved.is_some_and(|scroll| scroll.moved));
        assert_eq!(MiddleScroll::next(moved, MiddleInput { pointer: Some(pos2(0.0, 40.0)), middle_released: true, ..MiddleInput::default() }), None);
    }


    #[test]
    fn click_without_moving_keeps_scrolling_until_the_next_press() {
        let held = press_at(pos2(0.0, 0.0));
        let sticky = MiddleScroll::next(held, MiddleInput { pointer: Some(pos2(2.0, 2.0)), middle_released: true, ..MiddleInput::default() });
        assert!(sticky.is_some_and(|scroll| scroll.sticky));
        let still = MiddleScroll::next(sticky, MiddleInput { pointer: Some(pos2(0.0, 300.0)), ..MiddleInput::default() });
        assert_eq!(still, sticky);
        assert_eq!(MiddleScroll::next(still, MiddleInput { pointer: Some(pos2(0.0, 300.0)), middle_pressed: true, middle_pressed_here: true, ..MiddleInput::default() }), None);
        assert_eq!(MiddleScroll::next(still, MiddleInput { other_pressed: true, ..MiddleInput::default() }), None);
        assert_eq!(MiddleScroll::next(still, MiddleInput { escape: true, ..MiddleInput::default() }), None);
    }


    #[test]
    fn escape_ends_held_scrolling() {
        assert_eq!(MiddleScroll::next(press_at(pos2(0.0, 0.0)), MiddleInput { escape: true, ..MiddleInput::default() }), None);
    }


    #[test]
    fn middle_velocity_has_a_dead_zone_and_grows_with_distance() {
        let scroll = MiddleScroll { origin: pos2(100.0, 100.0), sticky: false, moved: false };
        assert_eq!(scroll.velocity(pos2(105.0, 93.0), 20.0, 20.0), Vec2::ZERO);
        let reference = scroll.velocity(pos2(100.0, 100.0 + MIDDLE_DEAD_ZONE + MIDDLE_REFERENCE_DISTANCE), 20.0, 20.0);
        assert_eq!(reference.x, 0.0);
        assert!((reference.y - 400.0).abs() < 0.01, "{reference:?}");
        let up_left = scroll.velocity(pos2(50.0, 20.0), 20.0, 20.0);
        assert!(up_left.x < 0.0 && up_left.y < up_left.x);
    }


    #[test]
    fn dead_zone_is_per_axis() {
        assert!(!outside_dead_zone(vec2(MIDDLE_DEAD_ZONE, -MIDDLE_DEAD_ZONE)));
        assert!(outside_dead_zone(vec2(0.0, MIDDLE_DEAD_ZONE + 1.0)));
    }
}
