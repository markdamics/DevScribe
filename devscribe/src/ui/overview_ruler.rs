//! A thin fixed-height strip along the editor pane's right edge, sitting
//! next to the scrollable rather than inside it, mapping the *whole* file
//! to one glance (roadmap item 11) — the default `scrollable` scrollbar
//! only communicates position via thumb size/offset, and shows nothing
//! about *where* errors, warnings, search matches, or bookmarks sit
//! elsewhere in the file. A translucent band tracks the currently visible
//! region; clicking or dragging anywhere on the strip jumps straight there.
//!
//! Every row index this module is handed (`cursor_row`, `diagnostic_rows`,
//! ...) has already been resolved through `shell.rs`'s own word-wrap
//! mapping (`wrap_offsets`) before it gets here, against `total_rows` —
//! this module only ever deals in "row N of `total_rows`", never in
//! buffer-line numbers directly, so it doesn't need to know word wrap
//! exists.
use devscribe_core::lsp::DiagnosticSeverity;
use devscribe_core::theme::{Palette, Rgba};
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::color::color;
use crate::state::{Message, Pane};

pub const WIDTH: f32 = 14.0;

/// Smallest the visible-region band ever draws at, even for a file so long
/// the true proportional height would be a sliver — it's also the click
/// target for "scroll to roughly here", so it can't shrink to unusable.
const MIN_BAND_HEIGHT: f32 = 18.0;

pub struct OverviewRuler {
    pub palette: Palette,
    pub pane: Pane,
    pub total_rows: usize,
    /// Total content height (px) of the real scrollable canvas —
    /// `scroll_offset`/`viewport_height` are already measured against this
    /// same value (see `shell.rs`'s `code_area`), so the visible-region
    /// band lines up with the actual scrollbar thumb's own fractions.
    pub content_height: f32,
    pub viewport_height: f32,
    pub scroll_offset: f32,
    pub cursor_row: usize,
    pub diagnostic_rows: Vec<(usize, DiagnosticSeverity)>,
    pub find_rows: Vec<usize>,
    pub bookmark_rows: Vec<usize>,
}

impl OverviewRuler {
    fn row_fraction(&self, row: usize) -> f32 {
        if self.total_rows <= 1 {
            0.0
        } else {
            (row as f32 / self.total_rows as f32).clamp(0.0, 1.0)
        }
    }

    /// A click/drag at `fraction` (0.0 top .. 1.0 bottom of the strip) turns
    /// into the scroll offset that centers that point in the viewport —
    /// the inverse of the visible-region band's own position, scaled to
    /// `content_height` px rather than `total_rows`.
    fn offset_for_fraction(&self, fraction: f32) -> f32 {
        let max_offset = (self.content_height - self.viewport_height).max(0.0);
        (fraction * self.content_height - self.viewport_height / 2.0).clamp(0.0, max_offset)
    }
}

impl canvas::Program<Message> for OverviewRuler {
    /// Whether a left-button drag is in progress — `CursorMoved` only keeps
    /// scrolling while this is set, same shape `editor_canvas::CanvasState`
    /// uses for drag-selecting text.
    type State = bool;

    fn update(
        &self,
        dragging: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds)?;
                *dragging = true;
                let fraction = position.y / bounds.height.max(1.0);
                Some(
                    canvas::Action::publish(Message::EditorScrollTo {
                        pane: self.pane,
                        offset: self.offset_for_fraction(fraction),
                    })
                    .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if *dragging => {
                let position = cursor.position_in(bounds)?;
                let fraction = (position.y / bounds.height.max(1.0)).clamp(0.0, 1.0);
                Some(
                    canvas::Action::publish(Message::EditorScrollTo {
                        pane: self.pane,
                        offset: self.offset_for_fraction(fraction),
                    })
                    .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                *dragging = false;
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let p = self.palette;
        let mut frame = Frame::new(renderer, bounds.size());

        frame.fill(&Path::rectangle(Point::ORIGIN, bounds.size()), color(p.surface_sunken));

        // The visible-region band: the part that actually stands in for the
        // scrollbar thumb. Everything drawn after it is markers layered on
        // top, positioned by `total_rows` rather than these same px values,
        // since a marker's *row* is what matters, not scroll pixels.
        if self.content_height > 0.0 {
            let top = (self.scroll_offset / self.content_height).clamp(0.0, 1.0) * bounds.height;
            let height = (self.viewport_height / self.content_height * bounds.height).max(MIN_BAND_HEIGHT).min(bounds.height);
            frame.fill(
                &Path::rectangle(Point::new(0.0, top), Size::new(bounds.width, height)),
                tint(p.surface_hover, 0.9),
            );
        }

        for &(row, severity) in &self.diagnostic_rows {
            let y = self.row_fraction(row) * bounds.height;
            let c = if severity == DiagnosticSeverity::ERROR { p.status_danger } else { p.status_warning };
            frame.fill(&Path::rectangle(Point::new(bounds.width - 5.0, y), Size::new(5.0, 2.0)), color(c));
        }
        for &row in &self.find_rows {
            let y = self.row_fraction(row) * bounds.height;
            frame.fill(&Path::rectangle(Point::new(2.0, y), Size::new(5.0, 2.0)), color(p.status_info));
        }
        for &row in &self.bookmark_rows {
            let y = self.row_fraction(row) * bounds.height;
            frame.fill(&Path::rectangle(Point::new(2.0, y), Size::new(5.0, 2.0)), color(p.accent_solid));
        }

        // The cursor's own row, drawn last (on top) and spanning the full
        // width, so it's always findable even where it lands on top of a
        // diagnostic/bookmark tick.
        let cursor_y = (self.row_fraction(self.cursor_row) * bounds.height - 1.0).max(0.0);
        frame.fill(&Path::rectangle(Point::new(0.0, cursor_y), Size::new(bounds.width, 2.0)), color(p.text_strong));

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _state: &Self::State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if cursor.position_in(bounds).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

fn tint(c: Rgba, alpha: f32) -> Color {
    color(Rgba { a: alpha, ..c })
}

#[cfg(test)]
mod tests {
    use super::*;
    use devscribe_core::theme::{palette, Accent, ThemeMode};

    fn ruler(total_rows: usize, content_height: f32, viewport_height: f32, scroll_offset: f32) -> OverviewRuler {
        OverviewRuler {
            palette: palette(ThemeMode::default(), Accent::default()),
            pane: Pane::Primary,
            total_rows,
            content_height,
            viewport_height,
            scroll_offset,
            cursor_row: 0,
            diagnostic_rows: Vec::new(),
            find_rows: Vec::new(),
            bookmark_rows: Vec::new(),
        }
    }

    #[test]
    fn row_fraction_is_the_row_over_total_rows() {
        let r = ruler(200, 1000.0, 100.0, 0.0);
        assert_eq!(r.row_fraction(0), 0.0);
        assert_eq!(r.row_fraction(50), 0.25);
        assert_eq!(r.row_fraction(199), 199.0 / 200.0);
    }

    #[test]
    fn row_fraction_never_exceeds_one_past_the_last_row() {
        let r = ruler(10, 1000.0, 100.0, 0.0);
        assert_eq!(r.row_fraction(9999), 1.0);
    }

    #[test]
    fn row_fraction_is_zero_for_a_file_with_at_most_one_row() {
        // Guards the `total_rows <= 1` branch against a divide-by-zero.
        let r = ruler(1, 100.0, 100.0, 0.0);
        assert_eq!(r.row_fraction(0), 0.0);
    }

    #[test]
    fn offset_for_fraction_centers_the_clicked_point_in_the_viewport() {
        let r = ruler(200, 1000.0, 100.0, 0.0);
        // Clicking the ruler's midpoint (fraction 0.5) should land on
        // content-space y = 500, centered in a 100px-tall viewport — i.e.
        // scrolled so that point sits 50px from the top of the pane.
        assert_eq!(r.offset_for_fraction(0.5), 450.0);
    }

    #[test]
    fn offset_for_fraction_clamps_to_the_valid_scroll_range() {
        let r = ruler(200, 1000.0, 100.0, 0.0);
        assert_eq!(r.offset_for_fraction(0.0), 0.0, "must not go negative for a click near the very top");
        assert_eq!(r.offset_for_fraction(1.0), 900.0, "must not scroll past the last full viewport of content");
    }

    #[test]
    fn offset_for_fraction_is_always_zero_when_the_whole_file_already_fits_on_screen() {
        let r = ruler(5, 100.0, 400.0, 0.0);
        assert_eq!(r.offset_for_fraction(1.0), 0.0);
    }
}
