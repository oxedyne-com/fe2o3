pub mod hlc;

use crate::{
    prelude::*,
};

use std::{
    sync::OnceLock,
    time::Duration,
};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    path::PathBuf,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
    time::{
        Instant,
        SystemTime,
        UNIX_EPOCH,
    },
    thread,
};

/// Waits for the given boolean function to become true, or for the maximum duration to be reached.
/// Returns the starting `Instant` and whether the operation timed out.  Returns an error if the
/// given `Duration`s are inconsistent.
#[cfg(not(target_arch = "wasm32"))]
pub fn wait_for_true(
    check_interval: Duration,
    max_wait:       Duration,
    fn_true:        impl Fn() -> bool,
)
    -> Outcome<(Instant, bool)>
{
    if check_interval > max_wait {
        return Err(err!(
            "The given check interval, {:?}, should not be larger than the \
            given max wait, {:?}.", check_interval, max_wait;
        Invalid, Input));
    }
    let start = Instant::now();
    loop {
        if fn_true() {
            return Ok((start, false));
        } else {
            thread::sleep(check_interval);
        }
        if start.elapsed() > max_wait {
            return Ok((start, true));
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn wait_for_true(
    _check_interval: Duration,
    _max_wait:       Duration,
    _fn_true:        impl Fn() -> bool,
)
    -> Outcome<(std::time::Instant, bool)>
{
    Err(err!(
        "wait_for_true blocks the calling thread and is unavailable on the \
        wasm32 target; use an asynchronous wait instead.";
    Unimplemented))
}

/// A simple system clock stopwatch.
#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    t0:     Instant,
    #[cfg(target_arch = "wasm32")]
    t0:     f64,
    last:   usize,
}

impl Timer {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Self {
        Self {
            t0:     Instant::now(),
            last:   0,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Self {
        Self {
            t0:     crate::wasm::now_ms(),
            last:   0,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn reset(&mut self) {
        self.t0 = Instant::now();
        self.last = 0;
    }

    #[cfg(target_arch = "wasm32")]
    pub fn reset(&mut self) {
        self.t0 = crate::wasm::now_ms();
        self.last = 0;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn split_micros(&mut self) -> Outcome<usize> {
        self.last = try_sub!(self.t0.elapsed().as_micros() as usize, self.last);
        Ok(self.last)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn split_micros(&mut self) -> Outcome<usize> {
        let elapsed = ((crate::wasm::now_ms() - self.t0) * 1_000.0) as usize;
        self.last = try_sub!(elapsed, self.last);
        Ok(self.last)
    }
}

/// Where a process reads the wall clock from.
///
/// `System` is the operating system clock.  `Driven` reads the time from a
/// file holding exactly eight bytes, a little-endian `u64` count of
/// milliseconds since the Unix epoch, so that a test rig can move a process's
/// clock by rewriting one file.  The writer must replace the file by rename,
/// so a reader never sees a half-written value.
///
/// The methods take `&self`, so a caller can hold its own source; [`clock`] is
/// only the process default, set at most once by [`install`].
#[derive(Debug)]
pub enum ClockSource {
    System,
    #[cfg(not(target_arch = "wasm32"))]
    Driven(Driven),
}

/// A clock read from a file, see [`ClockSource`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct Driven {
    path:		PathBuf,
    last:		AtomicU64,	// Highest millisecond value returned
    last_ns:	AtomicU64,	// Last now_nanos value returned, u64 until 2554
}

#[cfg(not(target_arch = "wasm32"))]
impl Driven {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            path:		path.into(),
            last:		AtomicU64::new(0),
            last_ns:	AtomicU64::new(0),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    // Test hook: n calls' worth of ties already handed out in this millisecond.
    #[cfg(test)]
    fn preset_ties(&self, ms: u64, n: u64) {
        self.last_ns.store(ms * 1_000_000 + n, Ordering::SeqCst);
    }

    /// The file's milliseconds in nanoseconds, or one more than the last value
    /// handed out if that is higher.  A compare-and-swap loop rather than a
    /// counter, so no value repeats or steps back across threads however long
    /// the file is held still.
    fn next_nanos(&self) -> Outcome<u64> {
        let ms = res!(self.read_millis());
        let cand = match ms.checked_mul(1_000_000) {
            Some(n) => n,
            None    => return Err(err!(
                "Driven clock file {:?} holds {} ms, beyond the u64 nanosecond \
                range (the year 2554).", self.path, ms;
            Invalid, Input, Overflow)),
        };
        let mut prev = self.last_ns.load(Ordering::SeqCst);
        loop {
            let next = match prev.checked_add(1) {
                Some(p1)    => cand.max(p1),
                None        => return Err(err!(
                    "Driven clock file {:?}: the nanosecond counter is exhausted at {}.",
                    self.path, prev;
                Overflow)),
            };
            match self.last_ns.compare_exchange_weak(
                prev, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_)       => return Ok(next),
                Err(cur)    => prev = cur,
            }
        }
    }

    /// Reads the file and clamps the value so that it never goes backwards.
    fn read_millis(&self) -> Outcome<u64> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b)   => b,
            Err(e)  => return Err(err!(e,
                "Reading driven clock file {:?}.", self.path;
            IO, File, Read)),
        };
        let ms = match <[u8; 8]>::try_from(bytes.as_slice()) {
            Ok(arr) => u64::from_le_bytes(arr),
            Err(_)  => return Err(err!(
                "Driven clock file {:?} holds {} bytes, expected exactly 8 \
                (a little-endian u64 of milliseconds since the Unix epoch).",
                self.path, bytes.len();
            Invalid, Input, Size)),
        };
        let prev = self.last.fetch_max(ms, Ordering::SeqCst);
        Ok(prev.max(ms))
    }
}

impl ClockSource {
    /// Parses a clock specification: absent or `"system"` for the operating
    /// system clock, `"driven:<path>"` for a file-driven clock.
    pub fn from_spec(spec: Option<&str>) -> Outcome<Self> {
        let spec = match spec {
            None    => return Ok(Self::System),
            Some(s) => s.trim(),
        };
        if spec.is_empty() || spec == "system" {
            return Ok(Self::System);
        }
        if let Some(path) = spec.strip_prefix("driven:") {
            if path.is_empty() {
                return Err(err!(
                    "Clock spec '{}' names no file after 'driven:'.", spec;
                Invalid, Input, Missing));
            }
            #[cfg(not(target_arch = "wasm32"))]
            return Ok(Self::Driven(Driven::new(path)));
            #[cfg(target_arch = "wasm32")]
            return Err(err!(
                "Clock spec '{}': a driven clock reads a file and is \
                unavailable on the wasm32 target.", spec;
            Unimplemented));
        }
        Err(err!(
            "Clock spec '{}' is neither 'system' nor 'driven:<path>'.", spec;
        Invalid, Input))
    }

    /// Is the time read from a file rather than the operating system?
    pub fn is_driven(&self) -> bool {
        match self {
            Self::System    => false,
            #[cfg(not(target_arch = "wasm32"))]
            Self::Driven(_) => true,
        }
    }

    pub fn now_secs(&self) -> Outcome<u64> {
        Ok(res!(self.now_millis()) / 1_000)
    }

    pub fn now_millis(&self) -> Outcome<u64> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::System    => Ok(res!(system_since_epoch()).as_millis() as u64),
            #[cfg(target_arch = "wasm32")]
            Self::System    => Ok(crate::wasm::now_ms() as u64),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Driven(d) => d.read_millis(),
        }
    }

    /// The time as a `SystemTime`, for code that keeps `SystemTime` stamps.  For
    /// `Driven`, the file's milliseconds after the epoch.  Native only, since
    /// `SystemTime::now` is unavailable on wasm32.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn now_system_time(&self) -> Outcome<SystemTime> {
        match self {
            Self::System    => Ok(SystemTime::now()),
            Self::Driven(d) => {
                let ms = res!(d.read_millis());
                match UNIX_EPOCH.checked_add(Duration::from_millis(ms)) {
                    Some(t) => Ok(t),
                    None    => Err(err!(
                        "Driven clock file {:?} holds {} ms, beyond what SystemTime \
                        can represent.", d.path, ms;
                    Invalid, Input, Overflow)),
                }
            }
        }
    }

    /// For `Driven`, the file's milliseconds scaled to nanoseconds, bumped to
    /// one past the previous value when the file has not moved, so successive
    /// calls strictly increase and can order ledger entries.  Over a million
    /// calls in one file millisecond run ahead of it until the file catches up.
    pub fn now_nanos(&self) -> Outcome<u128> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::System    => Ok(res!(system_since_epoch()).as_nanos()),
            #[cfg(target_arch = "wasm32")]
            Self::System    => Ok((crate::wasm::now_ms() * 1_000_000.0) as u128),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Driven(d) => {
                Ok(res!(d.next_nanos()) as u128)
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn system_since_epoch() -> Outcome<Duration> {
    Ok(res!(SystemTime::now().duration_since(UNIX_EPOCH)))
}

static CLOCK:   OnceLock<ClockSource>   = OnceLock::new();
static SYSTEM:  ClockSource             = ClockSource::System;

/// Sets the process clock.  It can be set once; a second call is an error, so
/// a process cannot quietly change its time source part way through.
#[cfg(not(target_arch = "wasm32"))]
pub fn install(src: ClockSource) -> Outcome<()> {
    match CLOCK.set(src) {
        Ok(())      => Ok(()),
        Err(src)    => Err(err!(
            "The process clock is already installed ({:?}); refusing to \
            replace it with {:?}.", CLOCK.get(), src;
        Invalid, Input, Exists)),
    }
}

/// The process clock, `System` unless [`install`] set another.
pub fn clock() -> &'static ClockSource {
    match CLOCK.get() {
        Some(c) => c,
        None    => &SYSTEM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_timer() -> Outcome<()> {
        let mut t0 = Timer::new();
        sleep(Duration::from_micros(1000));
        let mut t = res!(t0.split_micros());
        msg!("timer split: {}", t); 
        sleep(Duration::from_micros(2000));
        t = res!(t0.split_micros());
        msg!("timer split: {}", t); 
        Ok(())
    }

    // Holding the file still past 10^6 calls must neither repeat nor step back.
    #[test]
    fn test_driven_nanos_strictly_increase_past_wrap() -> Outcome<()> {
        let dir = std::env::temp_dir().join(fmt!("fe2o3_clock_wrap_{}", std::process::id()));
        res!(std::fs::create_dir_all(&dir));
        let path = dir.join("clock");
        let ms: u64 = 1_000_000;
        res!(std::fs::write(&path, ms.to_le_bytes()));
        let src = ClockSource::Driven(Driven::new(&path));
        let mut prev = res!(src.now_nanos());
        if let ClockSource::Driven(d) = &src {
            d.preset_ties(ms, 999_998);
        }
        for i in 0..5 {
            let t = res!(src.now_nanos());
            if t <= prev {
                return Err(err!(
                    "Call {} after the preset returned {} <= previous {}.", i, t, prev;
                Test, Order));
            }
            prev = t;
        }
        // A later file value is still honoured once it passes the ties.
        res!(std::fs::write(&path, (ms + 1_000).to_le_bytes()));
        let t = res!(src.now_nanos());
        if t != (ms as u128 + 1_000) * 1_000_000 {
            return Err(err!("After advancing 1 s, got {} not {}.", t,
                (ms as u128 + 1_000) * 1_000_000; Test, Unexpected));
        }
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
