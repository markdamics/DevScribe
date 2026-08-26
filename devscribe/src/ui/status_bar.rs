use devscribe_core::lsp::DiagnosticSeverity;
use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{container, row, text};
use iced::{Alignment, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, Message, OpenTab, State};
use crate::widgets;

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    // Summed across every open file, not just the active one — diagnostics
    // are a workspace-wide concern, and files stay open (and keep getting
    // `didChange`d) even while another tab is focused.
    let diagnostics = state.open_tabs.iter().filter_map(|t| match t {
        OpenTab::File(editor) => Some(&editor.diagnostics),
        _ => None,
    });
    let errors = diagnostics
        .clone()
        .flatten()
        .filter(|d| d.severity == DiagnosticSeverity::ERROR)
        .count();
    let warnings = diagnostics
        .flatten()
        .filter(|d| d.severity == DiagnosticSeverity::WARNING)
        .count();
    let total = errors + warnings;

    let problems_color = if errors > 0 {
        p.status_danger
    } else if warnings > 0 {
        p.status_warning
    } else {
        p.status_success
    };
    let problems_label = if total == 0 {
        "NO PROBLEMS".to_string()
    } else {
        format!("{total} PROBLEM{}", if total == 1 { "" } else { "S" })
    };

    let problems = row![
        widgets::dot(color(problems_color), 6.0),
        text(problems_label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);

    let (lsp_color, lsp_label) = state.lsp_status.describe(state::active_server_name(state), p);
    let lsp = row![
        widgets::dot(color(lsp_color), 6.0),
        text(lsp_label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);

    let bar = row![
        problems,
        iced::widget::Space::new().width(Length::Fill),
        lsp,
    ]
    .spacing(16.0)
    .align_y(Alignment::Center)
    .padding([0.0, 16.0]);

    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(state.density.status_bar_h()))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            ..container::Style::default()
        })
        .into()
}
