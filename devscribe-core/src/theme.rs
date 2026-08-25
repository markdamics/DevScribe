//! Maho design tokens, ported from the DevScribe.dc.html mockup's
//! `_ds/maho-design-system-*/tokens/*.css`. Kept UI-framework-agnostic
//! (plain floats, no `iced::Color`) so this crate has no windowing dependency.
//!
//! Maho's model is two orthogonal axes rather than Axiom HUD's ten fixed
//! named themes: a `ThemeMode` (dark "Yoru" / light "Asa") crossed with an
//! `Accent` hue (six of the mockup's nine defined ramps are exposed as
//! picker options — `tsuki`/`seiji`/`matcha`/`fuji`/`kohaku`/`nezu` — the
//! other three (`sakura`/`shu`/`wakaba`) exist in the source CSS but aren't
//! offered in the mockup's own settings, so aren't ported here either).

/// Straight port of CSS `rgb()`/`rgba()` — components in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// From a `#rrggbb` hex literal, e.g. `Rgba::hex(0x3c9cff)`.
    pub const fn hex(hex: u32) -> Self {
        Self::hex_a(hex, 1.0)
    }

    /// From a `#rrggbb` hex literal plus an explicit alpha, matching the
    /// CSS `rgba(r,g,b,a)` tokens (e.g. `--accent-quiet`, `--surface-inset`)
    /// and `color-mix(in srgb, X N%, transparent)`, which is just `X` at
    /// `N%` alpha.
    pub const fn hex_a(hex: u32, a: f32) -> Self {
        let r = ((hex >> 16) & 0xff) as f32 / 255.0;
        let g = ((hex >> 8) & 0xff) as f32 / 255.0;
        let b = (hex & 0xff) as f32 / 255.0;
        Self { r, g, b, a }
    }
}

/// `--bg-canvas`/`--bg-base` base — dark ("Yoru") or light ("Asa").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 2] = [ThemeMode::Dark, ThemeMode::Light];

    pub const fn label(self) -> &'static str {
        match self {
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        }
    }
}

impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One `data-accent` scope's 5-step ramp (`tokens/accents.css`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Accent {
    /// Moonlight blue. Default.
    #[default]
    Tsuki,
    /// Muted celadon green.
    Seiji,
    /// Soft tea green.
    Matcha,
    /// Wisteria violet.
    Fuji,
    /// Amber.
    Kohaku,
    /// Warm grey.
    Nezu,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Tsuki,
        Accent::Seiji,
        Accent::Matcha,
        Accent::Fuji,
        Accent::Kohaku,
        Accent::Nezu,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Accent::Tsuki => "Tsuki",
            Accent::Seiji => "Seiji",
            Accent::Matcha => "Matcha",
            Accent::Fuji => "Fuji",
            Accent::Kohaku => "Kohaku",
            Accent::Nezu => "Nezu",
        }
    }
}

impl std::fmt::Display for Accent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One accent's 5-step ramp (`--a-200`..`--a-600`), as raw hex — combined
/// with `ThemeMode` in `palette()` to derive the mode-appropriate semantic
/// accent tokens (dark reads `a400`/`a300`, light reads `a600`/`a500`, per
/// `tokens/accents.css`'s `[data-theme="light"][data-accent]` override).
struct Ramp {
    a300: u32,
    a400: u32,
    a500: u32,
    a600: u32,
}

const fn ramp(accent: Accent) -> Ramp {
    match accent {
        Accent::Tsuki => Ramp { a300: 0xB7D8F0, a400: 0x8CBFE4, a500: 0x5C9BC9, a600: 0x3E7AA6 },
        Accent::Seiji => Ramp { a300: 0xB5DBD5, a400: 0x7FBFB6, a500: 0x549C93, a600: 0x3A7A72 },
        Accent::Matcha => Ramp { a300: 0xB9D8C0, a400: 0x8CBF9B, a500: 0x5F9A72, a600: 0x427857 },
        Accent::Fuji => Ramp { a300: 0xCBC1E7, a400: 0xA99BD6, a500: 0x7F6EB4, a600: 0x5E4E92 },
        Accent::Kohaku => Ramp { a300: 0xEBD3A6, a400: 0xD8A857, a500: 0xB4842F, a600: 0x8E6520 },
        Accent::Nezu => Ramp { a300: 0xC0C8D2, a400: 0x98A5B4, a500: 0x71808F, a600: 0x53616F },
    }
}

/// One theme's full semantic color set (`colors.css`'s `:root` /
/// `[data-theme="light"]` blocks, with the accent-scope overrides from
/// `accents.css` folded in).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg_canvas: Rgba,
    pub bg_base: Rgba,

    pub surface_card: Rgba,
    pub surface_raised: Rgba,
    pub surface_sunken: Rgba,
    pub surface_inset: Rgba,
    pub surface_hover: Rgba,
    pub overlay_scrim: Rgba,

    pub text_strong: Rgba,
    pub text_body: Rgba,
    pub text_muted: Rgba,
    pub text_faint: Rgba,
    pub text_inverse: Rgba,
    pub text_accent: Rgba,
    pub text_seal: Rgba,

    pub border_hairline: Rgba,
    pub border_strong: Rgba,
    pub border_accent: Rgba,
    pub border_focus: Rgba,

    pub accent_solid: Rgba,
    pub accent_solid_hover: Rgba,
    pub accent_quiet: Rgba,
    pub accent_on: Rgba,
    pub seal_solid: Rgba,

    pub status_success: Rgba,
    pub status_warning: Rgba,
    pub status_danger: Rgba,
    pub status_info: Rgba,
}

pub const fn palette(mode: ThemeMode, accent: Accent) -> Palette {
    let r = ramp(accent);
    match mode {
        ThemeMode::Dark => Palette {
            bg_canvas: Rgba::hex(0x04060A),
            bg_base: Rgba::hex(0x070A0F),

            surface_card: Rgba::hex(0x0B1017),
            surface_raised: Rgba::hex(0x111823),
            surface_sunken: Rgba::hex(0x04060A),
            surface_inset: Rgba::hex_a(0x0B1017, 0.502),
            surface_hover: Rgba::hex_a(0x182230, 0.059),
            overlay_scrim: Rgba::hex_a(0x04060A, 0.788),

            text_strong: Rgba::hex(0xF5F7FA),
            text_body: Rgba::hex(0xDCE3EB),
            text_muted: Rgba::hex(0x8794A4),
            text_faint: Rgba::hex(0x5A6879),
            text_inverse: Rgba::hex(0x070A0F),
            text_accent: Rgba::hex(r.a300),
            text_seal: Rgba::hex(0xE4714D),

            border_hairline: Rgba::hex_a(0x2E3B4C, 0.6),
            border_strong: Rgba::hex(0x2E3B4C),
            border_accent: Rgba::hex(r.a500),
            border_focus: Rgba::hex(r.a400),

            accent_solid: Rgba::hex(r.a400),
            accent_solid_hover: Rgba::hex(r.a300),
            accent_quiet: Rgba::hex_a(r.a400, 0.12),
            accent_on: Rgba::hex(0x070A0F),
            seal_solid: Rgba::hex(0xCF5533),

            status_success: Rgba::hex(0x6E9E7F),
            status_warning: Rgba::hex(0xC4963F),
            status_danger: Rgba::hex(0xE4714D),
            status_info: Rgba::hex(r.a400),
        },
        ThemeMode::Light => Palette {
            bg_canvas: Rgba::hex(0xE9EDF3),
            bg_base: Rgba::hex(0xF5F7FA),

            surface_card: Rgba::hex(0xFFFFFF),
            surface_raised: Rgba::hex(0xFFFFFF),
            surface_sunken: Rgba::hex(0xE9EDF3),
            surface_inset: Rgba::hex_a(0xE9EDF3, 0.702),
            surface_hover: Rgba::hex_a(0x0B1017, 0.039),
            overlay_scrim: Rgba::hex_a(0x0B1017, 0.651),

            text_strong: Rgba::hex(0x070A0F),
            text_body: Rgba::hex(0x111823),
            text_muted: Rgba::hex(0x5A6879),
            text_faint: Rgba::hex(0x8794A4),
            text_inverse: Rgba::hex(0xF5F7FA),
            text_accent: Rgba::hex(r.a600),
            text_seal: Rgba::hex(0xA84124),

            border_hairline: Rgba::hex_a(0xB4C0CD, 0.502),
            border_strong: Rgba::hex(0x8794A4),
            border_accent: Rgba::hex(r.a500),
            border_focus: Rgba::hex(r.a500),

            accent_solid: Rgba::hex(r.a600),
            accent_solid_hover: Rgba::hex(r.a500),
            accent_quiet: Rgba::hex_a(r.a600, 0.10),
            accent_on: Rgba::hex(0xFFFFFF),
            seal_solid: Rgba::hex(0xCF5533),

            status_success: Rgba::hex(0x4E7D5F),
            status_warning: Rgba::hex(0x9C7326),
            status_danger: Rgba::hex(0xA84124),
            status_info: Rgba::hex(r.a600),
        },
    }
}

/// The `--text-*` size scale, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontSizes {
    pub micro: f32,
    pub body_sm: f32,
    pub body_md: f32,
    pub body_lg: f32,
    pub subheading: f32,
    pub heading: f32,
    pub title: f32,
    pub display_3: f32,
    pub display_2: f32,
    pub display_1: f32,
}

pub const FONT_SIZES: FontSizes = FontSizes {
    micro: 11.0,
    body_sm: 13.0,
    body_md: 15.0,
    body_lg: 17.0,
    subheading: 18.0,
    heading: 22.0,
    title: 28.0,
    display_3: 36.0,
    display_2: 48.0,
    display_1: 64.0,
};

/// Letter-spacing (`--tracking-*`), in em.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracking {
    pub display: f32,
    pub normal: f32,
    pub wide: f32,
    pub label: f32,
}

pub const TRACKING: Tracking = Tracking {
    display: -0.04,
    normal: 0.0,
    wide: 0.04,
    label: 0.16,
};

/// The `--space-*` scale, in logical pixels (4px base unit, widening jumps
/// at the top — `tokens/spacing.css`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    pub s1: f32,
    pub s2: f32,
    pub s3: f32,
    pub s4: f32,
    pub s5: f32,
    pub s6: f32,
    pub s7: f32,
    pub s8: f32,
    pub s9: f32,
    pub s10: f32,
    pub s11: f32,
    pub s12: f32,
}

pub const SPACING: Spacing = Spacing {
    s1: 2.0,
    s2: 4.0,
    s3: 8.0,
    s4: 12.0,
    s5: 16.0,
    s6: 24.0,
    s7: 32.0,
    s8: 48.0,
    s9: 64.0,
    s10: 96.0,
    s11: 128.0,
    s12: 192.0,
};

/// Geometry constants from `spacing.css`/`radius.css`: soft by default
/// (unlike Axiom HUD's shaved-corner sharpness), plus the control-height
/// steps `density.rs` maps its three tiers onto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub radius_xs: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub radius_xl: f32,
    pub hairline: f32,
    pub control_h_sm: f32,
    pub control_h: f32,
    pub control_h_lg: f32,
    pub sidebar_w: f32,
}

pub const GEOMETRY: Geometry = Geometry {
    radius_xs: 2.0,
    radius_sm: 3.0,
    radius_md: 6.0,
    radius_lg: 10.0,
    radius_xl: 16.0,
    hairline: 1.0,
    control_h_sm: 30.0,
    control_h: 38.0,
    control_h_lg: 46.0,
    sidebar_w: 272.0,
};

/// A full theme: mode + accent, and the palette they resolve to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub mode: ThemeMode,
    pub accent: Accent,
    pub palette: Palette,
}

impl Theme {
    pub const fn new(mode: ThemeMode, accent: Accent) -> Self {
        Self {
            mode,
            accent,
            palette: palette(mode, accent),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemeMode::default(), Accent::default())
    }
}
