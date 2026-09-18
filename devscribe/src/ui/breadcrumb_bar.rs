//! The breadcrumb module: `title_crumbs` renders the enclosing scope stack
//! at the cursor (`ledger::engine › settle_batch › for (id, amount) in
//! delta`) inline in the title bar, right after the DevScribe wordmark (see
//! `title_bar::view`). `toggle_view_button` is the other survivor of the old
//! under-tab-bar strip (removed entirely — its diagnostics/Ln,Col/language
//! readout duplicated `status_bar.rs`'s own) — the JSON/Markdown "switch
//! back to the rendered view" badge, now floated over the editor pane
//! itself (see `shell.rs::view_mode_toggle`).
//!
//! Crumbs come from `EditorState::breadcrumbs()` — see
//! `devscribe_core::outline` for how the scope stack is actually found.
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

/// How long the mouse has to rest on a breadcrumb segment before its hover
/// tooltip (`hover_view`) appears — same value `tab_bar::TAB_PREVIEW_DWELL`
/// uses for the same kind of "don't flash a tooltip while just passing
/// through" reasoning, and same 200-300ms responsiveness band
/// `state::editor::HOVER_DWELL` (the LSP hover popup) already sits in.
pub const HOVER_DWELL: Duration = Duration::from_millis(250);

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

/// The "switch back to the other view" button — JSON's "Tree View" and
/// Markdown's "Preview", both shown only while that file's `_text_mode`
/// flag has flipped it over to the plain `code_area`, as a small floating
/// badge over the editor pane (see `shell.rs::view_mode_toggle`).
pub fn toggle_view_button(label: &'static str, on_press: Message, p: Palette) -> Element<'static, Message> {
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

/// Just the crumb trail (no chrome, no right-side diagnostics/Ln/Col) — for
/// embedding inline in the title bar next to the DevScribe wordmark (see
/// `title_bar::view`). `None` with no crumbs to show, same as `view` showing
/// an empty trail in that case, except here there's no strip left to anchor
/// a lone right-side readout to, so the title bar just omits the separator
/// and trail entirely.
pub fn title_crumbs(editor: &EditorState, p: Palette) -> Option<Element<'static, Message>> {
    let crumbs = editor.breadcrumbs();
    if crumbs.is_empty() {
        return None;
    }
    let emphasized = outline::emphasized_index(&crumbs);

    let mut path = Vec::with_capacity(crumbs.len() * 2);
    for (i, crumb) in crumbs.iter().enumerate() {
        if i > 0 {
            path.push(chevron(p));
        }
        path.push(crumb_view(i, crumb, emphasized == Some(i), p));
    }

    Some(row(path).spacing(7.0).align_y(Alignment::Center).into())
}

/// The hover-context tooltip for whichever breadcrumb segment `state`'s
/// `breadcrumb_hover` dwell has settled on (roadmap item 10) — the crumb's
/// `header` (a function signature, a class definition, ...). Rendered as a
/// `stack!` layer in `shell.rs`, same pattern as `hover_popup::view`.
/// Anchored just under the title bar's own fixed height rather than the
/// hovered segment's exact on-screen x — this app doesn't measure widget
/// positions anywhere else either (see `tab_bar::hover_preview`'s own doc
/// comment on the same tradeoff), so "near the title bar" is as precise as
/// any popup here gets.
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
    )
    .direction(scrollable::Direction::Vertical(widgets::thin_scrollbar()))
    .style(widgets::scrollbar_style(p));

    let popup = container(body)
        .max_width(480.0)
        .max_height(160.0)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 4.0.into() },
            ..container::Style::default()
        });

    // The title bar's own painted height (`bar_panel`) plus the 1px hline
    // `title_bar::view` stacks under it — crumbs render inline in that bar
    // now, not in a separate strip below the tab bar.
    let positioned = container(popup)
        .padding(Padding { top: state.density.title_bar_h() + 1.0, left: 16.0, right: 0.0, bottom: 0.0 })
        .width(Length::Fill)
        .height(Length::Fill);

    Some(positioned.into())
}
