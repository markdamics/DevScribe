//! A unified (not side-by-side) diff of the current file against its `HEAD`
//! version. Mostly read-only — no Stage; that mutates git state and deserves
//! its own careful pass with explicit confirmation, not a bolt-on here. The
//! one exception is "revert selected changes": the user checks one or more
//! hunks and reverts just those, via the same undo-able document edits the
//! editor gutter's per-line revert already uses (`EditorState::revert_lines`).
use devscribe_core::diff::{DiffLine, DiffLineKind, Hunk};
use devscribe_core::theme::{Palette, Rgba};
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Color, Element, Length, Padding};

use std::path::{Path, PathBuf};

use crate::color::color;
use crate::fonts;
use crate::state::{self, DiffStatus, Message, State};
use crate::widgets;

const GUTTER_WIDTH: f32 = 44.0;
const MARKER_WIDTH: f32 = 20.0;

fn tint(c: Rgba, alpha: f32) -> Color {
    color(Rgba { a: alpha, ..c })
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

/// One diff row. `marker` is the hunk checkbox glyph on a hunk's first line,
/// blank (but still `MARKER_WIDTH` wide, so every row's columns line up)
/// everywhere else.
fn diff_row(line: &DiffLine, marker: &'static str, marker_color: Color, p: Palette) -> Element<'static, Message> {
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
        text(line.text.clone())
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body)),
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

fn row_tint(mut c: Rgba) -> Color {
    c.a = 0.14;
    color(c)
}

/// How many unchanged lines to show immediately around a hunk, on each side
/// — mirrors `git diff`'s default context. Gaps between hunks (or before the
/// first/after the last) longer than this collapse behind `context_separator`
/// instead of rendering the whole file.
const CONTEXT_LINES: usize = 3;

/// The collapsed-gap row standing in for `hidden` skipped unchanged lines.
/// Indented to roughly the text column so it reads as "elided" rather than
/// as a real diff row.
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

/// One hunk's rows, wrapped in a single button so clicking anywhere in it
/// toggles the whole hunk's selection — the checkbox glyph on its first row
/// is a visual cue, not the only click target (small glyphs make poor click
/// targets on their own).
fn hunk_block(lines: &[DiffLine], hunk: &Hunk, selected: bool, path: PathBuf, p: Palette) -> Element<'static, Message> {
    let marker_color = if selected { color(p.accent_solid) } else { tint(p.text_muted, 0.55) };
    let checkbox = if selected { "\u{2611}" } else { "\u{2610}" };

    let rows: Vec<Element<'static, Message>> = lines[hunk.range.clone()]
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let marker = if i == 0 { checkbox } else { "" };
            diff_row(line, marker, marker_color, p)
        })
        .collect();

    let hunk_id = hunk.range.start;
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

/// The "Revert selected" toolbar above the diff — the button when nothing's
/// pending, or the confirm/cancel step once it's been clicked once
/// (`EditorState::pending_hunk_revert`) — same two-step shape as the
/// sidebar's Changes panel rollback.
fn toolbar(path: &Path, selected_count: usize, pending: bool, p: Palette) -> Element<'static, Message> {
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
        revert_button,
    ]
    .spacing(4.0)
    .padding(Padding { top: 6.0, right: 12.0, bottom: 6.0, left: 12.0 })
    .align_y(Alignment::Center)
    .into()
}

/// Renders the diff for `path`, whose `HEAD`-vs-buffer `DiffStatus` is
/// cached on its backing `File` tab — a `Diff` tab is only ever opened
/// alongside one (see `state::open_or_focus_diff`).
pub fn view(state: &State, path: &Path, p: Palette) -> Element<'static, Message> {
    let Some(editor) = state::find_editor(state, path) else {
        return widgets::placeholder("No file open \u{2014} pick one from the sidebar", p);
    };

    match &editor.diff {
        DiffStatus::NoRepo => widgets::placeholder("Not inside a git repository", p),
        DiffStatus::Untracked => {
            widgets::placeholder("New file \u{2014} no HEAD version to diff against", p)
        }
        DiffStatus::UpToDate => widgets::placeholder("No changes since HEAD", p),
        DiffStatus::Changed(lines) => {
            let path_buf = path.to_path_buf();
            let mut rows: Vec<Element<'static, Message>> = Vec::new();
            let mut hunk_iter = editor.hunks.iter().peekable();
            let mut i = 0;
            let mut seen_hunk = false;
            while i < lines.len() {
                if hunk_iter.peek().is_some_and(|h| h.range.start == i) {
                    let hunk = hunk_iter.next().unwrap();
                    let selected = editor.diff_selected_hunks.contains(&hunk.range.start);
                    rows.push(hunk_block(lines, hunk, selected, path_buf.clone(), p));
                    i = hunk.range.end;
                    seen_hunk = true;
                    continue;
                }

                // A run of `Equal` lines between hunks (or leading/trailing
                // the whole diff) — show up to `CONTEXT_LINES` adjacent to
                // each neighboring hunk and collapse the rest, rather than
                // rendering every unchanged line in the file.
                let gap_end = hunk_iter.peek().map(|h| h.range.start).unwrap_or(lines.len());
                let gap_len = gap_end - i;
                let lead = if seen_hunk { CONTEXT_LINES.min(gap_len) } else { 0 };
                let trail = if hunk_iter.peek().is_some() { CONTEXT_LINES.min(gap_len) } else { 0 };

                if lead + trail >= gap_len {
                    for line in &lines[i..gap_end] {
                        rows.push(diff_row(line, "", color(p.text_muted), p));
                    }
                } else {
                    for line in &lines[i..i + lead] {
                        rows.push(diff_row(line, "", color(p.text_muted), p));
                    }
                    rows.push(context_separator(gap_len - lead - trail, p));
                    for line in &lines[gap_end - trail..gap_end] {
                        rows.push(diff_row(line, "", color(p.text_muted), p));
                    }
                }
                i = gap_end;
            }

            let selected_count = editor.diff_selected_hunks.len();

            container(column![
                toolbar(path, selected_count, editor.pending_hunk_revert, p),
                widgets::hline(color(p.border_hairline)),
                scrollable(column(rows).padding([8.0, 0.0]))
                    .width(Length::Fill)
                    .height(Length::Fill),
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(color(p.bg_canvas).into()),
                ..container::Style::default()
            })
            .into()
        }
    }
}
