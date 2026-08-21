//! Axiom HUD design tokens, ported from the DevScribe.dc.html mockup's
//! `_ds/axiom-hud-design-system-*/tokens/*.css`. Kept UI-framework-agnostic
//! (plain floats, no `iced::Color`) so this crate has no windowing dependency.

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
    /// CSS `rgba(r,g,b,a)` tokens (e.g. `--accent-soft`).
    pub const fn hex_a(hex: u32, a: f32) -> Self {
        let r = ((hex >> 16) & 0xff) as f32 / 255.0;
        let g = ((hex >> 8) & 0xff) as f32 / 255.0;
        let b = (hex & 0xff) as f32 / 255.0;
        Self { r, g, b, a }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeName {
    /// Matte black + electric blue. Tech-minimalism. Default.
    #[default]
    NullGrid,
    /// Bone white + safety orange. Hard sci-fi / NASA-punk, light.
    Gantry,
    /// Deep navy + neon cyan / electric yellow. Cyber HUD.
    Abyssal,
    /// Near-black + hot magenta / ice. Gacha sci-fi, high drama.
    Raven,
    /// Warm carbon + amber phosphor. CRT terminal, low-light cockpit.
    Ember,
    /// Oxidized copper on deep pine. Salvage rig, long-duration life support.
    Verdigris,
    /// Cool paper + indigo. Second light theme: analysis, reports, screenshots.
    Meridian,
    /// Achromatic. No hue except danger. Maximum contrast, e-ink and projection.
    Stark,
}

impl ThemeName {
    pub const ALL: [ThemeName; 8] = [
        ThemeName::NullGrid,
        ThemeName::Gantry,
        ThemeName::Abyssal,
        ThemeName::Raven,
        ThemeName::Ember,
        ThemeName::Verdigris,
        ThemeName::Meridian,
        ThemeName::Stark,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            ThemeName::NullGrid => "NULLGRID",
            ThemeName::Gantry => "GANTRY",
            ThemeName::Abyssal => "ABYSSAL",
            ThemeName::Raven => "RAVEN",
            ThemeName::Ember => "EMBER",
            ThemeName::Verdigris => "VERDIGRIS",
            ThemeName::Meridian => "MERIDIAN",
            ThemeName::Stark => "STARK",
        }
    }
}

impl std::fmt::Display for ThemeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One theme's full semantic color set (`colors.css`'s `:root` /
/// `[data-theme="..."]` blocks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg_void: Rgba,
    pub bg_panel: Rgba,
    pub bg_panel_2: Rgba,
    pub bg_inset: Rgba,
    pub bg_scrim: Rgba,

    pub surface_glass: Rgba,
    pub surface_raised: Rgba,
    pub surface_hover: Rgba,

    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub text_on_accent: Rgba,

    pub line_hair: Rgba,
    pub line_neutral: Rgba,
    pub line_strong: Rgba,
    pub line_accent: Rgba,

    pub accent: Rgba,
    pub accent_dim: Rgba,
    pub accent_soft: Rgba,
    pub accent_glow: Rgba,
    pub accent_2: Rgba,
    pub accent_2_soft: Rgba,

    pub status_ok: Rgba,
    pub status_warn: Rgba,
    pub status_danger: Rgba,
    pub status_info: Rgba,
    pub status_idle: Rgba,
    pub status_ok_soft: Rgba,
    pub status_warn_soft: Rgba,
    pub status_danger_soft: Rgba,
    pub status_info_soft: Rgba,

    pub grid_line: Rgba,
    pub grid_dot: Rgba,
}

pub const fn palette(theme: ThemeName) -> Palette {
    match theme {
        ThemeName::NullGrid => Palette {
            bg_void: Rgba::hex(0x07090a),
            bg_panel: Rgba::hex(0x0e1214),
            bg_panel_2: Rgba::hex(0x141a1d),
            bg_inset: Rgba::hex(0x0a0e10),
            bg_scrim: Rgba::hex_a(0x040607, 0.72),

            surface_glass: Rgba::hex_a(0x141a1d, 0.62),
            surface_raised: Rgba::hex(0x1c2427),
            surface_hover: Rgba::hex_a(0x3c9cff, 0.08),

            text_primary: Rgba::hex(0xe6efec),
            text_secondary: Rgba::hex(0x9aaaa6),
            text_muted: Rgba::hex(0x62716e),
            text_on_accent: Rgba::hex(0x00101f),

            line_hair: Rgba::hex_a(0x3c9cff, 0.12),
            line_neutral: Rgba::hex_a(0xbed2cd, 0.14),
            line_strong: Rgba::hex_a(0xbed2cd, 0.30),
            line_accent: Rgba::hex_a(0x3c9cff, 0.6),

            accent: Rgba::hex(0x3c9cff),
            accent_dim: Rgba::hex(0x2a7ad4),
            accent_soft: Rgba::hex_a(0x3c9cff, 0.15),
            accent_glow: Rgba::hex_a(0x3c9cff, 0.38),
            accent_2: Rgba::hex(0xff7a1a),
            accent_2_soft: Rgba::hex_a(0xff7a1a, 0.14),

            status_ok: Rgba::hex(0x34e07a),
            status_warn: Rgba::hex(0xffb01f),
            status_danger: Rgba::hex(0xff3b30),
            status_info: Rgba::hex(0x59e3e0),
            status_idle: Rgba::hex(0x62716e),
            status_ok_soft: Rgba::hex_a(0x34e07a, 0.14),
            status_warn_soft: Rgba::hex_a(0xffb01f, 0.14),
            status_danger_soft: Rgba::hex_a(0xff3b30, 0.14),
            status_info_soft: Rgba::hex_a(0x59e3e0, 0.14),

            grid_line: Rgba::hex_a(0xbed2cd, 0.055),
            grid_dot: Rgba::hex_a(0xbed2cd, 0.13),
        },
        ThemeName::Gantry => Palette {
            bg_void: Rgba::hex(0xdfdcd4),
            bg_panel: Rgba::hex(0xf3f1ea),
            bg_panel_2: Rgba::hex(0xe9e6dd),
            bg_inset: Rgba::hex(0xd8d5cb),
            bg_scrim: Rgba::hex_a(0x3c3830, 0.42),

            surface_glass: Rgba::hex_a(0xf3f1ea, 0.72),
            surface_raised: Rgba::hex(0xffffff),
            surface_hover: Rgba::hex_a(0xe2571b, 0.07),

            text_primary: Rgba::hex(0x16181a),
            text_secondary: Rgba::hex(0x4b5158),
            text_muted: Rgba::hex(0x82867e),
            text_on_accent: Rgba::hex(0xfff6f0),

            line_hair: Rgba::hex_a(0x14181c, 0.10),
            line_neutral: Rgba::hex_a(0x14181c, 0.16),
            line_strong: Rgba::hex_a(0x14181c, 0.34),
            line_accent: Rgba::hex_a(0xe2571b, 0.65),

            accent: Rgba::hex(0xe2571b),
            accent_dim: Rgba::hex(0xb8410f),
            accent_soft: Rgba::hex_a(0xe2571b, 0.12),
            accent_glow: Rgba::hex_a(0xe2571b, 0.22),
            accent_2: Rgba::hex(0x1d6f8f),
            accent_2_soft: Rgba::hex_a(0x1d6f8f, 0.12),

            status_ok: Rgba::hex(0x1f7a4d),
            status_warn: Rgba::hex(0xb26a00),
            status_danger: Rgba::hex(0xb3251d),
            status_info: Rgba::hex(0x1d6f8f),
            status_idle: Rgba::hex(0x82867e),
            status_ok_soft: Rgba::hex_a(0x1f7a4d, 0.12),
            status_warn_soft: Rgba::hex_a(0xb26a00, 0.12),
            status_danger_soft: Rgba::hex_a(0xb3251d, 0.12),
            status_info_soft: Rgba::hex_a(0x1d6f8f, 0.12),

            grid_line: Rgba::hex_a(0x14181c, 0.06),
            grid_dot: Rgba::hex_a(0x14181c, 0.16),
        },
        ThemeName::Abyssal => Palette {
            bg_void: Rgba::hex(0x04070f),
            bg_panel: Rgba::hex(0x0a1020),
            bg_panel_2: Rgba::hex(0x101a30),
            bg_inset: Rgba::hex(0x070c18),
            bg_scrim: Rgba::hex_a(0x02050c, 0.74),

            surface_glass: Rgba::hex_a(0x101a30, 0.62),
            surface_raised: Rgba::hex(0x16223c),
            surface_hover: Rgba::hex_a(0x3ff0ff, 0.07),

            text_primary: Rgba::hex(0xe2ecff),
            text_secondary: Rgba::hex(0x93a6c6),
            text_muted: Rgba::hex(0x5b6c8a),
            text_on_accent: Rgba::hex(0x001318),

            line_hair: Rgba::hex_a(0x3ff0ff, 0.12),
            line_neutral: Rgba::hex_a(0x78b4ff, 0.14),
            line_strong: Rgba::hex_a(0x78b4ff, 0.30),
            line_accent: Rgba::hex_a(0x3ff0ff, 0.6),

            accent: Rgba::hex(0x3ff0ff),
            accent_dim: Rgba::hex(0x22b6c9),
            accent_soft: Rgba::hex_a(0x3ff0ff, 0.13),
            accent_glow: Rgba::hex_a(0x3ff0ff, 0.34),
            accent_2: Rgba::hex(0xffe14d),
            accent_2_soft: Rgba::hex_a(0xffe14d, 0.14),

            status_ok: Rgba::hex(0x37e0a8),
            status_warn: Rgba::hex(0xffe14d),
            status_danger: Rgba::hex(0xff4d6d),
            status_info: Rgba::hex(0x6fb0ff),
            status_idle: Rgba::hex(0x5b6c8a),
            status_ok_soft: Rgba::hex_a(0x37e0a8, 0.14),
            status_warn_soft: Rgba::hex_a(0xffe14d, 0.14),
            status_danger_soft: Rgba::hex_a(0xff4d6d, 0.14),
            status_info_soft: Rgba::hex_a(0x6fb0ff, 0.14),

            grid_line: Rgba::hex_a(0x78b4ff, 0.07),
            grid_dot: Rgba::hex_a(0x78b4ff, 0.16),
        },
        ThemeName::Raven => Palette {
            bg_void: Rgba::hex(0x08070a),
            bg_panel: Rgba::hex(0x100d14),
            bg_panel_2: Rgba::hex(0x17131d),
            bg_inset: Rgba::hex(0x0b0910),
            bg_scrim: Rgba::hex_a(0x060408, 0.74),

            surface_glass: Rgba::hex_a(0x17131d, 0.64),
            surface_raised: Rgba::hex(0x211b29),
            surface_hover: Rgba::hex_a(0xff2d78, 0.07),

            text_primary: Rgba::hex(0xefe9f2),
            text_secondary: Rgba::hex(0xa89fb2),
            text_muted: Rgba::hex(0x6d6577),
            text_on_accent: Rgba::hex(0x14000a),

            line_hair: Rgba::hex_a(0xff2d78, 0.12),
            line_neutral: Rgba::hex_a(0xe6d2ff, 0.12),
            line_strong: Rgba::hex_a(0xe6d2ff, 0.28),
            line_accent: Rgba::hex_a(0xff2d78, 0.6),

            accent: Rgba::hex(0xff2d78),
            accent_dim: Rgba::hex(0xc81f5c),
            accent_soft: Rgba::hex_a(0xff2d78, 0.14),
            accent_glow: Rgba::hex_a(0xff2d78, 0.34),
            accent_2: Rgba::hex(0xa9e6ff),
            accent_2_soft: Rgba::hex_a(0xa9e6ff, 0.14),

            status_ok: Rgba::hex(0x4de3a0),
            status_warn: Rgba::hex(0xffb01f),
            status_danger: Rgba::hex(0xff2d78),
            status_info: Rgba::hex(0xa9e6ff),
            status_idle: Rgba::hex(0x6d6577),
            status_ok_soft: Rgba::hex_a(0x4de3a0, 0.14),
            status_warn_soft: Rgba::hex_a(0xffb01f, 0.14),
            status_danger_soft: Rgba::hex_a(0xff2d78, 0.14),
            status_info_soft: Rgba::hex_a(0xa9e6ff, 0.14),

            grid_line: Rgba::hex_a(0xe6d2ff, 0.06),
            grid_dot: Rgba::hex_a(0xe6d2ff, 0.14),
        },
        ThemeName::Ember => Palette {
            bg_void: Rgba::hex(0x0b0806),
            bg_panel: Rgba::hex(0x14100c),
            bg_panel_2: Rgba::hex(0x1c1712),
            bg_inset: Rgba::hex(0x0f0b08),
            bg_scrim: Rgba::hex_a(0x080503, 0.74),

            surface_glass: Rgba::hex_a(0x1c1712, 0.64),
            surface_raised: Rgba::hex(0x241d16),
            surface_hover: Rgba::hex_a(0xffa02e, 0.08),

            text_primary: Rgba::hex(0xf2e6d6),
            text_secondary: Rgba::hex(0xb39c81),
            text_muted: Rgba::hex(0x7a6a58),
            text_on_accent: Rgba::hex(0x1a0d00),

            line_hair: Rgba::hex_a(0xffa02e, 0.13),
            line_neutral: Rgba::hex_a(0xe6cdaf, 0.13),
            line_strong: Rgba::hex_a(0xe6cdaf, 0.30),
            line_accent: Rgba::hex_a(0xffa02e, 0.6),

            accent: Rgba::hex(0xffa02e),
            accent_dim: Rgba::hex(0xc9761a),
            accent_soft: Rgba::hex_a(0xffa02e, 0.14),
            accent_glow: Rgba::hex_a(0xffa02e, 0.34),
            accent_2: Rgba::hex(0x7fd4c1),
            accent_2_soft: Rgba::hex_a(0x7fd4c1, 0.14),

            status_ok: Rgba::hex(0x8fd46a),
            status_warn: Rgba::hex(0xffcf40),
            status_danger: Rgba::hex(0xff5436),
            status_info: Rgba::hex(0x7fd4c1),
            status_idle: Rgba::hex(0x7a6a58),
            status_ok_soft: Rgba::hex_a(0x8fd46a, 0.14),
            status_warn_soft: Rgba::hex_a(0xffcf40, 0.14),
            status_danger_soft: Rgba::hex_a(0xff5436, 0.14),
            status_info_soft: Rgba::hex_a(0x7fd4c1, 0.14),

            grid_line: Rgba::hex_a(0xe6cdaf, 0.06),
            grid_dot: Rgba::hex_a(0xe6cdaf, 0.14),
        },
        ThemeName::Verdigris => Palette {
            bg_void: Rgba::hex(0x050b09),
            bg_panel: Rgba::hex(0x0b1512),
            bg_panel_2: Rgba::hex(0x111f1a),
            bg_inset: Rgba::hex(0x081110),
            bg_scrim: Rgba::hex_a(0x030807, 0.74),

            surface_glass: Rgba::hex_a(0x111f1a, 0.62),
            surface_raised: Rgba::hex(0x16271f),
            surface_hover: Rgba::hex_a(0x4fd6a8, 0.07),

            text_primary: Rgba::hex(0xdff0e6),
            text_secondary: Rgba::hex(0x95b3a4),
            text_muted: Rgba::hex(0x5f7a6d),
            text_on_accent: Rgba::hex(0x00160f),

            line_hair: Rgba::hex_a(0x4fd6a8, 0.13),
            line_neutral: Rgba::hex_a(0xb4dcc8, 0.13),
            line_strong: Rgba::hex_a(0xb4dcc8, 0.29),
            line_accent: Rgba::hex_a(0x4fd6a8, 0.6),

            accent: Rgba::hex(0x4fd6a8),
            accent_dim: Rgba::hex(0x2fa37c),
            accent_soft: Rgba::hex_a(0x4fd6a8, 0.13),
            accent_glow: Rgba::hex_a(0x4fd6a8, 0.32),
            accent_2: Rgba::hex(0xc96a3a),
            accent_2_soft: Rgba::hex_a(0xc96a3a, 0.16),

            status_ok: Rgba::hex(0x4fd6a8),
            status_warn: Rgba::hex(0xe8c05a),
            status_danger: Rgba::hex(0xe8604a),
            status_info: Rgba::hex(0x6fc8d8),
            status_idle: Rgba::hex(0x5f7a6d),
            status_ok_soft: Rgba::hex_a(0x4fd6a8, 0.14),
            status_warn_soft: Rgba::hex_a(0xe8c05a, 0.14),
            status_danger_soft: Rgba::hex_a(0xe8604a, 0.14),
            status_info_soft: Rgba::hex_a(0x6fc8d8, 0.14),

            grid_line: Rgba::hex_a(0xb4dcc8, 0.06),
            grid_dot: Rgba::hex_a(0xb4dcc8, 0.14),
        },
        ThemeName::Meridian => Palette {
            bg_void: Rgba::hex(0xdbe0e8),
            bg_panel: Rgba::hex(0xf2f4f8),
            bg_panel_2: Rgba::hex(0xe6eaf1),
            bg_inset: Rgba::hex(0xd3d9e3),
            bg_scrim: Rgba::hex_a(0x282e3c, 0.44),

            surface_glass: Rgba::hex_a(0xf2f4f8, 0.74),
            surface_raised: Rgba::hex(0xffffff),
            surface_hover: Rgba::hex_a(0x4a5cf0, 0.07),

            text_primary: Rgba::hex(0x12161f),
            text_secondary: Rgba::hex(0x454e60),
            text_muted: Rgba::hex(0x7b8496),
            text_on_accent: Rgba::hex(0xf2f4ff),

            line_hair: Rgba::hex_a(0x12161f, 0.10),
            line_neutral: Rgba::hex_a(0x12161f, 0.15),
            line_strong: Rgba::hex_a(0x12161f, 0.32),
            line_accent: Rgba::hex_a(0x4a5cf0, 0.6),

            accent: Rgba::hex(0x4a5cf0),
            accent_dim: Rgba::hex(0x3341c4),
            accent_soft: Rgba::hex_a(0x4a5cf0, 0.11),
            accent_glow: Rgba::hex_a(0x4a5cf0, 0.20),
            accent_2: Rgba::hex(0x8a5cf0),
            accent_2_soft: Rgba::hex_a(0x8a5cf0, 0.12),

            status_ok: Rgba::hex(0x177a55),
            status_warn: Rgba::hex(0x9a6a00),
            status_danger: Rgba::hex(0xb32438),
            status_info: Rgba::hex(0x2a5fb8),
            status_idle: Rgba::hex(0x7b8496),
            status_ok_soft: Rgba::hex_a(0x177a55, 0.11),
            status_warn_soft: Rgba::hex_a(0x9a6a00, 0.11),
            status_danger_soft: Rgba::hex_a(0xb32438, 0.11),
            status_info_soft: Rgba::hex_a(0x2a5fb8, 0.11),

            grid_line: Rgba::hex_a(0x12161f, 0.055),
            grid_dot: Rgba::hex_a(0x12161f, 0.15),
        },
        ThemeName::Stark => Palette {
            bg_void: Rgba::hex(0x000000),
            bg_panel: Rgba::hex(0x0b0b0b),
            bg_panel_2: Rgba::hex(0x141414),
            bg_inset: Rgba::hex(0x060606),
            bg_scrim: Rgba::hex_a(0x000000, 0.8),

            surface_glass: Rgba::hex_a(0x141414, 0.66),
            surface_raised: Rgba::hex(0x1c1c1c),
            surface_hover: Rgba::hex_a(0xffffff, 0.07),

            text_primary: Rgba::hex(0xffffff),
            text_secondary: Rgba::hex(0xa6a6a6),
            text_muted: Rgba::hex(0x6b6b6b),
            text_on_accent: Rgba::hex(0x000000),

            line_hair: Rgba::hex_a(0xffffff, 0.14),
            line_neutral: Rgba::hex_a(0xffffff, 0.18),
            line_strong: Rgba::hex_a(0xffffff, 0.42),
            line_accent: Rgba::hex_a(0xffffff, 0.75),

            accent: Rgba::hex(0xffffff),
            accent_dim: Rgba::hex(0xbdbdbd),
            accent_soft: Rgba::hex_a(0xffffff, 0.12),
            accent_glow: Rgba::hex_a(0xffffff, 0.26),
            accent_2: Rgba::hex(0x8c8c8c),
            accent_2_soft: Rgba::hex_a(0x8c8c8c, 0.16),

            status_ok: Rgba::hex(0xffffff),
            status_warn: Rgba::hex(0xd4d4d4),
            status_danger: Rgba::hex(0xff2b1c),
            status_info: Rgba::hex(0xa6a6a6),
            status_idle: Rgba::hex(0x6b6b6b),
            status_ok_soft: Rgba::hex_a(0xffffff, 0.13),
            status_warn_soft: Rgba::hex_a(0xd4d4d4, 0.13),
            status_danger_soft: Rgba::hex_a(0xff2b1c, 0.16),
            status_info_soft: Rgba::hex_a(0xa6a6a6, 0.13),

            grid_line: Rgba::hex_a(0xffffff, 0.06),
            grid_dot: Rgba::hex_a(0xffffff, 0.16),
        },
    }
}

/// `font-family` roles from `typography.css`. Each resolves to a bundled
/// font asset in the `devscribe` crate (see `assets/`); Google Fonts
/// `Oxanium` / `Archivo` / `IBM Plex Mono`, per the mockup's own
/// substitution note (no vendor binaries were provided for the original
/// Eurostile/DIN/JetBrains Mono references).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFamily {
    /// `--font-display`: Oxanium. Headings, logo, technical titles.
    Display,
    /// `--font-sans`: Archivo. Body/UI text.
    Sans,
    /// `--font-mono`: IBM Plex Mono. Code, telemetry readouts, labels.
    Mono,
}

/// The `--fs-*` size scale, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontSizes {
    pub micro: f32,
    pub label: f32,
    pub caption: f32,
    pub body: f32,
    pub body_lg: f32,
    pub h4: f32,
    pub h3: f32,
    pub h2: f32,
    pub h1: f32,
    pub display: f32,
}

pub const FONT_SIZES: FontSizes = FontSizes {
    micro: 10.0,
    label: 11.0,
    caption: 12.0,
    body: 14.0,
    body_lg: 16.0,
    h4: 20.0,
    h3: 26.0,
    h2: 34.0,
    h1: 46.0,
    display: 64.0,
};

/// Letter-spacing (`--tr-*`), in em.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracking {
    pub micro: f32,
    pub label: f32,
    pub caps: f32,
    pub normal: f32,
    pub tight: f32,
    pub display: f32,
}

pub const TRACKING: Tracking = Tracking {
    micro: 0.22,
    label: 0.12,
    caps: 0.06,
    normal: 0.0,
    tight: -0.01,
    display: -0.02,
};

/// The `--s-*` / `--u` spacing scale, in logical pixels (4px base unit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    pub s1: f32,
    pub s2: f32,
    pub s3: f32,
    pub s4: f32,
    pub s5: f32,
    pub s6: f32,
    pub s8: f32,
    pub s10: f32,
    pub s12: f32,
    pub s16: f32,
    pub s20: f32,
    pub s24: f32,
}

pub const SPACING: Spacing = Spacing {
    s1: 4.0,
    s2: 8.0,
    s3: 12.0,
    s4: 16.0,
    s5: 20.0,
    s6: 24.0,
    s8: 32.0,
    s10: 40.0,
    s12: 48.0,
    s16: 64.0,
    s20: 80.0,
    s24: 96.0,
};

/// Geometry constants from `spacing.css` / `effects.css`: sharp by
/// default, with radii existing only to soften 1px hairlines, plus the
/// diagonal shaved-corner depths used by the `--clip-cut-*` silhouettes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub radius_1: f32,
    pub radius_2: f32,
    pub cut_sm: f32,
    pub cut_md: f32,
    pub cut_lg: f32,
    pub hairline: f32,
    pub rule: f32,
    pub bar: f32,
    pub control_h_sm: f32,
    pub control_h: f32,
    pub control_h_lg: f32,
    pub rail_w: f32,
    pub sidebar_w: f32,
}

pub const GEOMETRY: Geometry = Geometry {
    radius_1: 2.0,
    radius_2: 3.0,
    cut_sm: 6.0,
    cut_md: 10.0,
    cut_lg: 16.0,
    hairline: 1.0,
    rule: 2.0,
    bar: 3.0,
    control_h_sm: 24.0,
    control_h: 32.0,
    control_h_lg: 40.0,
    rail_w: 64.0,
    sidebar_w: 248.0,
};

/// Which corner(s) of a container get the signature shaved-corner cut,
/// matching the mockup's `--clip-cut-tr` / `--clip-cut-bl` / `--clip-cut-diag`
/// / `--clip-cut-tag` polygons. Consumed by the `devscribe` crate's canvas
/// code to build the actual clip path at a given container size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CornerCut {
    /// Top-right corner only, depth `cut_md`.
    TopRight,
    /// Bottom-left corner only, depth `cut_md`.
    BottomLeft,
    /// Top-right and bottom-left, depth `cut_md` — the chat window / card frame.
    Diagonal,
    /// Top-left and bottom-right, depth `cut_sm` — the small logo "tag" glyph.
    Tag,
}

/// A full theme: name, semantic palette, and the (currently theme-invariant)
/// typography/spacing/geometry scales bundled for convenience.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub name: ThemeName,
    pub palette: Palette,
}

impl Theme {
    pub const fn new(name: ThemeName) -> Self {
        Self {
            name,
            palette: palette(name),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ThemeName::default())
    }
}
