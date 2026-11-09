use std::ops::Range;

use egui::{Color32, CursorIcon, Rect, Sense, Stroke, StrokeKind, Ui, pos2};
use kdiff4_protocol::Document;
use kdiff4_protocol::engine::{HunkChange, MergeStatus, RegionStatus};

use crate::mirror::Mirror;
use crate::panes::PaneState;
use crate::rows::Rows;
use crate::text_view::Viewport;
use crate::theme::Palette;

/// Width of the map, in pixels.
pub const WIDTH: f32 = 28.0;
/// Space between the map's border and its blocks.
const INSET: f32 = 3.0;
/// Shortest block drawn, so a one-line difference in a long file stays visible.
const MIN_BLOCK_HEIGHT: f32 = 2.0;
/// Share of the caret color the viewport rectangle is filled with.
const VIEWPORT_FILL_SHARE: f32 = 0.2;

/// What a block of the map stands for, which picks its color.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Source(Document),
    Edited,
    Conflict,
}

/// A region that differs, as rows of the source panes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MapBlock {
    region: usize,
    rows: Range<usize>,
    kind: Kind,
}

/// The color group of a region that differs: the document it differs in, the conflict red, or the neutral color for what both B and C changed alike and for hand-edited regions.
fn kind_of(status: &RegionStatus) -> Kind {
    match status {
        RegionStatus::Diff(HunkChange::Unchanged) | RegionStatus::Merge(MergeStatus::Unchanged | MergeStatus::ChangedAlike) => Kind::Edited,
        RegionStatus::Diff(_) | RegionStatus::Merge(MergeStatus::ChangedInB) => Kind::Source(Document::B),
        RegionStatus::Merge(MergeStatus::ChangedInC) => Kind::Source(Document::C),
        RegionStatus::Merge(MergeStatus::Conflict | MergeStatus::WhitespaceConflict) => Kind::Conflict,
    }
}

/// The regions that stand out in the rows of the source panes, in order.
fn blocks(mirror: &Mirror) -> Vec<MapBlock> {
    let Some(rows) = mirror.rows(Document::A).map(Rows::Mapped) else {
        return Vec::new();
    };
    mirror
        .comparison()
        .regions
        .iter()
        .enumerate()
        .filter_map(|(region, data)| {
            let range = rows.region_rows(region).filter(|range| !range.is_empty())?;
            rows.row(range.start).filter(|row| row.boxed)?;
            Some(MapBlock {
                region,
                rows: range,
                kind: kind_of(&data.status),
            })
        })
        .collect()
}

fn color_of(palette: &Palette, kind: &Kind) -> Color32 {
    match kind {
        Kind::Source(document) => palette.colors(document.clone()).gutter_border,
        Kind::Edited => palette.output.gutter_border,
        Kind::Conflict => palette.red(),
    }
}

/// Where `rows` lie in the map, as distances from its top in pixels, for a map `height` pixels tall that stands for a scroll area of `viewport`.
fn span_of(rows: &Range<usize>, viewport: &Viewport, height: f32) -> (f32, f32) {
    let scale = height / viewport.content_height.max(1.0);
    let top = rows.start as f32 * viewport.row_height * scale;
    let bottom = rows.end as f32 * viewport.row_height * scale;
    (top, bottom.max(top + MIN_BLOCK_HEIGHT))
}

/// The scroll offset that centers the view on the point `y` pixels below the top of a map `height` pixels tall.
fn offset_at(y: f32, height: f32, viewport: &Viewport) -> f32 {
    if height <= 0.0 || !y.is_finite() {
        return viewport.offset;
    }
    let centered = y / height * viewport.content_height - viewport.height / 2.0;
    centered.clamp(0.0, viewport.max_offset())
}

/// The whole document as one narrow column: a block for every region that differs, and a rectangle for what the source panes show. A press or drag in it scrolls the source panes there, like a scroll bar. While the panes scroll independently, it shows and scrolls the scroll leader.
pub fn show(ui: &mut Ui, mirror: &Mirror, state: &mut PaneState, palette: &Palette) {
    let (outer, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 0.0, palette.pane_bg);
    painter.rect_stroke(outer, 0.0, Stroke::new(1.0, palette.pane_border), StrokeKind::Inside);
    let viewports = state.viewports.clone();
    let (index, scroll) = state.map_scroll(mirror.is_aligned());
    let Some(viewport) = viewports[index].clone() else {
        return;
    };
    let map = outer.shrink2(egui::vec2(INSET, INSET));
    if map.height() <= 0.0 || viewport.row_height <= 0.0 {
        return;
    }
    if response.is_pointer_button_down_on()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let offset = offset_at(pointer.y - map.top(), map.height(), &viewport);
        scroll.set(offset);
    }
    let viewport = Viewport {
        offset: scroll.offset.clone(),
        ..viewport
    };
    for block in blocks(mirror) {
        let (top, bottom) = span_of(&block.rows, &viewport, map.height());
        let rect = Rect::from_min_max(pos2(map.left(), map.top() + top), pos2(map.right(), (map.top() + bottom).min(map.bottom())));
        painter.rect_filled(rect, 0.0, color_of(palette, &block.kind));
        if mirror.current_region() == Some(block.region) {
            painter.rect_stroke(rect.expand(1.0), 0.0, Stroke::new(1.5, palette.text), StrokeKind::Outside);
        }
    }
    let scale = map.height() / viewport.content_height.max(1.0);
    let top = map.top() + viewport.offset * scale;
    let view = Rect::from_min_max(pos2(outer.left() + 1.0, top), pos2(outer.right() - 1.0, (top + viewport.height * scale).min(map.bottom())));
    painter.rect_filled(view, 0.0, palette.caret.gamma_multiply(VIEWPORT_FILL_SHARE));
    painter.rect_stroke(view, 0.0, Stroke::new(1.0, palette.caret), StrokeKind::Inside);
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Viewport = Viewport {
        offset: 0.0,
        height: 100.0,
        row_height: 10.0,
        content_height: 1000.0,
    };

    #[test]
    fn regions_take_the_color_of_the_document_they_differ_in() {
        assert_eq!(kind_of(&RegionStatus::Diff(HunkChange::Modified)), Kind::Source(Document::B));
        assert_eq!(kind_of(&RegionStatus::Merge(MergeStatus::ChangedInB)), Kind::Source(Document::B));
        assert_eq!(kind_of(&RegionStatus::Merge(MergeStatus::ChangedInC)), Kind::Source(Document::C));
    }

    #[test]
    fn conflicts_are_red_and_alike_changes_and_hand_edits_are_neutral() {
        assert_eq!(kind_of(&RegionStatus::Merge(MergeStatus::Conflict)), Kind::Conflict);
        assert_eq!(kind_of(&RegionStatus::Merge(MergeStatus::WhitespaceConflict)), Kind::Conflict);
        assert_eq!(kind_of(&RegionStatus::Merge(MergeStatus::ChangedAlike)), Kind::Edited);
        assert_eq!(kind_of(&RegionStatus::Merge(MergeStatus::Unchanged)), Kind::Edited);
        assert_eq!(kind_of(&RegionStatus::Diff(HunkChange::Unchanged)), Kind::Edited);
    }

    #[test]
    fn blocks_are_placed_in_proportion_to_their_rows() {
        let (top, bottom) = span_of(&(10..20), &VIEWPORT, 200.0);
        assert_eq!((top, bottom), (20.0, 40.0));
    }

    #[test]
    fn short_blocks_stay_visible() {
        let (top, bottom) = span_of(&(50..51), &VIEWPORT, 100.0);
        assert_eq!((top, bottom), (50.0, 50.0 + MIN_BLOCK_HEIGHT));
    }

    #[test]
    fn pressing_the_map_centers_the_view_there() {
        assert_eq!(offset_at(100.0, 200.0, &VIEWPORT), 450.0);
    }

    #[test]
    fn the_view_stays_inside_the_document() {
        assert_eq!(offset_at(0.0, 200.0, &VIEWPORT), 0.0);
        assert_eq!(offset_at(200.0, 200.0, &VIEWPORT), 900.0);
        assert_eq!(offset_at(500.0, 200.0, &VIEWPORT), 900.0);
        assert_eq!(offset_at(-50.0, 200.0, &VIEWPORT), 0.0);
    }

    #[test]
    fn a_map_without_height_keeps_the_offset() {
        let viewport = Viewport { offset: 123.0, ..VIEWPORT };
        assert_eq!(offset_at(10.0, 0.0, &viewport), 123.0);
        assert_eq!(offset_at(f32::NAN, 100.0, &viewport), 123.0);
    }
}
