mod color;
mod density;
mod fonts;
mod fs_tree;
mod state;
mod text_scale;
mod ui;
mod widgets;

use iced::font::Weight;
use iced::window;
use iced::Element;
use state::{Message, State};

const ICON_RGBA: &[u8] = include_bytes!("../assets/icons/devscribe-icon-64.rgba");
const ICON_SIZE: u32 = 64;

fn view(state: &State) -> Element<'_, Message> {
    ui::shell::view(state)
}

fn window_icon() -> window::Icon {
    window::icon::from_rgba(ICON_RGBA.to_vec(), ICON_SIZE, ICON_SIZE)
        .expect("devscribe-icon-64.rgba must be exactly ICON_SIZE x ICON_SIZE RGBA8 pixels")
}

pub fn main() -> iced::Result {
    let mut app = iced::application(State::default, state::update, view)
        .title("DevScribe")
        .default_font(fonts::sans(Weight::Normal))
        .subscription(state::subscription)
        .window(window::Settings {
            icon: Some(window_icon()),
            ..window::Settings::default()
        })
        .window_size((1280.0, 800.0));

    for bytes in fonts::BYTES {
        app = app.font(*bytes);
    }

    app.run()
}
