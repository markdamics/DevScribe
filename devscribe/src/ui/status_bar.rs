use devscribe_core::lsp::DiagnosticSeverity;
use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, EditorDiagnostic, Message, OpenTab, State};
use crate::widgets;
use std::path::{Path, PathBuf};

/// Every open file's diagnostics, workspace-wide (not just the active tab —
/// diagnostics are a workspace-wide concern, and files stay open, and keep
/// getting `didChange`d, even while another tab is focused).
fn all_diagnostics(state: &State) -> impl Iterator<Item = (&Path, &EditorDiagnostic)> + Clone {
    state.open_tabs.iter().flat_map(|t| match t {
        OpenTab::File(editor) => {
            Some(editor.diagnostics.iter().map(|d| (editor.path.as_path(), d)))
        }
        _ => None,
    }).flatten()
}

fn problems_button(state: &State, p: Palette) -> Element<'static, Message> {
    let diagnostics = all_diagnostics(state);
    let errors = diagnostics.clone().filter(|(_, d)| d.severity == DiagnosticSeverity::ERROR).count();
    let warnings = diagnostics.filter(|(_, d)| d.severity == DiagnosticSeverity::WARNING).count();
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

    button(
        row![
            widgets::dot(color(problems_color), 6.0),
            text(problems_label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .padding(0.0)
    .on_press(Message::ToggleProblemsPanel)
    .style(move |_theme, status| button::Style {
        text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
        ..button::Style::default()
    })
    .into()
}

/// `Ln {line}, Col {col} · UTF-8 · {Language}` — only shown once a file
/// tab is active; there's no cursor/encoding/language to report otherwise.
/// Encoding is always UTF-8 (`Document` has no other encoding to track), so
/// unlike the other two this one never varies — it's shown anyway to match
/// the mockup's three-part indicator.
fn cursor_info(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let editor = state::active_editor(state)?;
    let lang_label = editor.language.map(syntax_label).unwrap_or("Plain Text");
    let label = format!(
        "Ln {}, Col {} \u{b7} UTF-8 \u{b7} {}",
        editor.cursor.line + 1,
        editor.cursor.col + 1,
        lang_label
    );
    Some(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted))
            .into(),
    )
}

fn syntax_label(lang: devscribe_core::syntax::Language) -> &'static str {
    lang.label()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
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

    let mut right_children: Vec<Element<'static, Message>> = Vec::new();
    if let Some(cursor) = cursor_info(state, p) {
        right_children.push(cursor);
    }
    right_children.push(lsp.into());
    let right = row(right_children).spacing(16.0).align_y(Alignment::Center);

    let bar = row![
        problems_button(state, p),
        iced::widget::Space::new().width(Length::Fill),
        right,
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

pub(crate) const PROBLEMS_PANEL_H: f32 = 196.0;

fn diagnostic_row(path: &Path, root: &Path, d: &EditorDiagnostic, p: Palette) -> Element<'static, Message> {
    let severity_color = match d.severity {
        DiagnosticSeverity::ERROR => p.status_danger,
        DiagnosticSeverity::WARNING => p.status_warning,
        _ => p.status_info,
    };
    let location = format!(
        "{}:{}:{}",
        path.strip_prefix(root).unwrap_or(path).display(),
        d.start.line + 1,
        d.start.col + 1,
    );

    button(
        row![
            widgets::dot(color(severity_color), 6.0),
            text(location)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
            text(d.message.clone())
                .font(fonts::mono(Weight::Normal))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_body)),
        ]
        .spacing(10.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([4.0, 16.0])
    .on_press(Message::OpenDiagnosticAt(path.to_path_buf(), d.start))
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.surface_raised).into()) } else { None },
            ..button::Style::default()
        }
    })
    .into()
}

/// The Problems dock panel: every diagnostic across all open files, listed
/// as clickable `file:line:col` rows. Shown between the editor content and
/// the status bar (see `shell.rs`) while `state.problems_panel_open`.
pub fn dock_panel(state: &State, p: Palette) -> Element<'static, Message> {
    let diagnostics: Vec<(PathBuf, EditorDiagnostic)> = all_diagnostics(state)
        .map(|(path, d)| (path.to_path_buf(), d.clone()))
        .collect();
    let errors = diagnostics.iter().filter(|(_, d)| d.severity == DiagnosticSeverity::ERROR).count();
    let warnings = diagnostics.iter().filter(|(_, d)| d.severity == DiagnosticSeverity::WARNING).count();

    let header = row![
        text(format!("Problems \u{2014} {errors} error{} \u{b7} {warnings} warning{}", if errors == 1 { "" } else { "s" }, if warnings == 1 { "" } else { "s" }))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.accent_solid)),
        iced::widget::Space::new().width(Length::Fill),
        button(text("\u{2715}").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)))
            .padding(0.0)
            .on_press(Message::ToggleProblemsPanel)
            .style(move |_theme, status| button::Style {
                text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
                ..button::Style::default()
            }),
    ]
    .align_y(Alignment::Center)
    .padding([8.0, 16.0]);

    let root = state.root.clone();
    let list: Element<'static, Message> = if diagnostics.is_empty() {
        widgets::placeholder("No problems in any open file", p)
    } else {
        let rows: Vec<Element<'static, Message>> =
            diagnostics.iter().map(|(path, d)| diagnostic_row(path, &root, d, p)).collect();
        scrollable(column(rows)).width(Length::Fill).height(Length::Fill).into()
    };

    container(column![header, widgets::hline(color(p.border_hairline)), list].width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fixed(PROBLEMS_PANEL_H))
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}
