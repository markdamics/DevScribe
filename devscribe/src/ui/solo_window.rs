//! The minimal window opened by `Message::OpenInNewWindow` — just a thin
//! filename/settings header plus the editor canvas (or the Markdown/JSON
//! preview, matching whichever view the main window would show for the same
//! file), no sidebar/tab bar/chat panel. The file itself is still a normal
//! `open_tabs` entry (see `state::open_in_new_window`), so its document,
//! diagnostics, highlighting, undo history, and LSP connection are all the
//! exact same shared `EditorState` the main window's tab uses — only the
//! cursor/selection/scroll are this window's own, via `Pane::Solo`.

use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::{button, column, container, row, text};
use iced::window;
use iced::{Alignment, Element, Length};

use crate::color::color;
use crate::fonts;
use crate::state::{self, Message, Pane, State};
use crate::ui::{json_view, markdown_view, settings_panel, shell};
use crate::widgets;

fn settings_button(p: Palette) -> Element<'static, Message> {
    button(text("\u{2699}").size(crate::text_scale::px(15.0)))
        .padding([4.0, 8.0])
        .on_press(Message::ToggleSettings)
        .style(move |_theme, status| {
            let hovered = status == button::Status::Hovered;
            button::Style {
                background: if hovered {
                    Some(color(p.surface_raised).into())
                } else {
                    None
                },
                text_color: if hovered {
                    color(p.accent_solid)
                } else {
                    color(p.text_muted)
                },
                ..button::Style::default()
            }
        })
        .into()
}

fn header(name: &str, p: Palette) -> Element<'static, Message> {
    let bar = row![
        text(name.to_string())
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_body))
            .width(Length::Fill),
        settings_button(p),
    ]
    .align_y(Alignment::Center)
    .padding([6.0, 10.0]);

    container(bar)
        .width(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_base).into()),
            ..container::Style::default()
        })
        .into()
}

pub fn view(state: &State, window: window::Id, p: Palette) -> Element<'_, Message> {
    let Some(solo) = state.solo_windows.get(&window) else {
        return widgets::placeholder("Window closed", p);
    };
    let name = solo.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    let Some(editor) = state::find_editor(state, &solo.path) else {
        // The file's tab was closed from the main window while this solo
        // window stayed open — shouldn't normally happen since closing a
        // solo window never touches `open_tabs`, but the reverse (closing
        // the tab from the main window) doesn't tear this window down, so
        // it needs its own fallback.
        return widgets::placeholder("File closed", p);
    };

    let pane = Pane::Solo(window);
    // Same branching `shell::primary_content` uses for the main window's own
    // active tab — a solo window shows whichever of these its file would get
    // there too, preview zoom and all, rather than always the plain canvas.
    let body = if editor.json.is_some() && !editor.json_text_mode {
        json_view::view(editor, p)
    } else if editor.markdown.is_some() && !editor.markdown_text_mode {
        markdown_view::view(editor, pane, state.markdown_preview_zoom, p)
    } else {
        shell::code_area(editor, state, pane, p)
    };

    let root = column![header(&name, p), widgets::hline(color(p.border_hairline)), body]
        .width(Length::Fill)
        .height(Length::Fill);

    let base: Element<'_, Message> = container(root)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            text_color: Some(color(p.text_strong)),
            ..container::Style::default()
        })
        .into();

    match settings_panel::view(state) {
        Some(panel) => iced::widget::stack![base, panel].into(),
        None => base,
    }
}
