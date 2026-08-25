//! The "Select a project" welcome screen — a full-window replacement for
//! the whole editor (see `shell::view`'s early return), shown while
//! `state.welcome_open`: on first run (no recent project to auto-reopen)
//! or after "Close project" from the sidebar dropdown. Matches the mockup's
//! `position:fixed;inset:0` welcome block: a fixed-width left rail (build/
//! toolchain info) beside a recent-projects list and Open folder / Clone
//! repository / New project actions, plus a "Loading workspace" overlay
//! while a background project load (`state::start_loading_project`) is in
//! flight.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, row, scrollable, text, Space};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, LoadingProject, Message, State, WelcomeRow};
use crate::ui::logo_mark::LogoMark;
use crate::widgets;

const LEFT_RAIL_WIDTH: f32 = 352.0;

fn toolchain_row(label: &'static str, ok: bool, p: Palette) -> Element<'static, Message> {
    row![
        widgets::dot(color(if ok { p.status_success } else { p.text_faint }), 6.0),
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_body)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .into()
}

fn left_rail(p: Palette) -> Element<'static, Message> {
    // The real title-bar fold mark (`LogoMark`, two theme-accented
    // chevrons), not the generic lang-badge "D" this used to show — the
    // welcome screen is the app's actual front door, so it gets the real
    // logo like the title bar does, just a size up to match this screen's
    // larger wordmark.
    let mark = canvas(LogoMark { left: color(p.accent_solid), right: color(p.seal_solid) })
        .width(Length::Fixed(22.0))
        .height(Length::Fixed(22.0));
    let logo = row![
        mark,
        text("DevScribe")
            .font(fonts::display(Weight::Semibold))
            .size(crate::text_scale::px(17.0))
            .color(color(p.text_strong)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);

    let build = column![
        text("Build")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
        text(format!("v{}", env!("CARGO_PKG_VERSION")))
            .font(fonts::mono(Weight::Light))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body)),
    ]
    .spacing(4.0);

    let toolchains = column![
        text("Toolchains")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
        toolchain_row("RUST (this build)", true, p),
        toolchain_row("LSP // ON DEMAND PER PROJECT", true, p),
    ]
    .spacing(8.0);

    let caption = text("Workspace index rebuilt on load.\nNo project is open until one is selected.")
        .font(fonts::mono(Weight::Light))
        .size(crate::text_scale::px(13.0))
        .color(color(p.text_muted));

    let top = column![logo, build, toolchains].spacing(28.0);

    container(column![top, widgets::hline(color(p.border_hairline)), caption].spacing(16.0).padding([0.0, 0.0]))
        .width(Length::Fixed(LEFT_RAIL_WIDTH))
        .height(Length::Fill)
        .padding(28.0)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 0.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn recent_row(row_data: &WelcomeRow, p: Palette) -> Element<'static, Message> {
    let (fg, bg) = row_data.lang.badge(p);
    let content = row![
        widgets::lang_badge(row_data.lang.code(&row_data.path), fg, bg),
        column![
            text(row_data.name.clone())
                .font(fonts::display(Weight::Semibold))
                .size(crate::text_scale::px(17.0))
                .color(color(p.text_strong)),
            text(row_data.subtitle.clone())
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
        ]
        .spacing(4.0)
        .width(Length::Fill),
        text(row_data.last_opened_label.clone())
            .font(fonts::mono(Weight::Light))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body)),
    ]
    .spacing(12.0)
    .align_y(Alignment::Center);

    let path = row_data.path.clone();
    button(content)
        .width(Length::Fill)
        .padding([10.0, 16.0])
        .on_press(Message::RecentProjectPicked(path))
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered { Some(color(p.surface_hover).into()) } else { None },
                text_color: color(p.text_strong),
                border: Border {
                    color: color(p.border_hairline),
                    width: 0.0,
                    radius: 0.0.into(),
                },
                ..button::Style::default()
            }
        })
        .into()
}

fn recent_list(state: &State, p: Palette) -> Element<'static, Message> {
    let header = row![
        text("Recent")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted))
            .width(Length::Fill),
        text("Last opened")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    ]
    .padding([8.0, 16.0]);

    let header = container(header).width(Length::Fill).style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        ..container::Style::default()
    });

    let body: Element<'static, Message> = if state.welcome_rows.is_empty() {
        widgets::placeholder("No recent projects \u{2014} open a folder to get started", p)
    } else {
        column(state.welcome_rows.iter().map(|row_data| recent_row(row_data, p)).collect::<Vec<_>>()).into()
    };

    container(column![header, body])
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.5,
                radius: 3.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn action_button(glyph: &'static str, label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(
        row![
            text(glyph).font(fonts::mono(Weight::Bold)).size(crate::text_scale::px(15.0)),
            text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .padding([8.0, 14.0])
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: None,
            text_color: if hovered { color(p.accent_solid) } else { color(p.text_strong) },
            border: Border {
                color: if hovered { color(p.accent_solid) } else { color(p.border_strong) },
                width: 1.5,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

fn right_pane(state: &State, p: Palette) -> Element<'static, Message> {
    let header = row![
        text("Select a project")
            .font(fonts::display(Weight::Semibold))
            .size(crate::text_scale::px(22.0))
            .color(color(p.text_strong))
            .width(Length::Fill),
        text(format!("{} PROJECTS", state.welcome_rows.len()))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    ]
    .align_y(Alignment::Center);

    let actions = row![
        action_button("+", "Open folder", Message::OpenFolderDialog, p),
        action_button("\u{2726}", "New project", Message::NewProjectDialog, p),
    ]
    .spacing(12.0);

    let hints = row![
        widgets::micro("\u{23ce} Load", color(p.text_muted)),
        widgets::micro("\u{2318}O Open folder", color(p.text_muted)),
        widgets::micro("\u{2318}K Run anything", color(p.text_muted)),
    ]
    .spacing(24.0);

    let content = column![header, recent_list(state, p), actions, hints]
        .spacing(24.0)
        .padding(48.0)
        .width(Length::Fill)
        .height(Length::Fill);

    container(scrollable(content).width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        })
        .into()
}

/// One segment of the loading overlay's progress bar — a fixed-height
/// colored strip sized by `portion` relative to its sibling in the
/// enclosing `row!`. No real animation (`iced` has no CSS-keyframe
/// equivalent); a static partial fill is enough to read as "in progress,"
/// matching the mockup, which also never reports real percentages.
fn progress_segment(portion: u16, bg: iced::Color) -> Element<'static, Message> {
    container(Space::new().width(Length::Fill).height(Length::Fill))
        .width(Length::FillPortion(portion))
        .height(Length::Fixed(3.0))
        .style(move |_theme| container::Style { background: Some(bg.into()), ..container::Style::default() })
        .into()
}

fn loading_overlay(loading: &LoadingProject, p: Palette) -> Element<'static, Message> {
    let panel = container(
        column![
            text("Loading workspace")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
            text(loading.name.clone())
                .font(fonts::display(Weight::Bold))
                .size(crate::text_scale::px(22.0))
                .color(color(p.text_strong)),
            text(state::shorten_home(&loading.path))
                .font(fonts::mono(Weight::Light))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
            row![progress_segment(45, color(p.accent_solid)), progress_segment(55, color(p.surface_inset))].width(Length::Fill),
            text("INDEXING SOURCES // SPAWNING LSP")
                .font(fonts::mono(Weight::Light))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_body)),
        ]
        .spacing(12.0),
    )
    .width(Length::Fixed(420.0))
    .padding(24.0)
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border { color: color(p.border_hairline), width: 1.5, radius: 3.0.into() },
        ..container::Style::default()
    });

    container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .style(move |_theme| container::Style { background: Some(color(p.overlay_scrim).into()), ..container::Style::default() })
        .into()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let base: Element<'static, Message> = container(row![left_rail(p), right_pane(state, p)].width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            text_color: Some(color(p.text_strong)),
            ..container::Style::default()
        })
        .into();

    match state.loading_project.as_ref() {
        Some(loading) => iced::widget::stack![base, loading_overlay(loading, p)].into(),
        None => base,
    }
}
