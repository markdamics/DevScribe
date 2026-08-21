//! Toast notifications: stacked bottom-right, auto-pruned after a few
//! seconds by `Message::PruneToasts` (see `state::subscription`), dismissible
//! by hand too. Rendered as a `stack!` layer over the whole shell.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State, Toast, ToastKind};

fn toast_card(toast: &Toast, p: Palette) -> Element<'static, Message> {
    let accent = match toast.kind {
        ToastKind::Success => p.status_ok,
        ToastKind::Warning => p.status_warn,
        ToastKind::Error => p.status_danger,
    };

    let dismiss = button(text("\u{2715}").size(crate::text_scale::px(10.0)).color(color(p.text_muted)))
        .padding(0.0)
        .width(Length::Fixed(16.0))
        .height(Length::Fixed(16.0))
        .on_press(Message::DismissToast(toast.id))
        .style(|_theme, _status| button::Style::default());

    let content = row![
        text(toast.message.clone())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_primary))
            .width(Length::Fill),
        dismiss,
    ]
    .spacing(10.0)
    .align_y(Alignment::Center);

    container(content)
        .width(Length::Fixed(320.0))
        .padding(Padding {
            top: 10.0,
            right: 10.0,
            bottom: 10.0,
            left: 12.0,
        })
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_panel).into()),
            border: Border {
                color: color(accent),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

pub fn view(state: &State) -> Option<Element<'static, Message>> {
    if state.toasts.is_empty() {
        return None;
    }
    let p = devscribe_core::theme::palette(state.theme);

    let cards: Vec<Element<'static, Message>> =
        state.toasts.iter().map(|toast| toast_card(toast, p)).collect();

    Some(
        container(column(cards).spacing(8.0))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_right(Length::Fill)
            .align_bottom(Length::Fill)
            .padding(16.0)
            .into(),
    )
}
