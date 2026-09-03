//! Project-wide text search: results grouped by file, each match line
//! shown plain with the matched term itself highlighted, plus a stats
//! line. Naive (re-scans every file in the sidebar tree) — see
//! `devscribe_core::search` for why that's an acceptable starting point.
//! Debounced and backgrounded, VSCode-style: typing schedules a search a
//! short pause later (`state::SEARCH_DEBOUNCE_DELAY`) rather than running
//! one per keystroke or waiting on Enter, and it runs on its own thread
//! (`state::start_search`) so it can never freeze this UI, no matter how
//! slow the underlying file reads turn out to be.
//!
//! Deliberately *not* syntax-colored (an earlier version ran the full
//! syntax highlighter once per matching file) — that was real, uncapped
//! CPU cost that scaled with how many files a broad query touched, capable
//! of making a single search hang the UI thread on a large project. See
//! `docs/differences-and-roadmap.md`'s search bug-fix writeup.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Border, Color, Element, Length, Padding};
use std::path::Path;

use crate::color::color;
use crate::fonts;
use crate::state::{Message, SearchResult, State};
use crate::widgets;

fn query_box(p: Palette, query: &str) -> Element<'static, Message> {
    text_input("Search project files\u{2026}", query)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .padding([8.0, 12.0])
        .on_input(Message::SearchQueryChanged)
        .on_submit(Message::SearchSubmit)
        .style(move |_theme, status| text_input::Style {
            background: color(p.bg_canvas).into(),
            // Visible keyboard-focus indicator (accessibility pass, item 12).
            border: if matches!(status, text_input::Status::Focused { .. }) {
                Border { color: color(p.border_focus), width: 1.5, radius: 3.0.into() }
            } else {
                Border { color: color(p.border_hairline), width: 1.5, radius: 3.0.into() }
            },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_strong),
            selection: {
                let mut c = p.accent_solid;
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
        text(format!("Results for \u{201c}{}\u{201d}", state.search_last_query))
            .font(fonts::sans(Weight::Semibold))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_strong)),
        text(format!("{matches} MATCHES // {files} FILES // {ms} MS"))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .into()
}

/// Splits `preview` into up to three plain runs at the
/// `[match_start, match_start + match_len)` char range, wrapping the
/// matched piece in an accent-tinted background. `match_start`/`match_len`
/// are char offsets (see `SearchHit::col`'s doc), so this indexes
/// char-wise via a collected `Vec<char>`, not by byte.
fn match_line_runs(preview: &str, match_start: usize, match_len: usize, p: Palette) -> Vec<Element<'static, Message>> {
    let chars: Vec<char> = preview.chars().collect();
    let len = chars.len();
    let run_color = color(p.text_body);

    if match_len == 0 {
        return vec![plain_run(preview.to_string(), run_color)];
    }

    let local_start = match_start.min(len);
    let local_end = (match_start + match_len).min(len);

    let mut out = Vec::new();
    if local_start > 0 {
        out.push(plain_run(chars[..local_start].iter().collect(), run_color));
    }
    if local_end > local_start {
        let matched: String = chars[local_start..local_end].iter().collect();
        out.push(matched_run(matched, color(p.text_strong), p));
    }
    if local_end < len {
        out.push(plain_run(chars[local_end..].iter().collect(), run_color));
    }
    out
}

fn plain_run(content: String, color_val: Color) -> Element<'static, Message> {
    text(content)
        .font(fonts::mono(Weight::Normal))
        .size(crate::text_scale::px(13.5))
        .color(color_val)
        .into()
}

fn matched_run(content: String, color_val: Color, p: Palette) -> Element<'static, Message> {
    container(
        text(content)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.5))
            .color(color_val),
    )
    .style(move |_theme| container::Style {
        background: Some(
            color({
                let mut c = p.accent_solid;
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
        .size(crate::text_scale::px(13.0))
        .color({
            let mut c = p.text_muted;
            c.a = 0.6;
            color(c)
        })
        .width(Length::Fixed(28.0));

    // `preview_col`, not `col` — `col` is the match's position in the real
    // (possibly enormous) line, for cursor placement below; `preview` is a
    // windowed, possibly-truncated snippet of it, and only `preview_col`
    // indexes correctly into that. See `SearchHit`'s doc.
    let runs = match_line_runs(&result.hit.preview, result.hit.preview_col, result.query_len_chars, p);

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
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_strong)),
        text(results.len().to_string())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
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

    // Search is debounced and backgrounded (see this module's doc), so
    // `search_results`/`search_last_query` can lag behind whatever's
    // currently typed — these extra arms cover that gap. Without them,
    // editing past a query that had results would either show those stale
    // results under the *new* query's label, or (once cleared) claim "No
    // matches" for a query that was never actually searched — both
    // actively misleading, not just stale.
    let body: Element<'static, Message> = if state.search_query.is_empty() {
        widgets::placeholder("Type to search project files", p)
    } else if state.search_in_progress {
        widgets::placeholder(format!("Searching for \u{201c}{}\u{201d}\u{2026}", state.search_query), p)
    } else if state.search_query != state.search_last_query {
        // Typed, but the debounce window hasn't fired yet — Enter jumps
        // straight there instead of waiting it out.
        widgets::placeholder(format!("Press Enter to search for \u{201c}{}\u{201d} now", state.search_query), p)
    } else if state.search_results.is_empty() {
        widgets::placeholder(format!("No matches for \u{201c}{}\u{201d}", state.search_query), p)
    } else {
        let mut sections: Vec<Element<'static, Message>> = vec![stats_row(state, p)];
        for (path, results) in group_by_file(&state.search_results) {
            sections.push(file_group(path, &results, p));
        }
        scrollable(
            column(sections).spacing(24.0).padding(Padding {
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

    container(column![input, widgets::hline(color(p.border_hairline)), body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        })
        .into()
}
