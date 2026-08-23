//! A collapsible JSON tree view for the currently open `.json` file, plus a
//! validation badge when the file doesn't parse.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Color, Element, Length, Padding};
use serde_json::Value;
use std::collections::HashSet;

use crate::color::color;
use crate::fonts;
use crate::state::{EditorState, Message};
use crate::widgets;

const INDENT: f32 = 16.0;
const GUTTER: f32 = 14.0;

fn value_color(value: &Value, p: Palette) -> Color {
    match value {
        Value::String(_) => color(p.status_ok),
        Value::Number(_) => color(p.accent_2),
        Value::Bool(_) => color(p.accent),
        Value::Null => color(p.text_muted),
        Value::Object(_) | Value::Array(_) => color(p.text_muted),
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

fn key_label(key: Option<&str>, p: Palette) -> Option<Element<'static, Message>> {
    key.map(|k| {
        text(format!("\"{k}\":"))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_primary))
            .into()
    })
}

fn toggle(p: Palette, path: String, collapsed: bool) -> Element<'static, Message> {
    button(widgets::center_fill(
        text(if collapsed { "\u{25b8}" } else { "\u{25be}" })
            .size(crate::text_scale::px(9.0))
            .color(color(p.text_muted)),
    ))
    .padding(0.0)
    .width(Length::Fixed(GUTTER))
    .height(Length::Fixed(16.0))
    .style(|_theme, _status| button::Style::default())
    .on_press(Message::JsonToggle(path))
    .into()
}

/// Appends this node's row(s) to `out`, recursing into children. `path` is
/// this node's collapse-state key (e.g. `"root.foo[2]"`).
fn node_rows(
    key: Option<&str>,
    value: &Value,
    path: String,
    collapsed: &HashSet<String>,
    depth: usize,
    p: Palette,
    out: &mut Vec<Element<'static, Message>>,
) {
    let indent = Padding {
        top: 1.0,
        right: 0.0,
        bottom: 1.0,
        left: 8.0 + depth as f32 * INDENT,
    };

    match value {
        Value::Object(map) if !map.is_empty() => {
            let is_collapsed = collapsed.contains(&path);
            let mut header = row![toggle(p, path.clone(), is_collapsed)]
                .spacing(4.0)
                .align_y(Alignment::Center);
            if let Some(label) = key_label(key, p) {
                header = header.push(label);
            }
            let summary = if is_collapsed {
                format!("{{\u{2026}}} {} keys", map.len())
            } else {
                "{".to_string()
            };
            header = header.push(
                text(summary)
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(12.0))
                    .color(color(p.text_muted)),
            );
            out.push(container(header).padding(indent).into());

            if !is_collapsed {
                for (k, v) in map.iter() {
                    node_rows(Some(k), v, format!("{path}.{k}"), collapsed, depth + 1, p, out);
                }
                out.push(closing_row("}", depth, p));
            }
        }
        Value::Array(items) if !items.is_empty() => {
            let is_collapsed = collapsed.contains(&path);
            let mut header = row![toggle(p, path.clone(), is_collapsed)]
                .spacing(4.0)
                .align_y(Alignment::Center);
            if let Some(label) = key_label(key, p) {
                header = header.push(label);
            }
            let summary = if is_collapsed {
                format!("[\u{2026}] {} items", items.len())
            } else {
                "[".to_string()
            };
            header = header.push(
                text(summary)
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(12.0))
                    .color(color(p.text_muted)),
            );
            out.push(container(header).padding(indent).into());

            if !is_collapsed {
                for (i, v) in items.iter().enumerate() {
                    node_rows(None, v, format!("{path}[{i}]"), collapsed, depth + 1, p, out);
                }
                out.push(closing_row("]", depth, p));
            }
        }
        // Empty object/array, or a scalar.
        other => {
            let mut r = row![Space::new().width(Length::Fixed(GUTTER))]
                .spacing(4.0)
                .align_y(Alignment::Center);
            if let Some(label) = key_label(key, p) {
                r = r.push(label);
            }
            let (text_content, color_val) = match other {
                Value::Object(_) => ("{}".to_string(), color(p.text_muted)),
                Value::Array(_) => ("[]".to_string(), color(p.text_muted)),
                scalar => (scalar_text(scalar), value_color(scalar, p)),
            };
            r = r.push(
                text(text_content)
                    .font(fonts::mono(Weight::Normal))
                    .size(crate::text_scale::px(12.0))
                    .color(color_val),
            );
            out.push(container(r).padding(indent).into());
        }
    }
}

fn closing_row(glyph: &'static str, depth: usize, p: Palette) -> Element<'static, Message> {
    container(
        text(glyph)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted)),
    )
    .padding(Padding {
        top: 1.0,
        right: 0.0,
        bottom: 1.0,
        left: 8.0 + GUTTER + depth as f32 * INDENT,
    })
    .into()
}

/// Renders `editor`'s JSON tree. Only called once the active tab is known to
/// be a `.json` file — see `shell.rs::content_area`.
pub fn view(editor: &EditorState, p: Palette) -> Element<'static, Message> {
    match &editor.json {
        None => widgets::placeholder(
            format!("{} isn't a .json file", editor.path.display()),
            p,
        ),
        Some(Err(parse_error)) => container(
            column![
                row![
                    widgets::dot(color(p.status_danger), 6.0),
                    text("INVALID JSON")
                        .font(fonts::mono(Weight::Bold))
                        .size(crate::text_scale::px(11.0))
                        .color(color(p.status_danger)),
                ]
                .spacing(6.0)
                .align_y(Alignment::Center),
                text(parse_error.clone())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(12.0))
                    .color(color(p.text_muted)),
            ]
            .spacing(10.0),
        )
        .padding(16.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_void).into()),
            ..container::Style::default()
        })
        .into(),
        Some(Ok(value)) => {
            let mut rows = Vec::new();
            node_rows(None, value, "root".to_string(), &editor.json_collapsed, 0, p, &mut rows);

            let badge = row![
                widgets::dot(color(p.status_ok), 6.0),
                text("VALID JSON")
                    .font(fonts::mono(Weight::Bold))
                    .size(crate::text_scale::px(10.0))
                    .color(color(p.text_muted)),
            ]
            .spacing(6.0)
            .align_y(Alignment::Center)
            .padding([8.0, 12.0]);

            let tree = scrollable(column(rows).padding([4.0, 12.0]))
                .width(Length::Fill)
                .height(Length::Fill);

            container(column![badge, tree])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_theme| container::Style {
                    background: Some(color(p.bg_void).into()),
                    ..container::Style::default()
                })
                .into()
        }
    }
}
