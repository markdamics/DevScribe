//! The floating in-file "find" pill (Ctrl+F) — a small overlay pinned to the
//! top-right of the editor pane, layered over `editor_canvas` via a local
//! `stack!` in `shell.rs`. Distinct from `search_view`, which searches every
//! file rather than the current buffer.
use iced::font::Weight;
use iced::widget::{button, canvas, container, row, text, text_input, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{self, EditorState, Message};
use crate::ui::search_icon::SearchIcon;
use crate::widgets;

fn nav_button(
    label: &'static str,
    on_press: Message,
    enabled: bool,
    p: devscribe_core::theme::Palette,
) -> Element<'static, Message> {
    let mut b = button(widgets::center_fill(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0)),
    ))
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(22.0))
    .padding(0.0)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            text_color: if enabled {
                color(p.text_strong)
            } else {
                color(p.text_muted)
            },
            ..button::Style::default()
        }
    });
    if enabled {
        b = b.on_press(on_press);
    }
    b.into()
}

/// `editor` must have `editor.find.is_some()` — callers check this before
/// including the bar as a stack layer.
pub fn view(editor: &EditorState, p: devscribe_core::theme::Palette) -> Element<'static, Message> {
    let find = editor.find.as_ref().expect("caller checks find.is_some()");

    let count_label = if find.query.is_empty() {
        String::new()
    } else if find.matches.is_empty() {
        "No results".to_string()
    } else {
        format!("{} of {}", find.current + 1, find.matches.len())
    };

    let has_matches = !find.matches.is_empty();

    let input = text_input("Find\u{2026}", &find.query)
        .id(state::find_query_id())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .padding([4.0, 6.0])
        .on_input(Message::FindQueryChanged)
        .on_submit(Message::FindNext)
        .style(move |_theme, _status| text_input::Style {
            background: Color::TRANSPARENT.into(),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_strong),
            selection: {
                let mut c = p.accent_solid;
                c.a = 0.35;
                color(c)
            },
        });

    let close = button(widgets::center_fill(text("\u{2715}").size(crate::text_scale::px(13.0))))
        .width(Length::Fixed(22.0))
        .height(Length::Fixed(22.0))
        .padding(0.0)
        .on_press(Message::CloseFind)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                text_color: color(p.text_muted),
                ..button::Style::default()
            }
        });

    let pill = row![
        canvas(SearchIcon { color: color(p.text_muted) })
            .width(Length::Fixed(13.0))
            .height(Length::Fixed(13.0)),
        input,
        text(count_label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
        nav_button("\u{2191}", Message::FindPrev, has_matches, p),
        nav_button("\u{2193}", Message::FindNext, has_matches, p),
        close,
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .padding(Padding {
        top: 4.0,
        right: 6.0,
        bottom: 4.0,
        left: 10.0,
    });

    let panel = container(pill)
        .width(Length::Fixed(320.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_accent),
                width: 1.5,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });

    container(row![Space::new().width(Length::Fill), panel])
        .width(Length::Fill)
        .padding(Padding {
            top: 10.0,
            right: 20.0,
            bottom: 0.0,
            left: 0.0,
        })
        .into()
}
