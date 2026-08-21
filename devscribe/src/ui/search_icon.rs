//! A small magnifying-glass icon (circle + handle), matching the mockup's
//! search glyph. Drawn on a canvas — like `logo_mark.rs` — rather than
//! loaded as an SVG asset, so it recolors live with the theme.
use iced::widget::canvas::{self, Frame, Geometry, LineCap, Path, Stroke, Style};
use iced::{mouse, Color, Point, Rectangle, Renderer, Theme};

use crate::state::Message;

pub struct SearchIcon {
    pub color: Color,
}

impl canvas::Program<Message> for SearchIcon {
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

        // Source is a 24x24 viewBox: circle at (10.5,10.5) r=6, handle from
        // (15,15) to (20,20).
        let sx = bounds.width / 24.0;
        let sy = bounds.height / 24.0;
        let pt = |x: f32, y: f32| Point::new(x * sx, y * sy);
        let stroke_width = 1.6 * sx.min(sy);
        let stroke = Stroke {
            style: Style::Solid(self.color),
            width: stroke_width,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };

        let lens = Path::new(|p| {
            p.circle(pt(10.5, 10.5), 6.0 * sx.min(sy));
        });
        frame.stroke(&lens, stroke);

        let handle = Path::new(|p| {
            p.move_to(pt(15.0, 15.0));
            p.line_to(pt(20.0, 20.0));
        });
        frame.stroke(&handle, stroke);

        vec![frame.into_geometry()]
    }
}
