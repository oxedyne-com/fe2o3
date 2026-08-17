//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::exif::{
    ByteOrder,
    Exif,
    IfdKind,
    PhotoMeta,
    Rational,
    Value,
    tag,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};

use std::{
    fs,
    path::PathBuf,
};


fn u16b(o: ByteOrder, v: u16) -> [u8; 2] {
    match o {
        ByteOrder::Little	=> v.to_le_bytes(),
        ByteOrder::Big		=> v.to_be_bytes(),
    }
}

fn u32b(o: ByteOrder, v: u32) -> [u8; 4] {
    match o {
        ByteOrder::Little	=> v.to_le_bytes(),
        ByteOrder::Big		=> v.to_be_bytes(),
    }
}

/// A run of unsigned ratios, in the given order.
fn rats(o: ByteOrder, pairs: &[(u32, u32)]) -> Vec<u8> {
    let mut v = Vec::new();
    for (n, d) in pairs {
        v.extend_from_slice(&u32b(o, *n));
        v.extend_from_slice(&u32b(o, *d));
    }
    v
}

/// One entry of a hand-built IFD, its payload already in the target byte order.
struct Entry {
    tag:    u16,
    typ:    u16,
    count:  u32,
    dat:    Vec<u8>,
}

impl Entry {

    /// NUL terminated, as the standard requires.
    fn ascii(tag: u16, s: &str) -> Self {
        let mut dat = s.as_bytes().to_vec();
        dat.push(0);
        let count = dat.len() as u32;
        Self { tag, typ: 2, count, dat }
    }

    fn short(o: ByteOrder, tag: u16, v: u16) -> Self {
        Self { tag, typ: 3, count: 1, dat: u16b(o, v).to_vec() }
    }

    fn long(o: ByteOrder, tag: u16, v: u32) -> Self {
        Self { tag, typ: 4, count: 1, dat: u32b(o, v).to_vec() }
    }

    fn byte(tag: u16, v: u8) -> Self {
        Self { tag, typ: 1, count: 1, dat: vec![v] }
    }

    fn rational(o: ByteOrder, tag: u16, pairs: &[(u32, u32)]) -> Self {
        Self { tag, typ: 5, count: pairs.len() as u32, dat: rats(o, pairs) }
    }

    /// A type code this library does not recognise.
    fn odd_type(tag: u16, typ: u16, count: u32, raw: [u8; 4]) -> Self {
        Self { tag, typ, count, dat: raw.to_vec() }
    }
}

fn ifd_len(n: usize) -> usize {
    2 + 12 * n + 4
}

/// Oversized payloads are appended to `heap`, which begins at `hoff`.
fn write_ifd(
    o:      ByteOrder,
    ents:   &[Entry],
    next:   u32,
    heap:   &mut Vec<u8>,
    hoff:   usize,
)
    -> Vec<u8>
{
    let mut out = Vec::new();
    out.extend_from_slice(&u16b(o, ents.len() as u16));
    for e in ents {
        out.extend_from_slice(&u16b(o, e.tag));
        out.extend_from_slice(&u16b(o, e.typ));
        out.extend_from_slice(&u32b(o, e.count));
        if e.dat.len() <= 4 {
            let mut inline = [0u8; 4];
            inline[..e.dat.len()].copy_from_slice(&e.dat);
            out.extend_from_slice(&inline);
        } else {
            let at = hoff + heap.len();
            out.extend_from_slice(&u32b(o, at as u32));
            heap.extend_from_slice(&e.dat);
            if heap.len() % 2 == 1 {
                heap.push(0); // Keep payloads on even boundaries, as cameras do.
            }
        }
    }
    out.extend_from_slice(&u32b(o, next));
    out
}

/// Builds a complete TIFF block carrying a known set of values in the given byte order.
///
/// The layout is IFD0, then the EXIF sub-IFD, the GPS sub-IFD and IFD1, then a heap of the
/// payloads too large to sit inside an entry.
fn build_fixture(o: ByteOrder) -> Vec<u8> {

    let n0 = 6usize;	// IFD0 entry count, including the two sub-IFD pointers
    let ne = 12usize;	// EXIF sub-IFD entry count
    let ng = 7usize;	// GPS sub-IFD entry count
    let n1 = 2usize;	// IFD1 entry count

    let off0 = 8usize;
    let offe = off0 + ifd_len(n0);
    let offg = offe + ifd_len(ne);
    let off1 = offg + ifd_len(ng);
    let hoff = off1 + ifd_len(n1);

    let ifd0 = vec![
        Entry::ascii(tag::MAKE, "Oxide Optics"),
        Entry::ascii(tag::MODEL, "Model 7 Field"),
        Entry::short(o, tag::ORIENTATION, 6),
        Entry::ascii(tag::DATE_TIME, "2019:04:07 13:45:09"),
        Entry::long(o, tag::EXIF_IFD_POINTER, offe as u32),
        Entry::long(o, tag::GPS_IFD_POINTER, offg as u32),
    ];
    let exif = vec![
        Entry::rational(o, tag::EXPOSURE_TIME, &[(1, 250)]),
        Entry::rational(o, tag::F_NUMBER, &[(28, 10)]),
        Entry::short(o, tag::ISO_SPEED_RATINGS, 400),
        Entry::ascii(tag::DATE_TIME_ORIGINAL, "2019:04:07 13:45:02"),
        Entry::ascii(tag::CREATE_DATE, "2019:04:07 13:45:03"),
        Entry::ascii(tag::SUBSEC_TIME_ORIGINAL, "880"),
        Entry::ascii(tag::OFFSET_TIME_ORIGINAL, "+08:00"),
        Entry::rational(o, tag::FOCAL_LENGTH, &[(350, 10)]),
        Entry::long(o, tag::EXIF_IMAGE_WIDTH, 4032),
        Entry::long(o, tag::EXIF_IMAGE_HEIGHT, 3024),
        // A tag this library has no name for, which must survive rather than be dropped.
        Entry::short(o, 0xBEEF, 7),
        // A type code this library has no decoder for, likewise.
        Entry::odd_type(0xBEEE, 199, 4, [0xDE, 0xAD, 0xBE, 0xEF]),
    ];
    let gps = vec![
        Entry::ascii(tag::GPS_LATITUDE_REF, "S"),
        Entry::rational(o, tag::GPS_LATITUDE, &[(31, 1), (57, 1), (1234, 100)]),
        Entry::ascii(tag::GPS_LONGITUDE_REF, "E"),
        Entry::rational(o, tag::GPS_LONGITUDE, &[(115, 1), (51, 1), (5678, 100)]),
        Entry::byte(tag::GPS_ALTITUDE_REF, 1),
        Entry::rational(o, tag::GPS_ALTITUDE, &[(4567, 100)]),
        Entry::ascii(tag::GPS_DATESTAMP, "2019:04:07"),
    ];
    let ifd1 = vec![
        Entry::long(o, tag::JPEG_INTERCHANGE_FORMAT, 900),
        Entry::long(o, tag::JPEG_INTERCHANGE_LENGTH, 128),
    ];

    let mut heap = Vec::new();
    let b0 = write_ifd(o, &ifd0, off1 as u32, &mut heap, hoff);
    let be = write_ifd(o, &exif, 0, &mut heap, hoff);
    let bg = write_ifd(o, &gps, 0, &mut heap, hoff);
    let b1 = write_ifd(o, &ifd1, 0, &mut heap, hoff);

    let mut out = Vec::new();
    match o {
        ByteOrder::Little	=> out.extend_from_slice(b"II"),
        ByteOrder::Big		=> out.extend_from_slice(b"MM"),
    }
    out.extend_from_slice(&u16b(o, 42));
    out.extend_from_slice(&u32b(o, off0 as u32));
    out.extend_from_slice(&b0);
    out.extend_from_slice(&be);
    out.extend_from_slice(&bg);
    out.extend_from_slice(&b1);
    out.extend_from_slice(&heap);
    out
}

/// Names the field on failure.
fn near(what: &str, got: Option<f64>, want: f64, tol: f64) -> Outcome<()> {
    match got {
        Some(g) if (g - want).abs() <= tol => Ok(()),
        Some(g) => Err(err!(
            "{}: parsed {}, expected {} to within {}.", what, g, want, tol;
        Test, Mismatch)),
        None => Err(err!(
            "{}: no value parsed, expected {}.", what, want;
        Test, Missing)),
    }
}

fn check_fixture_meta(m: &PhotoMeta) -> Outcome<()> {
    req!(m.make.as_deref(), Some("Oxide Optics"));
    req!(m.model.as_deref(), Some("Model 7 Field"));
    req!(m.orientation, Some(6u16));
    req!(m.datetime_original.as_deref(), Some("2019:04:07 13:45:02"));
    req!(m.create_date.as_deref(), Some("2019:04:07 13:45:03"));
    req!(m.modify_date.as_deref(), Some("2019:04:07 13:45:09"));
    req!(m.subsec_time_original.as_deref(), Some("880"));
    req!(m.offset_time_original.as_deref(), Some("+08:00"));
    req!(m.width, Some(4032u32));
    req!(m.height, Some(3024u32));
    req!(m.iso, Some(400u32));
    req!(m.gps_datestamp.as_deref(), Some("2019:04:07"));
    res!(near("FNumber", m.f_number, 2.8, 1e-9));
    res!(near("ExposureTime", m.exposure_time, 0.004, 1e-9));
    res!(near("FocalLength", m.focal_length, 35.0, 1e-9));
    // 31 degrees 57 minutes 12.34 seconds south.
    res!(near("GPSLatitude", m.gps_latitude, -31.953427777777, 1e-9));
    // 115 degrees 51 minutes 56.78 seconds east.
    res!(near("GPSLongitude", m.gps_longitude, 115.865772222222, 1e-9));
    res!(near("GPSAltitude", m.gps_altitude, -45.67, 1e-9));
    Ok(())
}

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("test_images");
    p.push(name);
    p
}

pub fn test_exif(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Synthetic TIFF, both byte orders 000", "all", "exif"], || {
        for o in [ByteOrder::Little, ByteOrder::Big] {
            let dat = build_fixture(o);
            let exif = res!(Exif::from_tiff(&dat));
            req!(exif.order, o);
            let m = exif.meta();
            res!(check_fixture_meta(&m));

            // The thumbnail pointer pair from IFD1.
            match exif.thumbnail {
                Some((off, len)) => {
                    req!(off, 900u32);
                    req!(len, 128u32);
                },
                None => return Err(err!(
                    "{}: IFD1 thumbnail offset and length were not recovered.", o;
                Test, Missing)),
            }

            // Every IFD must have been reached.
            for (kind, n) in [
                (IfdKind::Ifd0, 6usize),
                (IfdKind::Exif, 12),
                (IfdKind::Gps,	7),
                (IfdKind::Ifd1, 2),
            ] {
                req!(exif.ifd(kind).len(), n, "in {}", kind);
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Unknown tags and types survive 010", "all", "exif"], || {
        for o in [ByteOrder::Little, ByteOrder::Big] {
            let dat = build_fixture(o);
            let exif = res!(Exif::from_tiff(&dat));

            // An unnamed tag of a known type keeps its decoded value.
            match exif.field(IfdKind::Exif, 0xBEEF) {
                Some(f) => req!(f.value, Value::Short(vec![7])),
                None => return Err(err!(
                    "{}: the unnamed tag 0xBEEF was dropped.", o; Test, Missing)),
            }
            // A tag of an unknown type keeps its raw four bytes.
            match exif.field(IfdKind::Exif, 0xBEEE) {
                Some(f) => req!(
                    f.value,
                    Value::Unknown { typ: 199, count: 4, raw: [0xDE, 0xAD, 0xBE, 0xEF] }
                ),
                None => return Err(err!(
                    "{}: the tag 0xBEEE of unknown type was dropped.", o; Test, Missing)),
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Ratios keep their exact terms 020", "all", "exif"], || {
        let dat = build_fixture(ByteOrder::Big);
        let exif = res!(Exif::from_tiff(&dat));
        match exif.field(IfdKind::Exif, tag::EXPOSURE_TIME) {
            Some(f) => req!(f.value, Value::Rational(vec![Rational { num: 1, den: 250 }])),
            None => return Err(err!("ExposureTime was not parsed."; Test, Missing)),
        }
        match exif.field(IfdKind::Gps, tag::GPS_LATITUDE) {
            Some(f) => req!(f.value, Value::Rational(vec![
                Rational { num: 31, den: 1 },
                Rational { num: 57, den: 1 },
                Rational { num: 1234, den: 100 },
            ])),
            None => return Err(err!("GPSLatitude was not parsed."; Test, Missing)),
        }
        Ok(())
    }));

    res!(test_it(filter, &["Truncation at every boundary 030", "all", "exif"], || {
        // A parse of any prefix must return, never panic and never invent a value.  Where a
        // prefix does parse, the values it yields must agree with the whole.
        let full = build_fixture(ByteOrder::Little);
        let whole = res!(Exif::from_tiff(&full)).meta();
        let mut parsed = 0usize;
        let mut refused = 0usize;
        for n in 0..full.len() {
            match Exif::from_tiff(&full[..n]) {
                Ok(exif) => {
                    let m = exif.meta();
                    // Any field a truncated block does produce must match the whole.
                    if m.make.is_some() {
                        req!(m.make, whole.make, "at truncation length {}", n);
                    }
                    if m.gps_latitude.is_some() {
                        res!(near("GPSLatitude", m.gps_latitude, -31.953427777777, 1e-9));
                    }
                    parsed += 1;
                },
                Err(_) => refused += 1,
            }
        }
        req!(parsed + refused, full.len());
        // The header alone cannot yield a complete IFD, so most prefixes must be refused.
        if refused == 0 {
            return Err(err!(
                "No truncated prefix of a {} byte block was refused, which cannot be right.",
                full.len();
            Test, Invalid));
        }
        test!("Truncation sweep: {} prefixes parsed, {} refused.", parsed, refused);
        Ok(())
    }));

    res!(test_it(filter, &["Truncated JPEG at every boundary 040", "all", "exif"], || {
        let dat = res!(fs::read(fixture_path("exif_MM.jpg")), IO, File);
        let mut parsed = 0usize;
        let mut refused = 0usize;
        for n in 0..dat.len() {
            match Exif::from_jpeg(&dat[..n]) {
                Ok(_)	=> parsed += 1,
                Err(_)	=> refused += 1,
            }
        }
        req!(parsed + refused, dat.len());
        test!("JPEG truncation sweep: {} prefixes parsed, {} refused.", parsed, refused);
        Ok(())
    }));

    res!(test_it(filter, &["A looping IFD pointer is refused 050", "all", "exif"], || {
        let o = ByteOrder::Little;
        let mut dat = build_fixture(o);
        // IFD0 sits at offset 8; point its next-IFD field back at itself.
        let n0 = o.u16([dat[8], dat[9]]) as usize;
        let noff = 8 + 2 + n0 * 12;
        dat[noff..noff + 4].copy_from_slice(&u32b(o, 8));
        match Exif::from_tiff(&dat) {
            Ok(_) => Err(err!(
                "A chain whose IFD0 points back at itself was accepted."; Test, Invalid)),
            Err(e) => {
                let s = fmt!("{}", e);
                if !s.contains("loops") {
                    return Err(err!(
                        "The error for a looping chain did not name the loop: {}", s;
                    Test, Mismatch));
                }
                test!("Looping chain refused: {}", s);
                Ok(())
            },
        }
    }));

    res!(test_it(filter, &["A sub-IFD pointing at IFD0 is refused 060", "all", "exif"], || {
        let o = ByteOrder::Big;
        let mut dat = build_fixture(o);
        // The fifth IFD0 entry is the EXIF sub-IFD pointer; aim it at IFD0.
        let voff = 8 + 2 + 4 * 12 + 8;
        dat[voff..voff + 4].copy_from_slice(&u32b(o, 8));
        match Exif::from_tiff(&dat) {
            Ok(_) => Err(err!(
                "A sub-IFD pointer aimed at IFD0 was accepted."; Test, Invalid)),
            Err(e) => {
                test!("Self-referential sub-IFD refused: {}", e);
                Ok(())
            },
        }
    }));

    res!(test_it(filter, &["An overflowing entry count is refused 070", "all", "exif"], || {
        let o = ByteOrder::Little;
        let mut dat = build_fixture(o);
        // The second EXIF entry is FNumber, a RATIONAL; claim four thousand million of them.
        let offe = 8 + ifd_len(6);
        let coff = offe + 2 + 12 + 4;
        dat[coff..coff + 4].copy_from_slice(&u32b(o, 0xFFFF_FFFF));
        match Exif::from_tiff(&dat) {
            Ok(_) => Err(err!(
                "An entry claiming 4294967295 ratios was accepted."; Test, Invalid)),
            Err(e) => {
                let s = fmt!("{}", e);
                if !s.contains("4294967295") {
                    return Err(err!(
                        "The error for an overflowing count did not name it: {}", s;
                    Test, Mismatch));
                }
                test!("Overflowing count refused: {}", s);
                Ok(())
            },
        }
    }));

    res!(test_it(filter, &["An absurd IFD entry count is refused 080", "all", "exif"], || {
        let o = ByteOrder::Little;
        let mut dat = build_fixture(o);
        dat[8..10].copy_from_slice(&u16b(o, 0xFFFF));
        match Exif::from_tiff(&dat) {
            Ok(_) => Err(err!(
                "An IFD declaring 65535 entries was accepted."; Test, Invalid)),
            Err(e) => {
                test!("Absurd entry count refused: {}", e);
                Ok(())
            },
        }
    }));

    res!(test_it(filter, &["A value offset past the block is refused 090", "all", "exif"], || {
        let o = ByteOrder::Big;
        let mut dat = build_fixture(o);
        // The first IFD0 entry is Make, whose payload lives on the heap; move it out of range.
        let voff = 8 + 2 + 8;
        dat[voff..voff + 4].copy_from_slice(&u32b(o, 0x7FFF_0000));
        match Exif::from_tiff(&dat) {
            Ok(_) => Err(err!(
                "A value offset beyond the block was accepted."; Test, Invalid)),
            Err(e) => {
                let s = fmt!("{}", e);
                if !s.contains("extends beyond") {
                    return Err(err!(
                        "The error for an out of range value offset was unclear: {}", s;
                    Test, Mismatch));
                }
                test!("Out of range value offset refused: {}", s);
                Ok(())
            },
        }
    }));

    res!(test_it(filter, &["A bad header is refused 100", "all", "exif"], || {
        // Neither II nor MM.
        match Exif::from_tiff(b"XX\x00\x2a\x00\x00\x00\x08") {
            Ok(_)	=> return Err(err!("A header with no byte order mark was accepted.";
                        Test, Invalid)),
            Err(e)	=> test!("Bad byte order mark refused: {}", e),
        }
        // Wrong magic number.
        match Exif::from_tiff(b"MM\x00\x2b\x00\x00\x00\x08") {
            Ok(_)	=> return Err(err!("A header with magic 43 was accepted."; Test, Invalid)),
            Err(e)	=> test!("Bad magic refused: {}", e),
        }
        // Too short for a header at all.
        match Exif::from_tiff(b"MM\x00") {
            Ok(_)	=> return Err(err!("A three byte header was accepted."; Test, Invalid)),
            Err(e)	=> test!("Short header refused: {}", e),
        }
        // Not a JPEG and not a TIFF.
        match Exif::from_bytes(b"\x89PNG\r\n\x1a\n") {
            Ok(_)	=> return Err(err!("A PNG signature was accepted."; Test, Invalid)),
            Err(e)	=> test!("Foreign signature refused: {}", e),
        }
        Ok(())
    }));

    res!(test_it(filter, &["A JPEG without EXIF yields None 110", "all", "exif"], || {
        // Start of image, a comment segment, then end of image.
        let dat = b"\xFF\xD8\xFF\xFE\x00\x05hi!\xFF\xD9";
        match res!(Exif::from_jpeg(dat)) {
            Some(_)	=> Err(err!("EXIF was reported for a JPEG that has none."; Test, Invalid)),
            None	=> Ok(()),
        }
    }));

    res!(test_it(filter, &["JPEG fixtures match the reference tool 120", "all", "exif"], || {
        // The expected values below are those reported by exiftool 13.50 for these two files,
        // which were written by that tool, one in each byte order.  See test_images/README.txt.
        for (name, order) in [
            ("exif_MM.jpg", ByteOrder::Big),
            ("exif_II.jpg", ByteOrder::Little),
        ] {
            let dat = res!(fs::read(fixture_path(name)), IO, File);
            let exif = match res!(Exif::from_jpeg(&dat)) {
                Some(e) => e,
                None => return Err(err!(
                    "{}: no EXIF APP1 segment was found.", name; Test, Missing)),
            };
            req!(exif.order, order, "in {}", name);
            let m = exif.meta();
            req!(m.make.as_deref(), Some("Oxide Optics"), "in {}", name);
            req!(m.model.as_deref(), Some("Model 7 Field"), "in {}", name);
            req!(m.orientation, Some(6u16), "in {}", name);
            req!(m.datetime_original.as_deref(), Some("2019:04:07 13:45:02"), "in {}", name);
            req!(m.create_date.as_deref(), Some("2019:04:07 13:45:03"), "in {}", name);
            req!(m.subsec_time_original.as_deref(), Some("880"), "in {}", name);
            req!(m.iso, Some(400u32), "in {}", name);
            req!(m.width, Some(24u32), "in {}", name);
            req!(m.height, Some(16u32), "in {}", name);
            req!(m.lens_model.as_deref(), Some("35mm f/2 Prime"), "in {}", name);
            res!(near("ExposureTime", m.exposure_time, 0.004, 1e-9));
            res!(near("FNumber", m.f_number, 2.8, 1e-9));
            res!(near("FocalLength", m.focal_length, 35.0, 1e-9));
            // exiftool prints the composite values -31.9534276999917, 115.865772200058
            // and -45.67 for these three.
            res!(near("GPSLatitude", m.gps_latitude, -31.9534276999917, 1e-7));
            res!(near("GPSLongitude", m.gps_longitude, 115.865772200058, 1e-7));
            res!(near("GPSAltitude", m.gps_altitude, -45.67, 1e-6));

            // The frame dimensions from the start of frame marker, which identify reports as
            // Image Width 24 and Image Height 16.
            match res!(Exif::jpeg_dimensions(&dat)) {
                Some((w, h)) => {
                    req!(w, 24u32, "in {}", name);
                    req!(h, 16u32, "in {}", name);
                },
                None => return Err(err!(
                    "{}: no start of frame marker was found.", name; Test, Missing)),
            }
        }
        Ok(())
    }));

    Ok(())
}
