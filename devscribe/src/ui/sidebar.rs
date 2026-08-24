use devscribe_core::git::ChangeKind;
use devscribe_core::theme::{Palette, Rgba};
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::color::color;
use crate::fonts;
use crate::fs_tree::Node;
use crate::state::{self, ChangesEntry, Draft, DraftKind, Message, State};
use crate::widgets;

fn tint(c: Rgba, alpha: f32) -> Color {
    color(Rgba { a: alpha, ..c })
}

/// The single-letter badge + its color for a `ChangeKind`, shared by the
/// Changes panel rows and the file tree's per-file badges.
fn kind_letter(kind: ChangeKind, p: Palette) -> (&'static str, Rgba) {
    match kind {
        ChangeKind::Modified => ("M", p.status_warn),
        ChangeKind::Added => ("A", p.status_ok),
        ChangeKind::Untracked => ("U", p.status_info),
        ChangeKind::Deleted => ("D", p.status_danger),
    }
}

/// The `NEW`/`DIR`/`REN` badge glyph for a draft, matching the mockup's
/// `draftGlyph` convention.
fn draft_glyph(kind: DraftKind) -> &'static str {
    match kind {
        DraftKind::NewFile => "NEW",
        DraftKind::NewFolder => "DIR",
        DraftKind::Rename => "REN",
    }
}

fn draft_placeholder(kind: DraftKind) -> &'static str {
    match kind {
        DraftKind::NewFile => "filename.rs",
        DraftKind::NewFolder => "folder name",
        DraftKind::Rename => "new name",
    }
}

/// An inline text-input row for the open `Draft` — either inserted above a
/// directory's children (new file/folder) or standing in for an existing
/// node's own row (rename). Enter commits (`on_submit`). Escape *also*
/// cancels, but not reliably as the sole affordance: `iced`'s `text_input`
/// captures Escape itself while focused (to defocus, per its own internal
/// state) before the global `EscapePressed` listener — which only sees
/// events no widget captured — ever sees it, so a single Escape press while
/// typing just blurs the field rather than canceling the draft. The
/// explicit `cancel` button below is the reliable path.
fn draft_editor_row(draft: &Draft, depth: usize, row_h: f32, p: Palette) -> Element<'static, Message> {
    let input = text_input(draft_placeholder(draft.kind), &draft.text)
        .id(crate::state::draft_input_id())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(12.0))
        .padding([2.0, 6.0])
        .on_input(Message::DraftTextChanged)
        .on_submit(Message::CommitDraft)
        .style(move |_theme, _status| text_input::Style {
            background: color(p.bg_inset).into(),
            border: Border {
                color: color(p.line_accent),
                width: 1.5,
                radius: 2.0.into(),
            },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_primary),
            selection: tint(p.accent, 0.35),
        });

    let cancel = button(widgets::center_fill(text("\u{2715}").size(crate::text_scale::px(9.0))))
        .padding(0.0)
        .width(Length::Fixed(15.0))
        .height(Length::Fixed(15.0))
        .on_press(Message::CancelDraft)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(color(p.surface_raised).into())
                } else {
                    None
                },
                text_color: if hovered { color(p.text_primary) } else { color(p.text_muted) },
                ..button::Style::default()
            }
        });

    let content = row![
        widgets::lang_badge(draft_glyph(draft.kind), color(p.accent), tint(p.accent, 0.22)),
        input,
        text("\u{21b5}")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(9.0))
            .color(color(p.text_muted)),
        cancel,
    ]
    .spacing(7.0)
    .align_y(Alignment::Center);

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(row_h))
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 8.0 + depth as f32 * 14.0,
        })
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(tint(p.accent, 0.12).into()),
            ..container::Style::default()
        })
        .into()
}

/// `true` when `draft` is a new-file/new-folder draft targeting `dir` — the
/// signal to insert `draft_editor_row` at the top of that directory's
/// (or the root's) children.
fn draft_targets_dir(draft: Option<&Draft>, dir: &Path) -> bool {
    draft.is_some_and(|d| d.kind != DraftKind::Rename && d.dir == dir)
}

/// `true` when `draft` is a rename in progress for `path` — the signal to
/// replace that node's own row with `draft_editor_row` instead of its
/// normal button.
fn draft_renames(draft: Option<&Draft>, path: &Path) -> bool {
    draft.is_some_and(|d| d.kind == DraftKind::Rename && d.target.as_deref() == Some(path))
}

/// Read-only, per-`view()`-call lookups threaded through `node_view`'s
/// recursion — bundled into one struct rather than four separate params to
/// stay under clippy's argument-count lint.
struct TreeCtx<'a> {
    selected: Option<&'a Path>,
    collapsed_dirs: &'a HashSet<PathBuf>,
    changes: &'a HashMap<PathBuf, ChangeKind>,
    draft: Option<&'a Draft>,
}

fn node_view(node: &Node, depth: usize, p: Palette, row_h: f32, ctx: &TreeCtx<'_>) -> Element<'static, Message> {
    let draft = ctx.draft;
    match node {
        Node::Dir { name, path, children } => {
            if draft_renames(draft, path) {
                return draft_editor_row(draft.unwrap(), depth, row_h, p);
            }

            let is_collapsed = ctx.collapsed_dirs.contains(path);
            let glyph = if is_collapsed { "▸" } else { "▾" };

            let header = row![
                text(glyph).size(crate::text_scale::px(10.0)).color(color(p.text_muted)),
                text(name.clone())
                    .font(fonts::sans(Weight::Medium))
                    .size(crate::text_scale::px(13.0))
                    .color(color(p.text_primary)),
            ]
            .spacing(7.0)
            .align_y(Alignment::Center);

            let header_row = button(widgets::center_v(header))
                .width(Length::Fill)
                .height(Length::Fixed(row_h))
                .padding(Padding {
                    top: 0.0,
                    right: 8.0,
                    bottom: 0.0,
                    left: 8.0 + depth as f32 * 14.0,
                })
                .on_press(Message::ToggleDirCollapsed(path.clone()))
                .style(move |_theme, status| {
                    let hovered = status == button::Status::Hovered;
                    button::Style {
                        background: if hovered {
                            Some(tint(p.surface_raised, 0.6).into())
                        } else {
                            None
                        },
                        text_color: color(p.text_primary),
                        ..button::Style::default()
                    }
                });

            let header_row: Element<'static, Message> =
                mouse_area(header_row).on_right_press(Message::OpenTreeContext(Some(path.clone()))).into();

            if is_collapsed {
                header_row
            } else {
                let mut child_rows: Vec<Element<'static, Message>> = Vec::new();
                if draft_targets_dir(draft, path) {
                    child_rows.push(draft_editor_row(draft.unwrap(), depth + 1, row_h, p));
                }
                child_rows.extend(children.iter().map(|child| node_view(child, depth + 1, p, row_h, ctx)));

                column![header_row, column(child_rows)].into()
            }
        }
        Node::File { name, path, lang } => {
            if draft_renames(draft, path) {
                return draft_editor_row(draft.unwrap(), depth, row_h, p);
            }

            let is_selected = ctx.selected == Some(path.as_path());
            let (fg, bg) = lang.badge(p);
            let badge_label = lang.code(path);

            let mut contents = row![
                widgets::lang_badge(badge_label, fg, bg),
                text(name.clone())
                    .font(fonts::sans(Weight::Medium))
                    .size(crate::text_scale::px(13.0))
                    .color(color(p.text_primary))
                    .width(Length::Fill),
            ]
            .spacing(7.0)
            .align_y(Alignment::Center);
            if let Some(&kind) = ctx.changes.get(path) {
                let (letter, kind_color) = kind_letter(kind, p);
                contents = contents.push(widgets::lang_badge(letter, color(kind_color), tint(kind_color, 0.16)));
            }

            let row_content = container(contents)
                .padding(Padding {
                    top: 0.0,
                    right: 8.0,
                    bottom: 0.0,
                    left: 8.0 + depth as f32 * 14.0,
                })
                .height(Length::Fill)
                .align_y(Vertical::Center);

            let path = path.clone();
            let row_button = button(row_content)
                .width(Length::Fill)
                .height(Length::Fixed(row_h))
                .padding(0.0)
                .on_press(Message::SelectFile(path.clone()))
                .style(move |_theme, status| {
                    let hovered = status == button::Status::Hovered;
                    button::Style {
                        background: if is_selected {
                            Some(color(p.surface_raised).into())
                        } else if hovered {
                            Some(tint(p.surface_raised, 0.6).into())
                        } else {
                            None
                        },
                        text_color: color(p.text_primary),
                        ..button::Style::default()
                    }
                });

            mouse_area(row_button).on_right_press(Message::OpenTreeContext(Some(path))).into()
        }
    }
}

/// The branch name (+ `▲a ▼b` ahead/behind, when there's an upstream to
/// compare against) or, when `state.repo` is `None`, a "No repository" row
/// — replacing what used to be nothing at all here (the only place a
/// repo-less project said so was the footer and the status bar).
fn git_status_row(state: &State, p: Palette) -> Element<'static, Message> {
    let Some(repo) = state.repo.as_ref() else {
        return text("No repository")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted))
            .into();
    };

    let branch = repo.branch_name().unwrap_or_else(|| "detached HEAD".to_string());
    let mut contents = row![
        text(branch)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted))
            .width(Length::Fill),
    ]
    .spacing(6.0)
    .align_y(Alignment::Center);

    if let Some((ahead, behind)) = state.ahead_behind {
        contents = contents.push(
            text(format!("\u{25b2}{ahead} \u{25bc}{behind}"))
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(10.0))
                .color(color(p.text_muted)),
        );
    }

    contents.into()
}

fn project_switcher(state: &State, p: Palette) -> Element<'static, Message> {
    let root = &state.root;
    let (badge_fg, badge_bg) = (color(p.accent), tint(p.accent, 0.18));
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    let path_label = state::shorten_home(root);

    let header = button(
        row![
            widgets::lang_badge("PR", badge_fg, badge_bg),
            column![
                text(name)
                    .font(fonts::display(Weight::Semibold))
                    .size(crate::text_scale::px(15.0))
                    .color(color(p.text_primary)),
                text(path_label)
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(10.0))
                    .color(color(p.text_muted)),
            ]
            .spacing(1.0)
            .width(Length::Fill),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([6.0, 8.0])
    .on_press(Message::ToggleProjects)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_raised).into())
            } else {
                None
            },
            text_color: color(p.text_primary),
            ..button::Style::default()
        }
    });

    let git_row = container(git_status_row(state, p)).padding([0.0, 4.0]);

    let block = column![header, git_row].spacing(6.0).padding([12.0, 12.0]);

    container(block)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            border: Border {
                color: color(p.line_neutral),
                width: 0.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn project_row(row_data: &state::WelcomeRow, is_current: bool, p: Palette) -> Element<'static, Message> {
    let (fg, bg) = row_data.lang.badge(p);
    let right: Element<'static, Message> = if is_current {
        text("open")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.accent))
            .into()
    } else {
        text(row_data.last_opened_label.clone())
            .font(fonts::mono(Weight::Light))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted))
            .into()
    };

    let content = row![
        widgets::lang_badge(row_data.lang.code(&row_data.path), fg, bg),
        text(row_data.name.clone())
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_primary))
            .width(Length::Fill),
        right,
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);

    let path = row_data.path.clone();
    button(content)
        .width(Length::Fill)
        .padding([6.0, 8.0])
        .on_press(Message::RecentProjectPicked(path))
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if is_current {
                    Some(tint(p.accent, 0.16).into())
                } else if hovered {
                    Some(color(p.bg_panel).into())
                } else {
                    None
                },
                text_color: color(p.text_primary),
                border: Border { color: color(p.line_neutral), width: 0.0, radius: 2.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

fn projects_menu_row(glyph: &'static str, label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(
        row![
            text(glyph).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).width(Length::Fixed(18.0)),
            text(label).font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(12.0)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([6.0, 8.0])
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.bg_panel).into()) } else { None },
            text_color: if hovered { color(p.text_primary) } else { color(p.text_muted) },
            border: Border { color: color(p.line_neutral), width: 0.0, radius: 2.0.into() },
            ..button::Style::default()
        }
    })
    .into()
}

/// The project switcher header's dropdown: every recent project (current
/// one pinned first, marked "open"; picking another switches straight to
/// it), then "Open folder…" and "Close project" (back to the welcome
/// screen) — matches the mockup's `PROJECTS` dropdown. Same backdrop +
/// positioned-`container` pattern as `tab_bar::overflow_menu`, anchored
/// top-left under this header instead of top-right under the tab bar's
/// `⋯` button.
pub fn projects_menu(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.projects_open {
        return None;
    }

    let mut rows: Vec<Element<'static, Message>> = Vec::new();
    if !state.root.as_os_str().is_empty() {
        let current_name = state.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into());
        let current = state::WelcomeRow {
            path: state.root.clone(),
            name: current_name,
            lang: crate::recent_projects::detect_lang(&state.root),
            subtitle: String::new(),
            last_opened_label: String::new(),
        };
        rows.push(project_row(&current, true, p));
    }
    for row_data in state.welcome_rows.iter().filter(|r| r.path != state.root).take(5) {
        rows.push(project_row(row_data, false, p));
    }

    let menu = container(
        column![
            text("PROJECTS")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(9.0))
                .color(color(p.text_muted)),
            column(rows).spacing(2.0),
            widgets::hline(color(p.line_neutral)),
            projects_menu_row("+", "Open folder\u{2026}", Message::OpenFolderDialog, p),
            projects_menu_row("\u{21a9}", "Close project", Message::CloseProject, p),
        ]
        .spacing(6.0)
        .padding(6.0),
    )
    .width(Length::Fixed(260.0))
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border { color: color(p.line_neutral), width: 1.5, radius: 2.0.into() },
        ..container::Style::default()
    });

    let positioned = container(menu).width(Length::Fill).height(Length::Fill).padding(Padding {
        top: state.density.title_bar_h() + 78.0,
        right: 0.0,
        bottom: 0.0,
        left: 12.0,
    });

    let backdrop = mouse_area(container(Space::new().width(Length::Fill).height(Length::Fill)).width(Length::Fill).height(Length::Fill))
        .on_press(Message::ToggleProjects);

    Some(iced::widget::stack![backdrop, positioned].into())
}

fn changes_row(entry: &ChangesEntry, row_h: f32, p: Palette) -> Element<'static, Message> {
    let (letter, kind_color) = kind_letter(entry.kind, p);
    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut stats = row![].spacing(6.0).align_y(Alignment::Center);
    if entry.insertions > 0 {
        stats = stats.push(
            text(format!("+{}", entry.insertions))
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(10.0))
                .color(color(p.status_ok)),
        );
    }
    if entry.deletions > 0 {
        stats = stats.push(
            text(format!("\u{2212}{}", entry.deletions))
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(10.0))
                .color(color(p.status_danger)),
        );
    }

    let contents = row![
        text(name)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(12.5))
            .color(color(p.text_primary))
            .width(Length::Fill),
        stats,
        widgets::lang_badge(letter, color(kind_color), tint(kind_color, 0.16)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);

    let path = entry.path.clone();
    button(
        container(contents)
            .padding(Padding {
                top: 0.0,
                right: 8.0,
                bottom: 0.0,
                left: 22.0,
            })
            .height(Length::Fill)
            .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(row_h))
    .padding(0.0)
    .on_press(Message::OpenDiffFor(path))
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(tint(p.surface_raised, 0.6).into())
            } else {
                None
            },
            text_color: color(p.text_primary),
            ..button::Style::default()
        }
    })
    .into()
}

/// Shown in place of the (hidden-when-empty) CHANGES section for a git repo
/// with nothing changed — otherwise a clean tree just left a gap there with
/// no indication of *why* CHANGES wasn't showing.
fn clean_tree_row(p: Palette) -> Element<'static, Message> {
    row![
        widgets::dot(color(p.status_ok), 6.0),
        text("Working tree clean")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted)),
    ]
    .spacing(7.0)
    .align_y(Alignment::Center)
    .padding([8.0, 12.0])
    .into()
}

/// The "CHANGES [n]" toggle header, always `Shrink`-height — sized before
/// the tree gets whatever's left, so it never eats into the 50% cap the rows
/// list gets (see `changes_rows`). `None` when there's nothing to show — an
/// empty, always-visible "Changes 0" header would just be clutter on a
/// clean working tree.
fn changes_header(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if state.changed_files.is_empty() {
        return None;
    }

    let row_h = state.density.sidebar_row_h();
    let glyph = if state.changes_panel_open { "▾" } else { "▸" };
    let (total_ins, total_del) = state
        .changed_files
        .iter()
        .fold((0usize, 0usize), |(i, d), e| (i + e.insertions, d + e.deletions));

    Some(
        button(widgets::center_v(
            row![
                text(glyph).size(crate::text_scale::px(10.0)).color(color(p.text_muted)),
                text(format!("CHANGES [{}]", state.changed_files.len()))
                    .font(fonts::mono(Weight::Semibold))
                    .size(crate::text_scale::px(11.0))
                    .color(color(p.text_secondary)),
                text(format!("+{total_ins}"))
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(10.0))
                    .color(color(p.status_ok)),
                text(format!("\u{2212}{total_del}"))
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(10.0))
                    .color(color(p.status_danger)),
            ]
            .spacing(7.0)
            .align_y(Alignment::Center),
        ))
        .width(Length::Fill)
        .height(Length::Fixed(row_h))
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 8.0,
        })
        .on_press(Message::ToggleChangesPanel)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(tint(p.surface_raised, 0.6).into())
                } else {
                    None
                },
                ..button::Style::default()
            }
        })
        .into(),
    )
}

/// The scrollable list of changed-file rows, shown under the header when
/// `changes_panel_open`. Callers give this `Length::FillPortion(1)` against
/// the file tree's matching `FillPortion(1)` — an even split of whatever
/// vertical space is left after the header, which structurally caps this
/// list at half the sidebar (scrolling internally past that) rather than
/// letting a long changed-file list push the tree out of view.
fn changes_rows(state: &State, p: Palette) -> Element<'static, Message> {
    let row_h = state.density.sidebar_row_h();
    let rows: Vec<Element<'static, Message>> =
        state.changed_files.iter().map(|entry| changes_row(entry, row_h, p)).collect();
    scrollable(column(rows))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn footer(p: Palette) -> Element<'static, Message> {
    let settings = button(widgets::center_fill(text("⚙").size(crate::text_scale::px(14.0))))
        .width(Length::Fixed(26.0))
        .height(Length::Fixed(26.0))
        .padding(0.0)
        .on_press(Message::ToggleSettings)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(color(p.surface_raised).into())
                } else {
                    None
                },
                text_color: if hovered {
                    color(p.accent)
                } else {
                    color(p.text_muted)
                },
                ..button::Style::default()
            }
        });

    container(
        row![Space::new().width(Length::Fill), settings]
            .spacing(8.0)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8.0, 12.0])
    .into()
}

/// A small text-glyph header button (New file / New folder / Collapse all).
/// Deliberately doesn't use `widgets::lang_badge` — that widget bakes its
/// foreground color into the inner `text()` at construction time, which
/// would make the hover-color swap below inert (a `button::Style`'s
/// `text_color` only takes effect when the content doesn't already set its
/// own color).
fn header_icon_button(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(widgets::center_fill(
        text(label).font(fonts::mono(Weight::Bold)).size(crate::text_scale::px(10.0)),
    ))
    .width(Length::Fixed(24.0))
    .height(Length::Fixed(22.0))
    .padding(0.0)
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            text_color: if hovered { color(p.accent) } else { color(p.text_muted) },
            ..button::Style::default()
        }
    })
    .into()
}

/// The tree's "EXPLORER" header row: New file (⌘N) / New folder (⇧⌘N) /
/// Collapse all. Deliberately doesn't include the mockup's "More actions ⋯"
/// menu (New window/Open folder/Save as/…) — that needs a native
/// folder-picker dependency DevScribe doesn't have yet, tracked separately
/// (see the roadmap's Phase 9).
fn explorer_header(p: Palette) -> Element<'static, Message> {
    row![
        text("EXPLORER")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted))
            .width(Length::Fill),
        header_icon_button("+F", Message::BeginDraft(DraftKind::NewFile), p),
        header_icon_button("+D", Message::BeginDraft(DraftKind::NewFolder), p),
        header_icon_button("\u{21b1}", Message::CollapseAllDirs, p),
    ]
    .spacing(2.0)
    .align_y(Alignment::Center)
    .padding(Padding {
        top: 5.0,
        right: 8.0,
        bottom: 5.0,
        left: 12.0,
    })
    .into()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let selected: Option<&Path> = crate::state::active_editor(state).map(|e| e.path.as_path());

    let row_h = state.density.sidebar_row_h();
    // Empty (rather than gating the lookup in `node_view`) when the setting
    // is off, so the tree-rendering code doesn't need to know this toggle
    // exists at all.
    let changes: HashMap<PathBuf, ChangeKind> = if state.git_status_in_tree {
        state.changed_files.iter().map(|e| (e.path.clone(), e.kind)).collect()
    } else {
        HashMap::new()
    };
    let draft = state.draft.as_ref();
    let ctx = TreeCtx {
        selected,
        collapsed_dirs: &state.collapsed_dirs,
        changes: &changes,
        draft,
    };
    let mut tree_rows: Vec<Element<'static, Message>> = Vec::new();
    if draft_targets_dir(draft, &state.root) {
        tree_rows.push(draft_editor_row(draft.unwrap(), 0, row_h, p));
    }
    tree_rows.extend(state.tree.iter().map(|node| node_view(node, 0, p, row_h, &ctx)));

    // Right-click on empty tree space (not landing on any row's own
    // `mouse_area`) targets the project root — each row's `on_right_press`
    // captures the event first, so this only fires when the click missed
    // every row.
    let tree_view = mouse_area(
        scrollable(column(tree_rows).padding([8.0, 4.0]))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_right_press(Message::OpenTreeContext(None));

    // Everything between the two hairlines below the project switcher and
    // above the footer — the changes header/rows, the Explorer header, and
    // the file tree — shares one `Fill` region so the tree always gets
    // whatever the (optional, capped) changes list doesn't use.
    let mut middle = column![];
    if let Some(header) = changes_header(state, p) {
        middle = middle.push(header);
        if state.changes_panel_open {
            middle = middle
                .push(container(changes_rows(state, p)).height(Length::FillPortion(1)))
                .push(widgets::hline(color(p.line_neutral)));
        } else {
            middle = middle.push(widgets::hline(color(p.line_neutral)));
        }
    } else if state.repo.is_some() {
        // A git repo with nothing to show in CHANGES — say so, rather than
        // just leaving a gap where that section would otherwise be.
        middle = middle.push(clean_tree_row(p)).push(widgets::hline(color(p.line_neutral)));
    }
    middle = middle
        .push(explorer_header(p))
        .push(widgets::hline(color(p.line_neutral)))
        .push(container(tree_view).height(Length::Fill));

    let body = column![
        project_switcher(state, p),
        widgets::hline(color(p.line_neutral)),
        middle.height(Length::Fill),
        widgets::hline(color(p.line_neutral)),
        footer(p),
    ];

    container(body)
        .width(Length::Fixed(248.0))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_panel).into()),
            ..container::Style::default()
        })
        .into()
}
