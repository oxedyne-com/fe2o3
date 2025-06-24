use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedyne_fe2o3_hash::map::ShardMap;
use oxedyne_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    clone::Clone,
    fmt::Debug,
    sync::RwLock,
    //time::{
    //    Duration,
    //    SystemTime,
    //},
};

#[derive(Clone, Debug)]
pub enum UserState {
    Unknown,
    Blacklist, // No soup for you.
    Whitelist, // Come on through.
}

impl Default for UserState {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Default)]
pub struct UserLog<
    D: Clone + Debug + Default, // user supplied data container
> {
    pub state:  UserState,
    // Data
    pub data:   D,
}

#[derive(Debug)]
pub struct UserGuard<
    // ShardMap
    const C: usize, // Capacity (maximum number of bins).
    M: MapMut<HashForm, UserLog<D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
    // AddressData
    D: Clone + Debug + Default, // user supplied data container
> {
    pub umap: ShardMap<C, S, UserLog<D>, M, H>,
}

impl<
    // ShardMap
    const C: usize, // Capacity (maximum number of bins).
    M: MapMut<HashForm, UserLog<D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
    // AddressData
    D: Clone + Debug + Default, // user supplied data container
>
    UserGuard<C, M, H, S, D>
{
    /// Updates state for given address and returns whether the packet should be dropped.
    pub fn drop_packet<
        const UIDL: usize,
        UID: NumIdDat<UIDL>,
    >(
        &self,
        uid:            &UID,
        accept_unknown: bool,
    )
        -> Outcome<bool>
    {
        let (key, locked_map) = res!(self.get_locked_map(uid));
        let mut unlocked_map = lock_write!(locked_map);
        match unlocked_map.get_mut(&key) {
            Some(_ulog) => {
                // TODO examine user log
            },
            None => {
                if accept_unknown { 
                    let ulog = UserLog::default();
                    unlocked_map.insert(key, ulog);
                } else {
                    return Ok(true);
                }
            },
        }
        Ok(false)
    }

    pub fn get_locked_map<
        const UIDL: usize,
        UID: NumIdDat<UIDL>,
    >(
        &self,
        uid: &UID,
    )
        -> Outcome<(HashForm, &RwLock<M>)>
    {
        let key = self.umap.key(&uid.to_byte_array());
        let locked_map = res!(self.umap.get_shard_using_hash(&key));
        Ok((key, locked_map))
    }
    //pub fn get_user_log<'a>(&'a self, uid: &'a U) -> Option<&'a UserLog<D>> {
    //    self.umap.get(uid)
    //}

    //pub fn get_user_log_mut<'a>(&'a mut self, uid: &'a U) -> Option<&'a mut UserLog<D>> {
    //    self.umap.get_mut(uid)
    //}
}
