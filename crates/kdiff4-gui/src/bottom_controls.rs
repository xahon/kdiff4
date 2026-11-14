use egui::{CursorIcon, Image, Popup, PopupCloseBehavior, Rect, RectAlign, Role, ScrollArea, Sense, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, Vec2, WidgetInfo, WidgetText, pos2, vec2};

use crate::theme;
use crate::mirror::ConflictCounts;


const HOVER_FADE_SECONDS: f32 = 0.1;


pub fn conflict_label(ui: &mut Ui, counts: ConflictCounts) {
    let color = match (counts.unresolved, ui.visuals().dark_mode) {
        (0, false) => egui::Color32::from_rgb(0x17, 0x68, 0x2D),
        (0, true) => egui::Color32::from_rgb(0x8B, 0xDC, 0x9A),
        (_, false) => theme::LIGHT.red(),
        (_, true) => theme::DARK.red(),
    };
    let label = egui::RichText::new(format!("{} / {}", counts.unresolved, counts.total)).color(color);
    ui.add(egui::Label::new(label).extend()).on_hover_text("Unresolved conflicts / total conflicts");
}


pub struct SwitchPopup<'a> {
    pub title: Option<&'a str>,
    pub close_behavior: PopupCloseBehavior,
    pub max_height: Option<f32>,
}


pub fn bottom_switch<R>(
    ui: &mut Ui, hint: &str, selected: WidgetText, entries: impl IntoIterator<Item = impl AsRef<str>>, icon: Option<(Image<'_>, Vec2)>,
    popup: SwitchPopup<'_>, contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let value = selected.text().to_owned();
    let font = TextStyle::Button.resolve(ui.style());
    let galley = selected.into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, font.clone());
    let width = ui.fonts_mut(|fonts| entries.into_iter().fold(galley.size().x, |width, entry| {
        width.max(fonts.layout_no_wrap(entry.as_ref().to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size().x)
    }));
    let (icon_size, icon_gap) = icon.as_ref().map_or((Vec2::ZERO, 0.0), |(_, size)| (size.clone(), ui.spacing().icon_spacing));
    let padding = ui.spacing().button_padding;
    let size = vec2(width + icon_size.x + icon_gap, galley.size().y.max(icon_size.y)) + padding * 2.0;
    let (_, rect) = ui.allocate_space(vec2(size.x, size.y.max(ui.spacing().interact_size.y)));
    let response = ui.interact(rect, ui.make_persistent_id(popup.title.unwrap_or(hint)), Sense::click());
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(Role::ComboBox, response.enabled(), hint);
        info.current_text_value = Some(value.clone());
        info
    });

    let palette = theme::palette_of(ui);
    let popup_id = Popup::default_response_id(&response);
    let highlighted = response.hovered() || Popup::is_id_open(ui.ctx(), popup_id);
    let opacity = ui.ctx().animate_bool_with_time(response.id.with("hover_fade"), highlighted, HOVER_FADE_SECONDS);
    let fill = palette.bottom_switch_tint.gamma_multiply(opacity);
    if fill.a() > 0 {
        let stroke = Stroke::new(1.0, palette.bottom_switch_outline.gamma_multiply(opacity));
        ui.painter().rect(rect, theme::corner_radius_of(ui), fill, stroke, StrokeKind::Inside);
    }
    if let Some((icon, size)) = icon {
        let min = pos2(rect.left() + padding.x, rect.center().y - size.y / 2.0);
        icon.paint_at(ui, Rect::from_min_size(min, size));
    }
    let text_pos = pos2(rect.left() + padding.x + icon_size.x + icon_gap, rect.center().y - galley.size().y / 2.0);
    if response.has_focus() {
        ui.painter().hline(text_pos.x..=text_pos.x + galley.size().x, text_pos.y + galley.size().y, Stroke::new(1.0, palette.text));
    }
    ui.painter().galley(text_pos, galley, palette.text);

    let result = Popup::menu(&response)
        .align(RectAlign::TOP_END)
        .gap(2.0)
        .width(rect.width())
        .close_behavior(popup.close_behavior)
        .show(|ui| {
            ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
            if let Some(title) = popup.title {
                ui.strong(title);
                ui.separator();
            }
            if let Some(max_height) = popup.max_height {
                let height = max_height.min((ui.ctx().content_rect().height() - 72.0).max(ui.spacing().interact_size.y));
                ScrollArea::vertical().max_height(height).show(ui, contents).inner
            } else {
                contents(ui)
            }
        });
    response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(hint);
    result.map(|result| result.inner)
}


#[cfg(test)]
mod tests {
    use egui::{Color32, CornerRadius, epaint::Shape};
    use egui_kittest::{Harness, kittest::Queryable};

    use crate::config::ViewConfig;

    use super::*;


    #[test]
    fn a_bottom_switch_fades_its_configured_tint_and_outline_on_hover() {
        let mut view = ViewConfig { corner_radius: 7, ..Default::default() };
        view.light_colors.bottom_switch_tint = Some([221, 225, 237]);
        view.light_colors.bottom_switch_outline = Some([85, 105, 118]);
        let mut harness = Harness::builder().with_theme(egui::Theme::Light).with_step_dt(0.01)
            .with_context_setup(move |ctx| theme::apply_style(ctx, &view))
            .build_ui(|ui| {
                let popup = SwitchPopup { title: Some("Title"), close_behavior: PopupCloseBehavior::CloseOnClick, max_height: None };
                bottom_switch(ui, "Change value", "CR".into(), ["CR", "CRLF"], None, popup, |ui| ui.label("Choice"));
            });
        let bounds = harness.get_by_label("Change value").rect();
        let frame = |harness: &Harness<'_>| harness.output().shapes.iter().find_map(|shape| match &shape.shape {
            Shape::Rect(rect) if rect.rect == bounds && rect.stroke.width == 1.0 => Some((rect.fill, rect.stroke, rect.corner_radius)),
            _ => None,
        });
        let outside = pos2(bounds.right() + 20.0, bounds.center().y);
        harness.hover_at(outside);
        harness.run_steps(11);
        assert_eq!(frame(&harness), None);

        harness.hover_at(bounds.center());
        harness.run_steps(5);
        let (tint, outline, radius) = frame(&harness).unwrap();
        assert!((100..155).contains(&tint.a()));
        assert_eq!(outline.color.a(), tint.a());
        assert_eq!(radius, CornerRadius::same(7));
        assert_eq!(harness.output().platform_output.cursor_icon, CursorIcon::PointingHand);
        harness.run_steps(5);
        let full_tint = Color32::from_rgb(221, 225, 237);
        let full_outline = Color32::from_rgb(85, 105, 118);
        assert_eq!(frame(&harness), Some((full_tint, Stroke::new(1.0, full_outline), radius)));

        harness.hover_at(outside);
        harness.run_steps(5);
        let (tint, outline, _) = frame(&harness).unwrap();
        assert!((100..155).contains(&tint.a()));
        assert_eq!(outline.color.a(), tint.a());
        harness.run_steps(5);
        assert!(frame(&harness).is_none());

        harness.get_by_label("Change value").click();
        harness.run_steps(12);
        let title = harness.get_by_label("Title").rect();
        let choice = harness.get_by_label("Choice").rect();
        harness.hover_at(choice.center());
        harness.run_steps(10);
        assert_eq!(frame(&harness).unwrap().0, full_tint);
        let has_popup_frame = |shape: &Shape| matches!(shape,
            Shape::Rect(rect) if rect.corner_radius == CornerRadius::same(7) && rect.rect.contains_rect(title.union(choice)));
        assert!(harness.output().shapes.iter().any(|shape| match &shape.shape {
            Shape::Vec(shapes) => shapes.iter().any(has_popup_frame),
            shape => has_popup_frame(shape),
        }));
        assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape,
            Shape::LineSegment { points, .. } if points[0].y == points[1].y && points[0].y > title.bottom() && points[0].y < choice.top())));
    }
}
