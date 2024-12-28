use crate::{
    prelude::*,
    int::{
        Bound,
        One,
        Zero,
    },
};

use std::{
    fmt,
    ops::{
        Add,
        Div,
        Mul,
        Sub,
    },
};


#[derive(Clone, Debug, Default)]
pub struct ErrorWhen {
    cnt: usize,   
    lim: usize,   
}

impl ErrorWhen {
    pub fn new(lim: usize) -> Self {
        Self {
            cnt: 0,
            lim,
        }
    }

    pub fn inc(&mut self) -> Outcome<()> {
        if self.cnt == self.lim.saturating_sub(1) {
            Err(err!(
                "Counter reached limit of {}.", self.lim;
            LimitReached))
        } else {
            self.cnt += 1;
            Ok(())
        }
    }                                   
}

#[derive(Clone, Debug, Default)]
pub struct TrueWhen {
    cnt: usize,   
    lim: usize,   
}

impl TrueWhen {
    pub fn new(lim: usize) -> Self {
        Self {
            cnt: 0,
            lim,
        }
    }

    pub fn inc(&mut self) -> bool {
        if self.cnt == self.lim.saturating_sub(1) {
            true
        } else {
            self.cnt += 1;
            false
        }
    }                                   
}

/// A numerical iterator that provides the `next` and `prev` values in `delta` increments within a
/// range `[start, end]`.
#[derive(Clone, Debug)]
pub struct Counter<
    T: Clone
    + Copy
    + fmt::Debug
    + fmt::Display
    + PartialOrd
    + Ord
    + Add<Output = T>
    + Sub<Output = T>
    + Mul<Output = T>
    + Div<Output = T>
    + Bound
    + One
    + Zero
> {
    start:      T,
    end:        T,
    delta:      T,
    current:    T,
}

impl<
    T: Clone
    + Copy
    + fmt::Debug
    + fmt::Display
    + PartialOrd
    + Ord
    + Add<Output = T>
    + Sub<Output = T>
    + Mul<Output = T>
    + Div<Output = T>
    + Bound
    + One
    + Zero
>
    Default for Counter<T>
{
    fn default() -> Self {
        Self {
            start:      T::zero(),
            end:        T::max_size(),
            delta:      T::one(),
            current:    T::zero(),
        }
    }
}

impl<
    T: Clone
    + Copy
    + fmt::Debug
    + fmt::Display
    + PartialOrd
    + Ord
    + Add<Output = T>
    + Sub<Output = T>
    + Mul<Output = T>
    + Div<Output = T>
    + Bound
    + One
    + Zero
>
    Counter<T>
{
    pub fn new(
        start:  T,
        end:    T,
        delta:  T,
    )
        -> Outcome<Self>
    {
        if start >= end {
            return Err(err!(
                "The end {} should be after the start {}.", end, start;
            Input, Invalid, Order, Integer));
        }
        let d = (end - start) / delta;
        if start + d * delta != end {
            return Err(err!(
                "The range [{}, {}] is not divisible by the given delta {}.",
                start, end, delta;
            Input, Invalid, Divisibility, Integer));
        }
        Ok(Self {
            start,
            end,
            delta,
            current: start,
        })
    }

    pub fn start(&self)     -> T { self.start }
    pub fn end(&self)       -> T { self.end }
    pub fn delta(&self)     -> T { self.delta }
    pub fn current(&self)   -> T { self.current }

    pub fn next(&mut self) -> Option<T> {
        if self.current <= self.end - self.delta {
            self.current = self.current + self.delta;
            Some(self.current)
        } else {
            None
        }
    }

    pub fn prev(&mut self) -> Option<T> {
        if self.current >= self.start + self.delta {
            self.current = self.current - self.delta;
            Some(self.current)
        } else {
            None
        }
    }
}

/// A numerical iterator that provides the `next` and `prev` values in `delta` increments within a
/// range `[start, end]`, and when the end points are reached, cycles back.
#[derive(Clone, Debug)]
pub struct CycleCounter<
    T: Clone
    + Copy
    + fmt::Debug
    + fmt::Display
    + PartialOrd
    + Ord
    + Add<Output = T>
    + Sub<Output = T>
    + Mul<Output = T>
    + Div<Output = T>
    + Bound
    + One
    + Zero
> {
    start:      T,
    end:        T,
    delta:      T,
    current:    T,
}

impl<
    T: Clone
    + Copy
    + fmt::Debug
    + fmt::Display
    + PartialOrd
    + Ord
    + Add<Output = T>
    + Sub<Output = T>
    + Mul<Output = T>
    + Div<Output = T>
    + Bound
    + One
    + Zero
>
    Default for CycleCounter<T>
{
    fn default() -> Self {
        Self {
            start:      T::zero(),
            end:        T::max_size(),
            delta:      T::one(),
            current:    T::zero(),
        }
    }
}

impl<
    T: Clone
    + Copy
    + fmt::Debug
    + fmt::Display
    + PartialOrd
    + Ord
    + Add<Output = T>
    + Sub<Output = T>
    + Mul<Output = T>
    + Div<Output = T>
    + Bound
    + One
    + Zero
>
    CycleCounter<T>
{
    pub fn new(
        start:  T,
        end:    T,
        delta:  T,
    )
        -> Outcome<Self>
    {
        if start >= end {
            return Err(err!(
                "The end {} should be after the start {}.", end, start;
            Input, Invalid, Order, Integer));
        }
        let d = (end - start) / delta;
        if start + d * delta != end {
            return Err(err!(
                "The range [{}, {}] is not divisible by the given delta {}.",
                start, end, delta;
            Input, Invalid, Divisibility, Integer));
        }
        Ok(Self {
            start,
            end,
            delta,
            current: start,
        })
    }

    pub fn start(&self)     -> T { self.start }
    pub fn end(&self)       -> T { self.end }
    pub fn delta(&self)     -> T { self.delta }
    pub fn current(&self)   -> T { self.current }

    pub fn next(&mut self) -> T {
        if self.current <= self.end - self.delta {
            self.current = self.current + self.delta;
            self.current
        } else {
            self.current = self.start;
            self.current
        }
    }

    pub fn prev(&mut self) -> T {
        if self.current >= self.start + self.delta {
            self.current = self.current - self.delta;
            self.current
        } else {
            self.current = self.end;
            self.current
        }
    }
}
