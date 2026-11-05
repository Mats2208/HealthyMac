//! Design tokens: colour, type scale, spacing and radii. Nothing else defines a colour.
//!
//! The look: a deep blue-tinted black where the sidebar is the window itself and the page floats
//! above it as one panel. Elevation comes from faint white overlays and hairlines, not boxes. Each
//! area has one accent, used sparingly.

use gpui::{
    linear_color_stop, linear_gradient, point, px, rgb, Background, BoxShadow, Hsla, Pixels,
};
use hm_core::{Area, Risk};

pub const SIDEBAR_WIDTH: f32 = 216.0;
pub const MIN_WINDOW_W: f32 = 980.0;
pub const MIN_WINDOW_H: f32 = 640.0;
/// How far from the top the page's content starts, clearing the drag region.
pub const TITLEBAR_H: f32 = 44.0;
/// Page gutter.
pub const GUTTER: f32 = 28.0;

/// 4 pt spacing grid: `sp(3)` is 12 px.
pub fn sp(steps: f32) -> Pixels {
    px(steps * 4.0)
}

// ---- Type scale --------------------------------------------------------------------------------

pub mod font {
    pub const DISPLAY: f32 = 40.0;
    pub const TITLE: f32 = 26.0;
    pub const HEADLINE: f32 = 18.0;
    pub const SUBHEAD: f32 = 15.0;
    pub const BODY: f32 = 13.0;
    pub const CAPTION: f32 = 12.0;
    pub const MICRO: f32 = 11.0;
}

// ---- Radii -------------------------------------------------------------------------------------

pub mod radius {
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
}

// ---- Colour ------------------------------------------------------------------------------------

pub fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn hexa(value: u32, alpha: f32) -> Hsla {
    hex(value).alpha(alpha)
}

fn white(alpha: f32) -> Hsla {
    hexa(0xFFFFFF, alpha)
}

/// The window itself, and the sidebar that runs under the titlebar.
pub fn window_bg() -> Hsla {
    hex(0x08090E)
}
/// The floating page.
pub fn panel_bg() -> Hsla {
    hex(0x10121A)
}
pub fn sheet_bg() -> Hsla {
    hex(0x181A24)
}
pub fn scrim() -> Hsla {
    hexa(0x04050A, 0.62)
}

pub fn surface_1() -> Hsla {
    white(0.028)
}
pub fn surface_2() -> Hsla {
    white(0.055)
}
pub fn surface_3() -> Hsla {
    white(0.085)
}
pub fn surface_4() -> Hsla {
    white(0.12)
}
pub fn hairline() -> Hsla {
    white(0.07)
}
pub fn hairline_strong() -> Hsla {
    white(0.13)
}

pub fn text_1() -> Hsla {
    hex(0xF3F4F8)
}
pub fn text_2() -> Hsla {
    hex(0xA8ACBA)
}
pub fn text_3() -> Hsla {
    hex(0x7C808F)
}
/// Text on the bright gradient buttons.
pub fn text_on_accent() -> Hsla {
    hex(0x07090E)
}

pub fn teal() -> Hsla {
    hex(0x2DD4BF)
}
pub fn violet() -> Hsla {
    hex(0xA78BFA)
}
pub fn amber() -> Hsla {
    hex(0xFBBF24)
}
pub fn rose() -> Hsla {
    hex(0xFB7185)
}
pub fn blue() -> Hsla {
    hex(0x60A5FA)
}
pub fn lime() -> Hsla {
    hex(0xA3E635)
}
pub fn green() -> Hsla {
    hex(0x34D399)
}
pub fn orange() -> Hsla {
    hex(0xFB923C)
}

pub fn area_color(area: Area) -> Hsla {
    match area {
        Area::System => teal(),
        Area::Developer => violet(),
        Area::LocalAi => hex(0xFB9A5B),
        Area::Apps => lime(),
        Area::Files => blue(),
    }
}

/// The two ends of an area's gradient. Used for primary actions on that area's pages.
pub fn area_stops(area: Area) -> (Hsla, Hsla) {
    match area {
        Area::System => (hex(0x5EEAD4), hex(0x22D3EE)),
        Area::Developer => (hex(0xC4B5FD), hex(0x8B7CF8)),
        Area::LocalAi => (amber(), rose()),
        Area::Apps => (hex(0xD9F99D), hex(0x84CC16)),
        Area::Files => (hex(0x93C5FD), hex(0x4F8EF7)),
    }
}

pub fn gradient(stops: (Hsla, Hsla)) -> Background {
    linear_gradient(
        100.,
        linear_color_stop(stops.0, 0.),
        linear_color_stop(stops.1, 1.),
    )
}

/// The brand's own gradient, for primary actions that belong to no single area.
pub fn brand_stops() -> (Hsla, Hsla) {
    (hex(0x5EEAD4), hex(0x60A5FA))
}

/// Brightens both stops, for a hovered gradient button.
pub fn lift(stops: (Hsla, Hsla)) -> (Hsla, Hsla) {
    let up = |color: Hsla| Hsla {
        l: (color.l + 0.06).min(1.0),
        ..color
    };
    (up(stops.0), up(stops.1))
}

pub fn risk_color(risk: Risk) -> Hsla {
    match risk {
        Risk::Safe => green(),
        Risk::Moderate => amber(),
        Risk::Review => rose(),
    }
}

/// Interpolates in HSL along the shortest way round the hue circle.
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let mut delta = b.h - a.h;
    if delta > 0.5 {
        delta -= 1.0;
    } else if delta < -0.5 {
        delta += 1.0;
    }
    Hsla {
        h: (a.h + delta * t).rem_euclid(1.0),
        s: a.s + (b.s - a.s) * t,
        l: a.l + (b.l - a.l) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

// ---- Elevation ---------------------------------------------------------------------------------

pub fn sheet_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: hexa(0x000000, 0.55),
            offset: point(px(0.), px(24.)),
            blur_radius: px(64.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: hexa(0x000000, 0.35),
            offset: point(px(0.), px(2.)),
            blur_radius: px(8.),
            spread_radius: px(0.),
        },
    ]
}

/// A soft coloured halo, for the one thing on the page that deserves attention.
pub fn glow(color: Hsla, blur: f32, alpha: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: color.alpha(alpha),
        offset: point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    }]
}
