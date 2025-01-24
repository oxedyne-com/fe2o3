use crate::prelude::*;

use oxedize_fe2o3_core::{
    byte::{
        FromBytes,
        ToBytes,
    },
    mem::Extract,
};
use oxedize_fe2o3_jdat::{
    tup3dat,
    try_extract_dat_as,
    try_extract_tup3dat,
    daticle::Dat,
};
use oxedize_fe2o3_iop_hash::csum::Checksummer;

use std::io;

pub type FileNum = u32;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DataLocation {
    pub start:  u64,
    pub len:    u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct FileLocation {
    pub fnum:   FileNum,
    pub start:  u64,
    pub klen:   u64, // for deleting kv pair
    pub vlen:   u64, // for reading value
}

impl FileLocation {
    
    pub fn file_number(&self) -> FileNum { self.fnum }

    pub fn keyval(&self) -> DataLocation {
        DataLocation {
            start:  self.start,
            len:    self.klen + self.vlen,
        }
    }
    pub fn val(&self) -> DataLocation {
        DataLocation {
            start:  self.start + self.klen,
            len:    self.vlen,
        }
    }

}

#[derive(Clone, Debug, Default)]
pub struct StoredFileLocation {
    pub floc:   FileLocation,
    pub buf:    Vec<u8>, // encoding including checksum
}

impl StoredFileLocation {
    
    /// The encoding process is built into the creation of a new `FileLocation`, to avoid
    /// extra work calculating the encoded size when fbots keep track of index file sizes.
    pub fn new<
        C: Checksummer,
    >(
        fnum:       FileNum,
        start:      u64,
        klen:       u64,
        vlen:       u64,
        csummer:    C,
    )
        -> Outcome<Self>
    {
        let mut buf = Vec::new();
        let list = tup3dat![
            Dat::u64dat(start),
            Dat::u64dat(klen),
            Dat::u64dat(vlen),
        ];
        buf = res!(list.to_bytes(buf));
        (buf, _) = res!(csummer.append(buf));

        Ok(Self {
            floc: FileLocation {
                fnum,
                start,
                klen,
                vlen,
            },
            buf,
        })
    }

    pub fn ref_file_location(&self) -> &FileLocation { &self.floc }
    pub fn own_file_location(self) -> FileLocation { self.floc }

    /// Read `FileLocation` bytes, calculating but not reading the checksum.
    pub fn read_bytes<
        C: Checksummer,
        R: io::Read,
    >(
        r:          &mut R,
        fnum:       FileNum,
        csummer:    C,
    )
        -> Outcome<(Self, usize, Vec<u8>)>
    {
        let buf = res!(Dat::load_bytes(r));
        let (dat, n) = res!(Dat::from_bytes(&buf));
        let (buf, csum) = res!(csummer.append(buf));
        let mut v = try_extract_tup3dat!(dat);
        let start = try_extract_dat_as!(v[0].extract(), u64, U8, U16, U32, U64);
        let klen =  try_extract_dat_as!(v[1].extract(), u64, U8, U16, U32, U64);
        let vlen =  try_extract_dat_as!(v[2].extract(), u64, U8, U16, U32, U64);

        Ok((
            Self {
                floc: FileLocation {
                    fnum,
                    start,
                    klen,
                    vlen,
                },
                buf,
            },
            n,
            csum,
        ))
    }
}
