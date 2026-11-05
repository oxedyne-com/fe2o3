//! The mount table as the kernel shows it to this process (`/proc/self/mountinfo`), parsed
//! without following or touching any mount.  A mount point's octal escapes (`\040` for a space,
//! `\011`, `\012`, `\134`) are undone, so a point is the path as bytes.
use oxedyne_fe2o3_core::prelude::*;

use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{
        Path,
        PathBuf,
    },
};

/// One line of `mountinfo`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mount {
    pub id:     u64,
    pub parent: u64,
    pub major:  u32,
    pub minor:  u32,
    pub root:   PathBuf,    // the root of the mount within its filesystem
    pub point:  PathBuf,    // where it is mounted, in this process's namespace
    pub fstype: String,
}

/// The mounts of this process's mount namespace, in the kernel's order (a parent before its
/// children).
pub fn read() -> Outcome<Vec<Mount>> {
    let b = res!(std::fs::read("/proc/self/mountinfo").map_err(|e| err!(e,
        "Reading /proc/self/mountinfo failed."; IO, File)));
    parse(&b)
}

pub fn parse(b: &[u8]) -> Outcome<Vec<Mount>> {
    let mut out = Vec::new();
    for (i, l) in b.split(|c| *c == b'\n').enumerate() {
        if l.is_empty() {
            continue;
        }
        let f: Vec<&[u8]> = l.split(|c| *c == b' ').collect();
        // fixed: proc(5): id parent major:minor root point options [optional...] - fstype ...
        let sep = f.iter().position(|x| *x == b"-");
        let (sep, mm) = match (sep, f.get(2)) {
            (Some(s), Some(mm)) if f.len() > s + 1 && s >= 6 => (s, *mm),
            _ => return Err(err!("mountinfo line {} has too few fields.", i + 1; Invalid, Input)),
        };
        let num = |x: &[u8]| std::str::from_utf8(x).ok().and_then(|s| s.parse::<u64>().ok());
        let mut mmi = mm.split(|c| *c == b':');
        let (id, parent, major, minor) = match (num(f[0]), num(f[1]),
            mmi.next().and_then(num), mmi.next().and_then(num))
        {
            (Some(a), Some(p), Some(ma), Some(mi)) => (a, p, ma as u32, mi as u32),
            _ => return Err(err!("mountinfo line {} has a bad number.", i + 1; Invalid, Input)),
        };
        out.push(Mount {
            id, parent, major, minor,
            root:   res!(unescape(f[3])),
            point:  res!(unescape(f[4])),
            fstype: String::from_utf8_lossy(f[sep + 1]).to_string(),
        });
    }
    Ok(out)
}

/// The mounts whose point is `at` or under it, by path components (so `/home/u2` is not under
/// `/home/u`).
pub fn at_or_under<'a>(ms: &'a [Mount], at: &Path) -> Vec<&'a Mount> {
    ms.iter().filter(|m| m.point.starts_with(at)).collect()
}

fn unescape(x: &[u8]) -> Outcome<PathBuf> {
    let mut o = Vec::with_capacity(x.len());
    let mut i = 0;
    while i < x.len() {
        if x[i] == b'\\' {
            let d = match x.get(i + 1..i + 4) {
                Some(d) if d.iter().all(|c| (b'0'..=b'7').contains(c)) => d,
                _ => return Err(err!("A bad escape in mountinfo field {:?}.",
                    String::from_utf8_lossy(x); Invalid, Input)),
            };
            let v = ((d[0] - b'0') as u32) << 6 | ((d[1] - b'0') as u32) << 3 | (d[2] - b'0') as u32;
            if v > 255 {
                return Err(err!("A bad escape in mountinfo field {:?}.",
                    String::from_utf8_lossy(x); Invalid, Input));
            }
            o.push(v as u8);
            i += 4;
        } else {
            o.push(x[i]);
            i += 1;
        }
    }
    Ok(PathBuf::from(OsString::from_vec(o)))
}
