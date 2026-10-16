//! `ClockSource`: a driven clock read back, torn values, clamping, a short
//! file, strictly increasing nanoseconds and the system clock.  Each test holds
//! a local source, so the process-wide `install` is left alone here; its
//! once-only rule is tested in `time_clock_install.rs`, a binary of its own.

#![cfg(not(target_arch = "wasm32"))]

use oxedyne_fe2o3_core::{
    prelude::*,
    time::ClockSource,
};

use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    thread,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

fn scratch(name: &str) -> Outcome<PathBuf> {
    let dir = std::env::temp_dir().join(fmt!("fe2o3_time_clock_{}_{}", std::process::id(), name));
    res!(fs::create_dir_all(&dir));
    Ok(dir)
}

// The writer's way: a whole value into a temporary file, then a rename.
fn put(path: &PathBuf, ms: u64) -> Outcome<()> {
    let tmp = path.with_extension("tmp");
    res!(fs::write(&tmp, ms.to_le_bytes()));
    res!(fs::rename(&tmp, path));
    Ok(())
}

fn driven(path: &PathBuf) -> Outcome<ClockSource> {
    let spec = fmt!("driven:{}", path.display());
    ClockSource::from_spec(Some(&spec))
}

#[test]
fn test_driven_value_read_back_01() -> Outcome<()> {
    let dir = res!(scratch("read_back"));
    let path = dir.join("clock");
    res!(put(&path, 1_700_000_123_456));
    let src = res!(driven(&path));
    assert!(src.is_driven());
    assert_eq!(res!(src.now_millis()), 1_700_000_123_456);
    assert_eq!(res!(src.now_secs()), 1_700_000_123);
    res!(fs::remove_dir_all(&dir));
    Ok(())
}

#[test]
fn test_renamed_value_never_torn_02() -> Outcome<()> {
    let dir = res!(scratch("torn"));
    let path = dir.join("clock");
    // Every written value is a multiple of this, so a torn read would show.
    const BASE: u64 = 1_700_000_000_000;
    const STEP: u64 = 0x0101_0101;
    res!(put(&path, BASE));
    let done = Arc::new(AtomicBool::new(false));
    let done_w = done.clone();
    let path_w = path.clone();
    let writer = thread::spawn(move || -> Outcome<()> {
        for i in 1..=10_000u64 {
            res!(put(&path_w, BASE + i * STEP));
        }
        done_w.store(true, Ordering::SeqCst);
        Ok(())
    });
    let src = res!(driven(&path));
    let mut reads = 0u64;
    while !done.load(Ordering::SeqCst) {
        let ms = res!(src.now_millis());
        let off = ms - BASE;
        assert!(off % STEP == 0 && off / STEP <= 10_000, "value {} is not one that was written", ms);
        reads += 1;
    }
    match writer.join() {
        Ok(r)   => res!(r),
        Err(_)  => return Err(err!("Writer thread panicked."; Thread)),
    }
    assert!(reads > 0);
    assert_eq!(res!(src.now_millis()), BASE + 10_000 * STEP);
    res!(fs::remove_dir_all(&dir));
    Ok(())
}

#[test]
fn test_smaller_value_clamps_03() -> Outcome<()> {
    let dir = res!(scratch("clamp"));
    let path = dir.join("clock");
    res!(put(&path, 5_000));
    let src = res!(driven(&path));
    assert_eq!(res!(src.now_millis()), 5_000);
    res!(put(&path, 4_000));
    assert_eq!(res!(src.now_millis()), 5_000);
    res!(put(&path, 6_000));
    assert_eq!(res!(src.now_millis()), 6_000);
    res!(fs::remove_dir_all(&dir));
    Ok(())
}

#[test]
fn test_seven_byte_file_errors_04() -> Outcome<()> {
    let dir = res!(scratch("short"));
    let path = dir.join("clock");
    res!(fs::write(&path, [1u8; 7]));
    let src = res!(driven(&path));
    match src.now_millis() {
        Ok(v)   => return Err(err!("A 7-byte clock file read as {}.", v; Test)),
        Err(e)  => {
            let s = fmt!("{}", e);
            assert!(s.contains("7 bytes"), "error does not name the length: {}", s);
            assert!(s.contains("clock"), "error does not name the path: {}", s);
        }
    }
    res!(fs::remove_dir_all(&dir));
    Ok(())
}

#[test]
fn test_nanos_strictly_increase_05() -> Outcome<()> {
    let dir = res!(scratch("nanos"));
    let path = dir.join("clock");
    res!(put(&path, 1_700_000_000_000));
    let src = res!(driven(&path));
    let mut prev = res!(src.now_nanos());
    for _ in 0..10_000 {
        let n = res!(src.now_nanos());
        assert!(n > prev, "now_nanos went from {} to {}", prev, n);
        assert_eq!(n / 1_000_000, 1_700_000_000_000);
        prev = n;
    }
    res!(fs::remove_dir_all(&dir));
    Ok(())
}

#[test]
fn test_system_matches_systemtime_06() -> Outcome<()> {
    let src = res!(ClockSource::from_spec(None));
    assert!(!src.is_driven());
    assert!(!res!(ClockSource::from_spec(Some("system"))).is_driven());
    let a = res!(src.now_millis());
    let b = res!(SystemTime::now().duration_since(UNIX_EPOCH)).as_millis() as u64;
    assert!(a.abs_diff(b) <= 50, "System {} vs SystemTime {}", a, b);
    let n = res!(src.now_nanos());
    assert!((n / 1_000_000) as u64 >= a);
    assert!(ClockSource::from_spec(Some("sundial")).is_err());
    assert!(ClockSource::from_spec(Some("driven:")).is_err());
    Ok(())
}
