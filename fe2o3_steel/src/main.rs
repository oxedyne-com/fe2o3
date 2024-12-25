// TODO:
// 
// 1. HTTPS/TLS Configuration:
// - Currently only has basic cert file paths
// - Could add:
//   - Custom cipher suite selection
//   - TLS version requirements (min/max)
//   - HSTS settings
//   - Certificate reload/rotation settings
//   - Client certificate validation options
// 
// 2. CORS (Cross-Origin Resource Sharing):
// - Currently no CORS controls
// - Should configure:
//   - Allowed origins (domains that can access)
//   - Allowed methods (GET, POST etc)
//   - Allowed headers
//   - Whether to allow credentials
//   - Max age of preflight responses
// 
// 3. Rate Limiting:
// - No protection against abuse
// - Could add:
//   - Requests per second/minute per IP
//   - Burst allowances
//   - Custom rate limits per route
//   - Rate limit response headers
//   - Different strategies (token bucket, fixed window)
// 
// 4. Request Size Limits:
// - Currently unlimited
// - Should have:
//   - Max body size
//   - Max header size
//   - Max URI length
//   - Max number of headers
//   - Different limits for different routes/content types
// 
// 5. Compression:
// - No response compression
// - Could add:
//   - gzip/deflate/brotli support
//   - Compression level configuration
//   - Size threshold for compression
//   - MIME type based compression rules
//   - Client capability detection
// 
#![forbid(unsafe_code)]
pub mod app;
pub mod srv;

use crate::{
    app::tui,
};

use oxedize_fe2o3_core::{
    prelude::*,
    //log::{
    //    console::StdoutLoggerConsole,
    //},
};


fn main() -> Outcome<()> {
    
    let mut log_cfg = get_log_config!();
    log_cfg.file = None;
    set_log_config!(log_cfg);
    set_log_level!("debug");

    let outcome = tui::run();

    std::thread::sleep(std::time::Duration::from_secs(1));

    log_finish_wait!();

    outcome
}
