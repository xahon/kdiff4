use std::collections::HashMap;

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Response, Role, Sense, Stroke, StrokeKind, TextStyle, Ui, UiBuilder, Vec2, WidgetInfo};
use egui::text::{LayoutJob, TextWrapping};

use crate::{icons, theme};


const PANEL_WIDTH: f32 = 151.0;
const CLOSE_SIZE: Vec2 = egui::vec2(22.0, 16.0);
const CLOSE_INSET: f32 = 6.0;
const TITLE_PADDING: Vec2 = egui::vec2(12.0, 6.0);
const CORNER_RADIUS_REDUCTION: u8 = 2;
const MARQUEE_SPEED: f64 = 45.0;
const MARQUEE_END_PAUSE: f64 = 2.0;
const CLOSE_FADE_SECONDS: f32 = 0.05;
const SLIDE_SECONDS: f64 = 0.1;
const DRAG_SCROLL_EDGE: f32 = 32.0;
const DRAG_SCROLL_SPEED: f32 = 480.0;


pub struct TabOrder {
    // Closed slots keep their positions for reopening and briefly reserve space for the close animation.
    slots: Vec<(usize, Option<f64>)>,
    positions: HashMap<usize, TabPosition>,
}


struct TabPosition {
    slot: usize,
    offset: f32,
    slide_start: Option<f64>,
}


impl TabPosition {
    fn offset_at(&self, now: f64) -> f32 {
        self.slide_start.map_or(self.offset, |start| self.offset * (1.0 - (now - start) / SLIDE_SECONDS).clamp(0.0, 1.0) as f32)
    }
}


impl TabOrder {
    pub fn new(id: usize) -> Self {
        Self { slots: vec![(id, None)], positions: HashMap::new() }
    }

    pub fn push(&mut self, id: usize) {
        self.slots.push((id, None));
        self.positions.clear();
    }

    pub fn open_ids(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        self.slots.iter().filter_map(|&(id, closed)| closed.is_none().then_some(id))
    }

    pub fn close(&mut self, ctx: &egui::Context, id: usize) {
        let (_, closed) = self.slots.iter_mut().find(|(slot, _)| *slot == id).expect("Tabs: missing tab to close");
        *closed = Some(ctx.input(|input| input.time));
        self.positions.clear();
        ctx.request_repaint();
    }

    pub fn reopen(&mut self, id: usize) {
        let (_, closed) = self.slots.iter_mut().find(|(slot, _)| *slot == id).expect("Tabs: missing tab to reopen");
        *closed = None;
        self.positions.clear();
    }

    pub fn set_open_order(&mut self, ids: Vec<usize>) {
        for ((id, _), next) in self.slots.iter_mut().filter(|(_, closed)| closed.is_none()).zip(ids) {
            *id = next;
        }
    }

    pub fn is_sliding(&self, ctx: &egui::Context) -> bool {
        let now = ctx.input(|input| input.time);
        self.slots.iter().rev().skip_while(|(_, closed)| closed.is_some())
            .any(|&(_, closed)| closed.is_some_and(|start| now - start < SLIDE_SECONDS))
    }

    pub fn allocate(&self, ui: &mut Ui) -> Vec<Rect> {
        let now = ui.input(|input| input.time);
        let stride = PANEL_WIDTH + ui.spacing().item_spacing.x;
        let mut gap = 0.0;
        let mut rects = Vec::new();
        for &(_, closed) in &self.slots {
            if let Some(start) = closed {
                gap += (1.0 - (now - start) / SLIDE_SECONDS).clamp(0.0, 1.0) as f32;
            } else {
                ui.add_space(gap * stride);
                gap = 0.0;
                rects.push(allocate_panel(ui));
            }
        }
        rects
    }

    pub fn position(&mut self, ui: &Ui, id: usize, slot: usize, rects: &[Rect], floating: Option<Rect>) -> Rect {
        let target = rects[slot];
        let now = ui.input(|input| input.time);
        let position = self.positions.entry(id).or_insert(TabPosition { slot, offset: 0.0, slide_start: Some(now) });
        if let Some(rect) = floating {
            *position = TabPosition { slot, offset: rect.left() - target.left(), slide_start: None };
            return rect;
        }
        if position.slot != slot || position.slide_start.is_none() {
            // Keep the previous position relative to the strip, even when the strip scrolls.
            let offset = rects[position.slot].left() - target.left() + position.offset_at(now);
            *position = TabPosition { slot, offset, slide_start: Some(now) };
        }
        let offset = position.offset_at(now);
        if offset.abs() >= 0.01 {
            ui.ctx().request_repaint();
            return target.translate(egui::vec2(offset, 0.0));
        }
        target
    }
}


#[derive(Clone, Copy)]
pub struct TabDrag {
    pub id: usize,
    pub slot: usize,
    grab_offset: f32,
}


impl TabDrag {
    pub fn start(ui: &Ui, ids: &[usize], rects: &[Rect]) -> Option<Self> {
        let dragged = ui.ctx().drag_started_id()?;
        let slot = ids.iter().position(|id| tab_widget_id(ui, *id) == dragged)?;
        let origin = ui.input(|input| input.pointer.press_origin())?;
        Some(Self { id: ids[slot], slot, grab_offset: origin.x - rects[slot].left() })
    }

    pub fn follow(&mut self, rects: &[Rect], x: f32) {
        while self.slot > 0 && x < rects[self.slot - 1].center().x {
            self.slot -= 1;
        }
        while self.slot + 1 < rects.len() && x > rects[self.slot + 1].center().x {
            self.slot += 1;
        }
    }

    pub fn floating_rect(self, rect: Rect, x: f32, clip: Rect) -> Rect {
        let left = (x - self.grab_offset).clamp(clip.left(), clip.left().max(clip.right() - rect.width()));
        rect.translate(egui::vec2(left - rect.left(), 0.0))
    }

    pub fn scroll(ui: &Ui, rects: &[Rect], x: f32) {
        let clip = ui.clip_rect();
        let edge = DRAG_SCROLL_EDGE.min(clip.width() / 2.0);
        let left = rects.first().is_some_and(|rect| rect.left() < clip.left());
        let right = rects.last().is_some_and(|rect| rect.right() > clip.right());
        let left_pressure = if left { (clip.left() + edge - x).max(0.0) } else { 0.0 };
        let right_pressure = if right { (x - clip.right() + edge).max(0.0) } else { 0.0 };
        let fraction = (left_pressure - right_pressure) / edge.max(1.0);
        if fraction != 0.0 {
            let dt = ui.input(|input| input.stable_dt);
            ui.scroll_with_delta_animation(egui::vec2(fraction.clamp(-1.0, 1.0) * DRAG_SCROLL_SPEED * dt, 0.0), egui::style::ScrollAnimation::none());
            ui.ctx().request_repaint();
        }
    }
}


fn tab_widget_id(ui: &Ui, tab: usize) -> Id {
    ui.make_persistent_id(tab)
}


fn allocate_panel(ui: &mut Ui) -> Rect {
    let height = ui.text_style_height(&TextStyle::Button) + 2.0 * TITLE_PADDING.y;
    ui.allocate_space(egui::vec2(PANEL_WIDTH, height)).1
}


pub fn paint_separator(ui: &Ui, strip: Rect, selected: Option<(Rect, Rect)>) {
    let y = strip.bottom() - 0.5;
    let stroke = Stroke::new(1.0, theme::palette_of(ui).pane_border);
    let painter = ui.painter();
    let Some((tab, clip)) = selected else {
        painter.hline(strip.x_range(), y, stroke);
        return;
    };
    let left = tab.left().clamp(clip.left(), clip.right());
    let right = tab.right().clamp(clip.left(), clip.right());
    painter.hline(strip.left()..=left, y, stroke);
    painter.hline(right..=strip.right(), y, stroke);
    if left < right {
        let clip = Rect::from_min_max(egui::pos2(clip.left(), strip.top()), egui::pos2(clip.right(), strip.bottom()));
        painter.with_clip_rect(clip).add(egui::Shape::line(active_outline(tab, y), stroke));
    }
}


fn active_outline(tab: Rect, bottom: f32) -> Vec<Pos2> {
    const ARC_SEGMENTS: usize = 8;
    let top = tab.top() + 0.5;
    let radius = (((bottom - top) / 2.0).min(tab.width() / 4.0) - f32::from(CORNER_RADIUS_REDUCTION)).max(0.0);
    let mut points = Vec::with_capacity(4 * (ARC_SEGMENTS + 1));
    // Each shoulder uses opposite quarter circles joined by a short vertical segment.
    for (x, y, start, direction) in [
        (tab.left(), bottom - radius, 1.0, -1.0),
        (tab.left() + 2.0 * radius, top + radius, 2.0, 1.0),
        (tab.right() - 2.0 * radius, top + radius, 3.0, 1.0),
        (tab.right(), bottom - radius, 2.0, -1.0),
    ] {
        for step in 0..=ARC_SEGMENTS {
            let angle = (start + direction * step as f32 / ARC_SEGMENTS as f32) * std::f32::consts::FRAC_PI_2;
            let (sin, cos) = angle.sin_cos();
            points.push(egui::pos2(x + radius * cos, y + radius * sin));
        }
    }
    points.dedup_by(|a, b| a.distance_sq(*b) < 0.0001);
    points
}


pub fn panel(ui: &mut Ui, rect: Rect, tab: usize, label: &str, hint: &str, selected: bool, close_enabled: Option<bool>, drag: bool, floating: bool) -> (Response, bool) {
    let id = tab_widget_id(ui, tab);
    let response = ui.interact(rect, id, if drag { Sense::click_and_drag() } else { Sense::click() });
    response.widget_info(|| WidgetInfo::selected(Role::Tab, response.enabled(), selected, label));
    let marquee_id = id.with("marquee");
    let hover = !floating && response.enabled() && ui.ctx().dragged_id().is_none() && ui.rect_contains_pointer(rect);
    if !hover {
        ui.ctx().data_mut(|data| data.remove::<f64>(marquee_id));
    }
    let inset = if selected { rect.height() / 3.0 } else { 0.0 };
    let close_rect = Rect::from_center_size(egui::pos2(rect.right() - CLOSE_INSET - inset - CLOSE_SIZE.x / 2.0, rect.center().y), CLOSE_SIZE);
    if ui.is_rect_visible(rect) {
        let palette = theme::palette_of(ui);
        if floating {
            let bottom = rect.bottom() - 0.5;
            let mut mesh = egui::Mesh::default();
            for point in active_outline(rect, bottom) {
                let index = mesh.vertices.len() as u32;
                mesh.colored_vertex(point, palette.window_bg);
                mesh.colored_vertex(egui::pos2(point.x, bottom), palette.window_bg);
                if index > 0 {
                    mesh.add_triangle(index - 2, index, index - 1);
                    mesh.add_triangle(index, index + 1, index - 1);
                }
            }
            ui.painter().add(egui::Shape::mesh(mesh));
        }
        if !selected && (hover || response.has_focus()) {
            let fill = if hover { theme::ui_accent(palette.window_bg) } else { Color32::TRANSPARENT };
            let stroke = if response.has_focus() { ui.visuals().selection.stroke } else { Stroke::new(1.0, palette.pane_border) };
            let radius = theme::corner_radius_of(ui).saturating_sub(CORNER_RADIUS_REDUCTION);
            ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
        }
        let title_rect = Rect::from_min_max(rect.min + TITLE_PADDING + egui::vec2(inset, 0.0), egui::pos2(close_rect.left() - 2.0, rect.bottom() - TITLE_PADDING.y));
        let color = if selected || hover { palette.text } else { palette.weak_text };
        if selected && response.has_focus() {
            ui.painter().hline(title_rect.x_range(), title_rect.bottom(), ui.visuals().selection.stroke);
        }
        paint_title(ui, title_rect, label, color, marquee_id, hover);
    }
    if floating {
        if close_enabled.is_some() {
            paint_close_icon(ui, close_rect, 0.0);
        }
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        return (response, false);
    }

    let close = close_enabled.map(|enabled| close_button(ui, close_rect, id.with("close"), label, enabled));
    let response = if close.as_ref().is_some_and(Response::contains_pointer) { response } else { response.on_hover_text(hint) };
    (response, close.is_some_and(|close| close.clicked()))
}


fn paint_title(ui: &Ui, rect: Rect, label: &str, color: Color32, marquee_id: Id, hover: bool) {
    let mut job = LayoutJob::simple_singleline(label.to_owned(), TextStyle::Button.resolve(ui.style()), color);
    let painter = ui.painter().with_clip_rect(rect);
    if hover {
        let galley = painter.layout_job(job.clone());
        if galley.size().x > rect.width() {
            let now = ui.input(|input| input.time);
            let start = ui.ctx().data_mut(|data| *data.get_temp_mut_or_insert_with(marquee_id, || now));
            let distance = f64::from(galley.size().x - rect.width());
            let travel_time = distance / MARQUEE_SPEED;
            let cycle_time = travel_time + MARQUEE_END_PAUSE;
            let elapsed = (now - start).rem_euclid(cycle_time);
            let offset = (elapsed * MARQUEE_SPEED).min(distance) as f32;
            let pos = Pos2::new(rect.left() - offset, rect.center().y - galley.size().y / 2.0);
            painter.galley(pos, galley, color);
            if elapsed < travel_time {
                ui.ctx().request_repaint();
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(cycle_time - elapsed));
            }
            return;
        }
    }
    job.wrap = TextWrapping { max_width: rect.width(), max_rows: 1, break_anywhere: true, ..Default::default() };
    let galley = painter.layout_job(job);
    let pos = Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0);
    painter.galley(pos, galley, color);
}


fn close_button(ui: &mut Ui, rect: Rect, id: Id, label: &str, enabled: bool) -> Response {
    let builder = UiBuilder::new().id_salt(id).max_rect(rect);
    let ui = ui.new_child(if enabled { builder } else { builder.disabled() });
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, response.enabled(), format!("Close {label}")));
    let hover = response.enabled() && response.contains_pointer();
    let progress = ui.ctx().animate_bool_with_time(id, hover, CLOSE_FADE_SECONDS);
    if ui.is_rect_visible(rect) {
        if response.has_focus() {
            ui.painter().rect_stroke(rect, theme::corner_radius_of(&ui), Stroke::new(1.0, ui.visuals().selection.stroke.color), StrokeKind::Inside);
        }
        paint_close_icon(&ui, rect, progress);
    }
    response.on_hover_text("Close tab")
}


fn paint_close_icon(ui: &Ui, rect: Rect, progress: f32) {
    let (idle, hovered) = if ui.visuals().dark_mode { (190.0, 255.0) } else { (96.0, 0.0) };
    let tint = Color32::from_gray(egui::lerp(idle..=hovered, progress).round() as u8);
    icons::CLOSE.image().tint(tint).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(10.0)));
}


#[cfg(test)]
mod tests {
    use egui::epaint::{ClippedShape, Shape};
    use egui_kittest::{Harness, kittest::Queryable};

    use super::*;


    fn test_panel(ui: &mut Ui, tab: usize, label: &str, selected: bool, close_enabled: Option<bool>) {
        let rect = allocate_panel(ui);
        panel(ui, rect, tab, label, label, selected, close_enabled, false, false);
    }


    #[test]
    fn panels_have_equal_bounds_and_only_overflowing_titles_scroll() {
        let long = "A very long file name that does not fit inside the document tab.txt";
        let mut harness = Harness::builder().with_size(egui::vec2(640.0, 150.0)).with_step_dt(0.1)
            .with_context_setup(egui_extras::install_image_loaders)
            .build_ui(|ui| {
                ui.horizontal(|ui| {
                    test_panel(ui, 0, "Short.txt", true, Some(true));
                    test_panel(ui, 1, long, false, Some(true));
                });
            });
        let short_rect = harness.get_by_label("Short.txt").rect();
        let long_rect = harness.get_by_label(long).rect();
        let close_rect = harness.get_by_label(&format!("Close {long}")).rect();
        assert_eq!(short_rect.width(), 151.0);
        assert_eq!(short_rect.size(), long_rect.size());
        assert!(long_rect.contains_rect(close_rect));
        assert_eq!(close_rect.center().y, long_rect.center().y);
        assert!(close_rect.center().x > long_rect.center().x);
        assert_eq!(long_rect.right() - close_rect.right(), 6.0);

        let title_shapes = |shapes: &[ClippedShape], label: &str, tab_rect: Rect| -> Vec<_> {
            shapes.iter().filter_map(|shape| match &shape.shape {
                Shape::Text(text) if text.galley.job.text == label && tab_rect.contains_rect(shape.clip_rect) => {
                    Some((text.pos, text.galley.elided, shape.clip_rect, text.galley.size().x))
                }
                _ => None,
            }).collect()
        };
        let idle = title_shapes(&harness.output().shapes, long, long_rect);
        assert_eq!(idle.len(), 1);
        assert!(idle[0].1);
        assert!(idle[0].2.right() < close_rect.left());
        assert_eq!(idle[0].2.left() - long_rect.left(), 12.0);
        assert_eq!(idle[0].2.top() - long_rect.top(), 6.0);
        assert_eq!(long_rect.bottom() - idle[0].2.bottom(), 6.0);
        assert!(long_rect.height() < 34.0);

        harness.hover_at(long_rect.center());
        harness.step();
        harness.run_steps(6);
        let scrolling = title_shapes(&harness.output().shapes, long, long_rect);
        assert_eq!(scrolling.len(), 1);
        assert!(!scrolling[0].1);
        assert!(scrolling[0].0.x < idle[0].0.x);
        assert_eq!(scrolling[0].2, idle[0].2);

        let distance = scrolling[0].3 - scrolling[0].2.width();
        let travel_steps = (f64::from(distance) / MARQUEE_SPEED / 0.1).ceil() as usize;
        harness.run_steps(travel_steps - 6);
        let end = title_shapes(&harness.output().shapes, long, long_rect);
        assert!((end[0].0.x + distance - idle[0].0.x).abs() < 0.01);
        harness.run_steps(18);
        assert_eq!(title_shapes(&harness.output().shapes, long, long_rect), end);
        harness.run_steps(3);
        let restarted = title_shapes(&harness.output().shapes, long, long_rect);
        assert!(!restarted[0].1);
        assert!((restarted[0].0.x - idle[0].0.x).abs() <= (0.2 * MARQUEE_SPEED) as f32);
        harness.run_steps(6);
        assert!(title_shapes(&harness.output().shapes, long, long_rect)[0].0.x < restarted[0].0.x);

        let short = title_shapes(&harness.output().shapes, "Short.txt", short_rect);
        harness.hover_at(short_rect.center());
        harness.step();
        harness.run_steps(6);
        assert_eq!(title_shapes(&harness.output().shapes, "Short.txt", short_rect), short);
        assert_eq!(title_shapes(&harness.output().shapes, long, long_rect), idle);

        harness.hover_at(long_rect.center());
        harness.step();
        harness.run_steps(6);
        assert!(!title_shapes(&harness.output().shapes, long, long_rect)[0].1);
    }


    #[test]
    fn tabs_use_two_pixels_less_than_the_pane_and_button_radius() {
        for radius in [0, 1, 7] {
            let view = crate::config::ViewConfig { corner_radius: radius, ..Default::default() };
            let mut harness = Harness::builder().with_theme(egui::Theme::Light).with_context_setup(move |ctx| theme::apply_style(ctx, &view)).build_ui(|ui| {
                test_panel(ui, 0, "Tab.txt", false, None);
                let _ = ui.button("Button");
                let palette = theme::palette_of(ui);
                theme::pane_frame(&palette, theme::corner_radius_of(ui)).show(ui, |ui| ui.label("Pane"));
            });
            let tab = harness.get_by_label("Tab.txt").rect();
            let button = harness.get_by_label("Button").rect();
            let pane = harness.get_by_label("Pane").rect();
            assert!(!harness.output().shapes.iter().any(|shape| matches!(&shape.shape,
                Shape::Rect(rect) if rect.rect == tab && rect.stroke.width > 0.0)));
            harness.hover_at(tab.center());
            harness.step();
            let palette = theme::palette(egui::Theme::Light);
            assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape,
                Shape::Rect(rect) if rect.rect == tab && rect.fill == theme::ui_accent(palette.window_bg))));
            for (bounds, expected) in [(tab, radius.saturating_sub(2)), (button, radius), (pane, radius)] {
                assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape,
                    Shape::Rect(rect) if rect.rect.center().distance(bounds.center()) < 1.0
                        && rect.corner_radius == egui::CornerRadius::same(expected))));
            }
        }
    }


    #[test]
    fn active_outline_joins_the_separator_with_four_quarter_arcs() {
        let tab = Rect::from_min_size(egui::pos2(8.0, 4.0), egui::vec2(151.0, 25.0));
        let bottom = tab.bottom() - 0.5;
        let points = active_outline(tab, bottom);
        let top = tab.top() + 0.5;
        let radius = (bottom - top) / 2.0 - 2.0;
        let flat_left = egui::pos2(tab.left() + 2.0 * radius, top);
        let flat_right = egui::pos2(tab.right() - 2.0 * radius, top);
        assert!(points[0].distance(egui::pos2(tab.left(), bottom)) < 0.001);
        assert!(points.last().unwrap().distance(egui::pos2(tab.right(), bottom)) < 0.001);
        assert!(points.windows(2).any(|pair| pair[0].distance(flat_left) < 0.001 && pair[1].distance(flat_right) < 0.001));
        let lower_shoulder = egui::pos2(tab.left() + radius, bottom - radius);
        let upper_shoulder = egui::pos2(tab.left() + radius, top + radius);
        assert!(points.windows(2).any(|pair| pair[0].distance(lower_shoulder) < 0.001 && pair[1].distance(upper_shoulder) < 0.001));
        assert!(points.iter().skip(1).take(points.len() - 2).all(|p| p.y < bottom));
    }


    #[test]
    fn close_icon_tint_animates_in_both_directions() {
        let tint = |shapes: &[ClippedShape]| -> u8 {
            shapes.iter().find_map(|shape| match &shape.shape {
                Shape::Rect(rect) if rect.brush.is_some() => Some(rect.fill.r()),
                _ => None,
            }).unwrap()
        };
        for (theme, idle, hovered) in [(egui::Theme::Light, 96, 0), (egui::Theme::Dark, 190, 255)] {
            let mut harness = Harness::builder().with_step_dt(0.01).with_theme(theme)
                .with_context_setup(egui_extras::install_image_loaders)
                .build_ui(|ui| {
                    test_panel(ui, 0, "Short.txt", true, Some(true));
                });
            assert!(!harness.output().shapes.iter().any(|shape| matches!(shape.shape, Shape::Circle(_))));
            assert_eq!(tint(&harness.output().shapes), idle);
            harness.get_by_label("Close Short.txt").hover();
            harness.step();
            harness.run_steps(2);
            assert!((idle.min(hovered) + 1..idle.max(hovered)).contains(&tint(&harness.output().shapes)));
            harness.run_steps(3);
            assert_eq!(tint(&harness.output().shapes), hovered);

            harness.get_by_label("Short.txt").hover();
            harness.step();
            harness.run_steps(2);
            assert!((idle.min(hovered) + 1..idle.max(hovered)).contains(&tint(&harness.output().shapes)));
            harness.run_steps(3);
            assert_eq!(tint(&harness.output().shapes), idle);
        }
    }
}
