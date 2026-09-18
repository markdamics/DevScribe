use devscribe_core::lsp::DiagnosticSeverity;
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, mouse_area, responsive, row, scrollable, text, Space};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, ChatMode, EditorState, Message, Pane, State, TabKey};
use crate::ui::editor_canvas::{self, EditorCanvas};
use crate::ui::{
    breadcrumb_bar, chat_panel, command_palette, completions, context_menu, diff_view, find_bar, flash,
    hover_popup, json_view, markdown_view, open_project_prompt, overview_ruler, quick_open, references_panel,
    search_view, settings_panel, sidebar, status_bar, tab_bar, title_bar, toast, welcome,
};
use crate::widgets;

/// The floating "switch back to the rendered view" badge for a JSON/Markdown
/// file currently showing its plain `code_area` — JSON's "Tree View", or
/// Markdown's "Preview" (mirrors `json_view::edit_button`/
/// `markdown_view::edit_button`, the buttons that got it into text mode in
/// the first place). `None` for any other file, or while already showing
/// the tree/preview. Used to live as a row of its own under the tab bar
/// (`breadcrumb_bar::view`); now just this one still-live control, floated
/// over the editor pane's own top-right corner instead of a bar reserved
/// for it.
fn view_mode_toggle(editor: &EditorState, p: Palette) -> Option<Element<'static, Message>> {
    if editor.json.is_some() && editor.json_text_mode {
        Some(breadcrumb_bar::toggle_view_button("Tree View", Message::JsonToggleTextMode, p))
    } else if editor.markdown.is_some() && editor.markdown_text_mode {
        Some(breadcrumb_bar::toggle_view_button(
            "Preview",
            Message::MarkdownToggleTextMode { pane: Pane::Primary },
            p,
        ))
    } else {
        None
    }
}

pub(crate) fn code_area(editor: &EditorState, state: &State, pane: Pane, p: Palette) -> Element<'static, Message> {
    let line_count = editor.document.line_count();
    let find_matches: Vec<(usize, usize)> = editor
        .find
        .as_ref()
        .map(|f| f.matches.iter().map(|m| (m.start, m.end)).collect())
        .unwrap_or_default();
    let find_current = editor.find.as_ref().map(|f| f.current).unwrap_or(0);

    // `editor`'s own fields for `Primary`/`Split` (unchanged) — a solo
    // window's independent, persisted cursor/selection/scroll for `Solo`,
    // since the shared `EditorState` only ever holds that transiently, for
    // the span of one message (`state::with_solo_view`); by the time `view()`
    // runs again its fields are back to whichever tab actually owns them.
    // Everything else below (document, highlights, diagnostics, find, ...)
    // stays shared, read straight off `editor`, same as always.
    let view = state::effective_view(state, editor, pane);
    let document = editor.document.clone();
    let cursor = view.cursor;
    let selection = editor.selection_for(view.cursor, view.selection_anchor);
    let caret_visible = state.caret_visible;
    let highlights = editor.highlights.clone();
    // Recomputed fresh every `view()` rebuild rather than cached on
    // `EditorState` — same reasoning as `find_matches` above: it's cheap and
    // bounded (`bracket::MAX_BRACKET_SCAN`), and the cursor (which this
    // depends on) already changes on essentially every rebuild anyway.
    let bracket_match = devscribe_core::bracket::matching_bracket_pair(
        document.text(),
        &highlights,
        document.char_index(cursor.line, cursor.col),
    );
    // Only computed with no active selection — while dragging/extending a
    // selection, occurrence highlighting would otherwise flicker over
    // whatever identifier the selection's moving end happens to land on,
    // competing visually with the selection itself.
    let occurrences = if selection.is_none() {
        editor.word_occurrences(cursor.line, cursor.col)
    } else {
        Vec::new()
    };
    let diagnostics = editor.diagnostics.clone();
    let gutter_marks = editor.gutter_marks.clone();
    let bracket_depths = editor.bracket_depths.clone();
    let bracket_pair_colorization = state.bracket_pair_colorization;
    let inlay_hints = editor.inlay_hints.clone();
    // Multi-cursor state and bookmarks only ever apply to the primary
    // pane's own editing session — the split pane is a second read/write
    // *view* of the same document (see `effective_view`), but `Ctrl+D`/
    // column-select/bookmarks are keyed off `editor.cursor` specifically,
    // which the split pane doesn't share, so showing them there would be
    // showing state that isn't actually its own.
    let extra_cursors: Vec<(state::CursorPos, Option<(usize, usize)>)> = if pane == Pane::Primary {
        editor.extra_cursors.iter().map(|&(cursor, anchor)| (cursor, editor.selection_for(cursor, anchor))).collect()
    } else {
        Vec::new()
    };
    let bookmarks = if pane == Pane::Primary { editor.bookmarks.clone() } else { Default::default() };
    let content_revision = editor.revision();
    let pending_revert_line = editor.pending_revert_line;
    let problem_lens_enabled = state.problem_lens_enabled;
    let show_line_numbers = state.show_line_numbers;
    let word_wrap = state.word_wrap;
    let font_size = state.editor_font_size;
    let scroll_offset = view.scroll_offset;
    // Roadmap item 18: which enclosing scopes have scrolled above the
    // viewport, pinned atop the content instead — see `EditorCanvas::
    // sticky_scopes`'s own doc comment for why word wrap opts out.
    let sticky_scopes = if word_wrap {
        Vec::new()
    } else {
        editor.sticky_scopes_at_line(editor_canvas::first_visible_line(scroll_offset, font_size))
    };
    let max_line_chars = editor.max_line_chars();
    // Position-checked here (not trusted from storage) — see
    // `GhostCompletion`'s own doc comment. Only the suggestion's first line
    // is shown; see `EditorCanvas::ghost_text`'s own doc comment for why.
    let ghost_text = editor
        .ghost_completion
        .as_ref()
        .filter(|g| g.at == view.cursor)
        .and_then(|g| g.insert_text.lines().next())
        .filter(|line| !line.is_empty())
        .map(str::to_string);

    // `responsive` hands us the pane's actual available height up front
    // (rather than waiting for a scroll event to learn it), so the canvas
    // knows which lines are visible from the very first frame.
    let editor_pane = responsive(move |size| {
        // Computed here (not on `EditorCanvas`) because it needs `document`
        // and `size.width`, both of which only meet in this closure —
        // `draw`/`hit_test` then reuse this same copy rather than each
        // re-deriving their own. Left empty (and unused) when word wrap is
        // off — see `wrap_row_offsets`'s own doc comment for the O(document
        // length) cost this accepts whenever it's actually on.
        let wrap_offsets = std::rc::Rc::new(if word_wrap {
            let wrap_cols = editor_canvas::wrap_cols_for_pane(size.width, font_size);
            editor_canvas::wrap_row_offsets(&document, wrap_cols)
        } else {
            Vec::new()
        });

        let program = EditorCanvas {
            document: document.clone(),
            cursor,
            selection,
            palette: p,
            caret_visible,
            highlights: highlights.clone(),
            diagnostics: diagnostics.clone(),
            gutter_marks: gutter_marks.clone(),
            bracket_depths: bracket_depths.clone(),
            bracket_colorization_enabled: bracket_pair_colorization,
            inlay_hints: inlay_hints.clone(),
            content_revision,
            pending_revert_line,
            problem_lens_enabled,
            show_line_numbers,
            word_wrap,
            wrap_offsets: wrap_offsets.clone(),
            font_size,
            find_matches: find_matches.clone(),
            find_current,
            bracket_match,
            occurrences: occurrences.clone(),
            scroll_offset,
            viewport_height: size.height,
            ghost_text: ghost_text.clone(),
            pane,
            extra_cursors: extra_cursors.clone(),
            bookmarks: bookmarks.clone(),
            sticky_scopes: sticky_scopes.clone(),
        };

        // Unwrapped: at least the pane's own width (`size.width`, from
        // `responsive`) so a short-lined file still fills the pane exactly
        // like before; wider when the document's longest line needs more
        // than that, which is what makes it something to scroll sideways
        // *into*. Wrapped: always exactly the pane's width — word wrap's
        // whole point is that nothing needs scrolling sideways to be seen.
        let canvas_width = if word_wrap {
            size.width
        } else {
            editor_canvas::content_width(max_line_chars, font_size).max(size.width)
        };
        let content_rows = if word_wrap {
            wrap_offsets.last().copied().unwrap_or(0) as usize
        } else {
            line_count
        };

        let canvas_widget = canvas(program)
            .width(Length::Fixed(canvas_width))
            .height(Length::Fixed(editor_canvas::content_height(
                content_rows, font_size,
            )));

        let scroll = scrollable(canvas_widget)
            .id(match pane {
                Pane::Primary => state::editor_scroll_id(),
                Pane::Split => state::split_editor_scroll_id(),
                Pane::Solo(id) => state::solo_editor_scroll_id(id),
            })
            .direction(if word_wrap {
                scrollable::Direction::Vertical(widgets::thin_scrollbar())
            } else {
                scrollable::Direction::Both {
                    vertical: widgets::thin_scrollbar(),
                    horizontal: widgets::thin_scrollbar(),
                }
            })
            .style(widgets::scrollbar_style(p))
            .on_scroll(move |viewport| {
                let offset = viewport.absolute_offset();
                let bounds = viewport.bounds();
                Message::EditorScrolled {
                    offset: offset.y,
                    viewport_height: bounds.height,
                    offset_x: offset.x,
                    viewport_width: bounds.width,
                    pane,
                }
            })
            .width(Length::Fill)
            .height(Length::Fill);

        // Every marker row below is resolved through the same word-wrap
        // mapping the canvas itself draws with (`row_for_line`), so the
        // ruler's ticks land on the same visual row the wrapped text
        // actually renders at — see `overview_ruler`'s own doc comment.
        let row_for_line = |line: usize| -> usize {
            if word_wrap {
                wrap_offsets.get(line).copied().unwrap_or(0) as usize
            } else {
                line
            }
        };
        let diagnostic_rows: Vec<(usize, DiagnosticSeverity)> =
            diagnostics.iter().map(|d| (row_for_line(d.start.line), d.severity)).collect();
        let find_rows: Vec<usize> =
            find_matches.iter().map(|&(start, _)| row_for_line(document.line_col(start).0)).collect();
        let bookmark_rows: Vec<usize> = bookmarks.iter().map(|&line| row_for_line(line)).collect();

        let ruler = canvas(overview_ruler::OverviewRuler {
            palette: p,
            pane,
            total_rows: content_rows.max(1),
            content_height: editor_canvas::content_height(content_rows, font_size),
            viewport_height: size.height,
            scroll_offset,
            cursor_row: row_for_line(cursor.line),
            diagnostic_rows,
            find_rows,
            bookmark_rows,
        })
        .width(Length::Fixed(overview_ruler::WIDTH))
        .height(Length::Fill);

        row![scroll, ruler].width(Length::Fill).height(Length::Fill).into()
    });

    let base = container(editor_pane)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        });

    let editor_area: Element<'static, Message> = if editor.find.is_some() {
        iced::widget::stack![base, find_bar::view(editor, p)].into()
    } else {
        base.into()
    };

    // The split pane skips this floating badge rather than rendering a
    // non-interactive copy of it: `JsonToggleTextMode`/`MarkdownToggleTextMode`
    // both flip the *shared* `EditorState`'s mode flag, not anything scoped
    // to a pane, so showing a second live control for it over the split file
    // would just be a second way to trigger the exact same toggle.
    let editor_area: Element<'static, Message> = if pane == Pane::Primary {
        if let Some(toggle) = view_mode_toggle(editor, p) {
            let positioned = container(toggle)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_right(Length::Fill)
                .padding(8.0);
            iced::widget::stack![editor_area, positioned].into()
        } else {
            editor_area
        }
    } else {
        editor_area
    };

    editor_area
}

/// The mockup's styled "no buffer open" state (item 22): a two-line message
/// plus an "Open palette ⌘K" button, replacing the plain placeholder text
/// only when *no tab at all* is open — the defensive fallback below (an
/// `active_tab` pointing at a file with no matching editor, which shouldn't
/// normally happen) keeps the plain `widgets::placeholder` instead.
fn no_buffer_state(p: Palette) -> Element<'static, Message> {
    let open_palette = button(
        text("Open palette \u{2318}K")
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_strong)),
    )
    .padding([6.0, 14.0])
    .on_press(Message::TogglePalette)
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
                width: 1.5,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    });

    let content = column![
        text("NO BUFFER OPEN")
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
        text("Select a file in the explorer, or open the command palette.")
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(15.0))
            .color(color(p.text_body)),
        open_palette,
    ]
    .spacing(12.0)
    .align_x(Alignment::Center);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        })
        .into()
}

fn primary_content(state: &State, p: Palette) -> Element<'_, Message> {
    let Some(key) = state.active_tab.as_ref() else {
        return no_buffer_state(p);
    };
    match key {
        TabKey::File(path) => {
            let Some(editor) = state::find_editor(state, path) else {
                return widgets::placeholder("No file open \u{2014} pick one from the sidebar", p);
            };
            if editor.json.is_some() && !editor.json_text_mode {
                json_view::view(editor, p)
            } else if editor.markdown.is_some() && !editor.markdown_text_mode {
                markdown_view::view(editor, Pane::Primary, state.markdown_preview_zoom, p)
            } else {
                code_area(editor, state, Pane::Primary, p)
            }
        }
        TabKey::Diff(path) => diff_view::view(state, path, p),
        TabKey::Search => search_view::view(state, p),
        TabKey::Chat => chat_panel::tab_view(state, p),
    }
}

/// The split pane's own thin header — just the filename and a close button,
/// unlike the primary pane's full breadcrumb bar (see `code_area`'s own
/// comment on why it skips one). `None` whenever `state.split_tab` isn't
/// pointing at a still-open file, which `update()`'s own normalization
/// keeps from lingering for more than one frame.
fn split_pane_view(state: &State, p: Palette) -> Option<Element<'_, Message>> {
    let TabKey::File(path) = state.split_tab.as_ref()? else {
        return None;
    };
    let editor = state::find_editor(state, path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    let header = row![
        text(name)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_body))
            .width(Length::Fill),
        button(
            text("\u{2715}")
                .font(fonts::mono(Weight::Medium))
                .size(crate::text_scale::px(13.0))
                .color(color(p.text_muted)),
        )
        .padding([2.0, 6.0])
        .on_press(Message::ToggleSplitView)
        .style(move |_theme, status| button::Style {
            background: if status == button::Status::Hovered {
                Some(color(p.surface_hover).into())
            } else {
                None
            },
            ..button::Style::default()
        }),
    ]
    .align_y(Alignment::Center)
    .padding([6.0, 12.0]);

    let body = if editor.json.is_some() && !editor.json_text_mode {
        json_view::view(editor, p)
    } else if editor.markdown.is_some() && !editor.markdown_text_mode {
        markdown_view::view(editor, Pane::Split, state.markdown_preview_zoom, p)
    } else {
        code_area(editor, state, Pane::Split, p)
    };

    Some(
        column![header, body]
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    )
}

/// A drag handle between the primary and split panes — same 4px idiom as
/// `sidebar::resize_handle`/`chat_panel::resize_handle`, driving
/// `state.split_primary_width` (the primary pane's width; the split pane
/// itself always fills whatever's left) via the same window-wide
/// cursor-tracking subscription pattern (`Message::SplitResizeStarted`,
/// then `state::split_resize_events` while `state.split_resizing`).
fn split_resize_handle(p: Palette) -> Element<'static, Message> {
    mouse_area(
        container(Space::new().width(Length::Fixed(4.0)).height(Length::Fill))
            .width(Length::Fixed(4.0))
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(color(p.border_hairline).into()),
                ..container::Style::default()
            }),
    )
    .interaction(iced::mouse::Interaction::ResizingHorizontally)
    .on_press(Message::SplitResizeStarted)
    .into()
}

fn content_area(state: &State, p: Palette) -> Element<'_, Message> {
    let primary = primary_content(state, p);
    match split_pane_view(state, p) {
        Some(split) => row![
            container(primary).width(Length::Fixed(state.split_primary_width)).height(Length::Fill),
            split_resize_handle(p),
            container(split).width(Length::Fill).height(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
        None => primary,
    }
}

pub fn view(state: &State, window: iced::window::Id) -> Element<'_, Message> {
    crate::text_scale::set(state.ui_font_scale);

    let p = state::active_palette(state);

    // A "solo" window (`Message::OpenInNewWindow`) gets a completely
    // different, minimal view — no sidebar/tab bar/chat panel — rather than
    // this function's usual full-chrome body below.
    if state.solo_windows.contains_key(&window) {
        return crate::ui::solo_window::view(state, window, p);
    }

    // No project open — the welcome screen replaces the whole editor
    // (title bar included: the window still has default OS decorations,
    // confirmed in `main.rs`, so this doesn't lose window-move/close
    // controls), matching the mockup's own full-window
    // `position:fixed;inset:0` welcome overlay.
    if state.welcome_open {
        return welcome::view(state, p);
    }

    let mut main_column = column![tab_bar::view(state, p), content_area(state, p)];
    if state.problems_panel_open {
        main_column = main_column.push(status_bar::dock_panel(state, p));
    }
    if state.references_open {
        main_column = main_column.push(references_panel::dock_panel(state, p));
    }
    let main_column = main_column
        .push(status_bar::view(state, p))
        .width(Length::Fill)
        .height(Length::Fill);

    let mut body = row![sidebar::view(state, p)];
    if !state.sidebar_collapsed {
        body = body.push(sidebar::resize_handle(p));
    }
    body = body.push(main_column);
    match state.chat_mode {
        ChatMode::Docked => {
            body = body.push(chat_panel::resize_handle(p));
            body = body.push(chat_panel::docked_view(state, p));
        }
        ChatMode::Collapsed => body = body.push(chat_panel::collapsed_rail(p)),
        ChatMode::Closed => {}
    }
    let body = body.width(Length::Fill).height(Length::Fill);

    let root = column![title_bar::view(state, p), body]
        .width(Length::Fill)
        .height(Length::Fill);

    let base = container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            text_color: Some(color(p.text_strong)),
            ..container::Style::default()
        });

    let mut layers: Vec<Element<'_, Message>> = vec![base.into()];
    layers.extend(completions::view(state, p));
    layers.extend(completions::signature_help_view(state, p));
    layers.extend(hover_popup::view(state, p));
    layers.extend(breadcrumb_bar::hover_view(state, p));
    layers.extend(status_bar::background_tasks_panel(state, p));
    layers.extend(status_bar::eol_picker_view(state, p));
    layers.extend(status_bar::language_picker_view(state, p));
    layers.extend(status_bar::encoding_info_view(state, p));
    layers.extend(command_palette::view(state));
    layers.extend(quick_open::view(state));
    layers.extend(settings_panel::view(state));
    layers.extend(tab_bar::overflow_menu(state, p));
    layers.extend(tab_bar::hover_preview(state, p));
    layers.extend(tab_bar::switcher_view(state, p));
    layers.extend(sidebar::projects_menu(state, p));
    layers.extend(open_project_prompt::view(state, p));
    layers.extend(context_menu::view(state, p));
    layers.extend(context_menu::editor_menu_view(state, p));
    layers.extend(context_menu::rename_prompt_view(state, p));
    layers.extend(chat_panel::view_menu(state, p));
    layers.extend(chat_panel::actions_menu(state, p));
    layers.extend(toast::view(state));
    layers.extend(flash::view(state));

    // Tracks `State::mouse_pos` on every move across the whole window —
    // `mouse_area` only observes movement (it doesn't capture the event),
    // so this doesn't steal clicks/hovers from anything underneath. It
    // exists purely so a right-click's position is known even for widgets
    // like the sidebar tree's `mouse_area::on_right_press`, which carries
    // no position of its own — see `ui::context_menu::view`.
    mouse_area(iced::widget::Stack::with_children(layers))
        .on_move(Message::MouseMoved)
        .into()
}
