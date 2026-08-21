use devscribe_core::theme::{palette, Palette};
use iced::font::Weight;
use iced::widget::{canvas, column, container, row, scrollable, text};
use iced::{Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{Message, State, Tab};
use crate::ui::editor_canvas::{self, EditorCanvas};
use crate::ui::{sidebar, status_bar, tab_bar, title_bar};

fn placeholder_label(tab: Tab) -> &'static str {
    match tab {
        Tab::Code => "No file open \u{2014} pick one from the sidebar",
        Tab::Json => "toolchains.json viewer \u{2014} milestone 6",
        Tab::Search => "search results \u{2014} milestone 6",
        Tab::Diff => "diff panel \u{2014} milestone 6",
    }
}

fn placeholder(label: &'static str, p: Palette) -> Element<'static, Message> {
    container(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(12.0)
            .color(color(p.text_muted)),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(color(p.bg_void).into()),
        ..container::Style::default()
    })
    .into()
}

fn code_area(state: &State, p: Palette) -> Element<'static, Message> {
    let Some(editor) = state.editor.as_ref() else {
        return placeholder(placeholder_label(Tab::Code), p);
    };

    let line_count = editor.document.line_count();
    let program = EditorCanvas {
        document: editor.document.clone(),
        cursor: editor.cursor,
        selection: editor.selection(),
        palette: p,
        caret_visible: state.caret_visible,
        highlights: editor.highlights.clone(),
    };

    let canvas_widget = canvas(program)
        .width(Length::Fill)
        .height(Length::Fixed(editor_canvas::content_height(line_count)));

    container(scrollable(canvas_widget).width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_void).into()),
            ..container::Style::default()
        })
        .into()
}

fn content_area(state: &State, p: Palette) -> Element<'static, Message> {
    match state.active_tab {
        Tab::Code => code_area(state, p),
        other => placeholder(placeholder_label(other), p),
    }
}

pub fn view(state: &State) -> Element<'static, Message> {
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

    container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_void).into()),
            text_color: Some(color(p.text_primary)),
            ..container::Style::default()
        })
        .into()
}
