use devscribe_core::lsp::DiagnosticSeverity;
use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Space};
use iced::{Alignment, Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{self, EditorDiagnostic, LspProgressEntry, Message, OpenTab, State};
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

fn syntax_label(lang: devscribe_core::syntax::Language) -> &'static str {
    lang.label()
}

/// One clickable status-bar segment — bare text, no border/background, just
/// a hover color shift, matching `problems_button`'s own chromeless style.
fn status_segment(label: String, on_press: Message, p: Palette) -> Element<'static, Message> {
    button(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    )
    .padding(0.0)
    .on_press(on_press)
    .style(move |_theme, status| button::Style {
        text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
        ..button::Style::default()
    })
    .into()
}

fn dot_sep(p: Palette) -> Element<'static, Message> {
    text("\u{b7}").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.text_muted)).into()
}

/// `Ln {line}, Col {col} · {EOL} · {encoding} · {Language}`, each segment
/// independently clickable — only shown once a file tab is active; there's
/// nothing to report otherwise (roadmap item 9). Encoding only ever reads
/// "UTF-8" (`Document` has no other encoding to track — see
/// `encoding_popover`'s own doc comment), but stays clickable, matching the
/// mockup's three-part indicator and making that limitation discoverable
/// rather than silently baked in.
fn cursor_info(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let editor = state::active_editor(state)?;
    let lang_label = editor.language.map(syntax_label).unwrap_or("Plain Text");
    let eol_label = editor.document.detect_eol().label();

    Some(
        row![
            status_segment(format!("Ln {}, Col {}", editor.cursor.line + 1, editor.cursor.col + 1), Message::OpenGoToLine, p),
            dot_sep(p),
            status_segment(eol_label.to_string(), Message::ToggleEolPicker, p),
            dot_sep(p),
            status_segment("UTF-8".to_string(), Message::ToggleEncodingInfo, p),
            dot_sep(p),
            status_segment(lang_label.to_string(), Message::ToggleLanguagePicker, p),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center)
        .into(),
    )
}

/// The status bar's git indicator (roadmap item 9) — branch name plus a
/// dirty-file count and ahead/behind, the same summary
/// `background_tasks_panel`'s own git row shows, just inline and one click
/// away from the sidebar's Changes panel instead of a click away from the
/// popover. `None` outside a git repo — nothing to summarize.
fn git_indicator(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let repo = state.repo.as_ref()?;
    let branch = repo.branch_name().unwrap_or_else(|| "\u{2014}".to_string());
    let dirty = state.changed_files.len();
    let label = match state.ahead_behind {
        Some((ahead, behind)) if ahead > 0 || behind > 0 => {
            format!("{branch} \u{2191}{ahead} \u{2193}{behind}{}", if dirty > 0 { format!(" \u{00b7} {dirty}\u{25cf}") } else { String::new() })
        }
        _ => {
            if dirty > 0 {
                format!("{branch} \u{00b7} {dirty}\u{25cf}")
            } else {
                branch
            }
        }
    };
    Some(status_segment(label, Message::ToggleChangesPanel, p))
}

/// `"{title} — {message} (N%)"`, dropping whichever halves `entry` didn't
/// have — a `Report` with no `message` keeps whatever the last one said (see
/// `LspEvent::Progress`'s own doc comment), so this only ever sees `entry`
/// as already-merged, never a bare, empty update.
fn progress_label(entry: &LspProgressEntry) -> String {
    match (entry.message.as_deref(), entry.percentage) {
        (Some(msg), Some(pct)) => format!("{} \u{2014} {msg} ({pct}%)", entry.title),
        (Some(msg), None) => format!("{} \u{2014} {msg}", entry.title),
        (None, Some(pct)) => format!("{} ({pct}%)", entry.title),
        (None, None) => entry.title.clone(),
    }
}

/// A small fixed-width fill bar for `percentage` — indeterminate progress
/// (`None`, e.g. rust-analyzer's "Fetching metadata" before it knows a
/// count) renders as an empty track rather than guessing a fraction.
fn mini_progress_bar(percentage: Option<u32>, p: Palette) -> Element<'static, Message> {
    const TRACK_W: f32 = 40.0;
    let fill_w = TRACK_W * (percentage.unwrap_or(0).min(100) as f32 / 100.0);
    container(
        container(Space::new().width(Length::Fixed(fill_w)).height(Length::Fill))
            .height(Length::Fill)
            .style(move |_theme| container::Style { background: Some(color(p.accent_solid).into()), ..container::Style::default() }),
    )
    .width(Length::Fixed(TRACK_W))
    .height(Length::Fixed(4.0))
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border { color: color(p.border_hairline), width: 1.0, radius: 2.0.into() },
        ..container::Style::default()
    })
    .into()
}

/// The status bar's language-server indicator — the dot/label
/// `LspStatus::describe` already gave, swapped for whichever `$/progress`
/// operation is running (plus a `mini_progress_bar`) while `lsp_progress`
/// isn't empty, e.g. rust-analyzer's "Indexing" right after a project
/// opens. Clicking it either way opens the "Background Tasks" popover
/// (visual-feedback pass, item 8).
fn lsp_indicator(state: &State, p: Palette) -> Element<'static, Message> {
    let (lsp_color, lsp_label) = state.lsp_status.describe(state::active_server_name(state), p);
    let mut content = row![widgets::dot(color(lsp_color), 6.0)].spacing(8.0).align_y(Alignment::Center);
    if let Some(entry) = state.lsp_progress.values().next() {
        content = content.push(
            text(progress_label(entry))
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
        );
        content = content.push(mini_progress_bar(entry.percentage, p));
    } else {
        content = content.push(
            text(lsp_label)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
        );
    }

    button(content)
        .padding(0.0)
        .on_press(Message::ToggleBackgroundTasks)
        .style(move |_theme, status| button::Style {
            text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
            ..button::Style::default()
        })
        .into()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let mut right_children: Vec<Element<'static, Message>> = Vec::new();
    if let Some(cursor) = cursor_info(state, p) {
        right_children.push(cursor);
    }
    if let Some(git) = git_indicator(state, p) {
        right_children.push(git);
    }
    right_children.push(lsp_indicator(state, p));
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

/// One row of the "Background Tasks" popover — a status dot, a label, and
/// an optional detail line underneath (a progress readout, a path, ...).
fn task_row(dot_color: devscribe_core::theme::Rgba, label: String, detail: Option<String>, p: Palette) -> Element<'static, Message> {
    let mut col = column![row![
        widgets::dot(color(dot_color), 6.0),
        text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.text_strong)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)]
    .spacing(2.0);
    if let Some(detail) = detail {
        col = col.push(
            text(detail)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_muted)),
        );
    }
    col.into()
}

/// The status bar's "Background Tasks" popover (`state.background_tasks_open`,
/// toggled by `lsp_indicator`) — a roll-up of everything running quietly in
/// the background that the rest of the UI otherwise gives no sign of: the
/// language server's own status (with every in-flight `$/progress`
/// operation, not just the one `lsp_indicator` has room to show inline),
/// the file watcher, and git. Positioned like `tab_bar::overflow_menu`'s own
/// popup — a full-window backdrop that closes it on any outside click,
/// plus a small anchored card — just anchored to the status bar's bottom-
/// right corner instead of the tab bar's.
pub fn background_tasks_panel(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.background_tasks_open {
        return None;
    }

    let (lsp_color, lsp_label) = state.lsp_status.describe(state::active_server_name(state), p);
    let mut rows: Vec<Element<'static, Message>> = vec![task_row(lsp_color, lsp_label, None, p)];
    for entry in state.lsp_progress.values() {
        rows.push(task_row(p.accent_solid, entry.title.clone(), entry.message.clone(), p));
    }

    rows.push(task_row(
        p.status_success,
        "File watcher".to_string(),
        Some(format!("Watching {} for changes", state.root.display())),
        p,
    ));

    let git_detail = state.repo.as_ref().map(|repo| {
        let branch = repo.branch_name().unwrap_or_else(|| "\u{2014}".to_string());
        let dirty = state.changed_files.len();
        match state.ahead_behind {
            Some((ahead, behind)) => format!("{branch} \u{00b7} {dirty} changed \u{00b7} \u{2191}{ahead} \u{2193}{behind}"),
            None => format!("{branch} \u{00b7} {dirty} changed"),
        }
    });
    rows.push(task_row(
        if state.repo.is_some() { p.status_success } else { p.text_muted },
        "Git".to_string(),
        Some(git_detail.unwrap_or_else(|| "No repository".to_string())),
        p,
    ));

    let header = row![
        widgets::micro("BACKGROUND TASKS", color(p.text_muted)),
        iced::widget::Space::new().width(Length::Fill),
        button(text("\u{2715}").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)))
            .padding(0.0)
            .on_press(Message::ToggleBackgroundTasks)
            .style(move |_theme, status| button::Style {
                text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
                ..button::Style::default()
            }),
    ]
    .align_y(Alignment::Center);

    let panel = container(
        column![header, widgets::hline(color(p.border_hairline)), column(rows).spacing(10.0)]
            .spacing(10.0)
            .padding(12.0),
    )
    .width(Length::Fixed(320.0))
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border { color: color(p.border_hairline), width: 1.5, radius: 4.0.into() },
        ..container::Style::default()
    });

    let positioned = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(Padding { top: 0.0, right: 8.0, bottom: state.density.status_bar_h() + 8.0, left: 0.0 });

    let backdrop = mouse_area(container(Space::new().width(Length::Fill).height(Length::Fill)).width(Length::Fill).height(Length::Fill))
        .on_press(Message::ToggleBackgroundTasks);

    Some(iced::widget::stack![backdrop, positioned].into())
}

/// A status-bar option row (EOL/language picker) — a label, a checkmark
/// when it's the current value, filling the row so the whole thing is
/// clickable rather than just the text.
fn picker_row(label: String, selected: bool, on_press: Message, p: Palette) -> Element<'static, Message> {
    button(
        row![
            text(if selected { "\u{2713}" } else { " " })
                .font(fonts::mono(Weight::Bold))
                .size(crate::text_scale::px(12.0))
                .color(color(p.accent_solid))
                .width(Length::Fixed(16.0)),
            text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.text_strong)),
        ]
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([5.0, 10.0])
    .on_press(on_press)
    .style(move |_theme, status| button::Style {
        background: if status == button::Status::Hovered { Some(color(p.surface_hover).into()) } else { None },
        ..button::Style::default()
    })
    .into()
}

/// Shared chrome for the status bar's small popovers (EOL/language/encoding)
/// — same anchor and backdrop-closes-on-outside-click behavior as
/// `background_tasks_panel`, just narrower, and closed by `close_message`
/// rather than always `ToggleBackgroundTasks`.
fn status_popover(state: &State, title: &'static str, close_message: Message, rows: Vec<Element<'static, Message>>, p: Palette) -> Element<'static, Message> {
    let header = row![
        widgets::micro(title, color(p.text_muted)),
        iced::widget::Space::new().width(Length::Fill),
        button(text("\u{2715}").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)))
            .padding(0.0)
            .on_press(close_message.clone())
            .style(move |_theme, status| button::Style {
                text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
                ..button::Style::default()
            }),
    ]
    .align_y(Alignment::Center)
    .padding([8.0, 10.0]);

    let panel = container(column![header, widgets::hline(color(p.border_hairline)), column(rows).padding([4.0, 0.0])])
        .width(Length::Fixed(200.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.border_hairline), width: 1.5, radius: 4.0.into() },
            ..container::Style::default()
        });

    let positioned = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(Padding { top: 0.0, right: 8.0, bottom: state.density.status_bar_h() + 8.0, left: 0.0 });

    let backdrop = mouse_area(container(Space::new().width(Length::Fill).height(Length::Fill)).width(Length::Fill).height(Length::Fill))
        .on_press(close_message);

    iced::widget::stack![backdrop, positioned].into()
}

/// The status bar's EOL picker — "Convert to LF"/"Convert to CRLF", the
/// current one checked. `None` with no active file (nothing to convert) or
/// while the picker isn't open.
pub fn eol_picker_view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.eol_picker_open {
        return None;
    }
    let editor = state::active_editor(state)?;
    let current = editor.document.detect_eol();
    let rows = vec![
        picker_row("LF".to_string(), current == devscribe_core::document::Eol::Lf, Message::ConvertEol(devscribe_core::document::Eol::Lf), p),
        picker_row("CRLF".to_string(), current == devscribe_core::document::Eol::CrLf, Message::ConvertEol(devscribe_core::document::Eol::CrLf), p),
    ];
    Some(status_popover(state, "LINE ENDINGS", Message::ToggleEolPicker, rows, p))
}

/// The status bar's language-mode picker — every `syntax::Language`, plus
/// "Auto-detect" (clears the override, back to whatever the extension
/// says) at the top. `None` with no active file, or while the picker isn't
/// open.
pub fn language_picker_view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.language_picker_open {
        return None;
    }
    let editor = state::active_editor(state)?;
    let current = editor.language;
    let mut rows = vec![picker_row("Plain Text".to_string(), current.is_none(), Message::SetEditorLanguage(None), p)];
    rows.extend(devscribe_core::syntax::Language::ALL.iter().map(|&lang| {
        picker_row(syntax_label(lang).to_string(), current == Some(lang), Message::SetEditorLanguage(Some(lang)), p)
    }));
    Some(status_popover(state, "LANGUAGE MODE", Message::ToggleLanguagePicker, rows, p))
}

/// The status bar's encoding indicator popover — informational only (see
/// `cursor_info`'s own doc comment on why there's nothing to actually pick).
pub fn encoding_info_view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.encoding_info_open {
        return None;
    }
    let rows = vec![
        text("UTF-8 \u{2014} the only encoding this editor reads or writes.")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.5))
            .color(color(p.text_muted))
            .into(),
    ];
    let header = row![
        widgets::micro("ENCODING", color(p.text_muted)),
        iced::widget::Space::new().width(Length::Fill),
        button(text("\u{2715}").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)))
            .padding(0.0)
            .on_press(Message::ToggleEncodingInfo)
            .style(move |_theme, status| button::Style {
                text_color: if status == button::Status::Hovered { color(p.text_strong) } else { color(p.text_muted) },
                ..button::Style::default()
            }),
    ]
    .align_y(Alignment::Center)
    .padding([8.0, 10.0]);

    let panel = container(column![header, widgets::hline(color(p.border_hairline)), container(column(rows)).padding([8.0, 10.0])])
        .width(Length::Fixed(240.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.border_hairline), width: 1.5, radius: 4.0.into() },
            ..container::Style::default()
        });

    let positioned = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(Padding { top: 0.0, right: 8.0, bottom: state.density.status_bar_h() + 8.0, left: 0.0 });

    let backdrop = mouse_area(container(Space::new().width(Length::Fill).height(Length::Fill)).width(Length::Fill).height(Length::Fill))
        .on_press(Message::ToggleEncodingInfo);

    Some(iced::widget::stack![backdrop, positioned].into())
}
