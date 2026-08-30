use devscribe_core::theme::{palette, Palette};
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, responsive, row, scrollable, text};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, ChatMode, EditorState, Message, State, TabKey};
use crate::ui::editor_canvas::{self, EditorCanvas};
use crate::ui::{
    breadcrumb_bar, chat_panel, command_palette, completions, context_menu, diff_view, find_bar, flash,
    json_view, search_view, settings_panel, sidebar, status_bar, tab_bar, title_bar, toast, welcome,
};
use crate::widgets;

fn code_area(editor: &EditorState, state: &State, p: Palette) -> Element<'static, Message> {
    let line_count = editor.document.line_count();
    let find_matches: Vec<(usize, usize)> = editor
        .find
        .as_ref()
        .map(|f| f.matches.iter().map(|m| (m.start, m.end)).collect())
        .unwrap_or_default();
    let find_current = editor.find.as_ref().map(|f| f.current).unwrap_or(0);

    let document = editor.document.clone();
    let cursor = editor.cursor;
    let selection = editor.selection();
    let caret_visible = state.caret_visible;
    let highlights = editor.highlights.clone();
    let diagnostics = editor.diagnostics.clone();
    let gutter_marks = editor.gutter_marks.clone();
    let problem_lens_enabled = state.problem_lens_enabled;
    let font_size = state.editor_font_size;
    let scroll_offset = editor.scroll_offset;

    // `responsive` hands us the pane's actual available height up front
    // (rather than waiting for a scroll event to learn it), so the canvas
    // knows which lines are visible from the very first frame.
    let editor_pane = responsive(move |size| {
        let program = EditorCanvas {
            document: document.clone(),
            cursor,
            selection,
            palette: p,
            caret_visible,
            highlights: highlights.clone(),
            diagnostics: diagnostics.clone(),
            gutter_marks: gutter_marks.clone(),
            problem_lens_enabled,
            font_size,
            find_matches: find_matches.clone(),
            find_current,
            scroll_offset,
            viewport_height: size.height,
        };

        let canvas_widget = canvas(program)
            .width(Length::Fill)
            .height(Length::Fixed(editor_canvas::content_height(
                line_count, font_size,
            )));

        scrollable(canvas_widget)
            .id(state::editor_scroll_id())
            .on_scroll(|viewport| Message::EditorScrolled {
                offset: viewport.absolute_offset().y,
                viewport_height: viewport.bounds().height,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
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

    column![breadcrumb_bar::view(editor, p), editor_area]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
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

fn content_area(state: &State, p: Palette) -> Element<'_, Message> {
    let Some(key) = state.active_tab.as_ref() else {
        return no_buffer_state(p);
    };
    match key {
        TabKey::File(path) => {
            let Some(editor) = state::find_editor(state, path) else {
                return widgets::placeholder("No file open \u{2014} pick one from the sidebar", p);
            };
            if editor.json.is_some() {
                json_view::view(editor, p)
            } else {
                code_area(editor, state, p)
            }
        }
        TabKey::Diff(path) => diff_view::view(state, path, p),
        TabKey::Search => search_view::view(state, p),
        TabKey::Chat => chat_panel::tab_view(state, p),
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    crate::text_scale::set(state.ui_font_scale);

    let p = palette(state.theme_mode, state.accent);

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
    layers.extend(command_palette::view(state));
    layers.extend(settings_panel::view(state));
    layers.extend(tab_bar::overflow_menu(state, p));
    layers.extend(sidebar::projects_menu(state, p));
    layers.extend(context_menu::view(state, p));
    layers.extend(chat_panel::view_menu(state, p));
    layers.extend(chat_panel::actions_menu(state, p));
    layers.extend(toast::view(state));
    layers.extend(flash::view(state));

    iced::widget::Stack::with_children(layers).into()
}
