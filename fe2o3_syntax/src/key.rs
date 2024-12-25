use std::{
    fmt,
};

#[derive(Clone, Debug, Eq, Ord, PartialOrd, PartialEq)]
pub enum Key {
    Id(u16),
    Str(String),
}

impl From<&str> for Key {
    fn from(s: &str) -> Self {
        Key::Str(s.to_string())
    }
}

impl From<String> for Key {
    fn from(s: String) -> Self {
        Key::Str(s)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self {
            Key::Id(id) => write!(f, "{}", id)?,
            Key::Str(s) => write!(f, "{}", s)?,
        }
        Ok(())
    }
}
