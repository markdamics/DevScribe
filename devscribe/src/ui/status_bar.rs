use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{container, row, text};
use iced::{Alignment, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State};
use crate::widgets;

pub fn view(_state: &State, p: Palette) -> Element<'static, Message> {
    let problems = row![
        widgets::dot(color(p.status_danger), 6.0),
        text("2 PROBLEMS")
            .font(fonts::mono(Weight::Medium))
            .size(11.0)
            .color(color(p.text_muted)),
    ]
    .spacing(6.0)
    .align_y(Alignment::Center);

    let lsp = row![
        widgets::dot(color(p.status_ok), 6.0),
        text("rust-analyzer ready")
            .font(fonts::mono(Weight::Medium))
            .size(11.0)
            .color(color(p.text_muted)),
    ]
    .spacing(6.0)
    .align_y(Alignment::Center);

    let branch = text("feat/settle-batch")
        .font(fonts::mono(Weight::Medium))
        .size(11.0)
        .color(color(p.text_muted));

    let bar = row![
        problems,
        iced::widget::Space::new().width(Length::Fill),
        lsp,
        branch,
    ]
    .spacing(16.0)
    .align_y(Alignment::Center)
    .padding([0.0, 16.0]);

    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(28.0))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_panel).into()),
            ..container::Style::default()
        })
        .into()
}
