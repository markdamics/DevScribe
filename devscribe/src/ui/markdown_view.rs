//! A rendered preview for the currently open `.md`/`.markdown` file, using
//! iced's own `widget::markdown` renderer (headings, lists, code blocks,
//! links, ...) restyled from this app's `Palette` instead of iced's built-in
//! `Theme` — every other widget in the app already sources its colors the
//! same way. Distinct from `json_view.rs`'s tree in shape, but the same
//! idea: a read-only rendering of the buffer that defaults over the plain
//! `code_area`, with an "Edit as Text" escape hatch back to it.
use devscribe_core::theme::Palette;
use iced::font::Weight;
use iced::widget::markdown::Catalog as _;
use iced::widget::{button, column, container, image, markdown, rich_text, row, scrollable, text, Space};
use iced::{Alignment, Border, Color, ContentFit, Element, Length, Padding};
use std::path::Path;

use crate::color::color;
use crate::fonts;
use crate::state::{
    self, resolve_local_markdown_path, EditorState, MarkdownHeading, Message, Pane,
    MARKDOWN_PREVIEW_ZOOM_MAX, MARKDOWN_PREVIEW_ZOOM_MIN, MARKDOWN_PREVIEW_ZOOM_STEP,
};
use crate::widgets;

/// Mirrors `json_view.rs::edit_button` — same affordance, its own message
/// variant since Markdown's text-mode flag is independent of JSON's.
fn edit_button(pane: Pane, p: Palette) -> Element<'static, Message> {
    badge_button("Edit as Text", Message::MarkdownToggleTextMode { pane }, p)
}

/// The table-of-contents panel's expand/collapse toggle, same affordance as
/// `edit_button` one spot over in the badge row.
fn toc_button(pane: Pane, open: bool, p: Palette) -> Element<'static, Message> {
    badge_button(if open { "Hide Outline" } else { "Outline" }, Message::MarkdownToggleToc { pane }, p)
}

fn badge_button(label: &'static str, on_press: Message, p: Palette) -> Element<'static, Message> {
    button(
        text(label)
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted)),
    )
    .padding([4.0, 10.0])
    .on_press(on_press)
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
                width: 1.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// A `-`/`+` stepper for `State::markdown_preview_zoom`, right in the
/// preview's own header — the Settings panel's "Markdown Preview Zoom" row
/// (`settings_panel.rs::markdown_zoom_row`) drives the same
/// `Message::SetMarkdownPreviewZoom`, just further away. Compact (no
/// boxed value between the buttons, unlike Settings' `stepper_row`) since
/// this has to share a single header row with PREVIEW/Outline/Edit.
fn zoom_control(zoom: f32, p: Palette) -> Element<'static, Message> {
    row![
        zoom_step_button(
            "\u{2212}",
            Message::SetMarkdownPreviewZoom(
                (zoom - MARKDOWN_PREVIEW_ZOOM_STEP).clamp(MARKDOWN_PREVIEW_ZOOM_MIN, MARKDOWN_PREVIEW_ZOOM_MAX)
            ),
            p,
        ),
        text(format!("{:.0}%", zoom * 100.0))
            .font(fonts::mono(Weight::Medium))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted))
            .width(Length::Fixed(36.0))
            .align_x(Alignment::Center),
        zoom_step_button(
            "+",
            Message::SetMarkdownPreviewZoom(
                (zoom + MARKDOWN_PREVIEW_ZOOM_STEP).clamp(MARKDOWN_PREVIEW_ZOOM_MIN, MARKDOWN_PREVIEW_ZOOM_MAX)
            ),
            p,
        ),
    ]
    .spacing(2.0)
    .align_y(Alignment::Center)
    .into()
}

fn zoom_step_button(label: &'static str, on_press: Message, p: Palette) -> Element<'static, Message> {
    button(
        text(label)
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(12.0))
            .color(color(p.text_muted)),
    )
    .padding([2.0, 8.0])
    .on_press(on_press)
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
                width: 1.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        }
    })
    .into()
}

/// The table-of-contents panel itself: one row per heading, indented by
/// level, each a link to its own `#slug` — routed through the same
/// `Message::OpenMarkdownLink` (and so the same `handle_markdown_link`
/// anchor-resolution/scroll) an in-document `[text](#heading)` link would
/// use, rather than a separate scroll-only message.
fn toc_panel<'a>(pane: Pane, headings: &'a [MarkdownHeading], p: Palette) -> Element<'a, Message> {
    let entries = headings.iter().map(|heading| {
        button(
            text(heading.text.as_str())
                .font(fonts::sans(Weight::Normal))
                .size(crate::text_scale::px(12.0))
                .color(color(p.text_muted)),
        )
        .padding(Padding { top: 3.0, right: 6.0, bottom: 3.0, left: 6.0 + f32::from(heading.level - 1) * 10.0 })
        .width(Length::Fill)
        .on_press(Message::OpenMarkdownLink { pane, url: format!("#{}", heading.slug) })
        .style(move |_theme, status| button::Style {
            background: (status == button::Status::Hovered).then(|| color(p.surface_hover).into()),
            text_color: color(p.text_muted),
            ..button::Style::default()
        })
        .into()
    });

    scrollable(column(entries).width(Length::Fill).padding([8.0, 4.0]))
        .width(Length::Fixed(200.0))
        .height(Length::Fill)
        .into()
}

/// Colors/fonts for iced's markdown renderer, sourced from this app's own
/// `Palette` rather than `iced::Theme` — built by hand (not the crate's
/// `Style::from_palette`, which converts iced's own unrelated `theme::Palette`
/// type of the same name) so it matches the rest of the app's styling.
fn style(p: Palette) -> markdown::Style {
    markdown::Style {
        font: fonts::sans(Weight::Normal),
        inline_code_highlight: markdown::Highlight {
            background: color(p.surface_hover).into(),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
        },
        inline_code_padding: Padding { top: 0.0, right: 4.0, bottom: 0.0, left: 4.0 },
        inline_code_color: color(p.text_strong),
        inline_code_font: fonts::mono(Weight::Medium),
        code_block_font: fonts::mono(Weight::Medium),
        link_color: color(p.accent_solid),
    }
}

/// A `markdown::Viewer` that renders `Item::Image` for real, instead of
/// iced's own default (just the alt text, boxed like a code block) —
/// everything else is left at the trait's defaults. `Message` stays `Uri`,
/// same as iced's own `markdown::view`: link-click routing happens via
/// `.map` at the call site (`view`, below), not here, since a link's
/// destination pane isn't known until then.
struct PreviewViewer<'a> {
    /// The previewed file's own directory — `None` for a buffer with no
    /// real path yet (an untitled scratch file), in which case every image
    /// falls back to the alt-text placeholder, same as an unresolvable one.
    base_dir: Option<&'a Path>,
    /// `State::markdown_preview_zoom` — images render at
    /// `IMAGE_BASE_WIDTH * zoom` (clamped down to whatever room the pane
    /// actually has), same multiplier `view` already applies to text size,
    /// so a resized image tracks the same zoom control rather than always
    /// filling the pane edge-to-edge regardless of it.
    zoom: f32,
}

/// A local image's width at 100% zoom, before the pane's own available
/// width (via normal layout clamping) or `PreviewViewer::zoom` scale it —
/// picked to comfortably fit typical preview panes without spilling past
/// the edge at the default zoom.
const IMAGE_BASE_WIDTH: f32 = 640.0;

impl<'a> markdown::Viewer<'a, markdown::Uri> for PreviewViewer<'a> {
    fn on_link_click(url: markdown::Uri) -> markdown::Uri {
        url
    }

    /// Local images (relative or absolute, resolved the same way
    /// `handle_markdown_link` resolves a relative link) render as an actual
    /// `image` widget, scaled to the preview's width. Remote images
    /// (`http(s)://...`) and anything that doesn't resolve to a real file
    /// fall back to iced's own default: the alt text, boxed like a code
    /// block — there's no fetch-over-the-network pipeline here, only a
    /// local-disk read.
    fn image(
        &self,
        settings: markdown::Settings,
        url: &'a markdown::Uri,
        _title: &'a str,
        alt: &markdown::Text,
    ) -> Element<'a, markdown::Uri> {
        let Some(path) = self.base_dir.and_then(|dir| resolve_local_markdown_path(dir, url)) else {
            return container(rich_text(alt.spans(settings.style)).on_link_click(Self::on_link_click))
                .padding(settings.spacing.0)
                .class(iced::Theme::code_block())
                .into();
        };

        container(
            image::Image::new(path)
                .width(Length::Fixed(IMAGE_BASE_WIDTH * self.zoom))
                .content_fit(ContentFit::Contain),
        )
        .padding(settings.spacing.0 / 2.0)
        .into()
    }
}

/// Renders `editor`'s Markdown preview for `pane`, at `zoom` (`State::
/// markdown_preview_zoom`, the Settings panel's "Markdown Preview Zoom"
/// stepper — a multiplier on top of the preview's own base text size, kept
/// independent of `text_scale`'s chrome-wide scale). Only called once the
/// active tab is known to be a `.md`/`.markdown` file with
/// `markdown_text_mode` off — see `shell.rs::content_area`. Tied to
/// `editor`'s own lifetime (unlike `json_view::view`'s `'static`): iced's
/// `markdown::view` ties its output to the borrow of the `Item`s it's
/// given, even though every span it builds is already an owned `String`
/// underneath.
pub fn view(editor: &EditorState, pane: Pane, zoom: f32, p: Palette) -> Element<'_, Message> {
    let Some(content) = editor.markdown.as_ref() else {
        return widgets::placeholder(format!("{} isn't a Markdown file", editor.path.display()), p);
    };

    let mut badge = row![
        text("PREVIEW")
            .font(fonts::mono(Weight::Bold))
            .size(crate::text_scale::px(13.0))
            .color(color(p.text_muted)),
        Space::new().width(Length::Fill),
        zoom_control(zoom, p),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center);
    if !editor.markdown_headings.is_empty() {
        badge = badge.push(toc_button(pane, editor.markdown_toc_open, p));
    }
    let badge = badge.push(edit_button(pane, p)).padding([8.0, 12.0]);

    let settings = markdown::Settings::with_text_size(crate::text_scale::px(15.0) * zoom, style(p));
    let viewer = PreviewViewer { base_dir: editor.path.parent(), zoom };
    let rendered = markdown::view_with(content.items(), settings, &viewer)
        .map(move |url| Message::OpenMarkdownLink { pane, url });

    let preview = scrollable(container(rendered).width(Length::Fill).padding([4.0, 16.0]))
        .id(state::markdown_scroll_id(pane))
        .width(Length::Fill)
        .height(Length::Fill);

    let body: Element<'_, Message> = if editor.markdown_toc_open && !editor.markdown_headings.is_empty() {
        row![toc_panel(pane, &editor.markdown_headings, p), preview].height(Length::Fill).into()
    } else {
        preview.into()
    };

    container(column![badge, body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(color(p.bg_canvas).into()),
            ..container::Style::default()
        })
        .into()
}
