//! Small reusable visual primitives shared across shell panels.
use crate::color::color;
use crate::fonts;
use devscribe_core::theme::Palette;
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
            .size(crate::text_scale::px(10.0))
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
        .size(crate::text_scale::px(13.0))
        .color(color)
        .into()
}

/// A 1px horizontal hairline, filling its parent's width.
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

/// A 1px vertical hairline, filling its parent's height — e.g. the divider
/// between the settings modal's category nav and its content pane.
pub fn vline<'a, Message: 'a>(color: Color) -> Element<'a, Message> {
    container(Space::new().width(Length::Fixed(1.0)).height(Length::Fill))
        .width(Length::Fixed(1.0))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            ..container::Style::default()
        })
        .into()
}

/// A muted, centered message filling its panel — "nothing to show here" for
/// any tab (no file open, not JSON, no diff, empty search).
pub fn placeholder<'a, Message: 'a>(
    label: impl text::IntoFragment<'a>,
    p: Palette,
) -> Element<'a, Message> {
    container(
        text(label)
            .font(fonts::mono(iced::font::Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_muted)),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(color(p.bg_canvas).into()),
        ..container::Style::default()
    })
    .into()
}

/// Wraps `content` (an icon-only control — a bare "\u{2715}"/"\u{2191}" glyph
/// button, with no text label of its own) with a small hover tooltip naming
/// what it does. iced 0.14 has no accessibility-tree/screen-reader API to
/// hook into at all (no `accesskit` anywhere in this dependency tree), so
/// this can't be a real ARIA-style hint for assistive tech — it's the
/// closest working equivalent this framework actually offers: a visible,
/// hoverable label for a control that otherwise has none, which is exactly
/// the same information a screen reader's own accessible-name lookup would
/// need if one were available (accessibility pass, item 12).
pub fn tooltip<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    p: Palette,
) -> Element<'a, Message> {
    iced::widget::Tooltip::new(
        content,
        container(
            text(label)
                .font(fonts::mono(iced::font::Weight::Medium))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_strong)),
        )
        .padding([4.0, 8.0])
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 3.0.into() },
            ..container::Style::default()
        }),
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(6.0)
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
