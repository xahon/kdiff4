use egui::{Color32, CornerRadius, FontId, Frame, Id, Margin, Stroke, Style, TextStyle, Theme, ThemePreference, Ui};
use kdiff4_protocol::Document;

use crate::char_diff::CharStyle;
use crate::config::{ThemeChoice, ThemeColors, ViewConfig};

/// Share of a document's color that its light fill (pane header, gutter blocks) keeps when blended into the pane background, in percent.
const FILL_SHARE: u16 = 20;
/// The same for the stronger fill of active gutter blocks and char-diff backgrounds.
const ACTIVE_SHARE: u16 = 50;

/// The colors of one document: its pane header, which also fills its gutter blocks, and the block border; `gutter_active` fills the blocks of the current region and the char-diff backgrounds. The red of a character or block that differs from every other document has a set too. All three come from one color, `gutter_border`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocumentColors {
    pub header: Color32,
    pub gutter_border: Color32,
    pub gutter_active: Color32,
}

impl DocumentColors {
    /// The set of the one color `base`, which is the border; the fills are `base` blended into `background`.
    pub const fn derive(base: Color32, background: Color32) -> DocumentColors {
        DocumentColors {
            header: blend(base, background, FILL_SHARE),
            gutter_border: base,
            gutter_active: blend(base, background, ACTIVE_SHARE),
        }
    }
}

const fn blend(over: Color32, under: Color32, share: u16) -> Color32 {
    Color32::from_rgb(mix(over.r(), under.r(), share), mix(over.g(), under.g(), share), mix(over.b(), under.b(), share))
}

const fn mix(over: u8, under: u8, share: u16) -> u8 {
    ((over as u16 * share + under as u16 * (100 - share) + 50) / 100) as u8
}

/// The colors the GUI draws with, one set per theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub window_bg: Color32,
    pub pane_bg: Color32,
    pub pane_border: Color32,
    pub pane_header_bg: Color32,
    /// Text of menus, dialogs, buttons and labels.
    pub text: Color32,
    pub pane_text: Color32,
    pub weak_text: Color32,
    pub caret: Color32,
    pub selection: Color32,
    /// The border of each region box.
    pub region_border: Color32,
    /// Hatching of void regions.
    pub void_hatch: Color32,
    /// A, B and C; D's header and D's blocks of hand-edited or unknown origin use `output`.
    pub sources: [DocumentColors; 3],
    pub output: DocumentColors,
    /// The red of text and gutter blocks that differ from every other document, and of the `<Merge Conflict>` marker of an unresolved conflict in D.
    pub conflict: DocumentColors,
    /// The Fold Ellipsis that stands for the hidden lines of a fold, and the button of a fold.
    pub fold_ellipsis: Color32,
}

const LIGHT_PANE_BG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const DARK_PANE_BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x17);

pub const LIGHT: Palette = Palette {
    window_bg: Color32::from_rgb(0xEC, 0xEE, 0xF1),
    pane_bg: LIGHT_PANE_BG,
    pane_border: Color32::from_rgb(0x9A, 0xA2, 0xAE),
    pane_header_bg: Color32::from_rgb(0xDE, 0xE3, 0xE9),
    text: Color32::BLACK,
    pane_text: Color32::BLACK,
    weak_text: Color32::from_rgb(0x55, 0x5D, 0x69),
    caret: Color32::from_rgb(0x1F, 0x5F, 0xD1),
    selection: Color32::from_rgb(0xB8, 0xD4, 0xFF),
    region_border: Color32::from_rgb(0x5A, 0x66, 0x76),
    void_hatch: Color32::from_rgb(0xC4, 0xCA, 0xD3),
    sources: [
        DocumentColors::derive(Color32::from_rgb(0x2F, 0x6B, 0xC4), LIGHT_PANE_BG),
        DocumentColors::derive(Color32::from_rgb(0x2E, 0x8B, 0x43), LIGHT_PANE_BG),
        DocumentColors::derive(Color32::from_rgb(0x7A, 0x4C, 0xC2), LIGHT_PANE_BG),
    ],
    output: DocumentColors {
        header: Color32::from_rgb(0xDE, 0xE3, 0xE9),
        gutter_border: Color32::from_rgb(0x6B, 0x74, 0x80),
        gutter_active: Color32::from_rgb(0xB4, 0xBC, 0xC7),
    },
    conflict: DocumentColors::derive(Color32::from_rgb(0xB0, 0x00, 0x00), LIGHT_PANE_BG),
    fold_ellipsis: Color32::from_rgb(0xD9, 0x6C, 0x00),
};

pub const DARK: Palette = Palette {
    window_bg: Color32::from_rgb(0x1E, 0x21, 0x26),
    pane_bg: DARK_PANE_BG,
    pane_border: Color32::from_rgb(0x55, 0x5C, 0x67),
    pane_header_bg: Color32::from_rgb(0x2A, 0x2F, 0x36),
    text: Color32::from_rgb(0xE8, 0xEA, 0xED),
    pane_text: Color32::from_rgb(0xE8, 0xEA, 0xED),
    weak_text: Color32::from_rgb(0xA0, 0xA8, 0xB3),
    caret: Color32::from_rgb(0x7A, 0xB0, 0xFF),
    selection: Color32::from_rgb(0x2C, 0x4A, 0x78),
    region_border: Color32::from_rgb(0x8A, 0x94, 0xA3),
    void_hatch: Color32::from_rgb(0x3C, 0x42, 0x4B),
    sources: [
        DocumentColors::derive(Color32::from_rgb(0x6F, 0xA4, 0xF0), DARK_PANE_BG),
        DocumentColors::derive(Color32::from_rgb(0x6C, 0xCB, 0x80), DARK_PANE_BG),
        DocumentColors::derive(Color32::from_rgb(0xB4, 0x8E, 0xF0), DARK_PANE_BG),
    ],
    output: DocumentColors {
        header: Color32::from_rgb(0x2A, 0x2F, 0x36),
        gutter_border: Color32::from_rgb(0x8A, 0x92, 0x9D),
        gutter_active: Color32::from_rgb(0x4A, 0x52, 0x5D),
    },
    conflict: DocumentColors::derive(Color32::from_rgb(0xFF, 0xA3, 0xA3), DARK_PANE_BG),
    fold_ellipsis: Color32::from_rgb(0xFF, 0x9F, 0x3A),
};

impl Palette {
    /// The red of text that differs from every other document and of the conflict marker.
    pub fn red(&self) -> Color32 {
        self.conflict.gutter_border
    }

    /// The colors a block or run marked with `style` is drawn with.
    pub fn style_colors(&self, style: CharStyle) -> &DocumentColors {
        match style {
            CharStyle::Red => &self.conflict,
            CharStyle::Document(document) => self.colors(document),
        }
    }

    /// The hatching of a void in the pane of `document`: that source document's color; D and views of no source document get the neutral `void_hatch`.
    pub fn void_hatch_of(&self, document: Option<Document>) -> Color32 {
        match document {
            Some(Document::D) | None => self.void_hatch,
            Some(source) => self.colors(source).gutter_active,
        }
    }

    pub fn colors(&self, document: Document) -> &DocumentColors {
        match document {
            Document::A => &self.sources[0],
            Document::B => &self.sources[1],
            Document::C => &self.sources[2],
            Document::D => &self.output,
        }
    }
}

pub fn palette(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Light => &LIGHT,
        Theme::Dark => &DARK,
    }
}

/// The theme's palette with the user's colors, as `apply_style` stored it.
pub fn palette_of(ui: &Ui) -> Palette {
    let theme = if ui.visuals().dark_mode { Theme::Dark } else { Theme::Light };
    ui.ctx().data(|data| data.get_temp::<Palette>(palette_id(theme))).unwrap_or_else(|| palette(theme).clone())
}

fn palette_id(theme: Theme) -> Id {
    Id::unique(("kdiff4_palette", theme == Theme::Dark))
}

/// The theme's palette with the colors the user picked for it.
pub fn customized(theme: Theme, colors: &ThemeColors) -> Palette {
    let mut palette = palette(theme).clone();
    if let Some([r, g, b]) = colors.ui_text {
        palette.text = Color32::from_rgb(r, g, b);
    }
    if let Some([r, g, b]) = colors.pane_text {
        palette.pane_text = Color32::from_rgb(r, g, b);
    }
    let background = palette.pane_bg;
    let derived = |[r, g, b]: [u8; 3]| DocumentColors::derive(Color32::from_rgb(r, g, b), background);
    for (document, picked) in palette.sources.iter_mut().zip([colors.document_a, colors.document_b, colors.document_c]) {
        if let Some(rgb) = picked {
            *document = derived(rgb);
        }
    }
    if let Some(rgb) = colors.red {
        palette.conflict = derived(rgb);
    }
    if let Some([r, g, b]) = colors.fold_ellipsis {
        palette.fold_ellipsis = Color32::from_rgb(r, g, b);
    }
    palette
}

pub fn preference(choice: ThemeChoice) -> ThemePreference {
    match choice {
        ThemeChoice::Light => ThemePreference::Light,
        ThemeChoice::Dark => ThemePreference::Dark,
        ThemeChoice::System => ThemePreference::System,
    }
}

/// Styles both themes and stores their palettes, so switching themes later needs no other change.
pub fn apply_style(ctx: &egui::Context, view: &ViewConfig) {
    for theme in [Theme::Light, Theme::Dark] {
        let palette = customized(theme, view.colors(theme == Theme::Dark));
        ctx.style_mut_of(theme, |style| configure_style(style, &palette, view));
        ctx.data_mut(|data| data.insert_temp(palette_id(theme), palette));
    }
}

pub fn configure_style(style: &mut Style, palette: &Palette, view: &ViewConfig) {
    let visuals = &mut style.visuals;
    visuals.panel_fill = palette.window_bg;
    visuals.window_fill = palette.window_bg;
    visuals.window_stroke = Stroke::new(1.0, palette.pane_border);
    visuals.extreme_bg_color = palette.pane_bg;
    visuals.faint_bg_color = palette.pane_header_bg;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.fg_stroke.color = palette.text;
    }
    visuals.widgets.noninteractive.bg_stroke.color = palette.pane_border;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.pane_border);
    visuals.weak_text_color = Some(palette.weak_text);
    let radius = CornerRadius::same(view.corner_radius.clone());
    visuals.window_corner_radius = radius.clone();
    visuals.menu_corner_radius = radius.clone();
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
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
    Frame::new().fill(palette.pane_bg).stroke(Stroke::new(1.0, palette.pane_border)).corner_radius(radius)
}

/// The header strip at the top of a pane, in the document's color; its top corners sit just inside the pane border.
pub fn pane_header_frame(palette: &Palette, document: Document, radius: u8) -> Frame {
    let radius = radius.saturating_sub(1);
    Frame::new()
        .fill(palette.colors(document).header)
        .inner_margin(Margin::symmetric(6, 3))
        .corner_radius(CornerRadius {
            nw: radius,
            ne: radius,
            sw: 0,
            se: 0,
        })
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
            for document in Document::ALL {
                assert!(contrast_ratio(palette.text, palette.colors(document).header) >= 7.0, "{document:?} in {palette:?}");
                assert!(contrast_ratio(palette.weak_text, palette.colors(document).header) >= 4.5, "{document:?} in {palette:?}");
            }
            assert!(contrast_ratio(palette.text, palette.window_bg) >= 7.0, "{palette:?}");
            assert!(contrast_ratio(palette.weak_text, palette.window_bg) >= 4.5, "{palette:?}");
            assert!(contrast_ratio(palette.caret, palette.pane_bg) >= 4.5, "{palette:?}");
            assert!(contrast_ratio(palette.text, palette.selection) >= 7.0, "{palette:?}");
            assert!(contrast_ratio(palette.pane_text, palette.pane_bg) >= 7.0, "{palette:?}");
            assert!(contrast_ratio(palette.pane_text, palette.selection) >= 7.0, "{palette:?}");
        }
    }

    #[test]
    fn region_boxes_and_voids_stand_out() {
        for palette in [&LIGHT, &DARK] {
            assert!(contrast_ratio(palette.region_border, palette.pane_bg) >= 3.0, "{palette:?}");
            assert!(contrast_ratio(palette.void_hatch, palette.pane_bg) >= 1.4, "{palette:?}");
        }
    }

    #[test]
    fn voids_are_hatched_in_the_color_of_their_source_document() {
        for palette in [&LIGHT, &DARK] {
            for document in [Document::A, Document::B, Document::C] {
                let hatch = palette.void_hatch_of(Some(document.clone()));
                assert_eq!(hatch, palette.colors(document).gutter_active);
                assert!(contrast_ratio(hatch, palette.pane_bg) >= 1.4, "{document:?} in {palette:?}");
            }
            assert_eq!(palette.void_hatch_of(Some(Document::D)), palette.void_hatch);
            assert_eq!(palette.void_hatch_of(None), palette.void_hatch);
        }
    }

    #[test]
    fn text_on_the_default_jump_highlight_has_high_contrast() {
        let [r, g, b, a] = crate::config::DEFAULT_CURRENT_REGION_COLOR;
        let blend = |over: u8, under: u8| ((over as f32 * a as f32 + under as f32 * (255 - a) as f32) / 255.0).round() as u8;
        for palette in [&LIGHT, &DARK] {
            let background = Color32::from_rgb(blend(r, palette.pane_bg.r()), blend(g, palette.pane_bg.g()), blend(b, palette.pane_bg.b()));
            assert!(contrast_ratio(palette.text, background) >= 7.0, "{background:?} in {palette:?}");
            assert!(
                contrast_ratio(background, palette.pane_bg) >= 1.1,
                "{background:?} must differ from the pane in {palette:?}"
            );
        }
    }

    #[test]
    fn default_red_is_readable_on_the_pane_the_selection_and_the_jump_highlight() {
        let [r, g, b, a] = crate::config::DEFAULT_CURRENT_REGION_COLOR;
        let blend = |over: u8, under: u8| ((over as f32 * a as f32 + under as f32 * (255 - a) as f32) / 255.0).round() as u8;
        for palette in [LIGHT, DARK] {
            let highlight = Color32::from_rgb(blend(r, palette.pane_bg.r()), blend(g, palette.pane_bg.g()), blend(b, palette.pane_bg.b()));
            for background in [palette.pane_bg, palette.selection, highlight] {
                assert!(contrast_ratio(palette.red(), background) >= 4.5, "{:?} on {background:?} in {palette:?}", palette.red());
            }
        }
    }

    #[test]
    fn gutters_and_conflict_markers_are_readable() {
        for palette in [LIGHT, DARK] {
            assert!(contrast_ratio(palette.red(), palette.pane_bg) >= 4.5, "{palette:?}");
            for colors in palette.sources.iter().chain([&palette.output, &palette.conflict]) {
                assert!(contrast_ratio(colors.gutter_border, colors.header) >= 2.0, "{colors:?} in {palette:?}");
                assert!(contrast_ratio(colors.gutter_border, colors.gutter_active) >= 1.5, "{colors:?} in {palette:?}");
                assert!(
                    contrast_ratio(colors.gutter_active, colors.header) >= 1.3,
                    "active blocks must stand out: {colors:?} in {palette:?}"
                );
                assert!(contrast_ratio(palette.text, colors.gutter_active) >= 4.5, "D's origin letters: {colors:?} in {palette:?}");
            }
        }
    }

    #[test]
    fn char_diff_fills_of_the_documents_stand_out_from_the_pane() {
        for palette in [LIGHT, DARK] {
            for colors in palette.sources.iter() {
                assert!(contrast_ratio(colors.gutter_active, palette.pane_bg) >= 1.5, "{colors:?} in {palette:?}");
            }
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
        assert_eq!(LIGHT.text, Color32::BLACK);
        assert_eq!(LIGHT.pane_text, Color32::BLACK);
        assert_eq!(palette(Theme::Light), &LIGHT);
        assert_eq!(palette(Theme::Dark), &DARK);
    }

    #[test]
    fn documents_have_their_own_colors() {
        for palette in [&LIGHT, &DARK] {
            assert_eq!(palette.colors(Document::D).header, palette.pane_header_bg);
            let headers = [
                palette.colors(Document::A).header,
                palette.colors(Document::B).header,
                palette.colors(Document::C).header,
                palette.pane_header_bg,
            ];
            for (index, header) in headers.iter().enumerate() {
                assert!(!headers[..index].contains(header), "{palette:?}");
            }
        }
    }

    #[test]
    fn configure_style_applies_view_settings() {
        let mut style = Style::default();
        let view = ViewConfig {
            font_family: None,
            font_size: 17.0,
            font_hinting: false,
            subpixel_positioning: false,
            corner_radius: 0,
            ..ViewConfig::default()
        };
        configure_style(&mut style, &LIGHT, &view);
        assert_eq!(style.text_styles[&TextStyle::Monospace], FontId::monospace(17.0));
        assert!(!style.visuals.text_options.font_hinting);
        assert!(!style.visuals.text_options.subpixel_binning);
        assert_eq!(style.visuals.extreme_bg_color, LIGHT.pane_bg);
        assert_eq!(style.visuals.panel_fill, LIGHT.window_bg);
        assert_eq!(style.visuals.widgets.inactive.bg_stroke, Stroke::new(1.0, LIGHT.pane_border));
        assert_eq!(style.visuals.widgets.inactive.fg_stroke.color, LIGHT.text);
        assert_eq!(style.visuals.widgets.hovered.fg_stroke.color, LIGHT.text);
        assert_eq!(style.visuals.widgets.noninteractive.fg_stroke.color, LIGHT.text);
        assert_eq!(style.visuals.window_corner_radius, CornerRadius::ZERO);
        assert_eq!(style.visuals.widgets.hovered.corner_radius, CornerRadius::ZERO);
        assert_eq!(style.visuals.menu_corner_radius, CornerRadius::ZERO);
    }

    #[test]
    fn customized_palette_takes_only_the_picked_colors() {
        assert_eq!(customized(Theme::Light, &ThemeColors::default()), LIGHT);
        let colors = ThemeColors {
            ui_text: Some([1, 2, 3]),
            ..ThemeColors::default()
        };
        let palette = customized(Theme::Dark, &colors);
        assert_eq!(palette.text, Color32::from_rgb(1, 2, 3));
        assert_eq!(palette.pane_text, DARK.pane_text);
        assert_eq!(palette.pane_bg, DARK.pane_bg);
        let colors = ThemeColors {
            pane_text: Some([4, 5, 6]),
            ..ThemeColors::default()
        };
        assert_eq!(customized(Theme::Light, &colors).pane_text, Color32::from_rgb(4, 5, 6));
    }

    #[test]
    fn derived_colors_keep_the_base_as_border_and_fade_into_the_background() {
        let base = Color32::from_rgb(0x20, 0x40, 0x80);
        let colors = DocumentColors::derive(base, Color32::WHITE);
        assert_eq!(colors.gutter_border, base);
        assert_eq!(DocumentColors::derive(base, base).header, base);
        assert_eq!(DocumentColors::derive(base, base).gutter_active, base);
        assert!(relative_luminance(colors.gutter_active) > relative_luminance(base));
        assert!(relative_luminance(colors.header) > relative_luminance(colors.gutter_active));
        assert!(relative_luminance(colors.header) < relative_luminance(Color32::WHITE));
    }

    #[test]
    fn picked_document_and_red_colors_replace_their_whole_set() {
        let colors = ThemeColors {
            document_a: Some([10, 20, 30]),
            document_b: None,
            document_c: Some([40, 50, 60]),
            red: Some([200, 0, 0]),
            ..ThemeColors::default()
        };
        for palette in [&LIGHT, &DARK] {
            let theme = if palette == &LIGHT { Theme::Light } else { Theme::Dark };
            let customized = customized(theme, &colors);
            assert_eq!(customized.colors(Document::A), &DocumentColors::derive(Color32::from_rgb(10, 20, 30), palette.pane_bg));
            assert_eq!(customized.colors(Document::B), palette.colors(Document::B));
            assert_eq!(customized.colors(Document::C), &DocumentColors::derive(Color32::from_rgb(40, 50, 60), palette.pane_bg));
            assert_eq!(customized.conflict, DocumentColors::derive(Color32::from_rgb(200, 0, 0), palette.pane_bg));
            assert_eq!(customized.red(), Color32::from_rgb(200, 0, 0));
            assert_eq!(customized.colors(Document::D), palette.colors(Document::D));
        }
    }

    #[test]
    fn style_colors_follow_the_style() {
        assert_eq!(LIGHT.style_colors(CharStyle::Red), &LIGHT.conflict);
        assert_eq!(LIGHT.style_colors(CharStyle::Document(Document::B)), LIGHT.colors(Document::B));
    }

    #[test]
    fn theme_choices_map_to_preferences() {
        assert_eq!(preference(ThemeChoice::Light), ThemePreference::Light);
        assert_eq!(preference(ThemeChoice::Dark), ThemePreference::Dark);
        assert_eq!(preference(ThemeChoice::System), ThemePreference::System);
    }
}
