//! A from-scratch code editor pane: `ropey`-backed text on a monospace grid,
//! drawn directly with `iced::widget::canvas` (gutter, selection, caret,
//! tree-sitter-driven per-span coloring) and driven by real mouse/keyboard
//! input — no LSP diagnostics yet (milestone 5).
use devscribe_core::syntax::{HighlightKind, Span};
use devscribe_core::theme::{Palette, Rgba};
use devscribe_core::Document;
use iced::alignment::Vertical;
use iced::widget::canvas::{self, Frame, Geometry, Path, Text};
use iced::widget::text::{Alignment, LineHeight};
use iced::{keyboard, mouse, Color, Pixels, Point, Rectangle, Renderer, Size, Theme};

use crate::color::color;
use crate::fonts;
use crate::state::{CursorPos, Direction, Message};

const LINE_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 13.0;
const CHAR_WIDTH: f32 = FONT_SIZE * 0.6;
const GUTTER_WIDTH: f32 = 52.0;
const TEXT_INSET: f32 = 4.0;
const TOP_PAD: f32 = 12.0;

pub struct EditorCanvas {
    pub document: Document,
    pub cursor: CursorPos,
    pub selection: Option<(usize, usize)>,
    pub palette: Palette,
    pub caret_visible: bool,
    /// Document-ordered, non-overlapping byte-range spans from
    /// `devscribe_core::syntax`. Empty for files with no wired grammar,
    /// in which case lines fall back to a single flat color.
    pub highlights: Vec<Span>,
}

/// Purely local interaction state — never synced back into `State` directly.
/// Real edits flow out as `Message`s; this only tracks "does this canvas
/// currently have focus" and "what modifiers are held", the same pattern
/// `iced::widget::text_input` uses internally.
#[derive(Default)]
pub struct CanvasState {
    focused: bool,
    modifiers: keyboard::Modifiers,
}

impl EditorCanvas {
    fn hit_test(&self, position: Point) -> (usize, usize) {
        let line = ((position.y - TOP_PAD) / LINE_HEIGHT).floor();
        let line = (line.max(0.0) as usize).min(self.document.line_count().saturating_sub(1));

        let x = position.x - GUTTER_WIDTH - TEXT_INSET;
        let col = if x <= 0.0 {
            0
        } else {
            (x / CHAR_WIDTH).round() as usize
        };
        let col = col.min(self.document.line_len_chars(line));
        (line, col)
    }

    fn handle_key(
        &self,
        key: &keyboard::Key,
        modifiers: keyboard::Modifiers,
        text: Option<&str>,
    ) -> Option<canvas::Action<Message>> {
        use keyboard::key::{Key, Named};

        let publish = |message: Message| Some(canvas::Action::publish(message).and_capture());

        if let Key::Named(named) = key {
            let extend = modifiers.shift();
            return match named {
                Named::Enter => publish(Message::EditorInsertText("\n".into())),
                Named::Backspace => publish(Message::EditorBackspace),
                Named::Delete => publish(Message::EditorDelete),
                Named::Tab => publish(Message::EditorInsertText("    ".into())),
                Named::ArrowLeft => publish(Message::EditorMove {
                    dir: Direction::Left,
                    extend,
                }),
                Named::ArrowRight => publish(Message::EditorMove {
                    dir: Direction::Right,
                    extend,
                }),
                Named::ArrowUp => publish(Message::EditorMove {
                    dir: Direction::Up,
                    extend,
                }),
                Named::ArrowDown => publish(Message::EditorMove {
                    dir: Direction::Down,
                    extend,
                }),
                Named::Home => publish(Message::EditorMove {
                    dir: Direction::LineStart,
                    extend,
                }),
                Named::End => publish(Message::EditorMove {
                    dir: Direction::LineEnd,
                    extend,
                }),
                _ => None,
            };
        }

        // Command/control/alt-modified keys are shortcuts, not text — never
        // insert their `text` payload (e.g. Ctrl+S shouldn't type an "s").
        if modifiers.command() || modifiers.control() || modifiers.alt() {
            return None;
        }

        let text = text?;
        if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
            return publish(Message::EditorInsertText(text.to_string()));
        }

        None
    }
}

impl canvas::Program<Message> for EditorCanvas {
    type State = CanvasState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds)?;
                state.focused = true;
                let (line, col) = self.hit_test(position);
                let extend = state.modifiers.shift();
                Some(
                    canvas::Action::publish(Message::EditorClick { line, col, extend })
                        .and_capture(),
                )
            }
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                None
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            }) => {
                state.modifiers = *modifiers;
                if !state.focused {
                    return None;
                }
                self.handle_key(key, *modifiers, text.as_deref())
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
        let mut frame = Frame::new(renderer, bounds.size());
        let p = self.palette;

        frame.fill(
            &Path::rectangle(Point::ORIGIN, Size::new(GUTTER_WIDTH, bounds.height)),
            color(p.bg_panel),
        );

        let mono = fonts::mono(iced::font::Weight::Normal);

        // Spans are sorted and document-ordered; walk them alongside lines
        // rather than rescanning from the start each time.
        let mut span_cursor = 0usize;

        for line in 0..self.document.line_count() {
            let y = TOP_PAD + line as f32 * LINE_HEIGHT;

            if let Some((start, end)) = self.selection {
                let line_start_idx = self.document.char_index(line, 0);
                let line_len = self.document.line_len_chars(line);
                let line_end_idx = line_start_idx + line_len;
                // Treat the line terminator as one extra selectable column so a
                // selection spanning multiple lines visibly covers each line's end.
                let selectable_end = line_end_idx + 1;

                if start < selectable_end && end > line_start_idx {
                    let sel_start_col = start.saturating_sub(line_start_idx);
                    let sel_end_col = (end.saturating_sub(line_start_idx)).min(line_len + 1);
                    let x0 = GUTTER_WIDTH + TEXT_INSET + sel_start_col as f32 * CHAR_WIDTH;
                    let x1 = GUTTER_WIDTH + TEXT_INSET + sel_end_col as f32 * CHAR_WIDTH;
                    frame.fill(
                        &Path::rectangle(
                            Point::new(x0, y),
                            Size::new((x1 - x0).max(CHAR_WIDTH * 0.4), LINE_HEIGHT),
                        ),
                        tint(p.accent, 0.22),
                    );
                }
            }

            let is_cursor_line = line == self.cursor.line;

            frame.fill_text(Text {
                content: (line + 1).to_string(),
                position: Point::new(GUTTER_WIDTH - 14.0, y),
                color: if is_cursor_line {
                    color(p.text_primary)
                } else {
                    tint(p.text_muted, 0.6)
                },
                size: Pixels(11.0),
                line_height: LineHeight::Absolute(Pixels(LINE_HEIGHT)),
                font: mono,
                align_x: Alignment::Right,
                align_y: Vertical::Top,
                ..Text::default()
            });

            let text = self.document.line_text(line);
            if !text.is_empty() {
                let line_start_byte = self.document.text().line_to_byte(line);
                let line_end_byte = line_start_byte + text.len();

                while span_cursor < self.highlights.len()
                    && self.highlights[span_cursor].end <= line_start_byte
                {
                    span_cursor += 1;
                }

                if self.highlights.is_empty() {
                    frame.fill_text(Text {
                        content: text,
                        position: Point::new(GUTTER_WIDTH + TEXT_INSET, y),
                        color: color(p.text_secondary),
                        size: Pixels(FONT_SIZE),
                        line_height: LineHeight::Absolute(Pixels(LINE_HEIGHT)),
                        font: mono,
                        align_y: Vertical::Top,
                        ..Text::default()
                    });
                } else {
                    let mut idx = span_cursor;
                    while idx < self.highlights.len() && self.highlights[idx].start < line_end_byte {
                        let span = self.highlights[idx];
                        let seg_start = span.start.max(line_start_byte) - line_start_byte;
                        let seg_end = span.end.min(line_end_byte) - line_start_byte;
                        if seg_start < seg_end {
                            let start_col = text[..seg_start].chars().count();
                            let content = text[seg_start..seg_end].to_string();
                            frame.fill_text(Text {
                                content,
                                position: Point::new(
                                    GUTTER_WIDTH + TEXT_INSET + start_col as f32 * CHAR_WIDTH,
                                    y,
                                ),
                                color: highlight_color(span.kind, p),
                                size: Pixels(FONT_SIZE),
                                line_height: LineHeight::Absolute(Pixels(LINE_HEIGHT)),
                                font: mono,
                                align_y: Vertical::Top,
                                ..Text::default()
                            });
                        }
                        idx += 1;
                    }
                }
            }

            if is_cursor_line && self.caret_visible {
                let x = GUTTER_WIDTH + TEXT_INSET + self.cursor.col as f32 * CHAR_WIDTH;
                frame.fill(
                    &Path::rectangle(Point::new(x, y + 1.0), Size::new(2.0, LINE_HEIGHT - 4.0)),
                    color(p.accent),
                );
            }
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }
}

fn tint(c: Rgba, alpha: f32) -> Color {
    color(Rgba { a: alpha, ..c })
}

/// Maps a syntax category onto the current theme's tokens. Chosen to match
/// the original mockup's static Rust code sample exactly (keyword→accent,
/// type→status-info, string→status-ok, constant→status-warn, comment→muted).
fn highlight_color(kind: HighlightKind, p: Palette) -> Color {
    match kind {
        HighlightKind::Default => color(p.text_secondary),
        HighlightKind::Keyword => color(p.accent),
        HighlightKind::Type => color(p.status_info),
        HighlightKind::Function => color(p.text_primary),
        HighlightKind::Macro => color(p.accent_2),
        HighlightKind::String => color(p.status_ok),
        HighlightKind::Number => color(p.accent_2),
        HighlightKind::Comment => color(p.text_muted),
        HighlightKind::Constant => color(p.status_warn),
        HighlightKind::Attribute => color(p.text_muted),
        HighlightKind::Punctuation => color(p.text_secondary),
    }
}

/// Total content height for `line_count` lines — used to size the `Canvas`
/// so it can sit inside a `scrollable` for vertical scrolling.
pub fn content_height(line_count: usize) -> f32 {
    TOP_PAD * 2.0 + line_count as f32 * LINE_HEIGHT
}
