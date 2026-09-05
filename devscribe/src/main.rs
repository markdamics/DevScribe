mod claude_permission_hook;
mod color;
mod density;
mod fonts;
mod fs_tree;
mod fuzzy;
mod logging;
mod recent_projects;
mod server_install;
mod session;
mod settings;
mod snippet;
mod state;
mod text_scale;
mod ui;
mod widgets;

use std::path::Path;

use iced::font::Weight;
use iced::window;
use iced::{Element, Task};
use state::{Message, State};

const ICON_RGBA: &[u8] = include_bytes!("../assets/icons/devscribe-icon-64.rgba");
const ICON_SIZE: u32 = 64;

/// Opens the main window itself, since a `Daemon` (unlike the old
/// single-window `Application`) doesn't open one automatically — see
/// `state::State::main_window_id`'s own doc comment for why `update` needs
/// to know this id, and `main`'s own comment for why this is a `Daemon` at
/// all rather than `Application`.
fn boot() -> (State, Task<Message>) {
    let mut state = State::default();
    let (id, opened) = window::open(window::Settings {
        icon: Some(window_icon()),
        maximized: true,
        size: (1280.0, 800.0).into(),
        ..window::Settings::default()
    });
    state.main_window_id = Some(id);
    (state, opened.map(|_| Message::Noop))
}

fn view(state: &State, window: window::Id) -> Element<'_, Message> {
    ui::shell::view(state, window)
}

fn title(state: &State, window: window::Id) -> String {
    match state.solo_windows.get(&window) {
        Some(solo) => solo.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "DevScribe".into()),
        None => "DevScribe".into(),
    }
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

    // `daemon` rather than `application`: `Message::OpenInNewWindow` needs a
    // second, independently-titled OS window (see `ui::solo_window`), and
    // `application`'s `view`/`title` builders only ever take `&State` — no
    // `window::Id` — so there's no way to tell which window is which. `boot`
    // opens the main window itself, and `Message::WindowClosed` calls
    // `iced::exit()` when it goes away, since a `Daemon` otherwise keeps
    // running once every window has closed.
    let mut app = iced::daemon(boot, state::update, view)
        .title(title)
        .default_font(fonts::sans(Weight::Normal))
        .subscription(state::subscription);

    for bytes in fonts::BYTES {
        app = app.font(*bytes);
    }

    app.run()
}
