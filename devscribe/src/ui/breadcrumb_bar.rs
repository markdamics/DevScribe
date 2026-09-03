//! The breadcrumb strip: `DevScribe.dc.html`'s `isCode`-only bar under the
//! tab bar, showing the enclosing scope stack at the cursor
//! (`ledger::engine › settle_batch › for (id, amount) in delta`) plus a
//! current-file-scoped error/warning/position/language readout on the
//! right. Distinct from `status_bar.rs`'s bottom Ln/Col/language readout,
//! which is workspace-wide; this one is specific to the active file, same
//! split the mockup itself draws (`1 error`/`1 warning` here vs `2
//! problems` there).
//!
//! Crumbs come from `EditorState::breadcrumbs()` — see
//! `devscribe_core::outline` for how the scope stack is actually found.
use devscribe_core::lsp::DiagnosticSeverity;
use devscribe_core::outline::{self, Crumb, CrumbKind};
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text};
use iced::{Alignment, Border, Element, Length, Padding};
use std::time::Duration;

use crate::color::color;
use crate::fonts;
use crate::state::{self, EditorState, Message, State};
use crate::widgets;

const HEIGHT: f32 = 30.0;

/// Same value `completions.rs`/`hover_popup.rs` use — every cursor/UI-
/// anchored popup in this app lines up off the same header height.
const HEADER_HEIGHT: f32 = 78.0;

/// How long the mouse has to rest on a breadcrumb segment before its hover
/// tooltip (`hover_view`) appears — same value `tab_bar::TAB_PREVIEW_DWELL`
/// uses for the same kind of "don't flash a tooltip while just passing
/// through" reasoning.
pub const HOVER_DWELL: Duration = Duration::from_millis(350);

/// A glyph per `CrumbKind`, standing in for the mockup's `box` /
/// `square-function` / `repeat` icon set — plain monospace characters,
/// same convention `sidebar.rs`/`json_view.rs` already use for their own
/// disclosure chevrons rather than loaded icon assets.
fn glyph(kind: CrumbKind) -> &'static str {
    match kind {
        CrumbKind::Module => "\u{25a3}",     // ▣
        CrumbKind::Type => "\u{25c6}",       // ◆
        CrumbKind::Function => "\u{192}",    // ƒ
        CrumbKind::Closure => "\u{3bb}",     // λ
        CrumbKind::Loop => "\u{21bb}",       // ↻ — same glyph the mockup's "repeat" icon draws
        CrumbKind::Conditional => "?",
        CrumbKind::Match => "\u{2261}",      // ≡
    }
}

/// A single breadcrumb segment — clickable (jumps the cursor to that
/// scope's start) and hoverable (shows `crumb.header` after `HOVER_DWELL`,
/// see `hover_view`) — roadmap item 10.
fn crumb_view(index: usize, crumb: &Crumb, emphasized: bool, p: Palette) -> Element<'static, Message> {
    let c = if emphasized { color(p.text_strong) } else { color(p.text_muted) };
    let inner = row![
        text(glyph(crumb.kind)).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(c),
        text(crumb.label.clone()).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(c),
    ]
    .spacing(5.0)
    .align_y(Alignment::Center);

    let clickable = button(inner)
        .padding(0.0)
        .on_press(Message::JumpToBreadcrumb(index))
        .style(move |_theme, status| button::Style {
            background: if status == button::Status::Hovered { Some(color(p.surface_hover).into()) } else { None },
            text_color: c,
            ..button::Style::default()
        });

    mouse_area(clickable)
        .on_enter(Message::BreadcrumbHoverStart(index))
        .on_exit(Message::BreadcrumbHoverEnd(index))
        .into()
}

/// The strip's "switch back to the other view" button — JSON's "Tree View"
/// and Markdown's "Preview", both shown only while that file's `_text_mode`
/// flag has flipped it over to the plain `code_area` (see `shell.rs`).
fn toggle_view_button(label: &'static str, on_press: Message, p: Palette) -> Element<'static, Message> {
    button(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(11.0))
            .color(color(p.text_muted)),
    )
    .padding([3.0, 8.0])
    .on_press(on_press)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered {
                Some(color(p.surface_raised).into())
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
    })
    .into()
}

fn chevron(p: Palette) -> Element<'static, Message> {
    text("\u{203a}")
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(12.0))
        .color(color(p.text_muted))
        .into()
}

/// `true` for a language `outline` never wires a landmark table for
/// (JSON/TOML/YAML/XML/INI) — those still get the strip (for its right-side
/// Ln/Col/language readout), just with an empty crumb trail, same as the
/// mockup showing the strip on every code tab regardless of how deep the
/// cursor happens to sit.
pub fn view(editor: &EditorState, p: Palette) -> Element<'static, Message> {
    let crumbs = editor.breadcrumbs();
    let emphasized = outline::emphasized_index(&crumbs);

    let mut path = Vec::with_capacity(crumbs.len() * 2);
    for (i, crumb) in crumbs.iter().enumerate() {
        if i > 0 {
            path.push(chevron(p));
        }
        path.push(crumb_view(i, crumb, emphasized == Some(i), p));
    }

    let errors = editor.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::ERROR).count();
    let warnings = editor.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::WARNING).count();
    let lang_label = editor.language.map(|l| l.label()).unwrap_or("Plain Text");
    let diff_counts = editor.diff_counts();

    let mut right = row![].spacing(12.0).align_y(Alignment::Center);
    if editor.json.is_some() && editor.json_text_mode {
        right = right.push(toggle_view_button("Tree View", Message::JsonToggleTextMode, p));
    }
    if editor.markdown.is_some() && editor.markdown_text_mode {
        right = right.push(toggle_view_button("Preview", Message::MarkdownToggleTextMode, p));
    }
    if let Some((inserted, deleted)) = diff_counts {
        if inserted > 0 || deleted > 0 {
            right = right.push(
                row![
                    text(format!("+{inserted}"))
                        .font(fonts::mono(Weight::Medium))
                        .size(crate::text_scale::px(11.0))
                        .color(color(p.status_success)),
                    text(format!("-{deleted}"))
                        .font(fonts::mono(Weight::Medium))
                        .size(crate::text_scale::px(11.0))
                        .color(color(p.status_danger)),
                ]
                .spacing(6.0),
            );
        }
    }
    if errors > 0 {
        right = right.push(
            text(format!("{errors} error{}", if errors == 1 { "" } else { "s" }))
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(11.0))
                .color(color(p.status_danger)),
        );
    }
    if warnings > 0 {
        right = right.push(
            text(format!("{warnings} warning{}", if warnings == 1 { "" } else { "s" }))
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(11.0))
                .color(color(p.status_warning)),
        );
    }
    right = right.push(
        text(format!("Ln {}, Col {}", editor.cursor.line + 1, editor.cursor.col + 1))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(11.0))
            .color(color(p.text_muted)),
    );
    right = right.push(
        text(lang_label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(11.0))
            .color(color(p.text_muted)),
    );

    let content = row![
        row(path).spacing(7.0).align_y(Alignment::Center).width(Length::Fill),
        right,
    ]
    .spacing(7.0)
    .align_y(Alignment::Center);

    let bar = container(content)
        .width(Length::Fill)
        .height(Length::Fixed(HEIGHT))
        .padding([0.0, 16.0])
        .align_y(Alignment::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        });

    column![bar, widgets::hline(color(p.border_hairline))].into()
}

/// The hover-context tooltip for whichever breadcrumb segment `state`'s
/// `breadcrumb_hover` dwell has settled on (roadmap item 10) — the crumb's
/// `header` (a function signature, a class definition, ...). Rendered as a
/// `stack!` layer in `shell.rs`, same pattern as `hover_popup::view`.
/// Anchored just under the breadcrumb strip's own fixed position rather
/// than the hovered segment's exact on-screen x — this app doesn't measure
/// widget positions anywhere else either (see `tab_bar::hover_preview`'s
/// own doc comment on the same tradeoff), so "near the strip" is as precise
/// as any popup here gets.
pub fn hover_view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let (index, since) = state.breadcrumb_hover.as_ref()?;
    if since.elapsed() < HOVER_DWELL {
        return None;
    }
    let editor = state::active_editor(state)?;
    let crumbs = editor.breadcrumbs();
    let crumb = crumbs.get(*index)?;

    let body = scrollable(
        column![text(crumb.header.clone())
            .font(fonts::mono(Weight::Normal))
            .size(crate::text_scale::px(12.5))
            .color(color(p.text_body))]
        .padding(10.0),
    );

    let popup = container(body)
        .max_width(480.0)
        .max_height(160.0)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 4.0.into() },
            ..container::Style::default()
        });

    let positioned = container(popup)
        .padding(Padding { top: HEADER_HEIGHT + HEIGHT, left: 16.0, right: 0.0, bottom: 0.0 })
        .width(Length::Fill)
        .height(Length::Fill);

    Some(positioned.into())
}
