use crate::{
    prelude::*,
};

use std::{
    time::{
        Duration,
        Instant,    
    },
    thread,
};

/// Waits for the given boolean function to become true, or for the maximum duration to be reached.
/// Returns the starting `Instant` and whether the operation timed out.  Returns an error if the
/// given `Duration`s are inconsistent.
pub fn wait_for_true(
    check_interval: Duration,
    max_wait:       Duration,
    fn_true:        impl Fn() -> bool,
) 
    -> Outcome<(Instant, bool)>
{
    if check_interval > max_wait {
        return Err(err!(errmsg!(
            "The given check interval, {:?}, should not be larger than the \
            given max wait, {:?}.", check_interval, max_wait,
        ), Invalid, Input));
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

/// A simple system clock stopwatch.
#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Timer {
    t0:     Instant,
    last:   usize, 
}

impl Timer {
    pub fn new() -> Self {
        Self {
            t0:     Instant::now(),
            last:   0,
        }
    }

    pub fn reset(&mut self) {
        self.t0 = Instant::now();
        self.last = 0;
    }

    pub fn split_micros(&mut self) -> Outcome<usize> {
        self.last = try_sub!(self.t0.elapsed().as_micros() as usize, self.last);
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
