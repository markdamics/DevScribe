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
use devscribe_core::outline::{Crumb, CrumbKind};
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{EditorState, Message};
use crate::widgets;

const HEIGHT: f32 = 30.0;

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

/// The mockup emphasizes exactly one crumb (`text-strong`; the rest sit at
/// `text-muted`) — not the innermost overall, but the innermost *named*
/// scope: in its own example, the enclosing function is bold while both
/// the outer module and the trailing `for`-loop stay muted. Mirrors that:
/// the last crumb that names an actual definition (Module/Type/Function/
/// Closure) is emphasized; any control-flow crumbs after it, and everything
/// before it, are not. Falls back to the last crumb overall so something is
/// always emphasized even when the cursor has no enclosing definition (a
/// loop at a script's top level, say).
fn emphasized_index(crumbs: &[Crumb]) -> Option<usize> {
    crumbs
        .iter()
        .rposition(|c| matches!(c.kind, CrumbKind::Module | CrumbKind::Type | CrumbKind::Function | CrumbKind::Closure))
        .or(if crumbs.is_empty() { None } else { Some(crumbs.len() - 1) })
}

fn crumb_view(crumb: &Crumb, emphasized: bool, p: Palette) -> Element<'static, Message> {
    let c = if emphasized { color(p.text_strong) } else { color(p.text_muted) };
    row![
        text(glyph(crumb.kind)).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(c),
        text(crumb.label.clone()).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(c),
    ]
    .spacing(5.0)
    .align_y(Alignment::Center)
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
    let emphasized = emphasized_index(&crumbs);

    let mut path = Vec::with_capacity(crumbs.len() * 2);
    for (i, crumb) in crumbs.iter().enumerate() {
        if i > 0 {
            path.push(chevron(p));
        }
        path.push(crumb_view(crumb, emphasized == Some(i), p));
    }

    let errors = editor.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::ERROR).count();
    let warnings = editor.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::WARNING).count();
    let lang_label = editor.language.map(|l| l.label()).unwrap_or("Plain Text");

    let mut right = row![].spacing(12.0).align_y(Alignment::Center);
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
