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
    styled_menu_row(label, shortcut, message, color(p.text_strong), p)
}

/// "Delete"'s own row, in `status_danger` rather than the normal text
/// color — the one destructive entry in this menu, colored to stand apart
/// from New file/New folder/Rename/Copy path the way the mockup's other
/// danger-tinted UI (e.g. the sidebar's `Deleted` change badge) already
/// does.
fn danger_menu_row(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    styled_menu_row(label, "", message, color(p.status_danger), p)
}

fn styled_menu_row(
    label: &'static str,
    shortcut: &'static str,
    message: Message,
    text_color: iced::Color,
    p: Palette,
) -> Element<'static, Message> {
    button(
        row![
            text(label)
                .font(fonts::sans(iced::font::Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(text_color)
                .width(Length::Fill),
            text(shortcut)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
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
                Some(color(p.bg_base).into())
            } else {
                None
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// The confirm/cancel step shown once "Delete" has been clicked once
/// (`ContextMenu::confirm_delete`) — a plain-text warning plus two rows,
/// replacing the normal New file/New folder/Rename/Copy path/Delete rows
/// entirely rather than layering a second menu on top.
fn confirm_delete_rows(target: &Path, p: Palette) -> Vec<Element<'static, Message>> {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let noun = if target.is_dir() { "folder and everything in it" } else { "file" };
    vec![
        text(format!("Delete this {noun}?"))
            .font(fonts::sans(iced::font::Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_strong))
            .into(),
        text(name)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted))
            .into(),
        text("This can't be undone.")
            .font(fonts::sans(iced::font::Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.status_danger))
            .into(),
        widgets::hline(color(p.border_hairline)),
        menu_row("Cancel", "", Message::CloseTreeContext, p),
        danger_menu_row("Delete permanently", Message::DeletePath(target.to_path_buf()), p),
    ]
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

    let rows: Vec<Element<'static, Message>> = if let Some(target) = ctx.target.as_ref().filter(|_| ctx.confirm_delete) {
        confirm_delete_rows(target, p)
    } else {
        let mut rows: Vec<Element<'static, Message>> = vec![
            text(target_label.to_uppercase())
                .font(fonts::mono(Weight::Semibold))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted))
                .into(),
            menu_row("New file", "\u{2318}N", Message::BeginDraftIn(DraftKind::NewFile, dir.clone()), p),
            menu_row("New folder", "\u{21e7}\u{2318}N", Message::BeginDraftIn(DraftKind::NewFolder, dir), p),
        ];
        if let Some(target) = ctx.target.clone() {
            rows.push(widgets::hline(color(p.border_hairline)));
            rows.push(menu_row("Rename", "\u{21b5}", Message::BeginRename(target.clone()), p));
            rows.push(menu_row("Copy path", "\u{2325}\u{2318}C", Message::CopyPath(target), p));
            rows.push(widgets::hline(color(p.border_hairline)));
            rows.push(danger_menu_row("Delete", Message::PromptDeletePath, p));
        }
        rows
    };

    let menu = container(column(rows).spacing(2.0).padding(6.0))
        .width(Length::Fixed(214.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.5,
                radius: 3.0.into(),
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
