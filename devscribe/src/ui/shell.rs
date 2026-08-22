use devscribe_core::theme::{palette, Palette};
use iced::font::Weight;
use iced::widget::{button, canvas, column, container, row, scrollable, text};
use iced::{Alignment, Border, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, EditorState, Message, State, TabKey};
use crate::ui::editor_canvas::{self, EditorCanvas};
use crate::ui::{
    command_palette, context_menu, diff_view, find_bar, flash, json_view, search_view,
    settings_panel, sidebar, status_bar, tab_bar, title_bar, toast,
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
    let program = EditorCanvas {
        document: editor.document.clone(),
        cursor: editor.cursor,
        selection: editor.selection(),
        palette: p,
        caret_visible: state.caret_visible,
        highlights: editor.highlights.clone(),
        diagnostics: editor.diagnostics.clone(),
        problem_lens_enabled: state.problem_lens_enabled,
        font_size: state.editor_font_size,
        find_matches,
        find_current,
    };

    let canvas_widget = canvas(program).width(Length::Fill).height(Length::Fixed(
        editor_canvas::content_height(line_count, state.editor_font_size),
    ));

    let base = container(scrollable(canvas_widget).width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_void).into()),
            ..container::Style::default()
        });

    if editor.find.is_some() {
        iced::widget::stack![base, find_bar::view(editor, p)].into()
    } else {
        base.into()
    }
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
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_primary)),
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
                color: color(p.line_neutral),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    });

    let content = column![
        text("NO BUFFER OPEN")
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(10.0))
            .color(color(p.text_muted)),
        text("Select a file in the explorer, or open the command palette.")
            .font(fonts::sans(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_secondary)),
        open_palette,
    ]
    .spacing(10.0)
    .align_x(Alignment::Center);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .center(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_void).into()),
            ..container::Style::default()
        })
        .into()
}

fn content_area(state: &State, p: Palette) -> Element<'static, Message> {
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
    }
}

pub fn view(state: &State) -> Element<'static, Message> {
    crate::text_scale::set(state.ui_font_scale);

    let p = palette(state.theme);

    let main_column = column![
        tab_bar::view(state, p),
        content_area(state, p),
        status_bar::view(state, p),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let body = row![sidebar::view(state, p), main_column]
        .width(Length::Fill)
        .height(Length::Fill);

    let root = column![title_bar::view(state, p), body]
        .width(Length::Fill)
        .height(Length::Fill);

    let base = container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_void).into()),
            text_color: Some(color(p.text_primary)),
            ..container::Style::default()
        });

    let mut layers: Vec<Element<'static, Message>> = vec![base.into()];
    layers.extend(command_palette::view(state));
    layers.extend(settings_panel::view(state));
    layers.extend(tab_bar::overflow_menu(state, p));
    layers.extend(context_menu::view(state, p));
    layers.extend(toast::view(state));
    layers.extend(flash::view(state));

    iced::widget::Stack::with_children(layers).into()
}
