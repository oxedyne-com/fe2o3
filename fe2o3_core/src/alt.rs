use crate::prelude::*;

use std::{
    borrow::Cow,
    fmt::Debug,
};


/// `Alt` is a non-binary superset of `std::option::Option` that differentiates between an
/// unspecified variant and none.
#[derive(Clone, Debug)]
pub enum Alt<S> {
    Specific(Option<S>),
    Unspecified,
}

impl<S> Default for Alt<S> {
    fn default() -> Self {
        Alt::Unspecified
    }
}

impl<S> From<Option<S>> for Alt<S> {
    fn from(opt: Option<S>) -> Self {
        Self::Specific(opt)
    }
}

impl<S> Alt<S> {
    pub fn is_unspecified(&self) -> bool {
        match self {
            Self::Unspecified => true,
            _ => false,
        }
    }
    pub fn is_none(&self) -> bool {
        match self {
            Self::Specific(None) => true,
            _ => false,
        }
    }
    pub fn is_some(&self) -> bool {
        match self {
            Self::Specific(Some(_)) => true,
            _ => false,
        }
    }
}

/// `DefAlt` is like an `Alt` that provides for a default value, potentially of a different type.
#[derive(Clone, Debug)]
pub enum DefAlt<D, G> {
    Default(D),
    Given(G),
    None,
}

impl<D, G> Default for DefAlt<D, G> {
    fn default() -> Self {
        DefAlt::None
    }
}

impl<D, G> From<Option<G>> for DefAlt<D, G> {
    fn from(opt: Option<G>) -> Self {
        match opt {
            Some(g) => DefAlt::Given(g),
            None => DefAlt::None,
        }
    }
}

impl<D, G> From<Alt<G>> for DefAlt<D, G> {
    fn from(alt: Alt<G>) -> Self {
        match alt {
            Alt::Specific(Some(g)) => DefAlt::Given(g),
            _ => DefAlt::None,
        }
    }
}

impl<D, G> DefAlt<D, G> {

    pub fn is_none(&self) -> bool {
        match self {
            Self::None => true,
            _ => false,
        }
    }

    pub fn is_some(&self) -> bool {
        match self {
            Self::None => false,
            _ => true,
        }
    }
}

impl<
    D: Clone + Debug,
    G: Clone + Debug,
>
    DefAlt<D, G>
{
    pub fn from_or<'a>(&'a self, or: Override<D, G>) -> Cow<'a, Self> {
        match or {
            Override::PassThrough       => Cow::Borrowed(self),
            Override::Default(inner)    => Cow::Owned(Self::Default(inner)),
            Override::Given(inner)      => Cow::Owned(Self::Given(inner)),
            Override::None              => Cow::Owned(Self::None),
        }
    }
}

/// Useful in combination with `DefAlt`, `Override` expresses the possibility that we just go with
/// the value of a given `DefAlt` rather than override it.
#[derive(Clone, Debug)]
pub enum Override<D, G> {
    PassThrough,
    Default(D),
    Given(G),
    None,
}

impl<D, G> Override<D, G> {

    pub fn is_some(&self) -> bool {
        match self {
            Self::None | Self::PassThrough => false,
            _ => true,
        }
    }
}

/// `Gnomon` is less linguistically definite than `std::option::Option`.  It means "one that knows"
/// in Ancient Greek.
#[derive(Clone, Debug)]
pub enum Gnomon<T> {
    Known(T),
    Unknown,
}

impl<T> Gnomon<T> {
    pub fn required(&self, value_name: &str) -> Outcome<&T> {
        match self {
            Self::Known(v) => Ok(v),
            Self::Unknown => Err(err!(errmsg!(
                "A known value of {} is required.", value_name,
            ), Data, Missing)),
        }
    }
}
