//! Safe Rust wrapper around `components/tinyusb_bridge`.
//!
//! The `ffi` module is the only place that touches `extern "C"` — everything
//! public from this file returns `Result<(), EspError>` and performs all
//! `unsafe` internally. `main.rs` should never need an `unsafe` block.
//!
//! A process-wide `Mutex` serialises every call into the bridge. TinyUSB's
//! report-submission functions are not reentrant, and we may call them from
//! multiple TCP-client threads, so we funnel everything through a single
//! critical section.

use std::sync::{Mutex, OnceLock};

use esp_idf_sys::EspError;

mod ffi {
    use esp_idf_sys::esp_err_t;

    // `tinyusb_bridge_ready` is re-exported as the safe `is_ready()` wrapper
    // below. It is useful for application code that wants to gate work on the
    // USB enumeration state, even if the current command protocol does not
    // happen to call it — keep it allowed.
    #[allow(dead_code)]
    unsafe extern "C" {
        pub fn tinyusb_bridge_init() -> esp_err_t;
        pub fn tinyusb_bridge_ready() -> bool;
        pub fn tinyusb_bridge_keyboard_press(modifier: u8, keycode: u8) -> esp_err_t;
        pub fn tinyusb_bridge_keyboard_release() -> esp_err_t;
        pub fn tinyusb_bridge_mouse_report(buttons: u8, dx: i8, dy: i8) -> esp_err_t;
        pub fn tinyusb_bridge_consumer_press(usage_code: u16) -> esp_err_t;
        pub fn tinyusb_bridge_consumer_release() -> esp_err_t;
    }
}

fn hid_mutex() -> &'static Mutex<()> {
    static HID_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    HID_MUTEX.get_or_init(|| Mutex::new(()))
}

fn with_lock<R>(f: impl FnOnce() -> R) -> R {
    // The mutex only guards a critical section around the FFI call; if another
    // thread panicked while holding it, the TinyUSB state is still consistent
    // (the C side is stateless w.r.t. this lock). Recover the guard either way.
    let guard = hid_mutex().lock().unwrap_or_else(|poison| poison.into_inner());
    let result = f();
    drop(guard);
    result
}

/// Initialise the TinyUSB driver. Call once at startup.
pub fn init() -> Result<(), EspError> {
    with_lock(|| esp_idf_sys::esp!(unsafe { ffi::tinyusb_bridge_init() }))
}

/// Whether the USB host has mounted us and HID is ready to accept reports.
#[allow(dead_code)]
pub fn is_ready() -> bool {
    unsafe { ffi::tinyusb_bridge_ready() }
}

/// Press a single keyboard key with optional modifier bitmap.
pub fn keyboard_press(modifier: u8, keycode: u8) -> Result<(), EspError> {
    with_lock(|| esp_idf_sys::esp!(unsafe { ffi::tinyusb_bridge_keyboard_press(modifier, keycode) }))
}

/// Release all keyboard keys.
pub fn keyboard_release() -> Result<(), EspError> {
    with_lock(|| esp_idf_sys::esp!(unsafe { ffi::tinyusb_bridge_keyboard_release() }))
}

/// Send a mouse report. `dx`/`dy` are relative movement in the range -127..127.
/// `buttons` bitmap: bit 0 = left, bit 1 = right, bit 2 = middle.
pub fn mouse_report(buttons: u8, dx: i8, dy: i8) -> Result<(), EspError> {
    with_lock(|| esp_idf_sys::esp!(unsafe { ffi::tinyusb_bridge_mouse_report(buttons, dx, dy) }))
}

/// Press a consumer-control (media) key by its 16-bit usage code.
pub fn consumer_press(usage_code: u16) -> Result<(), EspError> {
    with_lock(|| esp_idf_sys::esp!(unsafe { ffi::tinyusb_bridge_consumer_press(usage_code) }))
}

/// Release any pressed consumer-control key.
pub fn consumer_release() -> Result<(), EspError> {
    with_lock(|| esp_idf_sys::esp!(unsafe { ffi::tinyusb_bridge_consumer_release() }))
}

// ── TinyUSB weak-symbol callbacks implemented in Rust ────────────────────────
//
// TinyUSB declares `tud_hid_get_report_cb` and `tud_hid_set_report_cb` as weak
// C symbols and requires the application to provide them somewhere in the
// final binary. `#[no_mangle] extern "C"` makes the Rust functions visible to
// the linker at the same symbol names, so no extra C stub is needed.
//
// We do not currently support host-initiated GET/SET reports (e.g. keyboard
// LED state). Returning 0 / doing nothing is safe: the USB host treats it as
// "no data" and the bridge still functions as an output-only device.

#[no_mangle]
unsafe extern "C" fn tud_hid_get_report_cb(
    _instance: u8,
    _report_id: u8,
    _report_type: u8, // hid_report_type_t (uint8_t in TinyUSB)
    _buffer: *mut u8,
    _reqlen: u16,
) -> u16 {
    0
}

#[no_mangle]
unsafe extern "C" fn tud_hid_set_report_cb(
    _instance: u8,
    _report_id: u8,
    _report_type: u8,
    _buffer: *const u8,
    _bufsize: u16,
) {
}
