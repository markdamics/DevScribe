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
use devscribe_core::theme::{Accent, Palette, Rgba, ThemeMode};
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, mouse_area, row, scrollable, slider, text, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use devscribe_core::lsp::LspLanguage;

use crate::color::color;
use crate::density::Density;
use crate::fonts;
use crate::server_install;
use crate::ui::modifier_icon::{ModifierGlyph, ModifierIcon};
use crate::state::{
    self, Message, SettingsCategory, State, ThemePreview, EDITOR_FONT_SIZE_MAX,
    EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_STEP, MARKDOWN_PREVIEW_ZOOM_MAX,
    MARKDOWN_PREVIEW_ZOOM_MIN, MARKDOWN_PREVIEW_ZOOM_STEP, TAB_SIZE_MAX, TAB_SIZE_MIN,
    TAB_SIZE_STEP, UI_FONT_SCALE_MAX, UI_FONT_SCALE_MIN, UI_FONT_SCALE_STEP,
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

/// Presets `autosave_row`'s stepper cycles through — `0` is "Off", the
/// default; the rest are common autosave cadences (VS Code's own default is
/// 1000ms "afterDelay", but this app has no debounced-typing autosave, only
/// a flat timer, so seconds-to-minutes presets fit better than sub-second
/// ones).
const AUTOSAVE_PRESETS: [u32; 4] = [0, 30, 60, 300];

fn autosave_label(secs: u32) -> String {
    match secs {
        0 => "Off".to_string(),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

/// A label + description on the left (same shape `toggle_row` uses), a
/// preset stepper on the right instead of an ON/OFF badge — roadmap item
/// 20's "or on timer" auto-save, independent of `save_on_focus_loss` above.
fn autosave_row(state: &State, p: Palette) -> Element<'static, Message> {
    let secs = state.autosave_interval_secs;
    let idx = AUTOSAVE_PRESETS.iter().position(|&s| s == secs).unwrap_or(0);
    let dec = AUTOSAVE_PRESETS[idx.saturating_sub(1)];
    let inc = AUTOSAVE_PRESETS[(idx + 1).min(AUTOSAVE_PRESETS.len() - 1)];
    row![
        column![
            text("Auto-save on a timer")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_strong)),
            text("Write every dirty buffer on a fixed interval")
                .font(fonts::mono(Weight::Normal))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
        ]
        .spacing(2.0)
        .width(Length::Fill),
        stepper_row(
            autosave_label(secs),
            Message::SetAutosaveInterval(dec),
            Message::SetAutosaveInterval(inc),
            idx > 0,
            idx + 1 < AUTOSAVE_PRESETS.len(),
            p,
        ),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .padding([8.0, 10.0])
    .into()
}

fn markdown_zoom_row(state: &State, p: Palette) -> Element<'static, Message> {
    let zoom = state.markdown_preview_zoom;
    stepper_row(
        format!("{:.0}%", zoom * 100.0),
        Message::SetMarkdownPreviewZoom(
            (zoom - MARKDOWN_PREVIEW_ZOOM_STEP).clamp(MARKDOWN_PREVIEW_ZOOM_MIN, MARKDOWN_PREVIEW_ZOOM_MAX),
        ),
        Message::SetMarkdownPreviewZoom(
            (zoom + MARKDOWN_PREVIEW_ZOOM_STEP).clamp(MARKDOWN_PREVIEW_ZOOM_MIN, MARKDOWN_PREVIEW_ZOOM_MAX),
        ),
        zoom > MARKDOWN_PREVIEW_ZOOM_MIN + f32::EPSILON,
        zoom < MARKDOWN_PREVIEW_ZOOM_MAX - f32::EPSILON,
        p,
    )
}

/// Dark/Light toggle (2 buttons) — replaces the old ten-named-theme grid.
/// Hovering either button live-previews it app-wide (roadmap item 11)
/// before `on_press` actually commits it.
fn theme_mode_row(state: &State, p: Palette) -> Element<'static, Message> {
    let buttons: Vec<Element<'static, Message>> = ThemeMode::ALL
        .into_iter()
        .map(|mode| {
            let active = state.theme_mode == mode;
            let preview = ThemePreview {
                theme_mode: mode,
                accent: state.accent,
                custom_accent: state.custom_accent,
                high_contrast: state.high_contrast,
            };
            let btn = button(
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
            });
            mouse_area(btn).on_enter(Message::PreviewTheme(preview)).on_exit(Message::ClearThemePreview).into()
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
                    let active = state.accent == accent && state.custom_accent.is_none();
                    let swatch = devscribe_core::theme::palette(state.theme_mode, accent).accent_solid;
                    let preview = ThemePreview {
                        theme_mode: state.theme_mode,
                        accent,
                        custom_accent: None,
                        high_contrast: state.high_contrast,
                    };
                    let btn = button(
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
                    });
                    mouse_area(btn).on_enter(Message::PreviewTheme(preview)).on_exit(Message::ClearThemePreview).into()
                })
                .collect();
            row(buttons).spacing(8.0).into()
        })
        .collect();

    column(rows).spacing(8.0).into()
}

/// One RGB channel's labeled slider (0..=255) — dragging it fires
/// `Message::AdjustCustomAccentDraft` with the other two channels held at
/// their current draft value, live-previewing app-wide as it moves
/// (roadmap item 11).
fn rgb_slider_row(label: &'static str, draft: (u8, u8, u8), channel: u8, p: Palette) -> Element<'static, Message> {
    let value = match channel {
        0 => draft.0,
        1 => draft.1,
        _ => draft.2,
    };
    row![
        text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(color(p.text_muted)).width(Length::Fixed(14.0)),
        slider(0..=255, value, move |v| {
            let next = match channel {
                0 => (v, draft.1, draft.2),
                1 => (draft.0, v, draft.2),
                _ => (draft.0, draft.1, v),
            };
            Message::AdjustCustomAccentDraft(next.0, next.1, next.2)
        })
        .width(Length::Fill)
        .style(move |_theme, _status| iced::widget::slider::Style {
            rail: iced::widget::slider::Rail {
                backgrounds: (color(p.accent_solid).into(), color(p.surface_raised).into()),
                width: 4.0,
                border: Border { color: color(p.border_hairline), width: 1.0, radius: 2.0.into() },
            },
            handle: iced::widget::slider::Handle {
                shape: iced::widget::slider::HandleShape::Circle { radius: 7.0 },
                background: color(p.bg_base).into(),
                border_width: 1.5,
                border_color: color(p.accent_solid),
            },
        }),
        text(value.to_string()).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(color(p.text_muted)).width(Length::Fixed(28.0)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .into()
}

/// The custom accent color picker (roadmap item 11) — three RGB sliders
/// live-previewing app-wide as they're dragged, a swatch showing the
/// draft color, and "Apply"/"Reset to preset" actions. Distinct from
/// `accent_row`'s fixed presets above it — this is the "not just presets"
/// half of the item.
fn custom_accent_picker(state: &State, p: Palette) -> Element<'static, Message> {
    let draft = state.custom_accent_draft;
    let swatch_color = Rgba::from_rgb8(draft.0, draft.1, draft.2);

    let swatch = container(Space::new().width(Length::Fixed(28.0)).height(Length::Fixed(28.0)))
        .style(move |_theme| container::Style {
            background: Some(color(swatch_color).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 4.0.into() },
            ..container::Style::default()
        });

    let active = state.custom_accent.is_some();
    let apply = button(text("Apply").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.bg_base)))
        .padding([5.0, 12.0])
        .on_press(Message::SetCustomAccent(draft.0, draft.1, draft.2))
        .style(move |_theme, _status| button::Style {
            background: Some(color(p.accent_solid).into()),
            border: Border { radius: 3.0.into(), ..Border::default() },
            ..button::Style::default()
        });

    let reset = button(text("Reset to preset").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.text_muted)))
        .padding([5.0, 12.0])
        .on_press_maybe(active.then_some(Message::ClearCustomAccent))
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered { Some(color(p.surface_hover).into()) } else { None },
                border: Border { color: color(p.border_hairline), width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            }
        });

    column![
        row![
            swatch,
            column![
                rgb_slider_row("R", draft, 0, p),
                rgb_slider_row("G", draft, 1, p),
                rgb_slider_row("B", draft, 2, p),
            ]
            .spacing(4.0)
            .width(Length::Fill),
        ]
        .spacing(10.0)
        .align_y(Alignment::Center),
        row![apply, reset].spacing(8.0),
    ]
    .spacing(10.0)
    .into()
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
        column![section_label("CUSTOM ACCENT", p), custom_accent_picker(state, p)].spacing(8.0),
        column![
            section_label("ACCESSIBILITY", p),
            toggle_row(
                "High Contrast",
                "Stronger text and border contrast throughout the app",
                state.high_contrast,
                Message::ToggleHighContrast,
                p,
            ),
        ]
        .spacing(8.0),
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
        column![section_label("MARKDOWN PREVIEW ZOOM", p), markdown_zoom_row(state, p)].spacing(8.0),
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
            autosave_row(state, p),
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

/// One physical key or modifier in a shortcut combo, platform-agnostic —
/// `key_chip` below turns each into the right label (or, on macOS, icon)
/// for whichever OS this binary is actually running on, rather than the old
/// approach of hardcoding one Mac-only Unicode string per shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    /// The app's primary modifier — `Cmd` on macOS, `Ctrl` on Windows/Linux.
    /// Matches `iced::keyboard::Modifiers::command()`'s own per-OS split,
    /// which is what every `Primary`-tagged binding in this table is
    /// actually wired against (`state::global_keys`, `editor_canvas::handle_key`).
    Primary,
    Shift,
    /// `Option` on macOS, `Alt` on Windows/Linux.
    Alt,
    /// The literal physical Control key — stays "Ctrl" on every platform,
    /// including macOS. Only `Ctrl+Tab` (cycle tabs) uses this; see
    /// `state::global_keys`'s own doc comment on why that one binding is
    /// never translated to Cmd the way `Primary` is everywhere else.
    Control,
    /// Anything that isn't a modifier: a letter, a symbol, or a named key.
    Lit(&'static str),
}

/// `true` on macOS, where `Primary`/`Shift`/`Alt`/`Control` render as the
/// small drawn glyphs (⌘⇧⌥⌃) — the bundled font has no glyphs for those
/// codepoints, so drawing them (`modifier_icon.rs`) is the only option that
/// doesn't produce tofu boxes. Windows/Linux instead spell every modifier
/// out as plain "Ctrl"/"Shift"/"Alt" text, which needs no icon at all: it's
/// both more legible (nobody's keyboard has a ⌘ key to recognize the symbol
/// from) and sidesteps the missing-glyph problem entirely.
const USE_MODIFIER_GLYPHS: bool = cfg!(target_os = "macos");

/// One key rendered as a small pill/"keycap" badge — an icon for a macOS
/// modifier, plain text for everything else (a letter, a named key, or a
/// Windows/Linux modifier).
fn key_chip(key: Key, p: Palette) -> Element<'static, Message> {
    let content: Element<'static, Message> = if USE_MODIFIER_GLYPHS {
        let glyph = match key {
            Key::Primary => Some(ModifierGlyph::Command),
            Key::Shift => Some(ModifierGlyph::Shift),
            Key::Alt => Some(ModifierGlyph::Option),
            Key::Control => Some(ModifierGlyph::Control),
            Key::Lit(_) => None,
        };
        if let Some(glyph) = glyph {
            canvas(ModifierIcon { glyph, color: color(p.text_strong) })
                .width(Length::Fixed(13.0))
                .height(Length::Fixed(13.0))
                .into()
        } else {
            keycap_label(key, p)
        }
    } else {
        keycap_label(key, p)
    };

    container(content)
        .padding(Padding { top: 4.0, right: 7.0, bottom: 4.0, left: 7.0 })
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 5.0.into() },
            ..container::Style::default()
        })
        .into()
}

/// `key_chip`'s text fallback — every `Lit`, plus every modifier on
/// Windows/Linux (`USE_MODIFIER_GLYPHS` is `false` there).
fn keycap_label(key: Key, p: Palette) -> Element<'static, Message> {
    let label = match key {
        Key::Primary | Key::Control => "Ctrl",
        Key::Shift => "Shift",
        Key::Alt => "Alt",
        Key::Lit(s) => s,
    };
    text(label)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(12.0))
        .color(color(p.text_strong))
        .into()
}

/// A row of `key_chip`s for one shortcut, generously spaced (`spacing(6.0)`)
/// rather than the old glued-together Unicode string — each key now reads as
/// its own distinct cap instead of a blurred run of symbols.
fn key_row(keys: &'static [Key], p: Palette) -> Element<'static, Message> {
    row(keys.iter().map(|&key| key_chip(key, p))).spacing(6.0).align_y(Alignment::Center).into()
}

fn shortcut_row(label: &'static str, keys: &'static [Key], p: Palette) -> Element<'static, Message> {
    row![
        text(label)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body))
            .width(Length::Fill),
        key_row(keys, p),
    ]
    .align_y(Alignment::Center)
    .padding([7.0, 0.0])
    .into()
}

/// `shortcut_row`'s variant for a binding with two independent ways to
/// trigger it ("Go to definition" — a click gesture, or a plain function
/// key) — a `key_row` per alternative, joined by a plain "/" the way the two
/// used to be joined inside one glued string.
fn shortcut_row_alt(label: &'static str, alternatives: &'static [&'static [Key]], p: Palette) -> Element<'static, Message> {
    let mut keys_row = row![].spacing(8.0).align_y(Alignment::Center);
    for (i, combo) in alternatives.iter().enumerate() {
        if i > 0 {
            keys_row = keys_row.push(
                text("/").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.text_muted)),
            );
        }
        keys_row = keys_row.push(key_row(combo, p));
    }
    row![
        text(label)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body))
            .width(Length::Fill),
        keys_row,
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
    use Key::{Alt, Control, Lit, Primary, Shift};
    column![
        column![
            section_label("GENERAL", p),
            column![
                shortcut_row("Command palette", &[Primary, Lit("K")], p),
                shortcut_row("Quick open recent file", &[Primary, Lit("E")], p),
                shortcut_row("Keyboard shortcuts", &[Primary, Lit("/")], p),
                // The one binding that's literally Ctrl on every platform,
                // never `Primary` — see `Key::Control`'s own doc comment.
                shortcut_row("Cycle tabs", &[Control, Lit("Tab")], p),
                shortcut_row("Escape / close", &[Lit("Esc")], p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("AI CHAT ASSIST", p),
            column![
                shortcut_row("Toggle chat", &[Primary, Lit("I")], p),
                shortcut_row("Focus chat", &[Shift, Primary, Lit("I")], p),
                shortcut_row("New session", &[Alt, Primary, Lit("I")], p),
                shortcut_row("Open actions menu", &[Shift, Primary, Lit("U")], p),
                shortcut_row("Attach file", &[Primary, Lit("U")], p),
                shortcut_row("Send message", &[Lit("Enter")], p),
                shortcut_row("New line", &[Shift, Lit("Enter")], p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("FILES", p),
            column![
                shortcut_row("New file", &[Primary, Lit("N")], p),
                shortcut_row("New folder", &[Shift, Primary, Lit("N")], p),
                shortcut_row("Save", &[Primary, Lit("S")], p),
                shortcut_row("Copy path", &[Alt, Primary, Lit("C")], p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("TABS & SEARCH", p),
            column![
                shortcut_row("Close tab", &[Primary, Lit("W")], p),
                shortcut_row("Close other tabs", &[Alt, Primary, Lit("W")], p),
                shortcut_row("Reopen closed tab", &[Shift, Primary, Lit("T")], p),
                shortcut_row("Reveal in tree", &[Shift, Primary, Lit("E")], p),
                shortcut_row("Find in file", &[Primary, Lit("F")], p),
                shortcut_row("Find in project", &[Shift, Primary, Lit("F")], p),
                shortcut_row("Working tree diff", &[Shift, Primary, Lit("D")], p),
                shortcut_row("Go to line", &[Primary, Lit("G")], p),
                shortcut_row("Split editor", &[Primary, Lit("\\")], p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("EDITING", p),
            column![
                shortcut_row("Indent / block indent", &[Lit("Tab")], p),
                shortcut_row("Dedent / block dedent", &[Shift, Lit("Tab")], p),
                shortcut_row("Toggle line comment", &[Primary, Lit("/")], p),
                shortcut_row("Select next occurrence", &[Primary, Lit("D")], p),
                shortcut_row_alt(
                    "Move / select by word",
                    &[&[Primary, Lit("\u{2190}\u{2192}")], &[Shift, Primary, Lit("\u{2190}\u{2192}")]],
                    p,
                ),
                shortcut_row("Column/block selection", &[Alt, Lit("Drag")], p),
                shortcut_row("Toggle bookmark", &[Shift, Primary, Lit("M")], p),
            ]
        ]
        .spacing(8.0),
        column![
            section_label("NAVIGATION", p),
            column![
                shortcut_row_alt("Go to definition", &[&[Primary, Lit("Click")], &[Lit("F12")]], p),
                shortcut_row("Find all references", &[Shift, Lit("F12")], p),
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
        .direction(scrollable::Direction::Vertical(widgets::thin_scrollbar()))
        .style(widgets::scrollbar_style(p))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn view(state: &State) -> Option<Element<'static, Message>> {
    if !state.settings_open {
        return None;
    }
    let p = crate::state::active_palette(state);

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
