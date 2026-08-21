//! The DevScribe "fold mark": two outward-facing chevrons (`devscribe-mark-2a-fold.svg`),
//! one per theme accent token (`--accent`, `--accent-2`) so the whole mark visibly
//! shifts with every theme, not just half of it. Drawn on a canvas rather than
//! loaded as an SVG asset so it recolors live, the same way every other piece of
//! chrome in the shell does.
use iced::widget::canvas::{self, Frame, Geometry, LineCap, Path, Stroke, Style};
use iced::{mouse, Color, Point, Rectangle, Renderer, Theme};

use crate::state::Message;

pub struct LogoMark {
    pub left: Color,
    pub right: Color,
}

impl canvas::Program<Message> for LogoMark {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        // Source SVG is a 100x100 viewBox; scale its coordinates to the frame.
        let sx = bounds.width / 100.0;
        let sy = bounds.height / 100.0;
        let pt = |x: f32, y: f32| Point::new(x * sx, y * sy);
        let stroke_width = 7.0 * sx.min(sy);

        let left = Path::new(|p| {
            p.move_to(pt(40.0, 18.0));
            p.line_to(pt(14.0, 50.0));
            p.line_to(pt(40.0, 82.0));
        });
        frame.stroke(
            &left,
            Stroke {
                style: Style::Solid(self.left),
                width: stroke_width,
                line_cap: LineCap::Square,
                ..Stroke::default()
            },
        );

        let right = Path::new(|p| {
            p.move_to(pt(60.0, 18.0));
            p.line_to(pt(86.0, 50.0));
            p.line_to(pt(60.0, 82.0));
        });
        frame.stroke(
            &right,
            Stroke {
                style: Style::Solid(self.right),
                width: stroke_width,
                line_cap: LineCap::Square,
                ..Stroke::default()
            },
        );

        vec![frame.into_geometry()]
    }
}
