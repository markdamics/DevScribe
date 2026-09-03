//! The lighter, non-stacking "flash" confirmation pill: center-bottom,
//! auto-cleared by `Message::PruneToasts` once `Flash::created_at` exceeds
//! `FLASH_LIFETIME` (see `state.rs`). Distinct from the stacked, dismissible
//! toast system in `ui/toast.rs` — this is only for direct-action
//! confirmations (file/folder created, renamed, path copied, tree
//! collapsed), fired by the tree's draft-commit/copy-path/collapse-all
//! actions.
use iced::font::Weight;
use iced::widget::{container, text};
use iced::{alignment::Horizontal, Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State};

pub fn view(state: &State) -> Option<Element<'static, Message>> {
    let flash = state.flash.as_ref()?;
    let p = crate::state::active_palette(state);

    let pill = container(
        text(flash.text.clone())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.accent_solid)),
    )
    .padding([6.0, 14.0])
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border {
            color: color(p.border_accent),
            width: 1.5,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    });

    Some(
        container(pill)
            .width(Length::Fill)
            .align_x(Horizontal::Center)
            .align_bottom(Length::Fill)
            .padding(Padding {
                top: 0.0,
                right: 0.0,
                bottom: 44.0,
                left: 0.0,
            })
            .into(),
    )
}
