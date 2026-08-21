//! Small reusable visual primitives shared across shell panels.
use crate::fonts;
use iced::alignment::Vertical;
use iced::border;
use iced::widget::{container, text, Space};
use iced::{Border, Color, Element, Length};

/// Centers `content` in both axes, filling whatever space its parent (e.g. a
/// fixed-size `button`) allocates. `iced`'s `button`/`container` don't center
/// their content by default, they lay it out top-left, so every fixed-size
/// icon-only control needs this.
pub fn center_fill<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content).center(Length::Fill).into()
}

/// Centers `content` vertically only, leaving its width at `Shrink` — for
/// controls that should auto-size horizontally (e.g. a pill button sized by
/// its label) but still need vertical centering within a taller fixed-height box.
pub fn center_v<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .height(Length::Fill)
        .align_y(Vertical::Center)
        .into()
}

/// A small circular status marker (git-status dot, LSP/problem indicator).
pub fn dot<'a, Message: 'a>(color: Color, size: f32) -> Element<'a, Message> {
    container(Space::new().width(Length::Fixed(size)).height(Length::Fixed(size)))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            border: border::rounded(size / 2.0),
            ..container::Style::default()
        })
        .into()
}

/// A two-letter monospace language badge on a tinted square (e.g. `RS`, `TS`, `JV`).
pub fn lang_badge<'a, Message: 'a>(
    label: impl text::IntoFragment<'a>,
    fg: Color,
    bg: Color,
) -> Element<'a, Message> {
    container(
        text(label)
            .font(fonts::mono(iced::font::Weight::Bold))
            .size(8.0)
            .color(fg),
    )
    .center(Length::Fixed(18.0))
    .style(move |_theme| container::Style {
        background: Some(bg.into()),
        border: border::rounded(2.0),
        ..container::Style::default()
    })
    .into()
}

/// A `--type-micro` uppercase mono label: the system's most-used text treatment.
pub fn micro<'a, Message: 'a>(content: &'a str, color: Color) -> Element<'a, Message> {
    text(content)
        .font(fonts::mono(iced::font::Weight::Medium))
        .size(10.0)
        .color(color)
        .into()
}

/// A 1px hairline, horizontal by default.
pub fn hline<'a, Message: 'a>(color: Color) -> Element<'a, Message> {
    container(Space::new().width(Length::Fill).height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            ..container::Style::default()
        })
        .into()
}

/// Shorthand for an unfilled rectangular border, e.g. panel dividers.
#[allow(dead_code)]
pub fn border_1(color: Color) -> Border {
    Border {
        color,
        width: 1.0,
        radius: 0.0.into(),
    }
}
