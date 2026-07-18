use crate::{
    prelude::*,
    file::floc::{
        DataLocation,
        FileLocation,
        FileNum,
    },
};

use std::{
    collections::BTreeMap,
};

/// Whether a file is present as one of a data/index pair or on its own.
#[derive(Clone, Debug)]
pub enum Present {
    /// Only one file of the pair is present, of the given type.
    Solo(FileType),
    /// Both the data file and its index file are present.
    Pair,
}

impl Default for Present {
    fn default() -> Self {
        Self::Pair
    }
}

/// The liveness of a stored value: whether it is the current version for its
/// key or has been superseded and awaits garbage collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataState {
    Cur, // Current version of value for this key.
    Old, // Value flagged for garbage collection.
}

impl std::fmt::Display for DataState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Cur => write!(f, "cur"),
            Self::Old => write!(f, "old"),
        }
    }
}

/// The bookkeeping the database keeps for one on-disk file: its sizes, whether
/// it is the live (currently-appended) file, how many records within it are old,
/// where each record starts, any pending garbage-collection moves, and the
/// current reader count.
#[derive(Clone, Debug, Default)]
pub struct FileState {
    present:    Present,
    dat_size:   usize,
    ind_size:   usize,
    live:       bool,
    oldsum:     u64,
    oldcnt:     usize,
    dmap:       BTreeMap<u64, DataState>, // Map of key-value pair starting positions in data file.
    mmap:       BTreeMap<u64, u64>, // Ephemeral map of the movement of starting positions due to gc.
    gc_active:  bool,
    readers:    usize,
}

impl FileState {

    // Getters.
    /// Returns whether the file is present solo or as a pair.
    pub fn present(&self)               -> &Present                         { &self.present }
    /// Returns the data file size in bytes.
    pub fn get_data_file_size(&self)    -> usize                            { self.dat_size }
    /// Returns the index file size in bytes.
    pub fn get_index_file_size(&self)   -> usize                            { self.ind_size }
    /// Returns whether this is the live file currently being appended to.
    pub fn is_live(&self)               -> bool                             { self.live }
    /// Returns the total size in bytes of old (superseded) records in the file.
    pub fn get_old_sum(&self)           -> u64                              { self.oldsum }
    /// Returns the map of record start positions to their liveness state.
    pub fn data_map(&self)              -> &BTreeMap<u64, DataState>        { &self.dmap }
    /// Returns a mutable reference to the record start-position map.
    pub fn data_map_mut(&mut self)      -> &mut BTreeMap<u64, DataState>    { &mut self.dmap }
    /// Returns whether garbage collection is currently active on the file.
    pub fn gc_active(&self)             -> bool                             { self.gc_active }
    /// Returns the current number of active readers of the file.
    pub fn readers(&self)               -> usize                            { self.readers }
    /// Returns whether the file has no active readers.
    pub fn no_readers(&self)            -> bool                             { self.readers == 0 }

    /// Returns the liveness state of the record starting at the given offset.
    pub fn get_data_state(&self, start: u64) -> Option<&DataState> {
        self.dmap.get(&start)
    }
    /// Provides mutable access to a data state entry.  Used during initialisation and state
    /// updates to modify entry states.
    pub fn get_data_state_mut(&mut self, start: u64) -> Option<&mut DataState> {
        self.dmap.get_mut(&start)
    }
    /// Returns every record start position, terminated by the data file size,
    /// so consecutive pairs bound each record.
    pub fn get_data_start_positions(&self) -> Outcome<Vec<u64>> {
        let mut starts = Vec::new();
        for start in self.dmap.keys() {
            starts.push(*start);
        }
        starts.push(res!(self.dat_size.try_into()));
        Ok(starts)
    }

    /// Returns whether every record in the file is old (none current).
    pub fn is_all_old(&self) -> bool {
        for (_, dstat) in &self.dmap {
            if *dstat == DataState::Cur {
                return false;
            }
        }
        true
    }

    // Setters.
    /// Replaces the record start-position map.
    pub fn set_data_map(&mut self, dmap: BTreeMap<u64, DataState>) {
        self.dmap = dmap;
    }
    /// Sets the data file size in bytes directly.
    pub fn set_data_file_size(&mut self, size: usize) {
        self.dat_size = size;
    }

    /// Updates the index file size directly.  Separate from data file size for accurate space
    /// tracking.
    pub fn set_index_file_size(&mut self, size: usize) {
        self.ind_size = size;
    }
    /// Sets whether this is the live file.
    pub fn set_live(&mut self, live: bool) {
        self.live = live;
    }
    /// Sets the present (solo or pair) status.
    pub fn set_present(&mut self, present: Present) {
        self.present = present;
    }
    /// Sets whether garbage collection is active on the file.
    pub fn set_gc(&mut self, active: bool) {
        self.gc_active = active;
    }
    /// Increments the reader count, erroring on overflow.
    pub fn inc_readers(&mut self) -> Outcome<()> {
        let (new, oflow) = self.readers().overflowing_add(1);
        if oflow {
            Err(err!(
                "Attempt to increment the number of readers for the file state. \
                The number, {}, is already at maximum.", self.readers();
            Bug, Overflow, Integer))
        } else {
            self.readers = new;
            Ok(())
        }
    }
    /// Decrements the reader count, erroring on underflow.
    pub fn dec_readers(&mut self) -> Outcome<()> {
        let (new, uflow) = self.readers().overflowing_sub(1);
        if uflow {
            Err(err!(
                "Attempt to decrement the number of readers for the file state. \
                The number, {}, is already at minimum.", self.readers();
            Bug, Underflow, Integer))
        } else {
            self.readers = new;
            Ok(())
        }
    }

    /// Clears all sizes, old-data accounting and record maps back to empty.
    pub fn reset(&mut self) {
        self.dat_size   = 0;
        self.ind_size   = 0;
        self.oldsum     = 0;
        self.oldcnt     = 0;
        self.dmap       = BTreeMap::new();
        self.mmap       = BTreeMap::new();
    }

    /// Zeroes the data file size, returning its previous value.
    pub fn reset_data_file_size(&mut self) -> usize {
        let dat_size = self.dat_size;
        self.dat_size = 0;
        dat_size
    }
    /// Zeroes the index file size, returning its previous value.
    pub fn reset_index_file_size(&mut self) -> usize {
        let ind_size = self.ind_size;
        self.ind_size = 0;
        ind_size
    }
    /// Zeroes the old-data sum and count.
    pub fn reset_old_accounting(&mut self) {
        self.oldsum = 0;
        self.oldcnt = 0;
    }

    // Queries.
    /// Returns whether every record is accounted old (count matches map size).
    pub fn is_all_data_old(&self) -> bool {
        self.oldcnt == self.dmap.len()
    }

    /// Returns whether there are no pending garbage-collection moves.
    pub fn no_pending_moves(&self) -> bool {
        self.mmap.len() == 0
    }

    /// Returns whether the record map is empty.
    pub fn data_map_empty(&self) -> bool {
        self.dmap.len() == 0
    }

    /// Returns the number of records tracked in the file.
    pub fn data_map_len(&self) -> usize {
        self.dmap.len()
    }

    /// Returns the number of pending garbage-collection moves.
    pub fn move_map_len(&self) -> usize {
        self.mmap.len()
    }

    // Data map mutation - this is where the size of old data and the file are modified.

    /// Records a newly written record at its location, growing the tracked data
    /// and index file sizes, and returns the combined byte growth.
    pub fn insert_new(
        &mut self,
        floc:   &FileLocation,
        ilen:   usize, // encoded index length
    ) 
        -> Outcome<usize>
    {
        self.dmap.insert(floc.start, DataState::Cur);
        let dat_len = try_into!(usize, floc.klen + floc.vlen);
        match self.dat_size.checked_add(dat_len) {
            Some(sum) => self.dat_size = sum,
            None => {
                let old_dat_size = self.dat_size;
                self.dat_size = usize::MAX;
                return Err(err!(
                    "When inserting the new {:?}, the data file size, {}, overflowed. \
                    It has been set to the maximum {}.", floc, old_dat_size, usize::MAX;
                Bug, Overflow, Integer));
            },
        }
        let ind_len = try_into!(usize, floc.klen) + ilen;
        match self.ind_size.checked_add(ind_len) {
            Some(sum) => self.ind_size = sum,
            None => {
                let old_ind_size = self.ind_size;
                self.ind_size = usize::MAX;
                return Err(err!(
                    "When inserting the new {:?}, the index file size, {}, overflowed. \
                    It has been set to the maximum {}.", floc, old_ind_size, usize::MAX;
                Bug, Overflow, Integer));
            },
        }
        match dat_len.checked_add(ind_len) {
            Some(sum) => Ok(sum),
            None => Err(err!(
                "When inserting the new {:?}, the sum of the data file size, {}, \
                and the index file size, {}, overflowed.",
                floc, self.dat_size, self.ind_size;
            Bug, Overflow, Integer)),
        }
    }

    /// Grows the tracked index file size by `len` bytes, erroring on overflow.
    pub fn inc_index_file_size(
        &mut self,
        len: usize,
    )
        -> Outcome<usize>
    {
        match self.ind_size.checked_add(len) {
            Some(sum) => self.ind_size = sum,
            None => {
                let old_ind_size = self.ind_size;
                self.ind_size = usize::MAX;
                return Err(err!(
                    "When incrementing the size of the index file, {}, by {}, \
                    an overflow occurred. It has been set to the maximum {}.",
                    old_ind_size, len, usize::MAX;
                Bug, Overflow, Integer));
            },
        }
        Ok(len)
    }

    /// Records that a record has been relocated during garbage collection,
    /// mapping its old start to its new one and removing the old record entry.
    pub fn update_moved(
        &mut self,
        dloc:       &DataLocation,
        new_start:  u64,
    ) {
        self.mmap.insert(dloc.start, new_start);
        self.dmap.remove(&dloc.start);
    }

    /// Marks a current record as old, updating the old-data sum and count,
    /// erroring if it was already old or is not present.
    pub fn register_old(
        &mut self,
        dloc: &DataLocation,
    )
        -> Outcome<()>
    {
        match self.dmap.insert(dloc.start, DataState::Old) {
            Some(DataState::Cur) => (),
            Some(DataState::Old) => {
                return Err(err!(
                    "{:?} has already been marked as old.", dloc;
                Bug, Mismatch, Data));
            }
            None => return Err(err!(
                "While attempting to flag {:?} as old, a data entry starting \
                at position {} in the FileState was not found.", dloc, dloc.start;
            Bug, Missing, Data)),
        }
        match self.oldsum.checked_add(dloc.len) {
            Some(sum) => self.oldsum = sum,
            None => {
                let old_oldsum = self.oldsum;
                self.oldsum = u64::MAX;
                return Err(err!(
                    "When registering the old {:?}, the sum of old data sizes, \
                    {} overflowed. It has been set to the maximum {}.",
                    dloc, old_oldsum, u64::MAX;
                Bug, Overflow, Integer));
            },
        }
        match self.oldcnt.checked_add(1) {
            Some(sum) => self.oldcnt = sum,
            None => {
                let old_oldcnt = self.oldcnt;
                self.oldcnt = usize::MAX;
                return Err(err!(
                    "When registering the old {:?}, the count of old data entries, \
                    {} overflowed. It has been set to the maximum {}.",
                    dloc, old_oldcnt, usize::MAX;
                Bug, Overflow, Integer));
            },
        }
        Ok(())
    }

    /// Removes an old record once its bytes have been reclaimed, shrinking the
    /// data file size and old-data accounting, and returns the reclaimed length.
    pub fn retire_old(
        &mut self,
        dloc: &DataLocation,
    )
        -> Outcome<usize>
    {
        self.dmap.remove(&dloc.start);
        let dat_len = try_into!(usize, dloc.len);
        if self.dat_size >= dat_len {
            self.dat_size -= dat_len;
        } else {
            return Err(err!(
                "While retiring {:?} from {:?}, the data file size, {}, will become negative.",
                dloc, self, self.dat_size;
            Bug, Underflow, Integer));
        }
        if self.oldsum >= dloc.len {
            self.oldsum -= dloc.len;
        } else {
            return Err(err!(
                "While retiring {:?} from {:?}, oldsum, {}, will become negative.",
                dloc, self, self.oldsum;
            Bug, Underflow, Integer));
        }
        if self.oldcnt > 0 {
            self.oldcnt -= 1;
        } else {
            return Err(err!(
                "While retiring {:?} from {:?}, oldcnt, {}, will become negative.",
                dloc, self, self.oldcnt;
            Bug, Underflow, Integer));
        }
        Ok(dat_len)
    }

    /// The move map maps old -> new.  If there is no old, nothing is done and `None` is returned.
    /// Otherwise this method removes old and returns `Some(new)`.
    pub fn delete_move_entry(
        &mut self,
        dloc: &DataLocation,
    )
        -> Option<u64>
    {
        self.mmap.remove(&dloc.start)
    }

    /// Consumes a pending move: removes the old->new entry and reinstates the
    /// record as current at its new start, returning that new start if present.
    pub fn map_and_remove(
        &mut self,
        dloc: &DataLocation,
    )
        -> Option<u64>
    {
        if let Some(new_start) = self.mmap.remove(&dloc.start) {
            self.dmap.insert(new_start, DataState::Cur);
            Some(new_start)
        } else {
            None
        }
    }
}

/// A portion, or shard of the FileState data for a zone.
#[derive(Clone, Debug, Default)]
pub struct FileStateMap {
    map:    BTreeMap<FileNum, FileState>,
    size:   usize, // Sum of all data and index file sizes for this shard.
}

impl FileStateMap {
    /// Returns the map of file numbers to their states.
    pub fn map(&self) -> &BTreeMap<FileNum, FileState> { &self.map }
    /// Returns a mutable reference to the file-number-to-state map.
    pub fn map_mut(&mut self) -> &mut BTreeMap<FileNum, FileState> { &mut self.map }

    /// Returns the state of the given file, erroring if absent.
    pub fn get_state(&self, fnum: FileNum) -> Outcome<&FileState> {
        match self.map.get(&fnum) {
            Some(fstat) => Ok(fstat),
            None => Err(err!(
                "No state entry for file number {}.", fnum;
            Bug, Missing, Data)),
        }
    }

    /// Returns a mutable reference to the given file's state, erroring if absent.
    pub fn get_state_mut(&mut self, fnum: FileNum) -> Outcome<&mut FileState> {
        match self.map.get_mut(&fnum) {
            Some(fstat) => Ok(fstat),
            None => Err(err!(
                "No state entry for file number {}.", fnum;
            Bug, Missing, Data)),
        }
    }

    /// Sets the total tracked byte size of the shard.
    pub fn set_size(&mut self, size: usize) { self.size = size; }
    /// Grows the total tracked byte size by `len`, erroring on overflow.
    pub fn inc_size(&mut self, len: usize) -> Outcome<()> {
        self.size = try_add!(&self.size, len);
        Ok(())
    }
    /// Shrinks the total tracked byte size by `len`, erroring on underflow.
    pub fn dec_size(&mut self, len: usize) -> Outcome<()> {
        self.size = try_sub!(&self.size, len);
        Ok(())
    }
    /// Returns the total tracked byte size of the shard.
    pub fn get_size(&self) -> usize { self.size }

    /// Returns the shard index a file number falls into, given `nf` shards.
    #[inline]
    pub fn shard_index(fnum: FileNum, nf: usize) -> usize { (fnum as usize) % nf }

    /// Inserts a state entry for the given file number.
    pub fn new_file_state(&mut self, fnum: FileNum, fs: FileState) {
        self.map.insert(fnum, fs);
    }

    /// Registers a new live file with the given initial data and index sizes.
    pub fn new_live_file(
        &mut self,
        num:        FileNum,
        dat_size:   u64,
        ind_size:   u64,
    ) {
        self.new_file_state(num, FileState {
            dat_size:   dat_size as usize,
            ind_size:   ind_size as usize,
            live:       true,
            ..Default::default()
        });
    }

    /// Records a newly written record into the appropriate file's state,
    /// creating the state if needed, and grows the shard's total size.
    pub fn insert_new(
        &mut self,
        floc:   &FileLocation,
        ilen:   usize,
    )
        -> Outcome<()>
    {
        if self.map.get(&floc.file_number()).is_none() {
            self.new_file_state(floc.file_number(), FileState::default());
        }
        let fstat = ok!(self.get_state_mut(floc.file_number()));
        let len = res!(fstat.insert_new(&floc, ilen));
        self.inc_size(len)
    }
}
