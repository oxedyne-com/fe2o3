use crate::{
    prelude::*,
    file::floc::{
        FileLocation,
        FileNum,
    },
    base::id::OzoneBotId,
    data::core::Key,
    file::stored::RecordDigest,
};

use oxedyne_fe2o3_core::byte::FromBytes;
use oxedyne_fe2o3_data::time::Timestamp;
use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::{
    daticle::Dat,
    id::NumIdDat,
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
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

/// Does the record stamped `t1` at `f1` supersede the record of the same key stamped `t2` at
/// `f2`?  The later stamp wins, and between equal stamps the later place, by file number then
/// offset.  This is a total order on the records of one key, and collection keeps it, because a
/// collected record keeps its file number and its order among the records of its file.  The
/// cache, replay and the scan all decide by it, so they pick the same winner.
pub fn supersedes(
    t1: &Timestamp,
    f1: &FileLocation,
    t2: &Timestamp,
    f2: &FileLocation,
)
    -> bool
{
    (t1, f1.fnum, f1.start) > (t2, f2.fnum, f2.start)
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
    tombs:  TombTracker<UIDL, UID>,
}

/// Does the key name a chunk, a `Dat::Tup5u64` and nothing more?  A user key of that form is
/// refused at the write, so a record under it without a chunk index is a chunk tombstone.
pub fn is_chunk_key(k: &[u8]) -> bool {
    if k.first() != Some(&Dat::TUP5_U64_CODE) {
        return false;
    }
    match Dat::from_bytes(k) {
        Ok((Dat::Tup5u64(_), n)) => n == k.len(),
        _ => false,
    }
}

/// The chunk tombstones a cache holds, and the older records of each chunk key still on disk.
/// A tombstone shadows those records, so it may be forgotten only once every one of them is
/// durably gone, which the collector and the file bots report.  Bounded only while collection
/// runs: a record never collected keeps its tombstone.
#[derive(Clone, Debug, Default)]
pub struct TombTracker<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    older:      BTreeMap<Vec<u8>, BTreeSet<(FileNum, RecordDigest)>>,
    by_rid:     BTreeMap<(FileNum, RecordDigest), Vec<u8>>,
    tombs:      BTreeMap<Vec<u8>, Meta<UIDL, UID>>,
    replayed:   bool,
    size:       usize, // estimate of bytes held
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    TombTracker<UIDL, UID>
{
    const RID_SIZE:  usize = std::mem::size_of::<(FileNum, RecordDigest)>();
    const META_SIZE: usize = std::mem::size_of::<Meta<UIDL, UID>>();

    pub fn older(&self)     -> &BTreeMap<Vec<u8>, BTreeSet<(FileNum, RecordDigest)>> { &self.older }
    pub fn tombs(&self)     -> &BTreeMap<Vec<u8>, Meta<UIDL, UID>>  { &self.tombs }
    pub fn replayed(&self)  -> bool                                 { self.replayed }
    pub fn size(&self)      -> usize                                { self.size }

    fn add_older(&mut self, k: &[u8], fnum: FileNum, rid: RecordDigest) {
        let set = match self.older.get_mut(k) {
            Some(set) => set,
            None => {
                self.size = self.size.saturating_add(k.len());
                self.older.entry(k.to_vec()).or_default()
            },
        };
        if set.insert((fnum, rid)) {
            self.size = self.size.saturating_add(2 * Self::RID_SIZE + k.len());
            self.by_rid.insert((fnum, rid), k.to_vec());
        }
    }

    /// Forgets the gone record, and gives the key whose set it emptied.
    fn remove_older(&mut self, fnum: FileNum, rid: RecordDigest) -> Option<Vec<u8>> {
        let k = match self.by_rid.remove(&(fnum, rid)) {
            Some(k) => k,
            None => return None,
        };
        self.size = self.size.saturating_sub(2 * Self::RID_SIZE + k.len());
        let emptied = match self.older.get_mut(&k) {
            Some(set) => {
                set.remove(&(fnum, rid));
                set.is_empty()
            },
            None => false,
        };
        if emptied {
            self.older.remove(&k);
            self.size = self.size.saturating_sub(k.len());
            return Some(k);
        }
        None
    }

    fn set_tomb(&mut self, k: &[u8], meta: Meta<UIDL, UID>) {
        if self.tombs.insert(k.to_vec(), meta).is_none() {
            self.size = self.size.saturating_add(k.len() + Self::META_SIZE);
        }
    }

    fn clear_tomb(&mut self, k: &[u8]) {
        if self.tombs.remove(k).is_some() {
            self.size = self.size.saturating_sub(k.len() + Self::META_SIZE);
        }
    }
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
    pub fn get_ancillary_size(&self) -> usize { self.cwt.size + self.tombs.size }
    pub fn tomb_tracker(&self) -> &TombTracker<UIDL, UID> { &self.tombs }
    /// Getter for cache size limit in bytes.
    pub fn get_lim(&self) -> usize { self.lim }
    /// Getter for a reference to the cache map.
    pub fn map(&self) -> &BTreeMap<Vec<u8>, CacheEntry<UIDL, UID>> { &self.map }
    pub fn mloc_size(&self) -> usize {
        self.csizes.mloc
    }

    /// The cache size in mebibytes, one of which is 1024^2 bytes.
    pub fn size_mb(&self) -> f64 { (self.size as f64) / 1_048_576.0 }

    pub fn lim_size_mb(&self) -> f64 { (self.lim as f64) / 1_048_576.0 }

    /// Returns the fraction of the cache size compared to the size limit.
    pub fn size_fraction(&self) -> f64 { self.size_mb() / (self.lim as f64) }

    pub fn set_lim(&mut self, lim: usize) {
        self.lim = lim;
    }

    /// Insert key, value and location into the cache.
    ///
    /// Returns the location of whichever copy of the key is now superseded, and the record it
    /// holds, for the caller to schedule for garbage collection, or `None` when nothing was
    /// superseded.  That is usually the copy the cache held, but when the offered copy is the
    /// older of the two the cache keeps what it has and the offered location comes back instead.
    pub fn insert(
        &mut self,
        kbyts:  Vec<u8>,
        val:    Option<Vec<u8>>,
        floc:   FileLocation,
        meta:   Meta<UIDL, UID>,
    )
        -> Outcome<Option<(FileLocation, RecordDigest)>>
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
                                let vsize = res!(Self::valsize(val2.len()));
                                saved += vsize;
                                self.size = try_sub!(&self.size, vsize);
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
                // 2.1 Only insert if the given record supersedes the cached one.  Otherwise
                //     the offered copy is the superseded one, so its location goes back for
                //     flagging as old; returning nothing would leave it marked current in its
                //     file state forever, never reclaimable.  A copy at the location already
                //     cached is the same record arriving twice, not a supersession.
                if floc == *mloc.file_location() {
                    return Ok(None);
                }
                if !supersedes(&meta.time, &floc, &mloc.meta.time, mloc.file_location()) {
                    trace!(sync_log::stream(),
                        "{:?}: The value offered for key = {:?} at {:?} is stamped {:?}, \
                        no newer than the cached {:?}, so the offered copy is superseded.",
                        self.ozid.clone(), kbyts, floc, meta.time, mloc.meta.time,
                    );
                    let rid = res!(RecordDigest::new(&kbyts, &meta));
                    return Ok(Some((floc, rid)));
                }
                // 2.2 It does, insert the new info and return the old floc.
                let new_mloc = MetaLocation {
                    meta: meta.clone(),
                    floc,
                };
                let old = (mloc.file_location().clone(), res!(RecordDigest::new(&kbyts, mloc.meta())));
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
                Ok(Some(old))
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

    /// Records what an insert of a chunk key superseded, and whether the key now holds a
    /// tombstone, the insert of `floc_new` carrying no chunk index.  Nothing is dropped here.
    pub fn note(
        &mut self,
        kbyts:      &[u8],
        cind:       Option<usize>,
        floc_new:   &FileLocation,
        meta:       &Meta<UIDL, UID>,
        superseded: &Option<(FileLocation, RecordDigest)>,
    ) {
        if !is_chunk_key(kbyts) {
            return;
        }
        if let Some((floc, rid)) = superseded {
            self.tombs.add_older(kbyts, floc.fnum, *rid);
        }
        let holds_new = match self.map.get(kbyts) {
            Some(CacheEntry::LocatedValue(mloc, _)) => mloc.floc == *floc_new,
            _ => false,
        };
        if holds_new {
            match cind {
                None    => self.tombs.set_tomb(kbyts, meta.clone()),
                Some(_) => self.tombs.clear_tomb(kbyts),
            }
        }
    }

    /// Forgets the old records of file `fnum` now durably gone, and once the start's replay is
    /// done drops each tombstone they were the last shadowed record of.  Gives the location and
    /// record of each tombstone dropped, for the caller to flag old.
    pub fn records_gone(
        &mut self,
        fnum:   FileNum,
        rids:   &[RecordDigest],
    )
        -> Outcome<Vec<(FileLocation, RecordDigest)>>
    {
        let mut emptied = Vec::new();
        for rid in rids {
            if let Some(k) = self.tombs.remove_older(fnum, *rid) {
                emptied.push(k);
            }
        }
        let mut dropped = Vec::new();
        if self.tombs.replayed {
            for k in emptied {
                if let Some(gone) = res!(self.drop_tomb(&k)) {
                    dropped.push(gone);
                }
            }
        }
        Ok(dropped)
    }

    /// The replay has entered every record on disk, so a tombstone with no older record known
    /// shadows none, and is dropped.
    pub fn replay_done(&mut self) -> Outcome<Vec<(FileLocation, RecordDigest)>> {
        self.tombs.replayed = true;
        let keys: Vec<Vec<u8>> = self.tombs.tombs.keys().cloned().collect();
        let mut dropped = Vec::new();
        for k in keys {
            if let Some(gone) = res!(self.drop_tomb(&k)) {
                dropped.push(gone);
            }
        }
        Ok(dropped)
    }

    fn drop_tomb(&mut self, k: &[u8]) -> Outcome<Option<(FileLocation, RecordDigest)>> {
        if self.tombs.older.contains_key(k) {
            return Ok(None);
        }
        let meta = match self.tombs.tombs.get(k) {
            Some(meta) => meta.clone(),
            None => return Ok(None),
        };
        let floc = match self.map.get(k) {
            Some(CacheEntry::LocatedValue(mloc, _)) if mloc.meta == meta => mloc.floc.clone(),
            _ => return Ok(None),
        };
        let rid = res!(RecordDigest::new(k, &meta));
        res!(self.remove(k, &meta));
        self.tombs.clear_tomb(k);
        Ok(Some((floc, rid)))
    }

    /// Removes the entry of `k` if it is still the record stamped `meta`.
    pub fn remove(&mut self, k: &[u8], meta: &Meta<UIDL, UID>) -> Outcome<bool> {
        let vlen = match self.map.get(k) {
            Some(CacheEntry::LocatedValue(mloc, val)) if mloc.meta == *meta =>
                val.as_ref().map(|v| v.len()),
            _ => return Ok(false),
        };
        self.map.remove(k);
        self.size = try_sub!(&self.size, k.len());
        if let Some(vlen) = vlen {
            self.size = try_sub!(&self.size, res!(Self::valsize(vlen)));
        }
        res!(self.cwt.remove(k));
        Ok(true)
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

    /// Moves the cached location of a record a collection has carried to its new start, if the
    /// cache still names that record, and gives the location it named before.  A record is its
    /// key and its stamp: matched by file alone, a key with two records in the file, the older
    /// carried because its supersession had yet to arrive, had its location moved to the older
    /// record's and back, and the move entries were spent against the wrong offsets.
    pub fn reanchor(
        &mut self,
        k:      &Vec<u8>,
        loc:    &FileLocation,
        meta:   &Meta<UIDL, UID>,
    )
        -> Option<FileLocation>
    {
        match self.map.get_mut(k) {
            Some(CacheEntry::LocatedValue(mloc, _)) => {
                if mloc.floc.fnum == loc.fnum && mloc.meta == *meta {
                    let old_floc = mloc.floc.clone();
                    mloc.floc.start = loc.start;
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
    fn remove(&mut self, k: &[u8]) -> Outcome<()> {
        let hash = seahash::hash(k);
        if let Some(t) = self.rev.remove(&hash) {
            if self.fwd.remove(&t).is_some() {
                self.size = try_sub!(&self.size, self.bs + k.len());
            }
        }
        Ok(())
    }

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
