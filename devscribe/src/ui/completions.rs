//! Inline completion popup: shows up to 50 LSP completion items anchored near
//! the cursor, navigable with arrow keys, and committed with Tab/Enter.
//! Rendered as a `stack!` layer in `shell.rs`, same pattern as
//! `command_palette` and `context_menu`.
use devscribe_core::lsp::CompletionItem;
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Space};
use iced::{Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State};
use crate::ui::editor_canvas;

/// Height of the title bar + tab bar combined — used to offset the popup
/// from the top of the window to the top of the editor content area.
/// Approximate; exact values depend on density/font-scale settings.
const HEADER_HEIGHT: f32 = 78.0;

fn item_row(
    index: usize,
    item: &CompletionItem,
    selected: usize,
    p: Palette,
) -> Element<'static, Message> {
    let is_selected = index == selected;

    let label = text(item.label.clone())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(13.0))
        .color(color(if is_selected { p.text_strong } else { p.text_body }));

    let detail = item.detail.clone().map(|d| {
        text(d)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted))
    });

    let inner: Element<'static, Message> = if let Some(det) = detail {
        row![label, Space::new().width(Length::Fill), det]
            .align_y(iced::Alignment::Center)
            .into()
    } else {
        label.into()
    };

    button(inner)
        .width(Length::Fill)
        .padding(Padding {
            top: 5.0,
            right: 10.0,
            bottom: 5.0,
            left: 10.0,
        })
        .on_press(Message::CompletionSelect)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if is_selected || hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                text_color: color(p.text_strong),
                ..button::Style::default()
            }
        })
        .into()
}

pub fn view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let path = crate::state::active_file_path(state)?;
    let editor = crate::state::find_editor(state, &path)?;
    let items = editor.completions.as_ref()?;
    if items.is_empty() {
        return None;
    }

    let selected = editor.completion_selected.min(items.len().saturating_sub(1));
    let font_size = state.editor_font_size;

    let (x, y) = editor_canvas::cursor_pixel_pos(
        editor.completion_anchor.line,
        editor.completion_anchor.col,
        font_size,
        editor.scroll_offset,
        editor.scroll_offset_x,
        HEADER_HEIGHT,
    );

    let rows: Vec<Element<'static, Message>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| item_row(i, item, selected, p))
        .collect();

    let list = scrollable(column(rows))
        .height(Length::Fixed(220.0));

    let popup = container(list)
        .width(Length::Fixed(280.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        });

    // Wrap in a mouse_area so clicks inside the popup don't propagate to the
    // editor canvas and accidentally dismiss the popup via EditorClick.
    let shielded = mouse_area(popup).on_press(Message::Noop);

    // Outer container fills the window; left/top padding positions the popup.
    let positioned = container(shielded)
        .padding(Padding {
            top: y.max(0.0),
            left: x.max(0.0),
            right: 0.0,
            bottom: 0.0,
        })
        .width(Length::Fill)
        .height(Length::Fill);

    Some(positioned.into())
}
