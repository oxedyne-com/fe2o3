use oxedize_fe2o3_core::prelude::*;

/// Represents a known year value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct KnownYear(pub i32);

impl KnownYear {
    pub fn new(year: i32) -> Self {
        Self(year)
    }
    
    pub fn value(&self) -> i32 {
        self.0
    }
}

/// Represents a known month value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct KnownMonth(pub u8);

impl KnownMonth {
    pub fn new(month: u8) -> Outcome<Self> {
        if month < 1 || month > 12 {
            return Err(err!("Invalid month: {}", month; Invalid, Input, Range));
        }
        Ok(Self(month))
    }
    
    pub fn value(&self) -> u8 {
        self.0
    }
}

/// Represents a known day value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct KnownDay(pub u8);

impl KnownDay {
    pub fn new(day: u8) -> Outcome<Self> {
        if day < 1 || day > 31 {
            return Err(err!("Invalid day: {}", day; Invalid, Input, Range));
        }
        Ok(Self(day))
    }
    
    pub fn value(&self) -> u8 {
        self.0
    }
}

/// Represents a known hour value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct KnownHour(pub u8);

impl KnownHour {
    pub fn new(hour: u8) -> Outcome<Self> {
        if hour > 23 {
            return Err(err!("Invalid hour: {}", hour; Invalid, Input, Range));
        }
        Ok(Self(hour))
    }
    
    pub fn value(&self) -> u8 {
        self.0
    }
}

/// Represents a known minute value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct KnownMinute(pub u8);

impl KnownMinute {
    pub fn new(minute: u8) -> Outcome<Self> {
        if minute > 59 {
            return Err(err!("Invalid minute: {}", minute; Invalid, Input, Range));
        }
        Ok(Self(minute))
    }
    
    pub fn value(&self) -> u8 {
        self.0
    }
}

/// Represents a known second value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct KnownSecond(pub u8);

impl KnownSecond {
    pub fn new(second: u8) -> Outcome<Self> {
        if second > 59 {
            return Err(err!("Invalid second: {}", second; Invalid, Input, Range));
        }
        Ok(Self(second))
    }
    
    pub fn value(&self) -> u8 {
        self.0
    }
}

/// Represents a known nanosecond value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub struct KnownNanoSecond(pub u32);

impl KnownNanoSecond {
    pub fn new(nano: u32) -> Outcome<Self> {
        if nano > 999_999_999 {
            return Err(err!("Invalid nanosecond: {}", nano; Invalid, Input, Range));
        }
        Ok(Self(nano))
    }
    
    pub fn value(&self) -> u32 {
        self.0
    }
}