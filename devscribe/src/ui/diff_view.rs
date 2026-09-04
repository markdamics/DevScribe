//! A diff of the current file against its `HEAD` version, either unified
//! (old/new lines interleaved, git-diff style) or side-by-side — see
//! `State::diff_view_mode`. Mostly read-only — no Stage; that mutates git
//! state and deserves its own careful pass with explicit confirmation, not
//! a bolt-on here. The one exception is "revert selected changes": the user
//! checks one or more hunks and reverts just those, via the same undo-able
//! document edits the editor gutter's per-line revert already uses
//! (`EditorState::revert_lines`).
use devscribe_core::diff::{diff_words, DiffLine, DiffLineKind, Hunk, WordSpan};
use devscribe_core::theme::{Palette, Rgba};
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::color::color;
use crate::fonts;
use crate::state::{self, DiffStatus, DiffViewMode, Message, State};
use crate::widgets;

const GUTTER_WIDTH: f32 = 44.0;
const MARKER_WIDTH: f32 = 20.0;

fn tint(c: Rgba, alpha: f32) -> Color {
    color(Rgba { a: alpha, ..c })
}

fn row_tint(mut c: Rgba) -> Color {
    c.a = 0.14;
    color(c)
}

/// The stronger tint layered on top of `row_tint` for just the words a
/// `Modified` line pair's word-level diff (`diff_words`) actually flags as
/// changed — `row_tint` alone (the old whole-line-only highlighting) still
/// applies to the rest of the line, so a one-word edit on a long line reads
/// as "mostly unchanged, this bit changed" instead of "the whole line is
/// different".
fn word_tint(mut c: Rgba) -> Color {
    c.a = 0.4;
    color(c)
}

fn line_number(n: Option<usize>, p: Palette) -> Element<'static, Message> {
    let label = n.map(|n| (n + 1).to_string()).unwrap_or_default();
    text(label)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(13.0))
        .color(color(p.text_muted))
        .width(Length::Fixed(GUTTER_WIDTH))
        .into()
}

/// `diff_words(old, new)`, filtered and flagged for one side: `side` is
/// `Delete` to reconstruct `old`'s own text (keeping `Equal` + `Delete`
/// spans) or `Insert` to reconstruct `new`'s (keeping `Equal` + `Insert`).
/// The returned `bool` is whether that span is part of the change (so the
/// caller highlights it) — `Equal` spans are shared, unhighlighted context.
fn word_spans_for_side(spans: &[WordSpan], side: DiffLineKind) -> Vec<(bool, String)> {
    spans
        .iter()
        .filter(|s| s.kind == DiffLineKind::Equal || s.kind == side)
        .map(|s| (s.kind != DiffLineKind::Equal, s.text.clone()))
        .collect()
}

/// Renders a line's text as a run of small fragments, tinting only the
/// spans `word_spans_for_side` flagged as changed — used instead of a
/// single `text` widget whenever a modified line has a word-level diff to
/// show. `kind` picks the tint color (`Delete` red, `Insert` green).
fn word_spans_row(spans: &[(bool, String)], kind: DiffLineKind, p: Palette) -> Element<'static, Message> {
    let strong = match kind {
        DiffLineKind::Delete => word_tint(p.status_danger),
        DiffLineKind::Insert => word_tint(p.status_success),
        DiffLineKind::Equal => Color::TRANSPARENT,
    };
    let fragments: Vec<Element<'static, Message>> = spans
        .iter()
        .map(|(changed, s)| {
            let t = text(s.clone())
                .font(fonts::mono(Weight::Normal))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_body));
            if *changed {
                container(t)
                    .style(move |_theme| container::Style {
                        background: Some(strong.into()),
                        ..container::Style::default()
                    })
                    .into()
            } else {
                t.into()
            }
        })
        .collect();
    row(fragments).into()
}

fn line_text(line: &DiffLine, spans: Option<&[(bool, String)]>, p: Palette) -> Element<'static, Message> {
    match spans {
        Some(spans) => word_spans_row(spans, line.kind, p),
        None => text(line.text.clone())
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body))
            .into(),
    }
}

/// One diff row, unified layout: `marker` is the hunk checkbox glyph on a
/// hunk's first line, blank (but still `MARKER_WIDTH` wide, so every row's
/// columns line up) everywhere else. `spans`, when given, replaces the
/// plain whole-line text with `word_spans_row`'s per-word highlighting —
/// only ever passed for a `Modified` pair's two lines (see `hunk_block`).
fn diff_row(line: &DiffLine, marker: &'static str, marker_color: Color, spans: Option<&[(bool, String)]>, p: Palette) -> Element<'static, Message> {
    let (sign, sign_color, bg) = match line.kind {
        DiffLineKind::Equal => (" ", color(p.text_muted), None),
        DiffLineKind::Insert => ("+", color(p.status_success), Some(row_tint(p.status_success))),
        DiffLineKind::Delete => ("-", color(p.status_danger), Some(row_tint(p.status_danger))),
    };

    let content = row![
        text(marker)
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(14.0))
            .color(marker_color)
            .width(Length::Fixed(MARKER_WIDTH)),
        line_number(line.old_line, p),
        line_number(line.new_line, p),
        text(sign)
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(15.0))
            .color(sign_color)
            .width(Length::Fixed(16.0)),
        line_text(line, spans, p),
    ]
    .spacing(4.0)
    .align_y(Alignment::Center);

    container(content)
        .width(Length::Fill)
        .padding(Padding {
            top: 1.0,
            right: 12.0,
            bottom: 1.0,
            left: 8.0,
        })
        .style(move |_theme| container::Style {
            background: bg.map(Into::into),
            ..container::Style::default()
        })
        .into()
}

/// How many unchanged lines to show immediately around a hunk, on each side
/// — mirrors `git diff`'s default context. Gaps between hunks (or before the
/// first/after the last) longer than this collapse behind a separator row
/// instead of rendering the whole file.
const CONTEXT_LINES: usize = 3;

/// The collapsed-gap row standing in for `hidden` skipped unchanged lines,
/// unified layout — indented to roughly the text column so it reads as
/// "elided" rather than as a real diff row.
fn context_separator(hidden: usize, p: Palette) -> Element<'static, Message> {
    container(
        text(format!(
            "\u{22ef} {hidden} unchanged line{} \u{22ef}",
            if hidden == 1 { "" } else { "s" }
        ))
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(12.0))
        .color(color(p.text_muted)),
    )
    .width(Length::Fill)
    .padding(Padding {
        top: 4.0,
        right: 12.0,
        bottom: 4.0,
        left: 8.0 + MARKER_WIDTH + GUTTER_WIDTH * 2.0,
    })
    .into()
}

/// The paired-up shape every `Hunk`'s line range actually has (see
/// `devscribe_core::diff::hunks`' own doc comment): zero or more `Delete`
/// lines immediately followed by zero or more `Insert` lines. The first
/// `paired` of each side line up as a `Modified` pair (word-level diff
/// eligible); anything past that on the longer side is a pure
/// addition/removal with nothing to compare it against.
struct HunkShape {
    n_delete: usize,
    n_insert: usize,
    paired: usize,
}

fn hunk_shape(lines: &[DiffLine]) -> HunkShape {
    let n_delete = lines.iter().take_while(|l| l.kind == DiffLineKind::Delete).count();
    let n_insert = lines.len() - n_delete;
    HunkShape { n_delete, n_insert, paired: n_delete.min(n_insert) }
}

/// One hunk's rows, unified layout, wrapped in a single button so clicking
/// anywhere in it toggles the whole hunk's selection — the checkbox glyph
/// on its first row is a visual cue, not the only click target (small
/// glyphs make poor click targets on their own).
fn hunk_block(lines: &[DiffLine], hunk_id: usize, selected: bool, path: PathBuf, p: Palette) -> Element<'static, Message> {
    let marker_color = if selected { color(p.accent_solid) } else { tint(p.text_muted, 0.55) };
    let checkbox = if selected { "\u{2611}" } else { "\u{2610}" };
    let shape = hunk_shape(lines);

    let rows: Vec<Element<'static, Message>> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let marker = if i == 0 { checkbox } else { "" };
            let spans = match line.kind {
                DiffLineKind::Delete if i < shape.paired => {
                    let words = diff_words(&line.text, &lines[shape.n_delete + i].text);
                    Some(word_spans_for_side(&words, DiffLineKind::Delete))
                }
                DiffLineKind::Insert if i - shape.n_delete < shape.paired => {
                    let old = &lines[i - shape.n_delete];
                    let words = diff_words(&old.text, &line.text);
                    Some(word_spans_for_side(&words, DiffLineKind::Insert))
                }
                _ => None,
            };
            diff_row(line, marker, marker_color, spans.as_deref(), p)
        })
        .collect();

    button(column(rows))
        .width(Length::Fill)
        .padding(0.0)
        .on_press(Message::ToggleDiffHunkSelected { path, hunk_id })
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            let background = if selected {
                Some(tint(p.accent_solid, 0.10).into())
            } else if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            };
            button::Style { background, ..button::Style::default() }
        })
        .into()
}

/// One column's cell in a side-by-side row — `show_old_number` picks
/// whether the gutter shows `line.old_line` (left column) or `line.new_line`
/// (right column); both are populated for an `Equal` line, only one for a
/// `Delete`/`Insert`.
fn side_cell(line: &DiffLine, spans: Option<&[(bool, String)]>, show_old_number: bool, p: Palette) -> Element<'static, Message> {
    let bg = match line.kind {
        DiffLineKind::Equal => None,
        DiffLineKind::Insert => Some(row_tint(p.status_success)),
        DiffLineKind::Delete => Some(row_tint(p.status_danger)),
    };
    let number = if show_old_number { line.old_line } else { line.new_line };

    let content = row![line_number(number, p), line_text(line, spans, p)]
        .spacing(4.0)
        .align_y(Alignment::Center);

    container(content)
        .width(Length::Fill)
        .padding(Padding { top: 1.0, right: 12.0, bottom: 1.0, left: 8.0 })
        .style(move |_theme| container::Style {
            background: bg.map(Into::into),
            ..container::Style::default()
        })
        .into()
}

/// A blank placeholder cell — the opposite column when a row's line only
/// exists on one side (a pure addition has nothing to show on the left, a
/// pure removal nothing on the right). A non-empty space (not truly empty
/// text) so the row still reports the same line height as a populated cell.
fn side_blank_cell(p: Palette) -> Element<'static, Message> {
    container(
        text("\u{a0}")
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_muted)),
    )
    .width(Length::Fill)
    .padding(Padding { top: 1.0, right: 12.0, bottom: 1.0, left: 8.0 + GUTTER_WIDTH + 4.0 })
    .into()
}

fn side_divider(p: Palette) -> Element<'static, Message> {
    container(Space::new().width(Length::Fixed(1.0)).height(Length::Fill))
        .style(move |_theme| container::Style {
            background: Some(color(p.border_hairline).into()),
            ..container::Style::default()
        })
        .into()
}

fn side_row(
    left: Option<(&DiffLine, Option<&[(bool, String)]>)>,
    right: Option<(&DiffLine, Option<&[(bool, String)]>)>,
    p: Palette,
) -> Element<'static, Message> {
    let left_cell = match left {
        Some((line, spans)) => side_cell(line, spans, true, p),
        None => side_blank_cell(p),
    };
    let right_cell = match right {
        Some((line, spans)) => side_cell(line, spans, false, p),
        None => side_blank_cell(p),
    };
    row![left_cell, side_divider(p), right_cell].into()
}

/// The collapsed-gap row standing in for `hidden` skipped unchanged lines,
/// side-by-side layout — spans both columns rather than sitting in just one
/// (unlike `context_separator`, unified layout's version), since it isn't
/// attached to either side in particular.
fn side_separator(hidden: usize, p: Palette) -> Element<'static, Message> {
    container(
        text(format!(
            "\u{22ef} {hidden} unchanged line{} \u{22ef}",
            if hidden == 1 { "" } else { "s" }
        ))
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(12.0))
        .color(color(p.text_muted)),
    )
    .width(Length::Fill)
    .align_x(Alignment::Center)
    .padding(Padding { top: 4.0, right: 12.0, bottom: 4.0, left: 12.0 })
    .into()
}

/// One hunk's rows, side-by-side layout — old lines paired with new lines
/// row-for-row (see `HunkShape`), each pair's word-level diff highlighted
/// the same way `hunk_block` does. Same click-anywhere-to-toggle-selection
/// button as `hunk_block`.
fn side_hunk_block(lines: &[DiffLine], hunk_id: usize, selected: bool, path: PathBuf, p: Palette) -> Element<'static, Message> {
    let shape = hunk_shape(lines);
    let max_rows = shape.n_delete.max(shape.n_insert);

    let rows: Vec<Element<'static, Message>> = (0..max_rows)
        .map(|i| {
            let left = (i < shape.n_delete).then(|| &lines[i]);
            let right = (i < shape.n_insert).then(|| &lines[shape.n_delete + i]);
            let (left_spans, right_spans) = if i < shape.paired {
                let words = diff_words(&lines[i].text, &lines[shape.n_delete + i].text);
                (
                    Some(word_spans_for_side(&words, DiffLineKind::Delete)),
                    Some(word_spans_for_side(&words, DiffLineKind::Insert)),
                )
            } else {
                (None, None)
            };
            side_row(
                left.map(|l| (l, left_spans.as_deref())),
                right.map(|l| (l, right_spans.as_deref())),
                p,
            )
        })
        .collect();

    button(column(rows))
        .width(Length::Fill)
        .padding(0.0)
        .on_press(Message::ToggleDiffHunkSelected { path, hunk_id })
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            let background = if selected {
                Some(tint(p.accent_solid, 0.10).into())
            } else if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            };
            button::Style { background, ..button::Style::default() }
        })
        .into()
}

/// The shared shape both `unified_rows` and `side_by_side_rows` walk `lines`
/// into — everything about *which* lines get grouped into a hunk vs.
/// trimmed context vs. collapsed behind a separator lives here, once, so
/// the two layouts can't drift apart on that (only on how each segment
/// renders).
enum RowPlan<'a> {
    Context(&'a DiffLine),
    Separator(usize),
    Hunk { lines: &'a [DiffLine], hunk_id: usize, selected: bool },
}

fn plan_rows<'a>(lines: &'a [DiffLine], hunks: &'a [Hunk], selected_hunks: &HashSet<usize>) -> Vec<RowPlan<'a>> {
    let mut plan = Vec::new();
    let mut hunk_iter = hunks.iter().peekable();
    let mut i = 0;
    let mut seen_hunk = false;
    while i < lines.len() {
        if hunk_iter.peek().is_some_and(|h| h.range.start == i) {
            let hunk = hunk_iter.next().unwrap();
            let selected = selected_hunks.contains(&hunk.range.start);
            plan.push(RowPlan::Hunk { lines: &lines[hunk.range.clone()], hunk_id: hunk.range.start, selected });
            i = hunk.range.end;
            seen_hunk = true;
            continue;
        }

        // A run of `Equal` lines between hunks (or leading/trailing the
        // whole diff) — show up to `CONTEXT_LINES` adjacent to each
        // neighboring hunk and collapse the rest, rather than rendering
        // every unchanged line in the file.
        let gap_end = hunk_iter.peek().map(|h| h.range.start).unwrap_or(lines.len());
        let gap_len = gap_end - i;
        let lead = if seen_hunk { CONTEXT_LINES.min(gap_len) } else { 0 };
        let trail = if hunk_iter.peek().is_some() { CONTEXT_LINES.min(gap_len) } else { 0 };

        if lead + trail >= gap_len {
            for line in &lines[i..gap_end] {
                plan.push(RowPlan::Context(line));
            }
        } else {
            for line in &lines[i..i + lead] {
                plan.push(RowPlan::Context(line));
            }
            plan.push(RowPlan::Separator(gap_len - lead - trail));
            for line in &lines[gap_end - trail..gap_end] {
                plan.push(RowPlan::Context(line));
            }
        }
        i = gap_end;
    }
    plan
}

fn unified_rows(plan: &[RowPlan], path: &Path, p: Palette) -> Vec<Element<'static, Message>> {
    plan.iter()
        .map(|item| match item {
            RowPlan::Context(line) => diff_row(line, "", color(p.text_muted), None, p),
            RowPlan::Separator(hidden) => context_separator(*hidden, p),
            RowPlan::Hunk { lines, hunk_id, selected } => hunk_block(lines, *hunk_id, *selected, path.to_path_buf(), p),
        })
        .collect()
}

fn side_by_side_rows(plan: &[RowPlan], path: &Path, p: Palette) -> Vec<Element<'static, Message>> {
    plan.iter()
        .map(|item| match item {
            RowPlan::Context(line) => side_row(Some((line, None)), Some((line, None)), p),
            RowPlan::Separator(hidden) => side_separator(*hidden, p),
            RowPlan::Hunk { lines, hunk_id, selected } => side_hunk_block(lines, *hunk_id, *selected, path.to_path_buf(), p),
        })
        .collect()
}

/// A small outlined toggle button for the toolbar's "Ignore Whitespace" and
/// "Side by Side" switches — filled/accented while `active`, same hover
/// treatment as the toolbar's other buttons otherwise.
fn toggle_pill(label: &'static str, active: bool, on_press: Message, p: Palette) -> Element<'static, Message> {
    button(
        text(label)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(if active { color(p.accent_solid) } else { color(p.text_muted) }),
    )
    .padding([4.0, 8.0])
    .on_press(on_press)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if active {
                Some(tint(p.accent_solid, 0.12).into())
            } else if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            border: Border {
                color: if active { color(p.accent_solid) } else { color(p.border_hairline) },
                width: 1.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// The toolbar above the diff — the "Revert selected"/view-mode/whitespace
/// controls when nothing's pending, or the confirm/cancel step once
/// "Revert Selected" has been clicked once (`EditorState::pending_hunk_revert`)
/// — same two-step shape as the sidebar's Changes panel rollback.
fn toolbar(path: &Path, selected_count: usize, pending: bool, ignore_whitespace: bool, view_mode: DiffViewMode, p: Palette) -> Element<'static, Message> {
    if pending {
        return row![
            text(format!(
                "Revert {selected_count} selected change{}?",
                if selected_count == 1 { "" } else { "s" }
            ))
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_strong))
            .width(Length::Fill),
            button(
                text("Cancel")
                    .font(fonts::sans(Weight::Medium))
                    .size(crate::text_scale::px(13.0))
                    .color(color(p.text_muted)),
            )
            .padding([4.0, 8.0])
            .on_press(Message::CancelRevertSelectedHunks(path.to_path_buf()))
            .style(move |_theme, status| {
                let hovered = status == button::Status::Hovered;
                button::Style {
                    background: if hovered { Some(color(p.surface_hover).into()) } else { None },
                    ..button::Style::default()
                }
            }),
            button(
                text("Revert")
                    .font(fonts::sans(Weight::Medium))
                    .size(crate::text_scale::px(13.0))
                    .color(color(p.status_danger)),
            )
            .padding([4.0, 8.0])
            .on_press(Message::ConfirmRevertSelectedHunks(path.to_path_buf()))
            .style(move |_theme, status| {
                let hovered = status == button::Status::Hovered;
                button::Style {
                    background: if hovered { Some(tint(p.status_danger, 0.16).into()) } else { None },
                    ..button::Style::default()
                }
            }),
        ]
        .spacing(4.0)
        .padding(Padding { top: 6.0, right: 12.0, bottom: 6.0, left: 12.0 })
        .align_y(Alignment::Center)
        .into();
    }

    let label = if selected_count == 0 {
        "No changes selected".to_string()
    } else {
        format!("{selected_count} change{} selected", if selected_count == 1 { "" } else { "s" })
    };

    let revert_button = button(
        text("Revert Selected")
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(if selected_count == 0 { tint(p.text_muted, 0.5) } else { color(p.status_danger) }),
    )
    .padding([4.0, 8.0])
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered && selected_count > 0;
        button::Style {
            background: if hovered { Some(tint(p.status_danger, 0.16).into()) } else { None },
            ..button::Style::default()
        }
    });
    let revert_button = if selected_count > 0 {
        revert_button.on_press(Message::PromptRevertSelectedHunks(path.to_path_buf()))
    } else {
        revert_button
    };

    row![
        text(label)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted))
            .width(Length::Fill),
        toggle_pill("Ignore Whitespace", ignore_whitespace, Message::ToggleDiffIgnoreWhitespace, p),
        toggle_pill(
            if view_mode == DiffViewMode::SideBySide { "Side by Side" } else { "Unified" },
            view_mode == DiffViewMode::SideBySide,
            Message::ToggleDiffViewMode,
            p,
        ),
        revert_button,
    ]
    .spacing(8.0)
    .padding(Padding { top: 6.0, right: 12.0, bottom: 6.0, left: 12.0 })
    .align_y(Alignment::Center)
    .into()
}

/// Renders the diff for `path`, whose `HEAD`-vs-buffer `DiffStatus` is
/// cached on its backing `File` tab — a `Diff` tab is only ever opened
/// alongside one (see `state::open_or_focus_diff`).
pub fn view(state: &State, path: &Path, p: Palette) -> Element<'static, Message> {
    let Some(editor) = state::find_editor(state, path) else {
        return widgets::placeholder("No file open \u{2014} pick one from the sidebar", p.editor_canvas, p);
    };

    match &editor.diff {
        DiffStatus::NoRepo => widgets::placeholder("Not inside a git repository", p.editor_canvas, p),
        DiffStatus::Untracked => {
            widgets::placeholder("New file \u{2014} no HEAD version to diff against", p.editor_canvas, p)
        }
        DiffStatus::UpToDate => widgets::placeholder("No changes since HEAD", p.editor_canvas, p),
        DiffStatus::Changed(lines) => {
            let plan = plan_rows(lines, &editor.hunks, &editor.diff_selected_hunks);
            let rows = match state.diff_view_mode {
                DiffViewMode::Unified => unified_rows(&plan, path, p),
                DiffViewMode::SideBySide => side_by_side_rows(&plan, path, p),
            };

            let selected_count = editor.diff_selected_hunks.len();

            container(column![
                toolbar(path, selected_count, editor.pending_hunk_revert, state.diff_ignore_whitespace, state.diff_view_mode, p),
                widgets::hline(color(p.border_hairline)),
                scrollable(column(rows).padding([8.0, 0.0]))
                    .width(Length::Fill)
                    .height(Length::Fill),
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(color(p.editor_canvas).into()),
                ..container::Style::default()
            })
            .into()
        }
    }
}
