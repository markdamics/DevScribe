use devscribe_core::theme::Rgba;
use iced::Color;

/// Bridges `devscribe-core`'s UI-framework-agnostic `Rgba` tokens into `iced::Color`.
pub fn color(c: Rgba) -> Color {
    Color::from_rgba(c.r, c.g, c.b, c.a)
}

/// Blends `top` over `base` by `amount` (0.0..=1.0), approximating CSS
/// `color-mix(in srgb, top <amount*100>%, base)` as used throughout the
/// mockup for tinted badge/status backgrounds.
#[allow(dead_code)]
pub fn mix(top: Rgba, amount: f32, base: Rgba) -> Color {
    let a = amount.clamp(0.0, 1.0);
    Color::from_rgba(
        top.r * a + base.r * (1.0 - a),
        top.g * a + base.g * (1.0 - a),
        top.b * a + base.b * (1.0 - a),
        top.a * a + base.a * (1.0 - a),
    )
}
