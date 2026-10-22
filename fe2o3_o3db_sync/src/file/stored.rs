use crate::{
    prelude::*,
    data::{
        core::Key,
    },
    file::floc::{
        FileLocation,
        FileNum,
        StoredFileLocation,
    },
};

use oxedyne_fe2o3_core::{
    byte::ToBytes,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedyne_fe2o3_hash::csum::ChecksumScheme;
use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_iop_hash::csum::Checksummer;

use std::{
    io::{
        self,
        SeekFrom,
    },
};

/// Names one record, by a digest of its key and the stamp it was written with.  An offset names a
/// record only within one generation of its file: a collection moves the records it keeps, and
/// with records of one size a new offset can equal an old one still in use elsewhere, so a
/// location matched by offset alone can be taken for another record (2026-09-23).
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct RecordDigest(u64);

impl RecordDigest {

    pub fn new<
        const UIDL: usize,
        UID: NumIdDat<UIDL>,
    >(
        key:    &[u8],
        meta:   &Meta<UIDL, UID>,
    )
        -> Outcome<Self>
    {
        // The key bytes are a daticle, which ends itself, so the stamp cannot run into it.
        let mut buf = Vec::with_capacity(key.len() + Meta::<UIDL, UID>::BYTE_LEN);
        buf.extend_from_slice(key);
        buf = res!(meta.to_bytes(buf));
        Ok(Self(seahash::hash(&buf)))
    }
}

#[derive(Debug)]
pub struct StoredKey<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    chash:  alias::ChooseHash, // Hash used to determine cache pathway.
    key:    Key,
    meta:   Meta<UIDL, UID>,
    csum:   Vec<u8>,
}

impl<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
>
    StoredKey<UIDL, UID>
{
    pub fn ref_chash(&self)     -> &alias::ChooseHash { &self.chash }
    pub fn mut_chash(&mut self) -> &mut alias::ChooseHash { &mut self.chash }
    pub fn key(&self)       -> &Key     { &self.key }
    pub fn into_key(self)   -> Key      { self.key }
    pub fn meta(&self)      -> &Meta<UIDL, UID>    { &self.meta }

    /// This associated function tries to avoid unnecessary copying during the encoding of a
    /// `StoredKey` by accepting a mutable reference to a vector of key bytes and appending a
    /// timestamp and a checksum.
    pub fn build_bytes<C: Checksummer>(
        chash:      alias::ChooseHash,
        mut buf:    Vec<u8>,
        cind:       Option<usize>,
        meta:       &Meta<UIDL, UID>,
        csummer:    C,
    )
        -> Outcome<Vec<u8>>
    {
        let mut cbuf = chash.to_vec();
        cbuf.append(&mut buf);
        buf = cbuf;
        // 0. Append chunk index information.
        let cind = match cind {
            Some(uint) => Some(try_into!(u64, uint)),
            None => None::<u64>,
        };
        let dat_cind = dat!(cind);
        buf = res!(dat_cind.to_bytes(buf));
        // 1. Append meta to key.
        buf = res!(meta.to_bytes(buf));
        // 2. Calculate checksum and append.
        (buf, _) = res!(csummer.append(buf));
        Ok(buf)
    }

    /// Do these stored key bytes, as they lie in a data file, hold this key with this stamp?
    /// Read in place, since this is on every read from a file: the key follows the cache hash,
    /// and the stamp comes just before the checksum.  Key bytes are a daticle, which ends itself,
    /// so equal bytes at the head of the stored key are the whole of its key.
    pub fn holds(
        stored:     &[u8],
        key:        &[u8],
        meta:       &Meta<UIDL, UID>,
        csum_len:   usize,
    )
        -> Outcome<bool>
    {
        let key_end = constant::CACHE_HASH_BYTES + key.len();
        let meta_end = match stored.len().checked_sub(csum_len) {
            Some(n) => n,
            None    => return Ok(false),
        };
        let meta_start = match meta_end.checked_sub(Meta::<UIDL, UID>::BYTE_LEN) {
            Some(n) => n,
            None    => return Ok(false),
        };
        if key_end > meta_start || &stored[constant::CACHE_HASH_BYTES..key_end] != key {
            return Ok(false);
        }
        let (stamp, _) = res!(Meta::<UIDL, UID>::from_bytes(&stored[meta_start..meta_end]));
        Ok(stamp == *meta)
    }

    /// Returns the key bytes, the complete `StoredKey` bytes and the length.
    pub fn load<
        C: Checksummer,
        R: io::Read,
    >(
        mut r:      &mut R,
        csummer:    C,
    )
        -> Outcome<Option<(Self, Vec<u8>, usize)>>
    {
        // Load the chash bytes.
        let mut chash = alias::ChooseHash::default();
        match r.read_exact(&mut chash) {
            Err(e) => match e.kind() {
                std::io::ErrorKind::UnexpectedEof => return Ok(None),
                _ => return Err(err!(e,
                    "While trying to read cache hash from the start of a new key.";
                    Decode, Bytes)),
            }
            _ => (),
        }
        let mut skey = chash.to_vec();

        // Load key Daticle.
        let keybyts = res!(Dat::load_bytes(&mut r), Decode, Bytes);
        skey.extend_from_slice(&keybyts[..]);

        // Load chunk index information.
        let cind_byts = res!(Dat::load_bytes(&mut r), Decode, Bytes);
        let (dat_cind, _) = res!(Dat::from_bytes(&cind_byts));
        let cind = match &dat_cind {
            Dat::Opt(boxoptd) => {
                match **boxoptd {
                    Some(Dat::U64(i)) => Some(try_into!(usize, i)),
                    None => None,
                    _ => return Err(err!(
                        "Expected Dat::Opt(Dat::U64), decoded {:?}.", dat_cind;
                    Invalid, Input)),
                }
            },
            _ => {
                //debug!(sync_log::stream(), "chash={:02x?} keybyts={:02x?} cind_byts={:02x?}",chash,keybyts,cind_byts);
                return Err(err!(
                    "Expected Dat::Opt(Dat::U64), decoded {:?}.", dat_cind;
                Invalid, Input));
            },
        };
        skey.extend_from_slice(&cind_byts);

        // Load rest of data in one go, but we're forced to use a vec because of the potentially
        // variable checksummer byte length.
        let mut buf = vec![0; Meta::<UIDL, UID>::BYTE_LEN + res!(csummer.len())];
        res!(r.read_exact(&mut buf), Decode, Bytes);

        // Fuse with the key.
        skey.extend_from_slice(&buf);
        let skey_len = skey.len();

        // Verify checksum.
        let csum = res!(csummer.verify(&skey));

        // Remove the cache hash from the front of the key, now that the overall checksum has been
        // verified.
        skey.drain(..constant::CACHE_HASH_BYTES);

        // Read Meta data.
        let (meta, _) = res!(Meta::from_bytes(&buf));

        Ok(Some((
            Self {
                chash,
                key:    match cind {
                    Some(i) => Key::Chunk(keybyts, i),
                    None => Key::Complete(keybyts),
                },
                meta,
                csum,
            },
            skey,
            skey_len,
        )))
    }

}

pub struct StoredValue {}

impl StoredValue {

    /// This associated function is intended to avoid unnecessary copying during the encoding of a
    /// `StoredValue` by accepting a mutable reference to a vector of value bytes and appending a
    /// checksum.
    pub fn build_bytes<C: Checksummer>(
        mut buf:    Vec<u8>,
        csummer:    C,
    )
        -> Outcome<Vec<u8>>
    {
        (buf, _) = res!(csummer.append(buf));
        Ok(buf)
    }

    pub fn count<RS: io::Read + io::Seek>(
        mut rs:     &mut RS,
        csumlen:    usize,
    )
        -> Outcome<usize>
    {
        // 1. Count value Daticle bytes.
        let n = res!(Dat::count_bytes(&mut rs), Decode, Bytes);
        if n == 0 {
            return Ok(0);
        }
        let csumlen_i64 = try_into!(i64, csumlen);
        res!(rs.seek(SeekFrom::Current(csumlen_i64)));
        Ok(n + csumlen)
    }

}

#[derive(Debug)]
pub struct StoredIndex {
    sfloc:  StoredFileLocation,
    csum:   Vec<u8>,
}

impl StoredIndex {

    pub fn keyval_len(&self) -> u64 {
        self.sfloc.floc.keyval().len
    }

    pub fn ref_file_location(&self) -> &FileLocation { &self.sfloc.floc }
    pub fn own_file_location(self) -> FileLocation { self.sfloc.floc }
    pub fn ref_stored_file_location(&self) -> &StoredFileLocation { &self.sfloc }
    pub fn own_stored_file_location(self) -> StoredFileLocation { self.sfloc }

    pub fn read<
        C: Checksummer,
        R: io::Read,
    >(
        r:          &mut R,
        fnum:       FileNum,
        csummer:    C,
    )
        -> Outcome<(Option<Self>, usize)>
    {
        // Read a StoredFileLocation, minus the checksum (but calculating a checksum).
        let (sfloc, n, csum1) = res!(StoredFileLocation::read_bytes(r, fnum, csummer.clone()));
        let (csum2, n2) = res!(csummer.read_bytes(r));
        res!(ChecksumScheme::compare(&csum1, &csum2));
        Ok((
            Some(Self {
                sfloc:  sfloc,
                csum:   csum2,
            }),
            n + n2,
        ))
    }

}
