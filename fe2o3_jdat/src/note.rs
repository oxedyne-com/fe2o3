//! A `Dat::ABox` is an Annotated Box encapsulating a `Daticle` but extending a `Dat::Box` by
//! including a `String` annotation, which can be displayed natively in daticle string formatted
//! output.
//!
use crate::{
    daticle::Dat,
    map::MapKey,
};

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

impl Dat {

    /// The daticle under any notes on it.
    pub fn bare(&self) -> &Dat {
        match self {
            Dat::ABox(_, inner, _)  => inner.bare(),
            _                       => self,
        }
    }

    /// Is this a note standing alone, a comment on a line of its own?  The text decoder keeps one
    /// inside a list as an `ABox` round `Dat::Empty`, and inside a map as such a key or value.
    pub fn is_lone_note(&self) -> bool {
        matches!(self, Dat::ABox(..)) && matches!(self.bare(), Dat::Empty)
    }

    /// Does this map entry hold nothing but a comment?  A note on nothing is an `ABox` round
    /// `Dat::Empty`, as key or as value; a bare empty key and value is a stray comma instead.
    pub fn is_note_entry(key: &Dat, val: &Dat) -> bool {
        (matches!(key, Dat::ABox(..)) || matches!(val, Dat::ABox(..)))
            && matches!(key.bare(), Dat::Empty)
            && matches!(val.bare(), Dat::Empty)
    }

    /// The daticle as data alone, at every depth: a note on a value gives way to the value, and
    /// a note standing alone in a list or map is dropped, so a reader that matches on shape, such
    /// as `Dat::List`, sees through the comments a jdat file carries.  A note standing alone in a
    /// tuple becomes `Dat::Empty`, since a tuple's arity is fixed.
    pub fn without_notes(self) -> Dat {
        match self {
            Dat::ABox(_, inner, _)  => inner.without_notes(),
            Dat::Box(inner)         => Dat::Box(Box::new(inner.without_notes())),
            Dat::Opt(inner)         => Dat::Opt(Box::new((*inner).map(|d| d.without_notes()))),
            Dat::Usr(id, inner)     => Dat::Usr(id, inner.map(|d| Box::new(d.without_notes()))),
            Dat::List(items)        => Dat::List(items.into_iter()
                .filter(|d| !d.is_lone_note())
                .map(|d| d.without_notes())
                .collect()),
            Dat::Tup2(t)            => Dat::Tup2(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup3(t)            => Dat::Tup3(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup4(t)            => Dat::Tup4(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup5(t)            => Dat::Tup5(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup6(t)            => Dat::Tup6(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup7(t)            => Dat::Tup7(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup8(t)            => Dat::Tup8(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup9(t)            => Dat::Tup9(Box::new(t.map(|d| d.without_notes()))),
            Dat::Tup10(t)           => Dat::Tup10(Box::new(t.map(|d| d.without_notes()))),
            Dat::Map(m)             => Dat::Map(m.into_iter()
                .filter(|(k, v)| !Dat::is_note_entry(k, v))
                .map(|(k, v)| (k.without_notes(), v.without_notes()))
                .collect()),
            Dat::OrdMap(m)          => Dat::OrdMap(m.into_iter()
                .filter(|(k, v)| !Dat::is_note_entry(k.dat(), v))
                .map(|(k, v)| (MapKey::new(k.ord(), k.into_dat().without_notes()),
                    v.without_notes()))
                .collect()),
            other                   => other,
        }
    }
}
