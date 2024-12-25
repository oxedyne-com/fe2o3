use crate::{
    text::nav::Navigator,
};

use oxedize_fe2o3_core::prelude::*;

use std::{
    fmt,
};


#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorMode {
    #[default]
    Navigation,
    Modify,
    Insert,
    Replace,
}

impl fmt::Display for EditorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Navigation    => "NAV",
            Self::Modify        => "MOD",
            Self::Insert        => "INS",
            Self::Replace       => "REP",
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Editor {
    pub nav:    Navigator,
    pub mode:   EditorMode,
}
