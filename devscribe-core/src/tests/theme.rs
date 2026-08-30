use super::*;

/// CSS `#rrggbbaa` — the exact form the Maho token files use — as the port
/// should produce it. Alpha is the trailing byte over 255, which is where
/// hand-porting goes wrong: `--surface-hover:#1822300f` is a 6% wash, not
/// the 22% one it was first read as.
fn css(hex: u32, alpha_byte: u8) -> Rgba {
    Rgba::hex_a(hex, alpha_byte as f32 / 255.0)
}

fn approx(a: Rgba, b: Rgba, token: &str) {
    let close = |x: f32, y: f32| (x - y).abs() < 0.002;
    assert!(
        close(a.r, b.r) && close(a.g, b.g) && close(a.b, b.b) && close(a.a, b.a),
        "{token}: ported {a:?} != css {b:?}"
    );
}

/// Every token in `tokens/colors.css` that carries a non-opaque alpha. The
/// opaque ones are plain hex and hard to get wrong; these are the ones that
/// have to be read out of a packed `#rrggbbaa` literal by hand.
#[test]
fn translucent_tokens_match_the_css_they_were_ported_from() {
    let d = palette(ThemeMode::Dark, Accent::Tsuki);
    approx(d.surface_inset, css(0x0B1017, 0x80), "dark --surface-inset");
    approx(d.surface_hover, css(0x182230, 0x0f), "dark --surface-hover");
    approx(d.overlay_scrim, css(0x04060A, 0xc9), "dark --overlay-scrim");
    approx(d.border_hairline, css(0x2E3B4C, 0x99), "dark --border-hairline");

    let l = palette(ThemeMode::Light, Accent::Tsuki);
    approx(l.surface_inset, css(0xE9EDF3, 0xb3), "light --surface-inset");
    approx(l.surface_hover, css(0x0B1017, 0x0a), "light --surface-hover");
    approx(l.overlay_scrim, css(0x0B1017, 0xa6), "light --overlay-scrim");
    approx(l.border_hairline, css(0xB4C0CD, 0x80), "light --border-hairline");
}

/// `accents.css` remaps the accent-derived tokens per mode: dark reads
/// `--a-400`/`--a-300`, light reads `--a-600`/`--a-500`. Getting the pair
/// backwards is invisible in dark mode and wrong everywhere in light.
#[test]
fn accent_derived_tokens_read_the_mode_appropriate_ramp_step() {
    for accent in Accent::ALL {
        let d = palette(ThemeMode::Dark, accent);
        let r = ramp(accent);
        approx(d.accent_solid, Rgba::hex(r.a400), "dark --accent-solid");
        approx(d.accent_solid_hover, Rgba::hex(r.a300), "dark --accent-solid-hover");
        approx(d.text_accent, Rgba::hex(r.a300), "dark --text-accent");
        approx(d.border_accent, Rgba::hex(r.a500), "dark --border-accent");
        approx(d.border_focus, Rgba::hex(r.a400), "dark --border-focus");
        approx(d.accent_quiet, Rgba::hex_a(r.a400, 0.12), "dark --accent-quiet");

        let l = palette(ThemeMode::Light, accent);
        approx(l.accent_solid, Rgba::hex(r.a600), "light --accent-solid");
        approx(l.accent_solid_hover, Rgba::hex(r.a500), "light --accent-solid-hover");
        approx(l.text_accent, Rgba::hex(r.a600), "light --text-accent");
        // Light collapses both hairline and focus onto a-500.
        approx(l.border_accent, Rgba::hex(r.a500), "light --border-accent");
        approx(l.border_focus, Rgba::hex(r.a500), "light --border-focus");
        approx(l.accent_quiet, Rgba::hex_a(r.a600, 0.10), "light --accent-quiet");
    }
}

/// The scales in `spacing.css` / `radius.css` / `typography.css`, which the
/// chrome sizes itself from.
#[test]
fn scales_match_the_token_files() {
    assert_eq!(
        [SPACING.s1, SPACING.s2, SPACING.s3, SPACING.s4, SPACING.s5, SPACING.s6,
         SPACING.s7, SPACING.s8, SPACING.s9, SPACING.s10, SPACING.s11, SPACING.s12],
        [2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0, 96.0, 128.0, 192.0],
        "--space-1..12"
    );
    assert_eq!(
        [GEOMETRY.radius_xs, GEOMETRY.radius_sm, GEOMETRY.radius_md, GEOMETRY.radius_lg, GEOMETRY.radius_xl],
        [2.0, 3.0, 6.0, 10.0, 16.0],
        "--radius-xs..xl"
    );
    assert_eq!(
        [GEOMETRY.control_h_sm, GEOMETRY.control_h, GEOMETRY.control_h_lg],
        [30.0, 38.0, 46.0],
        "--control-h-sm/md/lg"
    );
    // Not a token-file value: the mockup's own shell sets the sidebar to
    // 272px (`sidebarWidth: (this.props.sidebarWidth ?? 272)`).
    assert_eq!(GEOMETRY.sidebar_w, 272.0);
    assert_eq!(
        [FONT_SIZES.micro, FONT_SIZES.body_sm, FONT_SIZES.body_md, FONT_SIZES.body_lg,
         FONT_SIZES.subheading, FONT_SIZES.heading, FONT_SIZES.title,
         FONT_SIZES.display_3, FONT_SIZES.display_2, FONT_SIZES.display_1],
        [11.0, 13.0, 15.0, 17.0, 18.0, 22.0, 28.0, 36.0, 48.0, 64.0],
        "--text-micro..display-1"
    );
    assert_eq!(
        [TRACKING.display, TRACKING.normal, TRACKING.wide, TRACKING.label],
        [-0.04, 0.0, 0.04, 0.16],
        "--tracking-display/normal/wide/label"
    );
}
