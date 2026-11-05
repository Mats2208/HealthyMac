//! Gives the Dock the app's icon however it was launched.
//!
//! macOS reads the icon from the bundle, which only helps when there is a bundle it recognizes.
//! Launched with `cargo run`, or still running after its folder has moved, the Dock falls back to a
//! generic icon. Setting it from the binary covers those cases too.

use std::ffi::{c_char, c_void};

const ICON_PNG: &[u8] = include_bytes!("../assets/AppIcon.png");

type Id = *mut c_void;

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

/// Must run on the main thread, once `NSApplication` exists.
pub fn apply() {
    type SendId = unsafe extern "C" fn(Id, *mut c_void) -> Id;
    type SendObject = unsafe extern "C" fn(Id, *mut c_void, Id) -> Id;
    type SendBytes = unsafe extern "C" fn(Id, *mut c_void, *const u8, usize) -> Id;
    type SendSetObject = unsafe extern "C" fn(Id, *mut c_void, Id);
    type SendVoid = unsafe extern "C" fn(Id, *mut c_void);
    // SAFETY: The class and selector names are NUL-terminated literals of real AppKit and Foundation
    // API with exactly these signatures, and `objc_msgSend` must be called through a cast to the
    // signature of the method it dispatches to. `ICON_PNG` outlives the call and NSData copies it.
    // The image comes from `alloc`, so it is checked for null and released once; NSApplication
    // retains what it is given.
    unsafe {
        let send_id = std::mem::transmute::<unsafe extern "C" fn(), SendId>(objc_msgSend);
        let send_object = std::mem::transmute::<unsafe extern "C" fn(), SendObject>(objc_msgSend);
        let send_bytes = std::mem::transmute::<unsafe extern "C" fn(), SendBytes>(objc_msgSend);
        let send_set = std::mem::transmute::<unsafe extern "C" fn(), SendSetObject>(objc_msgSend);
        let send_void = std::mem::transmute::<unsafe extern "C" fn(), SendVoid>(objc_msgSend);

        let pool = objc_autoreleasePoolPush();
        let data = send_bytes(
            objc_getClass(c"NSData".as_ptr()),
            sel_registerName(c"dataWithBytes:length:".as_ptr()),
            ICON_PNG.as_ptr(),
            ICON_PNG.len(),
        );
        let image = send_id(
            objc_getClass(c"NSImage".as_ptr()),
            sel_registerName(c"alloc".as_ptr()),
        );
        let image = send_object(image, sel_registerName(c"initWithData:".as_ptr()), data);
        if !image.is_null() {
            let app = send_id(
                objc_getClass(c"NSApplication".as_ptr()),
                sel_registerName(c"sharedApplication".as_ptr()),
            );
            send_set(
                app,
                sel_registerName(c"setApplicationIconImage:".as_ptr()),
                image,
            );
            send_void(image, sel_registerName(c"release".as_ptr()));
        }
        objc_autoreleasePoolPop(pool);
    }
}
