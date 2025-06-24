//! A `Dat::ABox` is an Annotated Box encapsulating a `Daticle` but extending a `Dat::Box` by
//! including a `String` annotation, which can be displayed natively in daticle string formatted
//! output.
//!
use oxedyne_fe2o3_core::{
    prelude::*,
    byte::{
        ToBytes,
        FromBytes,
    },
};


#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NoteConfig {
    pub adjacent:   bool,
    pub type1:      bool,
}

impl Default for NoteConfig {
    fn default() -> Self {
        Self {
            adjacent:   true,
            type1:      true,
        }
    }
}

impl NoteConfig {
    pub const ADJACENT_BIT: u8 = 0b0000_0001;
    pub const TYPE1_BIT:    u8 = 0b0000_0010;

    pub fn is_adjacent(&self) -> bool { self.adjacent }
    pub fn is_type1(&self) -> bool { self.type1 }

    pub fn set_adjacent(mut self, b: bool) -> Self {
        self.adjacent = b;
        self
    }
    pub fn set_type1(mut self, b: bool) -> Self {
        self.type1 = b;
        self
    }
}

impl ToBytes for NoteConfig {
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        let mut b = 0;
        if self.adjacent {
            b |= Self::ADJACENT_BIT;
        }
        if self.type1 {
            b |= Self::TYPE1_BIT;
        }
        buf.push(b);
        Ok(buf)
    }
}

impl FromBytes for NoteConfig {
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        Ok((
            Self {
                adjacent: (buf[0] & Self::ADJACENT_BIT) != 0,
                type1: (buf[0] & Self::TYPE1_BIT) != 0,
            },
            1,
        ))
    }
}
