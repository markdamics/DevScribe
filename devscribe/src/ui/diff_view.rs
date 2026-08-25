//! A unified (not side-by-side) diff of the current file against its `HEAD`
//! version. Read-only — no Stage/Discard; those mutate git state and deserve
//! their own careful pass with explicit confirmation, not a bolt-on here.
use devscribe_core::diff::{DiffLine, DiffLineKind};
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{column, container, row, scrollable, text};
use iced::{Alignment, Color, Element, Length, Padding};

use std::path::Path;

use crate::color::color;
use crate::fonts;
use crate::state::{self, DiffStatus, Message, State};
use crate::widgets;

const GUTTER_WIDTH: f32 = 44.0;

fn line_number(n: Option<usize>, p: Palette) -> Element<'static, Message> {
    let label = n.map(|n| (n + 1).to_string()).unwrap_or_default();
    text(label)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(13.0))
        .color(color(p.text_muted))
        .width(Length::Fixed(GUTTER_WIDTH))
        .into()
}

fn diff_row(line: &DiffLine, p: Palette) -> Element<'static, Message> {
    let (marker, marker_color, bg) = match line.kind {
        DiffLineKind::Equal => (" ", color(p.text_muted), None),
        DiffLineKind::Insert => ("+", color(p.status_success), Some(tint(p.status_success))),
        DiffLineKind::Delete => ("-", color(p.status_danger), Some(tint(p.status_danger))),
    };

    let content = row![
        line_number(line.old_line, p),
        line_number(line.new_line, p),
        text(marker)
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(15.0))
            .color(marker_color)
            .width(Length::Fixed(16.0)),
        text(line.text.clone())
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body)),
    ]
    .spacing(4.0)
    .align_y(Alignment::Center);

    container(content)
        .width(Length::Fill)
        .padding(Padding {
            top: 1.0,
            right: 12.0,
            bottom: 1.0,
            left: 8.0,
        })
        .style(move |_theme| container::Style {
            background: bg.map(Into::into),
            ..container::Style::default()
        })
        .into()
}

fn tint(mut c: devscribe_core::theme::Rgba) -> Color {
    c.a = 0.14;
    color(c)
}

/// Renders the diff for `path`, whose `HEAD`-vs-buffer `DiffStatus` is
/// cached on its backing `File` tab — a `Diff` tab is only ever opened
/// alongside one (see `state::open_or_focus_diff`).
pub fn view(state: &State, path: &Path, p: Palette) -> Element<'static, Message> {
    let Some(editor) = state::find_editor(state, path) else {
        return widgets::placeholder("No file open \u{2014} pick one from the sidebar", p);
    };

    match &editor.diff {
        DiffStatus::NoRepo => widgets::placeholder("Not inside a git repository", p),
        DiffStatus::Untracked => {
            widgets::placeholder("New file \u{2014} no HEAD version to diff against", p)
        }
        DiffStatus::UpToDate => widgets::placeholder("No changes since HEAD", p),
        DiffStatus::Changed(lines) => {
            let rows: Vec<Element<'static, Message>> =
                lines.iter().map(|line| diff_row(line, p)).collect();

            container(
                scrollable(column(rows).padding([8.0, 0.0]))
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(color(p.bg_canvas).into()),
                ..container::Style::default()
            })
            .into()
        }
    }
}
