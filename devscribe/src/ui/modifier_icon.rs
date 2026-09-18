//! Small canvas-drawn glyphs for the macOS modifier keys (⌘ ⇧ ⌥ ⌃). The
//! bundled Maho/JetBrains Mono font (see `fonts.rs`) has no glyphs at these
//! codepoints — rendering them as plain `text!` produces empty tofu boxes,
//! which is exactly what the old hardcoded-Unicode shortcut strings did on
//! anything but a system that happens to have a fallback symbol font.
//! Drawn as vector paths instead, like `search_icon.rs`/`logo_mark.rs`, so
//! they recolor live with the theme rather than needing a second raster
//! asset per palette. Windows/Linux never reach this: `shortcut_key.rs`
//! spells those modifiers out as plain "Ctrl"/"Shift"/"Alt" text, which the
//! bundled font already covers.
use iced::widget::canvas::{self, Frame, Geometry, LineCap, LineJoin, Path, Stroke, Style};
use iced::{mouse, Color, Point, Rectangle, Renderer, Theme};

use crate::state::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierGlyph {
    /// ⌘ — the primary modifier (`Command`).
    Command,
    /// ⇧ — Shift.
    Shift,
    /// ⌥ — Option/Alt.
    Option,
    /// ⌃ — the literal physical Control key, for the one binding that stays
    /// Ctrl even on macOS (`Ctrl+Tab` — see `state::global_keys`'s own doc
    /// comment on why that one is never translated to Cmd).
    Control,
}

pub struct ModifierIcon {
    pub glyph: ModifierGlyph,
    pub color: Color,
}

impl canvas::Program<Message> for ModifierIcon {
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
        // Every glyph is authored on an 18x18 grid, scaled to whatever size
        // the caller actually draws it at.
        let s = (bounds.width / 18.0).min(bounds.height / 18.0);
        let pt = |x: f32, y: f32| Point::new(x * s, y * s);
        let stroke = Stroke {
            style: Style::Solid(self.color),
            width: 1.6 * s,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };

        match self.glyph {
            // Four loops joined by a central cross-bar — the real
            // construction of the glyph, simplified to plain stroked circles.
            ModifierGlyph::Command => {
                for (cx, cy) in [(5.5, 5.5), (12.5, 5.5), (5.5, 12.5), (12.5, 12.5)] {
                    frame.stroke(&Path::circle(pt(cx, cy), 2.7 * s), stroke);
                }
                let cross = Path::new(|p| {
                    p.move_to(pt(7.2, 9.0));
                    p.line_to(pt(10.8, 9.0));
                    p.move_to(pt(9.0, 7.2));
                    p.line_to(pt(9.0, 10.8));
                });
                frame.stroke(&cross, stroke);
            }
            // A filled up-arrow — the usual "Shift" pictogram.
            ModifierGlyph::Shift => {
                let arrow = Path::new(|p| {
                    p.move_to(pt(9.0, 2.3));
                    p.line_to(pt(15.2, 9.5));
                    p.line_to(pt(11.2, 9.5));
                    p.line_to(pt(11.2, 15.7));
                    p.line_to(pt(6.8, 15.7));
                    p.line_to(pt(6.8, 9.5));
                    p.line_to(pt(2.8, 9.5));
                    p.close();
                });
                frame.fill(&arrow, self.color);
            }
            // The "railroad switch" shape — a horizontal run, a diagonal
            // ramp, then another horizontal run at the top.
            ModifierGlyph::Option => {
                let path = Path::new(|p| {
                    p.move_to(pt(2.8, 15.0));
                    p.line_to(pt(7.5, 15.0));
                    p.line_to(pt(13.5, 3.0));
                    p.line_to(pt(15.2, 3.0));
                });
                frame.stroke(&path, stroke);
            }
            // A plain upward chevron.
            ModifierGlyph::Control => {
                let chevron = Path::new(|p| {
                    p.move_to(pt(4.0, 12.0));
                    p.line_to(pt(9.0, 6.3));
                    p.line_to(pt(14.0, 12.0));
                });
                frame.stroke(&chevron, stroke);
            }
        }

        vec![frame.into_geometry()]
    }
}
