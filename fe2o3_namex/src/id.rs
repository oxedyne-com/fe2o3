use oxedize_fe2o3_core::{
    prelude::*,
    rand::RanDef,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
};

use std::{
    collections::BTreeMap,
    fmt,
};


new_type!(LocalId, u8, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd);

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ToDat for LocalId {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(dat!(self.0))
    }
}

impl FromDat for LocalId {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        Ok(Self(try_extract_dat!(dat, U8)))
    }
}

new_type!(NamexId, B32, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd);

impl From<B32> for NamexId {
    fn from(b: B32) -> Self {
        Self(b)
    }
}

impl From<&B32> for NamexId {
    fn from(b: &B32) -> Self {
        Self(*b)
    }
}

impl TryFrom<&Dat> for NamexId {
    type Error = Error<ErrTag>;

    fn try_from(dat: &Dat) -> std::result::Result<Self, Self::Error> {
        Ok(match dat {
            Dat::B32(a) => Self::from(a),
            _ => return Err(err!(errmsg!(
                "Expected a daticle of kind {:?}, received a {:?}.",
                Kind::B32, dat.kind(),
            ), Invalid, Input, Mismatch)),
        })
    }
}

impl ToDat for NamexId {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(Dat::B32(self.0))
    }
}

impl FromDat for NamexId {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        Ok(match dat {
            Dat::B32(a) => Self::from(a),
            _ => return Err(err!(errmsg!(
                "Expected a daticle of kind {:?}, received a {:?}.",
                Kind::B32, dat.kind(),
            ), Invalid, Input, Mismatch)),
        })
    }
}

impl TryFrom<&str> for NamexId {
    type Error = Error<ErrTag>;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        let byts = res!(base64::decode(s));
        let id = res!(Self::try_from(&byts[..]));
        Ok(id)
    }
}

impl TryFrom<&[u8]> for NamexId {
    type Error = Error<ErrTag>;

    fn try_from(b: &[u8]) -> std::result::Result<Self, Self::Error> {
        let id = Self::from(res!(B32::from_bytes(&b[..])).0);
        Ok(id)
    }
}

impl RanDef for NamexId {
    fn randef() -> Self {
        let a = B32::randef();
        Self(a)
    }
}

impl fmt::Display for NamexId {
    fn fmt (&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64::encode_config(&self[..], base64::STANDARD))
    }
}

pub struct NameIndex {
    s2i: BTreeMap<&'static str, NamexId>,
    i2s: BTreeMap<NamexId, &'static str>,
}

impl NameIndex {
    
    pub fn by_name(&self, name: &str) -> Option<&NamexId> {
        self.s2i.get(name)
    }

    pub fn contains_name(&self, name: &str) -> bool {
        self.s2i.contains_key(name)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.s2i.keys().cloned().collect()
    }

    pub fn by_id(&self, id: &NamexId) -> Option<&'static str> {
        self.i2s.get(id).copied()
    }

    pub fn contains_id(&self, id: &NamexId) -> bool {
        self.i2s.contains_key(id)
    }

    pub fn ids(&self) -> Vec<NamexId> {
        self.i2s.keys().cloned().collect()
    }
}

pub trait InNamex {

    /// The universal identifier, using a type so large that randomly selected ids have a very low
    /// chance of collision.
    fn name_id(&self) -> Outcome<NamexId>;

    /// A shorter id for local use.  Use 0 for "unknown" or "none".
    fn local_id(&self) -> LocalId { LocalId::default() }

    /// Allow the name to be associated with groups of other names, for specific purposes.  This
    /// returns a vector of Namex ids as base64 strings.
    fn assoc_names_base64(
        _gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        Ok(None)
    }

    /// Returns binary Namex ids based on user-specified base64 ids, indexed by name.
    fn assoc_names_index(
        gname: &'static str,
    )
        -> Outcome<Option<NameIndex>>
    {
        Ok(match res!(Self::assoc_names_base64(gname)) {
            Some(ids) => {
                let mut s2i = BTreeMap::new();
                let mut i2s = BTreeMap::new();
                for (name, idstr) in ids {
                    let id = res!(NamexId::try_from(idstr));
                    s2i.insert(name, id.clone());
                    i2s.insert(id, name);
                }
                if s2i.len() == 0 {
                    None
                } else {
                    Some(NameIndex { s2i, i2s })
                }
            },
            None => None,
        })
    }

    ///// Provides a search-by-name functionality with scope and ordering defined by the implementor.
    //fn by_name<'a>(
    //    &self,
    //    gname: &'static str,
    //    name: &'static str,
    //    ndb: &'a Namex<M1, M2>,
    //)
    //    -> Outcome<Option<(B32, &'a Entity)>>
    //{
    //    match res!(self.assoc_as_ids(gname)) {
    //        Some(ids) => {
    //            for id in ids {
    //                match ndb.by_id(&id) {
    //                    Some(entity) => {
    //                        for nam in &entity.nams {
    //                            if nam == name {
    //                                return Ok(Some((id, entity)));
    //                            }
    //                        }
    //                    },
    //                    None => return Err(err!(errmsg!(
    //                        "Namex entry '{}' does not have a (nams) field, which is required.",
    //                        Namex::<M1, M2>::id_to_string(&id),
    //                    ), Invalid, Input, Missing)),
    //                }
    //            }
    //        },
    //        None => (),
    //    }
    //    Ok(None)
    //}
}

impl InNamex for () {
    fn name_id(&self) -> Outcome<NamexId> { Ok(NamexId::default()) }
    fn assoc_names_base64(
        _gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        Ok(None)
    }
    fn assoc_names_index(
        _gname: &'static str,
    )
        -> Outcome<Option<NameIndex>>
    {
        Ok(None)
    }
}
