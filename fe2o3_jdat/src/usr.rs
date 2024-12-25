//! The `Dat::Usr` and `Kind::Usr` variants make use of `UsrKIndId` in order to allow custom user
//! daticles.

use crate::{
    Dat,
    Kind,
};

use oxedize_fe2o3_core::{
    prelude::*,
    map::MapMut,
};

use std::{
    fmt::{
        self,
        Debug,
    },
};

pub type UsrKindCode = u16;

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct UsrKind {
    label:  String,
    kind:   Option<Box<Kind>>,
}

impl UsrKind {

    pub fn new(code: UsrKindCode) -> Self {
        Self {
            label: Self::code_to_label(code),
            kind: None,
        }
    }

    pub fn code_to_label(code: UsrKindCode) -> String {
        fmt!("{}{}", Kind::USR_LABEL_PREFIX, Self::fmt_code(code))
    }

    pub fn fmt_code(code: UsrKindCode) -> String {
        fmt!("0x{:04x}", code)
    }

}

#[derive(Clone, Debug, Default, Ord,  PartialOrd)]
pub struct UsrKindId {
    code:   UsrKindCode,
    ukind:  UsrKind,
}

impl PartialEq for UsrKindId {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
    }
}
impl Eq for UsrKindId {}

impl From<UsrKindCode> for UsrKindId {
    fn from(code: UsrKindCode) -> Self {
        Self {
            code,
            ukind: UsrKind::new(code),
        }
    }
}

impl UsrKindId {

    pub const CODE_BYTE_LEN: usize = std::mem::size_of::<UsrKindCode>();

    pub fn new(
        code: UsrKindCode,
        lab_opt: Option<&str>,
        kind_opt: Option<Kind>,
    )
        -> Self
    {
        let label = match lab_opt {
            Some(lab) => lab.to_string(),
            None => UsrKind::code_to_label(code),
        };
        Self {
            code,
            ukind: UsrKind {
                label,
                kind: match kind_opt {
                    None => None,
                    Some(kind) => Some(Box::new(kind)),
                },
            },
        }
    }

    pub fn code(&self) -> UsrKindCode {
        self.code
    }

    pub fn label(&self) -> &str {
        &self.ukind.label
    }

    pub fn kind(&self) -> &Option<Box<Kind>> {
        &self.ukind.kind
    }

    pub fn label_to_code(label: &str) -> Outcome<UsrKindCode> {

        let prefix = Kind::USR_LABEL_PREFIX;

        if label.starts_with(prefix) {
            let rest = &label[prefix.len()..];
            if rest.starts_with("0x") {
                match UsrKindCode::from_str_radix(&rest[2..], 16) {
                    Ok(code) => Ok(code),
                    Err(e) => Err(err!(e, errmsg!(
                        "Suffix '{}' of label '{}' cannot be interpreted as a u{}.",
                        rest, label, Self::CODE_BYTE_LEN * 8,
                    ), Input, Invalid, String)),
                }
            } else {
                Err(err!(errmsg!(
                    "The code in label '{}' should start with a '0x'.", label,
                ), Input, Invalid, String))
            }
        } else {
            Err(err!(errmsg!(
                "Label '{}' does not start with prefix '{}'.",
                label, prefix,
            ), Input, Invalid, String))
        }
    }

    pub fn byte_prefix(&self) -> [u8; Self::CODE_BYTE_LEN] {
        self.code.to_be_bytes()
    }

    /// The caller must check that the given byte slice is at least as long as
    /// `UsrKindId::BYTE_LEN`.
    pub fn prefix_matches(prefix: [u8; Self::CODE_BYTE_LEN], byts: &[u8]) -> bool {
        prefix[0] == byts[0] &&
        prefix[1] == byts[1]
    }

    pub fn dat<D: Into<Dat>>(&self, optd: Option<D>) -> Dat {
        Dat::Usr(self.clone(), match optd {
            None => None,
            Some(d) => Some(Box::new(d.into())),
        })
    }
}                                   

#[derive(Clone, Debug, Default)]
pub struct UsrKinds<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>{
    c2k: M1,
    s2i: M2,
}

impl<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>
    From<(M1, M2)> for UsrKinds<M1, M2>
{
    fn from((c2k, s2i): (M1, M2)) -> Self {
        Self {
            c2k,
            s2i,
        }
    }
}

impl<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>
    UsrKinds<M1, M2>
{
    pub fn new(c2k: M1, s2i: M2) -> Self {
        Self {
            c2k,
            s2i,
        }
    }

    pub fn code_to_kind_map(&self) -> &M1 {
        &self.c2k
    }

    pub fn label_to_id_map(&self) -> &M2 {
        &self.s2i
    }

    pub fn add(&mut self, ukid: UsrKindId) -> Outcome<()> {
        let code = ukid.code();
        let label = ukid.label().to_string();
        match Kind::from_str(&label) {
            Ok(k) => Err(err!(errmsg!(
                "Label '{}' matches existing standard JDAT type, which \
                represents '{}'", label, k.desc(),
            ), Input, Exists, Invalid)),
            Err(_) => {
                if self.c2k.contains_key(&code) {
                    Err(err!(errmsg!(
                        "{:?} is already registered as a user kind.", ukid,
                    ), Input, Exists))
                } else {
                    self.c2k.insert(code, ukid.ukind.clone());
                    self.s2i.insert(label, UsrKindId {
                        code,
                        ukind: ukid.ukind,
                    });
                    Ok(())
                }
            },
        }
    }

    /// Looks for an entry matching the given string.
    pub fn get_label(&self, label: &str) -> Option<&UsrKindId> {
        let label = label.to_string();
        self.s2i.get(&label)
        //match self.s2i.get(&label) {
        //    Some(id) => Some(id.clone()),
        //    None => None,
        //}
    }

    /// Looks for an entry matching the given code.
    pub fn get_code(&self, code: &UsrKindCode) -> Option<UsrKindId> {
        match self.c2k.get(&code) {
            Some(ukind) => Some(UsrKindId {
                code: *code,
                ukind: ukind.clone(),
            }),
            None => None,
        }
    }

}
