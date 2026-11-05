//! A thin overlay scrollbar, in the macOS spirit. GPUI has scrolling but draws no scrollbars, and a
//! long list with no sense of position feels broken. It can be dragged.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    canvas, point, prelude::*, px, quad, Bounds, DispatchPhase, HitboxBehavior, ListState,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollHandle, Size,
    UniformListScrollHandle,
};

use super::f;
use crate::theme;

#[derive(Clone)]
pub enum ScrollSource {
    Handle(ScrollHandle),
    Uniform(UniformListScrollHandle),
    List(ListState),
}

impl ScrollSource {
    /// Viewport height, maximum offset and current offset, all positive.
    fn metrics(&self) -> (f32, f32, f32) {
        match self {
            ScrollSource::Handle(handle) => (
                f(handle.bounds().size.height),
                f(handle.max_offset().height),
                -f(handle.offset().y),
            ),
            ScrollSource::Uniform(handle) => {
                let state = handle.0.borrow();
                let base = &state.base_handle;
                (
                    f(base.bounds().size.height),
                    f(base.max_offset().height),
                    -f(base.offset().y),
                )
            }
            ScrollSource::List(state) => (
                f(state.viewport_bounds().size.height),
                f(state.max_offset_for_scrollbar().height),
                -f(state.scroll_px_offset_for_scrollbar().y),
            ),
        }
    }

    fn set_offset(&self, offset: f32) {
        match self {
            ScrollSource::Handle(handle) => handle.set_offset(point(px(0.), px(-offset))),
            ScrollSource::Uniform(handle) => handle
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.), px(-offset))),
            ScrollSource::List(state) => {
                state.set_offset_from_scrollbar(point(px(0.), px(-offset)))
            }
        }
    }

    /// Where to draw. A plain handle knows its scroller's real bounds, so a bar for one can be placed
    /// outside the scroller. That matters inside content that scrolls with it.
    fn track(&self, fallback: Bounds<Pixels>) -> Bounds<Pixels> {
        match self {
            ScrollSource::Handle(handle) => {
                let area = handle.bounds();
                Bounds {
                    origin: point(
                        area.origin.x + area.size.width - px(TRACK_WIDTH),
                        area.origin.y,
                    ),
                    size: Size {
                        width: px(TRACK_WIDTH),
                        height: area.size.height,
                    },
                }
            }
            _ => fallback,
        }
    }

    fn drag_started(&self) {
        if let ScrollSource::List(state) = self {
            state.scrollbar_drag_started();
        }
    }

    fn drag_ended(&self) {
        if let ScrollSource::List(state) = self {
            state.scrollbar_drag_ended();
        }
    }
}

/// Holds the drag between frames. Keep one alongside the scroll handle.
#[derive(Clone, Default)]
pub struct ScrollbarState {
    /// Distance from the top of the thumb to the pointer, while dragging.
    drag: Rc<Cell<Option<f32>>>,
}

const TRACK_WIDTH: f32 = 12.0;
const THUMB_WIDTH: f32 = 5.0;
const MIN_THUMB: f32 = 28.0;
const INSET: f32 = 3.0;

fn thumb_geometry(viewport: f32, max_offset: f32, offset: f32) -> (f32, f32) {
    let track = (viewport - INSET * 2.0).max(1.0);
    let height = (viewport / (viewport + max_offset) * track).clamp(MIN_THUMB.min(track), track);
    let travel = (track - height).max(0.0);
    let top = INSET
        + if max_offset > 0.0 {
            offset / max_offset * travel
        } else {
            0.0
        };
    (top, height)
}

/// Absolutely positioned against its parent, which must be `relative`.
pub fn scrollbar(source: ScrollSource, state: ScrollbarState) -> impl IntoElement {
    let track_source = source.clone();
    canvas(
        // A hitbox, so that a sheet covering this scrollbar also covers its mouse handling.
        move |bounds, window, _cx| {
            window.insert_hitbox(track_source.track(bounds), HitboxBehavior::Normal)
        },
        move |bounds, hitbox, window, _cx| {
            let (viewport, max_offset, offset) = source.metrics();
            if max_offset <= 0.5 || viewport <= 0.0 {
                return;
            }
            let bounds = source.track(bounds);
            let (top, height) = thumb_geometry(viewport, max_offset, offset.clamp(0.0, max_offset));
            let origin_y = f(bounds.origin.y);
            let track_bounds = bounds;
            let thumb = Bounds {
                origin: point(
                    bounds.origin.x + px(TRACK_WIDTH - THUMB_WIDTH - 2.0),
                    bounds.origin.y + px(top),
                ),
                size: Size {
                    width: px(THUMB_WIDTH),
                    height: px(height),
                },
            };

            let hovered = hitbox.is_hovered(window);
            let dragging = state.drag.get().is_some();
            let color = if dragging || hovered {
                theme::surface_4().alpha(0.34)
            } else {
                theme::surface_4().alpha(0.18)
            };
            window.paint_quad(quad(
                thumb,
                px(THUMB_WIDTH / 2.0),
                color,
                px(0.),
                theme::hairline(),
                Default::default(),
            ));

            let hitbox_for_press = hitbox.clone();
            let down_state = state.clone();
            let down_source = source.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || event.button != MouseButton::Left
                    || !hitbox_for_press.is_hovered(window)
                {
                    return;
                }
                let y = f(event.position.y) - origin_y;
                let grab = if y >= top && y <= top + height {
                    y - top
                } else {
                    // Clicked the track: jump there, keeping the thumb centred on the pointer.
                    let travel = (viewport - INSET * 2.0 - height).max(1.0);
                    let target = (y - height / 2.0 - INSET) / travel * max_offset;
                    down_source.set_offset(target.clamp(0.0, max_offset));
                    height / 2.0
                };
                down_state.drag.set(Some(grab));
                down_source.drag_started();
                cx.stop_propagation();
                window.refresh();
            });

            let move_state = state.clone();
            let move_source = source.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                if let Some(grab) = move_state.drag.get() {
                    let (viewport, max_offset, _) = move_source.metrics();
                    let (_, height) = thumb_geometry(viewport, max_offset, 0.0);
                    let travel = (viewport - INSET * 2.0 - height).max(1.0);
                    let y = f(event.position.y) - origin_y - grab - INSET;
                    move_source.set_offset((y / travel * max_offset).clamp(0.0, max_offset));
                    window.refresh();
                } else if track_bounds.contains(&event.position) != hovered {
                    window.refresh();
                }
            });

            let up_state = state.clone();
            let up_source = source.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _cx| {
                if phase == DispatchPhase::Capture && up_state.drag.take().is_some() {
                    up_source.drag_ended();
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .top(px(0.))
    .right(px(0.))
    .bottom(px(0.))
    .w(px(TRACK_WIDTH))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_stays_on_the_track() {
        for (viewport, max, offset) in [
            (500.0, 10_000.0, 0.0),
            (500.0, 10_000.0, 10_000.0),
            (500.0, 10.0, 5.0),
            (20.0, 1_000_000.0, 500_000.0),
        ] {
            let (top, height) = thumb_geometry(viewport, max, offset);
            assert!(top >= INSET - 0.01);
            assert!(top + height <= viewport - INSET + 0.01, "{top} + {height}");
        }
    }

    #[test]
    fn long_lists_keep_a_grabbable_thumb() {
        let (_, height) = thumb_geometry(500.0, 10_000_000.0, 0.0);
        assert!(height >= MIN_THUMB);
    }
}
