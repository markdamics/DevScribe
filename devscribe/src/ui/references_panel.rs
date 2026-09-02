//! The Locations dock panel: every candidate from a multi-result "Go to
//! Definition", or every hit from "Find All References", listed as
//! clickable `file:line` rows — the same shape and dock position as
//! `status_bar.rs`'s Problems panel, shown between the editor content and
//! the status bar (see `shell.rs`) while `state.references_open`.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{CursorPos, LocationEntry, Message, State};
use crate::widgets;

pub(crate) const PANEL_HEIGHT: f32 = 196.0;

fn location_row(entry: &LocationEntry, root: &std::path::Path, p: Palette) -> Element<'static, Message> {
    let location = format!(
        "{}:{}",
        entry.path.strip_prefix(root).unwrap_or(&entry.path).display(),
        entry.line + 1,
    );

    button(
        row![
            text(location)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
            text(entry.preview.clone())
                .font(fonts::mono(Weight::Normal))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_body)),
        ]
        .spacing(10.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([4.0, 16.0])
    .on_press(Message::JumpToLocation(
        entry.path.clone(),
        CursorPos { line: entry.line, col: entry.col },
    ))
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.surface_raised).into()) } else { None },
            ..button::Style::default()
        }
    })
    .into()
}

pub fn dock_panel(state: &State, p: Palette) -> Element<'static, Message> {
    let header = row![
        text(state.references_label.clone())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.accent_solid)),
        iced::widget::Space::new().width(Length::Fill),
        button(text("\u{2715}").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)))
            .padding(0.0)
            .on_press(Message::ToggleReferencesPanel)
            .style(move |_theme, status| button::Style {
                text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
                ..button::Style::default()
            }),
    ]
    .align_y(Alignment::Center)
    .padding([8.0, 16.0]);

    let root = state.root.clone();
    let list: Element<'static, Message> = if state.references_results.is_empty() {
        widgets::placeholder("No locations to show", p)
    } else {
        let rows: Vec<Element<'static, Message>> =
            state.references_results.iter().map(|entry| location_row(entry, &root, p)).collect();
        scrollable(column(rows)).width(Length::Fill).height(Length::Fill).into()
    };

    container(column![header, widgets::hline(color(p.border_hairline)), list].width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fixed(PANEL_HEIGHT))
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}
