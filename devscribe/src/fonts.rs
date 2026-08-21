//! Bundled Axiom HUD typefaces (see `assets/fonts/LICENSE-OFL.txt`):
//! Oxanium (`--font-display`), Archivo (`--font-sans`), IBM Plex Mono
//! (`--font-mono`). Oxanium/Archivo are instanced from Google Fonts'
//! variable sources at the weights the mockup actually uses; IBM Plex Mono
//! ships static weights upstream.
use iced::font::{Family, Weight};
use iced::Font;

pub const DISPLAY: &str = "Oxanium";
pub const SANS: &str = "Archivo";
pub const MONO: &str = "IBM Plex Mono";

/// `--type-h4` / project-name style: Oxanium, `--fw-medium` (500).
pub const fn display(weight: Weight) -> Font {
    Font {
        family: Family::Name(DISPLAY),
        weight,
        ..Font::DEFAULT
    }
}

/// Body/UI text: Archivo, `--fw-regular` (400) unless overridden.
pub const fn sans(weight: Weight) -> Font {
    Font {
        family: Family::Name(SANS),
        weight,
        ..Font::DEFAULT
    }
}

/// Code, telemetry readouts, micro/label text: IBM Plex Mono.
pub const fn mono(weight: Weight) -> Font {
    Font {
        family: Family::Name(MONO),
        weight,
        ..Font::MONOSPACE
    }
}

/// Every embedded font file, to be registered with `Application::font` at
/// startup. Only the weights the mockup actually uses are bundled.
pub const BYTES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Oxanium-500.ttf"),
    include_bytes!("../assets/fonts/Oxanium-600.ttf"),
    include_bytes!("../assets/fonts/Oxanium-800.ttf"),
    include_bytes!("../assets/fonts/Archivo-400.ttf"),
    include_bytes!("../assets/fonts/Archivo-500.ttf"),
    include_bytes!("../assets/fonts/Archivo-600.ttf"),
    include_bytes!("../assets/fonts/Archivo-700.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Bold.ttf"),
];
