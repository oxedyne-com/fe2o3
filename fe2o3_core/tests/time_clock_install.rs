//! The process clock is installed once.  A binary of its own, because the
//! global it sets would otherwise leak into every other test in the process.

#![cfg(not(target_arch = "wasm32"))]

use oxedyne_fe2o3_core::{
    prelude::*,
    time::{
        self,
        ClockSource,
    },
};

#[test]
fn test_second_install_errors_07() -> Outcome<()> {
    assert!(!time::clock().is_driven());
    let path = std::env::temp_dir().join(fmt!("fe2o3_time_clock_install_{}", std::process::id()));
    res!(std::fs::write(&path, 42u64.to_le_bytes()));
    let spec = fmt!("driven:{}", path.display());
    res!(time::install(res!(ClockSource::from_spec(Some(&spec)))));
    assert!(time::clock().is_driven());
    assert_eq!(res!(time::clock().now_millis()), 42);
    assert!(time::install(ClockSource::System).is_err());
    assert!(time::clock().is_driven());
    res!(std::fs::remove_file(&path));
    Ok(())
}
