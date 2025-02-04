use crate::{
    prelude::*,
    base::index::ZoneInd,
    comm::msg::OzoneMsg,
    file::{
        core::FileAccess,
        floc::{
            FileNum,
            StoredFileLocation,
        },
        zdir::ZoneDir,
    },
    test::{
        data::{
            prepare_write_messages,
            stopwatch,
        },
    },
};

use oxedize_fe2o3_iop_db::api::{
    RestSchemesOverride,
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};
use oxedize_fe2o3_hash::csum::ChecksumScheme;

use std::{
    collections::BTreeMap,
    fs::{
        File,
        OpenOptions,
    },
    io::{
        Read,
        Write,
    },
    path::PathBuf,
    time::{
        self,
        Instant,
    },
};

pub use humantime::format_rfc3339_seconds as timefmt;
use hostname;

pub fn delete_all_index_files(zone_dirs: &BTreeMap<ZoneInd, ZoneDir>) -> Outcome<()> {
    for (_, zdir) in zone_dirs {
        let dir = &zdir.dir;
        test!(sync_log::stream(), "Removing all index files from {:?}", dir);
        for entry in res!(std::fs::read_dir(dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if path.is_file() {
                if let Some(os_str) = path.extension() {
                    if let Some("ind") = os_str.to_str() {
                        res!(std::fs::remove_file(path));
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn corrupt_an_index_file(zone_dirs: &BTreeMap<ZoneInd, ZoneDir>) -> Outcome<()> {
    test!(sync_log::stream(), "Deliberately corrupting the first index file in the first zone.");
    if let Some(zdir) = zone_dirs.get(&ZoneInd::new(0usize)) {
        for entry in res!(std::fs::read_dir(&zdir.dir)) {
            let entry = res!(entry);
            let path = entry.path();
            if path.is_file() {
                let (fnum, typ) = res!(ZoneDir::ozone_file_number_and_type(&path));
                if fnum == 1 && typ == FileType::Index {
                    let mut buf = Vec::new();
                    {
                        let mut file = res!(std::fs::OpenOptions::new()
                            .read(true)
                            .open(&path)
                        );
                        res!(file.read_to_end(&mut buf));
                        // Pick a single byte in the middle and negate it.
                        let i = buf.len()/2;
                        buf[i] = !buf[i];
                    }
                    let mut file = res!(std::fs::OpenOptions::new()
                        .write(true)
                        .open(&path)
                    );
                    res!(file.write_all(&buf));
                }
            }
        }
    } else {
        return Err(err!(
            "Could not obtain the directory for the first zone.";
        Missing, Data));
    }
    
    Ok(())
}

pub fn save_single_file<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    dir:        PathBuf,
    mut db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    user:       UID,
    schms2:     Option<&RestSchemesOverride<ENC, KH>>,
    ks:         Vec<Dat>,
    vs:         Vec<Dat>,
    byts:       usize, // total bytes in data
)
    -> Outcome<(u64, u64)>
{
    test!(sync_log::stream(), "Performing control experiment by saving data directly to a single disk file.");
    test!(sync_log::stream(), "  RestSchemes: {:?}", schms2);

    let mut path = dir.clone();
    path.push("control_file");

    let mut file = res!(OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .append(true)
        .open(path)
    );

    let (n, msgs) = res!(prepare_write_messages(
        &mut db,
        user,
        schms2,
        ks,
        vs,
        byts,
    ));

    let mut datsize: usize = 0;
    test!(sync_log::stream(), "Saving {} key-value data pairs.", n);
    let start = Instant::now();
    for (msg, _zind) in msgs {
        match msg {
            OzoneMsg::Write{kstored, vstored, ..} => {
                datsize += res!(file.write(&kstored));
                datsize += res!(file.write(&vstored));
                
                res!(file.write(&kstored));
                let sfloc = res!(StoredFileLocation::new(
                    0,
                    datsize as u64,
                    kstored.len() as u64,
                    vstored.len() as u64,
                    ChecksumScheme::new_crc32(),
                ));
                let istored = sfloc.buf;
                res!(file.write(&istored));
            },
            _ => return Err(err!(
                "Expecting OzoneMsg::Write, got {:?}", msg;
            Unreachable, Bug)),
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    Ok(stopwatch(elapsed, n, byts))
}

pub fn save_multiple_files<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    dir:        PathBuf,
    size_lim:   usize,
    mut db:     &mut O3db<UIDL, UID, ENC, KH, PR, CS>,
    user:       UID,
    schms2:     Option<&RestSchemesOverride<ENC, KH>>,
    ks:         Vec<Dat>,
    vs:         Vec<Dat>,
    byts:       usize, // total bytes in data
)
    -> Outcome<(u64, u64)>
{
    test!(sync_log::stream(), "Performing control experiment by saving data directly to multiple disk files.");
    test!(sync_log::stream(), "  RestSchemes: {:?}", schms2);

    let mut fnum: FileNum = 0;
    let mut datsize: usize = 0;
    let mut datfile: Option<File> = None;
    let mut indfile: Option<File> = None;

    let (n, msgs) = res!(prepare_write_messages(
        &mut db,
        user,
        schms2,
        ks,
        vs,
        byts,
    ));

    let mut i = 0;
    test!(sync_log::stream(), "Saving {} key-value data pairs.", n);
    let start = Instant::now();
    for (msg, _zind) in msgs {
        match msg {
            OzoneMsg::Write{kstored, vstored, ..} => {
                if i == 0 || datsize + kstored.len() + vstored.len() > size_lim {
                    fnum += 1;
                    datsize = 0;
                    let mut path = dir.clone();
                    path.push(ZoneDir::relative_file_path(&FileType::Data, fnum));
                    test!(sync_log::stream(), "Creating file {:?}", path);
                    datfile = Some(res!(ZoneDir::open_file(&path, &FileAccess::Writing)));
                    let mut path = dir.clone();
                    path.push(ZoneDir::relative_file_path(&FileType::Index, fnum));
                    test!(sync_log::stream(), "Creating file {:?}", path);
                    indfile = Some(res!(ZoneDir::open_file(&path, &FileAccess::Writing)));
                }
                if let Some(file) = &mut datfile {
                    datsize += res!(file.write(&kstored));
                    datsize += res!(file.write(&vstored));
                }
                if let Some(file) = &mut indfile {
                    res!(file.write(&kstored));
                    let sfloc = res!(StoredFileLocation::new(
                        fnum,
                        datsize as u64,
                        kstored.len() as u64,
                        vstored.len() as u64,
                        ChecksumScheme::new_crc32(),
                    ));
                    let istored = sfloc.buf;
                    res!(file.write(&istored));
                }
                //if i % 100 == 0 {
                //    debug!(sync_log::stream(), "Key {} data written {}", i, datsize);
                //}
                i += 1;
            },
            _ => return Err(err!(
                "Expecting OzoneMsg::Write, got {:?}", msg;
            Unreachable, Bug)),
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    Ok(stopwatch(elapsed, n, byts))
}

#[macro_export]
macro_rules! table_row { () => { "|{:<60}|{:^15}|{:^15}|" } }
#[macro_export]
macro_rules! table_single_line { () => { "+{:-<60}+{:-^15}+{:-^15}+" } }
#[macro_export]
macro_rules! table_double_line { () => { "+{:=<60}+{:=^15}+{:=^15}+" } }

pub fn append_table(
    path:   PathBuf,
    table:  Vec<(String, u64, u64)>,
)
    -> Outcome<()>
{
    let mut file = res!(std::fs::OpenOptions::new().create(true).append(true).open(path));

    res!(file.write_all(fmt!("\n").as_bytes()));
    for line in [
        fmt!(table_single_line!(), "", "", ""),
        fmt!(table_row!(),
            fmt!("{} {:?}", timefmt(time::SystemTime::now()), res!(hostname::get())),
            "TPS",
            "BW [Mb]/[s]",
        ),
        fmt!(table_double_line!(), "", "", ""),
    ] {
        test!(sync_log::stream(), "{}", line);
        res!(file.write_all(fmt!("{}\n", line).as_bytes()));
    }
    for (s, tps, bw) in table {
        for line in [
            fmt!(table_row!(), s, tps, bw),
            fmt!(table_single_line!(), "", "", ""),
        ] {
            test!(sync_log::stream(), "{}", line);
            res!(file.write_all(fmt!("{}\n", line).as_bytes()));
        }
    }
    Ok(())
}

