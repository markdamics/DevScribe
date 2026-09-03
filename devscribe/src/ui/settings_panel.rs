//! The settings overlay: a left-nav modal (`Explorer`/`Editor`/`Toolchains`/
//! `Shortcuts`/`About`) matching the mockup's 760x520 shell and its nav
//! order. Rendered as a `stack!` layer over the shell, same backdrop-modal
//! mechanism as `command_palette`.
//!
//! Two deliberate departures from `DevScribe.dc.html`:
//!
//! - **Toolchains** shows live state — the active language's real
//!   `LspStatus`, and binary-present/absent for the rest — where the mockup
//!   shows four fixed `READY`/`FETCHING` sample rows. The mockup's states
//!   aren't a model DevScribe has; these are.
//! - **Shortcuts** lists every wired keybinding, which is a superset of the
//!   mockup's ten rows (it also has the tab, find and reveal bindings).
//!
//! Switch rows otherwise carry the mockup's own title + description copy
//! verbatim — see `toggle_row`.
use devscribe_core::theme::{Accent, Palette, ThemeMode};
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Space};
use iced::{Alignment, Border, Color, Element, Length};

use devscribe_core::lsp::LspLanguage;

use crate::color::color;
use crate::density::Density;
use crate::fonts;
use crate::server_install;
use crate::state::{
    self, Message, SettingsCategory, State, EDITOR_FONT_SIZE_MAX, EDITOR_FONT_SIZE_MIN,
    EDITOR_FONT_SIZE_STEP, TAB_SIZE_MAX, TAB_SIZE_MIN, TAB_SIZE_STEP, UI_FONT_SCALE_MAX,
    UI_FONT_SCALE_MIN, UI_FONT_SCALE_STEP,
};
use crate::widgets;

/// A stepper's `-`/`+` button. Always has an `on_press` — even sitting at
/// the min/max, it re-sends the same clamped value rather than going
/// disabled. A disabled `on_press_maybe(None)` button doesn't capture the
/// click, so it falls through to whatever is stacked underneath; here
/// that's the settings modal's backdrop `mouse_area`, which would close the
/// whole panel on a click that was only ever meant to hit a floor/ceiling.
fn stepper_button(label: &'static str, p: Palette, message: Message, enabled: bool) -> Element<'static, Message> {
    button(widgets::center_fill(
        text(label)
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(15.0))
            .color(if enabled { color(p.text_strong) } else { color(p.text_muted) }),
    ))
    .width(Length::Fixed(28.0))
    .height(Length::Fixed(28.0))
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
            border: Border {
                color: color(p.border_hairline),
                width: 1.5,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

fn stepper_row(
    value_label: String,
    dec_message: Message,
    inc_message: Message,
    dec_enabled: bool,
    inc_enabled: bool,
    p: Palette,
) -> Element<'static, Message> {
    row![
        stepper_button("\u{2212}", p, dec_message, dec_enabled),
        container(
            text(value_label)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_strong)),
        )
        .width(Length::Fixed(48.0))
        .align_x(Alignment::Center),
        stepper_button("+", p, inc_message, inc_enabled),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .into()
}

fn font_size_row(state: &State, p: Palette) -> Element<'static, Message> {
    let size = state.editor_font_size;
    stepper_row(
        format!("{size:.0}px"),
        Message::SetEditorFontSize((size - EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX)),
        Message::SetEditorFontSize((size + EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX)),
        size > EDITOR_FONT_SIZE_MIN + f32::EPSILON,
        size < EDITOR_FONT_SIZE_MAX - f32::EPSILON,
        p,
    )
}

fn tab_size_row(state: &State, p: Palette) -> Element<'static, Message> {
    let size = state.tab_size;
    stepper_row(
        format!("{size}"),
        Message::SetTabSize(size.saturating_sub(TAB_SIZE_STEP).clamp(TAB_SIZE_MIN, TAB_SIZE_MAX)),
        Message::SetTabSize(size.saturating_add(TAB_SIZE_STEP).clamp(TAB_SIZE_MIN, TAB_SIZE_MAX)),
        size > TAB_SIZE_MIN,
        size < TAB_SIZE_MAX,
        p,
    )
}

fn ui_scale_row(state: &State, p: Palette) -> Element<'static, Message> {
    let scale = state.ui_font_scale;
    stepper_row(
        format!("{:.0}%", scale * 100.0),
        Message::SetUiFontScale((scale - UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX)),
        Message::SetUiFontScale((scale + UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX)),
        scale > UI_FONT_SCALE_MIN + f32::EPSILON,
        scale < UI_FONT_SCALE_MAX - f32::EPSILON,
        p,
    )
}

/// Dark/Light toggle (2 buttons) — replaces the old ten-named-theme grid.
fn theme_mode_row(state: &State, p: Palette) -> Element<'static, Message> {
    let buttons: Vec<Element<'static, Message>> = ThemeMode::ALL
        .into_iter()
        .map(|mode| {
            let active = state.theme_mode == mode;
            button(
                text(mode.label())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(13.0))
                    .color(if active { color(p.accent_solid) } else { color(p.text_body) }),
            )
            .width(Length::Fill)
            .padding([6.0, 10.0])
            .on_press(Message::SetThemeMode(mode))
            .style(move |_theme, status| {
                let hovered = status == button::Status::Hovered;
                button::Style {
                    background: if active {
                        Some(color(p.accent_quiet).into())
                    } else if hovered {
                        Some(color(p.surface_hover).into())
                    } else {
                        None
                    },
                    border: Border {
                        color: if active { color(p.border_accent) } else { color(p.border_hairline) },
                        width: 1.5,
                        radius: 3.0.into(),
                    },
                    ..button::Style::default()
                }
            })
            .into()
        })
        .collect();

    row(buttons).spacing(8.0).into()
}

/// Accent swatch row (6 buttons, one per `Accent` variant) — Maho's
/// dark/light-orthogonal replacement for the old fixed named-theme colors.
fn accent_row(state: &State, p: Palette) -> Element<'static, Message> {
    let rows: Vec<Element<'static, Message>> = Accent::ALL
        .chunks(3)
        .map(|chunk| {
            let buttons: Vec<Element<'static, Message>> = chunk
                .iter()
                .map(|&accent| {
                    let active = state.accent == accent;
                    let swatch = devscribe_core::theme::palette(state.theme_mode, accent).accent_solid;
                    button(
                        row![
                            widgets::dot(color(swatch), 6.0),
                            text(accent.label())
                                .font(fonts::mono(Weight::Medium))
                                .size(crate::text_scale::px(13.0))
                                .color(if active { color(p.accent_solid) } else { color(p.text_body) }),
                        ]
                        .spacing(8.0)
                        .align_y(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .padding([6.0, 10.0])
                    .on_press(Message::SetAccent(accent))
                    .style(move |_theme, status| {
                        let hovered = status == button::Status::Hovered;
                        button::Style {
                            background: if active {
                                Some(color(p.accent_quiet).into())
                            } else if hovered {
                                Some(color(p.surface_hover).into())
                            } else {
                                None
                            },
                            border: Border {
                                color: if active { color(p.border_accent) } else { color(p.border_hairline) },
                                width: 1.5,
                                radius: 3.0.into(),
                            },
                            ..button::Style::default()
                        }
                    })
                    .into()
                })
                .collect();
            row(buttons).spacing(8.0).into()
        })
        .collect();

    column(rows).spacing(8.0).into()
}

fn section_label(label: &'static str, p: Palette) -> Element<'static, Message> {
    text(label)
        .font(fonts::mono(Weight::Bold))
        .size(crate::text_scale::px(13.0))
        .color(color(p.text_muted))
        .into()
}

fn density_row(state: &State, p: Palette) -> Element<'static, Message> {
    let options: Vec<Element<'static, Message>> = Density::ALL
        .into_iter()
        .map(|density| {
            let active = state.density == density;
            button(
                text(density.label())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(15.0))
                    .color(if active { color(p.accent_solid) } else { color(p.text_body) }),
            )
            .padding([6.0, 14.0])
            .on_press(Message::SetDensity(density))
            .style(move |_theme, status| {
                let hovered = status == button::Status::Hovered;
                button::Style {
                    background: if active {
                        Some(color(p.accent_quiet).into())
                    } else if hovered {
                        Some(color(p.surface_hover).into())
                    } else {
                        None
                    },
                    border: Border {
                        color: if active { color(p.border_accent) } else { color(p.border_hairline) },
                        width: 1.5,
                        radius: 3.0.into(),
                    },
                    ..button::Style::default()
                }
            })
            .into()
        })
        .collect();

    row(options).spacing(8.0).into()
}

/// One settings switch, as the mockup lays it out: a `--text-body-md`
/// title over a `--text-body-sm`/`--text-muted` line saying what the switch
/// actually does, with the control on the right. The app used to show a
/// single label carrying the *description's* wording ("Show inline problem
/// hints"), which lost the mockup's title ("Inline problem lens") and with
/// it the name the rest of the UI uses for the same feature.
fn toggle_row(
    title: &'static str,
    description: &'static str,
    enabled: bool,
    message: Message,
    p: Palette,
) -> Element<'static, Message> {
    button(
        row![
            widgets::dot(if enabled { color(p.accent_solid) } else { color(p.text_muted) }, 6.0),
            column![
                text(title)
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(15.0))
                    .color(color(p.text_strong)),
                text(description)
                    .font(fonts::mono(Weight::Normal))
                    .size(crate::text_scale::px(13.0))
                    .color(color(p.text_muted)),
            ]
            .spacing(2.0),
            iced::widget::Space::new().width(Length::Fill),
            text(if enabled { "ON" } else { "OFF" })
                .font(fonts::mono(Weight::Bold))
                .size(crate::text_scale::px(13.0))
                .color(if enabled { color(p.status_success) } else { color(p.text_muted) }),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8.0, 10.0])
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            border: Border {
                color: color(p.border_hairline),
                width: 1.5,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

fn category_nav_row(category: SettingsCategory, active: bool, p: Palette) -> Element<'static, Message> {
    button(
        text(category.label())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(if active { color(p.text_strong) } else { color(p.text_body) })
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding([8.0, 12.0])
    .on_press(Message::SetSettingsCategory(category))
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if active {
                Some(color(p.surface_raised).into())
            } else if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            border: Border {
                color: if active { color(p.border_accent) } else { Color::TRANSPARENT },
                width: 1.5,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

fn category_nav(state: &State, p: Palette) -> Element<'static, Message> {
    let rows: Vec<Element<'static, Message>> = SettingsCategory::ALL
        .into_iter()
        .map(|category| category_nav_row(category, state.settings_category == category, p))
        .collect();

    container(column(rows).spacing(2.0).padding(8.0))
        .width(Length::Fixed(168.0))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        })
        .into()
}

/// Everything the mockup's "Explorer" category holds: appearance settings
/// that aren't specific to the code editor itself (theme, chrome density,
/// UI text scale) plus the two Phase-2/3 toggles the mockup places here too
/// — "Git status in tree" and inline problem hints, the latter moved from
/// DevScribe's old flat panel where it sat under an "Editor" label that
/// didn't match the mockup's own categorization.
fn explorer_content(state: &State, p: Palette) -> Element<'static, Message> {
    column![
        column![section_label("THEME", p), theme_mode_row(state, p)].spacing(8.0),
        column![section_label("ACCENT", p), accent_row(state, p)].spacing(8.0),
        column![section_label("ROW DENSITY", p), density_row(state, p)].spacing(8.0),
        column![section_label("UI TEXT SIZE", p), ui_scale_row(state, p)].spacing(8.0),
        column![
            section_label("FILES", p),
            toggle_row(
                "Show hidden files",
                "Dotfiles and ignored paths in the tree",
                state.show_hidden_files,
                Message::ToggleShowHiddenFiles,
                p,
            ),
        ]
        .spacing(8.0),
        column![
            section_label("GIT", p),
            toggle_row(
                "Git status in tree",
                "Dirty dots, staged marks, ahead/behind counts",
                state.git_status_in_tree,
                Message::ToggleGitStatusInTree,
                p,
            ),
        ]
        .spacing(8.0),
        column![
            section_label("DIAGNOSTICS", p),
            toggle_row(
                "Inline problem lens",
                "Show diagnostics at the end of the offending line",
                state.problem_lens_enabled,
                Message::ToggleProblemLens,
                p,
            ),
        ]
        .spacing(8.0),
    ]
    .spacing(24.0)
    .into()
}

/// Keeps DevScribe's existing font-size stepper (a real, working control)
/// even though the mockup's refreshed Editor category dropped it in favor
/// of just the two toggles below — a deliberate divergence, confirmed
/// rather than assumed, since dropping it would make editor font size
/// unreachable (no other control sets it). "Inline problem lens" duplicates
/// the Explorer toggle of the same name (both drive the same
/// `state.problem_lens_enabled`) — the mockup shows it in both places, and
/// there's no reason to disallow that here.
fn editor_content(state: &State, p: Palette) -> Element<'static, Message> {
    column![
        column![section_label("FONT SIZE", p), font_size_row(state, p)].spacing(8.0),
        column![section_label("TAB SIZE", p), tab_size_row(state, p)].spacing(8.0),
        column![
            section_label("GUTTER", p),
            toggle_row(
                "Show line numbers",
                "Line number digits in the editor gutter",
                state.show_line_numbers,
                Message::ToggleShowLineNumbers,
                p,
            ),
        ]
        .spacing(8.0),
        column![
            section_label("WRAPPING", p),
            toggle_row(
                "Word wrap",
                "Wrap long lines to the pane width instead of scrolling sideways",
                state.word_wrap,
                Message::ToggleWordWrap,
                p,
            ),
        ]
        .spacing(8.0),
        column![
            section_label("DIAGNOSTICS", p),
            toggle_row(
                "Inline problem lens",
                "Show diagnostics at the end of the offending line",
                state.problem_lens_enabled,
                Message::ToggleProblemLens,
                p,
            ),
        ]
        .spacing(8.0),
        column![
            section_label("SAVING", p),
            toggle_row(
                "Save on focus loss",
                "Write buffers when the window is backgrounded",
                state.save_on_focus_loss,
                Message::ToggleSaveOnFocusLoss,
                p
            ),
        ]
        .spacing(8.0),
    ]
    .spacing(24.0)
    .into()
}

/// A non-interactive status row: a colored dot, a name, and a status label
/// pushed to the right — shared by Toolchains' language-server row and
/// About's matching summary row, both driven by the same
/// `LspStatus::describe`.
fn status_row(name: &'static str, status_color: Color, status_label: String, p: Palette) -> Element<'static, Message> {
    container(
        row![
            widgets::dot(status_color, 6.0),
            text(name)
                .font(fonts::mono(Weight::Semibold))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_strong))
                .width(Length::Fill),
            text(status_label)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(status_color),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8.0, 10.0])
    .style(move |_theme| container::Style {
        border: Border {
            color: color(p.border_hairline),
            width: 1.5,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// One row per supported language server — shows live status for the active
/// server and "INSTALLED" / "NOT INSTALLED" for all others.
fn toolchains_content(state: &State, p: Palette) -> Element<'static, Message> {
    let active_lang = state::active_lsp_language(state);

    let all_langs = [
        (LspLanguage::Rust,       "RUST-ANALYZER",              "rust"),
        (LspLanguage::Java,       "JDTLS",                      "java"),
        (LspLanguage::Python,     "PYRIGHT",                    "python"),
        (LspLanguage::TypeScript, "TYPESCRIPT-LANGUAGE-SERVER", "ts/js"),
        (LspLanguage::Cpp,        "CLANGD",                     "c/c++"),
    ];

    let rows: Vec<Element<'static, Message>> = all_langs
        .iter()
        .map(|(lang, display, _ext)| {
            let (dot_color, label) = if active_lang == Some(*lang) {
                // Use the live status for the currently active language.
                state.lsp_status.describe(display, p)
            } else {
                // For inactive languages, show whether the binary is available.
                let spec = server_install::spec_for(*lang);
                if server_install::resolve_binary(&spec).is_some() {
                    (p.status_success, format!("{display} installed"))
                } else {
                    (p.text_muted, format!("{display} not installed"))
                }
            };
            status_row(display, color(dot_color), label, p)
        })
        .collect();

    let mut col = column![
        section_label("LANGUAGE SERVERS", p),
    ]
    .spacing(8.0);
    for row in rows {
        col = col.push(row);
    }
    col = col.push(toggle_row(
        "Install toolchains automatically",
        "Fetch syntax and language servers on first open",
        state.lsp_enabled,
        Message::ToggleLspEnabled,
        p,
    ));

    col = col.push(section_label("AI COMPLETIONS", p));
    if state.copilot_inline_enabled {
        let (dot_color, label) = state.copilot_completion_status.describe(p);
        col = col.push(status_row("GITHUB COPILOT", color(dot_color), label, p));
    }
    col.push(toggle_row(
        "Inline suggestions",
        "GitHub Copilot ghost-text completions as you type — requires copilot-language-server on PATH and a signed-in account",
        state.copilot_inline_enabled,
        Message::ToggleCopilotInline,
        p,
    ))
    .into()
}

fn shortcut_row(label: &'static str, keys: &'static str, p: Palette) -> Element<'static, Message> {
    row![
        text(label)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body))
            .width(Length::Fill),
        text(keys)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_strong)),
    ]
    .align_y(Alignment::Center)
    .padding([7.0, 0.0])
    .into()
}

/// A static reference table of every keybinding actually wired in
/// `state::global_keys` (plus the two right-click-only ones at the bottom,
/// clearly separated) — not the mockup's 10-row list verbatim. The mockup
/// includes "New window"/"Open folder"/"Save as", none of which have a real
/// handler in DevScribe (no multi-window support, no folder-picker
/// dependency yet, no save-as flow), so listing them here would document
/// shortcuts that don't work. Conversely this table includes several real
/// shortcuts the mockup's illustrative list didn't happen to mention
/// (Close tab, Close others, Reopen closed tab, Reveal in tree, Escape) —
/// a keyboard reference is more useful complete than mockup-literal.
fn shortcuts_content(p: Palette) -> Element<'static, Message> {
    column![
        column![
            section_label("GENERAL", p),
            column![
                shortcut_row("Command palette", "\u{2318}K", p),
                shortcut_row("Keyboard shortcuts", "\u{2318}/", p),
                shortcut_row("Escape / close", "esc", p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("AI CHAT ASSIST", p),
            column![
                shortcut_row("Toggle chat", "\u{2318}I", p),
                shortcut_row("Focus chat", "\u{21e7}\u{2318}I", p),
                shortcut_row("New session", "\u{2325}\u{2318}I", p),
                shortcut_row("Open actions menu", "\u{21e7}\u{2318}U", p),
                shortcut_row("Attach file", "\u{2318}U", p),
                shortcut_row("Send message", "Enter", p),
                shortcut_row("New line", "\u{21e7}Enter", p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("FILES", p),
            column![
                shortcut_row("New file", "\u{2318}N", p),
                shortcut_row("New folder", "\u{21e7}\u{2318}N", p),
                shortcut_row("Save", "\u{2318}S", p),
                shortcut_row("Copy path", "\u{2325}\u{2318}C", p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("TABS & SEARCH", p),
            column![
                shortcut_row("Close tab", "\u{2318}W", p),
                shortcut_row("Close other tabs", "\u{2325}\u{2318}W", p),
                shortcut_row("Reopen closed tab", "\u{21e7}\u{2318}T", p),
                shortcut_row("Reveal in tree", "\u{21e7}\u{2318}E", p),
                shortcut_row("Find in file", "\u{2318}F", p),
                shortcut_row("Find in project", "\u{21e7}\u{2318}F", p),
                shortcut_row("Working tree diff", "\u{21e7}\u{2318}D", p),
                shortcut_row("Go to line", "\u{2318}G", p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("EDITING", p),
            column![
                shortcut_row("Indent / block indent", "Tab", p),
                shortcut_row("Dedent / block dedent", "\u{21e7}Tab", p),
                shortcut_row("Toggle line comment", "\u{2318}/", p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("NAVIGATION", p),
            column![
                shortcut_row("Go to definition", "\u{2318}Click / F12", p),
                shortcut_row("Find all references", "\u{21e7}F12", p),
            ]
        ]
        .spacing(8.0),
    ]
    .spacing(24.0)
    .into()
}

/// A real BUILD/PLATFORM/TOOLCHAIN table — not the mockup's BUILD/RUNTIME/
/// TOOLCHAINS/LICENSE. `RUNTIME` (the compiling rustc's version) and
/// `LICENSE` are dropped rather than faked: neither is tracked anywhere in
/// this workspace (no `build.rs` capturing `rustc --version`, no `license`
/// field in `Cargo.toml`), and inventing values for either would be exactly
/// the kind of fabricated content this doc keeps calling out.
fn about_content(state: &State, p: Palette) -> Element<'static, Message> {
    let (status_color, status_label) = state.lsp_status.describe(state::active_server_name(state), p);
    let banner = column![
        text("DEVSCRIBE")
            .font(fonts::display(Weight::ExtraBold))
            .size(crate::text_scale::px(22.0))
            .color(color(p.text_strong)),
        text(concat!("v", env!("CARGO_PKG_VERSION")))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    ]
    .spacing(2.0);

    column![
        banner,
        column![
            section_label("BUILD", p),
            status_row("VERSION", color(p.text_strong), env!("CARGO_PKG_VERSION").to_string(), p),
            status_row(
                "PLATFORM",
                color(p.text_strong),
                format!("{} // {}", std::env::consts::OS, std::env::consts::ARCH),
                p
            ),
        ]
        .spacing(8.0),
        column![
            section_label("TOOLCHAIN", p),
            status_row("RUST-ANALYZER", color(status_color), status_label, p),
        ]
        .spacing(8.0),
    ]
    .spacing(24.0)
    .into()
}

fn category_content(state: &State, p: Palette) -> Element<'static, Message> {
    let content = match state.settings_category {
        SettingsCategory::Explorer => explorer_content(state, p),
        SettingsCategory::Editor => editor_content(state, p),
        SettingsCategory::Toolchains => toolchains_content(state, p),
        SettingsCategory::Shortcuts => shortcuts_content(p),
        SettingsCategory::About => about_content(state, p),
    };
    scrollable(container(content).width(Length::Fill).padding(20.0))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn view(state: &State) -> Option<Element<'static, Message>> {
    if !state.settings_open {
        return None;
    }
    let p = devscribe_core::theme::palette(state.theme_mode, state.accent);

    let header = row![
        text("SETTINGS")
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_strong))
            .width(Length::Fill),
        button(widgets::center_fill(
            text("\u{2715}").size(crate::text_scale::px(13.0)).color(color(p.text_muted)),
        ))
            .padding(0.0)
            .width(Length::Fixed(18.0))
            .height(Length::Fixed(18.0))
            .on_press(Message::CloseSettings)
            .style(move |_theme, status| button::Style {
                background: if status == button::Status::Hovered { Some(color(p.surface_hover).into()) } else { None },
                border: Border { radius: 3.0.into(), ..Border::default() },
                ..button::Style::default()
            }),
    ]
    .align_y(Alignment::Center)
    .padding([12.0, 16.0]);

    let split = row![category_nav(state, p), widgets::vline(color(p.border_hairline)), category_content(state, p)]
        .height(Length::Fill);

    let body = column![header, widgets::hline(color(p.border_hairline)), split];

    let panel = container(body)
        .width(Length::Fixed(860.0))
        .height(Length::Fixed(700.0))
        // Padding matching the border's own width — without it, `category_nav`'s
        // full-height `bg_void` fill (the left nav rail's distinct background,
        // zero-inset since `body`/`split` carry no padding of their own) sits
        // flush against this container's true left edge and paints over the
        // border stroke there, since children draw on top of their parent's
        // own background+border quad. The other three edges look fine only
        // because nothing else here has an opaque background reaching that far
        // out — this one child did.
        .padding(1.5)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_accent),
                width: 1.5,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        });

    // Shields the panel from the backdrop below: a plain `container`/`row`
    // (e.g. `status_row`/`shortcut_row`'s reference content — most of
    // Toolchains/Shortcuts/About) doesn't capture clicks on its own, so
    // without this, clicking any non-button spot inside the panel — a
    // status row, a shortcut's key label, blank space between sections —
    // falls through all the way to the backdrop `mouse_area` below and
    // closes the whole modal. Real buttons/inputs inside `panel` still
    // capture their own presses first, so this only catches what they
    // don't.
    let panel = mouse_area(panel).on_press(Message::Noop);

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(
                    Color {
                        a: 0.55,
                        ..color(p.bg_canvas)
                    }
                    .into(),
                ),
                ..container::Style::default()
            }),
    )
    .on_press(Message::CloseSettings);

    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center);

    Some(iced::widget::stack![backdrop, centered].into())
}
