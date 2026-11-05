#![cfg(target_os = "linux")]

use oxedyne_fe2o3_sys::mounts::{
    at_or_under,
    parse,
    read,
};

use std::path::Path;

const TABLE: &[u8] = b"\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
40 22 8:17 / /home/jason/.cache rw,relatime shared:20 - ext4 /dev/sda1 rw
41 22 8:17 /lanes /home/jason/lanes rw,relatime - ext4 /dev/sda1 rw
42 22 0:50 / /home/jason2 rw - tmpfs tmpfs rw
43 22 0:51 / /home/jason/my\\040disk rw master:3 shared:9 - fuse.x x rw
";

#[test]
fn test_mounts_parse_and_select_under_a_folder() {
    let ms = parse(TABLE).expect("parse");
    assert_eq!(ms.len(), 5);
    assert_eq!((ms[1].major, ms[1].minor), (8, 17));
    assert_eq!(ms[2].root, Path::new("/lanes"));
    assert_eq!(ms[4].point, Path::new("/home/jason/my disk"));
    assert_eq!(ms[4].fstype, "fuse.x");
    let under: Vec<_> = at_or_under(&ms, Path::new("/home/jason")).iter().map(|m| m.id).collect();
    assert_eq!(under, vec![40, 41, 43], "/home/jason2 is not under /home/jason");
    assert!(parse(b"1 2 3\n").is_err());
    assert!(parse(b"1 0 8:1 / /a\\9 rw - ext4 x rw\n").is_err());
    assert!(!read().expect("read").is_empty());
}
