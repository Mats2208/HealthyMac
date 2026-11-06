//! Motion: short transitions that say something changed, never decoration. Everything eases out
//! (a quick start and a soft landing), nothing loops, and with Reduce Motion on in macOS every
//! transition jumps straight to its end.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{div, prelude::*, Animation, AnimationExt, ElementId, Hsla, SharedString};
use hm_core::fmt::human_size;

use crate::ui::chrome::big_number;

static REDUCED: AtomicBool = AtomicBool::new(false);

/// Reads Accessibility > Display > Reduce motion once, at launch.
pub fn read_system_preference() {
    let reduced = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "1");
    REDUCED.store(reduced, Ordering::Relaxed);
}

pub fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Durations used across the app, so related things move at the same pace.
pub mod pace {
    /// Hover-sized changes: a row's details, a body swapping inside a sheet.
    pub const QUICK: u64 = 180;
    /// Something entering the screen: a page, a sheet, a bar.
    pub const ENTER: u64 = 320;
    /// Leaving is quicker than arriving, so dismissing never feels sluggish.
    pub const LEAVE: u64 = 170;
    /// Numbers counting up and bars filling.
    pub const FILL: u64 = 900;
}

/// An animation of `ms` milliseconds, or an instant one under Reduce Motion.
pub fn animation(ms: u64) -> Animation {
    let ms = if reduced() { 1 } else { ms };
    Animation::new(Duration::from_millis(ms)).with_easing(ease_out)
}

/// Quartic ease-out: most of the movement happens early, then it settles.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4)
}

/// Ease-out that overshoots a little and settles back, for things that arrive with some weight,
/// like a sheet. `t` here is linear; apply it inside an animation that uses `linear`.
pub fn ease_out_back(t: f32) -> f32 {
    const S: f32 = 1.2;
    let t = t.clamp(0.0, 1.0) - 1.0;
    1.0 + (S + 1.0) * t.powi(3) + S * t.powi(2)
}

/// A linear animation, for closures that shape `t` themselves.
pub fn linear(ms: u64) -> Animation {
    let ms = if reduced() { 1 } else { ms };
    Animation::new(Duration::from_millis(ms))
}

/// A byte count that counts up from zero to `bytes` when it first appears. `key` restarts it: pass
/// something that changes when the value means something new, such as the scan it came from.
pub fn counting_size(
    id: impl Into<SharedString>,
    key: u64,
    bytes: u64,
    size: f32,
    color: Hsla,
) -> impl IntoElement {
    div().with_animation(
        ElementId::NamedInteger(id.into(), key),
        animation(pace::FILL),
        move |_, t| {
            let shown = (bytes as f64 * t as f64).round() as u64;
            // Fades in over the first part of the count, so a bare "0 B" never flashes.
            div()
                .opacity((t * 4.0).min(1.0))
                .child(big_number(human_size(shown), size, color))
        },
    )
}
