use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::kind::Kind;


/// How many words a value takes.  Only the last values of a command may be anything but
/// `One`: a value that must be present cannot follow one that may be absent, and a repeated
/// value is always the last.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Arity {
    #[default]
    One,        // exactly one word
    Optional,   // zero or one
    Many,       // zero or more
    OneOrMore,
}

impl Arity {
    /// Can the value be left with nothing?
    pub fn may_be_empty(&self) -> bool {
        match self {
            Self::One       => false,
            Self::Optional  => true,
            Self::Many      => true,
            Self::OneOrMore => false,
        }
    }

    /// Does the value go on taking words after the first?
    pub fn repeats(&self) -> bool {
        match self {
            Self::One       => false,
            Self::Optional  => false,
            Self::Many      => true,
            Self::OneOrMore => true,
        }
    }
}

/// A value expected by a message, a command or an argument.
///
/// A `(Kind, String)` pair, the form every caller used before values had names, converts with
/// `.into()`: the string becomes the help text and the value is unnamed and required.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Val {
    pub kind:       Kind,
    pub name:       String,         // shown as <name>; empty shows the kind
    pub help:       String,
    pub arity:      Arity,
    pub missing:    Option<String>, // the caller's own sentence when it is absent
}

impl From<(Kind, String)> for Val {
    fn from((kind, help): (Kind, String)) -> Self {
        Self {
            kind,
            help,
            ..Default::default()
        }
    }
}

impl From<(Kind, &str)> for Val {
    fn from((kind, help): (Kind, &str)) -> Self {
        Self::from((kind, help.to_string()))
    }
}

impl Val {

    pub fn new<S: Into<String>>(kind: Kind, name: S) -> Self {
        Self {
            kind,
            name: name.into(),
            ..Default::default()
        }
    }

    /// A string value taken from a command line word as it stands.
    pub fn text<S: Into<String>>(name: S) -> Self {
        Self::new(Kind::Str, name)
    }

    pub fn help<S: Into<String>>(mut self, s: S) -> Self {
        self.help = s.into();
        self
    }

    pub fn arity(mut self, arity: Arity) -> Self {
        self.arity = arity;
        self
    }

    pub fn optional(self) -> Self { self.arity(Arity::Optional) }
    pub fn many(self) -> Self { self.arity(Arity::Many) }
    pub fn one_or_more(self) -> Self { self.arity(Arity::OneOrMore) }

    pub fn missing<S: Into<String>>(mut self, s: S) -> Self {
        self.missing = Some(s.into());
        self
    }

    /// The value's name as help shows it, without brackets: its own name, or else its kind.
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            fmt!("{}", self.kind)
        } else {
            self.name.clone()
        }
    }

    /// The value as a synopsis shows it, e.g. `<mark>`, `[<dir>]`, `<path>...`.
    pub fn synopsis(&self) -> String {
        let base = fmt!("<{}>", self.label());
        match self.arity {
            Arity::One          => base,
            Arity::Optional     => fmt!("[{}]", base),
            Arity::Many         => fmt!("[{}...]", base),
            Arity::OneOrMore    => fmt!("{}...", base),
        }
    }

    /// Checks that a list of values has a shape a parser can read without guessing.
    pub fn check_shape(vals: &[Val], owner: &str) -> Outcome<()> {
        let mut loose = false;
        for (i, val) in vals.iter().enumerate() {
            if val.arity.repeats() && i + 1 != vals.len() {
                return Err(err!(
                    "Value {} ('{}') of {} repeats, but only the last value may.",
                    i + 1, val.label(), owner;
                Input, Invalid));
            }
            if loose && val.arity == Arity::One {
                return Err(err!(
                    "Value {} ('{}') of {} is required, but follows a value that may be \
                    absent, so no reader could tell which one a word fills.",
                    i + 1, val.label(), owner;
                Input, Invalid));
            }
            if val.arity != Arity::One {
                loose = true;
            }
        }
        Ok(())
    }

    /// The value that the `i`th word fills, if any.
    pub fn slot(vals: &[Val], i: usize) -> Option<&Val> {
        match vals.get(i) {
            Some(val) => Some(val),
            None => match vals.last() {
                Some(last) if last.arity.repeats() => Some(last),
                _ => None,
            },
        }
    }

    /// Is `n` a number of words that the values can hold?
    pub fn count_fits(vals: &[Val], n: usize) -> bool {
        let min = vals.iter().filter(|v| !v.arity.may_be_empty()).count();
        let unbounded = match vals.last() {
            Some(last) => last.arity.repeats(),
            None => false,
        };
        n >= min && (unbounded || n <= vals.len())
    }
}

/// Where a parser has got to in a list of values.
#[derive(Clone, Debug)]
pub struct Slots<'a> {
    pub vals:   &'a [Val],
    pub idx:    usize,  // the value being filled
    pub count:  usize,  // words already given to it
}

impl<'a> Slots<'a> {

    pub fn new(vals: &'a [Val]) -> Self {
        Self {
            vals,
            idx:    0,
            count:  0,
        }
    }

    pub fn current(&self) -> Option<&'a Val> { self.vals.get(self.idx) }

    /// Records that the current value took a word.
    pub fn accept(&mut self) {
        if let Some(val) = self.current() {
            if val.arity.repeats() {
                self.count += 1;
            } else {
                self.idx += 1;
                self.count = 0;
            }
        }
    }

    /// Could the values stop here and be complete?
    pub fn satisfied(&self) -> bool {
        for (i, val) in self.vals.iter().enumerate().skip(self.idx) {
            let given = if i == self.idx { self.count } else { 0 };
            if !val.arity.may_be_empty() && given == 0 {
                return false;
            }
        }
        true
    }

    /// The first value still wanting a word, if the values cannot stop here.
    pub fn wanting(&self) -> Option<&'a Val> {
        for (i, val) in self.vals.iter().enumerate().skip(self.idx) {
            let given = if i == self.idx { self.count } else { 0 };
            if !val.arity.may_be_empty() && given == 0 {
                return Some(val);
            }
        }
        None
    }

    /// The kinds of the values not yet filled, the current one included.
    pub fn outstanding(&self) -> Vec<Kind> {
        self.vals.iter().skip(self.idx).map(|v| v.kind.clone()).collect()
    }
}
