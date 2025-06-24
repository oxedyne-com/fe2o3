use crate::{
    prelude::*,
    file::floc::{
        FileLocation,
        FileNum,
    },
    base::id::OzoneBotId,
    data::core::Key,
};

use oxedyne_fe2o3_data::time::Timestamp;
use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::{
    daticle::Dat,
    id::NumIdDat,
};

use std::{
    collections::BTreeMap,
    fmt,
    marker::PhantomData,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheId(pub u16);

impl fmt::Display for CacheId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CacheId({})", self.0)
    }
}
    
impl CacheId {
    pub fn new(c: u16) -> Self {
        Self(c)
    }
}

/// Contains the value itself, or its location. Used for cache retrieval.
#[derive(Clone, Debug)]
pub enum ValueOrLocation<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    Deleted(Meta<UIDL, UID>),
    Value(Vec<u8>, Meta<UIDL, UID>),
    Location(MetaLocation<UIDL, UID>),
}

/// Used for cache storage.
#[derive(Clone, Debug)]
pub struct MetaLocation<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    meta: Meta<UIDL, UID>,
    floc: FileLocation,
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    MetaLocation<UIDL, UID>
{
    pub fn meta(&self)          -> &Meta<UIDL, UID> { &self.meta }
    pub fn meta_move(self)      -> Meta<UIDL, UID>  { self.meta }
    pub fn file_location(&self) -> &FileLocation    { &self.floc }
    pub fn file_number(&self)   -> FileNum          { self.floc.file_number() }

    pub fn new_start_position(&mut self, new_start: u64) {
        self.floc.start = new_start
    }
}

#[derive(Clone, Debug)]
struct CacheSizes<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    byte: usize,
    meta: usize,
    floc: usize,
    mloc: usize,
    phantom: PhantomData<UID>,
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    Default for CacheSizes<UIDL, UID>
{
    fn default() -> Self {
        Self {
            byte: std::mem::size_of::<u8>(),
            meta: UIDL,
            floc: std::mem::size_of::<FileLocation>(),
            mloc: std::mem::size_of::<MetaLocation<UIDL, UID>>(),
            phantom: PhantomData,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KeyVal<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    pub key:    Key,
    pub val:    Vec<u8>,
    pub chash:  alias::ChooseHash,
    pub meta:   Meta<UIDL, UID>,
    pub cbpind: usize
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    KeyVal<UIDL, UID>
{
    pub fn stamp_time_now(&mut self) -> Outcome<()> {
        self.meta.stamp_time_now()
    }
}

#[derive(Clone, Debug)]
pub enum CacheEntry<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    LocatedValue(MetaLocation<UIDL, UID>, Option<Vec<u8>>),
    Deleted(Meta<UIDL, UID>),
}

/// A central goal of Ozone is to hold as much data as possible in volatile memory in zone caches.
#[derive(Clone, Debug, Default)]
pub struct Cache<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    ozid:   Option<OzoneBotId>, // Creator
    map:    BTreeMap<Vec<u8>, CacheEntry<UIDL, UID>>,
    size:   usize, // estimate of bytes stored in encoded form
    lim:    usize, // limit on size in [MB]
    cwt:    CacheWriteTracker,
    csizes: CacheSizes<UIDL, UID>,
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    Cache<UIDL, UID>
{
    const MLOC_SIZE: usize = std::mem::size_of::<MetaLocation<UIDL, UID>>();

    pub fn new(ozid: Option<&OzoneBotId>) -> Self {
        Self {
            ozid: ozid.map(|id| id.clone()),
            ..Default::default()
        }
    }

    fn ozid(&self) -> &Option<OzoneBotId> { &self.ozid }

    /// Getter for cache size in bytes.
    pub fn get_size(&self) -> usize { self.size }
    /// Getter for ancillary data structures size in bytes.
    pub fn get_ancillary_size(&self) -> usize { self.cwt.size }
    /// Getter for cache size limit in bytes.
    pub fn get_lim(&self) -> usize { self.lim }
    /// Getter for a reference to the cache map.
    pub fn map(&self) -> &BTreeMap<Vec<u8>, CacheEntry<UIDL, UID>> { &self.map }
    pub fn mloc_size(&self) -> usize {
        self.csizes.mloc
    }

    /// Returns the cache size in Mebibytes (1 [MiB] = 1024^2 bytes).
    pub fn size_mb(&self) -> f64 { (self.size as f64) / 1_048_576.0 }

    pub fn lim_size_mb(&self) -> f64 { (self.lim as f64) / 1_048_576.0 }

    /// Returns the fraction of the cache size compared to the size limit.
    pub fn size_fraction(&self) -> f64 { self.size_mb() / (self.lim as f64) }

    pub fn set_lim(&mut self, lim: usize) {
        self.lim = lim;
    }

    /// Insert key, value and location into the cache.
    /// Returns the old location, if the key was present.
    pub fn insert(
        &mut self,
        kbyts:  Vec<u8>,
        val:    Option<Vec<u8>>,
        floc:   FileLocation,
        meta:   Meta<UIDL, UID>,
    )
        -> Outcome<Option<FileLocation>>
    {
        let klen = kbyts.len();
        // 1. Make space in the cache if we are going to exceed the size limit. We could just
        //    jettison enough in order to fit the new value into the limit.  But subsequently, this
        //    expensive jettison process would be activated more frequently as the cache continues
        //    to bump up against its limit.  A compromise is to chop cache value storage back by a
        //    solid amount (say 20%), which would improve performance at the expense of older
        //    values.  A bit like the difference between mowing the lawn every day, or just
        //    every few weeks.
        if let Some(v) = &val {
            let vlen = v.len();
            if self.size + vlen > self.lim {
                let start_size = self.size;
                let desired_cache_size =
                    ((1.0 - constant::CACHE_JETTISON_FRAC_OF_LIM) * (self.lim as f64)) as usize;
                let keys = res!(self.cwt.jettison(self.size + vlen - desired_cache_size));
                let mut saved = 0;
                for key in &keys {
                    match self.map.get_mut(key) {
                        Some(CacheEntry::LocatedValue(_, val2_opt)) => {
                            if let Some(val2) = val2_opt {
                                saved += val2.len();
                                self.size = try_sub!(&self.size, val2.len());
                                *val2_opt = None;
                            }
                        },
                        _ => (),
                    }
                }
                trace!(sync_log::stream(), 
                    "{:?}: Automatically jettisoned the oldest {} (~{:.1}%) cache values \
                    to reduce size from {} to {} bytes.",
                    self.ozid(), keys.len(),
                    constant::CACHE_JETTISON_FRAC_OF_LIM * 100.0,
                    start_size, start_size - saved,
                );
            }
        }
        // 2. See if the key already exists.
        match self.map.get_mut(&kbyts) {
            Some(CacheEntry::LocatedValue(mloc, val2)) => {
                // 2.1 Only insert if the given data is newer.
                if meta.time <= mloc.meta.time {
                    warn!(sync_log::stream(), "{:?}: Attempt to insert new value at key = {:?} with timestamp = {:?}, \
                        when existing timestamp is newer at {:?}",
                        self.ozid.clone(), kbyts, meta.time, mloc.meta.time,
                    );
                    return Ok(None);
                }
                // 2.2 It does, insert the new info and return the old floc.
                let new_mloc = MetaLocation {
                    meta: meta.clone(),
                    floc,
                };
                let old_floc = mloc.file_location().clone();
                *mloc = new_mloc;
                match val {
                    Some(v) => {
                        let vlen = v.len();
                        match &val2 {
                            Some(v2) => self.size = try_sub!(&self.size, res!(Self::valsize(v2.len()))),
                            None => (),
                        }
                        res!(self.cwt.insert(res!(Timestamp::now()), &kbyts, vlen));
                        self.size = try_add!(&self.size, res!(Self::valsize(vlen)));
                        *val2 = Some(v);
                    },
                    None => (), // leave any existing value untouched
                }
                Ok(Some(old_floc))
            },
            None |
            Some(CacheEntry::Deleted(_)) => {
                // 2.3 It doesn't exist or was deleted, so create the entry and insert.
                match &val {
                    Some(v) => {
                        let vlen = v.len();
                        res!(self.cwt.insert(res!(Timestamp::now()), &kbyts, vlen));
                        self.size = try_add!(&self.size, res!(Self::valsize(vlen)));
                    },
                    None => (),
                }
                let mloc = MetaLocation {
                    meta,
                    floc,
                };
                self.map.insert(kbyts, CacheEntry::LocatedValue(mloc, val));
                self.size = try_add!(&self.size, klen);
                Ok(None)
            },
        }
    }

    fn valsize(len: usize) -> Outcome<usize> {
        Ok(try_add!(&Self::MLOC_SIZE, len))
    }

    /// Update file location information for a key.
    pub fn update(
        &mut self,
        k:      &Vec<u8>,
        floc:   FileLocation,
        meta:   Meta<UIDL, UID>,
    )
        -> Outcome<()>
    {
        match self.map.get_mut(k) {
            Some(CacheEntry::LocatedValue(mloc, _)) => {
                *mloc = MetaLocation { meta, floc };
                Ok(())
            },
            Some(CacheEntry::Deleted(_)) => Err(err!(
                "Key starting with {:?} has been deleted from cache.",
                if k.len() > 8 { &k[..8] } else { &k };
                Missing, Data)),
            None => Err(err!(
                "Key starting with {:?} not present in cache.",
                if k.len() > 8 { &k[..8] } else { &k };
                Unknown, Data)),
        }
    }

    /// Update file location information for a key.
    pub fn update_if_same_fnum(
        &mut self,
        k:      &Vec<u8>,
        loc:    &FileLocation,
    )
        -> Option<FileLocation> // updated
    {
        match self.map.get_mut(k) {
            Some(CacheEntry::LocatedValue(MetaLocation { floc, .. }, _)) => {
                let old_floc = floc.clone();
                if floc.fnum == loc.fnum {
                    floc.start = loc.start;
                    return Some(old_floc);
                }
                None
            },
            _ => None,
        }
    }

    /// Looks for the key in the cache map and if present, returns the value if it is present, or
    /// the latest location.  If the value is a `Dat::Box`, this method will (recursively)
    /// obtain the final value, but note that Rust has a default recursion limit of 128.
    pub fn get(&self, k: &[u8]) -> Outcome<Option<ValueOrLocation<UIDL, UID>>> {
        match self.map.get(k) {
            Some(CacheEntry::LocatedValue(mloc, val)) => {
                match &val {
                    Some(val) => { // Use cache value.
                        if val.len() > 1 && val[0] == Dat::BOX_CODE {
                            // Automatic key referral allows multiple keys to
                            // point to the same value.
                            return self.get(&val[1..]);
                        }
                        return Ok(Some(ValueOrLocation::Value(
                            val.clone(),
                            mloc.meta().clone(),
                        )));
                    },
                    // Return data location.
                    None => return Ok(Some(ValueOrLocation::Location(mloc.clone()))),
                }
            },
            Some(CacheEntry::Deleted(meta)) =>
                return Ok(Some(ValueOrLocation::Deleted(meta.clone()))),
            None => return Ok(None),
        }
    }

    pub fn clear_all_values(&mut self) {
        for (_k, centry) in self.map.iter_mut() {
            if let CacheEntry::LocatedValue(_, val) = centry {
                *val = None;
            }
        }
    }

}

#[derive(Clone, Debug)]
struct CacheWriteTrackerInfo {
    key:    Vec<u8>,
    hash:   u64,
    vlen:   usize,    
}

/// # Cache resource management
/// Maintaining an ordered (forward) map of `Timestamp`s to keys can facilitate a first-in,
/// first-out cache size limiting strategy.  In other words, this allows us to dump the oldest
/// values from the cache first.  A reverse map is maintained to allow us to identify when we can
/// delete an old timestamp from the forward map for a given key.
/// ```ignore
///
///   Forward map:        Reverse map:
///   t1 -> k1            k1 -> t1
///   t2 -> k2            k2 -> t2
///   t3 -> k1
///
///   Now, when t3 -> k1 is added to the forward map, the presence of k1 -> t1 in the reverse map
///   tells us that the t1 -> k1 entry in the forward map is redundant and can be deleted.  At the
///   same time, the reverse map is also updated
///
///   Forward map:        Reverse map:
///   t2 -> k2            k1 -> t3
///   t3 -> k1            k2 -> t2
///
/// ```
#[derive(Clone, Debug)]
pub struct CacheWriteTracker {
    fwd:    BTreeMap<Timestamp, CacheWriteTrackerInfo>,
    rev:    BTreeMap<u64, Timestamp>,
    bs:     usize, // base size
    size:   usize, // track total size estimate for data structure
}

impl Default for CacheWriteTracker {
    fn default() -> Self {
        Self {
            fwd:    BTreeMap::new(),
            rev:    BTreeMap::new(),
            bs: ( // base size for an entry in both maps, not including CacheWriteTrackerInfo::Key
                2 * std::mem::size_of::<Timestamp>() +
                2 * std::mem::size_of::<u64>() +
                std::mem::size_of::<usize>()
            ),
            size:   0,
        }
    }
}

impl CacheWriteTracker {
    /// This is only used for value insertions into the cache, not file locations.
    fn insert(
        &mut self,
        t3:     Timestamp,
        k1:     &Vec<u8>,
        vlen:   usize,
    )
        -> Outcome<()>
    {
        let hash = seahash::hash(&k1);
        let cwti = CacheWriteTrackerInfo {
            key:    k1.clone(),
            hash:   hash,
            vlen:   vlen,
        };
        self.fwd.insert(t3.clone(), cwti);
        match self.rev.insert(hash, t3) {
            Some(t1) => {
                self.fwd.remove(&t1);
                // just an update, no size change
            },
            None => {
                self.size = try_add!(&self.size, self.bs + k1.len()); // new insertion
            },
        }
        Ok(())
    }

    /// Identifies the oldest cached values whose lengths sum to at least the given value length,
    /// deleting their entries in the `CacheWriteTracker` while returning the list of associated
    /// keys, allowing the caller to scrub values from the cache, and advising of the size
    /// reduction of the tracker.  If the given value length exceeds the length of all existing
    /// cached values, the entire `CacheWriteTracker` contents will be deleted and the desired
    /// cache size reduction will not be achieved.
    fn jettison(
        &mut self,
        vlen: usize,
    )
        -> Outcome<Vec<Vec<u8>>>
    {
        let mut vlensum = 0;
        let mut jettison = Vec::new();
        for (t, cwti) in &self.fwd {
            jettison.push(t.clone());
            // Account for the value in the cache and for the
            // entries in the fwd and rev maps here.
            vlensum += cwti.vlen + self.bs + cwti.key.len();
            if vlensum > vlen {
                break;
            }
        }

        let mut cwt_size_reduction = 0;
        let mut keys = Vec::new();
        for t in jettison {
            match self.fwd.remove(&t) {
                Some(cwti) => {
                    self.rev.remove(&cwti.hash);
                    cwt_size_reduction += self.bs + cwti.key.len();
                    keys.push(cwti.key);
                },
                None => (), // unreachable
            }
        }

        self.size = try_sub!(&self.size, cwt_size_reduction);

        Ok(keys)
    }
}
