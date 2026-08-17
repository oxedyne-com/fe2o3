use oxedyne_fe2o3_core::{
    prelude::*,
    mem::Extract,
};

use oxedyne_fe2o3_jdat::{
    prelude::*,
    try_extract_tup2dat,
    tup2dat,
};

use std::{
    fmt,
    time::{
        Duration,
        SystemTime,
    },
};

/// A generic ring buffer.
///
///
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///  | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |...| N |
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///        |   |
///       curr |
///           next
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///  | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |...| N |
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///                            |   |
///                           curr |
///                               next
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///  | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |...| N |
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///    |                                           |   
///    |                                          curr
///   next                                    
///
///
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RingBuffer<
    const N: usize,
    D: Clone + fmt::Debug,
> {
    pub curr:   usize,
    pub next:   usize,
    pub buf:    [Option<D>; N],
}

/// Fill buffer with the current time.
impl<
    const N: usize,
    D: Clone + fmt::Debug,
>
    Default for RingBuffer<N, D>
{
    fn default() -> Self {
        let d0 = None::<D>;
        let buf = std::array::from_fn(|_| d0.clone());
        let next = if N == 1 { 0 } else { 1 };
        Self {
            curr: 0,
            next,
            buf,
        }
    }
}

impl<
    const N: usize,
    D: Clone + fmt::Debug + ToDat,
>
    ToDat for RingBuffer<N, D>
{
    /// Converts `RingBuffer` to type:
    ///
    ///```ignore
    /// Dat::Tup2(Box<[
    ///     Dat::Vek(Vek<Vec<Dat::Opt(Box<Option<[  -+
    ///         res!(D::to_dat()),                   +-- the buffer as a Vec<Option<D>>
    ///     ]>>)>>),                                -+
    ///     Dat::U64(u64),                          -- the curr pointer (next can be reconstructed)
    /// ]>)
    ///```
    ///
    fn to_dat(&self) -> Outcome<Dat> {
        let mut v = Vec::with_capacity(N);
        for opt in &self.buf {
            let dat = match opt {
                Some(d) => Dat::Opt(Box::new(Some(res!(d.to_dat())))),
                None => Dat::Opt(Box::new(None)), 
            };
            v.push(dat)
        }
        Ok(tup2dat![
            Dat::Vek(Vek(v)),
            Dat::U64(self.curr as u64),
        ])
    }
}

impl<
    const N: usize,
    D: Clone + fmt::Debug + FromDat,
>
    FromDat for RingBuffer<N, D>
{
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut v = try_extract_tup2dat!(dat);
        let vek = try_extract_dat!(v[0].extract(), Vek);
        let d0 = None::<D>;
        let mut buf = std::array::from_fn(|_| d0.clone());
        for (i, dat1) in vek.into_iter().enumerate() {
            match *try_extract_dat!(dat1, Opt) {
                Some(dat2) => buf[i] = Some(res!(D::from_dat(dat2))),
                None => buf[i] = d0.clone(),
            }
        }
        let curr = try_extract_dat!(v[1].extract(), U64) as usize;
        Ok(Self {
            curr, 
            next: Self::next_index(curr),
            buf,
        })
    }
}

impl<
    const N: usize,
    D: Clone + fmt::Debug,
>
    RingBuffer<N, D>
{
    /// Set data at current location.
    pub fn set<I: Into<D>>(&mut self, new: I) {
        let new = new.into();
        self.buf[self.curr] = Some(new);
    }

    /// Set data at current location and advance pointer.
    pub fn set_and_adv<I: Into<D>>(&mut self, new: I) {
        self.set(new);
        self.adv();
    }

    /// Advance pointer.
    pub fn adv(&mut self) {
        self.curr = self.next;
        self.next = Self::next_index(self.curr);
    }

    /// Return the current value of the pointer index.
    pub fn curr(&self) -> usize { self.curr }

    /// Return the next value of the pointer index, given the current value.
    pub fn next_index(curr: usize) -> usize {
        if curr == N - 1 {
            0
        } else {
            curr + 1
        }
    }

    /// Return the previous value of the pointer index, given the current value.
    pub fn prev_index(curr: usize) -> usize {
        if curr == 0 {
            N - 1
        } else {
            curr - 1
        }
    }

    /// Get data at current pointer location.
    pub fn get(&self) -> Option<&D> {
        self.buf[self.curr].as_ref()
    }

    /// Get data at previous pointer location.
    pub fn prev(&self) -> Option<&D> {
        let index = if self.curr == 0 {
            N - 1
        } else {
            self.curr - 1
        };
        self.buf[index].as_ref()
    }

    /// Get data at next pointer location.
    pub fn next(&self) -> Option<&D> {
        self.buf[self.next].as_ref()
    }

    /// Copy the RingBuffer into a vector.
    pub fn to_vec(&self) -> Vec<Option<D>> {
        let mut result = Vec::with_capacity(N);
        for opt in &self.buf {
            result.push(opt.clone())
        }
        result
    }
}

/// A ring buffer consisting of timestamps.
///
///
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///  | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |...| N |
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///        i   |
///           next
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///  | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |...| N |
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///                            i   |
///                               next
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///  | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |...| N |
///  +---+---+---+---+---+---+---+---+---+---+---+---+
///    |                                           i   
///   next                                    
///
///

#[derive(Clone, Debug, Default)]
pub struct RingTimer<const N: usize>(RingBuffer<N, SystemTime>);

impl<const N: usize> std::ops::Deref for RingTimer<N> {
    type Target = RingBuffer<N, SystemTime>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const N: usize> RingTimer<N> {

    /// Add a timestamp to the ring buffer, returning the [Duration] since the previous entry.
    pub fn update(&mut self) -> Duration {
        // Note `self.0`, not a copy of it. `RingBuffer` is `Copy`, so binding
        // it to a local writes the timestamp into a temporary that is then
        // dropped, and the ring stays empty for ever. Every rate this type
        // reports would be zero, which is not a rate limiter, it is an
        // ornament.
        self.0.set_and_adv(SystemTime::now());
        self.last_duration()
    }

    fn extent(&self) -> (Option<SystemTime>, Option<SystemTime>, usize) {
        let mut oldest: Option<SystemTime> = None;
        let mut newest: Option<SystemTime> = None;
        let mut count = 0;
        for slot in self.0.buf.iter() {
            let t = match slot {
                Some(t) => *t,
                None => continue,
            };
            count += 1;
            match oldest {
                Some(o) if o <= t => (),
                _ => oldest = Some(t),
            }
            match newest {
                Some(n) if n >= t => (),
                _ => newest = Some(t),
            }
        }
        (oldest, newest, count)
    }

    pub fn count(&self) -> usize {
        let (_, _, count) = self.extent();
        count
    }

    pub fn is_full(&self) -> bool {
        self.count() == N
    }

    pub fn last_duration(&self) -> Duration {
        let newest_i = RingBuffer::<N, SystemTime>::prev_index(self.0.curr);
        let prev_i = RingBuffer::<N, SystemTime>::prev_index(newest_i);
        if newest_i == prev_i {
            return Duration::ZERO;
        }
        match (self.0.buf[newest_i], self.0.buf[prev_i]) {
            (Some(newest), Some(prev)) => match newest.duration_since(prev) {
                Ok(duration) => duration,
                Err(_) => Duration::ZERO,
            },
            _ => Duration::ZERO,
        }
    }

    pub fn total_duration(&self) -> Duration {
        let (oldest, newest, count) = self.extent();
        if count < 2 {
            return Duration::ZERO;
        }
        match (oldest, newest) {
            (Some(o), Some(n)) => match n.duration_since(o) {
                Ok(duration) => duration,
                Err(_) => Duration::ZERO,
            },
            _ => Duration::ZERO,
        }
    }

    pub fn avg_rps(&self) -> u64 {
        if !self.is_full() {
            return 0;
        }
        let ms = self.total_duration().as_millis();
        if ms == 0 {
            // A full ring inside a single millisecond is not a rate of zero,
            // it is the fastest rate this clock can resolve. Report the
            // maximum so that any finite limit is exceeded.
            return u64::MAX;
        }
        // N timestamps span N-1 intervals.
        let intervals = (N.saturating_sub(1)) as u128;
        let rps = intervals.saturating_mul(1_000) / ms;
        if rps > u64::MAX as u128 {
            u64::MAX
        } else {
            rps as u64
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_records_a_timestamp_00() {
        let mut timer = RingTimer::<4>::default();
        assert_eq!(timer.count(), 0);
        timer.update();
        assert_eq!(timer.count(), 1, "update must write to the ring, not a copy");
        timer.update();
        assert_eq!(timer.count(), 2);
    }

    #[test]
    fn test_ring_fills_and_wraps_00() {
        let mut timer = RingTimer::<4>::default();
        for _ in 0..4 {
            timer.update();
        }
        assert!(timer.is_full());
        assert_eq!(timer.count(), 4);
        // Wrapping overwrites rather than growing.
        timer.update();
        assert_eq!(timer.count(), 4);
    }

    #[test]
    fn test_avg_rps_is_zero_until_the_ring_fills_00() {
        let mut timer = RingTimer::<8>::default();
        for _ in 0..7 {
            timer.update();
            assert_eq!(timer.avg_rps(), 0,
                "a partial ring must not report a rate");
        }
    }

    #[test]
    fn test_a_fast_burst_reports_a_high_rate_00() {
        let mut timer = RingTimer::<8>::default();
        for _ in 0..8 {
            timer.update();
        }
        assert!(timer.is_full());
        // Eight timestamps taken as fast as the machine can loop span far less
        // than a second. That is a very high rate, not a zero one.
        let rps = timer.avg_rps();
        assert!(rps > 1_000,
            "a sub-second burst must report a high rate, got {}", rps);
    }

    #[test]
    fn test_a_slow_caller_reports_a_low_rate_00() {
        let mut timer = RingTimer::<4>::default();
        for _ in 0..4 {
            timer.update();
            std::thread::sleep(Duration::from_millis(60));
        }
        // Four timestamps spanning three 60 ms gaps is about 16 requests a
        // second: comfortably measurable, and well under a 50/s limit.
        let rps = timer.avg_rps();
        assert!(rps > 5 && rps < 40,
            "expected roughly 16 rps from 60 ms spacing, got {}", rps);
    }

    #[test]
    fn test_last_duration_measures_the_most_recent_gap_00() {
        let mut timer = RingTimer::<4>::default();
        // One timestamp: no interval to measure.
        timer.update();
        assert_eq!(timer.last_duration(), Duration::ZERO);
        std::thread::sleep(Duration::from_millis(40));
        timer.update();
        let gap = timer.last_duration();
        assert!(gap >= Duration::from_millis(35),
            "expected a ~40 ms gap, got {:?}", gap);
        assert!(gap < Duration::from_millis(500),
            "expected a ~40 ms gap, got {:?}", gap);
    }
}
