//! Support for the code `#[derive(ToDat, FromDat)]` writes. Not for direct use: the names here
//! may change with the derive. The encoding is described on the derive, in `dat_map`.

use crate::{
    daticle::Daticle,
    conv::{
        FromDat,
        ToDat,
    },
};

use oxedyne_fe2o3_core::prelude::*;

pub use crate::{
    Dat,
    map::DaticleMap,
};
pub use oxedyne_fe2o3_core::Outcome;


/// Encodes one field, naming the type and field in any failure.
pub fn enc<T: ToDat>(v: &T, ty: &str, at: &str) -> Outcome<Dat> {
    match v.to_dat() {
        Ok(dat) => Ok(dat),
        Err(e)  => Err(err!(e, "Encoding {}, field '{}'.", ty, at; Encode)),
    }
}

pub fn put(map: &mut DaticleMap, key: &str, dat: Dat) {
    map.insert(Dat::Str(key.to_string()), dat);
}

/// An enum variant on the wire: a one-entry map from its name to its payload.
pub fn variant(name: &str, payload: Dat) -> Dat {
    let mut map = DaticleMap::new();
    map.insert(Dat::Str(name.to_string()), payload);
    Dat::Map(map)
}

pub fn take_map(dat: Dat, ty: &str) -> Outcome<DaticleMap> {
    match dat {
        Dat::Map(map)   => Ok(map),
        other           => Err(err!(
            "Decoding {}: a map of fields was expected, a {:?} was found.", ty, other.kind();
        Decode, Mismatch)),
    }
}

/// Takes and decodes the named field, refusing it when missing.
pub fn field<T: FromDat>(map: &mut DaticleMap, ty: &str, key: &str) -> Outcome<T> {
    match map.remove(&Dat::Str(key.to_string())) {
        Some(dat) => match T::from_dat(dat) {
            Ok(v)   => Ok(v),
            Err(e)  => Err(err!(e, "Decoding {}, field '{}'.", ty, key; Decode)),
        },
        None => Err(err!("Decoding {}: field '{}' is missing.", ty, key; Decode, Missing)),
    }
}

/// Refuses any field left over once every known field has been taken.
pub fn done(map: DaticleMap, ty: &str) -> Outcome<()> {
    if map.is_empty() {
        return Ok(());
    }
    let keys: Vec<String> = map.keys().map(|k| fmt!("{:?}", k)).collect();
    Err(err!("Decoding {}: unknown field(s) {}.", ty, keys.join(", "); Decode, Unknown))
}

pub fn take_list(dat: Dat, ty: &str, n: usize) -> Outcome<std::vec::IntoIter<Dat>> {
    let list = match dat {
        Dat::List(list) => list,
        other           => return Err(err!(
            "Decoding {}: a list of {} fields was expected, a {:?} was found.", ty, n, other.kind();
        Decode, Mismatch)),
    };
    if list.len() != n {
        return Err(err!(
            "Decoding {}: a list of {} fields was expected, one of {} was found.", ty, n, list.len();
        Decode, Size, Mismatch));
    }
    Ok(list.into_iter())
}

pub fn item<T: FromDat>(it: &mut std::vec::IntoIter<Dat>, ty: &str, i: usize) -> Outcome<T> {
    match it.next() {
        Some(dat) => match T::from_dat(dat) {
            Ok(v)   => Ok(v),
            Err(e)  => Err(err!(e, "Decoding {}, field {}.", ty, i; Decode)),
        },
        None => Err(err!("Decoding {}: field {} is missing.", ty, i; Decode, Missing)),
    }
}

pub fn take_unit(dat: Dat, ty: &str) -> Outcome<()> {
    match dat {
        Dat::Empty  => Ok(()),
        other       => Err(err!(
            "Decoding {}: an empty daticle was expected, a {:?} was found.", ty, other.kind();
        Decode, Mismatch)),
    }
}

/// The name and payload of an enum variant: a map of exactly one entry, keyed by a string.
pub fn take_variant(dat: Dat, ty: &str) -> Outcome<(String, Dat)> {
    let map = match dat {
        Dat::Map(map)   => map,
        other           => return Err(err!(
            "Decoding {}: a one-entry variant map was expected, a {:?} was found.",
            ty, other.kind();
        Decode, Mismatch)),
    };
    if map.len() != 1 {
        return Err(err!(
            "Decoding {}: a variant map holds one entry, this one holds {}.", ty, map.len();
        Decode, Size, Mismatch));
    }
    match map.into_iter().next() {
        Some((Dat::Str(name), payload)) => Ok((name, payload)),
        Some((key, _)) => Err(err!(
            "Decoding {}: a variant is named by a string, not a {:?}.", ty, key.kind();
        Decode, Mismatch)),
        None => Err(err!("Decoding {}: the variant map is empty.", ty; Decode, Missing)),
    }
}

pub fn unknown_variant(ty: &str, name: &str) -> Error<ErrTag> {
    err!("Decoding {}: there is no variant '{}'.", ty, name; Decode, Unknown)
}
