//! The `Ctrl`/`Cmd`+`E` "recent files" overlay: fuzzy-search `State::
//! recent_files` (session file history — closed tabs included, unlike the
//! Ctrl+Tab switcher's open-tabs-only list) and jump straight to one. Same
//! backdrop-modal shape as `command_palette`, just scoped to a much smaller,
//! recency-ordered list instead of every command/theme/project file.
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use std::path::PathBuf;

use crate::color::color;
use crate::fonts;
use crate::state::{self, Message, State};
use crate::ui::search_icon::SearchIcon;
use crate::widgets;

fn hint(label: &'static str, p: devscribe_core::theme::Palette) -> Element<'static, Message> {
    text(label)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(13.0))
        .color(color(p.text_muted))
        .into()
}

fn entry_row(
    i: usize,
    label: String,
    path: PathBuf,
    selected: usize,
    p: devscribe_core::theme::Palette,
) -> Element<'static, Message> {
    let is_selected = i == selected;
    button(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .width(Length::Fill)
            .color(if is_selected { color(p.text_strong) } else { color(p.text_body) }),
    )
    .width(Length::Fill)
    .padding(Padding {
        top: 8.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    })
    .on_press(Message::QuickOpenOpen(path))
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if is_selected || hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            text_color: color(p.text_strong),
            border: if is_selected {
                Border { color: color(p.border_focus), width: 1.5, radius: 0.0.into() }
            } else {
                Border::default()
            },
            ..button::Style::default()
        }
    })
    .into()
}

pub fn view(state: &State) -> Option<Element<'static, Message>> {
    if !state.quick_open_open {
        return None;
    }
    let p = crate::state::active_palette(state);

    let entries = state::filtered_quick_open_entries(state);
    let selected = state.quick_open_selected.min(entries.len().saturating_sub(1));

    let results: Element<'static, Message> = if entries.is_empty() {
        let message = if state.recent_files.is_empty() {
            "No files opened yet this session"
        } else {
            "No matches"
        };
        container(
            text(message)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_muted)),
        )
        .padding(16.0)
        .into()
    } else {
        let rows: Vec<Element<'static, Message>> = entries
            .into_iter()
            .enumerate()
            .map(|(i, path)| {
                let label = path.strip_prefix(&state.root).unwrap_or(&path).display().to_string();
                entry_row(i, label, path, selected, p)
            })
            .collect();
        scrollable(column(rows))
            .direction(scrollable::Direction::Vertical(widgets::thin_scrollbar()))
            .style(widgets::scrollbar_style(p))
            .height(Length::Fixed(320.0))
            .into()
    };

    let input = text_input("Search recent files\u{2026}", &state.quick_open_query)
        .id(state::quick_open_query_id())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .padding([10.0, 12.0])
        .on_input(Message::QuickOpenQueryChanged)
        .on_submit(Message::QuickOpenExecute)
        .style(move |_theme, status| text_input::Style {
            background: Color::TRANSPARENT.into(),
            border: if matches!(status, text_input::Status::Focused { .. }) {
                Border { color: color(p.border_focus), width: 1.5, radius: 0.0.into() }
            } else {
                Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() }
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

    let header = row![
        canvas(SearchIcon { color: color(p.text_muted) })
            .width(Length::Fixed(15.0))
            .height(Length::Fixed(15.0)),
        input,
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .padding([10.0, 16.0]);

    let footer = row![
        hint("\u{2191}\u{2193} navigate", p),
        hint("\u{21b5} open", p),
        hint("esc close", p),
    ]
    .spacing(16.0)
    .padding([8.0, 16.0]);

    let panel = container(column![
        header,
        container(Space::new().width(Length::Fill).height(Length::Fixed(1.0))).style(move |_theme| {
            iced::widget::container::Style {
                background: Some(color(p.border_hairline).into()),
                ..iced::widget::container::Style::default()
            }
        }),
        results,
        container(Space::new().width(Length::Fill).height(Length::Fixed(1.0))).style(move |_theme| {
            iced::widget::container::Style {
                background: Some(color(p.border_hairline).into()),
                ..iced::widget::container::Style::default()
            }
        }),
        footer,
    ])
    .width(Length::Fixed(560.0))
    .style(move |_theme| iced::widget::container::Style {
        background: Some(color(p.bg_base).into()),
        border: Border {
            color: color(p.border_accent),
            width: 1.5,
            radius: 10.0.into(),
        },
        ..iced::widget::container::Style::default()
    });

    // Same "shield the panel from the backdrop" fix `command_palette`/
    // `settings_panel` both apply — otherwise a click on dead space inside
    // the panel (footer hints, "No matches") falls through to the backdrop
    // and closes the overlay.
    let panel = mouse_area(panel).on_press(Message::Noop);

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| iced::widget::container::Style {
                background: Some(
                    Color {
                        a: 0.55,
                        ..color(p.bg_canvas)
                    }
                    .into(),
                ),
                ..iced::widget::container::Style::default()
            }),
    )
    .on_press(Message::CloseQuickOpen);

    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .padding(Padding {
            top: 96.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        });

    Some(iced::widget::stack![backdrop, centered].into())
}
