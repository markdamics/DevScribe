use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, row, text};
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
        .size(crate::text_scale::px(14.0))
        .color(color(p.text_primary));

    let version = widgets::micro("V1.4.0", color(p.text_muted));

    let brand = row![logo, wordmark, version]
        .spacing(8.0)
        .align_y(Alignment::Center);

    let palette_button = button(widgets::center_v(
        row![
            text("RUN ANYTHING")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(11.0)),
            text("⌘K").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(10.0)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    ))
    .padding([0.0, 12.0])
    .height(Length::Fixed(24.0))
    .on_press(Message::TogglePalette)
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
                .size(crate::text_scale::px(10.0)),
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

    let bar = row![
        brand,
        container(palette_button)
            .width(Length::Fill)
            .center_x(Length::Fill),
        assist_button
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .padding([0.0, 12.0]);

    let bar_panel = container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(state.density.title_bar_h()))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_panel).into()),
            ..container::Style::default()
        });

    column![bar_panel, widgets::hline(color(p.line_neutral))].into()
}
