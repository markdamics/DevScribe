//! The "UI font size" setting: scales every chrome text size (sidebar, tabs,
//! status bar, title bar, palette, settings, toasts — everything except the
//! code editor canvas, which has its own independent `font_size`).
//!
//! Implemented as a render-scoped global rather than a `scale: f32`
//! parameter threaded through the ~15 view-building functions across the UI
//! layer: `shell::view` sets it once at the very start of each frame, and
//! every `.size(N)` call site reads it back via `px(N)`. This is safe
//! because iced calls `view()` synchronously on a single thread with no
//! reentrancy — there's exactly one write before any read, every frame.
use std::sync::atomic::{AtomicU32, Ordering};

static SCALE_BITS: AtomicU32 = AtomicU32::new(0);

/// Sets the scale for the frame about to be built. Call once, before
/// building any widgets — see `shell::view`.
pub fn set(scale: f32) {
    SCALE_BITS.store(scale.to_bits(), Ordering::Relaxed);
}

/// Scales `base` by the current UI font scale (1.0 if `set` was never
/// called, e.g. in a test).
pub fn px(base: f32) -> f32 {
    let bits = SCALE_BITS.load(Ordering::Relaxed);
    let scale = if bits == 0 { 1.0 } else { f32::from_bits(bits) };
    base * scale
}
