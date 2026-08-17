//! A dependency-free reader for TIFF-structured image metadata, better known as EXIF.
//!
//! The same byte layout appears in several containers.  A JPEG carries it inside an `APP1`
//! segment prefixed with `Exif\0\0`, a TIFF file *is* the structure, and formats such as HEIC
//! embed the identical block inside a box.  [`Exif::from_jpeg`] and [`Exif::from_tiff`] cover the
//! first two, and [`Exif::from_tiff`] serves the third once the caller has located the payload.
//!
//! Parsing is total: no input, however damaged, causes a panic.  Every failure returns a tagged
//! [`Error`] naming the byte offset and the structure that failed.  Fields whose tag or type this
//! module does not recognise are preserved as raw bytes rather than dropped, so a caller can
//! recover a maker note or a vendor extension that arrived after this code was written.
//!
//! # Example
//! ```no_run
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_file::exif::Exif;
//!
//! fn describe(path: &str) -> Outcome<()> {
//!     let dat = res!(std::fs::read(path), IO, File);
//!     if let Some(exif) = res!(Exif::from_jpeg(&dat)) {
//!         let meta = exif.meta();
//!         println!("{:?} {:?}", meta.make, meta.datetime_original);
//!     }
//!     Ok(())
//! }
//!
//! assert!(describe("photo.jpg").is_ok());
//! ```
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::BTreeSet,
    fmt,
};


// Ceilings on a structure that a malformed file could otherwise make unbounded.  Crossing either
// is an error, not a truncation.
const MAX_IFD_CHAIN: usize = 16;
const MAX_IFD_ENTRIES: u64 = 4096;

/// Size in bytes of one IFD entry, fixed by TIFF.
const ENTRY_LEN: u64 = 12;

/// The `Exif\0\0` signature that opens an EXIF `APP1` payload.
const APP1_SIG: &[u8] = b"Exif\0\0";

/// Numeric identifiers of the tags this module gives names to.
pub mod tag {
    // IFD0 and IFD1.
    /// Image width in pixels, as recorded in IFD0.
    pub const IMAGE_WIDTH:              u16 = 0x0100;
    /// Image height in pixels, as recorded in IFD0.
    pub const IMAGE_LENGTH:             u16 = 0x0101;
    /// Camera manufacturer.
    pub const MAKE:                     u16 = 0x010F;
    /// Camera model.
    pub const MODEL:                    u16 = 0x0110;
    /// Orientation of the rows and columns, 1 to 8.
    pub const ORIENTATION:              u16 = 0x0112;
    /// File change date and time.
    pub const DATE_TIME:                u16 = 0x0132;
    /// Byte offset of the IFD1 thumbnail.
    pub const JPEG_INTERCHANGE_FORMAT:  u16 = 0x0201;
    /// Byte length of the IFD1 thumbnail.
    pub const JPEG_INTERCHANGE_LENGTH:  u16 = 0x0202;
    /// Pointer from IFD0 to the EXIF sub-IFD.
    pub const EXIF_IFD_POINTER:         u16 = 0x8769;
    /// Pointer from IFD0 to the GPS sub-IFD.
    pub const GPS_IFD_POINTER:          u16 = 0x8825;

    // ExifIFD.
    /// Exposure time in seconds.
    pub const EXPOSURE_TIME:            u16 = 0x829A;
    /// Lens aperture as an f-number.
    pub const F_NUMBER:                 u16 = 0x829D;
    /// Sensitivity, historically ISOSpeedRatings.
    pub const ISO_SPEED_RATINGS:        u16 = 0x8827;
    /// Date and time the original image was captured.
    pub const DATE_TIME_ORIGINAL:       u16 = 0x9003;
    /// Date and time the image was digitised.
    pub const CREATE_DATE:              u16 = 0x9004;
    /// UTC offset applying to `DATE_TIME`.
    pub const OFFSET_TIME:              u16 = 0x9010;
    /// UTC offset applying to `DATE_TIME_ORIGINAL`.
    pub const OFFSET_TIME_ORIGINAL:     u16 = 0x9011;
    /// UTC offset applying to `CREATE_DATE`.
    pub const OFFSET_TIME_DIGITIZED:    u16 = 0x9012;
    /// Sub-second fraction for `DATE_TIME`.
    pub const SUBSEC_TIME:              u16 = 0x9290;
    /// Sub-second fraction for `DATE_TIME_ORIGINAL`.
    pub const SUBSEC_TIME_ORIGINAL:     u16 = 0x9291;
    /// Sub-second fraction for `CREATE_DATE`.
    pub const SUBSEC_TIME_DIGITIZED:    u16 = 0x9292;
    /// Actual focal length of the lens in millimetres.
    pub const FOCAL_LENGTH:             u16 = 0x920A;
    /// Valid image width in pixels, as recorded in the EXIF sub-IFD.
    pub const EXIF_IMAGE_WIDTH:         u16 = 0xA002;
    /// Valid image height in pixels, as recorded in the EXIF sub-IFD.
    pub const EXIF_IMAGE_HEIGHT:        u16 = 0xA003;
    /// Pointer from the EXIF sub-IFD to the interoperability sub-IFD.
    pub const INTEROP_IFD_POINTER:      u16 = 0xA005;
    /// Focal length expressed for a 35 mm film frame.
    pub const FOCAL_LENGTH_35MM:        u16 = 0xA405;
    /// Lens manufacturer.
    pub const LENS_MAKE:                u16 = 0xA433;
    /// Lens model.
    pub const LENS_MODEL:               u16 = 0xA434;

    // GPS IFD.
    /// Hemisphere of the latitude, `N` or `S`.
    pub const GPS_LATITUDE_REF:         u16 = 0x0001;
    /// Latitude as degrees, minutes and seconds.
    pub const GPS_LATITUDE:             u16 = 0x0002;
    /// Hemisphere of the longitude, `E` or `W`.
    pub const GPS_LONGITUDE_REF:        u16 = 0x0003;
    /// Longitude as degrees, minutes and seconds.
    pub const GPS_LONGITUDE:            u16 = 0x0004;
    /// Datum of the altitude, 0 above sea level and 1 below.
    pub const GPS_ALTITUDE_REF:         u16 = 0x0005;
    /// Altitude in metres relative to `GPS_ALTITUDE_REF`.
    pub const GPS_ALTITUDE:             u16 = 0x0006;
    /// UTC time of the fix, as hours, minutes and seconds.
    pub const GPS_TIMESTAMP:            u16 = 0x0007;
    /// UTC date of the fix, as `YYYY:MM:DD`.
    pub const GPS_DATESTAMP:            u16 = 0x001D;
}

/// Byte order declared by the TIFF header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteOrder {
    Little, // Intel, written II, least significant byte first
    Big,    // Motorola, written MM, most significant byte first
}

impl fmt::Display for ByteOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Little	=> write!(f, "II"),
            Self::Big		=> write!(f, "MM"),
        }
    }
}

impl ByteOrder {

    pub fn u16(&self, b: [u8; 2]) -> u16 {
        match self {
            Self::Little	=> u16::from_le_bytes(b),
            Self::Big		=> u16::from_be_bytes(b),
        }
    }

    pub fn u32(&self, b: [u8; 4]) -> u32 {
        match self {
            Self::Little	=> u32::from_le_bytes(b),
            Self::Big		=> u32::from_be_bytes(b),
        }
    }
}

/// The IFD in which a field was found.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IfdKind {
    Ifd0,       // the primary image
    Exif,       // sub-IFD reached through EXIF_IFD_POINTER
    Gps,        // sub-IFD reached through GPS_IFD_POINTER
    Interop,    // sub-IFD reached through INTEROP_IFD_POINTER
    Ifd1,       // the thumbnail
}

impl fmt::Display for IfdKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ifd0		=> write!(f, "IFD0"),
            Self::Exif		=> write!(f, "ExifIFD"),
            Self::Gps		=> write!(f, "GpsIFD"),
            Self::Interop	=> write!(f, "InteropIFD"),
            Self::Ifd1		=> write!(f, "IFD1"),
        }
    }
}

/// An unsigned ratio of two 32 bit integers, the TIFF `RATIONAL` type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

impl Rational {
    /// `None` when the denominator is zero.
    pub fn to_f64(&self) -> Option<f64> {
        if self.den == 0 {
            None
        } else {
            Some(self.num as f64 / self.den as f64)
        }
    }
}

/// A signed ratio of two 32 bit integers, the TIFF `SRATIONAL` type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SRational {
    pub num: i32,
    pub den: i32,
}

impl fmt::Display for SRational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

impl SRational {
    /// `None` when the denominator is zero.
    pub fn to_f64(&self) -> Option<f64> {
        if self.den == 0 {
            None
        } else {
            Some(self.num as f64 / self.den as f64)
        }
    }
}

/// The decoded payload of one IFD entry.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(Vec<u8>),             // TIFF type 1
    Ascii(String),             // TIFF type 2, NUL terminated
    Short(Vec<u16>),           // TIFF type 3
    Long(Vec<u32>),            // TIFF type 4
    Rational(Vec<Rational>),   // TIFF type 5
    SByte(Vec<i8>),            // TIFF type 6
    Undefined(Vec<u8>),        // TIFF type 7, an opaque run whose meaning the tag defines
    SShort(Vec<i16>),          // TIFF type 8
    SLong(Vec<i32>),           // TIFF type 9
    SRational(Vec<SRational>), // TIFF type 10
    Float(Vec<f32>),           // TIFF type 11
    Double(Vec<f64>),          // TIFF type 12
    Ifd(Vec<u32>),             // TIFF type 13, an offset to a nested IFD
    // A type this module does not know, preserved as the raw entry payload.
    Unknown {
        typ: u16,        // the type code as written in the file
        count: u32,      // the count as written in the file
        raw: [u8; 4],    // the four bytes of the value or offset field
    },
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ascii(s)		=> write!(f, "{}", s),
            Self::Byte(v)		=> write!(f, "{:?}", v),
            Self::Short(v)		=> write!(f, "{:?}", v),
            Self::Long(v)		=> write!(f, "{:?}", v),
            Self::Rational(v)	=> write!(f, "{}", Self::join(v)),
            Self::SByte(v)		=> write!(f, "{:?}", v),
            Self::Undefined(v)	=> write!(f, "<{} bytes>", v.len()),
            Self::SShort(v)		=> write!(f, "{:?}", v),
            Self::SLong(v)		=> write!(f, "{:?}", v),
            Self::SRational(v)	=> write!(f, "{}", Self::join(v)),
            Self::Float(v)		=> write!(f, "{:?}", v),
            Self::Double(v)		=> write!(f, "{:?}", v),
            Self::Ifd(v)		=> write!(f, "{:?}", v),
            Self::Unknown { typ, count, raw } =>
                write!(f, "<unknown type {} count {} raw {:02x?}>", typ, count, raw),
        }
    }
}

impl Value {

    /// Joins items with a space, the separator the ratio variants display under.
    fn join<T: fmt::Display>(v: &[T]) -> String {
        let mut s = String::new();
        for (i, item) in v.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&fmt!("{}", item));
        }
        s
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Byte(v)		=> v.len(),
            Self::Ascii(s)		=> s.len(),
            Self::Short(v)		=> v.len(),
            Self::Long(v)		=> v.len(),
            Self::Rational(v)	=> v.len(),
            Self::SByte(v)		=> v.len(),
            Self::Undefined(v)	=> v.len(),
            Self::SShort(v)		=> v.len(),
            Self::SLong(v)		=> v.len(),
            Self::SRational(v)	=> v.len(),
            Self::Float(v)		=> v.len(),
            Self::Double(v)		=> v.len(),
            Self::Ifd(v)		=> v.len(),
            Self::Unknown { count, .. } => *count as usize,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Ascii(s)	=> Some(s.as_str()),
            _				=> None,
        }
    }

    /// Returns the first element only, where the type widens.
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Self::Byte(v)		=> v.first().map(|n| *n as u32),
            Self::Short(v)		=> v.first().map(|n| *n as u32),
            Self::Long(v)		=> v.first().copied(),
            Self::Ifd(v)		=> v.first().copied(),
            Self::SShort(v)		=> v.first().and_then(|n| u32::try_from(*n).ok()),
            Self::SLong(v)		=> v.first().and_then(|n| u32::try_from(*n).ok()),
            _					=> None,
        }
    }

    /// Returns the first element only, where the type converts.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Byte(v)		=> v.first().map(|n| *n as f64),
            Self::Short(v)		=> v.first().map(|n| *n as f64),
            Self::Long(v)		=> v.first().map(|n| *n as f64),
            Self::SByte(v)		=> v.first().map(|n| *n as f64),
            Self::SShort(v)		=> v.first().map(|n| *n as f64),
            Self::SLong(v)		=> v.first().map(|n| *n as f64),
            Self::Float(v)		=> v.first().map(|n| *n as f64),
            Self::Double(v)		=> v.first().copied(),
            Self::Rational(v)	=> v.first().and_then(|r| r.to_f64()),
            Self::SRational(v)	=> v.first().and_then(|r| r.to_f64()),
            _					=> None,
        }
    }

    /// Returns every element, where the type converts.
    pub fn as_f64_vec(&self) -> Option<Vec<f64>> {
        match self {
            Self::Byte(v)		=> Some(v.iter().map(|n| *n as f64).collect()),
            Self::Short(v)		=> Some(v.iter().map(|n| *n as f64).collect()),
            Self::Long(v)		=> Some(v.iter().map(|n| *n as f64).collect()),
            Self::SByte(v)		=> Some(v.iter().map(|n| *n as f64).collect()),
            Self::SShort(v)		=> Some(v.iter().map(|n| *n as f64).collect()),
            Self::SLong(v)		=> Some(v.iter().map(|n| *n as f64).collect()),
            Self::Float(v)		=> Some(v.iter().map(|n| *n as f64).collect()),
            Self::Double(v)		=> Some(v.clone()),
            Self::Rational(v)	=> v.iter().map(|r| r.to_f64()).collect(),
            Self::SRational(v)	=> v.iter().map(|r| r.to_f64()).collect(),
            _					=> None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub tag: u16,
    pub ifd: IfdKind,
    pub value: Value,
}

/// The fields of every IFD reached, flattened into one list.
#[derive(Clone, Debug)]
pub struct Exif {
    pub order: ByteOrder,                   // as declared by the TIFF header
    pub fields: Vec<Field>,                 // in the order the IFDs were walked
    pub thumbnail: Option<(u32, u32)>,      // IFD1 offset and length within the TIFF block
}

impl Exif {

    /// Parses the EXIF `APP1` segment of a JPEG byte stream, returning `None` when there is none.
    pub fn from_jpeg(dat: &[u8]) -> Outcome<Option<Self>> {
        match res!(Self::find_jpeg_app1(dat)) {
            Some(payload)	=> Ok(Some(res!(Self::from_tiff(payload)))),
            None			=> Ok(None),
        }
    }

    /// Parses a bare TIFF block, the form used by TIFF files and by HEIC `Exif` boxes.
    pub fn from_tiff(dat: &[u8]) -> Outcome<Self> {
        if dat.len() < 8 {
            return Err(err!(
                "TIFF header: {} bytes available at offset 0, at least 8 are needed for the \
                byte order mark, magic and first IFD offset.", dat.len();
            Input, Invalid, TooSmall, Decode));
        }
        let order = match &dat[0..2] {
            b"II"	=> ByteOrder::Little,
            b"MM"	=> ByteOrder::Big,
            other	=> return Err(err!(
                "TIFF header at offset 0: byte order mark {:02x?} is neither II nor MM.", other;
            Input, Invalid, Decode)),
        };
        let magic = order.u16([dat[2], dat[3]]);
        if magic != 42 {
            return Err(err!(
                "TIFF header at offset 2: magic number is {}, expected 42 in {} order.",
                magic, order;
            Input, Invalid, Decode));
        }
        let first = order.u32([dat[4], dat[5], dat[6], dat[7]]);

        let mut rdr = Reader { dat, order };
        let mut fields = Vec::new();
        let mut seen = BTreeSet::new();

        // Walk the IFD0 chain, whose second link is the thumbnail IFD1.
        let mut next = first;
        let mut idx = 0usize;
        while next != 0 {
            if !seen.insert(next) {
                return Err(err!(
                    "IFD chain: the pointer at link {} returns to offset {}, which has already \
                    been read, so the chain loops.", idx, next;
                Input, Invalid, Duplicate, Decode));
            }
            if idx >= MAX_IFD_CHAIN {
                return Err(err!(
                    "IFD chain: more than {} linked IFDs, refusing to follow the pointer to \
                    offset {}.", MAX_IFD_CHAIN, next;
                Input, Invalid, Excessive, Decode));
            }
            let kind = if idx == 0 { IfdKind::Ifd0 } else { IfdKind::Ifd1 };
            next = res!(rdr.read_ifd(next, kind, &mut fields));
            idx += 1;
        }

        // Follow the sub-IFD pointers found in IFD0, then the interoperability pointer in the
        // EXIF sub-IFD.  Each is read once, and a pointer back into a visited IFD is refused.
        let subs = [
            (IfdKind::Ifd0, tag::EXIF_IFD_POINTER,		IfdKind::Exif),
            (IfdKind::Ifd0, tag::GPS_IFD_POINTER,		IfdKind::Gps),
        ];
        for (from, ptr_tag, kind) in subs {
            if let Some(off) = Self::pointer(&fields, from, ptr_tag) {
                res!(rdr.read_sub_ifd(off, kind, &mut seen, &mut fields));
            }
        }
        if let Some(off) = Self::pointer(&fields, IfdKind::Exif, tag::INTEROP_IFD_POINTER) {
            res!(rdr.read_sub_ifd(off, IfdKind::Interop, &mut seen, &mut fields));
        }

        let thumbnail = match (
            Self::find(&fields, IfdKind::Ifd1, tag::JPEG_INTERCHANGE_FORMAT),
            Self::find(&fields, IfdKind::Ifd1, tag::JPEG_INTERCHANGE_LENGTH),
        ) {
            (Some(off), Some(len)) => match (off.value.as_u32(), len.value.as_u32()) {
                (Some(o), Some(l))	=> Some((o, l)),
                _					=> None,
            },
            _ => None,
        };

        Ok(Self { order, fields, thumbnail })
    }

    /// Parses whichever of JPEG or bare TIFF the leading bytes indicate.
    pub fn from_bytes(dat: &[u8]) -> Outcome<Option<Self>> {
        if dat.len() >= 2 && dat[0] == 0xFF && dat[1] == 0xD8 {
            Self::from_jpeg(dat)
        } else if dat.len() >= 2 && (&dat[0..2] == b"II" || &dat[0..2] == b"MM") {
            Ok(Some(res!(Self::from_tiff(dat))))
        } else {
            Err(err!(
                "Byte stream at offset 0: leading bytes {:02x?} are neither a JPEG start of \
                image nor a TIFF byte order mark.",
                &dat[0..dat.len().min(2)];
            Input, Invalid, Decode))
        }
    }

    pub fn field(&self, ifd: IfdKind, tag: u16) -> Option<&Field> {
        Self::find(&self.fields, ifd, tag)
    }

    /// Searches every IFD, in walk order.
    pub fn any_field(&self, tag: u16) -> Option<&Field> {
        self.fields.iter().find(|f| f.tag == tag)
    }

    pub fn ifd(&self, ifd: IfdKind) -> Vec<&Field> {
        self.fields.iter().filter(|f| f.ifd == ifd).collect()
    }

    pub fn meta(&self) -> PhotoMeta {
        PhotoMeta::from_exif(self)
    }

    fn pointer(fields: &[Field], ifd: IfdKind, tag: u16) -> Option<u32> {
        Self::find(fields, ifd, tag).and_then(|f| f.value.as_u32())
    }

    fn find(fields: &[Field], ifd: IfdKind, tag: u16) -> Option<&Field> {
        fields.iter().find(|f| f.ifd == ifd && f.tag == tag)
    }

    /// Returns the bytes following the `Exif\0\0` signature, which begin the TIFF header.  A
    /// stream with no such segment yields `None`; a stream whose marker structure is broken
    /// yields an error naming the offset.
    pub fn find_jpeg_app1(dat: &[u8]) -> Outcome<Option<&[u8]>> {
        for seg in res!(JpegSegments::new(dat)) {
            let seg = res!(seg);
            if seg.marker == 0xE1 && seg.body.len() > APP1_SIG.len()
                && &seg.body[0..APP1_SIG.len()] == APP1_SIG
            {
                return Ok(Some(&seg.body[APP1_SIG.len()..]));
            }
            if seg.marker == 0xDA {
                break; // Scan data follows; no more metadata segments.
            }
        }
        Ok(None)
    }

    /// Reads the frame dimensions from a JPEG start of frame marker, as `(width, height)`.
    ///
    /// This is the size a decoder will produce, which is not always what the EXIF sub-IFD
    /// claims, so a photo application wanting the truth should prefer it.
    pub fn jpeg_dimensions(dat: &[u8]) -> Outcome<Option<(u32, u32)>> {
        for seg in res!(JpegSegments::new(dat)) {
            let seg = res!(seg);
            let is_sof = matches!(seg.marker,
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
            if is_sof {
                if seg.body.len() < 5 {
                    return Err(err!(
                        "JPEG start of frame marker FF{:02X} at offset {}: body is {} bytes, \
                        at least 5 are needed for the precision and dimensions.",
                        seg.marker, seg.offset, seg.body.len();
                    Input, Invalid, TooSmall, Decode));
                }
                let h = u16::from_be_bytes([seg.body[1], seg.body[2]]) as u32;
                let w = u16::from_be_bytes([seg.body[3], seg.body[4]]) as u32;
                return Ok(Some((w, h)));
            }
            if seg.marker == 0xDA {
                break;
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct JpegSegment<'a> {
    pub marker: u8,         // the byte following FF
    pub offset: usize,      // of the FF that introduced the marker
    pub body: &'a [u8],     // excludes the two byte length field itself
}

/// Iteration stops after the start of scan marker, since entropy coded data follows it and
/// cannot be walked as segments.
pub struct JpegSegments<'a> {
    dat: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> JpegSegments<'a> {

    /// Verifies the start of image marker before anything is yielded.
    pub fn new(dat: &'a [u8]) -> Outcome<Self> {
        if dat.len() < 2 {
            return Err(err!(
                "JPEG stream: {} bytes available at offset 0, at least 2 are needed for the \
                start of image marker.", dat.len();
            Input, Invalid, TooSmall, Decode));
        }
        if dat[0] != 0xFF || dat[1] != 0xD8 {
            return Err(err!(
                "JPEG stream at offset 0: found {:02X}{:02X}, expected the start of image \
                marker FFD8.", dat[0], dat[1];
            Input, Invalid, Decode));
        }
        Ok(Self { dat, pos: 2, done: false })
    }
}

impl<'a> Iterator for JpegSegments<'a> {
    type Item = Outcome<JpegSegment<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        // Skip any fill bytes, which the standard permits before a marker.
        let mut p = self.pos;
        while p < self.dat.len() && self.dat[p] == 0xFF {
            p += 1;
        }
        if p == self.pos {
            // No FF at all, so either the stream ended or the structure is broken.
            self.done = true;
            if self.pos >= self.dat.len() {
                return None;
            }
            return Some(Err(err!(
                "JPEG stream at offset {}: expected a marker introducer FF, found {:02X}.",
                self.pos, self.dat[self.pos];
            Input, Invalid, Decode)));
        }
        if p >= self.dat.len() {
            self.done = true;
            return Some(Err(err!(
                "JPEG stream at offset {}: the stream ends inside a run of FF fill bytes \
                with no marker code.", self.pos;
            Input, Invalid, TooSmall, Decode)));
        }
        let marker = self.dat[p];
        let start = p - 1;
        p += 1;

        // Markers that stand alone and carry no length or body.
        if marker == 0x01 || marker == 0xD8 || (0xD0..=0xD7).contains(&marker) {
            self.pos = p;
            return Some(Ok(JpegSegment { marker, offset: start, body: &self.dat[p..p] }));
        }
        if marker == 0xD9 {
            self.done = true;
            return Some(Ok(JpegSegment { marker, offset: start, body: &self.dat[p..p] }));
        }
        if p + 2 > self.dat.len() {
            self.done = true;
            return Some(Err(err!(
                "JPEG segment FF{:02X} at offset {}: the stream ends before its two byte \
                length field.", marker, start;
            Input, Invalid, TooSmall, Decode)));
        }
        let len = u16::from_be_bytes([self.dat[p], self.dat[p + 1]]) as usize;
        if len < 2 {
            self.done = true;
            return Some(Err(err!(
                "JPEG segment FF{:02X} at offset {}: declared length {} is below the two \
                bytes the length field itself occupies.", marker, start, len;
            Input, Invalid, Decode)));
        }
        let body_start = p + 2;
        let body_end = p + len;
        if body_end > self.dat.len() {
            self.done = true;
            return Some(Err(err!(
                "JPEG segment FF{:02X} at offset {}: body runs to offset {} but the stream is \
                only {} bytes.", marker, start, body_end, self.dat.len();
            Input, Invalid, TooSmall, Decode)));
        }
        self.pos = body_end;
        if marker == 0xDA {
            self.done = true; // Entropy coded data follows the scan header.
        }
        Some(Ok(JpegSegment { marker, offset: start, body: &self.dat[body_start..body_end] }))
    }
}

/// A bounds-checked cursor over the TIFF block.
struct Reader<'a> {
    dat: &'a [u8],
    order: ByteOrder,
}

impl<'a> Reader<'a> {

    /// The offset is absolute within the TIFF block.
    fn u16_at(&self, off: usize, what: &str) -> Outcome<u16> {
        if off + 2 > self.dat.len() {
            return Err(err!(
                "{}: two bytes wanted at offset {} but the TIFF block is only {} bytes.",
                what, off, self.dat.len();
            Input, Invalid, TooSmall, Decode));
        }
        Ok(self.order.u16([self.dat[off], self.dat[off + 1]]))
    }

    /// The offset is absolute within the TIFF block.
    fn u32_at(&self, off: usize, what: &str) -> Outcome<u32> {
        if off + 4 > self.dat.len() {
            return Err(err!(
                "{}: four bytes wanted at offset {} but the TIFF block is only {} bytes.",
                what, off, self.dat.len();
            Input, Invalid, TooSmall, Decode));
        }
        Ok(self.order.u32([
            self.dat[off],
            self.dat[off + 1],
            self.dat[off + 2],
            self.dat[off + 3],
        ]))
    }

    /// Reads a sub-IFD, refusing an offset already visited.
    fn read_sub_ifd(
        &mut self,
        off:    u32,
        kind:   IfdKind,
        seen:   &mut BTreeSet<u32>,
        fields: &mut Vec<Field>,
    )
        -> Outcome<()>
    {
        if !seen.insert(off) {
            return Err(err!(
                "{} pointer: offset {} has already been read as another IFD, so the pointers \
                loop.", kind, off;
            Input, Invalid, Duplicate, Decode));
        }
        res!(self.read_ifd(off, kind, fields));
        Ok(())
    }

    /// Appends the IFD's fields and returns the pointer to the next in the chain.
    fn read_ifd(
        &mut self,
        off:    u32,
        kind:   IfdKind,
        fields: &mut Vec<Field>,
    )
        -> Outcome<u32>
    {
        let base = off as usize;
        let count = res!(self.u16_at(base, &fmt!("{} entry count", kind))) as u64;
        if count > MAX_IFD_ENTRIES {
            return Err(err!(
                "{} at offset {}: declares {} entries, above the {} entry ceiling.",
                kind, base, count, MAX_IFD_ENTRIES;
            Input, Invalid, Excessive, Decode));
        }
        let end = (base as u64) + 2 + count * ENTRY_LEN + 4;
        if end > self.dat.len() as u64 {
            return Err(err!(
                "{} at offset {}: {} entries plus the next pointer run to offset {} but the \
                TIFF block is only {} bytes.", kind, base, count, end, self.dat.len();
            Input, Invalid, TooSmall, Decode));
        }
        for i in 0..count as usize {
            let eoff = base + 2 + i * ENTRY_LEN as usize;
            let field = res!(self.read_entry(eoff, i, kind));
            fields.push(field);
        }
        let noff = base + 2 + count as usize * ENTRY_LEN as usize;
        let next = res!(self.u32_at(noff, &fmt!("{} next IFD pointer", kind)));
        Ok(next)
    }

    /// Reads one twelve byte IFD entry.
    fn read_entry(&self, eoff: usize, idx: usize, kind: IfdKind) -> Outcome<Field> {
        if eoff + ENTRY_LEN as usize > self.dat.len() {
            return Err(err!(
                "{} entry {}: the twelve byte entry at offset {} extends beyond the {} byte \
                TIFF block.", kind, idx, eoff, self.dat.len();
            Input, Invalid, TooSmall, Decode));
        }
        let tag = res!(self.u16_at(eoff, &fmt!("{} entry {} tag", kind, idx)));
        let typ = res!(self.u16_at(eoff + 2, &fmt!("{} entry {} type", kind, idx)));
        let count = res!(self.u32_at(eoff + 4, &fmt!("{} entry {} count", kind, idx)));
        let raw = [
            self.dat[eoff + 8],
            self.dat[eoff + 9],
            self.dat[eoff + 10],
            self.dat[eoff + 11],
        ];

        let unit = match type_size(typ) {
            Some(n) => n,
            None => {
                // An unrecognised type has no known width, so the payload cannot be located.
                // Keep the entry with its raw bytes rather than dropping it.
                return Ok(Field {
                    tag,
                    ifd: kind,
                    value: Value::Unknown { typ, count, raw },
                });
            },
        };
        let total = (count as u64) * (unit as u64);
        if total > self.dat.len() as u64 {
            return Err(err!(
                "{} entry {} at offset {} (tag 0x{:04X}, type {}): count {} needs {} bytes but \
                the TIFF block is only {} bytes.",
                kind, idx, eoff, tag, typ, count, total, self.dat.len();
            Input, Invalid, Excessive, Decode));
        }
        let total = total as usize;

        let body: &[u8] = if total <= 4 {
            &self.dat[eoff + 8..eoff + 8 + total]
        } else {
            let voff = self.order.u32(raw) as usize;
            let vend = voff + total;
            if vend > self.dat.len() {
                return Err(err!(
                    "{} entry {} at offset {} (tag 0x{:04X}, type {}): value block [{}..{}] \
                    extends beyond the {} byte TIFF block.",
                    kind, idx, eoff, tag, typ, voff, vend, self.dat.len();
                Input, Invalid, TooSmall, Decode));
            }
            &self.dat[voff..vend]
        };

        let value = res!(self.decode(typ, count as usize, body, eoff, idx, kind, tag));
        Ok(Field { tag, ifd: kind, value })
    }

    #[allow(clippy::too_many_arguments)]
    fn decode(
        &self,
        typ:    u16,
        count:  usize,
        body:   &[u8],
        eoff:   usize,
        idx:    usize,
        kind:   IfdKind,
        tag:    u16,
    )
        -> Outcome<Value>
    {
        let o = self.order;
        Ok(match typ {
            1 => Value::Byte(body.to_vec()),
            2 => {
                // ASCII fields are NUL terminated.  Some cameras write a string that is not
                // valid UTF-8, so the conversion is lossy rather than fatal.
                let cut = body.iter().position(|b| *b == 0).unwrap_or(body.len());
                Value::Ascii(String::from_utf8_lossy(&body[..cut]).trim_end().to_string())
            },
            3 => {
                let mut v = Vec::with_capacity(count);
                for i in 0..count {
                    v.push(o.u16([body[i * 2], body[i * 2 + 1]]));
                }
                Value::Short(v)
            },
            4 => Value::Long(res!(Self::u32s(o, body, count))),
            5 => {
                let mut v = Vec::with_capacity(count);
                for i in 0..count {
                    let n = o.u32([body[i * 8], body[i * 8 + 1], body[i * 8 + 2], body[i * 8 + 3]]);
                    let d = o.u32([body[i * 8 + 4], body[i * 8 + 5], body[i * 8 + 6], body[i * 8 + 7]]);
                    v.push(Rational { num: n, den: d });
                }
                Value::Rational(v)
            },
            6 => Value::SByte(body.iter().map(|b| *b as i8).collect()),
            7 => Value::Undefined(body.to_vec()),
            8 => {
                let mut v = Vec::with_capacity(count);
                for i in 0..count {
                    v.push(o.u16([body[i * 2], body[i * 2 + 1]]) as i16);
                }
                Value::SShort(v)
            },
            9 => Value::SLong(res!(Self::u32s(o, body, count)).into_iter()
                .map(|n| n as i32).collect()),
            10 => {
                let mut v = Vec::with_capacity(count);
                for i in 0..count {
                    let n = o.u32([body[i * 8], body[i * 8 + 1], body[i * 8 + 2], body[i * 8 + 3]]) as i32;
                    let d = o.u32([body[i * 8 + 4], body[i * 8 + 5], body[i * 8 + 6], body[i * 8 + 7]]) as i32;
                    v.push(SRational { num: n, den: d });
                }
                Value::SRational(v)
            },
            11 => Value::Float(res!(Self::u32s(o, body, count)).into_iter()
                .map(f32::from_bits).collect()),
            12 => {
                let mut v = Vec::with_capacity(count);
                for i in 0..count {
                    let lo = o.u32([body[i * 8], body[i * 8 + 1], body[i * 8 + 2], body[i * 8 + 3]]) as u64;
                    let hi = o.u32([body[i * 8 + 4], body[i * 8 + 5], body[i * 8 + 6], body[i * 8 + 7]]) as u64;
                    let bits = match o {
                        ByteOrder::Little	=> (hi << 32) | lo,
                        ByteOrder::Big		=> (lo << 32) | hi,
                    };
                    v.push(f64::from_bits(bits));
                }
                Value::Double(v)
            },
            13 => Value::Ifd(res!(Self::u32s(o, body, count))),
            _ => return Err(err!(
                "{} entry {} at offset {} (tag 0x{:04X}): type {} has a known width but no \
                decoder.", kind, idx, eoff, tag, typ;
            Bug, Unreachable, Decode)),
        })
    }

    fn u32s(o: ByteOrder, body: &[u8], count: usize) -> Outcome<Vec<u32>> {
        let mut v = Vec::with_capacity(count);
        for i in 0..count {
            v.push(o.u32([body[i * 4], body[i * 4 + 1], body[i * 4 + 2], body[i * 4 + 3]]));
        }
        Ok(v)
    }
}

/// `None` for a type this module does not know, whose payload cannot then be located.
pub fn type_size(typ: u16) -> Option<usize> {
    Some(match typ {
        1	=> 1,	// BYTE
        2	=> 1,	// ASCII
        3	=> 2,	// SHORT
        4	=> 4,	// LONG
        5	=> 8,	// RATIONAL
        6	=> 1,	// SBYTE
        7	=> 1,	// UNDEFINED
        8	=> 2,	// SSHORT
        9	=> 4,	// SLONG
        10	=> 8,	// SRATIONAL
        11	=> 4,	// FLOAT
        12	=> 8,	// DOUBLE
        13	=> 4,	// IFD
        _	=> return None,
    })
}

/// The typed view of the fields a photo application reaches for first.
///
/// Every member is optional, since no tag is guaranteed to be present.  Values are normalised:
/// coordinates are signed decimal degrees, altitude is signed metres, and exposure time is
/// seconds rather than the recorded ratio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhotoMeta {
    pub datetime_original: Option<String>,     // DateTimeOriginal, as YYYY:MM:DD HH:MM:SS
    pub create_date: Option<String>,           // CreateDate, when the image was digitised
    pub modify_date: Option<String>,           // ModifyDate from IFD0, the file's last write
    pub subsec_time_original: Option<String>,  // fractional seconds of datetime_original
    pub subsec_time_digitized: Option<String>, // fractional seconds of create_date
    pub offset_time_original: Option<String>,  // UTC offset of datetime_original, such as +10:00
    pub make: Option<String>,                  // camera manufacturer
    pub model: Option<String>,
    pub lens_model: Option<String>,
    pub orientation: Option<u16>,              // 1 to 8
    pub width: Option<u32>,                    // pixels
    pub height: Option<u32>,                   // pixels
    pub gps_latitude: Option<f64>,             // north positive
    pub gps_longitude: Option<f64>,            // east positive
    pub gps_altitude: Option<f64>,             // below sea level negative
    pub gps_datestamp: Option<String>,         // UTC date of the fix, as YYYY:MM:DD
    pub f_number: Option<f64>,
    pub exposure_time: Option<f64>,
    pub iso: Option<u32>,                      // arithmetic speed
    pub focal_length: Option<f64>,             // millimetres
    pub focal_length_35mm: Option<u32>,        // millimetres, for a 35 mm film frame
}

impl PhotoMeta {

    pub fn from_exif(exif: &Exif) -> Self {
        let mut m = Self::default();

        m.make			= Self::text(exif, IfdKind::Ifd0, tag::MAKE);
        m.model			= Self::text(exif, IfdKind::Ifd0, tag::MODEL);
        m.modify_date	= Self::text(exif, IfdKind::Ifd0, tag::DATE_TIME);
        m.orientation	= exif.field(IfdKind::Ifd0, tag::ORIENTATION)
            .and_then(|f| f.value.as_u32())
            .and_then(|n| u16::try_from(n).ok());

        m.datetime_original		= Self::text(exif, IfdKind::Exif, tag::DATE_TIME_ORIGINAL);
        m.create_date			= Self::text(exif, IfdKind::Exif, tag::CREATE_DATE);
        m.subsec_time_original	= Self::text(exif, IfdKind::Exif, tag::SUBSEC_TIME_ORIGINAL)
            .or_else(|| Self::text(exif, IfdKind::Exif, tag::SUBSEC_TIME));
        m.subsec_time_digitized	= Self::text(exif, IfdKind::Exif, tag::SUBSEC_TIME_DIGITIZED);
        m.offset_time_original	= Self::text(exif, IfdKind::Exif, tag::OFFSET_TIME_ORIGINAL)
            .or_else(|| Self::text(exif, IfdKind::Exif, tag::OFFSET_TIME));
        m.lens_model			= Self::text(exif, IfdKind::Exif, tag::LENS_MODEL);

        // The EXIF sub-IFD records the dimensions after any in-camera processing, so it is
        // preferred; IFD0 carries them for a plain TIFF.
        m.width = exif.field(IfdKind::Exif, tag::EXIF_IMAGE_WIDTH)
            .and_then(|f| f.value.as_u32())
            .or_else(|| exif.field(IfdKind::Ifd0, tag::IMAGE_WIDTH)
                .and_then(|f| f.value.as_u32()));
        m.height = exif.field(IfdKind::Exif, tag::EXIF_IMAGE_HEIGHT)
            .and_then(|f| f.value.as_u32())
            .or_else(|| exif.field(IfdKind::Ifd0, tag::IMAGE_LENGTH)
                .and_then(|f| f.value.as_u32()));

        m.f_number			= Self::number(exif, IfdKind::Exif, tag::F_NUMBER);
        m.exposure_time		= Self::number(exif, IfdKind::Exif, tag::EXPOSURE_TIME);
        m.focal_length		= Self::number(exif, IfdKind::Exif, tag::FOCAL_LENGTH);
        m.iso				= exif.field(IfdKind::Exif, tag::ISO_SPEED_RATINGS)
            .and_then(|f| f.value.as_u32());
        m.focal_length_35mm	= exif.field(IfdKind::Exif, tag::FOCAL_LENGTH_35MM)
            .and_then(|f| f.value.as_u32());

        m.gps_latitude = Self::coord(
            exif,
            tag::GPS_LATITUDE,
            tag::GPS_LATITUDE_REF,
            'S',
        );
        m.gps_longitude = Self::coord(
            exif,
            tag::GPS_LONGITUDE,
            tag::GPS_LONGITUDE_REF,
            'W',
        );
        m.gps_altitude = Self::number(exif, IfdKind::Gps, tag::GPS_ALTITUDE).map(|a| {
            let below = exif.field(IfdKind::Gps, tag::GPS_ALTITUDE_REF)
                .and_then(|f| f.value.as_u32())
                .map(|r| r == 1)
                .unwrap_or(false);
            if below { -a } else { a }
        });
        m.gps_datestamp = Self::text(exif, IfdKind::Gps, tag::GPS_DATESTAMP);

        m
    }

    /// Trims, and treats an empty string as absent.
    fn text(exif: &Exif, ifd: IfdKind, tag: u16) -> Option<String> {
        exif.field(ifd, tag)
            .and_then(|f| f.value.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn number(exif: &Exif, ifd: IfdKind, tag: u16) -> Option<f64> {
        exif.field(ifd, tag).and_then(|f| f.value.as_f64())
    }

    /// Converts a degrees, minutes and seconds triple plus a hemisphere into signed degrees.
    fn coord(exif: &Exif, val_tag: u16, ref_tag: u16, negative: char) -> Option<f64> {
        let parts = match exif.field(IfdKind::Gps, val_tag)
            .and_then(|f| f.value.as_f64_vec())
        {
            Some(p) if !p.is_empty()	=> p,
            _							=> return None,
        };
        let deg = parts.first().copied().unwrap_or(0.0);
        let min = parts.get(1).copied().unwrap_or(0.0);
        let sec = parts.get(2).copied().unwrap_or(0.0);
        let mut d = deg + min / 60.0 + sec / 3600.0;
        if !d.is_finite() {
            return None;
        }
        let hemi = exif.field(IfdKind::Gps, ref_tag)
            .and_then(|f| f.value.as_str())
            .and_then(|s| s.chars().next())
            .map(|c| c.to_ascii_uppercase());
        if hemi == Some(negative) {
            d = -d;
        }
        Some(d)
    }
}
