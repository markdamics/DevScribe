//! Inline completion popup: shows up to 50 LSP completion items anchored near
//! the cursor, navigable with arrow keys, and committed with Tab/Enter.
//! Rendered as a `stack!` layer in `shell.rs`, same pattern as
//! `command_palette` and `context_menu`. Also renders the signature-help
//! popup (`signature_help_view`) — a separate popup, but close enough in
//! purpose (both are "help while typing a call") to share this module.
use devscribe_core::lsp::{self, CompletionItem, CompletionItemKind};
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Space};
use iced::{Border, Element, Length, Padding};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State};
use crate::ui::editor_canvas;
use crate::widgets;

/// Height of the title bar + tab bar combined — used to offset the popup
/// from the top of the window to the top of the editor content area.
/// Approximate; exact values depend on density/font-scale settings.
const HEADER_HEIGHT: f32 = 78.0;

/// Short, fixed-width tag for a completion item's kind — VS Code-style
/// single/double-letter glyphs rather than an icon font this app doesn't
/// have. `None` for kinds too rare in practice to bother tagging (`Color`,
/// `Folder`, `Event`, ...); those just show no tag.
fn kind_tag(kind: Option<CompletionItemKind>) -> Option<&'static str> {
    let kind = kind?;
    Some(if kind == CompletionItemKind::METHOD {
        "m"
    } else if kind == CompletionItemKind::FUNCTION || kind == CompletionItemKind::CONSTRUCTOR {
        "f"
    } else if kind == CompletionItemKind::FIELD || kind == CompletionItemKind::PROPERTY {
        "p"
    } else if kind == CompletionItemKind::VARIABLE {
        "v"
    } else if kind == CompletionItemKind::CLASS || kind == CompletionItemKind::STRUCT {
        "c"
    } else if kind == CompletionItemKind::INTERFACE {
        "i"
    } else if kind == CompletionItemKind::MODULE {
        "mod"
    } else if kind == CompletionItemKind::ENUM {
        "e"
    } else if kind == CompletionItemKind::ENUM_MEMBER {
        "em"
    } else if kind == CompletionItemKind::CONSTANT {
        "k"
    } else if kind == CompletionItemKind::KEYWORD {
        "kw"
    } else if kind == CompletionItemKind::SNIPPET {
        "sn"
    } else {
        return None;
    })
}

fn item_row(
    index: usize,
    item: &CompletionItem,
    selected: usize,
    p: Palette,
) -> Element<'static, Message> {
    let is_selected = index == selected;

    let label = text(item.label.clone())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(13.0))
        .color(color(if is_selected { p.text_strong } else { p.text_body }));

    let detail = item.detail.clone().map(|d| {
        text(d)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted))
    });

    let mut contents: Vec<Element<'static, Message>> = Vec::new();
    if let Some(tag) = kind_tag(item.kind) {
        contents.push(
            text(tag)
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(11.0))
                .color(color(p.accent_solid))
                .into(),
        );
    }
    contents.push(label.into());
    contents.push(Space::new().width(Length::Fill).into());
    if let Some(det) = detail {
        contents.push(det.into());
    }

    let inner: Element<'static, Message> = row(contents).spacing(6.0).align_y(iced::Alignment::Center).into();

    button(inner)
        .width(Length::Fill)
        .padding(Padding {
            top: 5.0,
            right: 10.0,
            bottom: 5.0,
            left: 10.0,
        })
        .on_press(Message::CompletionSelect)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if is_selected || hovered {
                    Some(color(p.surface_hover).into())
                } else {
                    None
                },
                text_color: color(p.text_strong),
                // Visible keyboard-selection indicator, not just a
                // background tint (accessibility pass, item 12).
                border: if is_selected {
                    Border { color: color(p.border_focus), width: 1.5, radius: 0.0.into() }
                } else {
                    Border::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// The selected item's documentation (LSP `documentation` field), shown as a
/// scrollable panel beside the list — `None` when the item has none, so the
/// caller can skip rendering the panel at all rather than showing empty
/// space.
fn doc_panel(item: &CompletionItem, p: Palette) -> Option<Element<'static, Message>> {
    let doc = item.documentation.clone()?;
    let text_content = lsp::documentation_to_text(doc);
    let text_content = text_content.trim();
    if text_content.is_empty() {
        return None;
    }
    let body = scrollable(
        text(text_content.to_string())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_body)),
    )
    .height(Length::Fixed(220.0));

    Some(
        container(body)
            .width(Length::Fixed(220.0))
            .padding(Padding { top: 8.0, right: 10.0, bottom: 8.0, left: 10.0 })
            .style(move |_theme| container::Style {
                background: Some(color(p.bg_base).into()),
                border: Border {
                    color: color(p.border_hairline),
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            })
            .into(),
    )
}

/// Footer key-hint strip shown under the completion list.
fn key_hints(p: Palette) -> Element<'static, Message> {
    container(
        text("\u{2191}\u{2193} navigate  \u{00b7}  Tab/Enter insert  \u{00b7}  Esc close")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(11.0))
            .color(color(p.text_muted)),
    )
    .padding(Padding { top: 4.0, right: 10.0, bottom: 5.0, left: 10.0 })
    .into()
}

pub fn view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let path = crate::state::active_file_path(state)?;
    let editor = crate::state::find_editor(state, &path)?;
    let items = editor.completions.as_ref()?;
    if items.is_empty() {
        return None;
    }

    let selected = editor.completion_selected.min(items.len().saturating_sub(1));
    let font_size = state.editor_font_size;

    let (x, y) = editor_canvas::cursor_pixel_pos_wrapped(
        &editor.document,
        state.word_wrap,
        editor.completion_anchor.line,
        editor.completion_anchor.col,
        font_size,
        editor.scroll_offset,
        editor.scroll_offset_x,
        HEADER_HEIGHT,
        if editor.viewport_width > 0.0 { editor.viewport_width } else { crate::state::ASSUMED_VIEWPORT_WIDTH },
    );

    let rows: Vec<Element<'static, Message>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| item_row(i, item, selected, p))
        .collect();

    let list = scrollable(column(rows)).height(Length::Fixed(220.0));

    let popup = container(column![list, widgets::hline(color(p.border_hairline)), key_hints(p)])
        .width(Length::Fixed(280.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        });

    let with_docs: Element<'static, Message> = match doc_panel(&items[selected], p) {
        Some(docs) => row![popup, docs].spacing(4.0).into(),
        None => popup.into(),
    };

    // Wrap in a mouse_area so clicks inside the popup don't propagate to the
    // editor canvas and accidentally dismiss the popup via EditorClick.
    let shielded = mouse_area(with_docs).on_press(Message::Noop);

    // Outer container fills the window; left/top padding positions the popup.
    let positioned = container(shielded)
        .padding(Padding {
            top: y.max(0.0),
            left: x.max(0.0),
            right: 0.0,
            bottom: 0.0,
        })
        .width(Length::Fill)
        .height(Length::Fill);

    Some(positioned.into())
}

/// The signature-help popup: the active signature's label, with the active
/// parameter highlighted in the accent color — shown just above the cursor
/// while typing a call's argument list. Independent of `view` above (both
/// can be showing at once); positioned the same way, at the request's
/// anchor rather than the live cursor, so it doesn't jump mid-request.
pub fn signature_help_view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let path = crate::state::active_file_path(state)?;
    let editor = crate::state::find_editor(state, &path)?;
    let help = editor.signature_help.as_ref()?;
    // Stale-position guard: if the cursor has left the line the request was
    // made for, the popup would be pinned to a position that no longer
    // means anything — better to show nothing than a stranded popup.
    if help.anchor.line != editor.cursor.line {
        return None;
    }
    let sig = help.signatures.get(help.active_signature)?;

    let font_size = state.editor_font_size;
    let (x, y) = editor_canvas::cursor_pixel_pos_wrapped(
        &editor.document,
        state.word_wrap,
        help.anchor.line,
        help.anchor.col,
        font_size,
        editor.scroll_offset,
        editor.scroll_offset_x,
        HEADER_HEIGHT,
        if editor.viewport_width > 0.0 { editor.viewport_width } else { crate::state::ASSUMED_VIEWPORT_WIDTH },
    );
    // One line height above the cursor rather than below it (unlike the
    // completion popup) — the argument list being typed is right at the
    // cursor, so below would sit on top of it.
    let line_h = editor_canvas::line_height_px(font_size);

    let label_row: Element<'static, Message> = match help
        .active_parameter
        .and_then(|idx| lsp::active_parameter_label(sig, idx))
        .and_then(|param| sig.label.find(param.as_str()).map(|byte| (byte, param)))
    {
        Some((start, param)) => {
            let end = start + param.len();
            row![
                text(sig.label[..start].to_string())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(12.0))
                    .color(color(p.text_muted)),
                text(sig.label[start..end].to_string())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(12.0))
                    .color(color(p.accent_solid)),
                text(sig.label[end..].to_string())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(12.0))
                    .color(color(p.text_muted)),
            ]
            .into()
        }
        None => text(sig.label.clone())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted))
            .into(),
    };

    let popup = container(label_row)
        .padding(Padding { top: 5.0, right: 10.0, bottom: 5.0, left: 10.0 })
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        });

    let shielded = mouse_area(popup).on_press(Message::Noop);

    let positioned = container(shielded)
        .padding(Padding {
            top: (y - line_h).max(0.0),
            left: x.max(0.0),
            right: 0.0,
            bottom: 0.0,
        })
        .width(Length::Fill)
        .height(Length::Fill);

    Some(positioned.into())
}
