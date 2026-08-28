//! The AI Chat Assist panel. One shared message-thread + input bar
//! (`thread_view`) is reused verbatim across the three modes with real
//! room to show it — docked, floating window, full tab — each wrapped in
//! a thin header of mode-switch buttons that differs only in which
//! actions make sense from that mode. The collapsed rail is the one
//! genuinely different shape: a narrow icon strip, mirroring
//! `sidebar.rs`'s own collapsed rail.
//!
//! A tool call is rendered from whatever `state.chat`'s `ToolActivity`
//! knows at the time — before a decision, that's just `tool.input` (e.g.
//! an `Edit`'s `old_string`/`new_string`), which is enough for the
//! Approve/Deny card's diff preview; there's no separate "apply" step
//! afterward the way the original mockup's diff card had one, since the
//! permission gate already runs *before* `claude` performs the edit (see
//! `devscribe_core::claude_agent`) — approving *is* applying.
use devscribe_core::claude_agent::SessionSummary;
use devscribe_core::theme::{Palette, Rgba};
use iced::font::Weight;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{Alignment, Border, Color, Element, Length};

use std::path::PathBuf;
use std::time::SystemTime;

use crate::color::color;
use crate::fonts;
use crate::recent_projects;
use crate::state::{ChatMessage, ChatStatus, Message, PermissionState, State, ToolActivity};
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
            background: if hovered { Some(color(p.surface_raised).into()) } else { None },
            text_color: if hovered { color(p.accent_solid) } else { color(p.text_muted) },
            border: Border { color: color(p.border_hairline), width: 1.0, radius: 3.0.into() },
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

fn assistant_row(text_content: &str, p: Palette) -> Element<'static, Message> {
    column![
        row![widgets::dot(color(p.accent_solid), 5.0), widgets::micro("CLAUDE CODE", color(p.text_muted))]
            .spacing(7.0)
            .align_y(Alignment::Center),
        text(text_content.to_string()).size(crate::text_scale::px(15.0)).color(color(p.text_body)),
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

fn message_row(msg: &ChatMessage, caret_visible: bool, p: Palette) -> Element<'static, Message> {
    match msg {
        ChatMessage::Operator(text_content) => operator_row(text_content, p),
        ChatMessage::Assistant(text_content) => assistant_row(text_content, p),
        ChatMessage::Tool(tool) if tool.permission == Some(PermissionState::Pending) => permission_card(tool, p),
        ChatMessage::Tool(tool) => tool_activity_row(tool, caret_visible, p),
    }
}

fn status_line(state: &State, p: Palette) -> Element<'static, Message> {
    let label = match &state.chat.status {
        ChatStatus::Starting => "STARTING CLAUDE CODE\u{2026}".to_string(),
        ChatStatus::Unavailable(reason) => format!("UNAVAILABLE \u{2014} {reason}"),
        ChatStatus::Ready => {
            let model = state.chat.model.as_deref().unwrap_or("claude");
            format!(
                "{model} \u{2014} ${:.2} \u{2014} {}+{} TOK",
                state.chat.cost_usd, state.chat.input_tokens, state.chat.output_tokens
            )
        }
    };
    let label_color = if matches!(state.chat.status, ChatStatus::Unavailable(_)) { p.status_danger } else { p.text_muted };
    text(label).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(label_color)).into()
}

fn input_bar(state: &State, p: Palette) -> Element<'static, Message> {
    let enabled = state.chat.sender.is_some();
    let placeholder = if enabled { "Ask about this project\u{2026}" } else { "Waiting for Claude Code to start\u{2026}" };

    let mut input = text_input(placeholder, &state.chat.input)
        .font(fonts::mono(Weight::Medium))
        .size(crate::text_scale::px(14.0))
        .padding([8.0, 10.0])
        .style(move |_theme, _status| text_input::Style {
            background: color(p.surface_inset).into(),
            border: Border { color: color(p.border_strong), width: 1.5, radius: 3.0.into() },
            icon: color(p.text_muted),
            placeholder: color(p.text_muted),
            value: color(p.text_strong),
            selection: {
                let mut c = p.accent_solid;
                c.a = 0.35;
                color(c)
            },
        });
    if enabled {
        input = input.on_input(Message::ChatInputChanged).on_submit(Message::ChatSubmit);
    }

    column![input, status_line(state, p)].spacing(6.0).into()
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
pub fn thread_view(state: &State, p: Palette) -> Element<'static, Message> {
    if state.chat_sessions_open {
        return session_list_view(state, p);
    }

    let rows: Vec<Element<'static, Message>> =
        state.chat.messages.iter().map(|m| message_row(m, state.caret_visible, p)).collect();

    let list: Element<'static, Message> = if rows.is_empty() {
        widgets::placeholder("Ask Claude Code about this project to get started", p)
    } else {
        scrollable(column(rows).spacing(16.0).padding(16.0).width(Length::Fill))
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

fn panel_header(title: &'static str, p: Palette, buttons: Vec<Element<'static, Message>>) -> Element<'static, Message> {
    let mut content = row![text(title).font(fonts::mono(Weight::Medium)).size(crate::text_scale::px(11.0)).color(color(p.accent_solid))]
        .spacing(8.0)
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

/// The docked side panel — pushed into `shell.rs`'s `body` row alongside
/// its own resize handle, same composition as `sidebar::view` +
/// `sidebar::resize_handle`.
pub fn docked_view(state: &State, p: Palette) -> Element<'static, Message> {
    let header = panel_header(
        "CLAUDE CODE",
        p,
        vec![
            header_button("THREADS", Message::ChatToggleSessions, p),
            header_button("TAB", Message::ChatOpenTab, p),
            header_button("WIN", Message::ChatPopOut, p),
            header_button("\u{ab}", Message::ChatCollapse, p),
        ],
    );

    container(column![header, thread_view(state, p)].width(Length::Fill).height(Length::Fill))
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

/// The floating window — pushed onto `shell.rs`'s top-level `layers` stack
/// (same mechanism as `toast`/`command_palette`), positioned bottom-right.
pub fn window_view(state: &State, p: Palette) -> Element<'static, Message> {
    let header = panel_header(
        "CLAUDE CODE",
        p,
        vec![
            header_button("THREADS", Message::ChatToggleSessions, p),
            header_button("DOCK", Message::ChatDock, p),
            header_button("\u{2715}", Message::ChatClose, p),
        ],
    );

    let card = container(column![header, thread_view(state, p)].width(Length::Fill).height(Length::Fill))
        .width(Length::Fixed(420.0))
        .height(Length::Fixed(480.0))
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            border: Border { color: color(p.border_strong), width: 1.5, radius: 6.0.into() },
            ..container::Style::default()
        });

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(20.0)
        .into()
}

/// Full-tab presentation — used by `shell.rs`'s `content_area` when
/// `state.active_tab == Some(TabKey::Chat)`.
pub fn tab_view(state: &State, p: Palette) -> Element<'static, Message> {
    let header = panel_header(
        "CLAUDE CODE",
        p,
        vec![header_button("THREADS", Message::ChatToggleSessions, p), header_button("Dock to side", Message::ChatDockFromTab, p)],
    );
    column![header, thread_view(state, p)].width(Length::Fill).height(Length::Fill).into()
}
