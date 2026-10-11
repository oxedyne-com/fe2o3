//! An index file standing without its data file, judged at the open (2026-10-11).
//!
//! A collection deletes a file only once every record in it is old, and removes the pair with two
//! unlinks.  Builds before 2026-10-11 removed the data file first, so a crash between the two left
//! the index alone, and a filesystem that does not order two unlinks before a directory sync can do
//! the same under the present order.  The same state also arises when a data file is really lost.
//! The difference is in the records the index names: after an interrupted delete each one is
//! superseded by a newer record elsewhere, or is a chunk no current value names, and a refusal to
//! open would be the wrong answer to a crash.  So the zones replay without such an index, and once
//! every zone is up `settle` reads each one and either completes the delete or refuses the open,
//! naming the record that would be lost.
//!
//! A user key's record cannot lose its only shadow: a key's tombstone is forgotten by no cache and
//! collected only when a newer record supersedes it.  A chunk tombstone is forgotten once the older
//! records of its key are gone, which a replay that skipped this index believes, so a chunk record
//! with nothing newer at its key is judged by whether a current value names it.

use crate::{
    prelude::*,
    api::legacy_chunk_set_id,
    base::{
        alias,
        constant,
        index::{
            BotPoolInd,
            WorkerInd,
            ZoneInd,
        },
    },
    bots::worker::worker_deps::WorkerType,
    comm::msg::OzoneMsg,
    data::{
        cache::supersedes,
        choose::ChooseCache,
        core::Key,
    },
    file::{
        floc::{
            FileLocation,
            FileNum,
        },
        stored::{
            RecordDigest,
            StoredIndex,
            StoredKey,
        },
        zdir::{
            ZoneDir,
            sync_dir,
        },
    },
};

use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};

use std::{
    collections::BTreeMap,
    fs,
    io::BufReader,
    path::PathBuf,
};


// One record an index names.
struct Named<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    kbyts:  Vec<u8>,
    meta:   Meta<UIDL, UID>,
    floc:   FileLocation,
    chash:  alias::ChooseHashUint,
}

// An index standing alone, and what it names.
struct Lone<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    zdir:   PathBuf,
    fnum:   FileNum,
    ind:    PathBuf,
    dat:    PathBuf,
    recs:   Vec<Named<UIDL, UID>>,
}

/// Completes each delete a crash interrupted, leaving an index without its data file, and refuses
/// when such an index names a record nothing else holds.  Run once every zone has replayed and
/// before the store is handed out.  Gives the number of indexes removed.
pub fn settle<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
)
    -> Outcome<usize>
{
    let mut lones = Vec::new();
    for (_, zdir) in res!(api.get_zone_dirs()) {
        res!(find(api, &zdir, &mut lones));
    }
    if lones.is_empty() {
        return Ok(0);
    }

    // 1. A record is superseded when its cache holds a newer record of the key, the order every
    //    reader decides by.  Asked of each cache bot once, behind its replay.
    let mut asks: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    for (l, lone) in lones.iter().enumerate() {
        for (r, rec) in lone.recs.iter().enumerate() {
            let w = ChooseCache::<PR>::choose_cbot_select(
                rec.chash,
                api.cfg().num_zones,
                api.cfg().num_cbots_per_zone,
            );
            asks.entry((**w.zind(), **w.bpind())).or_default().push((l, r));
        }
    }
    let mut chunks = Vec::new(); // (lone, record, chunk key) with nothing newer at its key
    for ((z, b), which) in asks {
        let keys: Vec<Vec<u8>> = which.iter().map(|(l, r)| lones[*l].recs[*r].kbyts.clone()).collect();
        let held = res!(records_of(api, z, b, keys));
        if held.len() != which.len() {
            return Err(err!(
                "{}: Cache bot {} of zone {} answered {} records for {} keys while judging an \
                index without its data file.", api.ozid(), b, z, held.len(), which.len();
                Bug, Mismatch));
        }
        for ((l, r), now) in which.into_iter().zip(held) {
            let rec = &lones[l].recs[r];
            let newer = match &now {
                Some((m, f)) => f.fnum != rec.floc.fnum
                    && supersedes(&m.time, f, &rec.meta.time, &rec.floc),
                None => false,
            };
            if newer {
                continue;
            }
            // A key of the chunk form is judged as a chunk.  The write has refused a user key of
            // that form only since cb05687c (2026-10-09), so an older store could hold one, but on
            // 2026-10-11 no caller writes such a key: a read-only grep for Tup5u64, t5u64 and
            // [u64; 5] over the Daimond gateway source, the Oxegen tree and fe2o3's callers of
            // Ozone (steel, ore, sbj, net) found none, and jarrah's r545 store copy checked on
            // 2026-10-10 held no chunked value at all (review F2).
            match Dat::from_bytes(&rec.kbyts) {
                Ok((Dat::Tup5u64(tup), n)) if n == rec.kbyts.len() => chunks.push((l, r, tup)),
                _ => return Err(refusal(api, &lones[l], rec, "no other file holds a newer record \
                    of its key")),
            }
        }
    }

    // 2. A chunk with nothing newer at its key is lost only if a current value names it.  One
    //    write stamps its head and every chunk with one metadata, so the head of the chunk's own
    //    write carries its stamp.  The exception is key reuse: the builds of 2026-09 took the set
    //    identifier from the key alone, so a torn rewrite of one key at one length leaves the
    //    first write's head naming the second write's chunks, and that head is found by its key
    //    (2026-10-11).  So the caches are asked for those heads alone and only they are read,
    //    never every value in the store (review F3, 2026-10-11).  A head that cannot be read is a
    //    head that may name the chunk, so it refuses; any other unreadable value is never looked
    //    at.
    if !chunks.is_empty() {
        let mut metas: Vec<Meta<UIDL, UID>> = Vec::new();
        let mut set_ids: Vec<u64> = Vec::new();
        for (l, r, tup) in &chunks {
            let m = &lones[*l].recs[*r].meta;
            if !metas.contains(m) {
                metas.push(m.clone());
            }
            if !set_ids.contains(&tup[0]) {
                set_ids.push(tup[0]);
            }
        }
        for z in 0..api.cfg().num_zones as usize {
            for b in 0..api.cfg().num_cbots_per_zone as usize {
                for (kbyts, hmeta) in res!(heads_of(api, z, b, metas.clone(), set_ids.clone())) {
                    let named = part_key_of(api, z, b, &kbyts);
                    let legacy = legacy_chunk_set_id(&kbyts);
                    for (l, r, tup) in &chunks {
                        let rec = &lones[*l].recs[*r];
                        if rec.meta != hmeta && legacy != tup[0] {
                            continue;
                        }
                        match &named {
                            Ok(Some(pk)) if names(pk, tup) => return Err(refusal(api,
                                &lones[*l], rec, "it is a chunk of a current value")),
                            Ok(Some(_)) => (), // another write's set, under the same stamp
                            Ok(None) => return Err(refusal(api, &lones[*l], rec,
                                "it is a chunk written with the head of a current value, and \
                                that head does not read as a part key, so whether it names this \
                                chunk cannot be told")),
                            Err(e) => return Err(err!(e.clone(), "{}", refusal(api, &lones[*l],
                                rec, "it is a chunk written with the head of a current value, and \
                                that head cannot be read to tell whether it names this chunk");
                                Init, File, Missing)),
                        }
                    }
                }
            }
        }
    }

    // 3. Every record is superseded, so the file was deleted whole: finish the delete.
    let mut synced = Vec::new();
    for lone in &lones {
        if let Err(e) = fs::remove_file(&lone.ind) {
            return Err(err!(e,
                "{}: Completing the interrupted delete of index {:?}, whose data file {:?} is \
                gone and every record of which is superseded, failed at the unlink, so the \
                store is not opened.  No record was changed: fix the cause and reopen.", api.ozid(), lone.ind, lone.dat;
                Init, IO, File, Write));
        }
        info!(sync_log::stream(), "{}: Removed index {:?}, left by an interrupted collection: its \
            data file {:?} was deleted and each of its {} records is superseded.",
            api.ozid(), lone.ind, lone.dat, lone.recs.len());
        if !synced.contains(&lone.zdir) {
            synced.push(lone.zdir.clone());
        }
    }
    for dir in &synced {
        if let Err(e) = sync_dir(dir) {
            let done = lones.iter()
                .filter(|l| &l.zdir == dir)
                .map(|l| fmt!("index {:?} (data file {:?})", l.ind, l.dat))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(err!(e,
                "{}: Completing the interrupted delete of {}, whose data files are gone and \
                every record superseded, failed at the sync of directory {:?} after the unlink, so \
                the store is not opened.  No record was changed: fix the cause and reopen.", api.ozid(), done, dir;
                Init, IO, File, Write));
        }
    }
    Ok(lones.len())
}

// Each index of the zone with records in it and no data file.
fn find<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    zdir:   &ZoneDir,
    out:    &mut Vec<Lone<UIDL, UID>>,
)
    -> Outcome<()>
{
    for entry in res!(fs::read_dir(&zdir.dir)) {
        let path = res!(entry).path();
        if !path.is_file() || ZoneDir::is_gc_temp_file(&path) {
            continue;
        }
        let (fnum, typ) = match ZoneDir::ozone_file_number_and_type(&path) {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        if typ != FileType::Index || res!(fs::metadata(&path)).len() == 0 {
            continue;
        }
        let dat = zdir.dir.join(ZoneDir::relative_file_path(&FileType::Data, fnum));
        if dat.exists() {
            continue;
        }
        let recs = res!(read(api, &path, fnum));
        out.push(Lone { zdir: zdir.dir.clone(), fnum, ind: path, dat, recs });
    }
    Ok(())
}

// The records an index names, read whole.  One that cannot be read cannot be judged.
fn read<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    ind:    &PathBuf,
    fnum:   FileNum,
)
    -> Outcome<Vec<Named<UIDL, UID>>>
{
    let csum = api.schemes().checksummer().clone();
    let mut rdr = BufReader::new(res!(fs::File::open(ind)));
    let mut recs = Vec::new();
    let mut pos = 0;
    loop {
        let (key, meta, chash) = match StoredKey::load(&mut rdr, csum.clone()) {
            Ok(None) => break,
            Ok(Some((skey, _, n))) => {
                pos += n;
                let meta = skey.meta().clone();
                let chash = skey.ref_chash().clone();
                (skey.into_key(), meta, chash)
            },
            Err(e) => return Err(err!(e,
                "{}: The index {:?} stands without its data file and cannot be read at byte \
                {}, so whether it names a record nothing else holds cannot be told.  Restore \
                its data file from a backup or a replica and reopen; only if losing the records \
                of that file is accepted, remove {:?} and reopen.  The open is refused.",
                api.ozid(), ind, pos, ind;
                Init, File, Missing)),
        };
        let floc = match StoredIndex::read(&mut rdr, fnum, csum.clone()) {
            Ok((Some(sindex), n)) => {
                pos += n;
                sindex.ref_file_location().clone()
            },
            Ok((None, _)) => return Err(err!(
                "{}: The index {:?} stands without its data file and ends inside an entry at byte \
                {}.  Restore its data file from a backup or a replica and reopen; only if losing \
                the records of that file is accepted, remove {:?} and reopen.  The open is \
                refused.", api.ozid(), ind, pos, ind;
                Init, File, Missing)),
            Err(e) => return Err(err!(e,
                "{}: The index {:?} stands without its data file and cannot be read at byte \
                {}.  Restore its data file from a backup or a replica and reopen; only if losing \
                the records of that file is accepted, remove {:?} and reopen.  The open is \
                refused.", api.ozid(), ind, pos, ind;
                Init, File, Missing)),
        };
        let chash = res!(<alias::ChooseHash>::try_from(&chash[..constant::CACHE_HASH_BYTES]));
        recs.push(Named {
            kbyts:  key.into_bytes(),
            meta,
            floc,
            chash:  alias::ChooseHashUint::from_be_bytes(chash),
        });
    }
    Ok(recs)
}

// The record cache bot `b` of zone `z` holds for each stored key.
fn records_of<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    z:      usize,
    b:      usize,
    keys:   Vec<Vec<u8>>,
)
    -> Outcome<Vec<Option<(Meta<UIDL, UID>, FileLocation)>>>
{
    let resp = api.responder();
    let cbots = res!(api.chans().get_workers_of_type_in_zone(&WorkerType::Cache, &ZoneInd::new(z)));
    let bot = res!(cbots.get_bot(b));
    if let Err(e) = bot.send(OzoneMsg::RecordsOf(keys, resp.clone())) {
        return Err(err!(e,
            "{}: Asking cache bot {} of zone {} for the records of an index without its data \
            file.", api.ozid(), b, z;
            Channel, Write));
    }
    match res!(resp.recv_timeout(constant::CONTROL_REQUEST_TIMEOUT)) {
        OzoneMsg::RecordsAt(held) => Ok(held),
        OzoneMsg::Error(e) => Err(err!(e,
            "{}: Cache bot {} of zone {} could not give the records of an index without its \
            data file.", api.ozid(), b, z;
            Channel)),
        msg => Err(err!(
            "{}: Unexpected answer from cache bot {} of zone {} for the records of an index \
            without its data file: {:?}", api.ozid(), b, z, msg;
            Channel, Unexpected)),
    }
}

// The current heads cache bot `b` of zone `z` holds written with any of `metas`.
fn heads_of<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    z:      usize,
    b:      usize,
    metas:  Vec<Meta<UIDL, UID>>,
    set_ids: Vec<u64>,
)
    -> Outcome<Vec<(Vec<u8>, Meta<UIDL, UID>)>>
{
    let resp = api.responder();
    let cbots = res!(api.chans().get_workers_of_type_in_zone(&WorkerType::Cache, &ZoneInd::new(z)));
    let bot = res!(cbots.get_bot(b));
    if let Err(e) = bot.send(OzoneMsg::HeadsStamped(metas, set_ids, resp.clone())) {
        return Err(err!(e,
            "{}: Asking cache bot {} of zone {} for the heads written with the chunks of an index \
            without its data file.", api.ozid(), b, z;
            Channel, Write));
    }
    match res!(resp.recv_timeout(constant::CONTROL_REQUEST_TIMEOUT)) {
        OzoneMsg::HeadsAt(heads) => Ok(heads),
        OzoneMsg::Error(e) => Err(err!(e,
            "{}: Cache bot {} of zone {} could not give the heads written with the chunks of an \
            index without its data file.", api.ozid(), b, z;
            Channel)),
        msg => Err(err!(
            "{}: Unexpected answer from cache bot {} of zone {} for the heads written with the \
            chunks of an index without its data file: {:?}", api.ozid(), b, z, msg;
            Channel, Unexpected)),
    }
}

// The part key the head at stored key `kbyts`, held by cache bot `b` of zone `z`, names, read with
// the store's own schemes.
fn part_key_of<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    z:      usize,
    b:      usize,
    kbyts:  &[u8],
)
    -> Outcome<Option<[u64; 5]>>
{
    let cbwind = WorkerInd::new(ZoneInd::new(z), BotPoolInd::new(b));
    let resp = res!(api.fetch_using_key(Key::Complete(kbyts.to_vec()), cbwind));
    match res!(resp.recv_daticle(api.schemes().encrypter(), None)) {
        (Some((Dat::Tup5u64(tup), _)), _)   => Ok(Some(tup)),
        _                                   => Ok(None),
    }
}

// Does the part key `pk` of a head name the chunk key `tup`?  The chunk keys of a set are the part
// key with its index running from 1 to the number of parts.
fn names(pk: &[u64; 5], tup: &[u64; 5]) -> bool {
    pk[0] == tup[0]
        && pk[2] == tup[2]
        && pk[3] == tup[3]
        && pk[4] == tup[4]
        && tup[1] >= 1
        && tup[1] <= pk[3]
}

// The refusal for a record nothing else holds: the index, the record and the data file it is in,
// and the way out.
fn refusal<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    PR:     Hasher + 'static,
    CS:     Checksummer + 'static,
>(
    api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    lone:   &Lone<UIDL, UID>,
    rec:    &Named<UIDL, UID>,
    why:    &str,
)
    -> Error<ErrTag>
{
    let key = match Dat::from_bytes(&rec.kbyts) {
        Ok((dat, n)) if n == rec.kbyts.len() => {
            let s = fmt!("{:?}", dat);
            match s.char_indices().nth(120) {
                Some((i, _)) => fmt!("{}...", &s[..i]),
                None => s,
            }
        },
        _ => fmt!("0x{}", rec.kbyts.iter().take(32).map(|b| fmt!("{:02x}", b)).collect::<String>()),
    };
    let rid = match RecordDigest::new(&rec.kbyts, &rec.meta) {
        Ok(rid) => fmt!("{:?}", rid),
        Err(_) => fmt!("?"),
    };
    err!(
        "{}: The data file {:?} is missing while its index {:?} stands, naming {} records.  The \
        record of key {} (record {}, at byte {} of the missing data file) would be lost: {}.  \
        Restore the data file from a backup or a replica and reopen.  Only if losing the records \
        of that file is accepted, remove the index file {:?} and reopen.  The open is refused.",
        api.ozid(), lone.dat, lone.ind, lone.recs.len(), key, rid, rec.floc.start, why, lone.ind;
        Init, File, Missing)
}
