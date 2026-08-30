mod claude_permission_hook;
mod color;
mod density;
mod fonts;
mod fs_tree;
mod logging;
mod recent_projects;
mod server_install;
mod session;
mod settings;
mod state;
mod text_scale;
mod ui;
mod widgets;

use std::path::Path;

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
    // A hidden, non-GUI entry point: `claude` (spawned by the AI Chat
    // Assist backend, `devscribe_core::claude_agent`) re-invokes this same
    // binary as its `PreToolUse` hook for every gated tool call, rather
    // than a separate shipped executable. Handled before anything
    // iced-related so a hook invocation never so much as touches a
    // window/GPU context.
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--claude-permission-hook") {
        if let Some(socket_path) = args.get(pos + 1) {
            claude_permission_hook::run(Path::new(socket_path));
        }
        return Ok(());
    }

    logging::init();

    let mut app = iced::application(State::default, state::update, view)
        .title("DevScribe")
        .default_font(fonts::sans(Weight::Normal))
        .subscription(state::subscription)
        .window(window::Settings {
            icon: Some(window_icon()),
            maximized: true,
            ..window::Settings::default()
        })
        .window_size((1280.0, 800.0));

    for bytes in fonts::BYTES {
        app = app.font(*bytes);
    }

    app.run()
}
