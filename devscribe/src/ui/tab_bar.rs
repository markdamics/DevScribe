use devscribe_core::theme::Palette;
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, mouse_area, row, scrollable, text, Space};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::color::color;
use crate::density::Density;
use crate::fonts;
use crate::state::{EditorState, Message, OpenTab, State, TabKey};
use crate::ui::search_icon::SearchIcon;
use crate::widgets;

fn underline_color(active: bool, p: Palette) -> Color {
    color(if active { p.accent_solid } else { p.border_hairline })
}

/// Wraps `content` with a 1px active/inactive indicator along its bottom
/// edge, sized to `content`'s own natural width rather than stretching to
/// fill the tab bar.
///
/// Deliberately *not* a `Length::Fill`-width strip stacked below `content`
/// in a `column!` — a `Fill` child inside a `Shrink` column still expands to
/// whatever (generous) max-width limit the ancestor `Row` hands it, so with
/// this tab as the last/only item in the bar, that stacked-underline version
/// stretched all the way to the far edge instead of tracking the tab. This
/// version paints the indicator as the *container's own background*, showing
/// through only the 1px of bottom padding `content` doesn't cover — so the
/// container's width is simply `content`'s width, no `Fill` involved.
fn with_underline(content: Element<'static, Message>, active: bool, p: Palette) -> Element<'static, Message> {
    container(content)
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: 1.0,
            left: 0.0,
        })
        .style(move |_theme| container::Style {
            background: Some(underline_color(active, p).into()),
            ..container::Style::default()
        })
        .into()
}

/// The fixed, always-visible project-search icon — not a closeable tab (see
/// `OpenTab`'s doc), just a pinned entry point into the search view.
fn search_icon_tab(active: bool, p: Palette, density: Density) -> Element<'static, Message> {
    let tab_h = density.tab_bar_h() - 1.0;
    let icon_color = if active { color(p.text_strong) } else { color(p.text_muted) };
    let icon = canvas(SearchIcon { color: icon_color })
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0));

    let select = button(widgets::center_fill(icon))
        .width(Length::Fixed(36.0))
        .height(Length::Fixed(tab_h))
        .padding(0.0)
        .on_press(Message::FocusSearchTab)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if active {
                    Some(color(p.bg_canvas).into())
                } else if hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                ..button::Style::default()
            }
        });

    with_underline(select.into(), active, p)
}

/// The select-button + close-button + active-underline shell shared by every
/// tab kind. `content` is whatever that tab kind wants to show as its label.
///
/// The active background is painted once, by the outer `container` wrapping
/// both buttons — not by each button individually. Two adjacent buttons each
/// only fill their own (content-sized) bounds, so painting the "active" fill
/// on both left a visible gap between the label and the close button instead
/// of one continuous highlighted tab. Buttons can't nest in iced, so this
/// container is the only shared surface both can sit on.
/// `select`/`close` are passed in as full `Message`s rather than derived
/// from a `TabKey` here, so this can back both real `open_tabs`-backed tabs
/// (`Message::SelectOpenTab`/`CloseTab`) *and* the virtual `Chat` tab
/// (`Message::ChatOpenTab`/`ChatCloseTab`, since `close_tab` only ever
/// looks tabs up in `open_tabs`, which `Chat` is never part of — see
/// `TabKey::Chat`'s own doc comment).
fn tab_shell(
    select: Message,
    close: Message,
    active: bool,
    p: Palette,
    density: Density,
    content: Element<'static, Message>,
) -> Element<'static, Message> {
    let tab_h = density.tab_bar_h() - 1.0;

    let select = button(widgets::center_v(content))
        .height(Length::Fixed(tab_h))
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 16.0,
        })
        .on_press(select)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if !active && hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                text_color: if active {
                    color(p.text_strong)
                } else {
                    color(p.text_muted)
                },
                ..button::Style::default()
            }
        });

    let close = button(widgets::center_fill(text("\u{2715}").size(crate::text_scale::px(13.0))))
        .padding(0.0)
        .width(Length::Fixed(20.0))
        .height(Length::Fixed(tab_h))
        .on_press(close)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                text_color: if hovered {
                    color(p.text_strong)
                } else {
                    color(p.text_muted)
                },
                ..button::Style::default()
            }
        });

    let inner = container(row![select, close].align_y(Alignment::Center))
        .height(Length::Fixed(tab_h))
        .style(move |_theme| container::Style {
            background: active.then(|| color(p.bg_canvas).into()),
            ..container::Style::default()
        });

    with_underline(inner.into(), active, p)
}

fn file_tab_label(editor: &EditorState, p: Palette) -> Element<'static, Message> {
    // An untitled buffer (no file on disk yet — `editor.path` is a
    // synthetic "Untitled-N", see `state::begin_untitled_buffer`) gets the
    // mockup's accent-tinted `TXT` glyph rather than a language badge:
    // `Lang::from_path` would otherwise fall through to `Lang::Other`'s
    // generic "no extension" glyph, indistinguishable from any other
    // unrecognized real file.
    let (fg, bg, code) = if editor.document.path().is_none() {
        (color(p.accent_solid), Color { a: 0.22, ..color(p.accent_solid) }, "TXT".to_string())
    } else {
        let lang = crate::fs_tree::Lang::from_path(&editor.path);
        let (fg, bg) = lang.badge(p);
        (fg, bg, lang.code(&editor.path))
    };
    let name = editor
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut contents = row![
        widgets::lang_badge(code, fg, bg),
        text(name)
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(15.0)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);
    if editor.document.is_dirty() {
        contents = contents.push(widgets::dot(color(p.status_warning), 5.0));
    }
    contents.into()
}

fn diff_tab_label(path: &std::path::Path, p: Palette) -> Element<'static, Message> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    row![text(format!("{name} \u{2194} HEAD"))
        .font(fonts::sans(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .color(color(p.text_strong))]
    .into()
}

fn chat_tab_label(p: Palette) -> Element<'static, Message> {
    row![
        widgets::dot(color(p.accent_solid), 6.0),
        text("Chat").font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(15.0)).color(color(p.text_strong)),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .into()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let bar_h = state.density.tab_bar_h();

    let search_active = state.active_tab.as_ref() == Some(&TabKey::Search);
    let mut tab_elements: Vec<Element<'static, Message>> =
        vec![search_icon_tab(search_active, p, state.density)];
    // Chat, when opened in tab view, is pinned immediately after the search
    // icon rather than appended after every file tab — keeping it visible
    // near the fixed search entry point instead of drifting off past
    // however many file tabs happen to be scrolled open.
    if state.chat_tab_open {
        let active = state.active_tab.as_ref() == Some(&TabKey::Chat);
        tab_elements.push(tab_shell(Message::ChatOpenTab, Message::ChatCloseTab, active, p, state.density, chat_tab_label(p)));
    }
    tab_elements.extend(state.open_tabs.iter().map(|tab| {
        let key = tab.key();
        let active = state.active_tab.as_ref() == Some(&key);
        let label = match tab {
            OpenTab::File(editor) => file_tab_label(editor, p),
            OpenTab::Diff(path) => diff_tab_label(path, p),
        };
        tab_shell(Message::SelectOpenTab(key.clone()), Message::CloseTab(key), active, p, state.density, label)
    }));

    let tabs = scrollable(row(tab_elements).height(Length::Fixed(bar_h)))
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default().width(0.0).scroller_width(0.0)))
        .width(Length::Fill)
        .height(Length::Fixed(bar_h));

    let overflow_open = state.overflow_open;
    let overflow = button(widgets::center_fill(text("\u{22ef}").size(crate::text_scale::px(15.0))))
        .width(Length::Fixed(bar_h))
        .height(Length::Fixed(bar_h))
        .padding(0.0)
        .on_press(Message::ToggleOverflow)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                text_color: if hovered || overflow_open {
                    color(p.text_strong)
                } else {
                    color(p.text_muted)
                },
                ..button::Style::default()
            }
        });

    let bar = row![tabs, overflow].align_y(Alignment::Center);

    container(bar)
        .width(Length::Fill)
        .height(Length::Fixed(bar_h))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            ..container::Style::default()
        })
        .into()
}

fn overflow_row(label: &'static str, shortcut: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(
        row![
            text(label)
                .font(fonts::sans(Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_strong))
                .width(Length::Fill),
            text(shortcut)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
        ]
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([6.0, 8.0])
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.bg_base).into())
            } else {
                None
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// The `⋯` button's dropdown: Close others / Reveal in tree / Reopen closed
/// tab — real, undecorated actions (see `state.rs`'s `close_other_tabs`/
/// `reveal_active_in_tree`/`reopen_closed_tab`). Deliberately omits the
/// mockup's "Split right" row: no split-pane state exists anywhere, and the
/// mockup's own `onClick` for it just closes the menu — same
/// not-a-real-build-target treatment as the window control buttons (see the
/// roadmap's item 7). Rendered as a top-level `stack!` layer, same
/// backdrop-close pattern as `settings_panel`/`command_palette`/
/// `context_menu`, positioned under the tab bar's own `⋯` button.
pub fn overflow_menu(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.overflow_open {
        return None;
    }

    let menu = container(
        column![
            overflow_row("Close all other tabs", "\u{2325}\u{2318}W", Message::CloseOtherTabs, p),
        ]
        .spacing(2.0)
        .padding(6.0),
    )
    .width(Length::Fixed(214.0))
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border {
            color: color(p.border_hairline),
            width: 1.5,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    });

    let positioned = container(menu)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .padding(Padding {
            top: state.density.title_bar_h() + state.density.tab_bar_h() + 4.0,
            right: 8.0,
            bottom: 0.0,
            left: 0.0,
        });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::ToggleOverflow);

    Some(iced::widget::stack![backdrop, positioned].into())
}
