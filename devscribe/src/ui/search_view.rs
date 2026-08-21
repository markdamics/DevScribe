//! Project-wide text search: results grouped by file, each match line
//! syntax-colored with the matched term highlighted, plus a stats line.
//! Naive (re-scans every file in the sidebar tree on submit) — see
//! `devscribe_core::search` for why that's an acceptable starting point.
use devscribe_core::syntax::HighlightKind;
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Border, Color, Element, Length, Padding};
use std::path::Path;

use crate::color::color;
use crate::fonts;
use crate::state::{Message, SearchResult, State};
use crate::ui::editor_canvas;
use crate::widgets;

fn query_box(p: Palette, query: &str) -> Element<'static, Message> {
    text_input("Search project files\u{2026}", query)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(13.0))
        .padding([8.0, 12.0])
        .on_input(Message::SearchQueryChanged)
        .on_submit(Message::SearchSubmit)
        .style(move |_theme, _status| text_input::Style {
            background: color(p.bg_void).into(),
            border: Border {
                color: color(p.line_neutral),
                width: 1.0,
                radius: 2.0.into(),
            },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_primary),
            selection: {
                let mut c = p.accent;
                c.a = 0.35;
                color(c)
            },
        })
        .into()
}

fn stats_row(state: &State, p: Palette) -> Element<'static, Message> {
    let files = state
        .search_results
        .iter()
        .map(|r| r.path.as_path())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let matches = state.search_results.len();
    let ms = state.search_elapsed.as_millis();

    row![
        text(format!("Results for \u{201c}{}\u{201d}", state.search_query))
            .font(fonts::sans(Weight::Semibold))
            .size(crate::text_scale::px(14.0))
            .color(color(p.text_primary)),
        text(format!("{matches} MATCHES // {files} FILES // {ms} MS"))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted)),
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .into()
}

/// Splits `segments` (or, if empty, `preview` as one flat run) at the
/// `[match_start, match_start + match_len)` char range, resolving each
/// piece's syntax color and wrapping the matched piece in an
/// accent-tinted background — reuses `editor_canvas`'s highlight→color
/// mapping so search previews and the editor agree on token colors.
fn match_line_runs(
    segments: &[(String, HighlightKind)],
    preview: &str,
    match_start: usize,
    match_len: usize,
    p: Palette,
) -> Vec<Element<'static, Message>> {
    let owned;
    let base: &[(String, HighlightKind)] = if segments.is_empty() {
        owned = vec![(preview.to_string(), HighlightKind::Default)];
        &owned
    } else {
        segments
    };

    let match_end = match_start + match_len;
    let mut out = Vec::new();
    let mut cursor = 0usize;

    for (seg_text, kind) in base {
        let seg_len = seg_text.chars().count();
        let seg_start = cursor;
        let seg_end = cursor + seg_len;
        cursor = seg_end;

        let run_color = editor_canvas::highlight_color(*kind, p);

        if match_len == 0 || match_end <= seg_start || match_start >= seg_end {
            out.push(plain_run(seg_text.clone(), run_color));
            continue;
        }

        let chars: Vec<char> = seg_text.chars().collect();
        let local_start = match_start.saturating_sub(seg_start).min(seg_len);
        let local_end = match_end.saturating_sub(seg_start).min(seg_len);

        if local_start > 0 {
            out.push(plain_run(chars[..local_start].iter().collect(), run_color));
        }
        if local_end > local_start {
            let matched: String = chars[local_start..local_end].iter().collect();
            out.push(matched_run(matched, run_color, p));
        }
        if local_end < seg_len {
            out.push(plain_run(chars[local_end..].iter().collect(), run_color));
        }
    }

    out
}

fn plain_run(content: String, color_val: Color) -> Element<'static, Message> {
    text(content)
        .font(fonts::mono(Weight::Normal))
        .size(crate::text_scale::px(11.5))
        .color(color_val)
        .into()
}

fn matched_run(content: String, color_val: Color, p: Palette) -> Element<'static, Message> {
    container(
        text(content)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(11.5))
            .color(color_val),
    )
    .style(move |_theme| container::Style {
        background: Some(
            color({
                let mut c = p.accent;
                c.a = 0.28;
                c
            })
            .into(),
        ),
        ..container::Style::default()
    })
    .into()
}

fn match_row(result: &SearchResult, p: Palette) -> Element<'static, Message> {
    let gutter = text((result.hit.line + 1).to_string())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(11.0))
        .color({
            let mut c = p.text_muted;
            c.a = 0.6;
            color(c)
        })
        .width(Length::Fixed(28.0));

    let runs = match_line_runs(
        &result.segments,
        &result.hit.preview,
        result.hit.col,
        result.query_len_chars,
        p,
    );

    let content = row![gutter, row(runs)].spacing(12.0).align_y(Alignment::Center);

    let path = result.path.clone();
    let line = result.hit.line;
    let col = result.hit.col;

    button(content)
        .width(Length::Fill)
        .padding(Padding {
            top: 2.0,
            right: 12.0,
            bottom: 2.0,
            left: 24.0,
        })
        .on_press(Message::SearchResultSelected { path, line, col })
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                ..button::Style::default()
            }
        })
        .into()
}

fn file_group<'a>(path: &'a Path, results: &[&'a SearchResult], p: Palette) -> Element<'static, Message> {
    let lang = crate::fs_tree::Lang::from_path(path);
    let (fg, bg) = lang.badge(p);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let header = row![
        widgets::lang_badge(lang.code(path), fg, bg),
        text(name)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_primary)),
        text(results.len().to_string())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);

    let lines: Vec<Element<'static, Message>> = results.iter().map(|r| match_row(r, p)).collect();

    column![header, column(lines)].spacing(4.0).into()
}

/// Groups `results` by file, relying on `recompute_search` already visiting
/// files one at a time (so each file's hits are already contiguous) rather
/// than sorting again here.
fn group_by_file(results: &[SearchResult]) -> Vec<(&Path, Vec<&SearchResult>)> {
    let mut groups: Vec<(&Path, Vec<&SearchResult>)> = Vec::new();
    for result in results {
        match groups.last_mut() {
            Some((path, items)) if *path == result.path.as_path() => items.push(result),
            _ => groups.push((result.path.as_path(), vec![result])),
        }
    }
    groups
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let input = container(query_box(p, &state.search_query)).padding(12.0);

    let body: Element<'static, Message> = if state.search_query.is_empty() {
        widgets::placeholder("Type a query and press Enter", p)
    } else if state.search_results.is_empty() {
        widgets::placeholder(format!("No matches for \u{201c}{}\u{201d}", state.search_query), p)
    } else {
        let mut sections: Vec<Element<'static, Message>> = vec![stats_row(state, p)];
        for (path, results) in group_by_file(&state.search_results) {
            sections.push(file_group(path, &results, p));
        }
        scrollable(
            column(sections).spacing(20.0).padding(Padding {
                top: 12.0,
                right: 24.0,
                bottom: 24.0,
                left: 24.0,
            }),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    };

    container(column![input, widgets::hline(color(p.line_neutral)), body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_void).into()),
            ..container::Style::default()
        })
        .into()
}
