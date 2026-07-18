use crate::prelude::*;

use oxedyne_fe2o3_core::{
    byte::{
        FromBytes,
        ToBytes,
    },
    mem::Extract,
};
use oxedyne_fe2o3_jdat::{
    tup3dat,
    try_extract_dat_as,
    try_extract_tup3dat,
    daticle::Dat,
};
use oxedyne_fe2o3_iop_hash::csum::Checksummer;

use std::io;

/// Sequential number identifying a data or index file within a zone.
pub type FileNum = u32;

/// A byte range within a file: a start offset and a length.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DataLocation {
    /// Start offset in bytes.
    pub start:  u64,
    /// Length in bytes.
    pub len:    u64,
}

/// The location of a stored key-value record: which file it is in, where it
/// starts, and the key and value lengths.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct FileLocation {
    /// Number of the file holding the record.
    pub fnum:   FileNum,
    /// Start offset of the record in the file.
    pub start:  u64,
    /// Key length in bytes (used when deleting the pair).
    pub klen:   u64, // for deleting kv pair
    /// Value length in bytes (used when reading the value).
    pub vlen:   u64, // for reading value
}

impl FileLocation {

    /// Returns the number of the file holding the record.
    pub fn file_number(&self) -> FileNum { self.fnum }

    /// Returns the byte range spanning both the key and the value.
    pub fn keyval(&self) -> DataLocation {
        DataLocation {
            start:  self.start,
            len:    self.klen + self.vlen,
        }
    }
    /// Returns the byte range spanning just the value.
    pub fn val(&self) -> DataLocation {
        DataLocation {
            start:  self.start + self.klen,
            len:    self.vlen,
        }
    }

}

/// A [`FileLocation`] together with its encoded byte form (including checksum),
/// as it is written into an index file.
#[derive(Clone, Debug, Default)]
pub struct StoredFileLocation {
    /// The decoded file location.
    pub floc:   FileLocation,
    /// The encoded bytes, including the trailing checksum.
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

    /// Returns the file location by reference.
    pub fn ref_file_location(&self) -> &FileLocation { &self.floc }
    /// Consumes self and returns the file location.
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
