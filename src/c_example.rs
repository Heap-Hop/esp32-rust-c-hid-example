//! Safe Rust wrapper around the `c_example` C component.
//!
//! Pattern for every C component in this project:
//!   1. `mod ffi` — raw `unsafe extern "C"` declarations, private to this module.
//!   2. Public `fn` wrappers — hide `unsafe`, convert `esp_err_t` to `Result`.
//!
//! Type mapping reference (C → Rust):
//!   esp_err_t        → Result<(), EspError>  (via esp_idf_sys::esp! macro)
//!   int32_t / i32    → i32
//!   uint8_t / u8     → u8
//!   bool             → bool
//!   const char*      → *const core::ffi::c_char  (wrap with CStr to read)
//!   void*            → *mut core::ffi::c_void

use esp_idf_sys::EspError;

// ── Raw FFI declarations ───────────────────────────────────────────────────────
// Keep this block private. Callers use the safe wrappers below instead.
mod ffi {
    use esp_idf_sys::esp_err_t;

    unsafe extern "C" {
        pub fn c_example_init() -> esp_err_t;
        pub fn c_example_add(a: i32, b: i32) -> i32;
    }
}

// ── Safe public API ────────────────────────────────────────────────────────────

/// Initialize the component. Call once at startup before any other function.
///
/// Returns `Err` if the underlying C call returns a non-zero `esp_err_t`.
pub fn init() -> Result<(), EspError> {
    esp_idf_sys::esp!(unsafe { ffi::c_example_init() })
}

/// Returns `a + b`.
///
/// Replace with logic that genuinely belongs in C: driver calls, DMA setup, etc.
pub fn add(a: i32, b: i32) -> i32 {
    unsafe { ffi::c_example_add(a, b) }
}
