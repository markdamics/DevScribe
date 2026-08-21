//! The settings overlay: density and the problem-lens toggle, per the
//! plan's milestone 8 scope. Rendered as a `stack!` layer over the shell,
//! same backdrop-modal mechanism as `command_palette`.
use devscribe_core::theme::{Palette, ThemeName};
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, text, Space};
use iced::{Alignment, Border, Color, Element, Length};

use crate::color::color;
use crate::density::Density;
use crate::fonts;
use crate::state::{
    Message, State, EDITOR_FONT_SIZE_MAX, EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_STEP,
    UI_FONT_SCALE_MAX, UI_FONT_SCALE_MIN, UI_FONT_SCALE_STEP,
};
use crate::widgets;

/// A stepper's `-`/`+` button. Always has an `on_press` — even sitting at
/// the min/max, it re-sends the same clamped value rather than going
/// disabled. A disabled `on_press_maybe(None)` button doesn't capture the
/// click, so it falls through to whatever is stacked underneath; here
/// that's the settings modal's backdrop `mouse_area`, which would close the
/// whole panel on a click that was only ever meant to hit a floor/ceiling.
fn stepper_button(label: &'static str, p: Palette, message: Message, enabled: bool) -> Element<'static, Message> {
    button(
        text(label)
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(13.0))
            .color(if enabled { color(p.text_primary) } else { color(p.text_muted) }),
    )
    .width(Length::Fixed(28.0))
    .height(Length::Fixed(28.0))
    .padding(0.0)
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            border: Border {
                color: color(p.line_neutral),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

fn stepper_row(
    value_label: String,
    dec_message: Message,
    inc_message: Message,
    dec_enabled: bool,
    inc_enabled: bool,
    p: Palette,
) -> Element<'static, Message> {
    row![
        stepper_button("\u{2212}", p, dec_message, dec_enabled),
        container(
            text(value_label)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_primary)),
        )
        .width(Length::Fixed(48.0))
        .align_x(Alignment::Center),
        stepper_button("+", p, inc_message, inc_enabled),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .into()
}

fn font_size_row(state: &State, p: Palette) -> Element<'static, Message> {
    let size = state.editor_font_size;
    stepper_row(
        format!("{size:.0}px"),
        Message::SetEditorFontSize((size - EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX)),
        Message::SetEditorFontSize((size + EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX)),
        size > EDITOR_FONT_SIZE_MIN + f32::EPSILON,
        size < EDITOR_FONT_SIZE_MAX - f32::EPSILON,
        p,
    )
}

fn ui_scale_row(state: &State, p: Palette) -> Element<'static, Message> {
    let scale = state.ui_font_scale;
    stepper_row(
        format!("{:.0}%", scale * 100.0),
        Message::SetUiFontScale((scale - UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX)),
        Message::SetUiFontScale((scale + UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX)),
        scale > UI_FONT_SCALE_MIN + f32::EPSILON,
        scale < UI_FONT_SCALE_MAX - f32::EPSILON,
        p,
    )
}

fn theme_grid(state: &State, p: Palette) -> Element<'static, Message> {
    let rows: Vec<Element<'static, Message>> = ThemeName::ALL
        .chunks(4)
        .map(|chunk| {
            let buttons: Vec<Element<'static, Message>> = chunk
                .iter()
                .map(|&theme| {
                    let active = state.theme == theme;
                    let swatch = devscribe_core::theme::palette(theme).accent;
                    button(
                        row![
                            widgets::dot(color(swatch), 6.0),
                            text(theme.label())
                                .font(fonts::mono(Weight::Medium))
                                .size(crate::text_scale::px(11.0))
                                .color(if active { color(p.accent) } else { color(p.text_secondary) }),
                        ]
                        .spacing(6.0)
                        .align_y(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([6.0, 10.0])
                    .on_press(Message::SetTheme(theme))
                    .style(move |_theme, status| {
                        let hovered = status == button::Status::Hovered;
                        button::Style {
                            background: if active {
                                Some(color(p.accent_soft).into())
                            } else if hovered {
                                Some(color(p.surface_hover).into())
                            } else {
                                None
                            },
                            border: Border {
                                color: if active { color(p.line_accent) } else { color(p.line_neutral) },
                                width: 1.0,
                                radius: 2.0.into(),
                            },
                            ..button::Style::default()
                        }
                    })
                    .into()
                })
                .collect();
            row(buttons).spacing(8.0).into()
        })
        .collect();

    column(rows).spacing(8.0).into()
}

fn section_label(label: &'static str, p: Palette) -> Element<'static, Message> {
    text(label)
        .font(fonts::mono(Weight::Bold))
        .size(crate::text_scale::px(10.0))
        .color(color(p.text_muted))
        .into()
}

fn density_row(state: &State, p: Palette) -> Element<'static, Message> {
    let options: Vec<Element<'static, Message>> = Density::ALL
        .into_iter()
        .map(|density| {
            let active = state.density == density;
            button(
                text(density.label())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(12.0))
                    .color(if active { color(p.accent) } else { color(p.text_secondary) }),
            )
            .padding([6.0, 14.0])
            .on_press(Message::SetDensity(density))
            .style(move |_theme, status| {
                let hovered = status == button::Status::Hovered;
                button::Style {
                    background: if active {
                        Some(color(p.accent_soft).into())
                    } else if hovered {
                        Some(color(p.surface_hover).into())
                    } else {
                        None
                    },
                    border: Border {
                        color: if active { color(p.line_accent) } else { color(p.line_neutral) },
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    ..button::Style::default()
                }
            })
            .into()
        })
        .collect();

    row(options).spacing(8.0).into()
}

fn problem_lens_row(state: &State, p: Palette) -> Element<'static, Message> {
    let enabled = state.problem_lens_enabled;
    button(
        row![
            widgets::dot(if enabled { color(p.accent) } else { color(p.text_muted) }, 6.0),
            text("Show inline problem hints")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_primary)),
            iced::widget::Space::new().width(Length::Fill),
            text(if enabled { "ON" } else { "OFF" })
                .font(fonts::mono(Weight::Bold))
                .size(crate::text_scale::px(10.0))
                .color(if enabled { color(p.status_ok) } else { color(p.text_muted) }),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8.0, 10.0])
    .on_press(Message::ToggleProblemLens)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            border: Border {
                color: color(p.line_neutral),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

pub fn view(state: &State) -> Option<Element<'static, Message>> {
    if !state.settings_open {
        return None;
    }
    let p = devscribe_core::theme::palette(state.theme);

    let header = row![
        text("SETTINGS")
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_primary))
            .width(Length::Fill),
        button(text("\u{2715}").size(crate::text_scale::px(11.0)).color(color(p.text_muted)))
            .padding(0.0)
            .width(Length::Fixed(18.0))
            .height(Length::Fixed(18.0))
            .on_press(Message::CloseSettings)
            .style(|_theme, _status| button::Style::default()),
    ]
    .align_y(Alignment::Center);

    let body = column![
        header,
        widgets::hline(color(p.line_neutral)),
        column![section_label("THEME", p), theme_grid(state, p)].spacing(8.0),
        column![section_label("DENSITY", p), density_row(state, p)].spacing(8.0),
        column![section_label("UI TEXT SIZE", p), ui_scale_row(state, p)].spacing(8.0),
        column![
            section_label("EDITOR", p),
            font_size_row(state, p),
            problem_lens_row(state, p),
        ]
        .spacing(8.0),
    ]
    .spacing(16.0)
    .padding(16.0);

    let panel = container(body)
        .width(Length::Fixed(440.0))
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
    .on_press(Message::CloseSettings);

    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center);

    Some(iced::widget::stack![backdrop, centered].into())
}
