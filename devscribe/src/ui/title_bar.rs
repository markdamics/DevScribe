use devscribe_core::theme::{Palette, ThemeName};
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, pick_list, row, text};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State};
use crate::ui::logo_mark::LogoMark;
use crate::widgets;

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let logo = canvas(LogoMark {
        left: color(p.accent),
        right: color(p.accent_2),
    })
    .width(Length::Fixed(18.0))
    .height(Length::Fixed(18.0));

    let wordmark = text("DEVSCRIBE")
        .font(fonts::display(Weight::ExtraBold))
        .size(14.0)
        .color(color(p.text_primary));

    let version = widgets::micro("V1.4.0", color(p.text_muted));

    let brand = row![logo, wordmark, version]
        .spacing(8.0)
        .align_y(Alignment::Center);

    let palette_button = button(widgets::center_v(
        row![
            text("RUN ANYTHING")
                .font(fonts::mono(Weight::Medium))
                .size(11.0),
            text("⌘K").font(fonts::mono(Weight::Medium)).size(10.0),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    ))
    .padding([0.0, 12.0])
    .height(Length::Fixed(24.0))
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: Some(color(p.bg_void).into()),
            text_color: if hovered {
                color(p.text_primary)
            } else {
                color(p.text_muted)
            },
            border: Border {
                color: if hovered {
                    color(p.accent)
                } else {
                    color(p.line_neutral)
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    });

    let assist_on = state.assist_on;
    let assist_button = button(widgets::center_v(
        row![
            widgets::dot(
                if assist_on {
                    color(p.accent)
                } else {
                    color(p.text_muted)
                },
                5.0
            ),
            text("ASSIST")
                .font(fonts::mono(Weight::Medium))
                .size(10.0),
        ]
        .spacing(6.0)
        .align_y(Alignment::Center),
    ))
    .padding([0.0, 9.0])
    .height(Length::Fixed(22.0))
    .on_press(Message::ToggleAssist)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if assist_on {
                Some(color(p.accent_soft).into())
            } else {
                None
            },
            text_color: if assist_on || hovered {
                color(p.accent)
            } else {
                color(p.text_muted)
            },
            border: Border {
                color: if assist_on {
                    color(p.line_accent)
                } else {
                    color(p.line_neutral)
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    });

    let theme_switcher = pick_list(ThemeName::ALL, Some(state.theme), Message::SetTheme)
        .font(fonts::mono(Weight::Medium))
        .text_size(10.0)
        .padding([0.0, 9.0])
        .style(move |_theme, status| {
            let open_or_hover = !matches!(status, pick_list::Status::Active);
            pick_list::Style {
                text_color: color(p.text_primary),
                placeholder_color: color(p.text_muted),
                handle_color: if open_or_hover {
                    color(p.accent)
                } else {
                    color(p.text_muted)
                },
                background: color(p.bg_void).into(),
                border: Border {
                    color: if open_or_hover {
                        color(p.accent)
                    } else {
                        color(p.line_neutral)
                    },
                    width: 1.0,
                    radius: 2.0.into(),
                },
            }
        });

    let bar = row![
        brand,
        container(palette_button)
            .width(Length::Fill)
            .center_x(Length::Fill),
        assist_button,
        theme_switcher,
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .padding([0.0, 12.0]);

    let bar_panel = container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(38.0))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_panel).into()),
            ..container::Style::default()
        });

    column![bar_panel, widgets::hline(color(p.line_neutral))].into()
}
