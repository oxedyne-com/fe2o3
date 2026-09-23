//! PMTiles version 3: a whole tile pyramid in one file, read by byte range.
//!
//! An archive is a 127-byte header, a root directory, optional JSON metadata, leaf directories
//! and the tile data, and every tile is found by at most four ranged reads with no index
//! server: the header and root, up to three levels of leaf directory, the tile.  That is what
//! lets a street map be served from a plain file by any web server that answers `Range`, or
//! read straight from disk.  The format is Protomaps' specification at
//! <https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md>.
//!
//! The reading is split so that nothing here does input or output of its own.  [`Header`],
//! [`decode_directory`] and [`next`] are pure: a caller fetching ranges asynchronously -- over
//! HTTP, from a browser, from object storage -- drives them itself, one read per step.  An
//! [`Archive`] drives them over any synchronous [`RangeSource`], of which a local file
//! ([`FileSource`]) and a buffer in memory are provided here; an HTTP one lives with the HTTP
//! client, in `fe2o3_net`, so that this crate carries no network code.
//!
//! Tile ids number the tiles of all zooms along one Hilbert curve per zoom, so that tiles near
//! each other on the map are near each other in the file and runs of identical tiles -- open
//! ocean -- collapse into one directory entry.

use oxedyne_fe2o3_core::prelude::*;

use std::io::Read;
#[cfg(not(unix))]
use std::io::{
    Seek,
    SeekFrom,
};

pub const HEADER_LEN: usize = 127;
pub const MAGIC: &[u8; 7] = b"PMTiles";
pub const VERSION: u8 = 3;
pub const MAX_DEPTH: usize = 4;         // the root and three levels of leaves
pub const MAX_ZOOM: u8 = 31;

/// How the directories or the tiles of an archive are compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Unknown,
    None,
    Gzip,
    Brotli,
    Zstd,
}

impl Compression {
    fn from_code(c: u8) -> Outcome<Self> {
        match c {
            0   => Ok(Self::Unknown),
            1   => Ok(Self::None),
            2   => Ok(Self::Gzip),
            3   => Ok(Self::Brotli),
            4   => Ok(Self::Zstd),
            _   => Err(err!("Compression {} is none the specification names.", c; Invalid, Input)),
        }
    }
}

/// What the tiles of an archive are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileType {
    Unknown,
    Mvt,
    Png,
    Jpeg,
    Webp,
    Avif,
    Mlt,
}

impl TileType {
    fn from_code(c: u8) -> Outcome<Self> {
        match c {
            0   => Ok(Self::Unknown),
            1   => Ok(Self::Mvt),
            2   => Ok(Self::Png),
            3   => Ok(Self::Jpeg),
            4   => Ok(Self::Webp),
            5   => Ok(Self::Avif),
            6   => Ok(Self::Mlt),
            _   => Err(err!("Tile type {} is none the specification names.", c; Invalid, Input)),
        }
    }
}

/// The fixed 127 bytes an archive begins with.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub root_offset:            u64,
    pub root_length:            u64,
    pub metadata_offset:        u64,
    pub metadata_length:        u64,
    pub leaf_offset:            u64,
    pub leaf_length:            u64,
    pub data_offset:            u64,
    pub data_length:            u64,
    pub addressed_tiles:        u64,
    pub tile_entries:           u64,
    pub tile_contents:          u64,
    pub clustered:              bool,
    pub internal_compression:   Compression,
    pub tile_compression:       Compression,
    pub tile_type:              TileType,
    pub min_zoom:               u8,
    pub max_zoom:               u8,
    pub min_lon_e7:             i32,    // degrees times ten million, as the file holds them
    pub min_lat_e7:             i32,
    pub max_lon_e7:             i32,
    pub max_lat_e7:             i32,
    pub centre_zoom:            u8,
    pub centre_lon_e7:          i32,
    pub centre_lat_e7:          i32,
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

fn e7_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Header {
    /// Reads the header from the first 127 bytes (or more) of an archive.
    pub fn parse(b: &[u8]) -> Outcome<Self> {
        if b.len() < HEADER_LEN {
            return Err(err!("A PMTiles header is {} bytes and {} were given.", HEADER_LEN, b.len();
                Invalid, Input, TooSmall));
        }
        if &b[..7] != MAGIC {
            return Err(err!("The archive does not begin with the PMTiles magic."; Invalid, Input));
        }
        if b[7] != VERSION {
            return Err(err!("The archive is PMTiles version {}, and this reads version {}.",
                b[7], VERSION; Invalid, Input, Version));
        }
        let h = Self {
            root_offset:            u64_at(b, 8),
            root_length:            u64_at(b, 16),
            metadata_offset:        u64_at(b, 24),
            metadata_length:        u64_at(b, 32),
            leaf_offset:            u64_at(b, 40),
            leaf_length:            u64_at(b, 48),
            data_offset:            u64_at(b, 56),
            data_length:            u64_at(b, 64),
            addressed_tiles:        u64_at(b, 72),
            tile_entries:           u64_at(b, 80),
            tile_contents:          u64_at(b, 88),
            clustered:              b[96] == 1,
            internal_compression:   res!(Compression::from_code(b[97])),
            tile_compression:       res!(Compression::from_code(b[98])),
            tile_type:              res!(TileType::from_code(b[99])),
            min_zoom:               b[100],
            max_zoom:               b[101],
            min_lon_e7:             e7_at(b, 102),
            min_lat_e7:             e7_at(b, 106),
            max_lon_e7:             e7_at(b, 110),
            max_lat_e7:             e7_at(b, 114),
            centre_zoom:            b[118],
            centre_lon_e7:          e7_at(b, 119),
            centre_lat_e7:          e7_at(b, 123),
        };
        if h.min_zoom > h.max_zoom || h.max_zoom > MAX_ZOOM {
            return Err(err!("The archive claims zooms {} to {}.", h.min_zoom, h.max_zoom;
                Invalid, Input, Range));
        }
        if h.root_length > 16_384 * 1024 {
            return Err(err!("The archive claims a root directory of {} bytes.", h.root_length;
                Invalid, Input, Excessive));
        }
        Ok(h)
    }

    /// The bounds as `[min lon, min lat, max lon, max lat]` in degrees times ten million.
    pub fn bounds_e7(&self) -> [i32; 4] {
        [self.min_lon_e7, self.min_lat_e7, self.max_lon_e7, self.max_lat_e7]
    }

    /// The bounds as `[min lon, min lat, max lon, max lat]` in degrees.
    pub fn bounds(&self) -> [f64; 4] {
        let b = self.bounds_e7();
        [b[0] as f64 / 1.0e7, b[1] as f64 / 1.0e7, b[2] as f64 / 1.0e7, b[3] as f64 / 1.0e7]
    }
}

/// One directory entry: a run of tiles sharing one stored content, or, with a run length of
/// nought, a pointer to a leaf directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub tile_id:    u64,
    pub offset:     u64,    // from the start of the tile data, or of the leaf directories
    pub length:     u32,
    pub run_length: u32,
}

/// The id of tile `z/x/y`: the tiles of every shallower zoom, then the tile's position along
/// the Hilbert curve through its own zoom.
pub fn zxy_to_id(z: u8, x: u32, y: u32) -> Outcome<u64> {
    if z > MAX_ZOOM {
        return Err(err!("Zoom {} is past the deepest, {}.", z, MAX_ZOOM; Invalid, Input, Range));
    }
    let n = 1u64 << z;
    let (mut x, mut y) = (x as u64, y as u64);
    if x >= n || y >= n {
        return Err(err!("Tile {}/{}/{} is off a grid {} tiles across.", z, x, y, n; Invalid, Input, Range));
    }
    let base = ((1u128 << (2 * z as u32)) - 1) / 3;
    let mut d: u64 = 0;
    let mut s = n / 2;
    while s > 0 {
        let rx = (x & s) != 0;
        let ry = (y & s) != 0;
        d += s * s * ((if rx { 3 } else { 0 }) ^ (if ry { 1 } else { 0 }));
        // Turn the quadrant so the curve runs on through the next level.
        if !ry {
            if rx {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    Ok(base as u64 + d)
}

/// The tile a tile id names.
pub fn id_to_zxy(id: u64) -> Outcome<(u8, u32, u32)> {
    let mut base: u128 = 0;
    for z in 0..=MAX_ZOOM {
        let count = 1u128 << (2 * z as u32);
        if (id as u128) < base + count {
            let n = 1u64 << z;
            let mut t = (id as u128 - base) as u64;
            let (mut x, mut y) = (0u64, 0u64);
            let mut s = 1u64;
            while s < n {
                let rx = 1 & (t / 2);
                let ry = 1 & (t ^ rx);
                if ry == 0 {
                    if rx == 1 {
                        x = s - 1 - x;
                        y = s - 1 - y;
                    }
                    std::mem::swap(&mut x, &mut y);
                }
                x += s * rx;
                y += s * ry;
                t /= 4;
                s *= 2;
            }
            return Ok((z, x as u32, y as u32));
        }
        base += count;
    }
    Err(err!("Tile id {} is past zoom {}.", id, MAX_ZOOM; Invalid, Input, Range))
}

fn varint(b: &[u8], pos: &mut usize) -> Outcome<u64> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = match b.get(*pos) {
            Some(x) => *x,
            None => return Err(err!("A directory ends inside a varint at byte {}.", *pos;
                Invalid, Input, Decode)),
        };
        *pos += 1;
        if shift >= 64 {
            return Err(err!("A directory varint at byte {} is longer than ten bytes.", *pos;
                Invalid, Input, Decode));
        }
        v |= ((byte & 0x7f) as u64) << shift;
        if byte < 0x80 {
            return Ok(v);
        }
        shift += 7;
    }
}

/// Decodes a directory from its bytes, already decompressed: an entry count, then the tile
/// ids as deltas, the run lengths, the lengths and the offsets, each column in turn, all
/// varints.  An offset of nought after the first entry means "straight after the last".
pub fn decode_directory(b: &[u8]) -> Outcome<Vec<Entry>> {
    let mut pos = 0usize;
    let n = res!(varint(b, &mut pos)) as usize;
    // Every entry costs at least four bytes, so a count past that is a lie and sizes nothing.
    if n > b.len() / 4 + 1 {
        return Err(err!("A directory of {} bytes claims {} entries.", b.len(), n; Invalid, Input, Decode));
    }
    let mut out = vec![Entry { tile_id: 0, offset: 0, length: 0, run_length: 0 }; n];
    let mut id: u64 = 0;
    for e in out.iter_mut() {
        id = match id.checked_add(res!(varint(b, &mut pos))) {
            Some(v) => v,
            None => return Err(err!("A directory's tile ids overflow."; Invalid, Input, Decode, Overflow)),
        };
        e.tile_id = id;
    }
    for e in out.iter_mut() {
        e.run_length = res!(u32_of(res!(varint(b, &mut pos)), "A run length"));
    }
    for e in out.iter_mut() {
        e.length = res!(u32_of(res!(varint(b, &mut pos)), "A length"));
    }
    for i in 0..n {
        let v = res!(varint(b, &mut pos));
        out[i].offset = if v == 0 && i > 0 {
            out[i - 1].offset + out[i - 1].length as u64
        } else if v == 0 {
            return Err(err!("A directory's first offset is written as 'after the last'.";
                Invalid, Input, Decode));
        } else {
            v - 1
        };
    }
    if pos != b.len() {
        return Err(err!("A directory has {} bytes after its last entry.", b.len() - pos;
            Invalid, Input, Decode));
    }
    Ok(out)
}

fn u32_of(v: u64, what: &str) -> Outcome<u32> {
    if v > u32::MAX as u64 {
        return Err(err!("{} of {} is more than 32 bits.", what, v; Invalid, Input, Decode));
    }
    Ok(v as u32)
}

/// The entry covering a tile id: the last entry at or before it, if that is a leaf pointer or
/// a run that reaches it.
pub fn find(entries: &[Entry], id: u64) -> Option<&Entry> {
    let at = entries.partition_point(|e| e.tile_id <= id);
    if at == 0 {
        return None;
    }
    let e = &entries[at - 1];
    if e.run_length == 0 || id - e.tile_id < e.run_length as u64 {
        Some(e)
    } else {
        None
    }
}

/// What to read next to find a tile, as absolute byte ranges of the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Tile { offset: u64, length: u64 },  // the tile, as stored
    Leaf { offset: u64, length: u64 },  // a leaf directory to decode and search in turn
    Missing,                            // no such tile
}

/// One step of finding a tile in a directory already read.
pub fn next(header: &Header, dir: &[Entry], id: u64) -> Next {
    match find(dir, id) {
        None => Next::Missing,
        Some(e) if e.run_length == 0 => Next::Leaf {
            offset: header.leaf_offset + e.offset, length: e.length as u64 },
        Some(e) => Next::Tile { offset: header.data_offset + e.offset, length: e.length as u64 },
    }
}

/// Undoes an archive's compression of its directories or tiles.
pub fn decompress(bytes: &[u8], c: Compression) -> Outcome<Vec<u8>> {
    match c {
        Compression::None | Compression::Unknown => Ok(bytes.to_vec()),
        Compression::Gzip => {
            let mut out = Vec::new();
            let mut dec = flate2::read::GzDecoder::new(bytes);
            res!(dec.read_to_end(&mut out), Decode, IO);
            Ok(out)
        },
        other => Err(err!("{:?} compression is not read here; only gzip and none are.", other;
            Invalid, Input, Unimplemented)),
    }
}

// ---------------------------------------------------------------------------------------------
// Synchronous reading
// ---------------------------------------------------------------------------------------------

/// Anything that can hand over a byte range: a file, a buffer, a remote file by HTTP `Range`.
///
/// A read takes `&self`, so one source serves many threads at once, as a tile server's
/// blocking workers do.
pub trait RangeSource {
    fn read(&self, offset: u64, len: u64) -> Outcome<Vec<u8>>;
}

/// A local archive file, read by position so that concurrent reads need no lock.
pub struct FileSource {
    #[cfg(unix)]
    file:   std::fs::File,
    #[cfg(not(unix))]
    file:   std::sync::Mutex<std::fs::File>,
}

impl FileSource {
    pub fn open<P: AsRef<std::path::Path>>(path: P) -> Outcome<Self> {
        let file = res!(std::fs::File::open(path.as_ref()), File, Read);
        #[cfg(unix)]
        return Ok(Self { file });
        #[cfg(not(unix))]
        return Ok(Self { file: std::sync::Mutex::new(file) });
    }
}

impl RangeSource for FileSource {
    fn read(&self, offset: u64, len: u64) -> Outcome<Vec<u8>> {
        if len > u32::MAX as u64 {
            return Err(err!("A read of {} bytes is more than one tile or directory needs.", len;
                Invalid, Input, Excessive));
        }
        let mut buf = vec![0u8; len as usize];
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            res!(self.file.read_exact_at(&mut buf, offset), File, Read);
        }
        #[cfg(not(unix))]
        {
            let mut f = lock_mutex!(self.file);
            res!(f.seek(SeekFrom::Start(offset)), File, Seek);
            res!(f.read_exact(&mut buf), File, Read);
        }
        Ok(buf)
    }
}

impl RangeSource for &[u8] {
    fn read(&self, offset: u64, len: u64) -> Outcome<Vec<u8>> {
        let end = offset.checked_add(len);
        match end {
            Some(e) if e <= self.len() as u64 => Ok(self[offset as usize..e as usize].to_vec()),
            _ => Err(err!("Bytes {}+{} are past the {} there are.", offset, len, self.len();
                Invalid, Input, Range)),
        }
    }
}

/// An archive over a synchronous source, with its root directory and the leaf directories it
/// has read lately kept in memory.  Reads take `&self`, so an archive shared between threads
/// serves them all; the leaf cache is the only thing they share, behind a lock held for a
/// lookup and never across a read.
pub struct Archive<S: RangeSource> {
    src:    S,
    header: Header,
    root:   Vec<Entry>,
    leaves: std::sync::Mutex<Vec<(u64, std::sync::Arc<Vec<Entry>>)>>, // most recent last
}

const LEAF_CACHE: usize = 32;

impl<S: RangeSource> Archive<S> {
    pub fn open(src: S) -> Outcome<Self> {
        let head = res!(src.read(0, HEADER_LEN as u64));
        let header = res!(Header::parse(&head));
        let raw = res!(src.read(header.root_offset, header.root_length));
        let root = res!(decode_directory(&res!(decompress(&raw, header.internal_compression))));
        Ok(Self { src, header, root, leaves: std::sync::Mutex::new(Vec::new()) })
    }

    pub fn header(&self) -> &Header { &self.header }

    pub fn root(&self) -> &[Entry] { &self.root }

    /// The archive's JSON metadata, decompressed.
    pub fn metadata(&self) -> Outcome<String> {
        let raw = res!(self.src.read(self.header.metadata_offset, self.header.metadata_length));
        let bytes = res!(decompress(&raw, self.header.internal_compression));
        match String::from_utf8(bytes) {
            Ok(s)   => Ok(s),
            Err(_)  => Err(err!("The archive's metadata is not UTF-8."; Invalid, Input, UTF8)),
        }
    }

    /// Tile `z/x/y` as stored, still compressed as [`Header::tile_compression`] says, or
    /// `None` where the archive has no such tile.  Stored bytes are what a server passes on
    /// with a `Content-Encoding`; [`Archive::tile_decoded`] undoes it.
    pub fn tile(&self, z: u8, x: u32, y: u32) -> Outcome<Option<Vec<u8>>> {
        if z < self.header.min_zoom || z > self.header.max_zoom {
            return Ok(None);
        }
        let id = res!(zxy_to_id(z, x, y));
        let mut step = next(&self.header, &self.root, id);
        for _ in 0..MAX_DEPTH {
            match step {
                Next::Missing => return Ok(None),
                Next::Tile { offset, length } => return Ok(Some(res!(self.src.read(offset, length)))),
                Next::Leaf { offset, length } => {
                    let dir = res!(self.leaf(offset, length));
                    step = next(&self.header, &dir, id);
                },
            }
        }
        Err(err!("Tile {}/{}/{} is nested deeper than {} directories.", z, x, y, MAX_DEPTH;
            Invalid, Input, Decode))
    }

    /// A leaf directory, from the cache or read and decoded.  Two threads missing the same
    /// leaf at once both read it, which costs a read and keeps the lock off the disk.
    fn leaf(&self, offset: u64, length: u64) -> Outcome<std::sync::Arc<Vec<Entry>>> {
        {
            let mut cache = lock_mutex!(self.leaves);
            if let Some(i) = cache.iter().position(|(o, _)| *o == offset) {
                let hit = cache.remove(i);
                let dir = hit.1.clone();
                cache.push(hit);
                return Ok(dir);
            }
        }
        let raw = res!(self.src.read(offset, length));
        let dir = res!(decode_directory(&res!(decompress(&raw, self.header.internal_compression))));
        if dir.is_empty() {
            return Err(err!("The leaf directory at byte {} is empty.", offset; Invalid, Input, Decode));
        }
        let dir = std::sync::Arc::new(dir);
        let mut cache = lock_mutex!(self.leaves);
        cache.push((offset, dir.clone()));
        if cache.len() > LEAF_CACHE {
            cache.remove(0);
        }
        Ok(dir)
    }

    /// Tile `z/x/y` with the archive's tile compression undone.
    pub fn tile_decoded(&self, z: u8, x: u32, y: u32) -> Outcome<Option<Vec<u8>>> {
        let c = self.header.tile_compression;
        match res!(self.tile(z, x, y)) {
            Some(raw)   => Ok(Some(res!(decompress(&raw, c)))),
            None        => Ok(None),
        }
    }
}
