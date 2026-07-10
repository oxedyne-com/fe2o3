//! Runtime shims for the `wasm32` target.
//!
//! A browser has no OS monotonic clock, no spawnable threads and no console
//! other than the JavaScript one.  Calls such as `Instant::now`,
//! `SystemTime::now` and `thread::spawn` compile for `wasm32-unknown-unknown`
//! but panic at runtime.  These helpers give `fe2o3_core`'s clock and logger a
//! panic-free path in the browser whilst the native code is left untouched.

use wasm_bindgen::JsValue;

/// Returns the current wall-clock time in milliseconds since the Unix epoch,
/// using the JavaScript `Date.now()` primitive.  It never panics and is
/// available on both the main thread and web workers.
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Emits a single line to the browser console via `console.log`.
pub fn console_log(s: &str) {
    web_sys::console::log_1(&JsValue::from_str(s));
}
