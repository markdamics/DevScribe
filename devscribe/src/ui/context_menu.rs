use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, text, text_input, Space};
use iced::{Alignment, Border, Element, Length, Padding};
use std::path::Path;

use crate::color::color;
use crate::fonts;
use crate::state::{self, active_lsp_language, DraftKind, LspStatus, Message, State};
use crate::widgets;

fn menu_row(label: &'static str, shortcut: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    styled_menu_row(label, shortcut, message, color(p.text_strong), p)
}

/// "Delete"'s own row, in `status_danger` rather than the normal text
/// color — the one destructive entry in this menu, colored to stand apart
/// from New file/New folder/Rename/Copy path the way the mockup's other
/// danger-tinted UI (e.g. the sidebar's `Deleted` change badge) already
/// does.
fn danger_menu_row(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    styled_menu_row(label, "", message, color(p.status_danger), p)
}

fn styled_menu_row(
    label: &'static str,
    shortcut: &'static str,
    message: Message,
    text_color: iced::Color,
    p: Palette,
) -> Element<'static, Message> {
    button(
        row![
            text(label)
                .font(fonts::sans(iced::font::Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(text_color)
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

/// The confirm/cancel step shown once "Delete" has been clicked once
/// (`ContextMenu::confirm_delete`) — a plain-text warning plus two rows,
/// replacing the normal New file/New folder/Rename/Copy path/Delete rows
/// entirely rather than layering a second menu on top.
fn confirm_delete_rows(target: &Path, p: Palette) -> Vec<Element<'static, Message>> {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let noun = if target.is_dir() { "folder and everything in it" } else { "file" };
    vec![
        text(format!("Delete this {noun}?"))
            .font(fonts::sans(iced::font::Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_strong))
            .into(),
        text(name)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted))
            .into(),
        text("This can't be undone.")
            .font(fonts::sans(iced::font::Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.status_danger))
            .into(),
        widgets::hline(color(p.border_hairline)),
        menu_row("Cancel", "", Message::CloseTreeContext, p),
        danger_menu_row("Delete permanently", Message::DeletePath(target.to_path_buf()), p),
    ]
}

const TREE_MENU_WIDTH: f32 = 214.0;

/// A rough per-row height (button padding + text line height, or the
/// hairline's own sliver) used only to clamp the tree/editor context menus
/// to the bottom edge — see `clamp_menu_origin`. Doesn't need to be exact,
/// just close enough that the menu doesn't render mostly off-screen.
const MENU_ROW_HEIGHT: f32 = 32.0;

/// Clamps a menu's top-left corner so a box of `width`x`height` opening at
/// `(x, y)` stays fully inside a `window_width`x`window_height` viewport —
/// shared by the tree context menu and `editor_menu_view`, both of which
/// otherwise just open at the click position with no idea whether the menu
/// itself fits below/right of it.
fn clamp_menu_origin(x: f32, y: f32, width: f32, height: f32, window_width: f32, window_height: f32) -> (f32, f32) {
    let left = x.clamp(0.0, (window_width - width - 8.0).max(0.0));
    let top = y.clamp(0.0, (window_height - height - 8.0).max(0.0));
    (left, top)
}

pub fn view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let ctx = state.ctx_menu.as_ref()?;

    let target_label = ctx
        .target
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "PROJECT ROOT".to_string());

    let dir = match ctx.target.as_ref() {
        Some(path) if path.is_dir() => path.clone(),
        Some(path) => path.parent().map(Path::to_path_buf).unwrap_or_else(|| state.root.clone()),
        None => state.root.clone(),
    };

    let rows: Vec<Element<'static, Message>> = if let Some(target) = ctx.target.as_ref().filter(|_| ctx.confirm_delete) {
        confirm_delete_rows(target, p)
    } else {
        let mut rows: Vec<Element<'static, Message>> = vec![
            text(target_label.to_uppercase())
                .font(fonts::mono(Weight::Semibold))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted))
                .into(),
            menu_row("New file", "\u{2318}N", Message::BeginDraftIn(DraftKind::NewFile, dir.clone()), p),
            menu_row("New folder", "\u{21e7}\u{2318}N", Message::BeginDraftIn(DraftKind::NewFolder, dir), p),
        ];
        if let Some(target) = ctx.target.clone() {
            rows.push(widgets::hline(color(p.border_hairline)));
            rows.push(menu_row("Rename", "\u{21b5}", Message::BeginRename(target.clone()), p));
            rows.push(menu_row("Copy path", "\u{2325}\u{2318}C", Message::CopyPath(target.clone()), p));
            if !target.is_dir() {
                rows.push(menu_row("Open in New Window", "", Message::OpenInNewWindow(target.clone()), p));
            }
            rows.push(widgets::hline(color(p.border_hairline)));
            rows.push(danger_menu_row("Delete", Message::PromptDeletePath, p));
        }
        rows
    };

    let row_count = rows.len();
    let menu = container(column(rows).spacing(2.0).padding(6.0))
        .width(Length::Fixed(TREE_MENU_WIDTH))
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.5,
                radius: 3.0.into(),
            },
            ..container::Style::default()
        });

    let estimated_height = 12.0 + row_count as f32 * MENU_ROW_HEIGHT;
    let (left, top) = clamp_menu_origin(
        ctx.x,
        ctx.y,
        TREE_MENU_WIDTH,
        estimated_height,
        state.window_width,
        state.window_height,
    );
    let positioned = container(menu).width(Length::Fill).height(Length::Fill).padding(Padding {
        top,
        right: 0.0,
        bottom: 0.0,
        left,
    });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::CloseTreeContext)
    .on_right_press(Message::CloseTreeContext);

    Some(iced::widget::stack![backdrop, positioned].into())
}

const EDITOR_MENU_WIDTH: f32 = 230.0;

/// The editor canvas's right-click code-actions menu — distinct from
/// `view`, the sidebar tree's own, and positioned at the click itself
/// (clamped to stay fully on screen via `clamp_menu_origin`, now that
/// `State` tracks window height too) rather than `view`'s fixed spot, since
/// it can open anywhere in a scrolled, possibly very long file.
/// "Rename Symbol"/"Go to Definition"/"Find All References" only show
/// while a language server is actually ready — same guard those already
/// have from the keyboard/click paths — so this never offers an action
/// that would silently do nothing.
pub fn editor_menu_view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let menu = state.editor_ctx_menu.as_ref()?;
    let lsp_ready = matches!(state.lsp_status, LspStatus::Ready) && active_lsp_language(state).is_some();

    let mut rows: Vec<Element<'static, Message>> = Vec::new();
    if lsp_ready {
        rows.push(menu_row("Rename Symbol\u{2026}", "", Message::BeginRenameSymbol, p));
        rows.push(menu_row(
            "Go to Definition",
            "\u{2318}Click",
            Message::GoToDefinition { line: menu.line, col: menu.col },
            p,
        ));
        rows.push(menu_row(
            "Find All References",
            "\u{21e7}F12",
            Message::FindReferences { line: menu.line, col: menu.col },
            p,
        ));
        rows.push(widgets::hline(color(p.border_hairline)));
    }
    rows.push(menu_row("Search Symbol in Project\u{2026}", "", Message::SearchSymbolInProject, p));
    let row_count = rows.len();

    let menu_box = container(column(rows).spacing(2.0).padding(6.0))
        .width(Length::Fixed(EDITOR_MENU_WIDTH))
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border {
                color: color(p.border_hairline),
                width: 1.5,
                radius: 3.0.into(),
            },
            ..container::Style::default()
        });

    let estimated_height = 12.0 + row_count as f32 * MENU_ROW_HEIGHT;
    let (left, top) = clamp_menu_origin(
        menu.x,
        menu.y,
        EDITOR_MENU_WIDTH,
        estimated_height,
        state.window_width,
        state.window_height,
    );
    let positioned = container(menu_box).width(Length::Fill).height(Length::Fill).padding(Padding {
        top,
        right: 0.0,
        bottom: 0.0,
        left,
    });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::CloseEditorContext)
    .on_right_press(Message::CloseEditorContext);

    Some(iced::widget::stack![backdrop, positioned].into())
}

/// The "Rename Symbol" floating prompt — a small `text_input` seeded with
/// whatever the click "Rename Symbol" was opened from turned out to be
/// (`EditorState::word_at`). Centered near the top of the window, same
/// fixed-spot idiom `command_palette.rs` itself uses, rather than following
/// the click the way `editor_menu_view` does — `editor_ctx_menu` (which
/// carries the click's screen position) is already gone by the time this
/// shows (`Message::BeginRenameSymbol` consumes it), and a small input
/// box benefits less from being right under the cursor than a menu of
/// click targets does.
pub fn rename_prompt_view(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    let prompt = state.rename_prompt.as_ref()?;

    let input = text_input("new name\u{2026}", &prompt.query)
        .id(state::rename_prompt_id())
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(15.0))
        .padding([6.0, 8.0])
        .on_input(Message::RenameQueryChanged)
        .on_submit(Message::ConfirmRenameSymbol)
        .style(move |_theme, _status| text_input::Style {
            background: color(p.surface_inset).into(),
            border: Border {
                color: color(p.border_accent),
                width: 1.5,
                radius: 3.0.into(),
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

    let hint = text("Enter to rename \u{2022} Esc to cancel")
        .font(fonts::sans(Weight::Medium))
        .size(crate::text_scale::px(12.0))
        .color(color(p.text_muted));

    let prompt_box = container(
        column![
            text("RENAME SYMBOL")
                .font(fonts::mono(Weight::Semibold))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_muted)),
            input,
            hint,
        ]
        .spacing(6.0),
    )
    .width(Length::Fixed(260.0))
    .padding(10.0)
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border {
            color: color(p.border_accent),
            width: 1.5,
            radius: 5.0.into(),
        },
        ..container::Style::default()
    });

    // Shields the prompt from the backdrop below, same reasoning
    // `command_palette.rs`'s own `panel` shield gives — the hint text is
    // plain, non-capturing, so without this a click on it would fall
    // through to the backdrop's `on_press` and cancel the prompt.
    let prompt_box = mouse_area(prompt_box).on_press(Message::Noop);

    let centered = container(prompt_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .padding(Padding { top: 160.0, right: 0.0, bottom: 0.0, left: 0.0 });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::CancelRenameSymbol);

    Some(iced::widget::stack![backdrop, centered].into())
}
