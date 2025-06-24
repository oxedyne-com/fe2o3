use crate::prelude::*;

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::ToBytes,
};

use std::{
    collections::BTreeMap,
};


pub type DaticleMap = BTreeMap<Dat, Dat>;
pub type OrdDaticleMap = BTreeMap<MapKey, Dat>;

pub fn create_dat_map(kv: Vec<(Dat, Dat)>) -> Dat {
    let mut map = DaticleMap::new();
    for (k, v) in kv {
        map.insert(k, v);
    }
    Dat::Map(map)
}

pub fn create_dat_ordmap(kv: Vec<(Dat, Dat)>) -> Dat {
    let mut map = OrdDaticleMap::new();
    let mut i: u64 = Dat::OMAP_ORDER_START_DEFAULT;
    for (k, v) in kv {
        map.insert(MapKey::new(i, k), v);
        i += Dat::OMAP_ORDER_DELTA_DEFAULT;
    }
    Dat::OrdMap(map)
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MapKey {
    ord: u64,
    dat: Dat,
}

impl ToBytes for MapKey {
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        let mut buf2 = Vec::new();
        buf2 = res!(self.dat.to_bytes(buf2));
        buf.extend_from_slice(&buf2);
        Ok(buf)
    }
}

impl MapKey {
    pub fn new(ord: u64, dat: Dat) -> Self {
        Self {
            ord,
            dat,
        }
    }
    pub fn ord(&self) -> u64 { self.ord }
    pub fn dat(&self) -> &Dat { &self.dat }
    pub fn into_dat(self) -> Dat { self.dat }
}

impl Dat {

    /// Find the key in the map and return a reference to the value.  If the map is an
    /// `Dat::OrdMap`, check that the key is not represented in multiple `MapKey`s.
    pub fn map_get(&self, key: &Self) -> Outcome<Option<&Self>> {
        match self {
            Dat::OrdMap(m) => {
                let entries: Vec<(_, _)> = m
                    .iter()
                    .filter(|(mk, _)| mk.dat() == key)
                    .collect();
                if entries.len() > 1 {
                    Err(err!(
                        "There are {} entries, {:?} with the same given daticle \
                        {:?} in the MapKey, which is not allowed.",
                        entries.len(), entries, key;
                    Invalid, Input, Exists))
                } else if entries.len() == 0 {
                    Ok(None)
                } else {
                    Ok(Some(entries[0].1))
                }
                
            },
            Dat::Map(m) => Ok(m.get(key)),
            _ => Ok(None),
        }
    }

    /// Find the key in the map and remove and return the value.  If the map is an
    /// `Dat::OrdMap`, check that the key is not represented in multiple `MapKey`s.
    pub fn map_remove(&mut self, key: &Self) -> Outcome<Option<Self>> {
        match self {
            Dat::OrdMap(m) => {
                let entries: Vec<(_, _)> = m
                    .iter()
                    .filter(|(mk, _)| mk.dat() == key)
                    .collect();
                if entries.len() > 1 {
                    Err(err!(
                        "There are {} entries, {:?} with the same given daticle \
                        {:?} in the MapKey, which is not allowed.",
                        entries.len(), entries, key;
                    Invalid, Input, Exists))
                } else if entries.len() == 0 {
                    Ok(None)
                } else {
                    Ok(m.remove(&entries[0].0.clone()))
                }
                
            },
            Dat::Map(m) => Ok(m.remove(key)),
            _ => Ok(None),
        }
    }

    /// Raise an error if the dat is not a map or the key is not present, otherwise return a
    /// reference to the associated value.
    pub fn map_get_must(&self, key: &Self) -> Outcome<&Self> {
        match res!(self.map_get(key)) {
            Some(dat) => Ok(dat),
            None => Err(err!(
                "The key {:?} does not map to any value, as expected.", key;
            Input, Missing)),
        }
    }

    /// Raise an error if the dat is not a map or the key is not present, otherwise return
    /// the removed value.
    pub fn map_remove_must(&mut self, key: &Self) -> Outcome<Self> {
        match res!(self.map_remove(key)) {
            Some(dat) => Ok(dat),
            None => Err(err!(
                "The key {:?} does not map to any value, as expected.", key;
            Input, Missing)),
        }
    }

    /// Get a reference to the `Dat` specified by the key from a `Dat::Map`.  The `Dat`
    /// must be of one of a `Kind` in the given list.
    pub fn map_get_type<'a>(
        &self,
        key: &'a Self,
        kinds: &[&Kind],
    )
        -> Outcome<Option<&Self>>
    {
        match self.map_get(key) {
            Ok(opt_val) => match opt_val {
                Some(val) => {
                    for kind in kinds {
                        if &val.kind() == *kind {
                            return Ok(Some(val));
                        }
                    }
                    Err(err!(
                        "The key {} maps to a value of kind {:?}, \
                        which does not correspond with any of {:?}.",
                        key, val.kind(), kinds;
                    Input, Mismatch))
                },
                None => Ok(None),
            },
            Err(e) => Err(e),
        }
    }

    /// Get a reference to the `Dat` specified by the key from a `Dat::Map`.  The `Dat` must be of
    /// one of a `Kind` in the given list.  If none is present, an error is returned.
    pub fn map_get_type_must<'a>(
        &self,
        key: &'a Self,
        kinds: &[&Kind],
    )
        -> Outcome<&Self>
    {
        match self.map_get(key) {
            Ok(opt_val) => match opt_val {
                Some(val) => {
                    for kind in kinds {
                        if &val.kind() == *kind {
                            return Ok(val);
                        }
                    }
                    Err(err!(
                        "The key {} maps to a value of kind {:?}, \
                        which does not correspond with any of {:?}.",
                        key, val.kind(), kinds;
                    Input, Mismatch))
                },
                None => Err(err!(
                    "The key {:?} does not map to any value, as expected.", key;
                Input, Missing)),
            },
            Err(e) => Err(e),
        }
    }

    /// Remove the `Dat` specified by the key from a `Dat::Map`.  The `Dat` must
    /// be of one of the `Kind`s given.
    pub fn map_remove_type(&mut self, key: &Self, kinds: &[&Kind]) -> Outcome<Self> {
        if let Dat::Map(m) = self {
            if let Some(d) = m.remove(key) {
                for kind in kinds {
                    if &d.kind() == *kind {
                        return Ok(d);
                    }
                }
                return Err(err!(
                    "The key {} maps to a value of kind {:?}, \
                    which does not correspond with any of {:?}.",
                    key, d.kind(), kinds;
                Input, Mismatch));
            } else {
                return Err(err!(
                    "The key {} does not map to any value, as \
                    required.",
                    key;
                Input, Missing));
            }
        } else {
            return Err(err!(
                "Dat {} must be a map.",
                self;
            Input, Mismatch));
        }
    }

    pub fn find(&self, keys: &Self) -> Outcome<Option<&Self>> {
        match (self, keys) {
            (Dat::Map(_m), Dat::List(keys)) => {
                let mut current = self;
                for key in keys {
                    match current {
                        Dat::Map(m) => {
                            current = match m.get(key) {
                                Some(v) => v,
                                None => return Ok(None),
                            };
                        }
                        _ => return Ok(None),
                    }
                }
                Ok(Some(current))
            }
            (Dat::OrdMap(_m), Dat::List(keys)) => {
                let mut current = self;
                for key in keys {
                    match current {
                        Dat::OrdMap(m) => {
                            current = match m.iter().find(|(k, _)| k.dat() == key) {
                                Some((_, v)) => v,
                                None => return Ok(None),
                            };
                        }
                        _ => return Ok(None),
                    }
                }
                Ok(Some(current))
            }
            (Dat::Map(_) | Dat::OrdMap(_), _) => Err(err!(
                "Expected a Dat::List argument.";
            Input, Invalid, Mismatch)),
            _ => Err(err!(
                "This method requires a map kind (Dat::Map or Dat::OrdMap).";
            Input, Invalid, Mismatch)),
        }
    }

    pub fn find_all<D: AsRef<Dat>>(&self, key: D) -> Outcome<Vec<&Self>> {
        let key = key.as_ref();
        let mut values = Vec::new();
    
        match self {
            Dat::Map(m) => {
                for (k, v) in m.iter() {
                    if k == key {
                        values.push(v);
                    }
                    if let Dat::Map(_) | Dat::OrdMap(_) = v {
                        values.extend(res!(v.find_all(key)));
                    }
                }
            }
            Dat::OrdMap(m) => {
                for (k, v) in m.iter() {
                    if k.dat() == key {
                        values.push(v);
                    }
                    if let Dat::Map(_) | Dat::OrdMap(_) = v {
                        values.extend(res!(v.find_all(key)));
                    }
                }
            }
            _ => (),
        }
    
        Ok(values)
    }
}
