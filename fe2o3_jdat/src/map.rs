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

    pub fn map_get_string(&self, key: &Self) -> Outcome<String> {
        let val = res!(self.map_get_must(key));
        match val.get_string() {
            Some(s) => Ok(s),
            None => Err(err!(
                "The key {:?} maps to a value of kind {:?}, \
                expected a string.", key, val.kind();
            Input, Mismatch)),
        }
    }

    pub fn map_get_i64(&self, key: &Self) -> Outcome<i64> {
        let val = res!(self.map_get_must(key));
        match val.get_i64() {
            Some(n) => Ok(n),
            None => Err(err!(
                "The key {:?} maps to a value of kind {:?}, \
                expected an integer.", key, val.kind();
            Input, Mismatch)),
        }
    }

    pub fn map_get_u64(&self, key: &Self) -> Outcome<u64> {
        let val = res!(self.map_get_must(key));
        match val.get_u64() {
            Some(n) => Ok(n),
            None => Err(err!(
                "The key {:?} maps to a value of kind {:?}, \
                expected a non-negative integer.", key, val.kind();
            Input, Mismatch)),
        }
    }

    pub fn map_get_f64(&self, key: &Self) -> Outcome<f64> {
        let val = res!(self.map_get_must(key));
        match val.get_float64() {
            Some(f) => Ok(f.0),
            None => Err(err!(
                "The key {:?} maps to a value of kind {:?}, \
                expected a number.", key, val.kind();
            Input, Mismatch)),
        }
    }

    pub fn map_get_map(&self, key: &Self) -> Outcome<&Self> {
        let val = res!(self.map_get_must(key));
        match val {
            Dat::Map(_) | Dat::OrdMap(_) => Ok(val),
            _ => Err(err!(
                "The key {:?} maps to a value of kind {:?}, \
                expected a map.", key, val.kind();
            Input, Mismatch)),
        }
    }

    pub fn map_get_list(&self, key: &Self) -> Outcome<&Vec<Dat>> {
        let val = res!(self.map_get_must(key));
        match val {
            Dat::List(v) | Dat::Vek(Vek(v)) => Ok(v),
            _ => Err(err!(
                "The key {:?} maps to a value of kind {:?}, \
                expected a list.", key, val.kind();
            Input, Mismatch)),
        }
    }

    pub fn map_put(&mut self, key: Self, val: Self) -> Outcome<Option<Self>> {
        match self {
            Dat::Map(m) => Ok(m.insert(key, val)),
            Dat::OrdMap(m) => {
                // Find an existing entry with a matching dat key.
                // Collect the matching MapKeys first so we can
                // mutate the map afterwards without aliasing.
                let existing: Vec<MapKey> = m
                    .iter()
                    .filter(|(mk, _)| mk.dat() == &key)
                    .map(|(mk, _)| mk.clone())
                    .collect();
                if existing.len() > 1 {
                    return Err(err!(
                        "There are {} entries with the same key {:?} \
                        in the OrdMap, which is not allowed.",
                        existing.len(), key;
                        Invalid, Input, Exists));
                }
                if let Some(mk) = existing.into_iter().next() {
                    // Replace in place, preserving `ord`.
                    let prev = m.remove(&mk);
                    m.insert(mk, val);
                    Ok(prev)
                } else {
                    // Append with the next available order slot.
                    let next_ord = m
                        .keys()
                        .map(|mk| mk.ord())
                        .max()
                        .map(|n| n + Dat::OMAP_ORDER_DELTA_DEFAULT)
                        .unwrap_or(Dat::OMAP_ORDER_START_DEFAULT);
                    m.insert(MapKey::new(next_ord, key), val);
                    Ok(None)
                }
            },
            _ => Err(err!(
                "Expected a Dat::Map or Dat::OrdMap, got {:?}.",
                self.kind();
                Input, Invalid, Mismatch)),
        }
    }

    /// As [`map_get`](Self::map_get), for a value that may be changed. Absent from anything but a map.
    pub fn map_get_mut(&mut self, key: &Self) -> Outcome<Option<&mut Self>> {
        match self {
            Dat::OrdMap(m) => {
                let mut found = m.iter_mut().filter(|(mk, _)| mk.dat() == key);
                let first = found.next();
                if found.next().is_some() {
                    return Err(err!(
                        "There are several entries with the same key {:?} in the OrdMap, \
                        which is not allowed.", key;
                    Invalid, Input, Exists));
                }
                Ok(first.map(|(_, v)| v))
            },
            Dat::Map(m) => Ok(m.get_mut(key)),
            _ => Ok(None),
        }
    }

    /// Sets `val` in the map at the path `keys`, a key for each level of nesting, and returns the value it
    /// replaced. A level that is absent is made as an empty map of the parent's kind, so setting
    /// `["colour", "space"]` in an empty map gives `{ colour: { space: .. } }`. A level that is present but
    /// is not a map is an error naming how far the path got, and nothing is changed by it.
    pub fn map_put_at(&mut self, keys: &[Self], val: Self) -> Outcome<Option<Self>> {
        let (key, rest) = match keys.split_first() {
            Some(split) => split,
            None => return Err(err!("A path into a map needs at least one key."; Input, Invalid, Missing)),
        };
        if rest.is_empty() {
            return self.map_put(key.clone(), val);
        }
        match res!(self.map_get(key)) {
            Some(Dat::Map(_)) | Some(Dat::OrdMap(_))    => (),
            Some(other) => return Err(err!(
                "The key {:?} holds a {:?}, not a map, so the path cannot go deeper than it.",
                key, other.kind();
            Input, Invalid, Mismatch)),
            None => {
                let child = match self {
                    Dat::OrdMap(_)  => Dat::OrdMap(BTreeMap::new()),
                    _               => Dat::Map(BTreeMap::new()),
                };
                res!(self.map_put(key.clone(), child));
            },
        }
        match res!(self.map_get_mut(key)) {
            Some(child) => child.map_put_at(rest, val),
            None        => Err(err!("The key {:?} was set and is gone.", key; Bug, Missing)),
        }
    }

    /// As [`map_put_at`](Self::map_put_at), the path a string of keys joined by dots, each a `Dat::Str`:
    /// `"colour.space"` is `["colour", "space"]`. An empty key, from a leading, trailing or doubled dot, is
    /// refused.
    pub fn map_put_dotted(&mut self, path: &str, val: Self) -> Outcome<Option<Self>> {
        let mut keys = Vec::new();
        for k in path.split('.') {
            if k.is_empty() {
                return Err(err!("The path '{}' has an empty key.", path; Input, Invalid));
            }
            keys.push(Dat::Str(k.to_string()));
        }
        self.map_put_at(&keys, val)
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
