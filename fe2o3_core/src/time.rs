use crate::{
    prelude::*,
};

use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    time::Instant,
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
}
