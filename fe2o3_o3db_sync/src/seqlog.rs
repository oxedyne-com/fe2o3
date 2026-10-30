//! A numbered append log over point lookups (30 Oct 2026, first caller Daimond's per-chat log).
//!
//! An Ozone store cannot prefix-scan on a request path: a walk of a whole prefix on a
//! large store takes minutes. So a log here is one INDEX record naming every batch it
//! holds, plus one record per appended batch, and every read and write is a point lookup.
//! The store assigns the sequence numbers: a batch of `n` rows appended at head `h` holds
//! `h+1..=h+n`, contiguous, and nothing a caller sends can choose them.
//!
//! The log takes no lock of its own. Every mutating call is a read-modify-write of the
//! index, so the caller serialises the calls on one log (one mutex per log key); reads may
//! run beside an append, because a batch is written before the index that names it.
//!
//! Snapshots. A caller that has folded the log to some seq stores that fold elsewhere and
//! registers it here as a [`SnapRef`], carried in the index. A trim never cuts a row the
//! registered snapshot does not cover, and with no snapshot it cuts nothing, so a reader
//! that finds the rows above its cursor gone always gets a snapshot to resume from (the
//! Kafka-compaction and Matrix-state shape).
//!
//! Crash order. An append writes the batch, then the index: a crash between leaves a
//! batch no index names, at the key the next append overwrites. A trim deletes the
//! batches, then rewrites the index: a crash between leaves an index naming a batch that
//! is gone, which a reader answers as a gap. Nothing is ever unreachable and still held.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    map::create_dat_ordmap,
};

use std::{
    collections::HashMap,
    sync::{
        Arc,
        Mutex,
    },
};

/// The three point operations a log needs of a store.
pub trait SeqStore {
    fn get(&self, key: &Dat) -> Outcome<Option<Dat>>;
    fn put(&self, key: Dat, val: Dat) -> Outcome<()>;
    fn delete(&self, key: &Dat) -> Outcome<bool>;
}

/// One appended batch, as the index names it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRef {
    pub first:  u64,    // seq of the batch's first row
    pub last:   u64,    // seq of its last row
    pub at:     i64,    // when it was appended, in the caller's clock
    pub bytes:  u64,    // the caller's measure of its size
}

/// A registered snapshot: the log folded through `seq`, stored by the caller, who names it
/// in `ext` (chunk addresses, fingerprint, epoch), carried whole and never read here.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapRef {
    pub seq:    u64,    // the last seq the snapshot covers, 0 for an empty fold
    pub ext:    Dat,    // the caller's description of it
}

/// The index record: where the log stands, which batches it holds, its snapshot, and the
/// caller's own fields in `ext`, carried whole and never read here.
#[derive(Clone, Debug)]
pub struct SeqIndex {
    pub head:       u64,                // last seq assigned, 0 for a new log
    pub low:        u64,                // lowest seq still held, `head + 1` when none is
    pub batches:    Vec<BatchRef>,      // oldest first
    pub snap:       Option<SnapRef>,    // the registered snapshot, if any
    pub ext:        Dat,                // the caller's fields, `Dat::Empty` when none
}

impl Default for SeqIndex {
    fn default() -> Self {
        Self { head: 0, low: 1, batches: Vec::new(), snap: None, ext: Dat::Empty }
    }
}

impl SeqIndex {
    /// The sum of the held batches' sizes.
    pub fn bytes(&self) -> u64 {
        self.batches.iter().map(|b| b.bytes).sum()
    }

    pub fn to_dat(&self) -> Dat {
        let batches = self.batches.iter().map(|b| Dat::List(vec![
            Dat::U64(b.first),
            Dat::U64(b.last),
            Dat::I64(b.at),
            Dat::U64(b.bytes),
        ])).collect();
        let mut fields = vec![
            (dat!("head"),      Dat::U64(self.head)),
            (dat!("low"),       Dat::U64(self.low)),
            (dat!("batches"),   Dat::List(batches)),
            (dat!("ext"),       self.ext.clone()),
        ];
        if let Some(s) = &self.snap {
            fields.push((dat!("snap"), Dat::List(vec![Dat::U64(s.seq), s.ext.clone()])));
        }
        create_dat_ordmap(fields)
    }

    pub fn from_dat(d: &Dat) -> Outcome<Self> {
        let head = res!(d.map_get_u64(&dat!("head")));
        let low = res!(d.map_get_u64(&dat!("low")));
        let mut batches = Vec::new();
        for (i, b) in res!(d.map_get_list(&dat!("batches"))).iter().enumerate() {
            let parts = match b {
                Dat::List(p) if p.len() == 4 => p,
                _ => return Err(err!(
                    "seqlog: batch entry {} of the index is not a four-part list.", i;
                    Invalid, Input)),
            };
            let num = |j: usize| parts[j].get_u64().ok_or_else(|| err!(
                "seqlog: part {} of batch entry {} is not a non-negative integer.", j, i;
                Invalid, Input));
            batches.push(BatchRef {
                first:  res!(num(0)),
                last:   res!(num(1)),
                at:     res!(parts[2].get_i64().ok_or_else(|| err!(
                            "seqlog: the time of batch entry {} is not an integer.", i;
                            Invalid, Input))),
                bytes:  res!(num(3)),
            });
        }
        let ext = match res!(d.map_get(&dat!("ext"))) {
            Some(e) => e.clone(),
            None    => Dat::Empty,
        };
        // An index written before snapshots existed has no "snap": it loads as none.
        let snap = match res!(d.map_get(&dat!("snap"))) {
            None                                => None,
            Some(Dat::List(p)) if p.len() == 2  => Some(SnapRef {
                seq:    res!(p[0].get_u64().ok_or_else(|| err!(
                            "seqlog: the snap seq of the index is not a non-negative integer.";
                            Invalid, Input))),
                ext:    p[1].clone(),
            }),
            Some(_) => return Err(err!(
                "seqlog: the snap of the index is not a two-part list."; Invalid, Input)),
        };
        Ok(Self { head, low, batches, snap, ext })
    }
}

/// What a read finds above a cursor.
#[derive(Clone, Debug)]
pub enum SeqRead {
    Gap { low: u64, snap: Option<SnapRef> },    // rows above the cursor are gone; resume from `snap`
    Rows { rows: Vec<(u64, Dat)>, more: bool }, // `(seq, row)` in order; `more` when the limit stopped it
}

/// What a snapshot registration did.
#[derive(Clone, Debug, PartialEq)]
pub enum SnapReg {
    Registered { prev: Option<SnapRef> },   // stored; `prev` is the one it supersedes
    Same,                                   // the snapshot already registered: nothing written
    Behind { least: u64 },                  // refused: the lowest seq that would be accepted
    Ahead { head: u64 },                    // refused: beyond the log's head
    Conflict { snap: SnapRef },             // refused: another snapshot holds this seq
}

/// One log: its index key and the prefix its batch keys take.
#[derive(Clone, Debug)]
pub struct SeqLog {
    index:  Dat,
    prefix: String,
}

impl SeqLog {
    /// A log whose index is at `index_key` and whose batches are at `batch_prefix` followed
    /// by the batch's first seq.
    pub fn new(index_key: &str, batch_prefix: &str) -> Self {
        Self { index: dat!(index_key.to_string()), prefix: batch_prefix.to_string() }
    }

    pub fn index_key(&self) -> &Dat { &self.index }

    pub fn batch_key(&self, first: u64) -> Dat {
        dat!(fmt!("{}{}", self.prefix, first))
    }

    /// The index, or a new log's when none is stored.
    pub fn load<S: SeqStore>(&self, store: &S) -> Outcome<SeqIndex> {
        match res!(store.get(&self.index)) {
            Some(d) => SeqIndex::from_dat(&d),
            None    => Ok(SeqIndex::default()),
        }
    }

    /// Store the index as it stands, for a change to `ext` alone.
    pub fn save<S: SeqStore>(&self, store: &S, idx: &SeqIndex) -> Outcome<()> {
        store.put(self.index.clone(), idx.to_dat())
    }

    /// Append `rows` as one batch, answering the seq of its first row. The caller holds
    /// the log's lock. An empty batch writes nothing and answers `head + 1`.
    pub fn append<S: SeqStore>(
        &self,
        store:  &S,
        idx:    &mut SeqIndex,
        rows:   Vec<Dat>,
        at:     i64,
        bytes:  u64,
    )
        -> Outcome<u64>
    {
        let first = idx.head + 1;
        if rows.is_empty() {
            return Ok(first);
        }
        let last = idx.head + rows.len() as u64;
        let batch = create_dat_ordmap(vec![
            (dat!("first"), Dat::U64(first)),
            (dat!("rows"),  Dat::List(rows)),
        ]);
        res!(store.put(self.batch_key(first), batch));
        if idx.batches.is_empty() {
            idx.low = first;
        }
        idx.head = last;
        idx.batches.push(BatchRef { first, last, at, bytes });
        res!(self.save(store, idx));
        Ok(first)
    }

    /// The rows above `since` (the last seq the reader holds), at most `max_rows` of them.
    /// A cursor below what the log still holds answers [`SeqRead::Gap`].
    pub fn read<S: SeqStore>(&self, store: &S, idx: &SeqIndex, since: u64, max_rows: usize)
        -> Outcome<SeqRead>
    {
        if since >= idx.head {
            return Ok(SeqRead::Rows { rows: Vec::new(), more: false });
        }
        if since + 1 < idx.low {
            return Ok(SeqRead::Gap { low: idx.low, snap: idx.snap.clone() });
        }
        let mut rows = Vec::new();
        for b in idx.batches.iter().filter(|b| b.last > since) {
            let rec = match res!(store.get(&self.batch_key(b.first))) {
                Some(d) => d,
                // Deleted by a trim whose index write did not land: the rows are gone.
                None    => return Ok(SeqRead::Gap { low: b.last + 1, snap: idx.snap.clone() }),
            };
            let list = res!(rec.map_get_list(&dat!("rows")));
            for (i, row) in list.iter().enumerate() {
                let seq = b.first + i as u64;
                if seq <= since {
                    continue;
                }
                if rows.len() >= max_rows {
                    return Ok(SeqRead::Rows { rows, more: true });
                }
                rows.push((seq, row.clone()));
            }
        }
        Ok(SeqRead::Rows { rows, more: false })
    }

    /// Register a snapshot of the log folded through `seq`. The caller holds the log's lock.
    /// It must lie above the current snapshot, at or below the head, and no lower than the
    /// row before `low`, so that the rows above it are all still held; seq 0 is accepted
    /// only on an empty log. A repeat of the registered snapshot answers [`SnapReg::Same`].
    pub fn register_snap<S: SeqStore>(&self, store: &S, idx: &mut SeqIndex, seq: u64, ext: Dat)
        -> Outcome<SnapReg>
    {
        if let Some(s) = &idx.snap {
            if seq == s.seq {
                return Ok(if s.ext == ext { SnapReg::Same } else { SnapReg::Conflict { snap: s.clone() } });
            }
        }
        if seq > idx.head {
            return Ok(SnapReg::Ahead { head: idx.head });
        }
        let least = match &idx.snap {
            Some(s)                 => s.seq + 1,
            None if idx.head == 0   => 0,
            None                    => 1,
        };
        let least = std::cmp::max(least, idx.low.saturating_sub(1));
        if seq < least {
            return Ok(SnapReg::Behind { least });
        }
        let prev = idx.snap.replace(SnapRef { seq, ext });
        res!(self.save(store, idx));
        Ok(SnapReg::Registered { prev })
    }

    /// Drop the oldest batches while `drop` holds for them, answering how many went. Only
    /// batches the registered snapshot covers are ever dropped, and with no snapshot none
    /// is; `drop` sees the index, snapshot included, for the caller's own bounds. The
    /// caller holds the log's lock.
    pub fn trim<S, F>(&self, store: &S, idx: &mut SeqIndex, drop: F) -> Outcome<usize>
    where
        S: SeqStore,
        F: Fn(&BatchRef, &SeqIndex) -> bool,
    {
        let cover = match &idx.snap {
            Some(s) => s.seq,
            None    => return Ok(0),
        };
        let mut n = 0;
        while n < idx.batches.len()
            && idx.batches[n].last <= cover
            && drop(&idx.batches[n], idx)
        {
            n += 1;
        }
        if n == 0 {
            return Ok(0);
        }
        for b in &idx.batches[..n] {
            res!(store.delete(&self.batch_key(b.first)));
        }
        idx.batches.drain(..n);
        idx.low = match idx.batches.first() {
            Some(b) => b.first,
            None    => idx.head + 1,
        };
        res!(self.save(store, idx));
        Ok(n)
    }

    /// Drop the oldest batches holding seqs up to and including `seq`, within the bound
    /// [`SeqLog::trim`] keeps.
    pub fn truncate_through<S: SeqStore>(&self, store: &S, idx: &mut SeqIndex, seq: u64)
        -> Outcome<usize>
    {
        self.trim(store, idx, |b, _| b.last <= seq)
    }

    /// Delete every batch and the index.
    pub fn purge<S: SeqStore>(&self, store: &S, idx: &SeqIndex) -> Outcome<()> {
        for b in &idx.batches {
            res!(store.delete(&self.batch_key(b.first)));
        }
        res!(store.delete(&self.index));
        Ok(())
    }
}

/// One mutex per log key, held only while some caller is inside it, so the map is bounded
/// by the logs in use rather than every log ever touched.
#[derive(Default)]
pub struct SeqLocks {
    map: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SeqLocks {
    pub fn new() -> Self { Self::default() }

    /// Run `f` holding the lock for `key`.
    pub fn with<T, F>(&self, key: &str, f: F) -> Outcome<T>
    where
        F: FnOnce() -> Outcome<T>,
    {
        let one = {
            let mut map = lock_mutex!(self.map);
            map.entry(key.to_string()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
        };
        let out = {
            let _held = lock_mutex!(one);
            f()
        };
        drop(one);
        // The entry goes once the map holds the only handle: nobody waits on it.
        let mut map = lock_mutex!(self.map);
        if map.get(key).map(|m| Arc::strong_count(m) == 1).unwrap_or(false) {
            map.remove(key);
        }
        out
    }

    /// How many keys are currently held or waited on.
    pub fn len(&self) -> Outcome<usize> {
        Ok(lock_mutex!(self.map).len())
    }
}
