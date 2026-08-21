use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State, Tab};
use crate::widgets;

fn tab_button(
    active: bool,
    p: Palette,
    tab: Tab,
    content: Element<'static, Message>,
) -> Element<'static, Message> {
    let inner = button(widgets::center_v(content))
        .height(Length::Fixed(37.0))
        .padding(Padding {
            top: 0.0,
            right: 16.0,
            bottom: 0.0,
            left: 16.0,
        })
        .on_press(Message::SelectTab(tab))
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if active {
                    Some(color(p.bg_void).into())
                } else if hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                text_color: if active {
                    color(p.text_primary)
                } else {
                    color(p.text_muted)
                },
                ..button::Style::default()
            }
        });

    let underline = container(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fixed(1.0)),
    )
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(move |_theme| container::Style {
            background: Some(
                if active {
                    color(p.accent)
                } else {
                    color(p.line_neutral)
                }
                .into(),
            ),
            ..container::Style::default()
        });

    column![inner, underline].into()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let code = tab_button(
        state.active_tab == Tab::Code,
        p,
        Tab::Code,
        row![
            widgets::lang_badge("RS", color(p.status_warn), {
                let mut c = p.status_warn;
                c.a = 0.16;
                color(c)
            }),
            text("engine.rs")
                .font(fonts::sans(Weight::Medium))
                .size(12.0),
            widgets::dot(color(p.status_warn), 5.0),
        ]
        .spacing(7.0)
        .align_y(Alignment::Center)
        .into(),
    );

    let json = tab_button(
        state.active_tab == Tab::Json,
        p,
        Tab::Json,
        row![
            widgets::lang_badge("JS", color(p.status_ok), {
                let mut c = p.status_ok;
                c.a = 0.16;
                color(c)
            }),
            text("toolchains.json")
                .font(fonts::sans(Weight::Medium))
                .size(12.0),
        ]
        .spacing(7.0)
        .align_y(Alignment::Center)
        .into(),
    );

    let search = tab_button(
        state.active_tab == Tab::Search,
        p,
        Tab::Search,
        row![text("\u{201c}settle_batch\u{201d}")
            .font(fonts::sans(Weight::Medium))
            .size(12.0)
            .color(color(p.text_primary))]
        .into(),
    );

    let diff = tab_button(
        state.active_tab == Tab::Diff,
        p,
        Tab::Diff,
        row![text("engine.rs \u{2194} HEAD")
            .font(fonts::sans(Weight::Medium))
            .size(12.0)
            .color(color(p.text_primary))]
        .into(),
    );

    let tabs = row![code, json, search, diff]
        .width(Length::Fill)
        .height(Length::Fixed(38.0));

    let overflow_open = state.overflow_open;
    let overflow = button(widgets::center_fill(text("\u{22ef}").size(15.0)))
        .width(Length::Fixed(38.0))
        .height(Length::Fixed(38.0))
        .padding(0.0)
        .on_press(Message::ToggleOverflow)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                text_color: if hovered || overflow_open {
                    color(p.text_primary)
                } else {
                    color(p.text_muted)
                },
                ..button::Style::default()
            }
        });

    let bar = row![tabs, overflow].align_y(Alignment::Center);

    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(38.0))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_panel).into()),
            ..container::Style::default()
        })
        .into()
}
