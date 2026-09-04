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
use devscribe_core::outline;
use devscribe_core::theme::{Palette, Rgba};
use iced::font::Weight;
use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_editor, text_input, Space};
use iced::{Alignment, Border, Color, Element, Length};

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::color::color;
use crate::fonts;
use crate::recent_projects;
use crate::state::{
    active_file_path, ask_about_file_prompt, chat_is_active, find_editor, fix_bug_prompt, last_operator_text, ChatMessage, ChatMode,
    ChatProvider, ChatStatus, Message, PermissionState, State, TabKey, ToolActivity, CONTINUE_PROMPT, REGENERATE_PROMPT,
    SUMMARIZE_PROJECT_PROMPT,
};
use crate::ui::status_bar;
use crate::widgets;

/// `panel_header`'s own fixed height — pulled out to a constant since
/// `view_menu` also needs it (to position its popup just below the header)
/// and the two used to drift out of sync as a bare `34.0` literal in each.
/// Shorter than it used to be (was `34.0`) as part of trimming the header's
/// visual weight down to "supporting UI, not a competing object" (chat-panel
/// UX pass, item 5) — close to the app's own Compact title/tab-bar height
/// (`Density::Compact`'s `30.0`) rather than a bespoke, taller number.
const HEADER_H: f32 = 30.0;

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
            // No border at rest — only on hover. A header button that's
            // outlined even when idle reads as its own boxed control
            // competing for attention; blending into the (now much
            // lighter-weight, see `panel_header`) header until actually
            // interacted with is the "supporting UI, not a competing
            // object" half of the chat-panel UX pass (item 5).
            background: if hovered { Some(color(p.surface_hover).into()) } else { None },
            text_color: if hovered { color(p.accent_solid) } else { color(p.text_muted) },
            border: Border { color: if hovered { color(p.border_accent) } else { Color::TRANSPARENT }, width: 1.0, radius: 3.0.into() },
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

/// The `@path` tokens a turn's prompt referenced — `claude`'s own file-
/// mention syntax (see `insert_chat_mention`'s doc comment), so parsing them
/// back out of the sent text is enough to know what a turn was "about"
/// without DevScribe tracking that separately. A bare `@` (nothing after
/// it) or `@` mid-word (an email-shaped string, `foo@bar`) isn't a mention,
/// so tokens are required to start the word.
fn context_mentions(text_content: &str) -> Vec<&str> {
    text_content
        .split_whitespace()
        .filter(|tok| tok.starts_with('@') && tok.len() > 1)
        .map(|tok| tok.trim_start_matches('@').trim_end_matches(|c: char| c.is_ascii_punctuation() && c != '/' && c != '.'))
        .filter(|tok| !tok.is_empty())
        .collect()
}

/// A small "referenced" chip row under a turn's prompt, one per `@`-mention
/// found in it (chat-panel UX pass, item 7's "context indicator") — skipped
/// entirely for a turn that mentioned nothing, rather than an empty row.
fn context_chips(text_content: &str, p: Palette) -> Option<Element<'static, Message>> {
    let mentions = context_mentions(text_content);
    if mentions.is_empty() {
        return None;
    }
    let chips: Vec<Element<'static, Message>> = mentions
        .into_iter()
        .map(|m| {
            container(
                text(m.to_string())
                    .font(fonts::mono(Weight::Medium))
                    .size(crate::text_scale::px(11.0))
                    .color(color(p.text_muted)),
            )
            .padding([2.0, 6.0])
            .style(move |_theme| container::Style {
                background: Some(color(p.surface_raised).into()),
                border: Border { color: color(p.border_hairline), width: 1.0, radius: 3.0.into() },
                ..container::Style::default()
            })
            .into()
        })
        .collect();
    Some(row(chips).spacing(6.0).into())
}

fn operator_row(text_content: &str, p: Palette) -> Element<'static, Message> {
    let mut content = column![
        row![
            widgets::micro("OPERATOR", color(p.text_muted)),
            Space::new().width(Length::Fill),
            link_button("Edit", Message::ChatEditMessage(text_content.to_string()), p),
            link_button("Copy", Message::ChatCopyText(text_content.to_string()), p),
        ]
        .spacing(10.0)
        .align_y(Alignment::Center),
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
    .spacing(6.0);
    if let Some(chips) = context_chips(text_content, p) {
        content = content.push(chips);
    }
    content.into()
}

/// `caret_visible` (state.rs's existing ~530ms blink tick) only ever
/// applies while `streaming` is true — a finalized bubble never shows the
/// live-typing caret, matching VS Code's own streaming indicator.
fn assistant_row(text_content: &str, streaming: bool, caret_visible: bool, provider: ChatProvider, p: Palette) -> Element<'static, Message> {
    let shown = if streaming && caret_visible { format!("{text_content}\u{258c}") } else { text_content.to_string() };
    let mut header = row![widgets::dot(color(p.accent_solid), 5.0), widgets::micro(provider.label(), color(p.text_muted))]
        .spacing(7.0)
        .align_y(Alignment::Center);
    // Only once the reply has actually settled — copying a still-streaming
    // bubble would grab whatever partial text happened to have landed at
    // the moment of the click, same reasoning as `assistant_row`'s own
    // caret never showing on a finalized bubble, just the other way round.
    if !streaming {
        header = header.push(Space::new().width(Length::Fill));
        header = header.push(link_button("Copy", Message::ChatCopyText(text_content.to_string()), p));
    }
    column![
        header,
        text(shown).size(crate::text_scale::px(15.0)).color(color(p.text_body)),
    ]
    .spacing(6.0)
    .into()
}

/// The gap between "sent" and "the first token/tool call landed" — see
/// `thread_view`'s own call site for exactly when this shows. Reuses the
/// same blink tick `assistant_row`'s streaming caret and `tool_status_glyph`
/// already ride on, so it reads as one consistent "something is
/// happening" idiom across the whole panel rather than a bespoke spinner.
fn typing_indicator_row(provider: ChatProvider, caret_visible: bool, p: Palette) -> Element<'static, Message> {
    column![
        row![widgets::dot(color(p.accent_solid), 5.0), widgets::micro(provider.label(), color(p.text_muted))]
            .spacing(7.0)
            .align_y(Alignment::Center),
        text(if caret_visible { "\u{2022}\u{2022}\u{2022}" } else { "\u{2022}\u{2022}\u{2022}\u{2022}" })
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_muted)),
    ]
    .spacing(6.0)
    .into()
}

/// The first `limit` lines of `text`, with a trailing count of what's
/// hidden — `permission_card`'s truncated-diff default, so a large
/// Edit/Write doesn't paste hundreds of lines of "payload-like content"
/// straight into the transcript (chat-panel UX pass, item 2).
fn truncate_lines(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= limit {
        text.to_string()
    } else {
        format!("{}\n\u{2026} {} more lines", lines[..limit].join("\n"), lines.len() - limit)
    }
}

/// A pending tool call gated on a human decision — the diff preview comes
/// straight from `old_string`/`new_string` on `Edit`'s own `tool.input`
/// (all that's known *before* the edit happens); nothing richer
/// (`structuredPatch` etc.) exists yet at this point, since that's only in
/// the eventual `ToolResult`.
///
/// `expanded` (from `ChatThread::expanded_tools`) shows the diff in full
/// instead of truncated to `DIFF_PREVIEW_LINES` per side — the toggle only
/// appears at all once either side is actually longer than that.
fn permission_card(tool: &ToolActivity, expanded: bool, p: Palette) -> Element<'static, Message> {
    const DIFF_PREVIEW_LINES: usize = 6;

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
        let is_long = old.lines().count() > DIFF_PREVIEW_LINES || new.lines().count() > DIFF_PREVIEW_LINES;
        let (old_shown, new_shown) = if is_long && !expanded {
            (truncate_lines(old, DIFF_PREVIEW_LINES), truncate_lines(new, DIFF_PREVIEW_LINES))
        } else {
            (old.to_string(), new.to_string())
        };

        content = content.push(
            container(
                column![
                    text(format!("- {old_shown}"))
                        .font(fonts::mono(Weight::Normal))
                        .size(crate::text_scale::px(12.5))
                        .color(color(p.status_danger)),
                    text(format!("+ {new_shown}"))
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

        if is_long {
            let label = if expanded { "Show less" } else { "Show full diff" };
            content = content.push(link_button(label, Message::ChatToggleToolExpanded(tool.id.clone()), p));
        }
    }

    content = content.push(
        row![
            primary_button("Approve", Message::ChatApprovePermission(tool.id.clone()), p),
            secondary_button("Deny", Message::ChatDenyPermission(tool.id.clone()), p),
        ]
        .spacing(8.0),
    );

    let card = container(content)
        .padding(10.0)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_raised).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 4.0.into() },
            ..container::Style::default()
        });

    // A solid left accent bar rather than the uniform warning-colored border
    // this used to have — `Border` has no per-side width in this iced
    // version, so a bordered box reads the same on every edge, the same
    // shape as any other bubble in the thread. A left accent bar is the
    // familiar "this needs your attention" idiom (VS Code's own
    // notifications, GitHub's alert callouts) — it's what actually makes a
    // permission card read as a workflow prompt to act on rather than a
    // message to read (chat-panel UX pass, item 4).
    let accent_bar = container(Space::new().width(Length::Fixed(3.0)).height(Length::Fill))
        .width(Length::Fixed(3.0))
        .style(move |_theme| container::Style { background: Some(color(p.status_warning).into()), ..container::Style::default() });

    row![accent_bar, card].width(Length::Fill).into()
}

/// The four states `tool_activity_row` can strengthen its semantics for —
/// "running/editing/approved/denied" (chat-panel UX pass, item 4) — plus
/// `Failed`, split out from what used to be the same red cross as `Denied`:
/// a tool call the human refused and a tool call that ran and genuinely
/// errored (a `Read` on a path that doesn't exist, a nonzero `Bash` exit)
/// look identical without this distinction, and they mean very different
/// things about what to do next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolStatus {
    Running,
    Done,
    Denied,
    Failed,
}

fn tool_status(tool: &ToolActivity) -> ToolStatus {
    match (&tool.permission, &tool.result) {
        (Some(PermissionState::Denied), _) => ToolStatus::Denied,
        (_, Some(r)) if r.is_error => ToolStatus::Failed,
        (_, Some(_)) => ToolStatus::Done,
        (_, None) => ToolStatus::Running,
    }
}

/// `caret_visible` (state.rs's existing ~530ms blink tick, already driving
/// the editor's own caret) is reused for `Running`'s pulse rather than
/// standing up a dedicated animation — this app has no CSS keyframes to
/// lean on, and this is the same "is something still happening" signal
/// either way.
fn tool_status_glyph(status: ToolStatus, caret_visible: bool, p: Palette) -> (&'static str, Rgba) {
    match status {
        ToolStatus::Denied => ("\u{2715}", p.status_danger),
        ToolStatus::Failed => ("!", p.status_danger),
        ToolStatus::Done => ("\u{2713}", p.status_success),
        ToolStatus::Running => (if caret_visible { "\u{25cf}" } else { "\u{25cb}" }, p.accent_solid),
    }
}

/// A compact `-old/+new` line-count readout for a landed Edit — the
/// "compact result summary" half of item 4, using data this row already
/// has on hand (`tool.input`'s own `old_string`/`new_string`, the same
/// fields `permission_card`'s diff preview reads) rather than needing the
/// wire protocol to report anything richer. Deliberately a size-of-change
/// hint, not a real diff — `ToolActivityResult` holds no result payload at
/// all by design (see its own doc comment on why), and a genuine diff is
/// one click away via `Open diff` regardless.
fn edit_diffstat(tool: &ToolActivity) -> Option<(usize, usize)> {
    let old = tool.input.get("old_string").and_then(|v| v.as_str())?;
    let new = tool.input.get("new_string").and_then(|v| v.as_str())?;
    Some((old.lines().count(), new.lines().count()))
}

/// A tool call that either never needed a decision (Read/Grep/...), or
/// already has one (approved/denied) — a single compact line, its status
/// (`ToolStatus`) read off both an icon and, for the two states an icon
/// alone reads ambiguously (`Running` vs. sitting idle; `Denied` vs. a
/// generic error), a short text label too.
fn tool_activity_row(tool: &ToolActivity, caret_visible: bool, p: Palette) -> Element<'static, Message> {
    let status = tool_status(tool);
    let (glyph, glyph_color) = tool_status_glyph(status, caret_visible, p);

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

    match status {
        ToolStatus::Running => content = content.push(widgets::micro("RUNNING\u{2026}", color(p.accent_solid))),
        ToolStatus::Denied => content = content.push(widgets::micro("DENIED", color(p.status_danger))),
        ToolStatus::Failed => content = content.push(widgets::micro("FAILED", color(p.status_danger))),
        ToolStatus::Done => {}
    }

    // A settled, successful, actually-approved Edit/Write — the file has
    // really landed on disk (the permission gate runs *before* `claude`
    // touches it, so a completed result here means it went through) —
    // gets a diffstat and a jump to the existing diff view instead of
    // DevScribe trying to re-render its own copy of the change.
    let landed_edit = matches!(tool.name.as_str(), "Edit" | "Write") && status == ToolStatus::Done;
    if landed_edit
        && let Some((old_lines, new_lines)) = edit_diffstat(tool)
    {
        content = content.push(
            row![
                text(format!("-{old_lines}")).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(color(p.status_danger)),
                text(format!("+{new_lines}")).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(12.0)).color(color(p.status_success)),
            ]
            .spacing(4.0),
        );
    }
    if landed_edit
        && let Some(path) = tool.input.get("file_path").and_then(|v| v.as_str())
    {
        content = content.push(Space::new().width(Length::Fill));
        content = content.push(link_button("Open diff", Message::OpenDiffFor(PathBuf::from(path)), p));
    }

    content.into()
}

/// One row of `message_row`'s own match — a pending permission still needs
/// its own full-weight card here even though `group_thread` otherwise
/// pulls every settled `Tool` message out into a `ThreadRow::ToolGroup`
/// instead (see its own doc comment): a decision that's still open is
/// primary content, not metadata to recede below it.
fn message_row(msg: &ChatMessage, caret_visible: bool, provider: ChatProvider, expanded_tools: &HashSet<String>, p: Palette) -> Element<'static, Message> {
    match msg {
        ChatMessage::Operator(text_content) => operator_row(text_content, p),
        ChatMessage::Assistant { text, streaming } => assistant_row(text, *streaming, caret_visible, provider, p),
        ChatMessage::Tool(tool) if tool.permission == Some(PermissionState::Pending) => {
            permission_card(tool, expanded_tools.contains(&tool.id), p)
        }
        ChatMessage::Tool(tool) => tool_activity_row(tool, caret_visible, p),
    }
}

/// One printable unit of the transcript, after `group_thread` has clustered
/// consecutive settled tool calls together — see its own doc comment.
enum ThreadRow<'a> {
    Message(&'a ChatMessage),
    ToolGroup(Vec<&'a ToolActivity>),
}

/// Clusters consecutive *settled* tool calls (approved-and-done, denied, or
/// never gated at all) into a single `ToolGroup`, leaving everything else —
/// operator/assistant messages, and any still-`Pending` permission card —
/// as its own `Message` row. A `claude` turn that reads five files before
/// answering used to paint five separate 16px-spaced rows at the same
/// visual weight as the answer itself; grouping them lets `tool_group_view`
/// render that whole run as one small, visually recessed block instead,
/// which is the "let AI/tool metadata sit below the primary content" half
/// of the chat-panel UX pass (item 2).
fn group_thread(messages: &[ChatMessage]) -> Vec<ThreadRow<'_>> {
    let mut rows: Vec<ThreadRow> = Vec::new();
    for msg in messages {
        match msg {
            ChatMessage::Tool(tool) if tool.permission != Some(PermissionState::Pending) => {
                if let Some(ThreadRow::ToolGroup(group)) = rows.last_mut() {
                    group.push(tool);
                } else {
                    rows.push(ThreadRow::ToolGroup(vec![tool]));
                }
            }
            other => rows.push(ThreadRow::Message(other)),
        }
    }
    rows
}

/// A run of settled tool calls (see `ThreadRow::ToolGroup`) as one visually
/// recessed block with tight internal spacing, rather than each call
/// competing for the thread's normal message rhythm.
fn tool_group_view(tools: &[&ToolActivity], caret_visible: bool, p: Palette) -> Element<'static, Message> {
    let rows: Vec<Element<'static, Message>> = tools.iter().map(|t| tool_activity_row(t, caret_visible, p)).collect();
    container(column(rows).spacing(5.0))
        .padding([6.0, 10.0])
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.surface_inset).into()),
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 4.0.into() },
            ..container::Style::default()
        })
        .into()
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

/// The Actions popup's rows, which depend on the active provider: most of
/// what the popup offers is `claude`-CLI-specific machinery rather than
/// generic chat. "Attach file…"/"Mention file…" (and the CONTEXT section's
/// own "Current file"/"Active symbol" quick actions — see
/// `quick_context_rows`) insert an `@path`, which is `claude`'s own
/// file-reference syntax (see `insert_chat_mention`) and reaches Copilot as
/// nothing but literal text — Copilot takes file context through
/// `conversation/*`'s own `references` parameter instead, which
/// `copilot_agent` doesn't send yet. "Switch model…", "Thinking" and
/// "Account & usage" are all just `/model`, `/effort` and `/usage` sent as
/// prompts, which only `claude` answers itself. "Shell Access" maps to
/// `SessionOptions::allow_bash`, which `copilot_agent` ignores outright.
///
/// Not *everything* is Claude-only, though — "Selected text" (a plain
/// fenced code block) and "Project root" (plain descriptive text) carry no
/// special syntax at all, so they work identically under Copilot, and
/// `quick_context_rows`'s own `include_mention_based` keeps those two
/// available there rather than hiding them behind the same blanket check as
/// the genuinely Claude-only rows. Bundling every CONTEXT row behind one
/// `is_claude_cli()` branch used to hide working functionality from Copilot
/// — exactly the kind of provider-specific-vs-generic mixing item 9 of the
/// chat-panel UX pass asks to keep separated.
///
/// So under Copilot the popup reduces to those two quick actions plus
/// "Clear conversation" — which does work there (a new session id re-keys
/// `chat_worker`'s subscription, so the worker respawns onto a fresh
/// conversation). A real Copilot model picker belongs here eventually —
/// `copilot_agent` already fetches the account's model list to choose a
/// default — but wiring the choice back through the subscription key is its
/// own change, and a non-functional row in the meantime is exactly what
/// this avoids.
fn actions_menu_items(state: &State, p: Palette) -> Vec<Element<'static, Message>> {
    if !state.chat_provider.is_claude_cli() {
        let mut items = vec![actions_section_label("CONTEXT", p)];
        items.extend(quick_context_rows(state, false, p));
        items.push(widgets::hline(color(p.border_hairline)));
        items.push(actions_section_label("SESSION", p));
        items.push(action_row("Clear conversation", None, Message::ChatNewSession, p));
        return items;
    }

    let model = state.chat.model.clone().unwrap_or_else(|| "\u{2014}".to_string());

    let mut context_rows: Vec<Element<'static, Message>> = vec![actions_section_label("CONTEXT", p)];
    context_rows.extend(quick_context_rows(state, true, p));
    context_rows.push(action_row("Attach file\u{2026}", Some("\u{2318}U".to_string()), Message::ChatAttachFileDialog, p));
    context_rows.push(action_row("Mention file from this project\u{2026}", Some("@".to_string()), Message::ChatMentionFileDialog, p));
    context_rows.push(action_row("Clear conversation", None, Message::ChatNewSession, p));

    let mut items = context_rows;
    items.extend(vec![
        widgets::hline(color(p.border_hairline)),
        actions_section_label("MODEL", p),
        action_row("Switch model\u{2026}", Some(model), Message::ChatShowModel, p),
        action_toggle_row("Thinking", state.chat_thinking_enabled, Message::ChatToggleThinking, p),
        widgets::hline(color(p.border_hairline)),
        actions_section_label("PERMISSIONS", p),
        action_toggle_row("Shell Access", state.chat_shell_access_enabled, Message::ChatToggleShellAccess, p),
        widgets::hline(color(p.border_hairline)),
        action_row("Account & usage\u{2026}", None, Message::ChatShowUsage, p),
    ]);
    items
}

/// The CONTEXT section's quick-action rows — "wire references into prompt
/// composition without requiring manual text entry" (chat-panel UX pass,
/// item 3). Each folds a piece of workspace context (the open file, the
/// current selection, the enclosing symbol) straight into the draft; a row
/// only appears once there's actually something for it to grab (no
/// "Selected text" row with nothing selected, no "Current file"/"Active
/// symbol" with no file open) — "Project root" is the one always available,
/// since a project is a precondition for the chat panel existing at all.
///
/// `include_mention_based` gates "Current file" and "Active symbol" — both
/// insert an `@path` token, `claude`'s own file-reference syntax, which
/// reaches Copilot as nothing but literal text (see `actions_menu_items`'s
/// own doc comment). "Selected text" and "Project root" insert plain text
/// with no special syntax, so they're offered regardless of provider.
fn quick_context_rows(state: &State, include_mention_based: bool, p: Palette) -> Vec<Element<'static, Message>> {
    let mut rows = Vec::new();

    let active_path = active_file_path(state);
    let active_editor = active_path.as_ref().and_then(|path| find_editor(state, path));

    if include_mention_based
        && let Some(path) = &active_path
    {
        let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned());
        rows.push(action_row("Current file", Some(label), Message::ChatMentionCurrentFile, p));
    }

    if let Some(lines) = active_editor.and_then(|e| e.selected_text()).map(|t| t.lines().count().max(1)) {
        rows.push(action_row("Selected text", Some(format!("{lines} line{}", if lines == 1 { "" } else { "s" })), Message::ChatMentionSelection, p));
    }

    if include_mention_based
        && let Some(label) = active_editor.and_then(|e| {
            let crumbs = e.breadcrumbs();
            let index = outline::emphasized_index(&crumbs)?;
            Some(crumbs[index].label.clone())
        })
    {
        rows.push(action_row("Active symbol", Some(label), Message::ChatMentionActiveSymbol, p));
    }

    let root_label = state.root.file_name().map(|n| n.to_string_lossy().into_owned());
    rows.push(action_row("Project root", root_label, Message::ChatMentionProjectRoot, p));

    rows
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

/// The composer's subtle status hint — "Sending…" while a turn is in
/// flight (`ChatThread::sending`), else "Draft saved" whenever there's
/// unsent text sitting in the input. The draft already survives switching
/// between docked/tab/collapsed presentation (see `ChatThread::input`'s own
/// doc comment on why it's independent of presentation) — this just makes
/// that fact visible instead of something the user has to take on faith.
/// Renders `Space` rather than nothing when neither applies, so this slot's
/// height stays constant and the row around it doesn't jump as state flips.
fn composer_status(state: &State, p: Palette) -> Element<'static, Message> {
    if state.chat.sending {
        row![widgets::dot(color(p.accent_solid), 5.0), widgets::micro("SENDING\u{2026}", color(p.accent_solid))]
            .spacing(6.0)
            .align_y(Alignment::Center)
            .into()
    } else if !state.chat.input.text().trim().is_empty() {
        widgets::micro("DRAFT SAVED", color(p.text_muted)).into()
    } else {
        Space::new().into()
    }
}

/// Static hint labels for the input's built-in trigger characters — `@`
/// mentions a project file (see `ChatMentionFileDialog`); `#`/`/` are
/// `claude`'s own conventions (a quick memory note, a skill/slash command)
/// that DevScribe doesn't intercept in any way — the CLI sees them in the
/// prompt text exactly as typed. `composer_status` rides along on the same
/// row, right-aligned, rather than as a separate line, so surfacing it
/// doesn't cost the composer any extra vertical space.
fn composer_meta_row(state: &State, p: Palette) -> Element<'static, Message> {
    row![
        text("@ file").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)),
        text("# symbol").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)),
        text("/ command").font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.text_muted)),
        Space::new().width(Length::Fill),
        composer_status(state, p),
    ]
    .spacing(12.0)
    .align_y(Alignment::Center)
    .into()
}

/// The composer's explicit send affordance. Enter already submits (see
/// `input_bar`'s own `key_binding`), but a visible, unmissable control
/// matters for discoverability and for driving the panel by mouse alone —
/// "keep a clear send affordance visible without crowding the layout" (the
/// chat-panel UX pass, item 1). `on_press_maybe(None)` while disabled
/// doesn't just dim the button, it stops it capturing the click at all, so
/// there's no way to fire a submit with nothing to send.
fn send_button(enabled: bool, p: Palette) -> Element<'static, Message> {
    button(widgets::center_fill(text("\u{2191}").font(fonts::mono(Weight::Bold)).size(crate::text_scale::px(16.0))))
        .width(Length::Fixed(32.0))
        .height(Length::Fixed(32.0))
        .padding(0.0)
        .on_press_maybe(enabled.then_some(Message::ChatSubmit))
        .style(move |_theme, status| {
            let hovered = enabled && status == button::Status::Hovered;
            button::Style {
                background: Some(
                    if !enabled {
                        color(p.surface_inset)
                    } else if hovered {
                        color(p.accent_solid_hover)
                    } else {
                        color(p.accent_solid)
                    }
                    .into(),
                ),
                text_color: if enabled { color(p.accent_on) } else { color(p.text_muted) },
                border: Border {
                    color: if enabled { Color::TRANSPARENT } else { color(p.border_strong) },
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..button::Style::default()
            }
        })
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
        .id(crate::state::chat_input_id())
        .placeholder(placeholder)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(14.0))
        .padding(10.0)
        // A floor as well as the existing ceiling: without it, an empty or
        // one-word draft measures shorter than the 32px actions/send
        // buttons beside it, so the whole input row visibly resizes as
        // typing crosses that threshold rather than growing smoothly from a
        // stable baseline — the "sticky," predictable composer item 1 asks
        // for.
        .min_height(40.0)
        .max_height(120.0)
        .key_binding(|press| {
            if press.key == Key::Named(Named::Enter) && !press.modifiers.shift() {
                Some(text_editor::Binding::Custom(Message::ChatSubmit))
            } else {
                text_editor::Binding::from_key_press(press)
            }
        })
        .style(move |_theme, status| text_editor::Style {
            background: color(p.surface_inset).into(),
            // Visible keyboard-focus indicator (accessibility pass, item 12).
            border: if matches!(status, text_editor::Status::Focused { .. }) {
                Border { color: color(p.border_focus), width: 1.5, radius: 3.0.into() }
            } else {
                Border { color: color(p.border_strong), width: 1.5, radius: 3.0.into() }
            },
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

    let send_enabled = enabled && !state.chat.input.text().trim().is_empty();
    let input_row = row![actions_menu_button(state.chat_actions_open, p), input, send_button(send_enabled, p)]
        .spacing(8.0)
        .align_y(Alignment::End);

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
    items.push(composer_meta_row(state, p));
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

/// A clickable starter-prompt suggestion for the empty thread — sends
/// `prompt` outright on press (see `Message::ChatSendPrompt`) rather than
/// dropping it into the draft for further editing, since the whole point of
/// an empty-state suggestion is a one-click start to the conversation.
/// Disabled (not just dimmed) while there's no live session to send to yet
/// (`enabled`), same `on_press_maybe` convention as `send_button`.
fn starter_prompt_button(label: &'static str, prompt: String, enabled: bool, p: Palette) -> Element<'static, Message> {
    button(
        row![
            text("\u{2192}").font(fonts::mono(Weight::Bold)).size(crate::text_scale::px(13.0)).color(color(p.accent_solid)),
            text(label).font(fonts::sans(Weight::Medium)).size(crate::text_scale::px(13.5)).color(color(p.text_strong)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    )
    .padding([8.0, 14.0])
    .on_press_maybe(enabled.then_some(Message::ChatSendPrompt(prompt)))
    .style(move |_theme, status| {
        let hovered = enabled && status == button::Status::Hovered;
        button::Style {
            background: if hovered { Some(color(p.surface_hover).into()) } else { None },
            border: Border { color: if hovered { color(p.border_accent) } else { color(p.border_hairline) }, width: 1.0, radius: 4.0.into() },
            ..button::Style::default()
        }
    })
    .into()
}

/// The empty thread's contextual starter prompts (chat-panel UX pass, item
/// 7) — "Ask about this file" only appears with a file actually open;
/// "Summarize this project" is always offered; "Fix this bug" adapts its
/// own prompt text to whatever's most specific (selection, then file, then
/// just the project) via `fix_bug_prompt`. All three are disabled together
/// while there's no live session yet (`Starting`/`Unavailable`) — nothing
/// for a click to send to.
fn empty_thread_prompts(state: &State, p: Palette) -> Element<'static, Message> {
    let enabled = state.chat.sender.is_some();
    let mut prompts: Vec<Element<'static, Message>> = Vec::new();
    if let Some(prompt) = ask_about_file_prompt(state) {
        prompts.push(starter_prompt_button("Ask about this file", prompt, enabled, p));
    }
    prompts.push(starter_prompt_button("Summarize this project", SUMMARIZE_PROJECT_PROMPT.to_string(), enabled, p));
    prompts.push(starter_prompt_button("Fix this bug", fix_bug_prompt(state), enabled, p));

    column![
        text(format!("Ask {} about this project to get started", state.chat_provider.label()))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(14.0))
            .color(color(p.text_muted)),
        column(prompts).spacing(6.0).align_x(Alignment::Center),
    ]
    .spacing(18.0)
    .align_x(Alignment::Center)
    .into()
}

/// The empty thread's failure state — `ChatStatus::Unavailable`'s reason
/// used to be the *only* content of the plain muted `widgets::placeholder`
/// this replaces, easy to miss especially right after switching providers
/// when the previous one's messages just disappeared. A heading plus a
/// "Try again" action (bumping `State::chat_restart_token`, reserved for
/// exactly this since it was added — see its own doc comment) makes this
/// read as an actionable failure rather than passive empty-state text
/// (chat-panel UX pass, item 7).
fn unavailable_state(provider: ChatProvider, reason: String, p: Palette) -> Element<'static, Message> {
    column![
        row![
            widgets::dot(color(p.status_danger), 8.0),
            text(format!("{} isn't available", provider.label()))
                .font(fonts::sans(Weight::Medium))
                .size(crate::text_scale::px(15.0))
                .color(color(p.text_strong)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
        text(reason).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(13.0)).color(color(p.text_muted)),
        primary_button("Try again", Message::ChatRetryConnection, p),
    ]
    .spacing(14.0)
    .align_x(Alignment::Center)
    .into()
}

/// The message list's empty state — either the contextual starter prompts
/// (`empty_thread_prompts`) or, while the provider itself couldn't start,
/// the failure state (`unavailable_state`). Both center in the same
/// `bg_canvas`-backed box `widgets::placeholder` used to fill alone.
fn empty_state(state: &State, p: Palette) -> Element<'static, Message> {
    let content = match &state.chat.status {
        ChatStatus::Unavailable(reason) => unavailable_state(state.chat_provider, reason.clone(), p),
        _ => empty_thread_prompts(state, p),
    };
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(24.0)
        .center(Length::Fill)
        .style(move |_theme| container::Style { background: Some(color(p.bg_canvas).into()), ..container::Style::default() })
        .into()
}

/// The "continuation" affordances at the end of a non-empty thread — retry
/// the last thing the human said, or (only once the thread's very last
/// message is a settled assistant answer) regenerate or continue it. Reduces
/// the panel to a single-shot input box otherwise: without this, redoing a
/// turn means re-typing or copy-pasting the same prompt by hand (chat-panel
/// UX pass, item 8). `None` with no live session, mid-send, or with no
/// operator message yet to retry.
fn continuation_row(state: &State, p: Palette) -> Option<Element<'static, Message>> {
    if state.chat.sender.is_none() || state.chat.sending {
        return None;
    }
    let last_request = last_operator_text(&state.chat.messages)?;

    let mut actions = row![link_button("\u{21bb} Retry", Message::ChatSendPrompt(last_request), p)].spacing(16.0);
    if matches!(state.chat.messages.last(), Some(ChatMessage::Assistant { streaming: false, .. })) {
        actions = actions.push(link_button("Regenerate", Message::ChatSendPrompt(REGENERATE_PROMPT.to_string()), p));
        actions = actions.push(link_button("Continue", Message::ChatSendPrompt(CONTINUE_PROMPT.to_string()), p));
    }
    Some(actions.into())
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

    let filter = state.chat_session_filter.trim().to_ascii_lowercase();
    let filtered: Vec<&SessionSummary> = state
        .chat_sessions
        .iter()
        .filter(|s| filter.is_empty() || s.title.to_ascii_lowercase().contains(&filter))
        .collect();

    let search = text_input("Search sessions\u{2026}", &state.chat_session_filter)
        .font(fonts::sans(Weight::Medium))
        .size(crate::text_scale::px(13.0))
        .padding([6.0, 10.0])
        .on_input(Message::ChatSessionFilterChanged)
        .style(move |_theme, status| text_input::Style {
            background: color(p.surface_inset).into(),
            // Visible keyboard-focus indicator (accessibility pass, item 12).
            border: if matches!(status, text_input::Status::Focused { .. }) {
                Border { color: color(p.border_focus), width: 1.5, radius: 4.0.into() }
            } else {
                Border { color: color(p.border_hairline), width: 1.0, radius: 4.0.into() }
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

    let list: Element<'static, Message> = if state.chat_sessions.is_empty() {
        widgets::placeholder("No past sessions for this project yet", p.bg_canvas, p)
    } else if filtered.is_empty() {
        widgets::placeholder("No sessions match your search", p.bg_canvas, p)
    } else {
        let rows: Vec<Element<'static, Message>> = filtered.into_iter().map(|s| session_row(s, p)).collect();
        scrollable(column(rows).spacing(2.0).padding([4.0, 8.0]).width(Length::Fill)).width(Length::Fill).height(Length::Fill).into()
    };

    column![
        header,
        widgets::hline(color(p.border_hairline)),
        new_session,
        widgets::hline(color(p.border_hairline)),
        container(search).padding([8.0, 10.0]).width(Length::Fill),
        widgets::hline(color(p.border_hairline)),
        list
    ]
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

    let thread_rows = group_thread(&state.chat.messages);
    let mut rows: Vec<Element<'static, Message>> = Vec::with_capacity(thread_rows.len() + 2);
    for (i, row) in thread_rows.into_iter().enumerate() {
        // A divider ahead of every turn but the first — an Operator message
        // is what starts a new turn (see `group_thread`'s own doc comment
        // on what a "turn" is here) — so the transcript reads as a sequence
        // of exchanges rather than one undifferentiated stream of bubbles
        // (chat-panel UX pass, item 7's "message threading").
        if i > 0 && matches!(row, ThreadRow::Message(ChatMessage::Operator(_))) {
            rows.push(widgets::hline(color(p.border_hairline)));
        }
        rows.push(match row {
            ThreadRow::Message(msg) => message_row(msg, state.caret_visible, state.chat_provider, &state.chat.expanded_tools, p),
            ThreadRow::ToolGroup(tools) => tool_group_view(&tools, state.caret_visible, p),
        });
    }
    if state.chat.history_truncated {
        rows.insert(0, load_earlier_row(p));
    }
    // `sending` clears the moment anything comes back (a delta, a tool
    // call, ...) — see `ChatThread::sending`'s own doc comment — so it's
    // only ever still `true` here while nothing has appeared for this turn
    // yet, the gap a streaming bubble's own caret can't cover because
    // there's no bubble at all until the first token lands.
    if state.chat.sending {
        rows.push(typing_indicator_row(state.chat_provider, state.caret_visible, p));
    }
    if !rows.is_empty()
        && let Some(row) = continuation_row(state, p)
    {
        rows.push(row);
    }

    let list: Element<'static, Message> = if rows.is_empty() { empty_state(state, p) } else {
        scrollable(column(rows).spacing(12.0).padding(16.0).width(Length::Fill))
            .id(crate::state::chat_scroll_id())
            .width(Length::Fill)
            .height(Length::Fill)
            // Drives `State::chat_pinned_to_bottom` — see its own doc
            // comment on why new output only re-snaps to the bottom while
            // this stays `true`.
            .on_scroll(|viewport| Message::ChatScrolled(viewport.absolute_offset_reversed().y))
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

/// The panel's top chrome — provider switcher, session/history actions,
/// view menu. Deliberately styled to recede: a `bg_base` background (the
/// same background `docked_view`/`tab_view`'s own body sits on, rather than
/// the raised, visually distinct `surface_raised` this used to have) with
/// just a hairline bottom border, so the header blends into the panel it
/// belongs to instead of standing out as its own elevated bar competing
/// with the thread underneath it (chat-panel UX pass, item 5).
fn panel_header(switcher: Element<'static, Message>, p: Palette, buttons: Vec<Element<'static, Message>>) -> Element<'static, Message> {
    let mut content = row![switcher].spacing(8.0).align_y(Alignment::Center);
    content = content.push(Space::new().width(Length::Fill));
    for b in buttons {
        content = content.push(b);
    }

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(HEADER_H))
        .padding([0.0, 8.0])
        .align_y(Alignment::Center)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
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
            // `surface_hover`, not `surface_raised` — an opaque same-color
            // swap over `panel_header`'s now-`bg_base` background wouldn't
            // read as a hover at all. No border at rest, same reasoning as
            // `header_button`'s own (see its doc comment): boxed-looking
            // chrome even when idle is exactly the "competing object" weight
            // item 5 trims off of this header.
            background: if open { Some(color(p.accent_quiet).into()) } else if hovered { Some(color(p.surface_hover).into()) } else { None },
            text_color: if active { color(p.accent_solid) } else { color(p.text_muted) },
            border: Border { color: if active { color(p.border_accent) } else { Color::TRANSPARENT }, width: 1.0, radius: 3.0.into() },
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
    // `HEADER_H`: the header's own height, plus a small gap — same height
    // `panel_header` itself is fixed to.
    let top = top + HEADER_H + 4.0;

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
