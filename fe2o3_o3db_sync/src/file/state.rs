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

#[derive(Clone, Debug)]
pub enum Present {
    Solo(FileType),
    Pair,
}

impl Default for Present {
    fn default() -> Self {
        Self::Pair
    }
}

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
    pub fn present(&self)               -> &Present                         { &self.present }
    pub fn get_data_file_size(&self)    -> usize                            { self.dat_size }
    pub fn get_index_file_size(&self)   -> usize                            { self.ind_size }
    pub fn is_live(&self)               -> bool                             { self.live }
    pub fn get_old_sum(&self)           -> u64                              { self.oldsum }
    pub fn data_map(&self)              -> &BTreeMap<u64, DataState>        { &self.dmap }
    pub fn data_map_mut(&mut self)      -> &mut BTreeMap<u64, DataState>    { &mut self.dmap }
    pub fn gc_active(&self)             -> bool                             { self.gc_active }
    pub fn readers(&self)               -> usize                            { self.readers }
    pub fn no_readers(&self)            -> bool                             { self.readers == 0 }

    pub fn get_data_state(&self, start: u64) -> Option<&DataState> {
        self.dmap.get(&start)
    }
    /// Provides mutable access to a data state entry.  Used during initialisation and state
    /// updates to modify entry states.
    pub fn get_data_state_mut(&mut self, start: u64) -> Option<&mut DataState> {
        self.dmap.get_mut(&start)
    }
    pub fn get_data_start_positions(&self) -> Outcome<Vec<u64>> {
        let mut starts = Vec::new();
        for start in self.dmap.keys() {
            starts.push(*start);
        }
        starts.push(res!(self.dat_size.try_into()));
        Ok(starts)
    }

    pub fn is_all_old(&self) -> bool {
        for (_, dstat) in &self.dmap {
            if *dstat == DataState::Cur {
                return false;
            }
        }
        true
    }

    // Setters.
    pub fn set_data_map(&mut self, dmap: BTreeMap<u64, DataState>) {
        self.dmap = dmap;
    }
    pub fn set_data_file_size(&mut self, size: usize) {
        self.dat_size = size;
    }

    /// Updates the index file size directly.  Separate from data file size for accurate space
    /// tracking.
    pub fn set_index_file_size(&mut self, size: usize) {
        self.ind_size = size;
    }
    pub fn set_live(&mut self, live: bool) {
        self.live = live;
    }
    pub fn set_present(&mut self, present: Present) {
        self.present = present;
    }
    pub fn set_gc(&mut self, active: bool) {
        self.gc_active = active;
    }
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

    pub fn reset(&mut self) {
        self.dat_size   = 0;
        self.ind_size   = 0;
        self.oldsum     = 0;
        self.oldcnt     = 0;
        self.dmap       = BTreeMap::new();
        self.mmap       = BTreeMap::new();
    }

    pub fn reset_data_file_size(&mut self) -> usize {
        let dat_size = self.dat_size;
        self.dat_size = 0;
        dat_size
    }
    pub fn reset_index_file_size(&mut self) -> usize {
        let ind_size = self.ind_size;
        self.ind_size = 0;
        ind_size
    }
    pub fn reset_old_accounting(&mut self) {
        self.oldsum = 0;
        self.oldcnt = 0;
    }

    // Queries.
    pub fn is_all_data_old(&self) -> bool {
        self.oldcnt == self.dmap.len()
    }

    pub fn no_pending_moves(&self) -> bool {
        self.mmap.len() == 0
    }

    pub fn data_map_empty(&self) -> bool {
        self.dmap.len() == 0
    }

    pub fn data_map_len(&self) -> usize {
        self.dmap.len()
    }

    pub fn move_map_len(&self) -> usize {
        self.mmap.len()
    }

    // Data map mutation - this is where the size of old data and the file are modified.
    
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

    pub fn update_moved(
        &mut self,
        dloc:       &DataLocation,
        new_start:  u64,
    ) {
        self.mmap.insert(dloc.start, new_start);
        self.dmap.remove(&dloc.start);
    }

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
    pub fn map(&self) -> &BTreeMap<FileNum, FileState> { &self.map }
    pub fn map_mut(&mut self) -> &mut BTreeMap<FileNum, FileState> { &mut self.map }

    pub fn get_state(&self, fnum: FileNum) -> Outcome<&FileState> {
        match self.map.get(&fnum) {
            Some(fstat) => Ok(fstat),
            None => Err(err!(
                "No state entry for file number {}.", fnum;
            Bug, Missing, Data)),
        }
    }

    pub fn get_state_mut(&mut self, fnum: FileNum) -> Outcome<&mut FileState> {
        match self.map.get_mut(&fnum) {
            Some(fstat) => Ok(fstat),
            None => Err(err!(
                "No state entry for file number {}.", fnum;
            Bug, Missing, Data)),
        }
    }

    pub fn set_size(&mut self, size: usize) { self.size = size; }
    pub fn inc_size(&mut self, len: usize) -> Outcome<()> {
        self.size = try_add!(&self.size, len);
        Ok(())
    }
    pub fn dec_size(&mut self, len: usize) -> Outcome<()> {
        self.size = try_sub!(&self.size, len);
        Ok(())
    }
    pub fn get_size(&self) -> usize { self.size }

    #[inline]
    pub fn shard_index(fnum: FileNum, nf: usize) -> usize { (fnum as usize) % nf }

    pub fn new_file_state(&mut self, fnum: FileNum, fs: FileState) {
        self.map.insert(fnum, fs);
    }

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
