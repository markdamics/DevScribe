//! A rendered preview for the currently open `.md`/`.markdown` file, using
//! iced's own `widget::markdown` renderer (headings, lists, code blocks,
//! links, ...) restyled from this app's `Palette` instead of iced's built-in
//! `Theme` — every other widget in the app already sources its colors the
//! same way. Distinct from `json_view.rs`'s tree in shape, but the same
//! idea: a read-only rendering of the buffer that defaults over the plain
//! `code_area`, with an "Edit as Text" escape hatch back to it.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, markdown, row, scrollable, text, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{EditorState, Message};
use crate::widgets;

/// Mirrors `json_view.rs::edit_button` — same affordance, its own message
/// variant since Markdown's text-mode flag is independent of JSON's.
fn edit_button(p: Palette) -> Element<'static, Message> {
    button(
        text("Edit as Text")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted)),
    )
    .padding([4.0, 10.0])
    .on_press(Message::MarkdownToggleTextMode)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_raised).into())
            } else {
                None
            },
            border: Border {
                color: color(p.border_hairline),
                width: 1.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// Colors/fonts for iced's markdown renderer, sourced from this app's own
/// `Palette` rather than `iced::Theme` — built by hand (not the crate's
/// `Style::from_palette`, which converts iced's own unrelated `theme::Palette`
/// type of the same name) so it matches the rest of the app's styling.
fn style(p: Palette) -> markdown::Style {
    markdown::Style {
        font: fonts::sans(Weight::Normal),
        inline_code_highlight: markdown::Highlight {
            background: color(p.surface_hover).into(),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
        },
        inline_code_padding: Padding { top: 0.0, right: 4.0, bottom: 0.0, left: 4.0 },
        inline_code_color: color(p.text_strong),
        inline_code_font: fonts::mono(Weight::Medium),
        code_block_font: fonts::mono(Weight::Medium),
        link_color: color(p.accent_solid),
    }
}

/// Renders `editor`'s Markdown preview. Only called once the active tab is
/// known to be a `.md`/`.markdown` file with `markdown_text_mode` off — see
/// `shell.rs::content_area`. Tied to `editor`'s own lifetime (unlike
/// `json_view::view`'s `'static`): iced's `markdown::view` ties its output
/// to the borrow of the `Item`s it's given, even though every span it
/// builds is already an owned `String` underneath.
pub fn view(editor: &EditorState, p: Palette) -> Element<'_, Message> {
    let Some(content) = editor.markdown.as_ref() else {
        return widgets::placeholder(format!("{} isn't a Markdown file", editor.path.display()), p);
    };

    let badge = row![
        text("PREVIEW")
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
        Space::new().width(Length::Fill),
        edit_button(p),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .padding([8.0, 12.0]);

    let settings = markdown::Settings::with_text_size(crate::text_scale::px(15.0), style(p));
    let rendered = markdown::view(content.items(), settings).map(Message::OpenMarkdownLink);

    let body = scrollable(container(rendered).width(Length::Fill).padding([4.0, 16.0]))
        .width(Length::Fill)
        .height(Length::Fill);

    container(column![badge, body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        })
        .into()
}
