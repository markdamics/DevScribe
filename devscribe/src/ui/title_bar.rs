use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, row, text};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, Message, State};
use crate::ui::breadcrumb_bar;
use crate::ui::logo_mark::LogoMark;
use crate::ui::run_icon::RunIcon;
use crate::widgets;

/// The `DEVSCRIBE` wordmark's own separator glyph — used again between the
/// brand and the breadcrumb trail (see `view` below), so the trail reads as
/// "attached to" the title rather than a second, unrelated bar segment.
fn pipe_sep(p: Palette) -> Element<'static, Message> {
    text("|")
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .color(color(p.border_hairline))
        .into()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let logo = canvas(LogoMark {
        left: color(p.accent_solid),
        right: color(p.seal_solid),
    })
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(22.0));

    let wordmark = text("DEVSCRIBE")
        .font(fonts::display(Weight::ExtraBold))
        .size(crate::text_scale::px(18.0))
        .color(color(p.text_strong));

    // The real crate version, not the mockup's static "V1.4.0" — Phase 7
    // added a real version readout to Settings → About, and having the
    // title bar show a *different, fake* number right next to it would be
    // its own new inconsistency.
    let version = widgets::micro(concat!("V", env!("CARGO_PKG_VERSION")), color(p.text_muted));

    let mut brand = row![logo, wordmark, version]
        .spacing(8.0)
        .align_y(Alignment::Center);

    // The active file's scope breadcrumbs, right after the wordmark and set
    // off by a "|" separator — see `breadcrumb_bar::title_crumbs`. `None`
    // with no active file (nothing to open a trail for) or an empty trail
    // (a landmark-less language, or the cursor sitting outside any scope).
    if let Some(editor) = state::active_editor(state) {
        if let Some(crumbs) = breadcrumb_bar::title_crumbs(editor, p) {
            brand = brand.push(pipe_sep(p)).push(crumbs);
        }
    }

    // A bare icon button, not the mockup's wide "RUN ANYTHING ⌘K" pill —
    // that pill duplicated the tab bar's own fixed search icon in spirit
    // (another big, centered entry point), so it's now a small glyph
    // sitting right next to Assist instead. A lightning bolt rather than
    // another magnifying glass — the tab bar already owns that glyph for
    // project search (`search_icon.rs`), and reusing it here for a
    // differently-scoped action (run/open anything) would blur the two.
    let run_icon = canvas(RunIcon { color: color(p.text_muted) })
        .width(Length::Fixed(15.0))
        .height(Length::Fixed(15.0));
    let palette_button = button(widgets::center_fill(run_icon))
        .width(Length::Fixed(32.0))
        .height(Length::Fixed(32.0))
        .padding(0.0)
        .on_press(Message::TogglePalette)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered { Some(color(p.surface_hover).into()) } else { None },
                border: Border {
                    color: if hovered { color(p.accent_solid) } else { color(p.border_hairline) },
                    width: 1.5,
                    radius: 3.0.into(),
                },
                ..button::Style::default()
            }
        });
    let palette_button = widgets::tooltip(palette_button, "Run anything \u{2318}K", p);

    // "On" here means "there's a live session at all" — includes it being
    // open as a full tab, not just `chat_mode != Closed` alone, since
    // opening as a tab actually sets `chat_mode` to `Closed` (the docked
    // panel and the tab view are mutually exclusive presentations of the
    // same session). See `state::chat_is_active`.
    let assist_on = crate::state::chat_is_active(state);
    let assist_button = button(widgets::center_v(
        row![
            widgets::dot(
                if assist_on {
                    color(p.accent_solid)
                } else {
                    color(p.text_muted)
                },
                7.0
            ),
            text("ASSIST")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(14.0)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    ))
    .padding([0.0, 14.0])
    .height(Length::Fixed(32.0))
    .on_press(Message::ChatToggle)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if assist_on {
                Some(color(p.accent_quiet).into())
            } else {
                None
            },
            text_color: if assist_on || hovered {
                color(p.accent_solid)
            } else {
                color(p.text_muted)
            },
            border: Border {
                color: if assist_on {
                    color(p.border_accent)
                } else {
                    color(p.border_hairline)
                },
                width: 1.5,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    });

    let bar = row![
        brand,
        iced::widget::Space::new().width(Length::Fill),
        palette_button,
        assist_button
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .padding([0.0, 12.0]);

    let bar_panel = container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(state.density.title_bar_h()))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            ..container::Style::default()
        });

    column![bar_panel, widgets::hline(color(p.border_hairline))].into()
}
