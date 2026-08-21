//! The `⌘K` "run anything" overlay: type to filter files/themes/actions,
//! arrow keys to move, Enter to run. Rendered as a `stack!` layer over the
//! whole shell — see `shell.rs`.
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{self, Message, State};
use crate::ui::search_icon::SearchIcon;

/// A `↑↓ navigate` / `esc close`-style hint, used in the palette's footer.
fn hint(label: &'static str, p: devscribe_core::theme::Palette) -> Element<'static, Message> {
    text(label)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(10.0))
        .color(color(p.text_muted))
        .into()
}

pub fn view(state: &State) -> Option<Element<'static, Message>> {
    if !state.palette_open {
        return None;
    }
    let p = devscribe_core::theme::palette(state.theme);

    let entries = state::filtered_palette_entries(state);
    let selected = state.palette_selected.min(entries.len().saturating_sub(1));

    let rows: Vec<Element<'static, Message>> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let is_selected = i == selected;
            let label = text(entry.label.clone())
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(12.0))
                .color(if is_selected {
                    color(p.text_primary)
                } else {
                    color(p.text_secondary)
                });
            button(label)
                .width(Length::Fill)
                .padding(Padding {
                    top: 8.0,
                    right: 12.0,
                    bottom: 8.0,
                    left: 12.0,
                })
                .on_press(Message::PaletteRun(entry.action.clone()))
                .style(move |_theme, status| {
                    let hovered = status == button::Status::Hovered;
                    button::Style {
                        background: if is_selected || hovered {
                            Some(color(p.surface_hover).into())
                        } else {
                            None
                        },
                        text_color: color(p.text_primary),
                        ..button::Style::default()
                    }
                })
                .into()
        })
        .collect();

    let results: Element<'static, Message> = if rows.is_empty() {
        container(
            text("No matches")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_muted)),
        )
        .padding(16.0)
        .into()
    } else {
        scrollable(column(rows)).height(Length::Fixed(320.0)).into()
    };

    let input = text_input("Run anything\u{2026}", &state.palette_query)
        .id(state::palette_query_id())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(14.0))
        .padding([10.0, 12.0])
        .on_input(Message::PaletteQueryChanged)
        .on_submit(Message::PaletteExecute)
        .style(move |_theme, _status| text_input::Style {
            background: Color::TRANSPARENT.into(),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_primary),
            selection: {
                let mut c = p.accent;
                c.a = 0.35;
                color(c)
            },
        });

    let mode_badge = container(
        text("Commands")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted)),
    )
    .padding([2.0, 8.0])
    .style(move |_theme| container::Style {
        border: Border {
            color: color(p.line_neutral),
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    });

    let header = row![
        canvas(SearchIcon { color: color(p.text_muted) })
            .width(Length::Fixed(15.0))
            .height(Length::Fixed(15.0)),
        input,
        mode_badge,
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .padding([10.0, 16.0]);

    let footer = row![
        hint("\u{2191}\u{2193} navigate", p),
        hint("\u{21b5} run", p),
        hint("esc close", p),
    ]
    .spacing(16.0)
    .padding([8.0, 16.0]);

    let panel = container(column![
        header,
        container(Space::new().width(Length::Fill).height(Length::Fixed(1.0))).style(move |_theme| {
            container::Style {
                background: Some(color(p.line_neutral).into()),
                ..container::Style::default()
            }
        }),
        results,
        container(Space::new().width(Length::Fill).height(Length::Fixed(1.0))).style(move |_theme| {
            container::Style {
                background: Some(color(p.line_neutral).into()),
                ..container::Style::default()
            }
        }),
        footer,
    ])
    .width(Length::Fixed(560.0))
    .style(move |_theme| container::Style {
        background: Some(color(p.bg_panel).into()),
        border: Border {
            color: color(p.line_accent),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(
                    Color {
                        a: 0.55,
                        ..color(p.bg_void)
                    }
                    .into(),
                ),
                ..container::Style::default()
            }),
    )
    .on_press(Message::ClosePalette);

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
