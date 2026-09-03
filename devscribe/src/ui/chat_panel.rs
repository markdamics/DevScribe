//! The AI Chat Assist panel. One shared message-thread + input bar
//! (`thread_view`) is reused verbatim across the two modes with real room
//! to show it — docked, full tab (the default — see `toggle_chat`'s own
//! doc comment) — each wrapped in a thin header with a "View" popup for
//! switching between them. The collapsed rail is the one genuinely
//! different shape: a narrow icon strip, mirroring `sidebar.rs`'s own
//! collapsed rail.
//!
//! A tool call is rendered from whatever `state.chat`'s `ToolActivity`
//! knows at the time — before a decision, that's just `tool.input` (e.g.
//! an `Edit`'s `old_string`/`new_string`), which is enough for the
//! Approve/Deny card's diff preview; there's no separate "apply" step
//! afterward the way the original mockup's diff card had one, since the
//! permission gate already runs *before* `claude` performs the edit (see
//! `devscribe_core::claude_agent`) — approving *is* applying.
use devscribe_core::claude_agent::{PermissionMode, SessionSummary};
use devscribe_core::theme::{Palette, Rgba};
use iced::font::Weight;
use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_editor, Space};
use iced::{Alignment, Border, Color, Element, Length};

use std::path::PathBuf;
use std::time::SystemTime;

use crate::color::color;
use crate::fonts;
use crate::recent_projects;
use crate::state::{chat_is_active, ChatMessage, ChatMode, ChatProvider, ChatStatus, Message, PermissionState, State, TabKey, ToolActivity};
use crate::ui::status_bar;
use crate::widgets;

/// A short, human-readable one-liner for a tool call — the field that
/// actually says what's happening, per tool, falling back to just the
/// tool's name for anything unrecognized (custom/MCP tools included).
fn tool_summary(name: &str, input: &serde_json::Value) -> String {
    let field = |key: &str| input.get(key).and_then(|v| v.as_str());
    match name {
        "Read" | "Edit" | "Write" => field("file_path").map(|p| format!("{name} {p}")).unwrap_or_else(|| name.to_string()),
        "Glob" => field("pattern").map(|p| format!("Glob {p}")).unwrap_or_else(|| "Glob".to_string()),
        "Grep" => field("pattern").map(|p| format!("Grep {p}")).unwrap_or_else(|| "Grep".to_string()),
        "Bash" => field("command").map(|c| format!("Bash {c}")).unwrap_or_else(|| "Bash".to_string()),
        other => other.to_string(),
    }
}

/// Small mono text-label button for panel-chrome actions (mode switches,
/// dock/close) — this app's established alternative to icon glyphs
/// wherever a proven-to-render glyph isn't already in use elsewhere.
fn header_button(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(widgets::center_v(
        text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)),
    ))
    .padding([0.0, 8.0])
    .height(Length::Fixed(22.0))
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            // A translucent overlay, not an opaque swap: this button sits on
            // `panel_header`'s `surface_raised` background in some call
            // sites and `bg_base` in others (`session_list_view`'s header,
            // `collapsed_rail`) — an opaque `surface_raised` hover would be
            // a no-op in the first case, so this needs to visibly lighten
            // *whatever* it's already sitting on.
            background: if hovered { Some(color(p.surface_hover).into()) } else { None },
            text_color: if hovered { color(p.accent_solid) } else { color(p.text_muted) },
            border: Border { color: if hovered { color(p.border_accent) } else { color(p.border_hairline) }, width: 1.0, radius: 3.0.into() },
            ..button::Style::default()
        }
    })
    .into()
}

fn primary_button(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(text(label).font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.accent_on)))
        .padding([5.0, 14.0])
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: Some(if hovered { color(p.accent_solid_hover) } else { color(p.accent_solid) }.into()),
                border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 3.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

/// A minimal, borderless text action for inline use in an activity row —
/// lighter-weight than `secondary_button`, which is sized for a card's own
/// button row rather than sitting flush at the end of a one-line summary.
fn link_button(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)))
        .padding([0.0, 4.0])
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: None,
                text_color: if hovered { color(p.accent_solid) } else { color(p.text_muted) },
                ..button::Style::default()
            }
        })
        .into()
}

fn secondary_button(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(text(label).font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(13.0)))
        .padding([5.0, 14.0])
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered { Some(color(p.surface_raised).into()) } else { None },
                text_color: color(p.text_strong),
                border: Border { color: color(p.border_strong), width: 1.5, radius: 3.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

fn operator_row(text_content: &str, p: Palette) -> Element<'static, Message> {
    column![
        widgets::micro("OPERATOR", color(p.text_muted)),
        container(
            text(text_content.to_string())
                .font(fonts::sans(Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_strong))
        )
        .padding([8.0, 10.0])
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_inset).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 3.0.into() },
            ..container::Style::default()
        }),
    ]
    .spacing(6.0)
    .into()
}

/// `caret_visible` (state.rs's existing ~530ms blink tick) only ever
/// applies while `streaming` is true — a finalized bubble never shows the
/// live-typing caret, matching VS Code's own streaming indicator.
fn assistant_row(text_content: &str, streaming: bool, caret_visible: bool, provider: ChatProvider, p: Palette) -> Element<'static, Message> {
    let shown = if streaming && caret_visible { format!("{text_content}\u{258c}") } else { text_content.to_string() };
    column![
        row![widgets::dot(color(p.accent_solid), 5.0), widgets::micro(provider.label(), color(p.text_muted))]
            .spacing(7.0)
            .align_y(Alignment::Center),
        text(shown).size(crate::text_scale::px(15.0)).color(color(p.text_body)),
    ]
    .spacing(6.0)
    .into()
}

/// A pending tool call gated on a human decision — the diff preview comes
/// straight from `old_string`/`new_string` on `Edit`'s own `tool.input`
/// (all that's known *before* the edit happens); nothing richer
/// (`structuredPatch` etc.) exists yet at this point, since that's only in
/// the eventual `ToolResult`.
fn permission_card(tool: &ToolActivity, p: Palette) -> Element<'static, Message> {
    let mut content = column![
        row![widgets::dot(color(p.status_warning), 6.0), widgets::micro("PERMISSION NEEDED", color(p.status_warning))]
            .spacing(7.0)
            .align_y(Alignment::Center),
        text(tool_summary(&tool.name, &tool.input))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_strong)),
    ]
    .spacing(8.0);

    if let (Some(old), Some(new)) =
        (tool.input.get("old_string").and_then(|v| v.as_str()), tool.input.get("new_string").and_then(|v| v.as_str()))
    {
        content = content.push(
            container(
                column![
                    text(format!("- {old}"))
                        .font(fonts::mono(Weight::Normal))
                        .size(crate::text_scale::px(12.5))
                        .color(color(p.status_danger)),
                    text(format!("+ {new}"))
                        .font(fonts::mono(Weight::Normal))
                        .size(crate::text_scale::px(12.5))
                        .color(color(p.status_success)),
                ]
                .spacing(2.0),
            )
            .padding(8.0)
            .width(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(color(p.bg_canvas).into()),
                border: Border { color: color(p.border_hairline), width: 1.0, radius: 3.0.into() },
                ..container::Style::default()
            }),
        );
    }

    content = content.push(
        row![
            primary_button("Approve", Message::ChatApprovePermission(tool.id.clone()), p),
            secondary_button("Deny", Message::ChatDenyPermission(tool.id.clone()), p),
        ]
        .spacing(8.0),
    );

    container(content)
        .padding(10.0)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.status_warning), width: 1.5, radius: 4.0.into() },
            ..container::Style::default()
        })
        .into()
}

/// A tool call that either never needed a decision (Read/Grep/...), or
/// already has one (approved/denied) — a single compact line: pulsing
/// while still running (no `result` yet), a check or a cross once it's
/// settled. `caret_visible` (state.rs's existing ~530ms blink tick,
/// already driving the editor's own caret) is reused for the pulse rather
/// than standing up a dedicated animation — this app has no CSS keyframes
/// to lean on, and this is the same "is something still happening" signal
/// either way.
fn tool_activity_row(tool: &ToolActivity, caret_visible: bool, p: Palette) -> Element<'static, Message> {
    let (glyph, glyph_color): (&str, Rgba) = match (&tool.permission, &tool.result) {
        (Some(PermissionState::Denied), _) => ("\u{2715}", p.status_danger),
        (_, Some(r)) if r.is_error => ("\u{2715}", p.status_danger),
        (_, Some(_)) => ("\u{2713}", p.status_success),
        (_, None) => (if caret_visible { "\u{25cf}" } else { "\u{25cb}" }, p.accent_solid),
    };

    let mut content = row![
        text(glyph).font(fonts::mono(Weight::Bold)).size(crate::text_scale::px(12.0)).color(color(glyph_color)),
        text(tool_summary(&tool.name, &tool.input))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
    ]
    .spacing(7.0)
    .align_y(Alignment::Center)
    .width(Length::Fill);

    // A settled, successful, actually-approved Edit/Write — the file has
    // really landed on disk (the permission gate runs *before* `claude`
    // touches it, so a completed result here means it went through) —
    // gets a jump to the existing diff view instead of DevScribe trying to
    // re-render its own copy of the change.
    let landed_edit = matches!(tool.name.as_str(), "Edit" | "Write")
        && tool.permission != Some(PermissionState::Denied)
        && tool.result.as_ref().is_some_and(|r| !r.is_error);
    if landed_edit
        && let Some(path) = tool.input.get("file_path").and_then(|v| v.as_str())
    {
        content = content.push(Space::new().width(Length::Fill));
        content = content.push(link_button("Open diff", Message::OpenDiffFor(PathBuf::from(path)), p));
    }

    content.into()
}

fn message_row(msg: &ChatMessage, caret_visible: bool, provider: ChatProvider, p: Palette) -> Element<'static, Message> {
    match msg {
        ChatMessage::Operator(text_content) => operator_row(text_content, p),
        ChatMessage::Assistant { text, streaming } => assistant_row(text, *streaming, caret_visible, provider, p),
        ChatMessage::Tool(tool) if tool.permission == Some(PermissionState::Pending) => permission_card(tool, p),
        ChatMessage::Tool(tool) => tool_activity_row(tool, caret_visible, p),
    }
}

fn status_line(state: &State, p: Palette) -> Element<'static, Message> {
    let label = match &state.chat.status {
        ChatStatus::Starting => format!("STARTING {}\u{2026}", state.chat_provider.label().to_uppercase()),
        ChatStatus::Unavailable(reason) => format!("UNAVAILABLE \u{2014} {reason}"),
        ChatStatus::Ready => {
            let fallback = state.chat_provider.label().to_lowercase();
            let model = state.chat.model.as_deref().unwrap_or(&fallback);
            format!(
                "{model} \u{2014} {}+{} TOK",
                state.chat.input_tokens, state.chat.output_tokens
            )
        }
    };
    let label_color = if matches!(state.chat.status, ChatStatus::Unavailable(_)) { p.status_danger } else { p.text_muted };
    text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(label_color)).into()
}

fn mode_button(mode: PermissionMode, active: PermissionMode, p: Palette) -> Element<'static, Message> {
    let is_active = mode == active;
    button(text(mode.label()).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)))
        .padding([2.0, 7.0])
        .on_press(Message::ChatSetPermissionMode(mode))
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if is_active { Some(color(p.accent_quiet).into()) } else if hovered { Some(color(p.surface_raised).into()) } else { None },
                text_color: if is_active { color(p.accent_solid) } else { color(p.text_muted) },
                border: Border { color: if is_active { color(p.border_accent) } else { color(p.border_hairline) }, width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

/// VS Code's own permission-mode picker, translated to a row of toggle
/// buttons rather than a dropdown — same simplification the session picker
/// makes (see its own doc comment): no positioning math needed, and with
/// only four options a row reads at a glance anyway.
fn mode_selector(state: &State, p: Palette) -> Element<'static, Message> {
    let modes = row(PermissionMode::ALL.into_iter().map(|mode| mode_button(mode, state.chat_permission_mode, p))).spacing(4.0);
    row![modes, Space::new().width(Length::Fill), link_button("Design Login\u{2026}", Message::ChatLaunchDesignLogin, p)]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .into()
}

/// The input bar's "+" trigger for the Actions popup — square, matching
/// the source design's own `+` button footprint. Highlighted while open,
/// same treatment as `view_menu_button`.
fn actions_menu_button(open: bool, p: Palette) -> Element<'static, Message> {
    button(widgets::center_fill(text("+").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(16.0))))
        .width(Length::Fixed(32.0))
        .height(Length::Fixed(32.0))
        .padding(0.0)
        .on_press(Message::ChatToggleActions)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: Some(if open || hovered { color(p.surface_raised) } else { color(p.surface_inset) }.into()),
                text_color: if open || hovered { color(p.accent_solid) } else { color(p.text_muted) },
                border: Border { color: if open || hovered { color(p.border_accent) } else { color(p.border_strong) }, width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

fn actions_section_label(label: &'static str, p: Palette) -> Element<'static, Message> {
    container(widgets::micro(label, color(p.text_muted))).padding(iced::Padding { top: 8.0, right: 10.0, bottom: 2.0, left: 10.0 }).into()
}

/// A plain Actions-popup row — `shortcut`, when given, is shown flush
/// right (e.g. `⌘U`, or the current model as a hint on "Switch model…").
/// Takes an owned `String` rather than `&'static str` since the model-name
/// hint is only known at render time.
fn action_row(label: &'static str, shortcut: Option<String>, message: Message, p: Palette) -> Element<'static, Message> {
    let mut content =
        row![text(label).font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(13.5)).color(color(p.text_strong)).width(Length::Fill)]
            .align_y(Alignment::Center);
    if let Some(shortcut) = shortcut {
        content = content.push(text(shortcut).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)));
    }
    button(content)
        .width(Length::Fill)
        .padding([6.0, 10.0])
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            // `surface_hover` (a translucent overlay), not `surface_raised`
            // (an opaque swap) — this row sits inside a popup whose own
            // background already *is* `surface_raised`, so an opaque swap
            // to the same color would be an invisible no-op on hover. The
            // accent border rides along so the highlight reads clearly even
            // at the overlay's necessarily-subtle opacity.
            button::Style {
                background: if hovered { Some(color(p.surface_hover).into()) } else { None },
                border: Border { color: if hovered { color(p.border_accent) } else { Color::TRANSPARENT }, width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

/// An Actions-popup row with an ON/OFF badge instead of a shortcut — same
/// visual idiom as `settings_panel::toggle_row`, sized for a menu row.
fn action_toggle_row(label: &'static str, enabled: bool, message: Message, p: Palette) -> Element<'static, Message> {
    button(
        row![
            text(label).font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(13.5)).color(color(p.text_strong)).width(Length::Fill),
            text(if enabled { "ON" } else { "OFF" })
                .font(fonts::mono(Weight::Bold))
                .size(crate::text_scale::px(11.0))
                .color(if enabled { color(p.status_success) } else { color(p.text_muted) }),
        ]
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([6.0, 10.0])
    .on_press(message)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.surface_hover).into()) } else { None },
            border: Border { color: if hovered { color(p.border_accent) } else { Color::TRANSPARENT }, width: 1.0, radius: 3.0.into() },
            ..button::Style::default()
        }
    })
    .into()
}

/// Where the chat panel is currently presented — the only two shapes that
/// actually show an input bar or a "View" popup trigger (Collapsed has
/// neither, Closed has nothing at all). Used solely to compute
/// `view_menu`/`actions_menu`'s absolute on-screen position now that both
/// are top-level `shell.rs` layers (see their own doc comments for why).
enum ChatPresentation {
    Docked,
    Tab,
}

fn current_presentation(state: &State) -> Option<ChatPresentation> {
    if state.chat_mode == ChatMode::Docked {
        Some(ChatPresentation::Docked)
    } else if state.chat_tab_open && state.active_tab == Some(TabKey::Chat) {
        Some(ChatPresentation::Tab)
    } else {
        None
    }
}

/// The Actions popup's rows, which depend on the active provider: almost
/// everything the popup offers is `claude`-CLI-specific machinery rather
/// than generic chat. "Attach file…"/"Mention file…" insert an `@path`,
/// which is `claude`'s own file-reference syntax (see `insert_chat_mention`)
/// and reaches Copilot as nothing but literal text — Copilot takes file
/// context through `conversation/*`'s own `references` parameter instead,
/// which `copilot_agent` doesn't send yet. "Switch model…", "Thinking" and
/// "Account & usage" are all just `/model`, `/effort` and `/usage` sent as
/// prompts, which only `claude` answers itself. "Shell Access" maps to
/// `SessionOptions::allow_bash`, which `copilot_agent` ignores outright.
///
/// So under Copilot the popup honestly reduces to "Clear conversation" —
/// which does work there (a new session id re-keys `chat_worker`'s
/// subscription, so the worker respawns onto a fresh conversation). A real
/// Copilot model picker belongs here eventually — `copilot_agent` already
/// fetches the account's model list to choose a default — but wiring the
/// choice back through the subscription key is its own change, and a
/// non-functional row in the meantime is exactly what this avoids.
fn actions_menu_items(state: &State, p: Palette) -> Vec<Element<'static, Message>> {
    if !state.chat_provider.is_claude_cli() {
        return vec![
            actions_section_label("SESSION", p),
            action_row("Clear conversation", None, Message::ChatNewSession, p),
        ];
    }

    let model = state.chat.model.clone().unwrap_or_else(|| "\u{2014}".to_string());
    vec![
        actions_section_label("CONTEXT", p),
        action_row("Attach file\u{2026}", Some("\u{2318}U".to_string()), Message::ChatAttachFileDialog, p),
        action_row("Mention file from this project\u{2026}", Some("@".to_string()), Message::ChatMentionFileDialog, p),
        action_row("Clear conversation", None, Message::ChatNewSession, p),
        widgets::hline(color(p.border_hairline)),
        actions_section_label("MODEL", p),
        action_row("Switch model\u{2026}", Some(model), Message::ChatShowModel, p),
        action_toggle_row("Thinking", state.chat_thinking_enabled, Message::ChatToggleThinking, p),
        widgets::hline(color(p.border_hairline)),
        actions_section_label("PERMISSIONS", p),
        action_toggle_row("Shell Access", state.chat_shell_access_enabled, Message::ChatToggleShellAccess, p),
        widgets::hline(color(p.border_hairline)),
        action_row("Account & usage\u{2026}", None, Message::ChatShowUsage, p),
    ]
}

/// The input bar's "+" Actions popup, as a `shell.rs` top-level layer (see
/// `shell::view`'s `layers.extend(chat_panel::actions_menu(...))`) —
/// *not* stacked locally inside `input_bar`'s own element the way a first
/// attempt at this did. That local attempt bottom-anchored the popup
/// within `input_bar`'s own box, which sounds right until you notice
/// `input_bar` itself sits at the *bottom* of the panel (pushed down by
/// the message list's `Length::Fill` above it in `thread_view`) — nested a
/// few `container`/`column` levels deep inside layout slots sized to their
/// own content, a popup taller than the box it's nested in doesn't reliably
/// paint outside that box, so it either ran off-window (top-anchored) or
/// stayed clipped (bottom-anchored). A true top-level layer, positioned
/// with absolute pixel offsets from `state.density`'s known bar heights —
/// same approach `tab_bar::overflow_menu` already uses successfully — has
/// no such parent box to fight.
pub fn actions_menu(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.chat_actions_open {
        return None;
    }
    let presentation = current_presentation(state)?;
    let left = match presentation {
        // The docked panel's own left edge — it's flush against the
        // window's right edge, so this is the only offset that matters.
        ChatPresentation::Docked => (state.window_width - state.chat_panel_width).max(0.0),
        // The content area's own left edge, past the sidebar (or its
        // collapsed rail) and its resize handle.
        ChatPresentation::Tab => {
            if state.sidebar_collapsed {
                40.0
            } else {
                state.sidebar_width + 4.0
            }
        }
    };
    let bottom = state.density.status_bar_h() + if state.problems_panel_open { status_bar::PROBLEMS_PANEL_H } else { 0.0 };

    let menu = container(column(actions_menu_items(state, p)).spacing(2.0).padding(6.0))
    .width(Length::Fixed(280.0))
    .style(move |_theme| container::Style {
        background: Some(color(p.surface_raised).into()),
        border: Border { color: color(p.border_hairline), width: 1.5, radius: 4.0.into() },
        ..container::Style::default()
    });

    let positioned = container(menu)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(iced::Padding { top: 0.0, right: 0.0, bottom, left });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill)).width(Length::Fill).height(Length::Fill),
    )
    .on_press(Message::ChatToggleActions);

    Some(iced::widget::stack![backdrop, positioned].into())
}

/// Static hint labels for the input's built-in trigger characters — `@`
/// mentions a project file (see `ChatMentionFileDialog`); `#`/`/` are
/// `claude`'s own conventions (a quick memory note, a skill/slash command)
/// that DevScribe doesn't intercept in any way — the CLI sees them in the
/// prompt text exactly as typed.
fn hint_row(p: Palette) -> Element<'static, Message> {
    row![
        text("@ file").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)),
        text("# symbol").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)),
        text("/ command").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)),
    ]
    .spacing(12.0)
    .into()
}

/// The multi-line draft editor — a real `text_editor`, not a single-line
/// `text_input`, specifically so cursor movement, click/drag text
/// selection, and multi-line prompts (Shift+Enter) all work the way
/// they'd work in any other text field. Plain Enter is intercepted via
/// `key_binding` to submit instead of inserting a newline; every other key
/// (Shift+Enter included) falls through to `Binding::from_key_press`,
/// which reproduces the widget's own normal behavior unchanged.
fn input_bar(state: &State, p: Palette) -> Element<'_, Message> {
    let enabled = state.chat.sender.is_some();
    let placeholder = if enabled {
        "Ask about this project\u{2026} (Shift+Enter for a new line)".to_string()
    } else {
        format!("Waiting for {} to start\u{2026}", state.chat_provider.label())
    };

    let mut input = text_editor(&state.chat.input)
        .placeholder(placeholder)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(14.0))
        .padding(10.0)
        .max_height(120.0)
        .key_binding(|press| {
            if press.key == Key::Named(Named::Enter) && !press.modifiers.shift() {
                Some(text_editor::Binding::Custom(Message::ChatSubmit))
            } else {
                text_editor::Binding::from_key_press(press)
            }
        })
        .style(move |_theme, _status| text_editor::Style {
            background: color(p.surface_inset).into(),
            border: Border { color: color(p.border_strong), width: 1.5, radius: 3.0.into() },
            placeholder: color(p.text_muted),
            value: color(p.text_strong),
            selection: {
                let mut c = p.accent_solid;
                c.a = 0.35;
                color(c)
            },
        });
    if enabled {
        input = input.on_action(Message::ChatInputAction);
    }

    let input_row = row![actions_menu_button(state.chat_actions_open, p), input].spacing(8.0).align_y(Alignment::Center);

    // The permission-mode row (Manual/Auto-Edit/Plan/Auto) and "Design
    // Login…" are both `claude`-specific — `copilot_agent::run` ignores
    // `chat_permission_mode` entirely (see its own doc comment), and
    // "Design Login…" launches a terminal running `claude` by name. Showing
    // either under Copilot would be the same kind of "looks like it should
    // do something but doesn't" trap the hardcoded Claude-branded
    // placeholder text used to be.
    let mut items: Vec<Element<'_, Message>> = Vec::new();
    if state.chat_provider.is_claude_cli() {
        items.push(mode_selector(state, p));
    }
    items.push(input_row.into());
    items.push(hint_row(p));
    items.push(status_line(state, p));
    column(items).spacing(6.0).into()
}

/// The message list's top row while `state.chat.history_truncated` — a
/// resumed session's saved transcript has more history than got replayed
/// (see `claude_agent::load_session_history`'s cap), and this is the escape
/// hatch back to the complete conversation.
fn load_earlier_row(p: Palette) -> Element<'static, Message> {
    button(widgets::center_fill(
        text("Load earlier messages")
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted)),
    ))
    .width(Length::Fill)
    .padding([8.0, 10.0])
    .on_press(Message::LoadEarlierChatHistory)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.surface_raised).into()) } else { None },
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 4.0.into() },
            ..button::Style::default()
        }
    })
    .into()
}

fn session_row(session: &SessionSummary, p: Palette) -> Element<'static, Message> {
    let ms = session.last_active.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let when = recent_projects::relative_label(ms);

    button(
        row![
            text(session.title.clone())
                .font(fonts::sans(Weight::Medium))
                .size(crate::text_scale::px(14.0))
                .color(color(p.text_strong))
                .width(Length::Fill),
            text(when).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8.0, 10.0])
    .on_press(Message::ChatResumeSession(session.id.clone()))
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.surface_raised).into()) } else { None },
            ..button::Style::default()
        }
    })
    .into()
}

/// The "Sessions" picker — replaces the message thread + input bar
/// entirely while open (`state.chat_sessions_open`), in all three modes
/// that share `thread_view`, rather than floating as a separate overlay:
/// simpler than positioning a dropdown relative to whichever header
/// button opened it across three differently-shaped presentations, and a
/// perfectly ordinary pattern for a chat app's own history view.
fn session_list_view(state: &State, p: Palette) -> Element<'static, Message> {
    let header = row![
        widgets::micro("SESSIONS", color(p.text_muted)),
        Space::new().width(Length::Fill),
        header_button("\u{2715}", Message::ChatToggleSessions, p),
    ]
    .align_y(Alignment::Center)
    .padding([8.0, 10.0]);

    let new_session = button(
        row![widgets::dot(color(p.accent_solid), 6.0), text("New session").font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(14.0))]
            .spacing(8.0)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8.0, 10.0])
    .on_press(Message::ChatNewSession)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.surface_raised).into()) } else { None },
            text_color: color(p.accent_solid),
            ..button::Style::default()
        }
    });

    let list: Element<'static, Message> = if state.chat_sessions.is_empty() {
        widgets::placeholder("No past sessions for this project yet", p)
    } else {
        let rows: Vec<Element<'static, Message>> = state.chat_sessions.iter().map(|s| session_row(s, p)).collect();
        scrollable(column(rows).spacing(2.0).padding([4.0, 8.0]).width(Length::Fill)).width(Length::Fill).height(Length::Fill).into()
    };

    column![header, widgets::hline(color(p.border_hairline)), new_session, widgets::hline(color(p.border_hairline)), list]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The message list + input bar shared by docked/window/tab presentation
/// — or, while `state.chat_sessions_open`, the session picker in their
/// place (see `session_list_view`).
pub fn thread_view(state: &State, p: Palette) -> Element<'_, Message> {
    if state.chat_sessions_open {
        return session_list_view(state, p);
    }

    let mut rows: Vec<Element<'static, Message>> =
        state.chat.messages.iter().map(|m| message_row(m, state.caret_visible, state.chat_provider, p)).collect();
    if state.chat.history_truncated {
        rows.insert(0, load_earlier_row(p));
    }

    let list: Element<'static, Message> = if rows.is_empty() {
        // An `Unavailable` reason (binary missing, spawn failure, ...) is
        // otherwise only visible in `status_line`'s small, muted text at the
        // very bottom of the panel — easy to miss, especially right after
        // switching providers when the *previous* provider's messages just
        // disappeared and this empty state is the only other thing on
        // screen. Surface it here too, the same prominent way VS Code shows
        // "Copilot isn't signed in" inline in the chat pane rather than only
        // in a status bar.
        match &state.chat.status {
            ChatStatus::Unavailable(reason) => widgets::placeholder(reason.clone(), p),
            _ => widgets::placeholder(format!("Ask {} about this project to get started", state.chat_provider.label()), p),
        }
    } else {
        scrollable(column(rows).spacing(16.0).padding(16.0).width(Length::Fill))
            .id(crate::state::chat_scroll_id())
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    };

    column![
        container(list).width(Length::Fill).height(Length::Fill),
        container(input_bar(state, p)).width(Length::Fill).padding(12.0).style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        }),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// The header's trailing buttons, shared by both presentations.
/// "THREADS" opens the saved-session picker, which reads `claude`'s own
/// on-disk transcripts (`claude_agent::list_sessions`) — there are no such
/// transcripts under Copilot, and `copilot_agent` doesn't resume sessions at
/// all, so the button is Claude-only rather than an empty list to nowhere.
fn header_buttons(state: &State, p: Palette) -> Vec<Element<'static, Message>> {
    let mut buttons = Vec::new();
    if state.chat_provider.is_claude_cli() {
        buttons.push(header_button("THREADS", Message::ChatToggleSessions, p));
    }
    buttons.push(view_menu_button(state.chat_view_menu_open, p));
    buttons
}

fn provider_button(provider: ChatProvider, active: ChatProvider, p: Palette) -> Element<'static, Message> {
    let is_active = provider == active;
    button(text(provider.label()).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)))
        .padding(0.0)
        .on_press(Message::ChatSetProvider(provider))
        .style(move |_theme, status| button::Style {
            text_color: if is_active {
                color(p.accent_solid)
            } else if status == button::Status::Hovered {
                color(p.text_strong)
            } else {
                color(p.text_muted)
            },
            ..button::Style::default()
        })
        .into()
}

/// The panel header's own title slot, replaced by the AI switcher (a status
/// dot plus one clickable label per provider) rather than a static
/// "CLAUDE CODE"/"GITHUB COPILOT" string — this is the one place in the app
/// a human is guaranteed to be looking at the chat panel itself, so it's a
/// more natural home for "which backend am I talking to, and can I change
/// it" than a strip that's hidden while the panel is actually open as a tab
/// (`breadcrumb_bar` only renders for a code-file tab).
///
/// The dot reports the chat session's health, but only while a session
/// should actually exist (`chat_is_active`); with the panel closed there's
/// no worker running, so `state.chat.status` is just whatever the last one
/// left behind and reporting it would be stale. In practice this function is
/// only ever called while the panel *is* open (`docked_view`/`tab_view`), so
/// the dot is never actually shown in that stale state — the check is
/// defensive, matching `chat_is_active`'s own doc comment on why it's not
/// simply `state.chat_mode != Closed`.
fn ai_switcher(state: &State, p: Palette) -> Element<'static, Message> {
    let dot_color = if !chat_is_active(state) {
        p.text_muted
    } else {
        match state.chat.status {
            ChatStatus::Ready => p.status_success,
            ChatStatus::Starting => p.status_warning,
            ChatStatus::Unavailable(_) => p.status_danger,
        }
    };

    let mut content = row![widgets::dot(color(dot_color), 6.0)].spacing(7.0).align_y(Alignment::Center);
    for provider in ChatProvider::ALL {
        content = content.push(provider_button(provider, state.chat_provider, p));
    }
    content.into()
}

fn panel_header(switcher: Element<'static, Message>, p: Palette, buttons: Vec<Element<'static, Message>>) -> Element<'static, Message> {
    let mut content = row![switcher]
        .spacing(10.0)
        .align_y(Alignment::Center);
    content = content.push(Space::new().width(Length::Fill));
    for b in buttons {
        content = content.push(b);
    }

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(34.0))
        .padding([0.0, 10.0])
        .align_y(Alignment::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}

/// The header's "View" trigger button — opens `view_menu`'s popup.
/// Highlighted the same way while open as while hovered, mirroring
/// `tab_bar`'s `⋯` overflow button.
fn view_menu_button(open: bool, p: Palette) -> Element<'static, Message> {
    button(widgets::center_v(
        row![
            text("VIEW").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)),
            text("\u{25be}").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(10.0)),
        ]
        .spacing(4.0)
        .align_y(Alignment::Center),
    ))
    .padding([0.0, 8.0])
    .height(Length::Fixed(22.0))
    .on_press(Message::ChatToggleViewMenu)
    .style(move |_theme, status| {
        let hovered = status == button::Status::Hovered;
        let active = open || hovered;
        button::Style {
            // `surface_hover`, not `surface_raised`, for the plain-hover
            // case — this button sits on `panel_header`'s `surface_raised`
            // background, where an opaque same-color swap would be a no-op.
            background: if open { Some(color(p.accent_quiet).into()) } else if hovered { Some(color(p.surface_hover).into()) } else { None },
            text_color: if active { color(p.accent_solid) } else { color(p.text_muted) },
            border: Border { color: if active { color(p.border_accent) } else { color(p.border_hairline) }, width: 1.0, radius: 3.0.into() },
            ..button::Style::default()
        }
    })
    .into()
}

fn view_menu_row(label: &'static str, message: Message, p: Palette) -> Element<'static, Message> {
    button(text(label).font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.text_strong)))
        .width(Length::Fill)
        .padding([6.0, 10.0])
        .on_press(message)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            // `surface_hover` (a translucent overlay), not `surface_raised`
            // (an opaque swap) — this row sits inside a popup whose own
            // background already *is* `surface_raised`, so an opaque swap
            // to the same color would be an invisible no-op on hover. The
            // accent border rides along so the highlight reads clearly even
            // at the overlay's necessarily-subtle opacity.
            button::Style {
                background: if hovered { Some(color(p.surface_hover).into()) } else { None },
                border: Border { color: if hovered { color(p.border_accent) } else { Color::TRANSPARENT }, width: 1.0, radius: 3.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

/// The header's "View" popup, as a `shell.rs` top-level layer — see
/// `actions_menu`'s own doc comment for why this isn't stacked locally
/// inside the header anymore. Lists whichever of Docked/Tab/Collapsed are
/// reachable from wherever the panel currently is (i.e. everything except
/// that).
pub fn view_menu(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if !state.chat_view_menu_open {
        return None;
    }
    let presentation = current_presentation(state)?;
    let (destinations, top): (Vec<(&'static str, Message)>, f32) = match presentation {
        ChatPresentation::Docked => {
            (vec![("Tab", Message::ChatOpenTab), ("Collapse", Message::ChatCollapse)], state.density.title_bar_h())
        }
        ChatPresentation::Tab => (
            vec![("Docked", Message::ChatDockFromTab), ("Collapse", Message::ChatCollapse)],
            state.density.title_bar_h() + state.density.tab_bar_h(),
        ),
    };
    // `+ 34.0 + 4.0`: the header's own height, plus a small gap — same
    // header height `panel_header` itself is fixed to.
    let top = top + 34.0 + 4.0;

    let menu = container(column(destinations.into_iter().map(|(label, msg)| view_menu_row(label, msg, p))).spacing(2.0).padding(6.0))
        .width(Length::Fixed(150.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.border_hairline), width: 1.5, radius: 3.0.into() },
            ..container::Style::default()
        });

    let positioned = container(menu)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .padding(iced::Padding { top, right: 8.0, bottom: 0.0, left: 0.0 });

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill)).width(Length::Fill).height(Length::Fill),
    )
    .on_press(Message::ChatToggleViewMenu);

    Some(iced::widget::stack![backdrop, positioned].into())
}

/// The docked side panel — pushed into `shell.rs`'s `body` row alongside
/// its own resize handle, same composition as `sidebar::view` +
/// `sidebar::resize_handle`.
pub fn docked_view(state: &State, p: Palette) -> Element<'_, Message> {
    let header = panel_header(ai_switcher(state, p), p, header_buttons(state, p));

    let body = column![header, thread_view(state, p)].width(Length::Fill).height(Length::Fill);

    container(body)
        .width(Length::Fixed(state.chat_panel_width))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            ..container::Style::default()
        })
        .into()
}

/// A thin drag handle on the chat panel's *left* edge (it's docked to the
/// right, unlike the sidebar) — see `Message::ChatResizeStarted` and
/// `state::chat_resize_events` for the drag itself.
pub fn resize_handle(p: Palette) -> Element<'static, Message> {
    mouse_area(
        container(Space::new().width(Length::Fixed(4.0)).height(Length::Fill))
            .width(Length::Fixed(4.0))
            .height(Length::Fill)
            .style(move |_theme| container::Style { background: Some(color(p.border_hairline).into()), ..container::Style::default() }),
    )
    .interaction(iced::mouse::Interaction::ResizingHorizontally)
    .on_press(Message::ChatResizeStarted)
    .into()
}

/// The narrow icon rail shown in place of the docked panel when
/// `chat_mode == Collapsed` — mirrors `sidebar::collapsed_rail`'s shape.
pub fn collapsed_rail(p: Palette) -> Element<'static, Message> {
    let expand = header_button("\u{bb}", Message::ChatDock, p);
    let body = column![Space::new().height(Length::Fill), widgets::dot(color(p.accent_solid), 6.0), Space::new().height(12.0), expand]
        .spacing(4.0)
        .align_x(Alignment::Center)
        .padding([12.0, 0.0]);

    container(body)
        .width(Length::Fixed(32.0))
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}

/// Full-tab presentation — used by `shell.rs`'s `content_area` when
/// `state.active_tab == Some(TabKey::Chat)`. The default presentation for a
/// freshly opened session — see `toggle_chat`'s own doc comment.
pub fn tab_view(state: &State, p: Palette) -> Element<'_, Message> {
    let header = panel_header(ai_switcher(state, p), p, header_buttons(state, p));
    column![header, thread_view(state, p)].width(Length::Fill).height(Length::Fill).into()
}
