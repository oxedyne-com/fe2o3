//! The review's crash campaign as a regression test (review of 9f64d2fc, 2026-10-11).  A child,
//! this test binary run again, writes a store with collection on, deleting whole files in the
//! order of builds before 2026-10-11 (data file first), and aborts after the first unlink of its
//! nth whole-file delete (`test::hooks::set_delete_cut`).  The parent opens the image three times
//! with collection off: each open must complete, every key must read as the child's log of
//! operations says (the one in flight may land either way), and no live file may take the number
//! of an index that stood alone.  The default runs a few dozen images; O3DB_LONE_CAMPAIGN_CUTS
//! raises the highest cut tried and O3DB_LONE_CAMPAIGN_SEEDS names the random scenarios' seeds.

mod gc_pair;

use gc_pair::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
    base::cfg::OzoneConfig,
    test::{
        hooks,
        setup::{
            self,
            Uid,
        },
    },
};

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Stdio,
    },
    time::{
        Duration,
        Instant,
    },
};

const DIR:      &str = "O3DB_LONE_CAMPAIGN_DIR";
const SCEN:     &str = "O3DB_LONE_CAMPAIGN_SCEN";
const CUT:      &str = "O3DB_LONE_CAMPAIGN_CUT";
const CUTS:     &str = "O3DB_LONE_CAMPAIGN_CUTS";
const SEEDS:    &str = "O3DB_LONE_CAMPAIGN_SEEDS";
const NK:       u64 = 12;

fn cfg() -> Outcome<OzoneConfig> {
    let mut c = res!(config(1_500));
    c.num_zones             = 2;
    c.num_cbots_per_zone    = 2;
    c.rest_chunk_threshold  = 1_000;
    Ok(c)
}

fn key_of(i: u64) -> Dat { dat!(fmt!("campaign key {:03}", i)) }

// The value carries its key, version and size, so a value read back names its write.
fn val(i: u64, ver: u64, big: bool) -> Dat {
    let n = if big { 2_600 } else { 90 };
    let mut v = vec![0u8; n];
    v[0] = i as u8;
    v[1] = ver as u8;
    v[2] = big as u8;
    for j in 3..n {
        v[j] = (i as u8) ^ (ver as u8).wrapping_mul(7) ^ (j as u8);
    }
    Dat::BU32(v)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn ops_path(root: &Path) -> PathBuf { PathBuf::from(fmt!("{}.ops", root.display())) }

// Each line is synced before the operation it names goes on, so the log outlives the abort.
fn log_op(f: &mut fs::File, line: &str) -> Outcome<()> {
    res!(writeln!(f, "{}", line));
    res!(f.sync_data());
    Ok(())
}

fn read_value(db: &TestDb, k: &Dat) -> Outcome<Option<Dat>> {
    match res!(db.get(k, None)) {
        None                        => Ok(None),
        Some((Dat::Tup5u64(t), _))  => Ok(Some(res!(db.api().fetch_chunks(&Dat::Tup5u64(t), None)))),
        Some((v, _))                => Ok(Some(v)),
    }
}

struct Ops<'a> {
    db:     &'a TestDb,
    log:    fs::File,
}

impl<'a> Ops<'a> {
    fn put(&mut self, i: u64, ver: u64, big: bool) -> Outcome<()> {
        res!(log_op(&mut self.log, &fmt!("try put {} {} {}", i, ver, big as u8)));
        res!(self.db.insert(key_of(i), val(i, ver, big), Uid::default(), None));
        log_op(&mut self.log, &fmt!("put {} {} {}", i, ver, big as u8))
    }
    fn del(&mut self, i: u64) -> Outcome<()> {
        res!(log_op(&mut self.log, &fmt!("try del {}", i)));
        let _ = res!(self.db.delete(&key_of(i), Uid::default(), None));
        log_op(&mut self.log, &fmt!("del {}", i))
    }
}

/// The child: writes one scenario with collection on and the old delete order, aborting at the
/// cut.  A child that reaches the end without it closes the store and says so in the log.
fn child(root: PathBuf, scen: String, cut: u64) -> Outcome<()> {
    log_set_level!("error");
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root));
    hooks::set_delete_cut(true, cut);
    let db: TestDb = res!(setup::start_db(root.clone(), Some(res!(cfg())), schemes(), None, true, true));
    let log = res!(fs::OpenOptions::new().create(true).truncate(true).write(true).open(ops_path(&root)));
    let mut ops = Ops { db: &db, log };
    match scen.as_str() {
        "over" => {
            for i in 0..NK { res!(ops.put(i, 1, false)); }
            for i in 0..NK { res!(ops.put(i, 2, false)); }
        },
        "del" => {
            for i in 0..NK { res!(ops.put(i, 1, false)); }
            for i in 0..NK / 2 { res!(ops.del(i)); }
            for i in NK / 2..NK { res!(ops.put(i, 2, false)); }
            // Pads, so that the files of tombstones seal.
            for i in NK..NK + 12 { res!(ops.put(i, 1, false)); }
        },
        "chunk" => {
            for i in 0..4 { res!(ops.put(i, 1, true)); }
            res!(ops.put(0, 2, true));
            res!(ops.del(1));
            res!(ops.put(2, 3, false));
            res!(ops.del(3));
            res!(ops.put(3, 4, true));
            for i in 4..NK + 4 { res!(ops.put(i, 1, false)); }
            for i in 4..NK + 4 { res!(ops.put(i, 2, false)); }
        },
        seed => {
            let mut r = Rng(res!(seed.parse::<u64>()) | 1);
            for s in 0..150 {
                let i = r.next() % NK;
                match r.next() % 10 {
                    0 | 1   => res!(ops.del(i)),
                    2 | 3   => res!(ops.put(i, 10 + s, true)),
                    _       => res!(ops.put(i, 10 + s, false)),
                }
            }
        },
    }
    let mut log = ops.log;
    res!(log_op(&mut log, "writes done"));
    let _ = db.api().settle_for_test(Duration::from_secs(20));
    std::thread::sleep(Duration::from_secs(3));
    let _ = db.api().settle_for_test(Duration::from_secs(20));
    res!(log_op(&mut log, "closing"));
    res!(db.close());
    log_op(&mut log, "closed")
}

type Want = BTreeMap<u64, Option<(u64, bool)>>;

// What each key must read, and the operation in flight at the cut, if any.
fn expected(root: &Path) -> Outcome<(Want, Option<(u64, Option<(u64, bool)>)>)> {
    let text = res!(fs::read_to_string(ops_path(root)));
    let mut want = BTreeMap::new();
    let mut pend = None;
    let num = |w: Option<&&str>| -> Outcome<u64> {
        let w = res!(w.ok_or_else(|| err!("A short line in the log of operations."; Test)));
        Ok(res!(w.parse::<u64>()))
    };
    for line in text.lines() {
        let w: Vec<&str> = line.split(' ').collect();
        match w.as_slice() {
            ["try", "put", ..]  => pend = Some((res!(num(w.get(2))), Some((res!(num(w.get(3))), w.get(4) == Some(&"1"))))),
            ["try", "del", ..]  => pend = Some((res!(num(w.get(2))), None)),
            ["put", ..]         => {
                pend = None;
                want.insert(res!(num(w.get(1))), Some((res!(num(w.get(2))), w.get(3) == Some(&"1"))));
            },
            ["del", ..]         => {
                pend = None;
                want.insert(res!(num(w.get(1))), None);
            },
            _                   => (),
        }
    }
    Ok((want, pend))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            match p.is_dir() {
                true    => walk(&p, out),
                false   => out.push(p),
            }
        }
    }
}

// The indexes standing without a data file, and every index, by path.
fn indexes(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut all = Vec::new();
    walk(root, &mut all);
    let inds: Vec<PathBuf> = all.into_iter()
        .filter(|p| p.extension().map_or(false, |x| x == "ind"))
        .collect();
    let lone = inds.iter().filter(|p| !p.with_extension("dat").exists()).cloned().collect();
    (lone, inds)
}

fn check(db: &TestDb, want: &Want, pend: &Option<(u64, Option<(u64, bool)>)>) -> Vec<String> {
    let mut bad = Vec::new();
    let mut all = want.clone();
    if let Some((k, _)) = pend {
        all.entry(*k).or_insert(None);
    }
    for (i, w) in &all {
        let got = read_value(db, &key_of(*i));
        let fits = |w: &Option<(u64, bool)>| match (&got, w) {
            (Ok(Some(v)), Some((ver, big)))  => *v == val(*i, *ver, *big),
            (Ok(None), None)                 => true,
            _                                => false,
        };
        let alt = match pend {
            Some((k, a)) if k == i  => fits(a),
            _                       => false,
        };
        if !fits(w) && !alt {
            bad.push(fmt!("key {} wants {:?} and reads {}", i, w, match &got {
                Ok(Some(v)) => fmt!("version {} big {}",
                    v.bytes_ref().and_then(|b| b.get(1).copied()).unwrap_or(0),
                    v.bytes_ref().and_then(|b| b.get(2).copied()).unwrap_or(0)),
                Ok(None)    => fmt!("nothing"),
                Err(e)      => fmt!("an error: {}", e),
            }));
        }
    }
    bad
}

// Runs one image: the child to its cut, then three opens.  Whether the child was cut.
fn image(base: &Path, scen: &str, cut: u64, fails: &mut Vec<String>) -> Outcome<bool> {
    let root = base.join(fmt!("{}_{}", scen, cut));
    let tag = fmt!("{} cut {}", scen, cut);
    let log = base.join(fmt!("{}_{}.log", scen, cut));
    let out = res!(fs::File::create(&log));
    let err = res!(out.try_clone());
    // No core file: the abort is the point.
    let mut kid = res!(Command::new("bash")
        .args(["-c", "ulimit -c 0; exec \"$0\" \"$@\""])
        .arg(res!(std::env::current_exe()))
        .args(["main", "--exact", "--nocapture", "--test-threads=1"])
        .env(DIR, &root)
        .env(SCEN, scen)
        .env(CUT, fmt!("{}", cut))
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn());
    let start = Instant::now();
    let status = loop {
        if let Some(s) = res!(kid.try_wait()) {
            break s;
        }
        if start.elapsed() > Duration::from_secs(120) {
            let _ = kid.kill();
            let _ = kid.wait();
            fails.push(fmt!("{}: the child ran past two minutes; see {:?}", tag, log));
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let ops = res!(fs::read_to_string(ops_path(&root)).or_else(|_| Ok::<_, std::io::Error>(String::new())));
    let cut_hit = !ops.contains("closed");
    if !cut_hit {
        if !status.success() {
            fails.push(fmt!("{}: the child failed without a cut; see {:?}", tag, log));
        }
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(ops_path(&root));
        let _ = fs::remove_file(&log);
        return Ok(false);
    }
    let (want, pend) = res!(expected(&root));
    let (lone, _) = indexes(&root);
    let mut fine = true;
    if lone.is_empty() {
        fails.push(fmt!("{}: positive control: the cut left no index alone", tag));
        fine = false;
    }
    let mut want = want;
    for round in 0..3 {
        let db: TestDb = match setup::start_db(root.clone(), Some(res!(cfg())), schemes(), None, false, false) {
            Ok(db) => db,
            Err(e) => {
                fails.push(fmt!("{}: open {} refused: {}", tag, round, e));
                fine = false;
                break;
            },
        };
        let bad = check(&db, &want, &pend);
        if !bad.is_empty() {
            fails.push(fmt!("{}: open {} reads {} keys wrong: {:?}", tag, round, bad.len(), bad));
            fine = false;
        }
        let (still, before) = indexes(&root);
        if !still.is_empty() {
            fails.push(fmt!("{}: open {} left indexes alone: {:?}", tag, round, still));
            fine = false;
        }
        if round == 0 {
            // Enough writes to roll over to new files, none of which may take a lone number.
            for j in 900..912 {
                res!(db.insert(key_of(j), val(j, 1, false), Uid::default(), None));
                want.insert(j, Some((1, false)));
            }
            let (_, after) = indexes(&root);
            let reused: Vec<&PathBuf> = after.iter()
                .filter(|p| !before.contains(p) && lone.contains(p))
                .collect();
            if !reused.is_empty() {
                fails.push(fmt!("{}: new files took the numbers of lone indexes: {:?}", tag, reused));
                fine = false;
            }
        }
        res!(db.close());
    }
    if fine {
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_file(ops_path(&root));
        let _ = fs::remove_file(&log);
    }
    Ok(true)
}

fn parent() -> Outcome<()> {
    log_set_level!("error");
    let dir = "./test_db_lone_index_campaign";
    let _ = fs::remove_dir_all(dir);
    res!(fs::create_dir_all(dir));
    let base = res!(Path::new(dir).canonicalize());
    let cuts = std::env::var(CUTS).ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(5);
    let seeds = std::env::var(SEEDS).unwrap_or(fmt!("7,11"));
    let mut scens: Vec<String> = vec![fmt!("chunk"), fmt!("over"), fmt!("del")];
    scens.extend(seeds.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    let mut fails = Vec::new();
    let mut images = 0;
    for scen in &scens {
        for cut in 1..(cuts + 1) {
            match res!(image(&base, scen, cut, &mut fails)) {
                true    => images += 1,
                false   => break, // fewer deletes than the cut: higher cuts are the same run
            }
        }
    }
    log_finish_wait!();
    println!("{} cut images, {} failures.", images, fails.len());
    for f in &fails {
        println!("FAIL {}", f);
    }
    assert!(images >= scens.len(), "positive control: only {} images were cut", images);
    assert!(fails.is_empty(), "{} failures over {} images: {:?}", fails.len(), images, fails);
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn main() -> Outcome<()> {
    match (std::env::var(DIR), std::env::var(SCEN), std::env::var(CUT)) {
        (Ok(dir), Ok(scen), Ok(cut)) => child(PathBuf::from(dir), scen, res!(cut.parse::<u64>())),
        _ => parent(),
    }
}
