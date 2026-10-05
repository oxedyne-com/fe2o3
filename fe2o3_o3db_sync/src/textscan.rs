//! A read-only check of the JDAT text that an Ozone store holds, against the decoder as it now refuses.
//!
//! Steel and Daimond keep binary `Dat` values, and a value may carry text inside it. Every live
//! key is read, the value tree is walked (lists, maps and their keys, boxes, options, notes, user
//! kinds, tuples) and each `Str` or UTF-8 byte string that starts like JDAT text (`{`, `[` or `(`)
//! is decoded. The counts, the classes and the report are `oxedyne_fe2o3_jdat::string::scan`'s,
//! shared with the scan of framed text files (Oxegen's tables), and nothing a store holds is ever
//! printed: a refusal carries the key's tag, the text's number within the value, the line, the
//! column and the class.
//!
//! The scan is read-only, but a read-only open of an Ozone store with garbage collection off still
//! rewrites its `config.jdat`: scan a `cp -a` COPY, never a live store, and never with a second
//! process on the same store.

use crate::{
    prelude::*,
    comm::response::Wait,
};

use oxedyne_fe2o3_iop_db::api::ScanOpts;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
    string::scan::{
        self,
        Refused,
        TextScan,
    },
};


// Every string and UTF-8 byte string in a value, however deep.
fn gather<'a>(d: &'a Dat, out: &mut Vec<&'a str>) {
    fn each<'a>(ds: &'a [Dat], out: &mut Vec<&'a str>) {
        for d in ds {
            gather(d, out);
        }
    }
    match d {
        Dat::Str(s) => out.push(s.as_str()),
        Dat::BU8(b) | Dat::BU16(b) | Dat::BU32(b) | Dat::BU64(b) => {
            if let Ok(s) = std::str::from_utf8(b) {
                out.push(s);
            }
        },
        Dat::List(v)    => each(v, out),
        Dat::Map(m)     => {
            for (k, v) in m {
                gather(k, out);
                gather(v, out);
            }
        },
        Dat::OrdMap(m)  => {
            for (k, v) in m {
                gather(k.dat(), out);
                gather(v, out);
            }
        },
        Dat::Box(b)             => gather(b, out),
        Dat::Opt(o)             => {
            if let Some(x) = &**o {
                gather(x, out);
            }
        },
        Dat::ABox(_, b, _)      => gather(b, out),
        Dat::Usr(_, Some(b))    => gather(b, out),
        Dat::Tup2(t)    => each(&**t, out),
        Dat::Tup3(t)    => each(&**t, out),
        Dat::Tup4(t)    => each(&**t, out),
        Dat::Tup5(t)    => each(&**t, out),
        Dat::Tup6(t)    => each(&**t, out),
        Dat::Tup7(t)    => each(&**t, out),
        Dat::Tup8(t)    => each(&**t, out),
        Dat::Tup9(t)    => each(&**t, out),
        Dat::Tup10(t)   => each(&**t, out),
        _ => {},
    }
}

/// Reads every live key of an Ozone store and decodes the JDAT text its values hold.
///
/// The store must be a COPY, opened with garbage collection off and used by nobody else; this
/// function issues no write. A key whose newest record is a deletion reads as absent and is
/// passed over.
///
/// # Arguments
/// * `scan_wait` - how long each zone's scan may take; a large store needs a generous wait.
pub fn scan_o3db<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:        &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    scan_wait:  Wait,
    out:        &mut TextScan,
)
    -> Outcome<()>
{
    let entries = res!(api.scan_with_wait(&ScanOpts::all(), None, scan_wait));
    for (kdat, _, _) in &entries {
        let val = match res!(api.get_wait(kdat, None)) {
            None            => continue,
            Some((val, _))  => val,
        };
        out.records += 1;
        let mut found = Vec::new();
        gather(&val, &mut found);
        let mut rec = 0usize;
        for text in found {
            let t = text.trim_start();
            if !(t.starts_with('{') || t.starts_with('[') || t.starts_with('(')) {
                out.skipped += 1;
                continue;
            }
            rec += 1;
            out.texts += 1;
            if let Some((line, col, class)) = scan::refusal(text) {
                out.refuse(Refused {
                    store:  "o3db".to_string(),
                    rec,
                    at:     0,
                    hash:   scan::tag(fmt!("{}", kdat).as_bytes()),
                    line,
                    col,
                    class,
                });
            }
        }
    }
    Ok(())
}
