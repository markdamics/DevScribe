//! A read-only tooltip showing `textDocument/hover` documentation for
//! whatever the mouse is resting on — the passive counterpart to
//! `completions.rs`'s popup (interactive, keyboard-driven), positioned the
//! same cursor-anchored way. Rendered as a `stack!` layer in `shell.rs`.
//!
//! Also renders the "quick peek" (roadmap item 12) underneath the hover
//! text when hovering the same position resolved to a definition/reference
//! elsewhere — a handful of lines of that file (`EditorState::peek`), so it
//! can be checked without leaving the current tab. See
//! `state::editor::build_peek_preview` for how it's resolved and
//! `state::mod`'s `HoverDebounceTick`/`GoToDefinition` handlers for how a
//! peek reply is told apart from an explicit F12 one.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{column, container, row, scrollable, text};
use iced::{Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State};
use crate::ui::editor_canvas;
use crate::widgets;

/// Same value `completions.rs` uses — both anchor off the same header.
const HEADER_HEIGHT: f32 = 78.0;
const MAX_WIDTH: f32 = 440.0;
const MAX_HEIGHT: f32 = 320.0;

pub fn view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let path = crate::state::active_file_path(state)?;
    let editor = crate::state::find_editor(state, &path)?;
    // The completion popup is cursor-anchored the same way — showing both
    // at once over the same spot would just be visual noise stacked on
    // itself, and the completion popup is the one the user is actively
    // interacting with.
    if editor.completions.is_some() {
        return None;
    }
    let hover = editor.hover.clone();
    let peek = editor.peek.clone();
    if hover.is_none() && peek.is_none() {
        return None;
    }
    // Both ride the same hover dwell (see `HoverDebounceTick`), so they're
    // always anchored at the same position whenever both are present —
    // either one, whichever is showing, positions the popup.
    let pos = hover.as_ref().map(|(pos, _)| *pos).or_else(|| peek.as_ref().map(|(pos, _)| *pos))?;
    let font_size = state.editor_font_size;

    let (x, y) = editor_canvas::cursor_pixel_pos_wrapped(
        &editor.document,
        state.word_wrap,
        pos.line,
        pos.col,
        font_size,
        editor.scroll_offset,
        editor.scroll_offset_x,
        HEADER_HEIGHT,
        if editor.viewport_width > 0.0 { editor.viewport_width } else { crate::state::ASSUMED_VIEWPORT_WIDTH },
    );

    let mut sections: Vec<Element<'static, Message>> = Vec::new();
    if let Some((_, hover_text)) = hover {
        sections.push(
            text(hover_text)
                .font(fonts::mono(Weight::Normal))
                .size(crate::text_scale::px(12.5))
                .color(color(p.text_body))
                .into(),
        );
    }
    if let Some((_, preview)) = peek {
        sections.push(peek_view(&preview, p));
    }

    let body = scrollable(column(sections).spacing(10.0).padding(10.0))
        .direction(scrollable::Direction::Vertical(widgets::thin_scrollbar()))
        .style(widgets::scrollbar_style(p));

    let popup = container(body)
        .max_width(MAX_WIDTH)
        .max_height(MAX_HEIGHT)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        });

    // Unlike `completions.rs`'s popup, this one is purely informational —
    // nothing inside it is clickable, so a click landing on it is left to
    // fall through to the editor canvas underneath (repositioning the
    // cursor there, same as a click anywhere else in the pane would).
    let positioned = container(popup)
        .padding(Padding {
            // `cursor_pixel_pos` already lands one line below `pos` (see its
            // own doc) — the same "below, not overlapping" placement
            // `completions.rs` uses this for.
            top: y.max(0.0),
            left: x.max(0.0),
            right: 0.0,
            bottom: 0.0,
        })
        .width(Length::Fill)
        .height(Length::Fill);

    Some(positioned.into())
}

/// Renders a `PeekPreview`'s snippet: a small file:line header, then each
/// context line with its real file line number, the actual target line
/// picked out with a highlighted row so it's clear which one the hover
/// resolved to rather than just where the snippet happens to start.
fn peek_view(preview: &crate::state::PeekPreview, p: Palette) -> Element<'static, Message> {
    let file_name = preview.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    let header = text(format!("{file_name}:{}", preview.target_line + 1))
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(11.0))
        .color(color(p.text_muted));

    let mut lines_col = column![].spacing(1.0);
    for (i, line) in preview.lines.iter().enumerate() {
        let file_line = preview.start_line + i;
        let is_target = file_line == preview.target_line;
        let number = text(format!("{:>4}", file_line + 1))
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(11.0))
            .color(color(p.text_faint));
        let code = text(line.clone())
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(11.5))
            .color(color(if is_target { p.text_strong } else { p.text_muted }));
        let line_row = container(row![number, code].spacing(10.0))
            .width(Length::Fill)
            .padding([1.0, 4.0])
            .style(move |_theme| container::Style {
                background: if is_target { Some(color(p.surface_hover).into()) } else { None },
                ..container::Style::default()
            });
        lines_col = lines_col.push(line_row);
    }

    column![header, lines_col].spacing(4.0).into()
}
