use oxedize_fe2o3_core::{
    prelude::*,
    mem::Extract,
};
use oxedize_fe2o3_data::time::Timestamp;
use oxedize_fe2o3_jdat::{
    prelude::*,
    try_extract_tup2dat,
    tup2dat,
};
use oxedize_fe2o3_namex::id::LocalId as SchemeLocalId;

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct SchemeTimestamp {
    pub t: Timestamp,
    pub id: SchemeLocalId, // Minimalist mapping to Namex id.
}

impl ToDat for SchemeTimestamp {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(tup2dat![
            res!(self.t.to_dat()),
            res!(self.id.to_dat()),
        ])
    }
}

impl FromDat for SchemeTimestamp {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut v = try_extract_tup2dat!(dat);
        let t = res!(Timestamp::from_dat(v[0].extract()));
        let id = res!(SchemeLocalId::from_dat(v[1].extract()));
        Ok(Self {
            t,
            id,
        })
    }
}

impl SchemeTimestamp {
    pub fn now(id: SchemeLocalId) -> Outcome<Self> {
        Ok(Self {
            t: res!(Timestamp::now()),
            id,
        })
    }
}
