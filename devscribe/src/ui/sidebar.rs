use devscribe_core::theme::{Palette, Rgba};
use iced::alignment::Vertical;
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Border, Color, Element, Length, Padding};
use std::path::Path;

use crate::color::color;
use crate::fonts;
use crate::fs_tree::Node;
use crate::state::{Message, State};
use crate::widgets;

fn tint(c: Rgba, alpha: f32) -> Color {
    color(Rgba { a: alpha, ..c })
}

fn node_view(
    node: &Node,
    depth: usize,
    p: Palette,
    selected: Option<&Path>,
) -> Element<'static, Message> {
    match node {
        Node::Dir { name, children } => {
            let header = row![
                text("▸").size(10.0).color(color(p.text_muted)),
                text(name.clone())
                    .font(fonts::sans(Weight::Medium))
                    .size(13.0)
                    .color(color(p.text_primary)),
            ]
            .spacing(7.0)
            .align_y(Alignment::Center)
            .padding(Padding {
                top: 0.0,
                right: 8.0,
                bottom: 0.0,
                left: 8.0 + depth as f32 * 14.0,
            });

            let header_row = container(header)
                .width(Length::Fill)
                .height(Length::Fixed(28.0))
                .align_y(Vertical::Center);

            let child_rows: Vec<Element<'static, Message>> = children
                .iter()
                .map(|child| node_view(child, depth + 1, p, selected))
                .collect();

            column![header_row, column(child_rows)].into()
        }
        Node::File { name, path, lang } => {
            let is_selected = selected == Some(path.as_path());
            let (fg, bg) = lang.badge(p);
            let badge_label = lang.code(path);

            let contents = row![
                widgets::lang_badge(badge_label, fg, bg),
                text(name.clone())
                    .font(fonts::sans(Weight::Medium))
                    .size(13.0)
                    .color(color(p.text_primary))
                    .width(Length::Fill),
            ]
            .spacing(7.0)
            .align_y(Alignment::Center);

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
            button(row_content)
                .width(Length::Fill)
                .height(Length::Fixed(28.0))
                .padding(0.0)
                .on_press(Message::SelectFile(path))
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
                })
                .into()
        }
    }
}

fn shorten_home(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

fn project_switcher(p: Palette, root: &Path) -> Element<'static, Message> {
    let (badge_fg, badge_bg) = (color(p.accent), tint(p.accent, 0.18));
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    let path_label = shorten_home(root);

    let header = button(
        row![
            widgets::lang_badge("PR", badge_fg, badge_bg),
            column![
                text(name)
                    .font(fonts::display(Weight::Semibold))
                    .size(15.0)
                    .color(color(p.text_primary)),
                text(path_label)
                    .font(fonts::mono(Weight::Medium))
                    .size(10.0)
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

    let block = column![header].spacing(4.0).padding([12.0, 12.0]);

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

fn footer(p: Palette) -> Element<'static, Message> {
    let hint = text("git status lands in milestone 6")
        .font(fonts::mono(Weight::Medium))
        .size(10.0)
        .color(color(p.text_muted))
        .width(Length::Fill);

    let settings = button(widgets::center_fill(text("⚙").size(14.0)))
        .width(Length::Fixed(26.0))
        .height(Length::Fixed(26.0))
        .padding(0.0)
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
        row![hint, settings]
            .spacing(8.0)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8.0, 12.0])
    .into()
}

pub fn view(state: &State, p: Palette) -> Element<'static, Message> {
    let selected: Option<&Path> = state.editor.as_ref().map(|e| e.path.as_path());

    let tree_rows: Vec<Element<'static, Message>> = state
        .tree
        .iter()
        .map(|node| node_view(node, 0, p, selected))
        .collect();

    let tree_view = scrollable(column(tree_rows).padding([8.0, 4.0]))
        .width(Length::Fill)
        .height(Length::Fill);

    let body = column![
        project_switcher(p, &state.root),
        widgets::hline(color(p.line_neutral)),
        container(tree_view).height(Length::Fill),
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
