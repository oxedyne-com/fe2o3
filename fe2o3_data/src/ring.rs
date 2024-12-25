use oxedize_fe2o3_core::{
    prelude::*,
    mem::Extract,
};

use oxedize_fe2o3_jdat::{
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
        let mut inner = self.0;
        inner.set_and_adv(SystemTime::now());
        self.total_duration()
    }

    /// Duration since last timestamp.
    pub fn last_duration(&self) -> Duration {
        let inner = self.0;
        match inner.get() {
            Some(this) => match inner.prev() {
                Some(prev) => match this.duration_since(*prev) {
                    Ok(duration) => return duration,
                    Err(_) => (),
                },
                None => (),
            },
            None => (),
        }
        Duration::ZERO
    }

    /// Duration since the previous timestamp.
    pub fn total_duration(&self) -> Duration {
        let inner = self.0;
        match inner.get() {
            Some(this) => match inner.next() {
                Some(next) => match this.duration_since(*next) {
                    Ok(duration) => return duration,
                    Err(_) => (),
                },
                None => (),
            },
            None => (),
        }
        Duration::ZERO
    }

    /// Average Rate Per Second.
    pub fn avg_rps(&self) -> u64 {
        let d = self.total_duration().as_secs();
        if d == 0 {
            0
        } else {
            (N as u64) / d
        }
    }
}
