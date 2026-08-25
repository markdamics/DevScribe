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
        left: color(p.accent_solid),
        right: color(p.seal_solid),
    })
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(22.0));

    let wordmark = text("DEVSCRIBE")
        .font(fonts::display(Weight::ExtraBold))
        .size(crate::text_scale::px(18.0))
        .color(color(p.text_strong));

    // The real crate version, not the mockup's static "V1.4.0" — Phase 7
    // added a real version readout to Settings → About, and having the
    // title bar show a *different, fake* number right next to it would be
    // its own new inconsistency.
    let version = widgets::micro(concat!("V", env!("CARGO_PKG_VERSION")), color(p.text_muted));

    let brand = row![logo, wordmark, version]
        .spacing(8.0)
        .align_y(Alignment::Center);

    let palette_button = button(widgets::center_v(
        row![
            text("RUN ANYTHING")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(15.0)),
            text("⌘K").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(15.0)),
        ]
        .spacing(9.0)
        .align_y(Alignment::Center),
    ))
    .padding([0.0, 16.0])
    .height(Length::Fixed(34.0))
    .on_press(Message::TogglePalette)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: Some(color(p.bg_canvas).into()),
            text_color: if hovered {
                color(p.text_strong)
            } else {
                color(p.text_muted)
            },
            border: Border {
                color: if hovered {
                    color(p.accent_solid)
                } else {
                    color(p.border_hairline)
                },
                width: 1.5,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    });

    let assist_on = state.assist_on;
    let assist_button = button(widgets::center_v(
        row![
            widgets::dot(
                if assist_on {
                    color(p.accent_solid)
                } else {
                    color(p.text_muted)
                },
                7.0
            ),
            text("ASSIST")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(14.0)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    ))
    .padding([0.0, 14.0])
    .height(Length::Fixed(32.0))
    .on_press(Message::ToggleAssist)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if assist_on {
                Some(color(p.accent_quiet).into())
            } else {
                None
            },
            text_color: if assist_on || hovered {
                color(p.accent_solid)
            } else {
                color(p.text_muted)
            },
            border: Border {
                color: if assist_on {
                    color(p.border_accent)
                } else {
                    color(p.border_hairline)
                },
                width: 1.5,
                radius: 3.0.into(),
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
            background: Some(color(p.bg_base).into()),
            ..container::Style::default()
        });

    column![bar_panel, widgets::hline(color(p.border_hairline))].into()
}
