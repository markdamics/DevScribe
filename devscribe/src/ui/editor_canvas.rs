//! A from-scratch code editor pane: `ropey`-backed text on a monospace grid,
//! drawn directly with `iced::widget::canvas` (gutter, selection, caret,
//! tree-sitter-driven per-span coloring, LSP diagnostics) and driven by real
//! mouse/keyboard input.
use devscribe_core::diff::GutterMark;
use devscribe_core::lsp::DiagnosticSeverity;
use devscribe_core::syntax::{HighlightKind, Span};
use devscribe_core::theme::{Palette, Rgba};
use devscribe_core::Document;
use iced::alignment::Vertical;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Style, Text};
use iced::widget::text::{Alignment, LineHeight};
use iced::{keyboard, mouse, Color, Pixels, Point, Rectangle, Renderer, Size, Theme};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::color::color;
use crate::fonts;
use crate::state::{CursorPos, Direction, EditorDiagnostic, Message};

/// The font-size-to-`line_height`/`char_width` ratios the original static
/// design used (22/13 and 0.6 respectively) — kept fixed so the editor's
/// proportions stay the same as `font_size` changes via the settings panel.
const LINE_HEIGHT_RATIO: f32 = 22.0 / 13.0;
const CHAR_WIDTH_RATIO: f32 = 0.6;
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
    pub highlights: Rc<Vec<Span>>,
    pub diagnostics: Rc<Vec<EditorDiagnostic>>,
    /// Per-buffer-line added/modified/removed marker, from `EditorState`'s
    /// diff against `HEAD` — drawn as a small colored gutter bar, and what a
    /// gutter click's `Message::RevertLine` acts on. Empty for a file with
    /// no diff (no repo, untracked, or unchanged).
    pub gutter_marks: Rc<Vec<Option<GutterMark>>>,
    /// Toggled from the settings panel. Only hides the inline `// message`
    /// annotation — the wavy underline stays either way.
    pub problem_lens_enabled: bool,
    /// Set from the settings panel's font-size stepper.
    pub font_size: f32,
    /// In-file find (Ctrl+F) match ranges, as absolute char ranges — same
    /// coordinate space as `selection`. Empty when find is closed.
    pub find_matches: Vec<(usize, usize)>,
    /// Index into `find_matches` of the current (more prominently
    /// highlighted) match. Meaningless when `find_matches` is empty.
    pub find_current: usize,
    pub scroll_offset: f32,
    pub viewport_height: f32,
}

/// Purely local interaction state — never synced back into `State` directly.
/// Real edits flow out as `Message`s; this only tracks "does this canvas
/// currently have focus" and "what modifiers are held", the same pattern
/// `iced::widget::text_input` uses internally.
#[derive(Default)]
pub struct CanvasState {
    focused: bool,
    modifiers: keyboard::Modifiers,
    /// `true` from a left-button press until its matching release — while
    /// set, further `CursorMoved` events extend the selection (drag-select).
    dragging: bool,
    /// `(when, line, col)` of the last left-button press, for double/triple
    /// click detection — a same-cell press within `CLICK_STREAK_WINDOW`
    /// advances `click_streak`; anything else resets it to a fresh single.
    last_click: Option<(Instant, usize, usize)>,
    /// 1 = plain click, 2 = double (select word), 3 = triple (select line),
    /// wrapping back to 1 on a fourth same-cell click.
    click_streak: u8,
}

const CLICK_STREAK_WINDOW: Duration = Duration::from_millis(450);

impl EditorCanvas {
    fn line_height(&self) -> f32 {
        self.font_size * LINE_HEIGHT_RATIO
    }

    fn char_width(&self) -> f32 {
        self.font_size * CHAR_WIDTH_RATIO
    }

    fn hit_test(&self, position: Point) -> (usize, usize) {
        let line_height = self.line_height();
        let line = ((position.y - TOP_PAD) / line_height).floor();
        let line = (line.max(0.0) as usize).min(self.document.line_count().saturating_sub(1));

        let x = position.x - GUTTER_WIDTH - TEXT_INSET;
        let col = if x <= 0.0 {
            0
        } else {
            (x / self.char_width()).round() as usize
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

        if modifiers.command()
            && let Key::Character(c) = key.as_ref()
        {
            return if c.eq_ignore_ascii_case("a") {
                publish(Message::EditorSelectAll)
            } else if c.eq_ignore_ascii_case("c") {
                publish(Message::EditorCopy)
            } else if c.eq_ignore_ascii_case("x") {
                publish(Message::EditorCut)
            } else if c.eq_ignore_ascii_case("v") {
                publish(Message::EditorPaste)
            } else if c.eq_ignore_ascii_case("z") {
                if modifiers.shift() {
                    publish(Message::EditorRedo)
                } else {
                    publish(Message::EditorUndo)
                }
            } else if c.eq_ignore_ascii_case("y") {
                publish(Message::EditorRedo)
            } else {
                None
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

                // A click on a changed line's gutter marker reverts that
                // line instead of placing the cursor or starting a drag-
                // select — the marker itself is the affordance (unlike a
                // Changes-panel row, a changed line's bar is always visible,
                // so there's no separate hover-reveal step needed).
                // Unmarked-line gutter clicks fall through unchanged, to
                // today's cursor-to-column-0.
                if position.x < GUTTER_WIDTH && self.gutter_marks.get(line).and_then(Option::as_ref).is_some() {
                    return Some(canvas::Action::publish(Message::RevertLine { line }).and_capture());
                }
                state.dragging = true;

                // Shift-click always just extends, same as shift+arrow —
                // doesn't participate in double/triple-click detection.
                if state.modifiers.shift() {
                    state.last_click = None;
                    state.click_streak = 0;
                    return Some(
                        canvas::Action::publish(Message::EditorClick { line, col, extend: true })
                            .and_capture(),
                    );
                }

                let now = Instant::now();
                let repeats_last = state.last_click.is_some_and(|(at, l, c)| {
                    l == line && c == col && now.duration_since(at) < CLICK_STREAK_WINDOW
                });
                state.click_streak = if repeats_last { state.click_streak % 3 + 1 } else { 1 };
                state.last_click = Some((now, line, col));

                let message = match state.click_streak {
                    2 => Message::EditorSelectWord { line, col },
                    3 => Message::EditorSelectLine { line },
                    _ => Message::EditorClick { line, col, extend: false },
                };
                Some(canvas::Action::publish(message).and_capture())
            }
            // Drag-select: every cursor move while the button is still down
            // extends the selection from wherever it started, the same way
            // shift-click does. Computed from the cursor's absolute
            // position (not `position_in`, which is `None` once the cursor
            // strays outside the canvas's own bounds) so a drag past either
            // edge still keeps extending rather than freezing — `hit_test`
            // already clamps whatever it's given to a valid line/col.
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                let position = cursor.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y))?;
                let (line, col) = self.hit_test(position);
                Some(canvas::Action::publish(Message::EditorClick { line, col, extend: true }).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                None
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
        let font_size = self.font_size;
        let line_height = self.line_height();
        let char_width = self.char_width();

        let mono = fonts::mono(iced::font::Weight::Normal);

        // Only the lines that could plausibly be visible need geometry —
        // `bounds` covers the whole (fixed-height) document, so without
        // this a large file would re-shape/tessellate every line on every
        // scroll tick.
        const OVERSCAN_LINES: usize = 8;
        let total_lines = self.document.line_count();
        let first_onscreen =
            ((self.scroll_offset - TOP_PAD).max(0.0) / line_height).floor() as usize;
        let onscreen_count = (self.viewport_height.max(0.0) / line_height).ceil() as usize + 1;
        let first_line = first_onscreen.saturating_sub(OVERSCAN_LINES);
        let last_line = (first_onscreen + onscreen_count + OVERSCAN_LINES).min(total_lines);

        // Spans are sorted and document-ordered; walk them alongside lines
        // rather than rescanning from the start each time. Jump straight to
        // `first_line` via binary search rather than a linear fast-forward,
        // since that line can be far into the document.
        let mut span_cursor = if first_line == 0 {
            0
        } else {
            let first_line_byte = self.document.text().line_to_byte(first_line);
            self.highlights
                .partition_point(|s| s.end <= first_line_byte)
        };

        // Horizontal culling. There is no horizontal scrolling (the canvas
        // is `Length::Fill` inside a vertical-only `scrollable`), so every
        // column past the right edge is not merely off-screen, it is
        // unreachable — and a project can hold lines with hundreds of
        // thousands of chars (a minified bundle, a generated blob). Without
        // this, one such line materialized its whole self as a `String` and
        // handed it to the text shaper on *every* frame; the per-span
        // column arithmetic below was quadratic in the line's length on top
        // of that. Landing on one via Find navigation was enough to wedge
        // the app.
        let text_x0 = GUTTER_WIDTH + TEXT_INSET;
        // +1 so a glyph straddling the right edge is still drawn, not clipped
        // to nothing.
        let max_cols = (((bounds.width - text_x0).max(0.0) / char_width).ceil() as usize) + 1;
        // Clamps a column-derived x to the visible strip, so a rect or
        // underline spanning a pathological line stays a screen wide rather
        // than a million pixels wide.
        let clamp_x = |x: f32| x.min(bounds.width);

        // Only the find matches and diagnostics that touch the drawn line
        // range matter. Both used to be re-scanned in full for every visible
        // line — O(visible_lines × total), which the 200-match/200-diagnostic
        // caps kept survivable but never cheap. Narrow each once instead.
        let drawn_start_idx = self.document.char_index(first_line, 0);
        let drawn_end_idx = self
            .document
            .char_index(last_line.saturating_sub(1), usize::MAX)
            + 1;
        // `find_matches` is document-ordered (built by scanning line by line),
        // so the relevant window is a contiguous slice. `find_base` keeps the
        // original index, which is what `find_current` is measured against.
        let find_base = self
            .find_matches
            .partition_point(|(_, end)| *end <= drawn_start_idx);
        // `.max(find_base)` only matters if the two partitions ever disagree
        // (they shouldn't — both clamp to the same line); slicing backwards
        // would panic, and this is the render path.
        let find_end = self
            .find_matches
            .partition_point(|(start, _)| *start < drawn_end_idx)
            .max(find_base);
        let find_visible = &self.find_matches[find_base..find_end];
        // Diagnostics arrive in whatever order the server sent them, so
        // filter rather than slice.
        let diags_visible: Vec<&EditorDiagnostic> = self
            .diagnostics
            .iter()
            .filter(|d| d.start.line < last_line && d.end.line >= first_line)
            .collect();

        for line in first_line..last_line {
            let y = TOP_PAD + line as f32 * line_height;
            let is_cursor_line = line == self.cursor.line;

            if is_cursor_line {
                frame.fill(
                    &Path::rectangle(
                        Point::new(GUTTER_WIDTH, y),
                        Size::new(bounds.width - GUTTER_WIDTH, line_height),
                    ),
                    tint(p.accent_solid, 0.08),
                );
            }

            let line_start_idx = self.document.char_index(line, 0);
            let line_len = self.document.line_len_chars(line);
            let line_end_idx = line_start_idx + line_len;
            // Treat the line terminator as one extra selectable column so a
            // selection spanning multiple lines visibly covers each line's end.
            let selectable_end = line_end_idx + 1;

            if let Some((start, end)) = self.selection
                && start < selectable_end
                && end > line_start_idx
            {
                let sel_start_col = start.saturating_sub(line_start_idx);
                let sel_end_col = (end.saturating_sub(line_start_idx)).min(line_len + 1);
                let x0 = text_x0 + sel_start_col as f32 * char_width;
                let x1 = clamp_x(text_x0 + sel_end_col as f32 * char_width);
                frame.fill(
                    &Path::rectangle(
                        Point::new(x0, y),
                        Size::new((x1 - x0).max(char_width * 0.4), line_height),
                    ),
                    tint(p.accent_solid, 0.22),
                );
            }

            for (i, (start, end)) in find_visible.iter().enumerate() {
                if *start >= line_end_idx || *end <= line_start_idx {
                    continue;
                }
                let match_start_col = start.saturating_sub(line_start_idx);
                if text_x0 + match_start_col as f32 * char_width > bounds.width {
                    continue;
                }
                let is_current = find_base + i == self.find_current;
                let match_end_col = (end.saturating_sub(line_start_idx)).min(line_len);
                let x0 = text_x0 + match_start_col as f32 * char_width;
                let x1 = clamp_x(text_x0 + match_end_col as f32 * char_width);
                let rect = Path::rectangle(
                    Point::new(x0, y + 1.0),
                    Size::new((x1 - x0).max(char_width * 0.4), line_height - 2.0),
                );
                if is_current {
                    frame.fill(&rect, tint(p.status_warning, 0.35));
                    frame.stroke(
                        &rect,
                        Stroke {
                            style: Style::Solid(color(p.status_warning)),
                            width: 1.0,
                            ..Stroke::default()
                        },
                    );
                } else {
                    frame.fill(&rect, tint(p.status_warning, 0.16));
                }
            }

            // The gutter's per-line git-diff marker: a tinted background
            // across the whole gutter cell plus a solid accent bar at its
            // left edge for a line that's added/modified (a lone 3px sliver
            // at the frame's own edge, tried first, was too easy to miss —
            // this reads unmistakably as "this line has a pending action"
            // the way the cursor-line highlight already reads as "this is
            // the active line"), or a solid bar at the line's top edge for
            // `RemovedAbove` (deleted `HEAD` lines don't occupy a line of
            // their own in the new buffer, so they can't get a full-height
            // mark). Clicking anywhere in the gutter on a marked line
            // (`update()`, above) reverts it to `HEAD`.
            if let Some(mark) = self.gutter_marks.get(line).and_then(Option::as_ref) {
                let accent = match mark {
                    GutterMark::Added => p.status_success,
                    GutterMark::Modified { .. } => p.status_warning,
                    GutterMark::RemovedAbove { .. } => p.status_danger,
                };
                frame.fill(
                    &Path::rectangle(Point::new(0.0, y), Size::new(GUTTER_WIDTH, line_height)),
                    tint(accent, 0.16),
                );
                match mark {
                    GutterMark::Added | GutterMark::Modified { .. } => {
                        frame.fill(
                            &Path::rectangle(Point::new(0.0, y), Size::new(4.0, line_height)),
                            color(accent),
                        );
                    }
                    GutterMark::RemovedAbove { .. } => {
                        frame.fill(
                            &Path::rectangle(Point::new(0.0, y), Size::new(GUTTER_WIDTH, 3.0)),
                            color(accent),
                        );
                    }
                }
            }

            frame.fill_text(Text {
                content: (line + 1).to_string(),
                position: Point::new(GUTTER_WIDTH - 14.0, y),
                color: if is_cursor_line {
                    color(p.text_strong)
                } else {
                    tint(p.text_muted, 0.6)
                },
                size: Pixels(11.0),
                line_height: LineHeight::Absolute(Pixels(line_height)),
                font: mono,
                align_x: Alignment::Right,
                align_y: Vertical::Top,
                ..Text::default()
            });

            // Only the columns that fit on screen — see `max_cols`.
            let text = self.document.line_text_capped(line, max_cols);
            if !text.is_empty() {
                let line_start_byte = self.document.text().line_to_byte(line);
                // End of the *drawn* slice, not of the line: spans past the
                // right edge are skipped entirely rather than shaped.
                let drawn_end_byte = line_start_byte + text.len();

                while span_cursor < self.highlights.len()
                    && self.highlights[span_cursor].end <= line_start_byte
                {
                    span_cursor += 1;
                }

                if self.highlights.is_empty() {
                    frame.fill_text(Text {
                        content: text,
                        position: Point::new(text_x0, y),
                        color: color(p.text_body),
                        size: Pixels(font_size),
                        line_height: LineHeight::Absolute(Pixels(line_height)),
                        font: mono,
                        align_y: Vertical::Top,
                        ..Text::default()
                    });
                } else {
                    // Spans are document-ordered and non-overlapping, so a
                    // running (byte, char) cursor converts each span's byte
                    // offset to a column in one forward pass over the line.
                    // Counting `text[..seg_start].chars()` afresh per span
                    // instead made this quadratic in the line's length.
                    let mut walked_bytes = 0usize;
                    let mut walked_chars = 0usize;
                    let mut idx = span_cursor;
                    while idx < self.highlights.len() && self.highlights[idx].start < drawn_end_byte
                    {
                        let span = self.highlights[idx];
                        let seg_start = span.start.max(line_start_byte) - line_start_byte;
                        let seg_end = span.end.min(drawn_end_byte) - line_start_byte;
                        if seg_start < seg_end {
                            if seg_start < walked_bytes {
                                // Defensive: an out-of-order span would make
                                // the forward walk wrong, so restart it.
                                walked_bytes = 0;
                                walked_chars = 0;
                            }
                            walked_chars += text[walked_bytes..seg_start].chars().count();
                            walked_bytes = seg_start;
                            let content = text[seg_start..seg_end].to_string();
                            frame.fill_text(Text {
                                content,
                                position: Point::new(
                                    text_x0 + walked_chars as f32 * char_width,
                                    y,
                                ),
                                color: highlight_color(span.kind, p),
                                size: Pixels(font_size),
                                line_height: LineHeight::Absolute(Pixels(line_height)),
                                font: mono,
                                align_y: Vertical::Top,
                                ..Text::default()
                            });
                        }
                        idx += 1;
                    }
                }
            }

            let mut lens: Option<(&str, Rgba)> = None;
            for diag in &diags_visible {
                if diag.start.line > line || diag.end.line < line {
                    continue;
                }
                let seg_start_col = if diag.start.line == line { diag.start.col } else { 0 };
                let seg_end_col = if diag.end.line == line {
                    diag.end.col.max(seg_start_col + 1)
                } else {
                    line_len.max(seg_start_col + 1)
                };
                let sev_color = severity_color(diag.severity, p);
                let x0 = text_x0 + seg_start_col as f32 * char_width;
                // A diagnostic covering a whole pathological line would
                // otherwise squiggle its way across a million pixels, one
                // `PERIOD`-wide segment at a time.
                let x1 = clamp_x(text_x0 + seg_end_col as f32 * char_width);
                if x0 <= bounds.width {
                    draw_wavy_underline(&mut frame, x0, x1, y + line_height - 5.0, color(sev_color));
                }

                if self.problem_lens_enabled && lens.is_none() {
                    lens = Some((diag.message.as_str(), sev_color));
                }
            }
            if let Some((message, sev_color)) = lens {
                let lens_col = line_len + 3;
                let lens_x = text_x0 + lens_col as f32 * char_width;
                // Past the right edge the annotation is unreachable (no
                // horizontal scrolling), so shaping it is pure waste.
                if lens_x <= bounds.width {
                frame.fill_text(Text {
                    content: format!("// {message}"),
                    position: Point::new(lens_x, y),
                    color: tint(sev_color, 0.65),
                    size: Pixels(font_size - 1.0),
                    line_height: LineHeight::Absolute(Pixels(line_height)),
                    font: mono,
                    align_y: Vertical::Top,
                    ..Text::default()
                });
                }
            }

            if is_cursor_line && self.caret_visible {
                let x = text_x0 + self.cursor.col as f32 * char_width;
                if x <= bounds.width {
                    frame.fill(
                        &Path::rectangle(Point::new(x, y + 1.0), Size::new(2.0, line_height - 4.0)),
                        color(p.accent_solid),
                    );
                }
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

fn severity_color(severity: DiagnosticSeverity, p: Palette) -> Rgba {
    if severity == DiagnosticSeverity::ERROR {
        p.status_danger
    } else if severity == DiagnosticSeverity::WARNING {
        p.status_warning
    } else if severity == DiagnosticSeverity::INFORMATION {
        p.status_info
    } else {
        p.text_muted
    }
}

/// A small squiggle under `[x0, x1)` at baseline `y`, in the rustc/VS Code
/// "wavy underline" style the mockup uses for inline diagnostics.
fn draw_wavy_underline(frame: &mut Frame, x0: f32, x1: f32, y: f32, color: Color) {
    const AMPLITUDE: f32 = 1.6;
    const PERIOD: f32 = 4.0;

    // Belt-and-braces against a caller handing in a span thousands of
    // screens wide: the loop below advances `PERIOD` px at a time, so an
    // unbounded `x1` is an unbounded number of path segments.
    const MAX_SEGMENTS: usize = 4096;
    let x1 = x1.min(x0 + MAX_SEGMENTS as f32 * PERIOD);

    let path = Path::new(|p| {
        p.move_to(Point::new(x0, y));
        let mut x = x0;
        let mut up = true;
        while x < x1 {
            let next_x = (x + PERIOD).min(x1);
            let next_y = if up { y - AMPLITUDE } else { y + AMPLITUDE };
            p.line_to(Point::new(next_x, next_y));
            x = next_x;
            up = !up;
        }
    });
    frame.stroke(
        &path,
        Stroke {
            style: Style::Solid(color),
            width: 1.2,
            ..Stroke::default()
        },
    );
}

/// Maps a syntax category onto the current theme's tokens. Chosen to match
/// the original mockup's static Rust code sample exactly (keyword→accent,
/// type→status-info, string→status-ok, constant→status-warn, comment→muted).
pub(crate) fn highlight_color(kind: HighlightKind, p: Palette) -> Color {
    match kind {
        HighlightKind::Default => color(p.text_body),
        HighlightKind::Keyword => color(p.accent_solid),
        HighlightKind::Type => color(p.status_info),
        HighlightKind::Function => color(p.text_strong),
        HighlightKind::Macro => color(p.seal_solid),
        HighlightKind::String => color(p.status_success),
        HighlightKind::Number => color(p.seal_solid),
        HighlightKind::Comment => color(p.text_muted),
        HighlightKind::Constant => color(p.status_warning),
        HighlightKind::Attribute => color(p.text_muted),
        HighlightKind::Punctuation => color(p.text_body),
    }
}

/// Total content height for `line_count` lines at `font_size` — used to size
/// the `Canvas` so it can sit inside a `scrollable` for vertical scrolling.
pub fn content_height(line_count: usize, font_size: f32) -> f32 {
    TOP_PAD * 2.0 + line_count as f32 * font_size * LINE_HEIGHT_RATIO
}

/// Absolute y-position (canvas/document coordinates, i.e. what
/// `scroll_offset` is measured against) of the top of `line` at `font_size`.
/// Used by Find navigation to decide whether a match is already within the
/// visible scroll range before scrolling to it.
pub fn line_top(line: usize, font_size: f32) -> f32 {
    TOP_PAD + line as f32 * font_size * LINE_HEIGHT_RATIO
}

/// Line height in px at `font_size` — see `line_top`.
pub fn line_height_px(font_size: f32) -> f32 {
    font_size * LINE_HEIGHT_RATIO
}

/// Approximate window-relative pixel position of the bottom-left corner of
/// the cursor glyph, for anchoring overlay widgets (e.g. the completion
/// popup) near the typing point.
///
/// `header_height` should include everything above the editor's scrollable
/// area (title bar + tab bar). The result is approximate; sub-pixel and font
/// metric variations mean it may be off by a few pixels.
pub fn cursor_pixel_pos(
    line: usize,
    col: usize,
    font_size: f32,
    scroll_offset: f32,
    header_height: f32,
) -> (f32, f32) {
    let line_height = font_size * LINE_HEIGHT_RATIO;
    let char_width = font_size * CHAR_WIDTH_RATIO;
    let x = GUTTER_WIDTH + TEXT_INSET + col as f32 * char_width;
    // +1 so the popup appears *below* the cursor line, not overlapping it.
    let y = header_height + TOP_PAD + (line as f32 + 1.0) * line_height - scroll_offset;
    (x, y)
}
