//! The room left on a filesystem, and the bytes a directory tree takes on it.
//!
//! [`room`] reads `statvfs` for the bytes an unprivileged writer may still use
//! (`f_bavail`, which leaves out the blocks reserved for root, so it is the figure an
//! application meets before it is told the disk is full) beside the filesystem's size.
//! [`bytes_under`] totals the regular files under a directory, which is what a copy of
//! that directory will write. A service that must keep room for a copy of its own store
//! (a nightly backup to the same disk, say) weighs the two before it accepts new bytes.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    path::Path,
};

/// One reading of a filesystem's room, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Room {
    pub avail: u64, // Bytes an unprivileged writer may still use.
    pub total: u64, // The filesystem's size.
}

/// The room on the filesystem holding `path`.
pub fn room(path: &Path) -> Outcome<Room> {
    let st = match nix::sys::statvfs::statvfs(path) {
        Ok(s)  => s,
        Err(e) => return Err(err!(e,
            "Reading the free room on the filesystem holding {:?}.", path;
            IO, File, Read)),
    };
    let frag = st.fragment_size() as u64;
    Ok(Room {
        avail: (st.blocks_available() as u64).saturating_mul(frag),
        total: (st.blocks() as u64).saturating_mul(frag),
    })
}

/// The bytes the regular files under `dir` take, by their lengths, walking every
/// directory below it. Symbolic links are not followed, so a link out of the tree
/// is neither counted nor walked.
pub fn bytes_under(dir: &Path) -> Outcome<u64> {
    let mut sum   = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match fs::read_dir(&d) {
            Ok(r)  => r,
            Err(e) => return Err(err!(e,
                "Listing {:?} to total the bytes under {:?}.", d, dir; IO, File, Read)),
        };
        for ent in rd {
            let ent = match ent {
                Ok(x)  => x,
                Err(e) => return Err(err!(e,
                    "Reading an entry of {:?}.", d; IO, File, Read)),
            };
            let ft = match ent.file_type() {
                Ok(t)  => t,
                Err(e) => return Err(err!(e,
                    "Reading the type of {:?}.", ent.path(); IO, File, Read)),
            };
            if ft.is_dir() {
                stack.push(ent.path());
            } else if ft.is_file() {
                let md = match ent.metadata() {
                    Ok(m)  => m,
                    Err(e) => return Err(err!(e,
                        "Reading the length of {:?}.", ent.path(); IO, File, Read)),
                };
                sum = sum.saturating_add(md.len());
            }
        }
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_is_totalled_and_the_disk_has_room() -> Outcome<()> {
        let base = std::env::temp_dir().join(fmt!("fe2o3_sys_room_{}", std::process::id()));
        let sub  = base.join("a").join("b");
        res!(fs::create_dir_all(&sub));
        res!(fs::write(base.join("one"), vec![0u8; 100]));
        res!(fs::write(sub.join("two"), vec![0u8; 23]));
        let n = res!(bytes_under(&base));
        let r = res!(room(&base));
        res!(fs::remove_dir_all(&base));
        assert_eq!(n, 123, "100 + 23 bytes under the tree");
        assert!(r.total > 0 && r.avail <= r.total, "{:?}", r);
        Ok(())
    }
}
