//! The "run anything" glyph — a lightning bolt, standing in for the command
//! palette's launch-anything action. Drawn on a canvas, like
//! `search_icon.rs`/`modifier_icon.rs`, so it recolors live with the theme.
//! Deliberately not a magnifying glass: the tab bar already pins a search
//! icon of its own (`search_icon_tab`), and reusing that glyph here would
//! make two differently-behaving controls look like the same action.
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::{mouse, Color, Point, Rectangle, Renderer, Theme};

use crate::state::Message;

pub struct RunIcon {
    pub color: Color,
}

impl canvas::Program<Message> for RunIcon {
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

        // Source is an 18x18 grid — same authoring convention
        // `modifier_icon.rs` uses — a plain bolt: down the right of the
        // upper half, then down the left of the lower half.
        let s = (bounds.width / 18.0).min(bounds.height / 18.0);
        let pt = |x: f32, y: f32| Point::new(x * s, y * s);

        let bolt = Path::new(|p| {
            p.move_to(pt(10.2, 1.5));
            p.line_to(pt(4.0, 10.5));
            p.line_to(pt(8.4, 10.5));
            p.line_to(pt(7.2, 16.5));
            p.line_to(pt(14.0, 7.2));
            p.line_to(pt(9.6, 7.2));
            p.close();
        });
        frame.fill(&bolt, self.color);

        vec![frame.into_geometry()]
    }
}
