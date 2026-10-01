//! The boot clock, and noticing that the machine slept.
//!
//! `CLOCK_BOOTTIME` counts the time a machine is suspended; `CLOCK_MONOTONIC`, which is
//! what [`Instant`] reads on Linux, does not. A loop timed by `Instant` therefore sleeps
//! through a suspend without knowing: a quiet window it was measuring has the same time
//! left on waking as it had on going down. The two clocks differ by exactly the time
//! asleep, which is what [`Asleep`] reports.
//!
//! A process that is stopped (`SIGSTOP`, a debugger, a long swap-out) is not a suspend.
//! Both clocks run through it, so they stay level and nothing is reported.
//!
//! The system call comes from `nix`, which owns the `unsafe` this crate forbids.

use oxedyne_fe2o3_core::prelude::*;

use std::time::{
    Duration,
    Instant,
};

use nix::time::{
    clock_gettime,
    ClockId,
};

/// The shortest absence [`Asleep::new`] reports. The two clocks are read a moment apart, so
/// a smaller difference may be that moment and not a sleep.
pub const LEAST: Duration = Duration::from_secs(2);

/// Time since the kernel booted, counting any time the machine spent suspended.
///
/// The first field of `/proc/uptime` is the same clock.
pub fn since_boot() -> Outcome<Duration> {
    let ts = match clock_gettime(ClockId::CLOCK_BOOTTIME) {
        Ok(ts) => ts,
        Err(e) => return Err(err!(
            "Cannot read CLOCK_BOOTTIME: {}.", e;
            IO, System)),
    };
    let secs = match u64::try_from(ts.tv_sec()) {
        Ok(s) => s,
        Err(_) => return Err(err!(
            "CLOCK_BOOTTIME read {} seconds, before the boot.", ts.tv_sec();
            IO, System, Invalid)),
    };
    // A timespec holds under a billion nanoseconds, so the cast cannot lose any.
    Ok(Duration::new(secs, ts.tv_nsec() as u32))
}

/// How long the machine was suspended across a stretch that the boot clock timed as `boot`
/// and the monotonic clock as `mono`, when that is at least `least`.
///
/// The whole of the arithmetic, apart from reading the clocks, so a test can give it the
/// readings a suspend would leave.
pub fn slept(boot: Duration, mono: Duration, least: Duration) -> Option<Duration> {
    let gap = boot.saturating_sub(mono);
    if !gap.is_zero() && gap >= least {
        Some(gap)
    } else {
        None
    }
}

/// A reading of both clocks, kept to be compared with the next.
///
/// Call [`Asleep::check`] once a turn. It answers with the time the machine was suspended
/// since the call before, when that was at least the threshold, and starts counting afresh,
/// so one suspend is reported once.
#[derive(Clone, Copy, Debug)]
pub struct Asleep {
    boot:   Duration,   // CLOCK_BOOTTIME at the last check
    seen:   Instant,    // the monotonic clock at the same moment
    least:  Duration,   // a shorter absence is not reported
}

impl Asleep {

    /// Reads both clocks now, to be reported against from here, with the threshold [`LEAST`].
    pub fn new() -> Outcome<Self> {
        Self::with_least(LEAST)
    }

    pub fn with_least(least: Duration) -> Outcome<Self> {
        let boot = res!(since_boot());
        Ok(Self::at(boot, Instant::now(), least))
    }

    /// Starts from readings the caller took, which is how a test, or a caller that reads
    /// the clocks itself, places the baseline.
    pub fn at(boot: Duration, seen: Instant, least: Duration) -> Self {
        Self { boot, seen, least }
    }

    /// Reads both clocks now and compares them with the last check.
    ///
    /// A clock that cannot be read is no news: the check says nothing and keeps the last
    /// reading, so the next one measures across both turns and loses no suspend.
    pub fn check(&mut self) -> Option<Duration> {
        match since_boot() {
            Ok(boot) => self.check_at(boot, Instant::now()),
            Err(_) => None,
        }
    }

    /// As [`Asleep::check`], on readings the caller took.
    pub fn check_at(&mut self, boot: Duration, now: Instant) -> Option<Duration> {
        let found = slept(
            boot.saturating_sub(self.boot),
            now.saturating_duration_since(self.seen),
            self.least,
        );
        self.boot = boot;
        self.seen = now;
        found
    }
}
