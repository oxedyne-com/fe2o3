//! An index standing without its data file is judged at the open (2026-10-11).  A collection
//! deletes a whole file only when every record in it is old, by two unlinks.  Builds before this
//! day removed the data file first, so a crash between the two left the index alone, and the open
//! refused the store as if the data had been lost.  A store opened by this build completes such a
//! delete, rebuilds an index removed first, and refuses only when the index names a record nothing
//! else holds, saying which and what to do.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Override,
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
};
use oxedyne_fe2o3_iop_db::api::{
    Database,
    RestSchemesOverride,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    O3db,
    api::OzoneApi,
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
    file::core::FileType,
    test::{
        hooks,
        setup::{
            self,
            Uid,
            UID_LEN,
        },
    },
};

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Mutex,
        MutexGuard,
    },
};

// The test hooks are process-wide, and a head or retire dropped for one test would drop another's
// running beside it, so every test here runs alone.
static HOOKS: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    match HOOKS.lock() { Ok(g) => g, Err(p) => p.into_inner() }
}

fn fresh(dir: &str) -> Outcome<PathBuf> {
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    Ok(res!(Path::new(dir).canonicalize()))
}

// Collection is off throughout, so the files stay as the test leaves them.
fn open(root: &Path, cfg: &OzoneConfig, wipe: bool) -> Outcome<TestDb> {
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(cfg.clone()), schemes(), None, false, wipe)
}

// Five records to a file, written at version 1 and then all overwritten at version 2, so files 1
// and 2 hold only old records: the files a collection deletes whole.
fn superseded(name: &str) -> Outcome<(PathBuf, OzoneConfig)> {
    let root = res!(fresh(&fmt!("./test_db_lone_index_{}", name)));
    let probe = res!(probe(&fmt!("./test_db_lone_index_{}_probe", name)));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let db = res!(open(&root, &cfg, true));
    res!(fill(&db));
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
    }
    assert_eq!(judge(&db, &[2u8; NKEYS], "before the cut"), 0);
    res!(db.close());
    for n in 1..3 {
        assert!(file(&root, &cfg, FileType::Data, n).is_file()
            && size(&file(&root, &cfg, FileType::Index, n)) > 0,
            "positive control: file {} stands whole with its index", n);
    }
    Ok((root, cfg))
}

/// The order of builds before 2026-10-11: the data file of an all-old pair was removed and the
/// crash came before its index went.  The open completes the delete.
#[test]
fn the_old_order_completes_the_delete() -> Outcome<()> {
    let _lock = serial();
    let (root, cfg) = res!(superseded("old_order"));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    res!(fs::remove_file(&d1));
    let db = match open(&root, &cfg, false) {
        Ok(db) => db,
        Err(e) => return Err(err!(e, "An interrupted delete of an all-old file was refused."; Test)),
    };
    assert!(!i1.exists(), "the index of the deleted data file {:?} still stands", i1);
    assert!(!d1.exists());
    assert_eq!(judge(&db, &[2u8; NKEYS], "after the completed delete"), 0);
    // The store goes on as usual, and opens again.
    res!(db.insert(key(0), value(0, 3), Uid::default(), None));
    res!(db.close());
    let db = res!(open(&root, &cfg, false));
    let mut want = [2u8; NKEYS];
    want[0] = 3;
    assert_eq!(judge(&db, &want, "after a second open"), 0);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The present order: the index went first and the crash came before the data file.  The index
/// is rebuilt from the data and the old records stay old.
#[test]
fn the_current_order_rebuilds_the_index() -> Outcome<()> {
    let _lock = serial();
    let (root, cfg) = res!(superseded("current_order"));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    res!(fs::remove_file(&i1));
    let db = res!(open(&root, &cfg, false));
    assert!(d1.is_file(), "the data file {:?} is gone", d1);
    assert!(size(&i1) > 0, "the index {:?} was not rebuilt", i1);
    assert_eq!(judge(&db, &[2u8; NKEYS], "after the rebuild"), 0);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A data file really lost: its index names records nothing else holds.  The open refuses and
/// says which index, which record and which data file, and what to do.  Taking the way out it
/// names opens the store without those records.
#[test]
fn a_lone_index_naming_a_current_record_refuses() -> Outcome<()> {
    let _lock = serial();
    let root = res!(fresh("./test_db_lone_index_loss"));
    let probe = res!(probe("./test_db_lone_index_loss_probe"));
    let cfg = res!(config(5 * probe.len + probe.len / 2));
    let db = res!(open(&root, &cfg, true));
    res!(fill(&db));
    res!(db.close());
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    res!(fs::remove_file(&d1));
    let e = match open(&root, &cfg, false) {
        Ok(db) => {
            let bad = judge(&db, &[1u8; NKEYS], "opened without a data file");
            let _ = db.close();
            return Err(err!("The store opened with {:?} lost; {} keys read wrong.", d1, bad; Test));
        },
        Err(e) => fmt!("{}", e),
    };
    println!("Refused: {}", e);
    assert!(e.contains(&fmt!("{:?}", i1)), "the refusal does not name the index {:?}: {}", i1, e);
    assert!(e.contains(&fmt!("{:?}", d1)), "the refusal does not name the data file {:?}: {}", d1, e);
    assert!(e.contains("is missing"), "{}", e);
    assert!(e.contains("gc pair key 00"), "the refusal does not name the key: {}", e);
    assert!(e.contains("(record "), "the refusal does not name the record: {}", e);
    assert!(e.contains("backup or a replica"), "the refusal does not say to restore: {}", e);
    assert!(e.contains("remove the index file"), "the refusal does not give the last resort: {}", e);
    assert!(size(&i1) > 0, "a refused open removed the index {:?}", i1);

    // The way out: losing those records is accepted, so the index goes.
    res!(fs::remove_file(&i1));
    let db = res!(open(&root, &cfg, false));
    for i in 0..NKEYS {
        let got = res!(db.get(&key(i), None));
        match i < 5 {
            true    => assert!(got.is_none(), "key {} of the lost file reads back", i),
            false   => assert!(matches!(got, Some((ref v, _)) if *v == value(i, 1)), "key {} is wrong", i),
        }
    }
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The chunks of a value since overwritten, in a file deleted whole whose index outlived it.
/// Nothing names them, so the delete is completed.
#[test]
fn chunks_of_a_retired_set_complete_the_delete() -> Outcome<()> {
    let _lock = serial();
    let root = res!(fresh("./test_db_lone_index_chunks"));
    let mut cfg = res!(config(2_000));
    cfg.rest_chunk_threshold    = 1_000;
    cfg.rest_chunk_bytes        = 400;
    let db = res!(open(&root, &cfg, true));
    let big = dat!("lone index big");
    res!(db.insert(big.clone(), Dat::BU32(vec![0xa7; 3_000]), Uid::default(), None));
    res!(db.insert(big.clone(), Dat::BU32(vec![0x5c; 200]), Uid::default(), None));
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    res!(db.close());
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    let chunky = files_holding(&root, &cfg, &[0xa7u8; 64]);
    assert!(chunky.contains(&d1), "positive control: file 1 holds chunks of the first value: {:?}", chunky);
    assert!(!files_holding(&root, &cfg, &[0x5cu8; 64]).contains(&d1)
        && !files_holding(&root, &cfg, b"gc pair key").contains(&d1),
        "positive control: file 1 holds nothing current");
    res!(fs::remove_file(&d1));
    let db = match open(&root, &cfg, false) {
        Ok(db) => db,
        Err(e) => return Err(err!(e, "An interrupted delete of retired chunks was refused."; Test)),
    };
    assert!(!i1.exists(), "the index {:?} of the retired chunks still stands", i1);
    let got = res!(db.get(&big, None));
    assert!(matches!(got, Some((Dat::BU32(ref v), _)) if *v == vec![0x5c; 200]), "the value of the big key is wrong");
    assert_eq!(judge(&db, &[1u8; NKEYS], "after the completed delete"), 0);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The number of an index standing alone at the survey is not handed out again as a live file,
/// even once the judgement has removed it: until then the old index would have named the new
/// file's records.  Here the index is put above every data file, so a counter seeded from the data
/// files alone gives its number to the first live file the store rolls over to.
#[test]
fn a_lone_index_number_is_not_reused() -> Outcome<()> {
    let _lock = serial();
    let (root, cfg) = res!(superseded("reuse"));
    let mut top = 0;
    for n in 1..40 {
        if file(&root, &cfg, FileType::Data, n).is_file() {
            top = n;
        }
    }
    let lone = top + 1;
    res!(fs::remove_file(file(&root, &cfg, FileType::Data, 1)));
    res!(fs::rename(file(&root, &cfg, FileType::Index, 1), file(&root, &cfg, FileType::Index, lone)));
    let db = res!(open(&root, &cfg, false));
    assert!(!file(&root, &cfg, FileType::Index, lone).exists(), "the lone index {} still stands", lone);
    for i in 0..NKEYS {
        res!(db.insert(key(i), value(i, 3), Uid::default(), None));
    }
    assert_eq!(judge(&db, &[3u8; NKEYS], "after the rollovers"), 0);
    res!(db.close());
    assert!(file(&root, &cfg, FileType::Data, lone + 1).is_file(), "positive control: the store rolled over past {}", lone);
    assert!(!file(&root, &cfg, FileType::Data, lone).exists(),
        "a live file took the number {} of the index that stood alone", lone);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// Completing the delete can itself fail, here at the unlink of the index in a directory the
/// process may not write.  The refusal names the index, the data file and the phase, and says
/// that nothing was changed; once the cause is fixed the open completes.
#[test]
fn a_failed_unlink_names_the_phase() -> Outcome<()> {
    let _lock = serial();
    use std::os::unix::fs::PermissionsExt;
    let (root, cfg) = res!(superseded("unlink"));
    let (d1, i1) = (file(&root, &cfg, FileType::Data, 1), file(&root, &cfg, FileType::Index, 1));
    res!(fs::remove_file(&d1));
    let zdir = res!(i1.parent().map(Path::to_path_buf).ok_or_else(|| err!("No zone directory."; Test)));
    res!(fs::set_permissions(&zdir, fs::Permissions::from_mode(0o555)));
    // Root writes regardless, so the test has nothing to show there.
    let canary = zdir.join("lone_index_canary.tmp");
    if fs::write(&canary, b"x").is_ok() {
        let _ = fs::remove_file(&canary);
        res!(fs::set_permissions(&zdir, fs::Permissions::from_mode(0o755)));
        let _ = fs::remove_dir_all(&root);
        msg!("Skipped: this process writes a read-only directory.");
        return Ok(());
    }
    let opened = open(&root, &cfg, false);
    res!(fs::set_permissions(&zdir, fs::Permissions::from_mode(0o755)));
    let e = match opened {
        Ok(db) => {
            let _ = db.close();
            return Err(err!("The store opened though the index {:?} could not be unlinked.", i1; Test));
        },
        Err(e) => fmt!("{}", e),
    };
    println!("Refused: {}", e);
    assert!(e.contains(&fmt!("{:?}", i1)), "the refusal does not name the index {:?}: {}", i1, e);
    assert!(e.contains(&fmt!("{:?}", d1)), "the refusal does not name the data file {:?}: {}", d1, e);
    assert!(e.contains("unlink"), "the refusal does not name the phase: {}", e);
    assert!(e.contains("fix the cause and reopen"), "the refusal does not give the way out: {}", e);
    assert!(size(&i1) > 0 && !d1.exists(), "the refused open changed the files");
    let db = match open(&root, &cfg, false) {
        Ok(db) => db,
        Err(e) => return Err(err!(e, "The open did not complete once the directory was writable."; Test)),
    };
    assert!(!i1.exists(), "the index {:?} still stands", i1);
    assert_eq!(judge(&db, &[2u8; NKEYS], "after the completed delete"), 0);
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

type EncDb = O3db<{ UID_LEN }, Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>;

// A store encrypted by default with key A, the override that writes with key B, and its config:
// files of 2000 bytes, values over 1000 bytes in chunks of 400.
fn encrypted() -> Outcome<(OzoneConfig, RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    RestSchemesOverride<EncryptionScheme, HashScheme>)>
{
    let mut cfg = res!(config(2_000));
    cfg.rest_chunk_threshold    = 1_000;
    cfg.rest_chunk_bytes        = 400;
    let a = res!(EncryptionScheme::new_aes_256_gcm_with_key(&[0x0a; 32]));
    let b = res!(EncryptionScheme::new_aes_256_gcm_with_key(&[0x0b; 32]));
    let input = RestSchemesInput::new(Some(a), None::<HashScheme>, None::<HashScheme>,
        Some(ChecksumScheme::new_crc32()));
    let ovr = RestSchemesOverride::default().set_encrypter(Override::Default(b));
    Ok((cfg, input, ovr))
}

fn open_enc(
    root:   &Path,
    cfg:    &OzoneConfig,
    input:  &RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
    wipe:   bool,
)
    -> Outcome<EncDb>
{
    log_set_level!("error");
    setup::start_db(root.to_path_buf(), Some(cfg.clone()), input.clone(), None, false, wipe)
}

fn part_key(db: &EncDb, k: &Dat, ovr: Option<&RestSchemesOverride<EncryptionScheme, HashScheme>>)
    -> Outcome<[u64; 5]>
{
    match res!(db.api().get_head_wait(k, ovr)) {
        Some((Dat::Tup5u64(t), _)) => Ok(t),
        got => Err(err!("Key {:?} reads {:?}, not a part key.", k, got; Test)),
    }
}

fn chunk_key_bytes(pk: &[u64; 5], n: u64) -> Outcome<Vec<u8>> {
    Dat::Tup5u64([pk[0], n, pk[2], pk[3], pk[4]]).as_bytes()
}

fn data_files_holding(root: &Path, cfg: &OzoneConfig, needle: &[u8]) -> Vec<PathBuf> {
    files_holding(root, cfg, needle).into_iter()
        .filter(|p| p.extension().map_or(false, |x| x == "dat"))
        .collect()
}

fn secret_value() -> Dat { Dat::BU32((0..3_000u32).map(|i| (i % 241) as u8).collect()) }

const PADS: usize = 20;

/// A lone index holding a retired chunk, in a store where another key's chunked value is written
/// with a key the store does not hold by default, so its head cannot be read at the open.  Only
/// the heads stamped with the lone chunk's metadata are read, so that value is never looked at and
/// the delete is completed (review F3: the open used to read every value and refuse).
#[test]
fn an_unreadable_value_elsewhere_does_not_block_the_delete() -> Outcome<()> {
    let _lock = serial();
    let root = res!(fresh("./test_db_lone_index_unreadable"));
    let (cfg, input, ovr) = res!(encrypted());
    let db = res!(open_enc(&root, &cfg, &input, true));
    let (big, secret) = (dat!("lone index big"), dat!("lone index secret"));
    res!(db.insert(big.clone(), Dat::BU32(vec![0xa7; 3_000]), Uid::default(), None));
    let pk1 = res!(part_key(&db, &big, None));
    for i in 0..PADS {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    // The retire of the displaced chunks is dropped, as a crash after the head would drop it, so
    // nothing newer stands at their keys and only the heads can tell they are not named.
    // The cache bot sends the retire after the insert returns, so the drop stays on to the close;
    // the other tests of this binary retire nothing they rely on.
    hooks::set_retire_drop(true);
    res!(db.insert(big.clone(), Dat::BU32(vec![0x5c; 200]), Uid::default(), None));
    res!(db.insert(secret.clone(), secret_value(), Uid::default(), Some(&ovr)));
    let sk = res!(part_key(&db, &secret, Some(&ovr)));
    for i in 0..PADS {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
    }
    let closed = db.close();
    hooks::set_retire_drop(false);
    res!(closed);

    let target = data_files_holding(&root, &cfg, &res!(chunk_key_bytes(&pk1, pk1[3])));
    assert_eq!(target.len(), 1, "positive control: one data file holds the last chunk of the first value, and no \
        tombstone retired it: {:?}", target);
    let d = target[0].clone();
    let i = d.with_extension("ind");
    let mut current = data_files_holding(&root, &cfg, &res!(secret.as_bytes()));
    for n in 1..(sk[3] + 1) {
        current.extend(data_files_holding(&root, &cfg, &res!(chunk_key_bytes(&sk, n))));
    }
    // The current head of the big key is the last of its records written.
    current.extend(data_files_holding(&root, &cfg, &res!(big.as_bytes())).into_iter().max());
    assert!(!current.contains(&d), "positive control: {:?} holds a current record", d);
    assert!(size(&i) > 0, "positive control: the index {:?} stands", i);
    res!(fs::remove_file(&d));

    let db = match open_enc(&root, &cfg, &input, false) {
        Ok(db) => db,
        Err(e) => return Err(err!(e, "An interrupted delete was refused over a value it does not touch."; Test)),
    };
    assert!(!i.exists(), "the index {:?} still stands", i);
    let got = res!(db.get(&big, None));
    assert!(matches!(got, Some((Dat::BU32(ref v), _)) if *v == vec![0x5c; 200]), "the value of the big key is wrong");
    let got = res!(db.api().fetch_chunks(&Dat::Tup5u64(res!(part_key(&db, &secret, Some(&ovr)))), Some(&ovr)));
    assert!(got == secret_value(), "the secret value reads wrong");
    for i in 0..PADS {
        let got = res!(db.get(&key(i), None));
        assert!(matches!(got, Some((ref v, _)) if *v == value(i, 2)), "pad {} reads wrong", i);
    }
    res!(db.close());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// The guard on that narrowing: a lone index holding a chunk of a current value whose head cannot
/// be read.  The head carries the chunk's own stamp, so it is read, fails, and the open refuses.
#[test]
fn an_unreadable_head_of_the_chunks_own_write_refuses() -> Outcome<()> {
    let _lock = serial();
    let root = res!(fresh("./test_db_lone_index_guard"));
    let (cfg, input, ovr) = res!(encrypted());
    let db = res!(open_enc(&root, &cfg, &input, true));
    let secret = dat!("lone index secret");
    res!(db.insert(secret.clone(), secret_value(), Uid::default(), Some(&ovr)));
    let sk = res!(part_key(&db, &secret, Some(&ovr)));
    for ver in 1..3 {
        for i in 0..PADS {
            res!(db.insert(key(i), value(i, ver), Uid::default(), None));
        }
    }
    res!(db.close());

    let heads = data_files_holding(&root, &cfg, &res!(secret.as_bytes()));
    let mut target = None;
    for n in (1..(sk[3] + 1)).rev() {
        if let Some(d) = data_files_holding(&root, &cfg, &res!(chunk_key_bytes(&sk, n)))
            .into_iter().find(|d| !heads.contains(d))
        {
            target = Some(d);
            break;
        }
    }
    let d = res!(target.ok_or_else(|| err!("Positive control: no data file holds a chunk of the \
        secret apart from its head."; Test)));
    let i = d.with_extension("ind");
    res!(fs::remove_file(&d));

    let e = match open_enc(&root, &cfg, &input, false) {
        Ok(db) => {
            let _ = db.close();
            return Err(err!("The store opened with a chunk of a current value lost in {:?}.", d; Test));
        },
        Err(e) => fmt!("{}", e),
    };
    println!("Refused: {}", e);
    assert!(e.contains("cannot be read"), "the refusal does not say the head cannot be read: {}", e);
    assert!(e.contains(&fmt!("{:?}", i)), "the refusal does not name the index {:?}: {}", i, e);
    assert!(size(&i) > 0, "a refused open removed the index {:?}", i);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

// A chunked value written as the builds of 2026-09 wrote it, its set taken from the key alone, and
// rewritten at the same length, so that both writes use one set of chunk keys.  With `torn` the
// rewrite stops after its chunks, so the head of the first write names the second write's chunks.
// The data file holding the rewrite's first chunk is then removed, as the old delete order leaves
// it.  The open must refuse either way: the chunk is named by the current head.
fn legacy_rewrite(name: &str, torn: bool) -> Outcome<()> {
    let root = res!(fresh(&fmt!("./test_db_lone_index_{}", name)));
    let mut cfg = res!(config(2_000));
    cfg.rest_chunk_threshold    = 1_000;
    cfg.rest_chunk_bytes        = 400;
    let db = res!(open(&root, &cfg, true));
    let k = dat!("lone index legacy");
    let (kbuf, _, _) = res!(db.api().ozone_key_dat(&k, None));
    let legacy = OzoneApi::<{ UID_LEN }, Uid, (), HashScheme, HashScheme, ChecksumScheme>::legacy_chunk_set_id(&kbuf);
    let resp = db.api().responder();
    res!(db.api().store_dat_using_responder_forcing_set_id(
        k.clone(), Dat::BU32(vec![0x11; 3_000]), Uid::default(), None, resp.clone(), legacy));
    res!(resp.recv_store_ack());
    let pk = match res!(db.api().get_head_wait(&k, None)) {
        Some((Dat::Tup5u64(t), _)) => t,
        got => return Err(err!("The legacy write reads {:?}, not a part key.", got; Test)),
    };
    assert_eq!(pk[0], legacy, "positive control: the first write is not under the legacy set");
    for i in 0..PADS {
        res!(db.insert(key(i), value(i, 1), Uid::default(), None));
    }
    hooks::set_head_drop(torn);
    let resp = db.api().responder();
    let second = db.api().store_dat_using_responder_forcing_set_id(
        k.clone(), Dat::BU32(vec![0x22; 3_000]), Uid::default(), None, resp.clone(), legacy)
        .and_then(|_| resp.recv_store_ack().map(|_| ()));
    hooks::set_head_drop(false);
    assert_eq!(second.is_err(), torn, "positive control: the rewrite failed {} where torn is {}", second.is_err(), torn);
    for i in 0..PADS {
        res!(db.insert(key(i), value(i, 2), Uid::default(), None));
    }
    res!(db.close());

    let holding = data_files_holding(&root, &cfg, &res!(chunk_key_bytes(&pk, 1)));
    assert_eq!(holding.len(), 2, "positive control: each write put its first chunk in a file of its own: {:?}", holding);
    let d = res!(holding.iter().max().cloned().ok_or_else(|| err!("No file."; Test)));
    let i = d.with_extension("ind");
    assert!(!data_files_holding(&root, &cfg, &res!(k.as_bytes())).contains(&d) || !torn,
        "positive control: {:?} holds a head of the key", d);
    res!(fs::remove_file(&d));

    let e = match open(&root, &cfg, false) {
        Ok(db) => {
            let got = db.get(&k, None);
            let _ = db.close();
            return Err(err!("The store opened with a chunk of the current value lost in {:?} (torn {}); \
                the key reads {:?}.", d, torn, got.map(|o| o.is_some()); Test));
        },
        Err(e) => fmt!("{}", e),
    };
    println!("Refused: {}", e);
    assert!(e.contains("a chunk of a current value"), "the refusal does not say the chunk is current: {}", e);
    assert!(e.contains(&fmt!("{:?}", i)), "the refusal does not name the index {:?}: {}", i, e);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A torn rewrite under a key-derived set leaves the current head stamped by the first write and
/// its chunks by the second.  The head is found by its key, and the open refuses (2026-10-11).
#[test]
fn a_torn_legacy_rewrite_refuses() -> Outcome<()> {
    let _lock = serial();
    legacy_rewrite("legacy_torn", true)
}

/// The control: the rewrite completes, so its head carries its chunks' stamp.
#[test]
fn a_whole_legacy_rewrite_refuses() -> Outcome<()> {
    let _lock = serial();
    legacy_rewrite("legacy_whole", false)
}
