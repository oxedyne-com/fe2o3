use oxedyne_fe2o3_core::prelude::*;

/// English ordinal numbers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OrdinalEnglish {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
    // ... more ordinals could be added
}

impl OrdinalEnglish {
    pub fn from_number(n: u8) -> Outcome<Self> {
        match n {
            1 => Ok(Self::First),
            2 => Ok(Self::Second),
            3 => Ok(Self::Third),
            4 => Ok(Self::Fourth),
            5 => Ok(Self::Fifth),
            _ => Err(err!("Ordinal {} not implemented", n; Unimplemented)),
        }
    }
}