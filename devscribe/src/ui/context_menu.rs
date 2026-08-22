use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, text, Space};
use iced::{Alignment, Border, Element, Length, Padding};
use std::path::Path;

use crate::color::color;
use crate::fonts;
use crate::state::{DraftKind, Message, State};
use crate::widgets;

fn menu_row(label: &'static str, shortcut: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(
        row![
            text(label)
                .font(fonts::sans(iced::font::Weight::Medium))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_primary))
                .width(Length::Fill),
            text(shortcut)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(10.0))
                .color(color(p.text_muted)),
        ]
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([6.0, 8.0])
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.bg_panel).into())
            } else {
                None
            },
            ..button::Style::default()
        }
    })
    .into()
}

pub fn view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let ctx = state.ctx_menu.as_ref()?;

    let target_label = ctx
        .target
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "PROJECT ROOT".to_string());

    let dir = match ctx.target.as_ref() {
        Some(path) if path.is_dir() => path.clone(),
        Some(path) => path.parent().map(Path::to_path_buf).unwrap_or_else(|| state.root.clone()),
        None => state.root.clone(),
    };

    let mut rows: Vec<Element<'static, Message>> = vec![
        text(target_label.to_uppercase())
            .font(fonts::mono(Weight::Semibold))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted))
            .into(),
        menu_row("New file", "\u{2318}N", Message::BeginDraftIn(DraftKind::NewFile, dir.clone()), p),
        menu_row("New folder", "\u{21e7}\u{2318}N", Message::BeginDraftIn(DraftKind::NewFolder, dir), p),
    ];
    if let Some(target) = ctx.target.clone() {
        rows.push(widgets::hline(color(p.line_neutral)));
        rows.push(menu_row("Rename", "\u{21b5}", Message::BeginRename(target.clone()), p));
        rows.push(menu_row("Copy path", "\u{2325}\u{2318}C", Message::CopyPath(target), p));
    }

    let menu = container(column(rows).spacing(2.0).padding(6.0))
        .width(Length::Fixed(214.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border {
                color: color(p.line_neutral),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        });

    let positioned = container(menu).width(Length::Fill).height(Length::Fill).padding(Padding {
        top: 128.0,
        right: 0.0,
        bottom: 0.0,
        left: 16.0,
    });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::CloseTreeContext)
    .on_right_press(Message::CloseTreeContext);

    Some(iced::widget::stack![backdrop, positioned].into())
}
