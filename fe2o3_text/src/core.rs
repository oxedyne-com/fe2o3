use oxedyne_fe2o3_core::prelude::*;

use std::fmt;


#[derive(Clone, Debug, Default)]
pub struct Text<T: Clone + fmt::Debug + Default> {
    pub typ: T,
    pub txt: String,
}

impl<
    T: Clone + fmt::Debug + Default
>
    fmt::Display for Text<T>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.txt)
    }
}

impl<
    T: Clone + fmt::Debug + Default
>
    From<&str> for Text<T>
{
    fn from(s: &str) -> Self {
        Self::new(s, None)
    }
}

impl<
    T: Clone + fmt::Debug + Default
>
    Text<T>
{
    pub fn new<S: Into<String>>(txt: S, typ: Option<T>) -> Self {
        Self {
            typ: if let Some(typ) = typ { typ } else { T::default() },
            txt: txt.into(),
        }
    }
    pub fn typ(&self) -> &T {
        &self.typ
    }
    pub fn len(&self) -> usize {
        self.txt.chars().count()
    }
    pub fn is_empty(&self) -> bool {
        self.txt.is_empty()
    }
}
