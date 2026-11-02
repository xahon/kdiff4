use egui::{Color32, CornerRadius, FontId, Frame, Margin, Stroke, Style, TextStyle, Theme, Ui};

use crate::config::ViewConfig;


/// The colors the GUI draws with, one set per theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub window_bg: Color32,
    pub pane_bg: Color32,
    pub pane_border: Color32,
    pub pane_header_bg: Color32,
    pub text: Color32,
    pub weak_text: Color32,
    pub caret: Color32,
    pub selection: Color32,
}


pub const LIGHT: Palette = Palette {
    window_bg: Color32::from_rgb(0xEC, 0xEE, 0xF1),
    pane_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    pane_border: Color32::from_rgb(0x9A, 0xA2, 0xAE),
    pane_header_bg: Color32::from_rgb(0xDE, 0xE3, 0xE9),
    text: Color32::from_rgb(0x16, 0x19, 0x1D),
    weak_text: Color32::from_rgb(0x55, 0x5D, 0x69),
    caret: Color32::from_rgb(0x1F, 0x5F, 0xD1),
    selection: Color32::from_rgb(0xB8, 0xD4, 0xFF),
};


pub const DARK: Palette = Palette {
    window_bg: Color32::from_rgb(0x1E, 0x21, 0x26),
    pane_bg: Color32::from_rgb(0x12, 0x14, 0x17),
    pane_border: Color32::from_rgb(0x55, 0x5C, 0x67),
    pane_header_bg: Color32::from_rgb(0x2A, 0x2F, 0x36),
    text: Color32::from_rgb(0xE8, 0xEA, 0xED),
    weak_text: Color32::from_rgb(0xA0, 0xA8, 0xB3),
    caret: Color32::from_rgb(0x7A, 0xB0, 0xFF),
    selection: Color32::from_rgb(0x2C, 0x4A, 0x78),
};


pub fn palette(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Light => &LIGHT,
        Theme::Dark => &DARK,
    }
}


pub fn palette_of(ui: &Ui) -> &'static Palette {
    palette(if ui.visuals().dark_mode { Theme::Dark } else { Theme::Light })
}


/// Styles both themes, so switching themes later needs no other change.
pub fn apply_style(ctx: &egui::Context, view: &ViewConfig) {
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| configure_style(style, palette(theme), view));
    }
}


pub fn configure_style(style: &mut Style, palette: &Palette, view: &ViewConfig) {
    let visuals = &mut style.visuals;
    visuals.panel_fill = palette.window_bg;
    visuals.window_fill = palette.window_bg;
    visuals.window_stroke = Stroke::new(1.0, palette.pane_border);
    visuals.extreme_bg_color = palette.pane_bg;
    visuals.faint_bg_color = palette.pane_header_bg;
    visuals.widgets.noninteractive.fg_stroke.color = palette.text;
    visuals.widgets.noninteractive.bg_stroke.color = palette.pane_border;
    visuals.widgets.inactive.fg_stroke.color = palette.text;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.pane_border);
    visuals.weak_text_color = Some(palette.weak_text);
    let radius = CornerRadius::same(view.corner_radius.clone());
    visuals.window_corner_radius = radius.clone();
    visuals.menu_corner_radius = radius.clone();
    for widget in [&mut visuals.widgets.noninteractive, &mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active, &mut visuals.widgets.open] {
        widget.corner_radius = radius.clone();
    }
    visuals.text_options.font_hinting = view.font_hinting;
    visuals.text_options.subpixel_binning = view.subpixel_positioning;
    style.text_styles.insert(TextStyle::Monospace, FontId::monospace(view.font_size));
}


/// The configured corner radius, as applied to the style of `ui`.
pub fn corner_radius_of(ui: &Ui) -> u8 {
    ui.visuals().window_corner_radius.nw
}


/// A bordered box for one pane. No inner margin, so the header strip reaches the border.
pub fn pane_frame(palette: &Palette, radius: u8) -> Frame {
    Frame::new()
        .fill(palette.pane_bg)
        .stroke(Stroke::new(1.0, palette.pane_border))
        .corner_radius(radius)
}


/// The header strip at the top of a pane; its top corners sit just inside the pane border.
pub fn pane_header_frame(palette: &Palette, radius: u8) -> Frame {
    let radius = radius.saturating_sub(1);
    Frame::new()
        .fill(palette.pane_header_bg)
        .inner_margin(Margin::symmetric(6, 3))
        .corner_radius(CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 })
}


pub fn pane_body_frame() -> Frame {
    Frame::new().inner_margin(Margin::same(4))
}


#[cfg(test)]
mod tests {
    use super::*;


    /// WCAG 2 contrast ratio, from 1 (same luminance) to 21 (black on white).
    fn contrast_ratio(first: Color32, second: Color32) -> f32 {
        let (lighter, darker) = {
            let a = relative_luminance(first);
            let b = relative_luminance(second);
            if a >= b { (a, b) } else { (b, a) }
        };
        (lighter + 0.05) / (darker + 0.05)
    }


    fn relative_luminance(color: Color32) -> f32 {
        let channel = |value: u8| {
            let c = f32::from(value) / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }


    #[test]
    fn contrast_ratio_extremes() {
        assert!((contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Color32::GRAY, Color32::GRAY) - 1.0).abs() < 0.001);
    }


    #[test]
    fn pane_text_has_high_contrast_in_both_themes() {
        for palette in [&LIGHT, &DARK] {
            assert!(contrast_ratio(palette.text, palette.pane_bg) >= 7.0, "{palette:?}");
            assert!(contrast_ratio(palette.text, palette.pane_header_bg) >= 7.0, "{palette:?}");
            assert!(contrast_ratio(palette.text, palette.window_bg) >= 7.0, "{palette:?}");
            assert!(contrast_ratio(palette.weak_text, palette.window_bg) >= 4.5, "{palette:?}");
            assert!(contrast_ratio(palette.caret, palette.pane_bg) >= 4.5, "{palette:?}");
            assert!(contrast_ratio(palette.text, palette.selection) >= 7.0, "{palette:?}");
        }
    }


    #[test]
    fn pane_border_stands_out_from_both_backgrounds() {
        for palette in [&LIGHT, &DARK] {
            assert!(contrast_ratio(palette.pane_border, palette.pane_bg) >= 2.0, "{palette:?}");
            assert!(contrast_ratio(palette.pane_border, palette.window_bg) >= 1.8, "{palette:?}");
        }
    }


    #[test]
    fn palettes_follow_their_theme() {
        assert!(relative_luminance(LIGHT.pane_bg) > relative_luminance(LIGHT.text));
        assert!(relative_luminance(DARK.pane_bg) < relative_luminance(DARK.text));
        assert_eq!(palette(Theme::Light), &LIGHT);
        assert_eq!(palette(Theme::Dark), &DARK);
    }


    #[test]
    fn configure_style_applies_view_settings() {
        let mut style = Style::default();
        let view = ViewConfig { font_family: None, font_size: 17.0, font_hinting: false, subpixel_positioning: false, corner_radius: 0 };
        configure_style(&mut style, &LIGHT, &view);
        assert_eq!(style.text_styles[&TextStyle::Monospace], FontId::monospace(17.0));
        assert!(!style.visuals.text_options.font_hinting);
        assert!(!style.visuals.text_options.subpixel_binning);
        assert_eq!(style.visuals.extreme_bg_color, LIGHT.pane_bg);
        assert_eq!(style.visuals.panel_fill, LIGHT.window_bg);
        assert_eq!(style.visuals.widgets.inactive.bg_stroke, Stroke::new(1.0, LIGHT.pane_border));
        assert_eq!(style.visuals.widgets.inactive.fg_stroke.color, LIGHT.text);
        assert_eq!(style.visuals.window_corner_radius, CornerRadius::ZERO);
        assert_eq!(style.visuals.widgets.hovered.corner_radius, CornerRadius::ZERO);
        assert_eq!(style.visuals.menu_corner_radius, CornerRadius::ZERO);
    }
}
