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
    pub pending_revert_line: Option<usize>,
    /// Toggled from the settings panel. Only hides the inline `// message`
    /// annotation — the wavy underline stays either way.
    pub problem_lens_enabled: bool,
    /// Toggled from the settings panel. Only hides the gutter's line-number
    /// digits — the gutter itself (git-diff marks, revert clicks) still
    /// works either way, and an armed line's "Revert" prompt still shows.
    pub show_line_numbers: bool,
    /// Toggled from the settings panel. When on, a buffer line wider than
    /// the pane renders as several visual rows instead of scrolling
    /// sideways — see the module-level wrap functions (`wrap_row_starts`,
    /// `wrap_row_offsets`) for how `draw`/`hit_test` share one code path
    /// with the unwrapped case (a line is just "one row" there) rather than
    /// duplicating all the position math per mode.
    pub word_wrap: bool,
    /// Cumulative visual-row count before each buffer line, plus a trailing
    /// total (`wrap_row_offsets`'s own doc comment) — `shell.rs` computes
    /// this once per view rebuild (it needs `Document` access anyway, to
    /// size the canvas) and both `draw` and `hit_test` reuse the same copy
    /// rather than each re-deriving it. Empty and unused whenever
    /// `word_wrap` is off.
    pub wrap_offsets: Rc<Vec<u32>>,
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
    /// The active GitHub Copilot inline suggestion's first line, already
    /// position-validated against the current cursor by the caller (`None`
    /// whenever `editor.ghost_completion` doesn't match `editor.cursor`) —
    /// see `EditorState::ghost_completion`'s own doc comment for why only
    /// the first line is shown even for a multi-line suggestion (accepting
    /// still inserts the whole thing; this is a rendering-only limitation).
    pub ghost_text: Option<String>,
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
    /// The `(line, col)` the mouse was last resting over — lets hover
    /// tracking publish `Message::EditorHoverMove` only on an actual cell
    /// change, not on every sub-pixel `CursorMoved` a stationary mouse can
    /// still generate.
    hover_cell: Option<(usize, usize)>,
}

const CLICK_STREAK_WINDOW: Duration = Duration::from_millis(450);

impl EditorCanvas {
    fn line_height(&self) -> f32 {
        self.font_size * LINE_HEIGHT_RATIO
    }

    fn char_width(&self) -> f32 {
        self.font_size * CHAR_WIDTH_RATIO
    }

    /// `canvas_width` is `bounds.width` from whichever `canvas::Event`
    /// handler called this — needed only for `word_wrap`'s column math
    /// (`wrap_cols_for`), since unwrapped hit-testing never had to know the
    /// canvas's own width (a line's column is just `x / char_width`
    /// regardless of how wide the canvas is).
    fn hit_test(&self, position: Point, canvas_width: f32) -> (usize, usize) {
        let line_height = self.line_height();
        let char_width = self.char_width();
        let total_lines = self.document.line_count().max(1);

        if self.word_wrap {
            let total_rows = self.wrap_offsets.last().copied().unwrap_or(1).max(1);
            let row = (((position.y - TOP_PAD) / line_height).floor().max(0.0) as u32)
                .min(total_rows - 1);
            // Last buffer line whose starting row is still `<= row`.
            let line = self
                .wrap_offsets
                .partition_point(|&o| o <= row)
                .saturating_sub(1)
                .min(total_lines - 1);
            let row_in_line = (row - self.wrap_offsets[line]) as usize;

            let wrap_cols = wrap_cols_for(canvas_width - GUTTER_WIDTH - TEXT_INSET, char_width);
            let text = self.document.line_text_capped(line, crate::state::MAX_RENDERED_LINE_CHARS);
            let row_starts = wrap_row_starts(&text, wrap_cols);
            let row_in_line = row_in_line.min(row_starts.len() - 1);
            let row_start_char = byte_to_char(&text, row_starts[row_in_line]);
            let row_end_char = row_starts
                .get(row_in_line + 1)
                .map(|&b| byte_to_char(&text, b))
                .unwrap_or_else(|| text.chars().count());

            let x = position.x - GUTTER_WIDTH - TEXT_INSET;
            let col_in_row = if x <= 0.0 { 0 } else { (x / char_width).round() as usize };
            let col = row_start_char + col_in_row.min(row_end_char - row_start_char);
            return (line, col);
        }

        let line = ((position.y - TOP_PAD) / line_height).floor();
        let line = (line.max(0.0) as usize).min(total_lines - 1);

        let x = position.x - GUTTER_WIDTH - TEXT_INSET;
        let col = if x <= 0.0 {
            0
        } else {
            (x / char_width).round() as usize
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

        // Command/control/alt-modified keys are shortcuts, not text or
        // navigation. This has to be decided *before* the named-key branch
        // below, which returns unconditionally: with the check left until
        // after it, Ctrl+Enter inserted a newline, Ctrl+Tab typed four
        // spaces, Ctrl+Backspace ate a character, and any app-wide shortcut
        // on a named key was silently swallowed by this canvas.
        //
        // Shift is deliberately absent: it is what turns the arrow/Home/End
        // keys into selection-extending moves.
        let plain = !(modifiers.command() || modifiers.control() || modifiers.alt());

        if plain && let Key::Named(named) = key {
            let extend = modifiers.shift();
            return match named {
                Named::Enter => publish(Message::EditorInsertText("\n".into())),
                Named::Space => publish(Message::EditorInsertText(" ".into())),
                Named::Backspace => publish(Message::EditorBackspace),
                Named::Delete => publish(Message::EditorDelete),
                Named::Tab => {
                    if extend {
                        publish(Message::EditorDedent)
                    } else {
                        publish(Message::EditorIndent)
                    }
                }
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
                // Only captured while a ghost-text suggestion is actually
                // showing — an unguarded arm here would swallow Escape even
                // with nothing to dismiss, keeping it from ever reaching the
                // app-wide Escape handling (`iced::keyboard::listen()` only
                // sees events no focused widget captured).
                Named::Escape if self.ghost_text.is_some() => publish(Message::DismissGhostCompletion),
                // Same VS Code bindings: `F12` "Go to Definition", `Shift+F12`
                // "Find All References" — both act on the cursor's current
                // position rather than needing a fresh click.
                Named::F12 => {
                    if extend {
                        publish(Message::FindReferences { line: self.cursor.line, col: self.cursor.col })
                    } else {
                        publish(Message::GoToDefinition { line: self.cursor.line, col: self.cursor.col })
                    }
                }
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
            } else if c.eq_ignore_ascii_case("/") {
                // Captured here rather than left to bubble to the app-wide
                // shortcut handler, which maps the same chord to "open
                // shortcuts help" — toggle-comment is what `Cmd+/` should do
                // while actually typing in a file; the help screen stays
                // reachable from everywhere else (sidebar, chat, welcome).
                publish(Message::EditorToggleComment)
            } else {
                None
            };
        }

        // Same reasoning as `plain` above, now for the `text` payload: Ctrl+S
        // shouldn't type an "s".
        if !plain {
            return None;
        }

        let text = text?;
        if text.is_empty() || text.chars().any(|c| c.is_control()) {
            return None;
        }
        // A lone char is a genuine physical keystroke — route it through
        // `EditorTypeChar` so auto-pairing (`EditorState::type_char`) gets a
        // chance at it. Anything longer (rare — some IME composition
        // sequences) is left as a literal `EditorInsertText`, same as
        // before; auto-pairing only ever makes sense for one character
        // typed at a time.
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => publish(Message::EditorTypeChar(ch)),
            _ => publish(Message::EditorInsertText(text.to_string())),
        }
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
                // A press outside this canvas hands focus away. Nothing used
                // to clear `focused` at all, and `handle_key` captures every
                // keystroke while it is set — so after one click in the
                // editor, typing into the chat panel's input (which sits
                // *after* `code_area` in the shell's row, and so only sees
                // events this canvas declines) went into the source file
                // instead. Focus has to be released explicitly, because an
                // outside click is exactly the event that never reaches the
                // branch below.
                let Some(position) = cursor.position_in(bounds) else {
                    state.focused = false;
                    return None;
                };
                state.focused = true;
                let (line, col) = self.hit_test(position, bounds.width);

                // Ctrl/Cmd+Click jumps to the symbol's definition instead of
                // placing the cursor or starting a drag-select — the same
                // modifier VS Code uses for this gesture. Checked before the
                // gutter/revert handling below since it's meant to apply
                // anywhere over the text, not just plain clicks.
                if state.modifiers.command() {
                    return Some(canvas::Action::publish(Message::GoToDefinition { line, col }).and_capture());
                }

                // A click on a changed line's gutter marker arms that line
                // for revert instead of placing the cursor or starting a
                // drag-select — the marker itself is the affordance (unlike
                // a Changes-panel row, a changed line's bar is always
                // visible, so there's no separate hover-reveal step
                // needed). A second click on the same already-armed marker
                // confirms and actually reverts it (drawn as a "Revert"
                // label in place of the line number — see `draw`), so a
                // stray click can't silently discard edits. Unmarked-line
                // gutter clicks fall through unchanged, to today's
                // cursor-to-column-0.
                if position.x < GUTTER_WIDTH && self.gutter_marks.get(line).and_then(Option::as_ref).is_some() {
                    let message = if self.pending_revert_line == Some(line) {
                        Message::RevertLine { line }
                    } else {
                        Message::PromptRevertLine { line }
                    };
                    return Some(canvas::Action::publish(message).and_capture());
                }

                // Any other click while a revert is armed just dismisses the
                // prompt, the same "click away to cancel" shape as a context
                // menu — a stray click can't both cancel the prompt and move
                // the cursor/selection in one motion, so the next click
                // always lands where the user actually meant it to.
                if self.pending_revert_line.is_some() {
                    return Some(canvas::Action::publish(Message::CancelRevertLine).and_capture());
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
                let (line, col) = self.hit_test(position, bounds.width);
                Some(canvas::Action::publish(Message::EditorClick { line, col, extend: true }).and_capture())
            }
            // Passive hover tracking (dwell-based `textDocument/hover`) —
            // only published on an actual cell change, so a stationary
            // mouse's sub-pixel `CursorMoved` noise doesn't restart the
            // dwell timer or force a `view()` rebuild every frame.
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if !state.dragging => {
                match cursor.position_in(bounds) {
                    Some(position) => {
                        let cell = self.hit_test(position, bounds.width);
                        if state.hover_cell == Some(cell) {
                            None
                        } else {
                            state.hover_cell = Some(cell);
                            Some(canvas::Action::publish(Message::EditorHoverMove {
                                line: cell.0,
                                col: cell.1,
                            }))
                        }
                    }
                    None if state.hover_cell.take().is_some() => {
                        Some(canvas::Action::publish(Message::EditorHoverLeave))
                    }
                    None => None,
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                None
            }
            // Right/middle presses outside the canvas give focus away too —
            // same reason as the left-button case above (a right-click into
            // the sidebar opens its context menu, and shouldn't leave this
            // canvas still eating the keyboard).
            canvas::Event::Mouse(mouse::Event::ButtonPressed(_)) => {
                if cursor.position_in(bounds).is_none() {
                    state.focused = false;
                }
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
        let text_x0 = GUTTER_WIDTH + TEXT_INSET;
        let wrap_cols = wrap_cols_for(bounds.width - text_x0, char_width);

        // Visible range in *visual rows* — for an unwrapped document a row
        // is a buffer line, so `total_rows == total_lines` and
        // `first_line..last_line` below is identical to the pre-wrap
        // arithmetic; wrapping just means the two can diverge.
        let total_rows = if self.word_wrap {
            self.wrap_offsets.last().copied().unwrap_or(0) as usize
        } else {
            total_lines
        };
        let first_onscreen_row =
            ((self.scroll_offset - TOP_PAD).max(0.0) / line_height).floor() as usize;
        let onscreen_rows = (self.viewport_height.max(0.0) / line_height).ceil() as usize + 1;
        let first_row = first_onscreen_row.saturating_sub(OVERSCAN_LINES);
        let last_row = (first_onscreen_row + onscreen_rows + OVERSCAN_LINES).min(total_rows);

        let (first_line, last_line) = if self.word_wrap {
            let first_line = self
                .wrap_offsets
                .partition_point(|&o| (o as usize) <= first_row)
                .saturating_sub(1)
                .min(total_lines.saturating_sub(1));
            let last_line = self
                .wrap_offsets
                .partition_point(|&o| (o as usize) < last_row)
                .min(total_lines);
            (first_line, last_line.max(first_line))
        } else {
            (first_row, last_row)
        };

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

        // Horizontal culling. Without word wrap, the canvas is sized to the
        // document's widest line (`shell.rs`'s `content_width`, from
        // `EditorState::max_line_chars`) so it can scroll sideways — but
        // that width is itself capped at `MAX_RENDERED_LINE_CHARS`, so
        // `bounds.width` here is bounded too. With word wrap there's no
        // horizontal scroll at all (the canvas is exactly the pane's
        // width), so every wrapped row is fully on-screen by construction —
        // only the pathological-line cap still matters there. Without a cap
        // somewhere, a project can hold lines with hundreds of thousands of
        // chars (a minified bundle, a generated blob), and materializing
        // one whole such line as a `String` and handing it to the text
        // shaper on *every* frame — with the per-span column arithmetic
        // below quadratic in the line's length on top of that — was enough
        // to wedge the app just by landing on one via Find navigation.
        let max_cols = if self.word_wrap {
            crate::state::MAX_RENDERED_LINE_CHARS
        } else {
            // +1 so a glyph straddling the right edge is still drawn, not
            // clipped to nothing.
            (((bounds.width - text_x0).max(0.0) / char_width).ceil() as usize) + 1
        };
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
            let is_cursor_line = line == self.cursor.line;
            let line_start_idx = self.document.char_index(line, 0);
            let line_len = self.document.line_len_chars(line);
            let line_end_idx = line_start_idx + line_len;
            // Treat the line terminator as one extra selectable column so a
            // selection spanning multiple lines visibly covers each line's end.
            let selectable_end = line_end_idx + 1;
            let line_start_byte = self.document.text().line_to_byte(line);

            while span_cursor < self.highlights.len()
                && self.highlights[span_cursor].end <= line_start_byte
            {
                span_cursor += 1;
            }

            // Only the columns that fit on screen (or, wrapped, up to the
            // pathological-line cap) — see `max_cols`.
            let text = self.document.line_text_capped(line, max_cols);
            // A single row spanning the whole (capped) line when unwrapped —
            // the degenerate case that keeps this one loop correct for both
            // modes instead of duplicating everything below per mode. See
            // `wrap_row_starts`'s own doc comment for the wrapped case.
            let row_starts: Vec<usize> =
                if self.word_wrap { wrap_row_starts(&text, wrap_cols) } else { vec![0] };
            let base_row = if self.word_wrap { self.wrap_offsets[line] as usize } else { line };
            let text_len_bytes = text.len();

            for (row_in_line, &row_start_byte) in row_starts.iter().enumerate() {
                let row_is_last = row_in_line + 1 == row_starts.len();
                let row_end_byte = row_starts.get(row_in_line + 1).copied().unwrap_or(text_len_bytes);
                let row_start_char = byte_to_char(&text, row_start_byte);
                let row_end_char = if row_is_last { line_len.min(byte_to_char(&text, text_len_bytes)) } else { byte_to_char(&text, row_end_byte) };
                let y = TOP_PAD + (base_row + row_in_line) as f32 * line_height;

                if is_cursor_line {
                    frame.fill(
                        &Path::rectangle(
                            Point::new(GUTTER_WIDTH, y),
                            Size::new(bounds.width - GUTTER_WIDTH, line_height),
                        ),
                        tint(p.accent_solid, 0.08),
                    );
                }

                let row_start_idx = line_start_idx + row_start_char;
                let row_end_idx = line_start_idx + row_end_char;
                let row_selectable_end = if row_is_last { selectable_end } else { row_end_idx };

                if let Some((start, end)) = self.selection
                    && start < row_selectable_end
                    && end > row_start_idx
                {
                    let sel_start_col = start.saturating_sub(row_start_idx);
                    let sel_end_col = (end.saturating_sub(row_start_idx))
                        .min(row_end_char - row_start_char + if row_is_last { 1 } else { 0 });
                    let x0 = text_x0 + sel_start_col as f32 * char_width;
                    let x1 = clamp_x(text_x0 + sel_end_col as f32 * char_width);
                    // 0.35, not the old 0.22 — text paints on top of this
                    // fill later in the same frame (see the glyph-drawing
                    // pass below), so raising the alpha only makes the
                    // selection itself easier to spot against syntax-colored
                    // text; it can't hurt legibility. Matches the alpha
                    // `command_palette.rs`'s own text-input selection uses,
                    // rather than picking a new one-off value.
                    let rect = Path::rectangle(
                        Point::new(x0, y),
                        Size::new((x1 - x0).max(char_width * 0.4), line_height),
                    );
                    frame.fill(&rect, tint(p.accent_solid, 0.6));
                }

                for (i, (start, end)) in find_visible.iter().enumerate() {
                    if *start >= row_end_idx || *end <= row_start_idx {
                        continue;
                    }
                    let match_start_col = start.saturating_sub(row_start_idx);
                    if text_x0 + match_start_col as f32 * char_width > bounds.width {
                        continue;
                    }
                    let is_current = find_base + i == self.find_current;
                    let match_end_col = (end.saturating_sub(row_start_idx)).min(row_end_char - row_start_char);
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

                // The gutter's per-line git-diff marker and line number only
                // belong on a line's first visual row — both are per-buffer-
                // line concepts (`update()`'s gutter click handling reverts
                // a whole line, not a wrapped fragment of one), so
                // continuation rows leave the gutter blank rather than
                // repeating either.
                if row_in_line == 0 {
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
                    let armed = self.pending_revert_line == Some(line);
                    if let Some(mark) = self.gutter_marks.get(line).and_then(Option::as_ref) {
                        let accent = match mark {
                            GutterMark::Added => p.status_success,
                            GutterMark::Modified { .. } => p.status_warning,
                            GutterMark::RemovedAbove { .. } => p.status_danger,
                        };
                        // Armed (one click already landed here, waiting on the
                        // confirming second one) reads as an unmistakably different,
                        // stronger-danger cell than the passive added/modified/
                        // removed tint above — see `update()`'s gutter click
                        // handling for the two-click shape this is confirming.
                        frame.fill(
                            &Path::rectangle(Point::new(0.0, y), Size::new(GUTTER_WIDTH, line_height)),
                            tint(if armed { p.status_danger } else { accent }, if armed { 0.30 } else { 0.16 }),
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

                    // Armed lines swap their line number for a "Revert" prompt —
                    // the same marker cell doubles as the confirm button a second
                    // click on it fires (`update()`), so labeling it is what makes
                    // that second click legible rather than a guess. This still
                    // shows with `show_line_numbers` off: it's a pending action, not
                    // a line number.
                    if armed || self.show_line_numbers {
                        frame.fill_text(Text {
                            content: if armed { "Revert".to_string() } else { (line + 1).to_string() },
                            position: Point::new(GUTTER_WIDTH - if armed { 4.0 } else { 14.0 }, y),
                            color: if armed {
                                color(p.status_danger)
                            } else if is_cursor_line {
                                color(p.text_strong)
                            } else {
                                tint(p.text_muted, 0.6)
                            },
                            size: Pixels(if armed { 10.0 } else { 11.0 }),
                            line_height: LineHeight::Absolute(Pixels(line_height)),
                            font: if armed { fonts::sans(iced::font::Weight::Medium) } else { mono },
                            align_x: Alignment::Right,
                            align_y: Vertical::Top,
                            ..Text::default()
                        });
                    }
                }

                let row_text = &text[row_start_byte..row_end_byte];
                if !row_text.is_empty() {
                    let row_start_byte_abs = line_start_byte + row_start_byte;
                    let row_end_byte_abs = line_start_byte + row_end_byte;

                    if self.highlights.is_empty() {
                        frame.fill_text(Text {
                            content: row_text.to_string(),
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
                        // offset to a column in one forward pass over the row.
                        // Counting `text[..seg_start].chars()` afresh per span
                        // instead made this quadratic in the row's length. `idx`
                        // is a fresh local walk from `span_cursor` every row (not
                        // written back to it) — `span_cursor` only advances at
                        // line granularity above, so a span straddling a wrap
                        // boundary is walked again, once per row it touches,
                        // clipped to that row's own byte range each time.
                        let mut walked_bytes = row_start_byte;
                        let mut walked_chars = row_start_char;
                        let mut idx = span_cursor;
                        while idx < self.highlights.len() && self.highlights[idx].start < row_end_byte_abs
                        {
                            let span = self.highlights[idx];
                            let seg_start = span.start.max(row_start_byte_abs) - line_start_byte;
                            let seg_end = span.end.min(row_end_byte_abs) - line_start_byte;
                            if seg_start < seg_end {
                                if seg_start < walked_bytes {
                                    // Defensive: an out-of-order span would make
                                    // the forward walk wrong, so restart it.
                                    walked_bytes = row_start_byte;
                                    walked_chars = row_start_char;
                                }
                                walked_chars += text[walked_bytes..seg_start].chars().count();
                                walked_bytes = seg_start;
                                let content = text[seg_start..seg_end].to_string();
                                frame.fill_text(Text {
                                    content,
                                    position: Point::new(
                                        text_x0 + (walked_chars - row_start_char) as f32 * char_width,
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
                    let clip_start = seg_start_col.max(row_start_char);
                    let clip_end = seg_end_col.min(row_end_char);
                    let sev_color = severity_color(diag.severity, p);
                    if clip_start < clip_end {
                        let x0 = text_x0 + (clip_start - row_start_char) as f32 * char_width;
                        // A diagnostic covering a whole pathological line would
                        // otherwise squiggle its way across a million pixels, one
                        // `PERIOD`-wide segment at a time.
                        let x1 = clamp_x(text_x0 + (clip_end - row_start_char) as f32 * char_width);
                        if x0 <= bounds.width {
                            draw_wavy_underline(&mut frame, x0, x1, y + line_height - 5.0, color(sev_color));
                        }
                    }

                    // The inline `// message` lens only ever shows once per
                    // line, past its very last visual row — showing it after
                    // every wrapped row would repeat the same annotation
                    // several times over for one diagnostic.
                    if row_is_last && self.problem_lens_enabled && lens.is_none() {
                        lens = Some((diag.message.as_str(), sev_color));
                    }
                }
                if let Some((message, sev_color)) = lens {
                    let lens_col = (row_end_char - row_start_char) + 3;
                    let lens_x = text_x0 + lens_col as f32 * char_width;
                    // Past the canvas's own (capped) width the annotation is
                    // unreachable even via horizontal scroll, so shaping it
                    // would be pure waste — only bites on the rare line long
                    // enough to hit `MAX_RENDERED_LINE_CHARS`.
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

                let cursor_in_row = is_cursor_line
                    && self.cursor.col >= row_start_char
                    && (self.cursor.col < row_end_char || (row_is_last && self.cursor.col <= row_end_char));

                if cursor_in_row && self.caret_visible {
                    let x = text_x0 + (self.cursor.col - row_start_char) as f32 * char_width;
                    if x <= bounds.width {
                        frame.fill(
                            &Path::rectangle(Point::new(x, y + 1.0), Size::new(2.0, line_height - 4.0)),
                            color(p.accent_solid),
                        );
                    }
                }
                // Ghost text steadily shown (not gated on `caret_visible`) —
                // VS Code's own inline suggestions don't blink either, only the
                // caret drawn in front of them does.
                if cursor_in_row
                    && let Some(ghost) = &self.ghost_text
                {
                    let x = text_x0 + (self.cursor.col - row_start_char) as f32 * char_width;
                    if x <= bounds.width {
                        frame.fill_text(Text {
                            content: ghost.clone(),
                            position: Point::new(x, y),
                            color: tint(p.text_muted, 0.6),
                            size: Pixels(font_size),
                            line_height: LineHeight::Absolute(Pixels(line_height)),
                            font: mono,
                            align_y: Vertical::Top,
                            ..Text::default()
                        });
                    }
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

/// Maps a syntax category onto the current theme's dedicated `syntax_*`
/// tokens — a wider, more mutually-distinct hue set than the chrome tokens
/// (`accent_solid`, `status_info`, etc.) it used to borrow, which collapsed
/// several kinds onto the same color (keyword and type were identical).
pub(crate) fn highlight_color(kind: HighlightKind, p: Palette) -> Color {
    match kind {
        HighlightKind::Default => color(p.text_body),
        HighlightKind::Keyword => color(p.syntax_keyword),
        HighlightKind::Type => color(p.syntax_type),
        HighlightKind::Function => color(p.syntax_function),
        HighlightKind::Macro => color(p.syntax_macro),
        HighlightKind::String => color(p.syntax_string),
        HighlightKind::Number => color(p.syntax_number),
        HighlightKind::Comment => color(p.syntax_comment),
        HighlightKind::Constant => color(p.syntax_constant),
        HighlightKind::Attribute => color(p.syntax_attribute),
        HighlightKind::Punctuation => color(p.syntax_punctuation),
    }
}

/// Total content height for `line_count` lines at `font_size` — used to size
/// the `Canvas` so it can sit inside a `scrollable` for vertical scrolling.
pub fn content_height(line_count: usize, font_size: f32) -> f32 {
    TOP_PAD * 2.0 + line_count as f32 * font_size * LINE_HEIGHT_RATIO
}

/// Total content width for a document whose longest line is
/// `max_line_chars` (already capped — see `EditorState::max_line_chars`) at
/// `font_size` — the horizontal sibling of `content_height`, sizing the
/// canvas so it can scroll sideways within a `scrollable` instead of
/// clipping anything past the pane's own width.
pub fn content_width(max_line_chars: usize, font_size: f32) -> f32 {
    let char_width = font_size * CHAR_WIDTH_RATIO;
    // A little slack past the last char so the caret sitting right at
    // end-of-line, and a diagnostic's inline `// message` lens past it,
    // both stay fully reachable rather than landing exactly on the
    // scrollable's own right edge.
    GUTTER_WIDTH + TEXT_INSET + max_line_chars as f32 * char_width + char_width * 24.0
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

/// Absolute x-position (canvas coordinates, what `scroll_offset_x` is
/// measured against) of the left edge of the glyph at `col`, at `font_size`
/// — the horizontal sibling of `line_top`, used by
/// `scroll_cursor_into_view`'s horizontal half.
pub fn col_left(col: usize, font_size: f32) -> f32 {
    GUTTER_WIDTH + TEXT_INSET + col as f32 * (font_size * CHAR_WIDTH_RATIO)
}

/// Char width in px at `font_size` — see `col_left`.
pub fn char_width_px(font_size: f32) -> f32 {
    font_size * CHAR_WIDTH_RATIO
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
    scroll_offset_x: f32,
    header_height: f32,
) -> (f32, f32) {
    let line_height = font_size * LINE_HEIGHT_RATIO;
    let char_width = font_size * CHAR_WIDTH_RATIO;
    let x = GUTTER_WIDTH + TEXT_INSET + col as f32 * char_width - scroll_offset_x;
    // +1 so the popup appears *below* the cursor line, not overlapping it.
    let y = header_height + TOP_PAD + (line as f32 + 1.0) * line_height - scroll_offset;
    (x, y)
}

/// Wrap-aware sibling of `cursor_pixel_pos`. A logical `(line, col)` no
/// longer lands at a fixed pixel row once word wrap is on — it can be on
/// any of that line's visual rows — so this additionally re-derives which
/// row `col` falls on (`wrap_row_starts`) and how many rows precede `line`
/// altogether (`wrap_row_offsets`) before doing the same "+1 row, minus
/// scroll" placement `cursor_pixel_pos` does. Falls back to the plain
/// unwrapped version when `word_wrap` is false, so callers can always go
/// through this one rather than branching themselves.
#[allow(clippy::too_many_arguments)]
pub fn cursor_pixel_pos_wrapped(
    document: &Document,
    word_wrap: bool,
    line: usize,
    col: usize,
    font_size: f32,
    scroll_offset: f32,
    scroll_offset_x: f32,
    header_height: f32,
    pane_width: f32,
) -> (f32, f32) {
    if !word_wrap {
        return cursor_pixel_pos(line, col, font_size, scroll_offset, scroll_offset_x, header_height);
    }
    let char_width = font_size * CHAR_WIDTH_RATIO;
    let line_height = font_size * LINE_HEIGHT_RATIO;
    let wrap_cols = wrap_cols_for_pane(pane_width, font_size);
    let text = document.line_text_capped(line, crate::state::MAX_RENDERED_LINE_CHARS);
    let col_byte = char_to_byte(&text, col);
    let row_starts = wrap_row_starts(&text, wrap_cols);
    let row_in_line = row_starts.partition_point(|&b| b <= col_byte).saturating_sub(1).min(row_starts.len() - 1);
    let row_start_char = byte_to_char(&text, row_starts[row_in_line]);

    let row_top = row_top_wrapped(document, wrap_cols, line, col, font_size);
    let x = GUTTER_WIDTH + TEXT_INSET + (col - row_start_char) as f32 * char_width - scroll_offset_x;
    // +1 row so the popup appears *below* the row, not overlapping it.
    let y = header_height + row_top + line_height - scroll_offset;
    (x, y)
}

/// Absolute y-position (canvas coordinates, what `scroll_offset` is measured
/// against) of the top of the specific *visual row* that `(line, col)` falls
/// on under word wrap — the wrap-aware sibling of `line_top`. A buffer
/// line's own first row depends on how many rows every wrapped line before
/// it took (`wrap_row_offsets`), and `col` can put the position on any of
/// that line's *own* rows too (`wrap_row_starts`), not just its first —
/// callers that only care about a line's start (not a specific column) can
/// pass `col: 0`, which always resolves to that line's own row 0.
pub fn row_top_wrapped(document: &Document, wrap_cols: usize, line: usize, col: usize, font_size: f32) -> f32 {
    let text = document.line_text_capped(line, crate::state::MAX_RENDERED_LINE_CHARS);
    let col_byte = char_to_byte(&text, col);
    let row_starts = wrap_row_starts(&text, wrap_cols);
    let row_in_line = row_starts.partition_point(|&b| b <= col_byte).saturating_sub(1).min(row_starts.len() - 1);
    let rows_before_line = wrap_row_offsets(document, wrap_cols).get(line).copied().unwrap_or(0);
    let visual_row = rows_before_line as usize + row_in_line;
    TOP_PAD + visual_row as f32 * font_size * LINE_HEIGHT_RATIO
}

/// Characters that fit in one wrapped row, given the *text* width already
/// (gutter/inset excluded) — at least 1, so a pathologically narrow pane
/// still makes progress one char at a time instead of never breaking.
fn wrap_cols_for(text_width: f32, char_width: f32) -> usize {
    ((text_width.max(0.0)) / char_width).floor().max(1.0) as usize
}

/// `wrap_cols_for`'s public sibling for callers outside this module, which
/// only ever have the pane's *whole* width (gutter included) to hand —
/// `GUTTER_WIDTH`/`TEXT_INSET` are private to this file, so this is the one
/// place that subtracts them out.
pub fn wrap_cols_for_pane(pane_width: f32, font_size: f32) -> usize {
    wrap_cols_for(pane_width - GUTTER_WIDTH - TEXT_INSET, font_size * CHAR_WIDTH_RATIO)
}

/// Visual row boundaries within one buffer line, as **byte** offsets into
/// `text` (so callers can slice it directly without a further char→byte
/// conversion) — `[0, b1, b2, ...]`, one entry per visual row. Always
/// non-empty: an empty line still has a single, empty row starting at 0.
///
/// Greedy word-wrap: counts *chars* per row (matching the monospace
/// char-cell width the rest of this file assumes, not byte length) and
/// breaks at the byte just past the row's last space when the row has one,
/// otherwise hard-breaks at exactly `wrap_cols` chars — so one unbroken
/// token (a long URL, a minified identifier) still can't grow a row past
/// its column budget. The row *count* this produces is what
/// `wrap_row_offsets` keys the vertical layout on, so any change here has
/// to stay consistent with it.
fn wrap_row_starts(text: &str, wrap_cols: usize) -> Vec<usize> {
    let wrap_cols = wrap_cols.max(1);
    let mut starts = vec![0usize];
    let mut row_start_byte = 0usize;
    let mut row_chars = 0usize;
    // Byte offset just *after* the most recent space seen in the current row.
    let mut last_space: Option<usize> = None;

    for (byte_idx, ch) in text.char_indices() {
        if row_chars >= wrap_cols {
            let break_at = match last_space {
                Some(s) if s > row_start_byte => s,
                _ => byte_idx,
            };
            starts.push(break_at);
            // At most `wrap_cols` chars can sit in `[break_at, byte_idx)` —
            // the row that just ended was exactly `wrap_cols` chars, and
            // `break_at` is somewhere inside it — so this recount is
            // O(wrap_cols), not O(line length).
            row_chars = text[break_at..byte_idx].chars().count();
            row_start_byte = break_at;
            last_space = None;
        }
        if ch == ' ' {
            last_space = Some(byte_idx + ch.len_utf8());
        }
        row_chars += 1;
    }
    starts
}

/// Cumulative visual-row count before each buffer line, plus a trailing
/// total — entry `i` is how many visual rows lines `0..i` occupy, so this
/// has `document.line_count() + 1` entries and its last one is the whole
/// document's row count. This is what lets `draw`/`hit_test` binary-search
/// "which buffer line is at visual row R" instead of walking the document
/// from the top.
///
/// Only ever computed while word wrap is actually on (an opt-in setting,
/// off by default) — its O(document length) cost is paid on every call
/// rather than chased down with the grow-only/settle-healed caching
/// `EditorState::max_line_chars` uses for the same reason: word wrap is
/// mostly reached for prose/markdown-sized files, not the multi-hundred-
/// thousand-line sources that caching exists to protect.
pub fn wrap_row_offsets(document: &Document, wrap_cols: usize) -> Vec<u32> {
    let line_count = document.line_count();
    let mut offsets = Vec::with_capacity(line_count + 1);
    let mut total = 0u32;
    for line in 0..line_count {
        offsets.push(total);
        let text = document.line_text_capped(line, crate::state::MAX_RENDERED_LINE_CHARS);
        total += wrap_row_starts(&text, wrap_cols).len() as u32;
    }
    offsets.push(total);
    offsets
}

/// Byte offset of the `char_idx`-th char of `text`, or `text.len()` past
/// the end — the inverse of `byte_to_char`, needed wherever a logical
/// column (char-indexed, like the rest of the document model) has to slice
/// or compare against a wrap row boundary (byte-indexed, like
/// `wrap_row_starts`'s output).
fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(text.len())
}

/// Char offset of the char starting at byte `byte_idx` of `text` — the
/// inverse of `char_to_byte`.
fn byte_to_char(text: &str, byte_idx: usize) -> usize {
    text[..byte_idx.min(text.len())].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use devscribe_core::Document;

    #[test]
    fn wrap_row_starts_does_not_break_a_line_shorter_than_wrap_cols() {
        assert_eq!(wrap_row_starts("hello", 80), vec![0]);
    }

    #[test]
    fn wrap_row_starts_does_not_break_a_line_exactly_wrap_cols_long() {
        assert_eq!(wrap_row_starts("abcde", 5), vec![0]);
    }

    #[test]
    fn wrap_row_starts_hard_breaks_an_unbroken_token_with_no_space() {
        // "abcdef" has no space to break at, so it hard-breaks at exactly
        // `wrap_cols` chars rather than growing past its budget.
        assert_eq!(wrap_row_starts("abcdef", 5), vec![0, 5]);
    }

    #[test]
    fn wrap_row_starts_breaks_at_the_last_space_within_the_row() {
        // wrap_cols=5: "ab cdef" overflows at the 6th char ('e'); the row's
        // last space is right after "ab", so it breaks there (byte 3) and
        // "cdef" moves to the next row, rather than hard-breaking mid-word.
        assert_eq!(wrap_row_starts("ab cdef", 5), vec![0, 3]);
    }

    #[test]
    fn wrap_row_starts_treats_an_empty_line_as_a_single_row() {
        assert_eq!(wrap_row_starts("", 80), vec![0]);
    }

    #[test]
    fn wrap_row_starts_clamps_wrap_cols_to_at_least_one() {
        // A pathologically narrow pane still makes progress one char per
        // row instead of never breaking (or panicking on a divide by zero).
        assert_eq!(wrap_row_starts("abc", 0), vec![0, 1, 2]);
    }

    #[test]
    fn wrap_row_starts_breaks_only_on_char_boundaries_for_multibyte_text() {
        // Each of these three chars is 3 bytes; breaking at a byte offset
        // that isn't a char boundary would panic the moment `draw`/
        // `hit_test` tried to slice `text` with it.
        let starts = wrap_row_starts("日本語", 2);
        assert_eq!(starts, vec![0, 6]);
        assert!("日本語".is_char_boundary(6));
    }

    #[test]
    fn wrap_row_offsets_matches_the_sum_of_each_lines_own_row_count() {
        // wrap_cols=5: "short" is 1 row, "a much longer line here" wraps
        // into several, "" is 1 row.
        let document = Document::from_str("short\na much longer line here\n\n");
        let wrap_cols = 5;
        let offsets = wrap_row_offsets(&document, wrap_cols);

        assert_eq!(offsets.len(), document.line_count() + 1);
        assert_eq!(offsets[0], 0);
        for line in 0..document.line_count() {
            let text = document.line_text_capped(line, MAX_RENDERED_LINE_CHARS_TEST);
            let rows = wrap_row_starts(&text, wrap_cols).len() as u32;
            assert_eq!(offsets[line + 1], offsets[line] + rows, "line {line}");
        }
    }

    const MAX_RENDERED_LINE_CHARS_TEST: usize = crate::state::MAX_RENDERED_LINE_CHARS;

    #[test]
    fn byte_to_char_and_char_to_byte_round_trip_through_each_other() {
        let text = "a日b本c";
        for char_idx in 0..=text.chars().count() {
            let byte_idx = char_to_byte(text, char_idx);
            assert_eq!(byte_to_char(text, byte_idx), char_idx, "char_idx {char_idx}");
        }
    }
}
