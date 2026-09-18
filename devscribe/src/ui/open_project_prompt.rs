//! The "Open in New Window" / "Open in This Window" choice shown whenever a
//! project-opening action (`Message::RecentProjectPicked`/
//! `FolderDialogResult`) fires while a project is already open — see
//! `State::pending_project_open`. Same centered-panel-over-backdrop shape as
//! `context_menu::rename_prompt_view`.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, text, Space};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{self, Message, State};

fn choice_button(label: &'static str, hint: &'static str, message: Message, primary: bool, p: Palette) -> Element<'static, Message> {
    let content = column![
        text(label).font(fonts::sans(Weight::Semibold)).size(crate::text_scale::px(14.0)),
        text(hint)
            .font(fonts::mono(Weight::Light))
            .size(crate::text_scale::px(12.0))
            .color(color(if primary { p.accent_on } else { p.text_muted })),
    ]
    .spacing(2.0);

    button(content)
        .width(Length::Fill)
        .padding([10.0, 14.0])
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            if primary {
                button::Style {
                    background: Some(color(p.accent_solid).into()),
                    text_color: color(p.accent_on),
                    border: Border { color: color(p.accent_solid), width: 1.5, radius: 3.0.into() },
                    ..button::Style::default()
                }
            } else {
                button::Style {
                    background: if hovered { Some(color(p.surface_hover).into()) } else { None },
                    text_color: color(p.text_strong),
                    border: Border { color: color(p.border_strong), width: 1.5, radius: 3.0.into() },
                    ..button::Style::default()
                }
            }
        })
        .into()
}

pub fn view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let pending = state.pending_project_open.as_ref()?;
    let path = pending.path().to_path_buf();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());

    let panel = container(
        column![
            text("OPEN PROJECT").font(fonts::mono(Weight::Semibold)).size(crate::text_scale::px(12.0)).color(color(p.text_muted)),
            text(name).font(fonts::display(Weight::Bold)).size(crate::text_scale::px(18.0)).color(color(p.text_strong)),
            text(state::shorten_home(&path))
                .font(fonts::mono(Weight::Light))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_muted)),
            Space::new().height(Length::Fixed(4.0)),
            choice_button("Open in New Window", "Keep the current project open", Message::OpenPendingProjectInNewWindow, true, p),
            choice_button("Open in This Window", "Replaces the current project", Message::OpenPendingProjectInThisWindow, false, p),
        ]
        .spacing(8.0),
    )
    .width(Length::Fixed(320.0))
    .padding(16.0)
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border { color: color(p.border_accent), width: 1.5, radius: 5.0.into() },
        ..container::Style::default()
    });

    // Shields the panel from the backdrop below, same reasoning
    // `rename_prompt_view`'s own shield gives.
    let panel = mouse_area(panel).on_press(Message::Noop);

    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .padding(Padding { top: 160.0, right: 0.0, bottom: 0.0, left: 0.0 });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill)).width(Length::Fill).height(Length::Fill),
    )
    .on_press(Message::CancelPendingProjectOpen);

    Some(iced::widget::stack![backdrop, centered].into())
}
