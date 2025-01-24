use crate::{
    prelude::*,
    file::floc::FileNum,
};

use std::{
    collections::BTreeMap,
    fs::File,
    sync::{
        Arc,
        RwLock,
    },
    time::{
        Duration,
        Instant,
    },
};

#[derive(Debug)]
pub struct FileCacheEntry {
    pub t:      Instant,
    pub file:   Arc<RwLock<File>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FileCacheIndex {
    pub fnum:   FileNum,
    pub typ:    FileType,
}

#[derive(Debug, Default)]
pub struct FileCache {
    map: BTreeMap<FileCacheIndex, FileCacheEntry>,
    exp: Duration, // expiry duration
}

impl FileCache {
    pub fn new(expiry_secs: u64) -> Self {
        Self {
            map: BTreeMap::new(),
            exp: Duration::from_secs(expiry_secs),
        }
    }

    pub fn ref_map(&self)       -> &BTreeMap<FileCacheIndex, FileCacheEntry>        { &self.map }
    pub fn mut_map(&mut self)   -> &mut BTreeMap<FileCacheIndex, FileCacheEntry>    { &mut self.map }
    pub fn expiry(&self)        -> &Duration                                        { &self.exp }
    pub fn len(&self)           -> usize                                            { self.map.len() }

    pub fn insert(
        &mut self,
        fnum:   FileNum,
        typ:    &FileType,
        file:   Arc<RwLock<File>>,
    ) {
        self.map.insert(
            FileCacheIndex{ fnum, typ: typ.clone() },
            FileCacheEntry {
                t:      Instant::now(),
                file:   file,
            },
        );
    }
}
