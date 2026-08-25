//! Bundled Maho typeface (see `assets/fonts/LICENSE-OFL.txt`): JetBrains
//! Mono, instanced from Google Fonts' variable source at the weights the
//! mockup actually uses. Maho is a single-voice design system —
//! `--font-display`/`--font-sans`/`--font-mono` are all aliases for the same
//! family — so `display()`/`sans()`/`mono()` stay as three named roles
//! purely so call sites keep saying which *role* a piece of text plays, even
//! though they now all resolve to the same font.
use iced::font::{Family, Weight};
use iced::Font;

pub const DISPLAY: &str = "JetBrains Mono";
pub const SANS: &str = "JetBrains Mono";
pub const MONO: &str = "JetBrains Mono";

/// Headings, logo, technical titles.
pub const fn display(weight: Weight) -> Font {
    Font {
        family: Family::Name(DISPLAY),
        weight,
        ..Font::DEFAULT
    }
}

/// Body/UI text.
pub const fn sans(weight: Weight) -> Font {
    Font {
        family: Family::Name(SANS),
        weight,
        ..Font::DEFAULT
    }
}

/// Code, telemetry readouts, micro/label text.
pub const fn mono(weight: Weight) -> Font {
    Font {
        family: Family::Name(MONO),
        weight,
        ..Font::MONOSPACE
    }
}

/// Every embedded font file, to be registered with `Application::font` at
/// startup. Only the weights the app actually uses are bundled (no italic —
/// nothing in the UI renders it).
pub const BYTES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/JetBrainsMono-300.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-400.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-500.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-600.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-700.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-800.ttf"),
];
