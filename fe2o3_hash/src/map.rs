//! A `ShardMap` is a shared array of generic sub-maps that can be independently locked for read
//! and write operations.  Access is via a key hash using the supplied hasher.  The capacity is
//! fixed at compile time and is limited to `u32::MAX`, since `u32` is the smallest common hash
//! format for `HashForm`.  A given key, value pair is assigned to a shard by taking the modulus of
//! the `HashForm::to_u32`.  Keys are provided as a byte slice but the hash bytes are stored as a
//! `HashForm` allowing more efficient primitive representations when possible.  Remapping to a
//! different number of shards is performed via cloning.  A `ShardMap` can be safely given to
//! threads simply by wrapping it in an `Arc`.

use oxedize_fe2o3_core::{
    prelude::*,
    map::{
        Map,
        MapMut,
    },
};
use oxedize_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};

use std::{
    clone::Clone,
    collections::BTreeMap,
    fmt::Debug,
    sync::{
        RwLock,
        RwLockReadGuard,
    },
};

#[derive(Clone, Debug)]
pub struct ExistingValue<K, V> {
    pub shard:      usize,
    pub key:        K,
    pub val:        V,
    pub val_old:    V,
}

impl<K, V> ExistingValue<K, V> {
    pub fn new(
        shard:      usize,
        key:        K,
        val:        V,
        val_old:    V,
    )
        -> Self
    {
        Self {
            shard,
            key,
            val,
            val_old,
        }
    }
}

#[derive(Debug)]
pub struct ShardMap<
    const C: usize, // Capacity (maximum number of shards).
    const S: usize, // Key hasher salt length.
    V: Clone + Debug, // Mapped value.
    M: MapMut<HashForm, V> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
>{
    pub n:      usize,
    pub shards: [Option<RwLock<M>>; C],
    pub hasher: H,
    pub salt:   [u8; S],
    phantom:    std::marker::PhantomData<V>,
}

impl<
    const C: usize,
    const S: usize,
    M: MapMut<HashForm, V> + Clone + Debug,
    V: Clone + Debug,
    H: Hasher + Send + Sync + 'static,
>
    ShardMap<C, S, V, M, H>
{
    const INIT: Option<RwLock<M>> = None;
    const MAX_CAPACITY: usize = u32::MAX as usize;

    pub fn new(
        n:          u32,
        salt:       [u8; S],
        init_map:   M,
        hasher:     H,
    )
        -> Outcome<Self>
    {
        if C > Self::MAX_CAPACITY {
            return Err(err!(
                "The specified capacity {} exceeds the (arbitrary) capacity limit of {}.",
                C, Self::MAX_CAPACITY;
            TooBig, Configuration));
        }
        let n = n as usize;
        if n > C {
            return Err(err!(
                "The specified number of maps {} exceeds the capacity of {}.",
                n, C;
            TooBig, Input));
        }
        let mut result = Self {
            n,
            shards: [Self::INIT; C],
            hasher,
            salt,
            phantom: std::marker::PhantomData,
        };
        for i in 0..(n as usize) {
            result.shards[i] = Some(RwLock::new(init_map.clone()));
        }
        Ok(result)
    }

    pub fn key(&self, pristine: &[u8]) -> HashForm {
        self.hasher.clone().hash(&[pristine], self.salt).as_hashform()
    }

    pub fn modulus(&self, hashform: &HashForm) -> Outcome<usize> {
        Ok((res!(hashform.to_u32()) as usize) % self.n)
    }

    pub fn insert(&self, key: &[u8], value: V) -> Outcome<Option<ExistingValue<HashForm, V>>> {
        let hashform = self.key(&key);
        self.insert_using_hash(hashform, value)
    }

    //pub fn insert_using_hash(&self, key: HashForm, value: V) -> Outcome<Option<V>> {
    //    let modulus = res!(self.modulus(&key));
    //    match &self.shards[modulus] {
    //        Some(locked_map) => {
    //            let mut unlocked_map = lock_write!(locked_map);
    //            Ok((*unlocked_map).insert(key, value))
    //        },
    //        None => return Err(err!(
    //            "Shard {} has not been initialised in a \
    //            ShardMap with size {} and capacity {}.",
    //            modulus, self.n, C,
    //        ), Bug, Configuration)),
    //    }
    //}

    pub fn insert_using_hash(&self, key: HashForm, value: V) -> Outcome<Option<ExistingValue<HashForm, V>>> {
        let modulus = res!(self.modulus(&key));
        match &self.shards[modulus] {
            Some(locked_map) => {
                let mut unlocked_map = lock_write!(locked_map);
                let existing_opt = match (*unlocked_map).get(&key) {
                    Some(value_old) => Some(ExistingValue::new(
                        modulus,
                        key.clone(),
                        value.clone(),
                        value_old.clone(),
                    )),
                    None => None,
                };
                let _value_opt = (*unlocked_map).insert(key, value);
                Ok(existing_opt)
            },
            None => return Err(err!(
                "Shard {} has not been initialised in a \
                ShardMap with size {} and capacity {}.",
                modulus, self.n, C;
            Bug, Configuration)),
        }
    }
    /// Returns the hashed key and the lock on the map.  If you want to access just a reference, get
    /// the map and lock it locally, e.g.
    /// ```ignore
    /// let (key, unlocked_shard) = res!(shardmap.get_lock(&k));
    /// let opt_ref_v = unlocked_shard.get(&key);
    /// ```
    pub fn get_lock(&self, key: &[u8]) -> Outcome<(HashForm, RwLockReadGuard<'_, M>)> {
        let hashform = self.key(&key);
        let locked_map = res!(self.get_shard_using_hash(&hashform));
        let unlocked_map = lock_read!(locked_map);
        Ok((hashform, unlocked_map))
    }

    /// Returns an optional clone of the map value.  If you want to access just a reference, get
    /// the map and lock it locally, e.g.
    /// ```ignore
    /// let locked_map = res!(maps.get_shard(&p));
    /// let unlocked_map = lock_read!(locked_map);
    /// let opt_ref_v = unlocked_map.get(&res!(maps.key(&p))); // Some(&v)
    /// ```
    pub fn get_clone(&self, key: &[u8]) -> Outcome<Option<V>> {
        let hashform = self.key(&key);
        let locked_map = res!(self.get_shard_using_hash(&hashform));
        let unlocked_map = lock_read!(locked_map);
        Ok(unlocked_map.get(&hashform).cloned())
    }

    pub fn get_shard(&self, key: &[u8]) -> Outcome<&RwLock<M>> {
        let hashform = self.key(&key);
        self.get_shard_using_hash(&hashform)
    }

    pub fn get_shard_using_hash(&self, key: &HashForm) -> Outcome<&RwLock<M>> {
        let modulus = res!(self.modulus(&key));
        match &self.shards[modulus] {
            Some(locked_map) => Ok(locked_map),
            None => return Err(err!(
                "Bin {} has not been initialised in a \
                ShardMap with size {} and capacity {}.",
                modulus, self.n, C;
            Bug, Configuration)),
        }
    }

    pub fn remap(&self, n2: u32) -> Outcome<Self> {
        let result = res!(Self::new(n2, self.salt, M::empty(), self.hasher.clone()));
        for i in 0..self.n {
            match &self.shards[i] {
                Some(locked_map) => {
                    let unlocked_map = lock_read!(locked_map);
                    for (k, v) in (*unlocked_map).iter() {
                        let modulus = (res!(k.to_u32()) as usize) % result.n;
                        match &result.shards[modulus] {
                            Some(locked_map2) => {
                                let mut unlocked_map2 = lock_write!(locked_map2);
                                unlocked_map2.insert(k.clone(), v.clone());
                            },
                            None => return Err(err!(
                                "Bin {} has not been initialised in the new \
                                ShardMap with size {} and capacity {}.",
                                modulus, result.n, C;
                            Bug, Configuration)),
                        }
                    }
                },
                None => return Err(err!(
                    "Bin {} has not been initialised in the existing \
                    ShardMap with size {} and capacity {}.",
                    i, self.n, C;
                Bug, Configuration)),
            }
        }
        Ok(result)
    }

    pub fn save(&self) -> Outcome<(BTreeMap<HashForm, V>, Vec<ExistingValue<HashForm, V>>)> {
        let mut collisions = Vec::new();
        let mut result = BTreeMap::new();
        for i in 0..self.n {
            match &self.shards[i] {
                Some(locked_map) => {
                    let unlocked_map = lock_read!(locked_map);
                    for (k, v) in (*unlocked_map).iter() {
                        if let Some(v_old) = result.insert(k.clone(), v.clone()) {
                            collisions.push(ExistingValue::new(
                                i,
                                k.clone(),
                                v.clone(),
                                v_old,
                            ));
                        }
                    }
                },
                None => (),
            }
        }
        Ok((result, collisions))
    }

    pub fn load<
        SRC: Map<Vec<u8>, V>,
    >(
        &self,
        src_map: SRC,
    )
        -> Outcome<Vec<ExistingValue<HashForm, V>>>
    {
        let mut existing = Vec::new();
        for (k, v) in src_map.iter() {
            if let Some(existing_value) = res!(self.insert(k, v.clone())) {
                existing.push(existing_value);
            }
        }
        Ok(existing)
    }
}
