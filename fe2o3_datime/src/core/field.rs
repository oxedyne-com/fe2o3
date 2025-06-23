use oxedize_fe2o3_core::prelude::*;

/// Enumeration of time field types.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TimeField {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    NanoSecond,
    DayOfWeek,
    DayOfYear,
    WeekOfYear,
}

/// Holder for parsed time field values.
#[derive(Clone, Debug, Default)]
pub struct TimeFieldHolder {
    pub year:		Option<i32>,
    pub month:		Option<u8>,
    pub day:		Option<u8>,
    pub hour:		Option<u8>,
    pub minute:		Option<u8>,
    pub second:		Option<u8>,
    pub nanosecond:	Option<u32>,
    pub day_of_week:	Option<u8>,
}

impl TimeFieldHolder {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn set_field(&mut self, field: TimeField, value: i64) -> Outcome<()> {
        match field {
            TimeField::Year => {
                self.year = Some(value as i32);
            },
            TimeField::Month => {
                if value < 1 || value > 12 {
                    return Err(err!("Month value {} out of range 1-12", value; Invalid, Input, Range));
                }
                self.month = Some(value as u8);
            },
            TimeField::Day => {
                if value < 1 || value > 31 {
                    return Err(err!("Day value {} out of range 1-31", value; Invalid, Input, Range));
                }
                self.day = Some(value as u8);
            },
            TimeField::Hour => {
                if value < 0 || value > 23 {
                    return Err(err!("Hour value {} out of range 0-23", value; Invalid, Input, Range));
                }
                self.hour = Some(value as u8);
            },
            TimeField::Minute => {
                if value < 0 || value > 59 {
                    return Err(err!("Minute value {} out of range 0-59", value; Invalid, Input, Range));
                }
                self.minute = Some(value as u8);
            },
            TimeField::Second => {
                if value < 0 || value > 59 {
                    return Err(err!("Second value {} out of range 0-59", value; Invalid, Input, Range));
                }
                self.second = Some(value as u8);
            },
            TimeField::NanoSecond => {
                if value < 0 || value > 999_999_999 {
                    return Err(err!("Nanosecond value {} out of range 0-999999999", value; Invalid, Input, Range));
                }
                self.nanosecond = Some(value as u32);
            },
            TimeField::DayOfWeek => {
                if value < 1 || value > 7 {
                    return Err(err!("Day of week value {} out of range 1-7", value; Invalid, Input, Range));
                }
                self.day_of_week = Some(value as u8);
            },
            _ => {
                return Err(err!("Cannot set field {:?} directly", field; Invalid, Input));
            }
        }
        Ok(())
    }
}