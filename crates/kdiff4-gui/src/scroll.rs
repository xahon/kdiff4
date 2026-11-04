use egui::emath;


/// Below this distance, in pixels, a scroll is where it is going.
const ARRIVED: f32 = 0.5;
/// Share of the way a smooth scroll covers within its duration.
const SMOOTH_REACH: f32 = 0.99;


/// How scrolling moves the panes: at once, or smoothly over `duration` seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollMotion {
    pub smooth: bool,
    pub duration: f32,
}


impl ScrollMotion {
    fn is_instant(&self) -> bool {
        !self.smooth || !(self.duration > 0.0)
    }
}


/// A vertical scroll offset that moves toward its target, in pixels: smoothly over a few frames, or at once.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SmoothScroll {
    pub offset: f32,
    pub target: f32,
}


impl SmoothScroll {
    /// Starts scrolling to `target`, kept between 0 and `max`.
    pub fn scroll_to(&mut self, target: f32, max: f32, motion: ScrollMotion) {
        let target = if target.is_finite() { target } else { self.target.clone() };
        self.target = target.min(max).max(0.0);
        if motion.is_instant() {
            self.offset = self.target.clone();
        }
    }


    /// Scrolls `delta` pixels further than the current target, so quick wheel notches add up.
    pub fn scroll_by(&mut self, delta: f32, max: f32, motion: ScrollMotion) {
        self.scroll_to(self.target.clone() + delta, max, motion);
    }


    /// Moves the offset and the target by `delta` at once, kept between 0 and `max`, so a scroll under way goes on from there. Trackpads scroll this way.
    pub fn shift(&mut self, delta: f32, max: f32) {
        if delta.is_finite() {
            let max = max.max(0.0);
            self.offset = (self.offset.clone() + delta).min(max).max(0.0);
            self.target = (self.target.clone() + delta).min(max).max(0.0);
        }
    }


    /// Puts the offset at `offset` at once, as when the user drags a scroll bar.
    pub fn set(&mut self, offset: f32) {
        if offset.is_finite() {
            self.offset = offset.clone();
            self.target = offset;
        }
    }


    /// Moves the offset toward the target for a frame of `dt` seconds. Returns whether it is still moving.
    pub fn step(&mut self, dt: f32, motion: ScrollMotion) -> bool {
        let distance = self.target.clone() - self.offset.clone();
        if distance.abs() < ARRIVED || motion.is_instant() {
            self.offset = self.target.clone();
            return false;
        }
        let share = emath::exponential_smooth_factor(SMOOTH_REACH, motion.duration.clone(), dt.max(0.0));
        self.offset += distance * share.clamp(0.0, 1.0);
        if (self.target.clone() - self.offset.clone()).abs() < ARRIVED {
            self.offset = self.target.clone();
        }
        self.offset != self.target
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    const SMOOTH: ScrollMotion = ScrollMotion { smooth: true, duration: 0.2 };
    const INSTANT: ScrollMotion = ScrollMotion { smooth: false, duration: 0.2 };


    #[test]
    fn instant_scrolls_arrive_at_once() {
        let mut scroll = SmoothScroll::default();
        scroll.scroll_to(120.0, 1000.0, INSTANT);
        assert_eq!(scroll, SmoothScroll { offset: 120.0, target: 120.0 });
        scroll.scroll_by(-30.0, 1000.0, INSTANT);
        assert_eq!(scroll.offset, 90.0);
        assert!(!scroll.step(0.016, INSTANT));
    }


    #[test]
    fn zero_duration_is_instant() {
        let mut scroll = SmoothScroll::default();
        scroll.scroll_to(50.0, 1000.0, ScrollMotion { smooth: true, duration: 0.0 });
        assert_eq!(scroll.offset, 50.0);
    }


    #[test]
    fn smooth_scrolls_approach_and_arrive() {
        let mut scroll = SmoothScroll::default();
        scroll.scroll_to(100.0, 1000.0, SMOOTH);
        assert_eq!(scroll.offset, 0.0);
        assert!(scroll.step(0.016, SMOOTH));
        let first = scroll.offset.clone();
        assert!(first > 0.0 && first < 100.0);
        assert!(scroll.step(0.016, SMOOTH));
        assert!(scroll.offset > first);
        let mut frames = 0;
        while scroll.step(0.016, SMOOTH) {
            frames += 1;
            assert!(frames < 100, "the scroll never arrives");
        }
        assert_eq!(scroll.offset, 100.0);
    }


    #[test]
    fn targets_stay_in_range() {
        let mut scroll = SmoothScroll::default();
        scroll.scroll_to(500.0, 200.0, INSTANT);
        assert_eq!(scroll.offset, 200.0);
        scroll.scroll_by(-900.0, 200.0, INSTANT);
        assert_eq!(scroll.offset, 0.0);
        scroll.scroll_to(f32::NAN, 200.0, INSTANT);
        assert_eq!(scroll.offset, 0.0);
        scroll.scroll_to(40.0, -10.0, INSTANT);
        assert_eq!(scroll.offset, 0.0);
    }


    #[test]
    fn wheel_notches_add_up_while_moving() {
        let mut scroll = SmoothScroll::default();
        scroll.scroll_by(60.0, 1000.0, SMOOTH);
        scroll.step(0.016, SMOOTH);
        scroll.scroll_by(60.0, 1000.0, SMOOTH);
        assert_eq!(scroll.target, 120.0);
    }


    #[test]
    fn shift_moves_a_scroll_under_way() {
        let mut scroll = SmoothScroll { offset: 10.0, target: 100.0 };
        scroll.shift(5.0, 1000.0);
        assert_eq!(scroll, SmoothScroll { offset: 15.0, target: 105.0 });
        scroll.shift(-50.0, 1000.0);
        assert_eq!(scroll, SmoothScroll { offset: 0.0, target: 55.0 });
        scroll.shift(f32::INFINITY, 1000.0);
        assert_eq!(scroll.target, 55.0);
    }


    #[test]
    fn set_stops_a_scroll_where_the_user_put_it() {

        let mut scroll = SmoothScroll::default();
        scroll.scroll_to(300.0, 1000.0, SMOOTH);
        scroll.set(42.0);
        assert_eq!(scroll, SmoothScroll { offset: 42.0, target: 42.0 });
        assert!(!scroll.step(0.016, SMOOTH));
    }
}
