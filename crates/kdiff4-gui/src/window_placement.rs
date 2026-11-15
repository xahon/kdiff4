use egui::{Pos2, Vec2, ViewportInfo};
use serde::{Deserialize, Serialize};

const DEFAULT_SIZE: Vec2 = Vec2::new(1200.0, 800.0);
/// The most of a monitor the default size takes, per axis.
const DEFAULT_MONITOR_SHARE: f32 = 0.8;


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub position: Option<[i32; 2]>,
    pub monitor: Option<[i32; 2]>,
    pub size: [f32; 2],
    pub maximized: bool,
    pub minimized: bool,
}


impl WindowPlacement {
    pub fn record(saved: &mut Option<Self>, viewport: &ViewportInfo, pixels_per_point: f32, monitor: Option<[i32; 2]>) {
        if viewport.minimized == Some(true) {
            if let Some(saved) = saved {
                saved.minimized = true;
            }
            return;
        }
        let maximized = viewport.maximized.unwrap_or_default();
        if maximized || viewport.fullscreen == Some(true) {
            if let Some(saved) = saved {
                if let (Some(previous), Some(current), Some(position)) = (saved.monitor, monitor, &mut saved.position) {
                    for axis in 0..2 {
                        position[axis] = position[axis].saturating_add(current[axis].saturating_sub(previous[axis]));
                    }
                }
                saved.monitor = monitor.or(saved.monitor);
                saved.maximized = maximized;
                saved.minimized = false;
            }
            return;
        }
        let Some(rect) = viewport.inner_rect else { return };
        let size = rect.size();
        if !size.is_finite() || size.min_elem() <= 0.0 {
            return;
        }
        let position = viewport.outer_rect.map(|rect| {
            let Pos2 { x, y } = rect.min * pixels_per_point;
            [x.round() as i32, y.round() as i32]
        });
        *saved = Some(Self { position, monitor, size: [size.x, size.y], maximized, minimized: false });
    }

    pub fn size_on_monitor(&self, default: Vec2, monitor_size: Vec2) -> Vec2 {
        let size = Vec2::from(self.size);
        let size = if size.is_finite() && size.min_elem() > 0.0 { size } else { default };
        size.max(Vec2::splat(64.0)).min(monitor_size)
    }
}


/// The normal size the main window gets when no placement is saved: what it returns to when the user restores it from maximized.
pub fn default_size(monitor_size: Option<Vec2>) -> Vec2 {
    let fitting = monitor_size
        .filter(|size| size.is_finite() && size.min_elem() > 0.0)
        .map_or(DEFAULT_SIZE, |size| DEFAULT_SIZE.min(size * DEFAULT_MONITOR_SHARE));
    fitting.max(Vec2::splat(64.0))
}


#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    #[test]
    fn saves_normal_geometry_and_preserves_it_when_maximized_or_minimized() {
        let normal = Rect::from_min_size(pos2(-600.0, 80.0), vec2(700.0, 500.0));
        let mut viewport = ViewportInfo { outer_rect: Some(normal), inner_rect: Some(normal), ..Default::default() };
        let mut saved = None;
        WindowPlacement::record(&mut saved, &viewport, 1.5, Some([-1440, 0]));
        let expected = WindowPlacement { position: Some([-900, 120]), monitor: Some([-1440, 0]), size: [700.0, 500.0], maximized: false, minimized: false };
        assert_eq!(saved, Some(expected.clone()));

        viewport.maximized = Some(true);
        viewport.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)));
        WindowPlacement::record(&mut saved, &viewport, 1.0, Some([0, 0]));
        viewport.minimized = Some(true);
        viewport.maximized = Some(false);
        viewport.inner_rect = None;
        WindowPlacement::record(&mut saved, &viewport, 1.0, None);
        assert_eq!(saved, Some(WindowPlacement { position: Some([540, 120]), monitor: Some([0, 0]), maximized: true, minimized: true, ..expected }));
    }

    #[test]
    fn the_default_size_fits_the_monitor_and_survives_a_missing_or_broken_one() {
        assert_eq!(default_size(Some(vec2(1920.0, 1080.0))), vec2(1200.0, 800.0));
        assert_eq!(default_size(Some(vec2(1000.0, 600.0))), vec2(800.0, 480.0));
        assert_eq!(default_size(None), vec2(1200.0, 800.0));
        assert_eq!(default_size(Some(vec2(0.0, f32::NAN))), vec2(1200.0, 800.0));
        assert_eq!(default_size(Some(vec2(50.0, 50.0))), vec2(64.0, 64.0));
    }

    #[test]
    fn clamps_saved_size_to_the_available_monitor() {
        let saved = WindowPlacement { position: None, monitor: None, size: [1500.0, 800.0], maximized: false, minimized: false };
        assert_eq!(saved.size_on_monitor(vec2(800.0, 600.0), vec2(1280.0, 720.0)), vec2(1280.0, 720.0));
    }
}
