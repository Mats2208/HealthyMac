//! What Metal says the GPU may use. Lives here so every crate reports the same number.

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

// Linking CoreGraphics is what Apple documents as necessary for command-line tools to get a default
// Metal device.
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

#[link(name = "Metal", kind = "framework")]
extern "C" {
    fn MTLCreateSystemDefaultDevice() -> *mut c_void;
}

#[link(name = "objc")]
extern "C" {
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

/// `MTLDevice.recommendedMaxWorkingSetSize`. Remembered because it can't change while the machine is
/// up with the sysctl at 0, and because creating a device isn't free.
pub fn recommended_working_set() -> Option<u64> {
    static CACHE: OnceLock<Option<u64>> = OnceLock::new();
    *CACHE.get_or_init(|| {
        type SendU64 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> u64;
        type SendVoid = unsafe extern "C" fn(*mut c_void, *mut c_void);
        // SAFETY: The selector names are NUL-terminated literals of real NSObject and MTLDevice
        // methods with exactly these signatures, and `objc_msgSend` must be called through a cast
        // to the signature of the method it dispatches to. The device is checked for null and
        // released once, as it comes from a function following the Create rule.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let device = MTLCreateSystemDefaultDevice();
            let value = if device.is_null() {
                None
            } else {
                let send_u64 = std::mem::transmute::<unsafe extern "C" fn(), SendU64>(objc_msgSend);
                let send_void =
                    std::mem::transmute::<unsafe extern "C" fn(), SendVoid>(objc_msgSend);
                let size = send_u64(
                    device,
                    sel_registerName(c"recommendedMaxWorkingSetSize".as_ptr()),
                );
                send_void(device, sel_registerName(c"release".as_ptr()));
                (size > 0).then_some(size)
            };
            objc_autoreleasePoolPop(pool);
            value
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_silicon_reports_a_plausible_share_of_ram() {
        let Some(size) = recommended_working_set() else {
            return; // No Metal device, as on some virtual machines.
        };
        let mut ram = 0u64;
        let mut length = std::mem::size_of::<u64>();
        // SAFETY: the name is NUL-terminated, and `ram` and `length` are live, correctly sized.
        let result = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut ram).cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        assert_eq!(result, 0);
        assert!(size > ram / 2 && size <= ram, "{size} of {ram}");
    }
}
