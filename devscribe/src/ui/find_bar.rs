//! The floating in-file "find" pill (Ctrl+F) — a small overlay pinned to the
//! top-right of the editor pane, layered over `editor_canvas` via a local
//! `stack!` in `shell.rs`. Distinct from `search_view`, which searches every
//! file rather than the current buffer.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, row, text, text_input, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{self, EditorState, FindState, Message};
use crate::ui::search_icon::SearchIcon;
use crate::widgets;

fn nav_button(label: &'static str, tooltip_label: &'static str, on_press: Message, enabled: bool, p: Palette) -> Element<'static, Message> {
    let mut b = button(widgets::center_fill(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0)),
    ))
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(22.0))
    .padding(0.0)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            text_color: if enabled {
                color(p.text_strong)
            } else {
                color(p.text_muted)
            },
            ..button::Style::default()
        }
    });
    if enabled {
        b = b.on_press(on_press);
    }
    widgets::tooltip(b, tooltip_label, p)
}

/// Toggles the replace row below the find row — an expand/collapse chevron,
/// same glyph pair `json_view.rs::toggle` uses for its tree nodes.
fn expand_toggle(p: Palette, open: bool) -> Element<'static, Message> {
    button(widgets::center_fill(
        text(if open { "\u{25be}" } else { "\u{25b8}" })
            .size(crate::text_scale::px(11.0))
            .color(color(p.text_muted)),
    ))
    .width(Length::Fixed(16.0))
    .height(Length::Fixed(22.0))
    .padding(0.0)
    .on_press(Message::ToggleReplace)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            ..button::Style::default()
        }
    })
    .into()
}

fn replace_action_button(
    label: &'static str,
    on_press: Message,
    enabled: bool,
    p: Palette,
) -> Element<'static, Message> {
    let mut b = button(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(if enabled { color(p.text_strong) } else { color(p.text_muted) }),
    )
    .padding([3.0, 8.0])
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
                width: 1.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    });
    if enabled {
        b = b.on_press(on_press);
    }
    b.into()
}

/// Small pill button used by the "Replace N matches?" confirmation prompt.
fn confirm_button(label: &'static str, on_press: Message, accent: bool, p: Palette) -> Element<'static, Message> {
    button(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(if accent { color(p.bg_base) } else { color(p.text_strong) }),
    )
    .padding([3.0, 10.0])
    .on_press(on_press)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: Some(color(if accent {
                p.accent_solid
            } else if hovered {
                p.surface_hover
            } else {
                p.surface_raised
            }).into()),
            border: Border {
                color: color(p.border_hairline),
                width: if accent { 0.0 } else { 1.0 },
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// Replaces the replace row's normal buttons while `find.confirm_replace_all`
/// — asks "Replace N matches?" instead of firing the replacement the moment
/// "All" is clicked, since replacing every match in the file isn't something
/// worth risking a stray click on.
fn replace_all_confirm_row(find: &FindState, p: Palette) -> Element<'static, Message> {
    let n = find.matches.len();
    row![
        Space::new().width(Length::Fixed(16.0 + 13.0)),
        text(format!("Replace {n} match{}?", if n == 1 { "" } else { "es" }))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_body)),
        Space::new().width(Length::Fill),
        confirm_button("Yes", Message::ConfirmReplaceAll, true, p),
        confirm_button("No", Message::CancelReplaceAll, false, p),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .padding(Padding {
        top: 2.0,
        right: 6.0,
        bottom: 6.0,
        left: 10.0,
    })
    .into()
}

/// The replace row, shown below the find row while `find.replace_open` —
/// its own text input plus "Replace"/"All" buttons, both disabled while
/// there's nothing to replace. Swapped for `replace_all_confirm_row` while
/// `find.confirm_replace_all` is asking the user to confirm "All".
fn replace_row(find: &FindState, p: Palette) -> Element<'static, Message> {
    if find.confirm_replace_all {
        return replace_all_confirm_row(find, p);
    }

    let can_replace = !find.matches.is_empty();

    let input = text_input("Replace\u{2026}", &find.replace_query)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .padding([4.0, 6.0])
        .on_input(Message::ReplaceQueryChanged)
        .on_submit(Message::ReplaceOne)
        .style(move |_theme, status| text_input::Style {
            background: Color::TRANSPARENT.into(),
            // Visible keyboard-focus indicator (accessibility pass, item 12).
            border: if matches!(status, text_input::Status::Focused { .. }) {
                Border { color: color(p.border_focus), width: 1.5, radius: 0.0.into() }
            } else {
                Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() }
            },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_strong),
            selection: {
                let mut c = p.accent_solid;
                c.a = 0.35;
                color(c)
            },
        });

    row![
        // Aligns the replace input under the find input, past the chevron
        // and search-icon columns above it.
        Space::new().width(Length::Fixed(16.0 + 13.0)),
        input,
        replace_action_button("Replace", Message::ReplaceOne, can_replace, p),
        replace_action_button("All", Message::ReplaceAll, can_replace, p),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .padding(Padding {
        top: 2.0,
        right: 6.0,
        bottom: 6.0,
        left: 10.0,
    })
    .into()
}

/// The "?" quick-help popover's body — a few lines of find/replace tips,
/// shown below the pill while `find.help_open`.
fn help_panel(p: Palette) -> Element<'static, Message> {
    let line = |s: &'static str| {
        text(s)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted))
    };
    container(
        column![
            line("Find matches case-insensitively, as plain text."),
            line("Enter / \u{2191}\u{2193} \u{2014} jump to next/previous match"),
            line("Ctrl/Cmd+H \u{2014} toggle replace"),
            line("\"All\" asks for confirmation before replacing every match"),
        ]
        .spacing(4.0),
    )
    .padding(Padding {
        top: 6.0,
        right: 10.0,
        bottom: 8.0,
        left: 10.0,
    })
    .into()
}

/// `editor` must have `editor.find.is_some()` — callers check this before
/// including the bar as a stack layer.
pub fn view(editor: &EditorState, p: Palette) -> Element<'static, Message> {
    let find = editor.find.as_ref().expect("caller checks find.is_some()");

    let count_label = if find.query.is_empty() {
        String::new()
    } else if find.matches.is_empty() {
        "No results".to_string()
    } else if find.just_wrapped {
        format!("{} of {} (wrapped)", find.current + 1, find.matches.len())
    } else {
        format!("{} of {}", find.current + 1, find.matches.len())
    };

    let has_matches = !find.matches.is_empty();

    let input = text_input("Find\u{2026}", &find.query)
        .id(state::find_query_id())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .padding([4.0, 6.0])
        .on_input(Message::FindQueryChanged)
        .on_submit(Message::FindNext)
        .style(move |_theme, status| text_input::Style {
            background: Color::TRANSPARENT.into(),
            // Visible keyboard-focus indicator (accessibility pass, item 12).
            border: if matches!(status, text_input::Status::Focused { .. }) {
                Border { color: color(p.border_focus), width: 1.5, radius: 0.0.into() }
            } else {
                Border { color: Color::TRANSPARENT, width: 0.0, radius: 0.0.into() }
            },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_strong),
            selection: {
                let mut c = p.accent_solid;
                c.a = 0.35;
                color(c)
            },
        });

    let help_toggle = button(widgets::center_fill(
        text("?")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(if find.help_open { color(p.accent_solid) } else { color(p.text_muted) }),
    ))
    .width(Length::Fixed(22.0))
    .height(Length::Fixed(22.0))
    .padding(0.0)
    .on_press(Message::ToggleFindHelp)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            ..button::Style::default()
        }
    });

    let close = button(widgets::center_fill(text("\u{2715}").size(crate::text_scale::px(13.0))))
        .width(Length::Fixed(22.0))
        .height(Length::Fixed(22.0))
        .padding(0.0)
        .on_press(Message::CloseFind)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                text_color: color(p.text_muted),
                ..button::Style::default()
            }
        });
    let help_toggle = widgets::tooltip(help_toggle, "Find/replace tips", p);
    let close = widgets::tooltip(close, "Close (Esc)", p);

    let pill = row![
        expand_toggle(p, find.replace_open),
        canvas(SearchIcon { color: color(p.text_muted) })
            .width(Length::Fixed(13.0))
            .height(Length::Fixed(13.0)),
        input,
        text(count_label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
        nav_button("\u{2191}", "Previous match", Message::FindPrev, has_matches, p),
        nav_button("\u{2193}", "Next match", Message::FindNext, has_matches, p),
        help_toggle,
        close,
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .padding(Padding {
        top: 4.0,
        right: 6.0,
        bottom: 4.0,
        left: 6.0,
    });

    let mut panel_column = column![pill];
    if find.replace_open {
        panel_column = panel_column.push(replace_row(find, p));
    }
    if find.help_open {
        panel_column = panel_column.push(widgets::hline(color(p.border_hairline)));
        panel_column = panel_column.push(help_panel(p));
    }

    let panel = container(panel_column)
        .width(Length::Fixed(320.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_accent),
                width: 1.5,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });

    container(row![Space::new().width(Length::Fill), panel])
        .width(Length::Fill)
        .padding(Padding {
            top: 10.0,
            right: 20.0,
            bottom: 0.0,
            left: 0.0,
        })
        .into()
}
