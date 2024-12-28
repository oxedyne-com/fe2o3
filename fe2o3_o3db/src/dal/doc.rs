//! This very thin document Data Abstraction Layer:
//!
//! - uses path-based heirarchical addressing,
//! - everything is still ultimately key-value `Dat` pairs.
//!
use crate::{
    base::constant,
};

use oxedize_fe2o3_core::{
    prelude::*,
};

use oxedize_fe2o3_jdat::{
    prelude::*,
    usr::{
        UsrKindId,
    },
};


#[derive(Clone, Debug)]
pub struct Doc {
    pub key: DocKey,
    pub val: Dat,
}

impl Doc {
    
    pub fn new_dir<S: Into<String>>(s: S, keys: Vec<String>) -> Outcome<Self> {
        Ok(Self {
            key: res!(DocKey::new_dir(s)),
            val: keys.into(),
        })
    }

    pub fn new_doc<S: Into<String>>(s: S, doc: Dat) -> Outcome<Self> {
        Ok(Self {
            key: res!(DocKey::new_doc(s)),
            val: doc.into(),
        })
    }

    pub fn into_dats(self) -> (Dat, Dat) {
        (self.key.into_dat(), self.val)
    }
}

#[derive(Clone, Debug)]
pub enum DocKey {
    Dir(String),
    Doc(String),
}

impl DocKey {

    pub fn new_dir<S: Into<String>>(s: S) -> Outcome<Self> {
        let mut s = s.into();
        res!(Self::validate_path(&s));
        if s.ends_with('/') { s.pop(); }
        Ok(Self::Dir(s))
    }

    pub fn new_doc<S: Into<String>>(s: S) -> Outcome<Self> {
        let mut s = s.into();
        res!(Self::validate_path(&s));
        if s.ends_with('/') { s.pop(); }
        Ok(Self::Doc(s))
    }

    fn ukind_dir() -> UsrKindId {
        UsrKindId::new(constant::USER_KIND_DIR_CODE, Some("dir"), Some(Kind::Str))
    }

    fn ukind_doc() -> UsrKindId {
        UsrKindId::new(constant::USER_KIND_DOC_CODE, Some("doc"), Some(Kind::Str))
    }

    pub fn into_dat(self) -> Dat {
        match self {
            Self::Dir(key) => {
                Dat::Usr(Self::ukind_dir(), Some(Box::new(Dat::Str(key))))
            }
            Self::Doc(key) => {
                Dat::Usr(Self::ukind_doc(), Some(Box::new(Dat::Str(key))))
            }
        }
    }

    pub fn parent(&self) -> Option<Self> {
        match self {
            Self::Dir(s) | Self::Doc(s) => {
                match s.rsplit_once('/') {
                    None => None,
                    Some((pre, _post)) => Some(Self::Dir(pre.to_string())),
                }
            }
        }
    }

    pub fn validate_path(s: &str) -> Outcome<()> {
        // Length limit.
        let len = s.chars().count();
        if len > constant::DOC_PATH_LEN_LIMIT {
            return Err(err!(
                "The doc path '{}' of length {} exceeds the limit of {}.",
                s, len, constant::DOC_PATH_LEN_LIMIT;
            Invalid, Input, String, TooBig));
        }
        // Must start with a '/'.
        if !s.starts_with('/') {
            return Err(err!(
                "The doc path '{}' does not start with a '/'.", s;
            Invalid, Input, String, Missing));
        }
        // No parts with zero length.
        for (i, part) in s.split('/').enumerate() {
            if part.chars().count() == 0 && i > 0 {
                return Err(err!(
                    "Zero length part of path '{}' not allowed in components {:?}",
                    s, s.split('/').collect::<Vec<_>>();
                Invalid, Input, String, TooSmall));
            }
        }
        Ok(())
    }
}
