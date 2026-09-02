//! A read-only tooltip showing `textDocument/hover` documentation for
//! whatever the mouse is resting on — the passive counterpart to
//! `completions.rs`'s popup (interactive, keyboard-driven), positioned the
//! same cursor-anchored way. Rendered as a `stack!` layer in `shell.rs`.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{column, container, scrollable, text};
use iced::{Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State};
use crate::ui::editor_canvas;

/// Same value `completions.rs` uses — both anchor off the same header.
const HEADER_HEIGHT: f32 = 78.0;
const MAX_WIDTH: f32 = 440.0;
const MAX_HEIGHT: f32 = 240.0;

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
    let (pos, hover_text) = editor.hover.clone()?;
    let font_size = state.editor_font_size;

    let (x, y) = editor_canvas::cursor_pixel_pos(
        pos.line,
        pos.col,
        font_size,
        editor.scroll_offset,
        editor.scroll_offset_x,
        HEADER_HEIGHT,
    );

    let body = scrollable(
        column![text(hover_text)
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(12.5))
            .color(color(p.text_body))]
        .padding(10.0),
    );

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
