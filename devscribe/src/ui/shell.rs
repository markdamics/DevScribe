use devscribe_core::theme::{palette, Palette};
use iced::widget::{canvas, column, container, row, scrollable};
use iced::{Element, Length};

use crate::color::color;
use crate::state::{self, EditorState, Message, State, TabKey};
use crate::ui::editor_canvas::{self, EditorCanvas};
use crate::ui::{
    command_palette, diff_view, find_bar, json_view, search_view, settings_panel, sidebar,
    status_bar, tab_bar, title_bar, toast,
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

fn content_area(state: &State, p: Palette) -> Element<'static, Message> {
    let Some(key) = state.active_tab.as_ref() else {
        return widgets::placeholder("No file open \u{2014} pick one from the sidebar", p);
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
    layers.extend(toast::view(state));

    iced::widget::Stack::with_children(layers).into()
}
