use oxedyne_fe2o3_hash::phash::{
    LumaGrid,
    PerceptualHash,
    hamming,
    luma_from_rgb,
    luma_from_rgba,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};

use std::{
    fs,
    path::PathBuf,
};


/// A greyscale image read from a portable greymap.
struct Pgm {
    dat: Vec<u8>,
    w:   usize,
    h:   usize,
}

/// Reads a binary portable greymap, the `P5` form with a maximum value of 255.
///
/// The format is deliberately trivial, which lets an external tool produce the fixtures and
/// leaves the decode entirely outside the library under test.
fn read_pgm(path: &PathBuf) -> Outcome<Pgm> {
    let raw = res!(fs::read(path), IO, File);
    if raw.len() < 2 || &raw[0..2] != b"P5" {
        return Err(err!(
            "{:?}: not a binary portable greymap, the first two bytes are {:02x?}.",
            path, &raw[0..raw.len().min(2)];
        Input, Invalid));
    }
    // Collect three whitespace separated fields after the magic, skipping comment lines.
    let mut fields: Vec<usize> = Vec::new();
    let mut i = 2usize;
    while fields.len() < 3 {
        while i < raw.len() && (raw[i] as char).is_whitespace() {
            i += 1;
        }
        if i < raw.len() && raw[i] == b'#' {
            while i < raw.len() && raw[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        while i < raw.len() && (raw[i] as char).is_ascii_digit() {
            i += 1;
        }
        if start == i {
            return Err(err!(
                "{:?}: the header ended at byte {} before three numeric fields were read.",
                path, i;
            Input, Invalid));
        }
        let s = res!(std::str::from_utf8(&raw[start..i]), Input, Invalid);
        fields.push(res!(s.parse::<usize>(), Input, Invalid));
    }
    i += 1; // The single whitespace byte that closes the header.
    let (w, h, max) = (fields[0], fields[1], fields[2]);
    if max != 255 {
        return Err(err!(
            "{:?}: maximum sample value {} is not supported, only 255 is.", path, max;
        Input, Invalid));
    }
    if raw.len() < i + w * h {
        return Err(err!(
            "{:?}: {} by {} needs {} sample bytes, only {} follow the header.",
            path, w, h, w * h, raw.len() - i;
        Input, Invalid, TooSmall));
    }
    Ok(Pgm { dat: raw[i..i + w * h].to_vec(), w, h })
}

/// Returns the path of a committed image fixture.
fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("test_images");
    p.push(name);
    p
}

/// Hashes a fixture both ways.
fn hash_fixture(name: &str) -> Outcome<(PerceptualHash, PerceptualHash)> {
    let img = res!(read_pgm(&fixture_path(name)));
    let grid = res!(LumaGrid::new(&img.dat, img.w, img.h));
    Ok((
        res!(PerceptualHash::dhash(&grid)),
        res!(PerceptualHash::phash(&grid)),
    ))
}

/// The three synthetic subjects held in the fixture directory.
const SUBJECTS: [&str; 3] = ["plasma", "gradient", "shapes"];

/// The four transforms an external tool applied to each subject.
const VARIANTS: [&str; 4] = ["half", "q40", "bright", "png2jpg"];

pub fn test_phash(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["A hash of itself is zero away 000", "all", "phash"], || {
        let px: Vec<u8> = (0..(48 * 32)).map(|i| ((i * 7) % 251) as u8).collect();
        let grid = res!(LumaGrid::new(&px, 48, 32));
        let d = res!(PerceptualHash::dhash(&grid));
        let p = res!(PerceptualHash::phash(&grid));
        req!(res!(d.distance(&d)), 0u32);
        req!(res!(p.distance(&p)), 0u32);
        // The two kinds are not comparable.
        if d.distance(&p).is_ok() {
            return Err(err!(
                "A difference hash was compared with a cosine transform hash."; Test, Invalid));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Malformed grids are refused 010", "all", "phash"], || {
        let px = [0u8; 16];
        if LumaGrid::new(&px, 0, 4).is_ok() {
            return Err(err!("A zero width grid was accepted."; Test, Invalid));
        }
        if LumaGrid::new(&px, 4, 0).is_ok() {
            return Err(err!("A zero height grid was accepted."; Test, Invalid));
        }
        match LumaGrid::new(&px, 8, 8) {
            Ok(_) => return Err(err!(
                "A 16 byte buffer was accepted as an 8 by 8 grid."; Test, Invalid)),
            Err(e) => test!("Short buffer refused: {}", e),
        }
        // A grid smaller than the reduction still hashes rather than failing.
        let tiny = [3u8, 200, 40, 250];
        let grid = res!(LumaGrid::new(&tiny, 2, 2));
        let _ = res!(PerceptualHash::dhash(&grid));
        let _ = res!(PerceptualHash::phash(&grid));
        Ok(())
    }));

    res!(test_it(filter, &["Colour conversion follows Rec. 601 020", "all", "phash"], || {
        // Pure red, green, blue and white, one pixel each.
        let rgb = [255u8, 0, 0,  0, 255, 0,  0, 0, 255,  255, 255, 255];
        let y = res!(luma_from_rgb(&rgb, 4, 1));
        req!(y, vec![76u8, 150, 29, 255]);
        let rgba = [255u8, 0, 0, 17,  0, 255, 0, 34,  0, 0, 255, 51,  255, 255, 255, 68];
        let ya = res!(luma_from_rgba(&rgba, 4, 1));
        req!(ya, vec![76u8, 150, 29, 255]);
        if luma_from_rgb(&rgb, 8, 1).is_ok() {
            return Err(err!("A short interleaved buffer was accepted."; Test, Invalid));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Variants of one subject stay close 030", "all", "phash"], || {
        // Every fixture in this test was produced by an external tool from the same master:
        // a half-size reduction, a quality forty re-encode, a ten per cent brightening, and a
        // lossless to lossy conversion.  A perceptual hash that did not survive these would be
        // of no use, so the distances are asserted, not merely printed.
        let mut d_same = Vec::new();
        let mut p_same = Vec::new();
        for subj in SUBJECTS {
            let (d0, p0) = res!(hash_fixture(&fmt!("{}_orig.pgm", subj)));
            for var in VARIANTS {
                let (d1, p1) = res!(hash_fixture(&fmt!("{}_{}.pgm", subj, var)));
                let dd = res!(d0.distance(&d1));
                let pd = res!(p0.distance(&p1));
                test!("{:>8} vs {:>8}: dhash {:>2}, phash {:>2}", subj, var, dd, pd);
                d_same.push(dd);
                p_same.push(pd);
                if dd > 12 {
                    return Err(err!(
                        "{} against its {} variant: difference hash distance {} is too large \
                        for the same image.", subj, var, dd;
                    Test, Mismatch));
                }
                if pd > 10 {
                    return Err(err!(
                        "{} against its {} variant: cosine transform hash distance {} is too \
                        large for the same image.", subj, var, pd;
                    Test, Mismatch));
                }
            }
        }
        let dmax = d_same.iter().copied().max().unwrap_or(0);
        let pmax = p_same.iter().copied().max().unwrap_or(0);
        let dsum: u32 = d_same.iter().sum();
        let psum: u32 = p_same.iter().sum();
        test!(
            "Same subject over {} pairs: dhash mean {:.2} max {}, phash mean {:.2} max {}.",
            d_same.len(),
            dsum as f64 / d_same.len() as f64, dmax,
            psum as f64 / p_same.len() as f64, pmax,
        );
        Ok(())
    }));

    res!(test_it(filter, &["Different subjects stay far apart 040", "all", "phash"], || {
        let mut d_diff = Vec::new();
        let mut p_diff = Vec::new();
        let mut names = Vec::new();
        for subj in SUBJECTS {
            for var in ["orig"].iter().chain(VARIANTS.iter()) {
                names.push((subj, *var, res!(hash_fixture(&fmt!("{}_{}.pgm", subj, var)))));
            }
        }
        for (i, (s1, v1, (d1, p1))) in names.iter().enumerate() {
            for (s2, v2, (d2, p2)) in names.iter().skip(i + 1) {
                if s1 == s2 {
                    continue;
                }
                let dd = res!(d1.distance(d2));
                let pd = res!(p1.distance(p2));
                d_diff.push(dd);
                p_diff.push(pd);
                if dd < 16 {
                    return Err(err!(
                        "{}_{} against {}_{}: difference hash distance {} is too small for \
                        unrelated images.", s1, v1, s2, v2, dd;
                    Test, Mismatch));
                }
                if pd < 16 {
                    return Err(err!(
                        "{}_{} against {}_{}: cosine transform hash distance {} is too small \
                        for unrelated images.", s1, v1, s2, v2, pd;
                    Test, Mismatch));
                }
            }
        }
        let dmin = d_diff.iter().copied().min().unwrap_or(0);
        let pmin = p_diff.iter().copied().min().unwrap_or(0);
        let dsum: u32 = d_diff.iter().sum();
        let psum: u32 = p_diff.iter().sum();
        test!(
            "Unrelated subjects over {} pairs: dhash mean {:.2} min {}, phash mean {:.2} min {}.",
            d_diff.len(),
            dsum as f64 / d_diff.len() as f64, dmin,
            psum as f64 / p_diff.len() as f64, pmin,
        );
        Ok(())
    }));

    res!(test_it(filter, &["The two populations do not overlap 050", "all", "phash"], || {
        // The point of a threshold is that it exists.  Collect the worst same-subject distance
        // and the best unrelated distance, and insist on a gap between them.
        let mut worst_same = (0u32, 0u32);
        let mut best_diff = (64u32, 64u32);
        let mut all = Vec::new();
        for subj in SUBJECTS {
            for var in ["orig"].iter().chain(VARIANTS.iter()) {
                all.push((subj, res!(hash_fixture(&fmt!("{}_{}.pgm", subj, var)))));
            }
        }
        for (i, (s1, (d1, p1))) in all.iter().enumerate() {
            for (s2, (d2, p2)) in all.iter().skip(i + 1) {
                let dd = res!(d1.distance(d2));
                let pd = res!(p1.distance(p2));
                if s1 == s2 {
                    worst_same = (worst_same.0.max(dd), worst_same.1.max(pd));
                } else {
                    best_diff = (best_diff.0.min(dd), best_diff.1.min(pd));
                }
            }
        }
        test!(
            "Separation: dhash worst same {} against best unrelated {}; \
            phash worst same {} against best unrelated {}.",
            worst_same.0, best_diff.0, worst_same.1, best_diff.1,
        );
        if worst_same.0 >= best_diff.0 {
            return Err(err!(
                "The difference hash populations overlap: worst same-subject distance {} is not \
                below the best unrelated distance {}.", worst_same.0, best_diff.0;
            Test, Mismatch));
        }
        if worst_same.1 >= best_diff.1 {
            return Err(err!(
                "The cosine transform hash populations overlap: worst same-subject distance {} \
                is not below the best unrelated distance {}.", worst_same.1, best_diff.1;
            Test, Mismatch));
        }
        Ok(())
    }));

    res!(test_it(filter, &["Hamming counts differing bits 060", "all", "phash"], || {
        req!(hamming(0, 0), 0u32);
        req!(hamming(u64::MAX, 0), 64u32);
        req!(hamming(0b1011, 0b0001), 2u32);
        req!(fmt!("{}", PerceptualHash::DHash(0x0123456789abcdef)), "d:0123456789abcdef");
        req!(fmt!("{}", PerceptualHash::PHash(0x0123456789abcdef)), "p:0123456789abcdef");
        Ok(())
    }));

    Ok(())
}
