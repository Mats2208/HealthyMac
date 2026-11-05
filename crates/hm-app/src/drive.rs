//! Debug builds only: drive the UI from a text file, for automated visual testing.
//!
//! GPUI draws everything itself, so a running window has no accessibility tree to click through. With
//! `HEALTHYMAC_DRIVE=<file>`, each line appended to the file becomes real input. Mouse lines build a
//! genuine `NSEvent` and hand it to the window's content view, so they take the same path as a real
//! click, without moving the pointer or taking the screen from whoever is using the Mac.
//!
//! ```text
//! click 100 200
//! double 100 200
//! move 100 200
//! drag 100 200 100 400
//! scroll 100 200 -300
//! key cmd-2
//! resize 980 640
//! ```

use std::ffi::{c_void, CString};
use std::path::PathBuf;
use std::time::Duration;

use gpui::{px, size, Context, Keystroke, Window};

use crate::state::Shell;
use crate::ui::f;

type Id = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct NsPoint {
    x: f64,
    y: f64,
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const std::ffi::c_char) -> Id;
    fn sel_registerName(name: *const std::ffi::c_char) -> Id;
    fn objc_msgSend();
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateScrollWheelEvent(source: Id, units: u32, wheels: u32, delta: i32, ...) -> Id;
    fn CGEventSetLocation(event: Id, location: NsPoint);
    fn CFRelease(object: Id);
}

fn sel(name: &str) -> Id {
    let name = CString::new(name).expect("no NUL in selector");
    unsafe { sel_registerName(name.as_ptr()) }
}

/// `[receiver selector]`, returning an object.
unsafe fn send0(receiver: Id, selector: &str) -> Id {
    let call: extern "C" fn(Id, Id) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    call(receiver, sel(selector))
}

unsafe fn send_id(receiver: Id, selector: &str, argument: Id) {
    let call: extern "C" fn(Id, Id, Id) =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    call(receiver, sel(selector), argument);
}

/// GPUI's own view: the subview of the first visible window's content view that paints.
unsafe fn content_view() -> Option<Id> {
    unsafe {
        let class_name = CString::new("NSApplication").unwrap();
        let app = send0(objc_getClass(class_name.as_ptr()), "sharedApplication");
        let windows = send0(app, "windows");
        let count_fn: extern "C" fn(Id, Id) -> usize =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let count = count_fn(windows, sel("count"));
        let at_fn: extern "C" fn(Id, Id, usize) -> Id =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let visible_fn: extern "C" fn(Id, Id) -> bool =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        for index in 0..count {
            let window = at_fn(windows, sel("objectAtIndex:"), index);
            if visible_fn(window, sel("isVisible")) {
                let subviews = send0(send0(window, "contentView"), "subviews");
                let responds: extern "C" fn(Id, Id, Id) -> bool =
                    std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
                for position in 0..count_fn(subviews, sel("count")) {
                    let view = at_fn(subviews, sel("objectAtIndex:"), position);
                    if responds(view, sel("respondsToSelector:"), sel("displayLayer:")) {
                        return Some(view);
                    }
                }
            }
        }
        None
    }
}

unsafe fn mouse_event(
    kind: u64,
    x: f64,
    y_from_top: f64,
    height: f64,
    clicks: i64,
    pressure: f32,
) -> Id {
    unsafe {
        let class_name = CString::new("NSEvent").unwrap();
        let call: extern "C" fn(Id, Id, u64, NsPoint, u64, f64, i64, Id, i64, i64, f32) -> Id =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        call(
            objc_getClass(class_name.as_ptr()),
            sel("mouseEventWithType:location:modifierFlags:timestamp:windowNumber:context:eventNumber:clickCount:pressure:"),
            kind,
            NsPoint { x, y: height - y_from_top },
            0,
            0.0,
            0,
            std::ptr::null_mut(),
            0,
            clicks,
            pressure,
        )
    }
}

/// Long enough for at least one frame to be painted.
const FRAME: Duration = Duration::from_millis(40);

const LEFT_DOWN: u64 = 1;
const LEFT_UP: u64 = 2;
const MOVED: u64 = 5;
const DRAGGED: u64 = 6;

/// Delivers one mouse event. Must not run inside an app update, because the view reenters the app.
fn mouse(kind: u64, x: f64, y: f64, height: f64, clicks: i64) {
    let selector = match kind {
        LEFT_DOWN => "mouseDown:",
        LEFT_UP => "mouseUp:",
        DRAGGED => "mouseDragged:",
        _ => "mouseMoved:",
    };
    unsafe {
        if let Some(view) = content_view() {
            let pressure = if kind == LEFT_DOWN || kind == DRAGGED {
                1.0
            } else {
                0.0
            };
            let event = mouse_event(kind, x, y, height, clicks, pressure);
            send_id(view, selector, event);
        }
    }
}

fn scroll(x: f64, y: f64, height: f64, delta: i32) {
    unsafe {
        let Some(view) = content_view() else {
            return;
        };
        // A wheel event with no window reports its location in screen coordinates, flipped from the
        // CGEvent's. Choosing the location so it comes out as window coordinates is enough here.
        let units_pixel = 0;
        let event = CGEventCreateScrollWheelEvent(std::ptr::null_mut(), units_pixel, 1, delta);
        if event.is_null() {
            return;
        }
        let class_name = CString::new("NSScreen").unwrap();
        let screens = send0(objc_getClass(class_name.as_ptr()), "screens");
        let at_fn: extern "C" fn(Id, Id, usize) -> Id =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let main_screen = at_fn(screens, sel("objectAtIndex:"), 0);
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Rect {
            origin: NsPoint,
            size: NsPoint,
        }
        let frame_fn: extern "C" fn(Id, Id) -> Rect =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let screen_height = frame_fn(main_screen, sel("frame")).size.y;
        CGEventSetLocation(
            event,
            NsPoint {
                x,
                y: screen_height - (height - y),
            },
        );
        let class_name = CString::new("NSEvent").unwrap();
        let wrap: extern "C" fn(Id, Id, Id) -> Id =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let ns_event = wrap(
            objc_getClass(class_name.as_ptr()),
            sel("eventWithCGEvent:"),
            event,
        );
        send_id(view, "scrollWheel:", ns_event);
        CFRelease(event);
    }
}

/// macOS stops a window's display link while another window fully covers it, and then nothing would
/// repaint. The display layer callback is what the link would have called, so call it ourselves.
fn pump_frame() {
    unsafe {
        if let Some(view) = content_view() {
            send_id(view, "displayLayer:", std::ptr::null_mut());
        }
    }
}

pub fn start(window: &mut Window, cx: &mut Context<Shell>) {
    let Some(path) = std::env::var_os("HEALTHYMAC_DRIVE").map(PathBuf::from) else {
        return;
    };
    cx.spawn_in(window, async move |_, cx| loop {
        cx.background_executor()
            .timer(Duration::from_millis(33))
            .await;
        pump_frame();
    })
    .detach();
    cx.spawn_in(window, async move |_, cx| {
        let mut offset = 0;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if text.len() <= offset {
                continue;
            }
            let fresh = text[offset..].to_string();
            offset = text.len();
            for line in fresh.lines() {
                let Ok(height) = cx.update(|window, _| f(window.viewport_size().height) as f64)
                else {
                    return;
                };
                let parts: Vec<String> = line.split_whitespace().map(str::to_string).collect();
                let numbers: Vec<f64> = parts
                    .iter()
                    .skip(1)
                    .filter_map(|part| part.parse().ok())
                    .collect();
                match (parts.first().map(String::as_str), numbers.as_slice()) {
                    (Some("move"), [x, y]) => mouse(MOVED, *x, *y, height, 0),
                    (Some("click"), [x, y]) => {
                        mouse(MOVED, *x, *y, height, 0);
                        // GPUI only arms a click's mouse-up during the frame after the mouse-down.
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_DOWN, *x, *y, height, 1);
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_UP, *x, *y, height, 1);
                    }
                    (Some("double"), [x, y]) => {
                        mouse(MOVED, *x, *y, height, 0);
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_DOWN, *x, *y, height, 1);
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_UP, *x, *y, height, 1);
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_DOWN, *x, *y, height, 2);
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_UP, *x, *y, height, 2);
                    }
                    (Some("drag"), [x1, y1, x2, y2]) => {
                        mouse(MOVED, *x1, *y1, height, 0);
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_DOWN, *x1, *y1, height, 1);
                        cx.background_executor().timer(FRAME).await;
                        mouse(DRAGGED, (*x1 + *x2) / 2.0, (*y1 + *y2) / 2.0, height, 1);
                        cx.background_executor().timer(FRAME).await;
                        mouse(DRAGGED, *x2, *y2, height, 1);
                        cx.background_executor().timer(FRAME).await;
                        mouse(LEFT_UP, *x2, *y2, height, 1);
                    }
                    (Some("scroll"), [x, y, dy]) => {
                        mouse(MOVED, *x, *y, height, 0);
                        scroll(*x, *y, height, *dy as i32);
                    }
                    (Some("resize"), [w, h]) => {
                        let (w, h) = (*w as f32, *h as f32);
                        let _ = cx.update(|window, _| window.resize(size(px(w), px(h))));
                    }
                    (Some("key"), _) => {
                        if let Some(Ok(keystroke)) = parts.get(1).map(|key| Keystroke::parse(key)) {
                            let _ =
                                cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx));
                        }
                    }
                    _ => {}
                }
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
            }
        }
    })
    .detach();
}
