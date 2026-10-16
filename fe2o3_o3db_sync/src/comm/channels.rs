use crate::{
    prelude::*,
    base::{
        id::{
            OzoneBotId,
            //OzoneBotType,
        },
        index::{
            BotPoolInd,
            WorkerInd,
            ZoneInd,
        },
    },
    bots::{
        worker::bot::WorkerType,
    },
    comm::msg::OzoneMsg,
    file::{
        floc::{
            FileLocation,
            FileNum,
        },
        refusal::Refusal,
        zdir::ZoneDir,
    },
    test::hooks,
};

use oxedyne_fe2o3_core::{
    channels::{
        simplex,
        Simplex,
    },
};
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    collections::{
        BTreeMap,
        HashSet,
    },
    fs::{
        self,
        File,
    },
    io::Write,
    ops::{
        Index,
        IndexMut,
    },
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        Mutex,
        RwLock,
        atomic::AtomicUsize,
    },
    time::{
        Duration,
        Instant,
        SystemTime,
        UNIX_EPOCH,
    },
};

use rand::Rng;

const FINISH_CHECK_INTERVAL: Duration = Duration::from_millis(5); // finer than CHECK_INTERVAL: a close waits on it

// The order in which a close finishes the bots.  A stage is sent its `Finish` only once every bot
// of the stage before it has ended, since those are the ones that send it work and wait on its
// answers.  Finished together, a read or collection still queued behind the `Finish` of its reader
// or collector met a cache bot that had already ended, and waited out `BOT_REQUEST_TIMEOUT` for an
// answer nobody would send; and a record a syncer released late, a failed barrier's error among
// them, reached a cache bot that had ended, so its caller waited out the durability deadline
// (2026-09-24).  The server bots come first: a chunked put's bunch key is sent by its server bot
// only once the chunks are readable, so with the writers finished alongside the servers, a put
// accepted before the close was told `Written` and then lost (A3 QA M2, 2026-10-09).
const FINISH_ORDER: [FinishStage; 5] = [
    FinishStage::Servers,                                                                       // see every put in hand through
    FinishStage::Workers(&[WorkerType::Reader, WorkerType::Scan, WorkerType::InitGarbage]),    // ask the others
    FinishStage::Workers(&[WorkerType::Writer]),                                               // release to the caches
    FinishStage::Workers(&[WorkerType::Cache]),                                                // tell the file bots
    FinishStage::Workers(&[WorkerType::File]),
];

/// A stage of a close, in `FINISH_ORDER`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishStage {
    Servers,
    Workers(&'static [WorkerType]),
}


#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PoolType {
    Cache,
    File,
    InitGarbage,
    Reader,
    Scan,
    Writer,
    Zone,
    Server,
}

impl From<&WorkerType> for PoolType {
    fn from(wtyp: &WorkerType) -> Self {
        match wtyp {
            WorkerType::Cache       => PoolType::Cache,
            WorkerType::File        => PoolType::File,
            WorkerType::InitGarbage => PoolType::InitGarbage,
            WorkerType::Reader      => PoolType::Reader,
            WorkerType::Scan        => PoolType::Scan,
            WorkerType::Writer      => PoolType::Writer,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ChooseBot {
    Randomly,
    ByFile(u32),
}

#[derive(Clone, Debug)]
pub struct ChannelPool<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    typ:    PoolType,
    pool:   Vec<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    ChannelPool<UIDL, UID, ENC, KH>
{
    pub fn new(typ: &PoolType, n: usize) -> Self {
        let mut pool = Vec::new();
        for _ in 0..n {
            pool.push(simplex());
        }
        Self {
            typ: typ.clone(),
            pool: pool,
        }
    }

    pub fn make(typ: &PoolType, pool: Vec<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>) -> Self {
        Self {
            typ: typ.clone(),
            pool: pool,
        }
    }

    fn check_index(&self, ind: usize) -> Outcome<()> {
        if ind > self.pool.len() {
            return Err(err!("Index {} exceeds pool size {}.", ind, self.pool.len(); Index, TooBig));
        }
        Ok(())
    }

    pub fn len(&self) -> usize { self.pool.len() }

    pub fn get_bot(&self, ind: usize) -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        res!(self.check_index(ind));
        Ok(&self.pool[ind])
    }

    pub fn choose_bot(
        &self,
        how: &ChooseBot,
    )
        -> (&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>, BotPoolInd)
    {
        let n = self.pool.len();
        let i = match how {
            ChooseBot::Randomly => {
                let mut rng = rand::thread_rng();
                rng.gen_range(0..n)
            },
            ChooseBot::ByFile(fnum) => (*fnum as usize) % n,
        };
        (&self.pool[i], BotPoolInd::new(i))
    }

    pub fn set_bot(&mut self, ind: usize, chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>) -> Outcome<()> {
        res!(self.check_index(ind));
        self.pool[ind] = chan;
        Ok(())
    }

    pub fn finish_all(&self) -> Outcome<()> {
        for chan in &self.pool {
            res!(chan.send(OzoneMsg::Finish));
        }
        Ok(())
    }

    pub fn msg_count(&self) -> Vec<usize> {
        let mut queues = Vec::new();
        for chan in &self.pool {
            queues.push(chan.len());
        }
        queues
    }

    pub fn msg_count_total(&self) -> usize {
        let mut total: usize = 0;
        for chan in &self.pool {
            total += chan.len();
        }
        total
    }

    pub fn msg_count_non_zero(&self) -> bool {
        let mut pending = false;
        for chan in &self.pool {
            pending = pending | (chan.len() > 0);
        }
        pending
    }

    /// Returns the number of messages sent.
    pub fn send_to_all(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        for chan in &self.pool {
            res!(chan.send(msg.clone()));
        }
        Ok(self.len())
    }
}

/// Channel message queue lengths for all worker bots in a zone.
#[derive(Clone, Debug)]
pub struct ZoneMsgCount {
    cbots:  Vec<usize>,
    fbots:  Vec<usize>,
    igbots: Vec<usize>,
    rbots:  Vec<usize>,
    scbots: Vec<usize>,
    wbots:  Vec<usize>,
}

impl ZoneMsgCount {
    pub fn total(&self) -> usize {
        let mut total = 0;
        total += self.cbots.iter().sum::<usize>();
        total += self.fbots.iter().sum::<usize>();
        total += self.igbots.iter().sum::<usize>();
        total += self.rbots.iter().sum::<usize>();
        total += self.scbots.iter().sum::<usize>();
        total += self.wbots.iter().sum::<usize>();
        total
    }
}

/// Channels for all worker bots in a zone.
#[derive(Clone, Debug)]
pub struct ZoneWorkerChannels<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    cbots:  ChannelPool<UIDL, UID, ENC, KH>,
    fbots:  ChannelPool<UIDL, UID, ENC, KH>,
    igbots: ChannelPool<UIDL, UID, ENC, KH>,
    rbots:  ChannelPool<UIDL, UID, ENC, KH>,
    scbots: ChannelPool<UIDL, UID, ENC, KH>,
    wbots:  ChannelPool<UIDL, UID, ENC, KH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
>
    Index<&WorkerType> for ZoneWorkerChannels<UIDL, UID, ENC, KH> {
    type Output = ChannelPool<UIDL, UID, ENC, KH>;

    fn index(&self, typ: &WorkerType) -> &Self::Output {
        match typ {
            WorkerType::Cache       => &self.cbots,
            WorkerType::File        => &self.fbots,
            WorkerType::InitGarbage => &self.igbots,
            WorkerType::Reader      => &self.rbots,
            WorkerType::Scan        => &self.scbots,
            WorkerType::Writer      => &self.wbots,
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
>
    IndexMut<&WorkerType> for ZoneWorkerChannels<UIDL, UID, ENC, KH>
{
    fn index_mut(&mut self, typ: &WorkerType) -> &mut Self::Output {
        match typ {
            WorkerType::Cache       => &mut self.cbots,
            WorkerType::File        => &mut self.fbots,
            WorkerType::InitGarbage => &mut self.igbots,
            WorkerType::Reader      => &mut self.rbots,
            WorkerType::Scan        => &mut self.scbots,
            WorkerType::Writer      => &mut self.wbots,
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    ZoneWorkerChannels<UIDL, UID, ENC, KH>
{
    /// Create a full set of worker channels for a zone, according to the given configuration.
    pub fn new(cfg: &OzoneConfig) -> Self {
        let nc = cfg.num_bots_per_zone(&WorkerType::Cache);
        let nf = cfg.num_bots_per_zone(&WorkerType::File);
        let nig = cfg.num_bots_per_zone(&WorkerType::InitGarbage);
        let nr = cfg.num_bots_per_zone(&WorkerType::Reader);
        let nsc = cfg.num_bots_per_zone(&WorkerType::Scan);
        let nw = cfg.num_bots_per_zone(&WorkerType::Writer);
        Self {
            cbots:  ChannelPool::new(&PoolType::Cache,      nc),
            fbots:  ChannelPool::new(&PoolType::File,       nf),
            igbots: ChannelPool::new(&PoolType::InitGarbage,nig),
            rbots:  ChannelPool::new(&PoolType::Reader,     nr),
            scbots: ChannelPool::new(&PoolType::Scan,       nsc),
            wbots:  ChannelPool::new(&PoolType::Writer,     nw),
        }
    }

    fn msg_count_non_zero(&self) -> bool {
        self.cbots.msg_count_non_zero()   |
        self.fbots.msg_count_non_zero()   |
        self.igbots.msg_count_non_zero()  |
        self.rbots.msg_count_non_zero()   |
        self.scbots.msg_count_non_zero()  |
        self.wbots.msg_count_non_zero()
    }

    /// Finishes the zone's workers of the given types.
    pub fn finish(&self, typs: &[WorkerType]) -> Outcome<()> {
        for typ in typs {
            res!(self[typ].finish_all());
        }
        Ok(())
    }

    pub fn msg_count(&self) -> ZoneMsgCount {
        ZoneMsgCount {
            cbots:  self.cbots.msg_count(),
            fbots:  self.fbots.msg_count(),
            igbots: self.igbots.msg_count(),
            rbots:  self.rbots.msg_count(),
            scbots: self.scbots.msg_count(),
            wbots:  self.wbots.msg_count(),
        }
    }

    pub fn total_bot_count(&self) -> usize {
        let mut count = 0;
        count += self.cbots.len();
        count += self.fbots.len();
        count += self.igbots.len();
        count += self.rbots.len();
        count += self.scbots.len();
        count += self.wbots.len();
        count
    }

    pub fn send_to_all(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        let mut count = 0;
        count += res!(self.cbots.send_to_all(msg.clone()));
        count += res!(self.fbots.send_to_all(msg.clone()));
        count += res!(self.igbots.send_to_all(msg.clone()));
        count += res!(self.rbots.send_to_all(msg.clone()));
        count += res!(self.scbots.send_to_all(msg.clone()));
        count += res!(self.wbots.send_to_all(msg.clone()));
        Ok(count)
    }
}

/// Message queue lengths for all channels.
#[derive(Clone, Debug)]
pub struct OzoneMsgCount {
    nz:     usize,
    zwbots: Vec<ZoneMsgCount>,
    zbots:  Vec<usize>,
    cfg:    usize,
    sbots:  Vec<usize>,
    sup:    usize,
}

impl OzoneMsgCount {

    pub fn total(&self) -> usize {
        let mut total = 0;
        self.zwbots.iter().for_each(|x| total += x.total());
        self.zbots.iter().for_each(|x| total += x);
        total += self.cfg;
        self.sbots.iter().for_each(|x| total += x);
        total += self.sup;
        total
    }

    pub fn total_zone(&self) -> usize {
        let mut total = 0;
        self.zwbots.iter().for_each(|x| total += x.total());
        self.zbots.iter().for_each(|x| total += x);
        total
    }
}

/// The chunk sets of one database's chunked stores in progress, which the orphan sweep must not
/// retire.  Shared by every clone of the database's `BotChannels`, the one thing all its bots and
/// handles derive from.
pub type PendingSets = Arc<Mutex<SetRegistry>>;

/// The sets held from the draw of their id until their head's outcome is final, and, while a
/// sweep watches, every set let go since it began.  A store that starts and finishes between two
/// looks of a sweep is still seen by the second, so the sweep needs no clock to spare it.
#[derive(Debug, Default)]
pub struct SetRegistry {
    held:       HashSet<u64>,
    watches:    usize,          // sweeps in progress
    released:   HashSet<u64>,   // let go while a sweep watched
}

impl SetRegistry {
    pub fn hold(&mut self, id: u64) {
        self.held.insert(id);
    }

    pub fn release(&mut self, id: &u64) {
        if self.held.remove(id) && self.watches > 0 {
            self.released.insert(*id);
        }
    }

    /// Is no set held?
    pub fn is_empty(&self) -> bool { self.held.is_empty() }

    pub fn is_held(&self, id: &u64) -> bool { self.held.contains(id) }

    /// Begins a watch: until the matching `unwatch`, every set held now or later is in `watched`.
    pub fn watch(&mut self) {
        self.watches = self.watches.saturating_add(1);
    }

    pub fn unwatch(&mut self) {
        self.watches = self.watches.saturating_sub(1);
        if self.watches == 0 {
            self.released.clear();
        }
    }

    /// The sets held now, with those let go during a watch.
    pub fn watched(&self) -> HashSet<u64> {
        let mut all = self.held.clone();
        all.extend(self.released.iter().copied());
        all
    }
}

/// How far the last successful barrier of each live file reached, its data and index ends, by
/// zone and file number.  A record past its file's mark may not survive a power cut, so nothing
/// that discards an older record acts on it until the mark has passed it.  A file with no entry
/// is durable: replayed and synced at start, sealed, or written by a collection that synced it.
/// The writer bot that owns each entry is kept beside it, so that a discard held on a file can
/// ask that writer's syncer for its barrier.
#[derive(Clone, Debug, Default)]
pub struct DurableMarks(Arc<RwLock<BTreeMap<(usize, FileNum), ((u64, u64), usize)>>>);

impl DurableMarks {
    /// Enters a live file of writer `bot` whose bytes up to `ends` are already durable.
    pub fn open(&self, zone: usize, fnum: FileNum, ends: (u64, u64), bot: usize) -> Outcome<()> {
        let mut map = lock_write!(self.0);
        map.insert((zone, fnum), (ends, bot));
        Ok(())
    }

    pub fn set(&self, zone: usize, fnum: FileNum, ends: (u64, u64)) -> Outcome<()> {
        let mut map = lock_write!(self.0);
        if let Some((mark, _)) = map.get_mut(&(zone, fnum)) {
            mark.0 = mark.0.max(ends.0);
            mark.1 = mark.1.max(ends.1);
        }
        Ok(())
    }

    /// The file is wholly durable, sealed by a barrier that covered all of it.
    pub fn remove(&self, zone: usize, fnum: FileNum) -> Outcome<()> {
        let mut map = lock_write!(self.0);
        map.remove(&(zone, fnum));
        Ok(())
    }

    pub fn get(&self, zone: usize, fnum: FileNum) -> Outcome<Option<(u64, u64)>> {
        let map = lock_read!(self.0);
        Ok(map.get(&(zone, fnum)).map(|(m, _)| *m))
    }

    /// The writer bot whose syncer moves the mark of `fnum`, while the file has one.
    pub fn owner(&self, zone: usize, fnum: FileNum) -> Outcome<Option<usize>> {
        let map = lock_read!(self.0);
        Ok(map.get(&(zone, fnum)).map(|(_, b)| *b))
    }

    /// Has a barrier covered the whole record at `floc`?
    pub fn covers(&self, zone: usize, floc: &FileLocation) -> Outcome<bool> {
        let map = lock_read!(self.0);
        Ok(match map.get(&(zone, floc.fnum)) {
            None                => true,
            Some(((d, _), _))   => floc.start + floc.klen + floc.vlen <= *d,
        })
    }

    /// The files of `zone` that still have records no barrier has covered, or may have.
    pub fn files(&self, zone: usize) -> Outcome<Vec<(FileNum, (u64, u64))>> {
        let map = lock_read!(self.0);
        Ok(map.iter().filter(|((z, _), _)| *z == zone).map(|((_, f), (m, _))| (*f, *m)).collect())
    }
}

/// One file a failed barrier leaves in doubt: everything past `upto` is cut.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cut {
    pub zone:       usize,
    pub fnum:       FileNum,
    pub data:       PathBuf,
    pub data_upto:  u64,
    pub index:      PathBuf,
    pub index_upto: u64,
}

impl Cut {
    /// The cut as a line of the fault record or the refusal marker, by zone and file number.
    pub fn line(&self) -> String {
        fmt!("cut {} {} {} {}", self.zone, self.fnum, self.data_upto, self.index_upto)
    }

    /// Cuts the data and index files back to their marks, first copying each tail to the zone's
    /// quarantine as evidence (best effort: the tail may hold page-cache ghosts).  A file is only
    /// ever shortened, never extended, so a cut run twice changes nothing the second time.  A
    /// missing file fails the cut: collection is refused once the store has stopped, so a file
    /// in doubt that is gone was lost, or left behind by a move or a restore.  Returns the bytes
    /// cut from the data and the index.
    pub fn apply(&self, stamp: u64) -> Outcome<(u64, u64)> {
        let d = res!(Self::cut_file(&self.data, self.data_upto, stamp, Some(hooks::Halt::Quarantined)));
        hooks::halt(hooks::Halt::DataCut);
        let i = res!(Self::cut_file(&self.index, self.index_upto, stamp, None));
        Ok((d, i))
    }

    fn cut_file(path: &PathBuf, upto: u64, stamp: u64, halt: Option<hooks::Halt>) -> Outcome<u64> {
        let file = match fs::OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(err!(e,
                "The file in doubt {:?} is missing, so it cannot be cut back to its durable mark \
                {}.  Collection stops with the store, so it was lost, or left behind when the \
                store was moved or restored: put it back, or restore the whole store.", path, upto;
                IO, File, Missing)),
            Err(e) => return Err(err!(e,
                "Cannot open {:?} to cut it back to its durable mark {}.", path, upto;
                IO, File, Write)),
        };
        let len = res!(file.metadata()).len();
        if len <= upto {
            return Ok(0);
        }
        Self::quarantine(path, &file, upto, len, stamp);
        if let Some(at) = halt {
            hooks::halt(at);
        }
        if let Err(e) = file.set_len(upto) {
            return Err(err!(e,
                "Cannot cut {:?} from {} back to its durable mark {}.", path, len, upto;
                IO, File, Write));
        }
        if let Err(e) = file.sync_all() {
            return Err(err!(e,
                "Cannot sync {:?} after cutting it back to {}.", path, upto;
                IO, File, Write));
        }
        if let Some(dir) = path.parent() {
            res!(crate::file::zdir::sync_dir(dir));
        }
        Ok(len - upto)
    }

    // Evidence only, so a failure is logged and the cut goes on.
    fn quarantine(path: &PathBuf, file: &File, upto: u64, len: u64, stamp: u64) {
        let result: Outcome<PathBuf> = (|| {
            let dir = match path.parent() {
                Some(dir)   => dir.join(crate::base::constant::QUARANTINE_DIR),
                None        => return Err(err!("{:?} has no directory.", path; Path, Missing)),
            };
            res!(fs::create_dir_all(&dir));
            let name = match path.file_name() {
                Some(name)  => fmt!("{}.{}", name.to_string_lossy(), stamp),
                None        => return Err(err!("{:?} has no file name.", path; Path, Missing)),
            };
            let qpath = dir.join(name);
            let mut tail = Vec::with_capacity((len - upto) as usize);
            {
                use std::io::{Read, Seek, SeekFrom};
                let mut f = file;
                res!(f.seek(SeekFrom::Start(upto)));
                res!(f.take(len - upto).read_to_end(&mut tail));
            }
            let mut q = res!(File::create(&qpath));
            res!(q.write_all(&tail));
            res!(q.sync_all());
            Ok(qpath)
        })();
        match result {
            Ok(qpath) => warn!(sync_log::stream(),
                "The {} bytes of {:?} past its durable mark {} are kept in {:?}.",
                len - upto, path, upto, qpath),
            Err(e) => error!(sync_log::stream(), err!(e,
                "The {} bytes of {:?} past its durable mark {} could not be kept in quarantine; \
                they are cut all the same.", len - upto, path, upto;
                IO, File, Write)),
        }
    }
}

/// What stopped a store writing.  Every message about the stop, and the refusal marker's `why`
/// line, is derived from it, so an operator is sent after the right path (QA APPENDSEEK F1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultCause {
    Barrier,    // a durability barrier (sync) failed
    CutBack,    // a torn append could not be cut back off its live pair
}

impl FaultCause {
    /// The word for the cause in the fault record and the refusal marker.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Barrier   => "barrier",
            Self::CutBack   => "cut-back",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        match word.trim() {
            "barrier"   => Some(Self::Barrier),
            "cut-back"  => Some(Self::CutBack),
            _           => None,
        }
    }

    /// The cause named in a fault record or refusal marker text, if it names one.
    pub fn read(text: &str) -> Option<Self> {
        text.lines().find_map(|l| l.strip_prefix("cause ").and_then(Self::from_word))
    }

    /// What failed, as it follows "after" in a message.
    pub fn what(&self) -> &'static str {
        match self {
            Self::Barrier   => "a failed sync",
            Self::CutBack   => "a failed cut back of a torn append",
        }
    }

    /// The marker's `why` line, given the error's plain text.
    pub fn why(&self, error: &str) -> String {
        match self {
            Self::Barrier   => fmt!("a sync failed at run time: {}", error),
            Self::CutBack   => fmt!("a torn append could not be cut back at run time: {}", error),
        }
    }

    /// Where the open cut the files in doubt back to, as it follows "cut back to".
    pub fn to(&self) -> &'static str {
        match self {
            Self::Barrier   => "their last durable marks",
            Self::CutBack   => "the lengths in the fault record",
        }
    }
}

impl std::fmt::Display for FaultCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.what())
    }
}

/// Why and where a store stopped writing.
#[derive(Clone, Debug)]
pub struct FaultReport {
    pub when:   SystemTime,
    pub cause:  FaultCause, // set by the first raise
    pub error:  String,
    pub cuts:   Vec<Cut>,
    pub record: Option<PathBuf>, // the durable fault record, if it could be written
    pub refusal: Option<PathBuf>, // the refusal marker, if it could be written
}

impl FaultReport {
    /// Was the stop written down durably, as the fault record or the refusal marker?  A process
    /// that exits on a fault, as the gateway does, exits only then: otherwise a restart would be
    /// told nothing of the failed sync and replay what is not on the disk, so the process stays
    /// up with writes refused and raises the alarm.
    pub fn recorded(&self) -> bool {
        self.record.is_some() || self.refusal.is_some()
    }

    /// The headline every refusal on the stopped store leads with, derived from the cause.
    pub fn stopped(&self) -> String {
        fmt!("The store stopped writing after {} at {:?} ({})", self.cause, self.when, self.error)
    }
}

/// The store's fault: set by the first failed barrier or failed cut back, and never cleared in
/// the process.  After a failed fsync the page cache cannot be trusted and no later fsync can
/// prove anything, so the store stops writing and the next open cuts each file in doubt back to
/// its mark, or to the length named for it.
#[derive(Clone, Debug, Default)]
pub struct Fault(Arc<Mutex<Option<FaultReport>>>);

impl Fault {
    pub fn get(&self) -> Option<FaultReport> {
        match self.0.lock() {
            Ok(f)   => f.clone(),
            // A poisoned lock says a thread panicked holding it, which is no proof of health.
            Err(p)  => p.into_inner().clone(),
        }
    }

    pub fn is_set(&self) -> bool {
        self.get().is_some()
    }

    /// Sets the fault and writes the record of it into `root`, durably, then the refusal marker
    /// (`Refusal`), so no open replays the store until an operator has looked.  The first raise
    /// sets the time, the cause and the error; a later one adds its cuts.  A record that cannot
    /// be written leaves the store stopped in this process, and is logged as critical, naming the
    /// cuts.
    pub fn raise(
        &self,
        root:   &PathBuf,
        cause:  FaultCause,
        cuts:   Vec<Cut>,
        error:  &Error<ErrTag>,
    )
        -> FaultReport
    {
        let mut f = match self.0.lock() {
            Ok(f)   => f,
            Err(p)  => p.into_inner(),
        };
        // A second writer failing later adds its cuts to the record, which is written again.
        let mut report = match f.take() {
            Some(mut report) => {
                for c in cuts {
                    if !report.cuts.contains(&c) {
                        report.cuts.push(c);
                    }
                }
                report
            },
            None => FaultReport {
                when:   SystemTime::now(),
                cause,
                error:  error.plain(), // plain text, for the files an operator reads
                cuts,
                record: None,
                refusal: None,
            },
        };
        // `record` and `refusal` each say that their file holds every cut in the report, so a
        // rewrite that fails withdraws the claim, even though an older file still stands.
        match Self::write_record(root, &report) {
            Ok(path) => report.record = Some(path),
            Err(e) => {
                report.record = None;
                error!(sync_log::stream(), err!(e,
                    "CRITICAL: The store at {:?} stopped after {}, but its fault record could \
                    not be written, so a restart will not cut the files in doubt unless the \
                    refusal marker holds them.  Before restarting, truncate them by hand: {:?}",
                    root, report.cause, report.cuts;
                    IO, File, Write, Fatal));
            },
        }
        hooks::halt(hooks::Halt::Recorded);
        // The marker is written anew on every raise with every cut, so that either file alone
        // is a whole record, and clearing the marker folds its cuts into the record (QA r2 F2).
        let why = report.cause.why(&report.error);
        match Refusal::write(root, &why, Some(report.cause), &report.cuts) {
            Ok(path) => report.refusal = Some(path),
            Err(e) => {
                report.refusal = None;
                error!(sync_log::stream(), err!(e,
                    "The store at {:?} stopped after {}, and its refusal marker could not be \
                    written with every cut: {:?}", root, report.cause, report.cuts;
                    IO, File, Write, Fatal));
            },
        }
        *f = Some(report.clone());
        report
    }

    pub fn record_path(root: &PathBuf) -> PathBuf {
        root.join(crate::base::constant::FAULT_FILENAME)
    }

    // Each cut names its zone and file number, not a path, so a store moved or restored to
    // another root is cut where it now stands (QA F2).
    fn write_record(root: &PathBuf, report: &FaultReport) -> Outcome<PathBuf> {
        if hooks::fault_record_write_fails() || hooks::fault_record_fails() {
            return Err(err!("The fault record could not be written (test::hooks)."; IO, Write));
        }
        let secs = report.when.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut text = fmt!("when {}\ncause {}\nerror {}\n",
            secs, report.cause.word(), report.error.replace('\n', " "));
        for c in &report.cuts {
            text.push_str(&c.line());
            text.push('\n');
        }
        crate::file::zdir::write_whole(root, constant::FAULT_FILENAME, &text,
            Some(hooks::Halt::RecordSynced))
    }

    /// Reads a fault record left by an earlier run, placing each cut in `zdirs`, the zone
    /// directories as the store now stands.  Anything after a cut's four numbers, such as the
    /// absolute paths an older record carried, is ignored.
    pub fn read_record(path: &PathBuf, zdirs: &[PathBuf]) -> Outcome<Vec<Cut>> {
        let text = res!(fs::read_to_string(path));
        let mut cuts = Vec::new();
        for line in text.lines() {
            let rest = match line.strip_prefix("cut ") {
                Some(rest)  => rest,
                None        => continue,
            };
            let mut it = rest.split(' ');
            let mut num = |what: &str| -> Outcome<u64> {
                let field = res!(it.next().ok_or_else(|| err!(
                    "Fault record {:?}: a cut has no {}: {:?}", path, what, line; Decode, Missing)));
                Ok(res!(field.parse::<u64>()))
            };
            let zone = res!(num("zone")) as usize;
            let fnum = res!(num("file number")) as FileNum;
            let data_upto = res!(num("data mark"));
            let index_upto = res!(num("index mark"));
            let dir = res!(zdirs.get(zone).ok_or_else(|| err!(
                "Fault record {:?}: a cut names zone {}, but the store has {} zones: {:?}",
                path, zone + 1, zdirs.len(), line; Decode, Invalid)));
            cuts.push(Cut {
                zone,
                fnum,
                data:       dir.join(ZoneDir::relative_file_path(&FileType::Data, fnum)),
                data_upto,
                index:      dir.join(ZoneDir::relative_file_path(&FileType::Index, fnum)),
                index_upto,
            });
        }
        Ok(cuts)
    }

    /// Applies the cuts left by an earlier run to the zone directories `zdirs` of the store at
    /// `root`, then removes the fault record.  The cuts are those of the record and of the
    /// refusal marker together, since each is written with every cut and either may be the one
    /// that survived; the marker itself is the operator's to clear.  A cut only ever shortens a
    /// file, so one interrupted part way is simply run again; any failure, a missing file in doubt
    /// among them, refuses the open and leaves the record for the next attempt.
    pub fn cut_after(root: &Path, zdirs: &[PathBuf]) -> Outcome<()> {
        let path = Self::record_path(&root.to_path_buf());
        let marker = Refusal::path(root);
        let mut cuts: Vec<Cut> = Vec::new();
        // The cause the record or the marker names; one written before causes were kept names
        // none, and the messages then say only that the store stopped.
        let mut cause: Option<FaultCause> = None;
        for (src, what) in [(&path, "fault record"), (&marker, "refusal marker")] {
            if !src.exists() {
                continue;
            }
            if cause.is_none() {
                if let Ok(text) = fs::read_to_string(src) {
                    cause = FaultCause::read(&text);
                }
            }
            match Self::read_record(src, zdirs) {
                Ok(more) => for c in more {
                    if !cuts.contains(&c) {
                        cuts.push(c);
                    }
                },
                Err(e) => return Err(err!(e,
                    "The store at {:?} stopped writing, and its {} {:?} cannot be read, so the \
                    files in doubt cannot be cut.", root, what, src;
                    Init, File, Read)),
            }
        }
        if cuts.is_empty() && !path.exists() {
            return Ok(());
        }
        let named = if path.exists() { path.clone() } else { marker.clone() };
        let (after, to) = match cause {
            Some(c) => (fmt!(" after {}", c), c.to()),
            None    => (String::new(), "the lengths named for them"),
        };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut done = Vec::new();
        for c in &cuts {
            match c.apply(stamp) {
                Ok((d, i)) => done.push(fmt!("file {} of zone {}: {} data and {} index bytes",
                    c.fnum, c.zone + 1, d, i)),
                Err(e) => return Err(err!(e,
                    "The store at {:?} stopped writing{}, and file {} of zone {} could not be \
                    cut back to the length named for it, so it is not opened.  The cut is named \
                    in {:?}, left over from a stop of the store.  Remove that file only if the \
                    store was restored whole from a copy taken before the stop; otherwise put \
                    the file back, or restore the whole store.",
                    root, after, c.fnum, c.zone + 1, named;
                    Init, File, Write)),
            }
        }
        // Where the marker alone named the cuts, it stands until the operator clears it.
        if path.exists() {
            if let Err(e) = fs::remove_file(&path) {
                return Err(err!(e,
                    "The files in doubt at {:?} are cut, but the fault record {:?} cannot be \
                    removed.", root, path;
                    Init, File, Write));
            }
        }
        res!(crate::file::zdir::sync_dir(root));
        error!(sync_log::stream(), err!(
            "The store at {:?} had stopped writing{}.  The files in doubt were cut back to {} \
            before replay: {}.", root, after, to, done.join("; ");
            Init, File, Fatal));
        Ok(())
    }
}

/// Channels for all bots in all zones.  Rather than sharing references to these channels, clone them.  Unlike `bots::base::handles::BotHandles`, this includes the `Supervisor`.
#[derive(Clone, Debug)]
pub struct BotChannels<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    nz:     usize,
    zwbots: Vec<ZoneWorkerChannels<UIDL, UID, ENC, KH>>,
    zbots:  ChannelPool<UIDL, UID, ENC, KH>,
    cfg:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    sbots:  ChannelPool<UIDL, UID, ENC, KH>,
    sup:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    sets:   PendingSets,
    marks:  DurableMarks,
    fault:  Fault,
    held:   Arc<AtomicUsize>,   // effects waiting on their winner's barrier, a gauge
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    BotChannels<UIDL, UID, ENC, KH>
{
    /// Create a full set of functioning channels according to the given configuration.
    pub fn new(cfg: &OzoneConfig) -> Self {
        let nz = cfg.num_zones();
        let mut zwbots = Vec::new();
        for _ in 0..nz {
            zwbots.push(ZoneWorkerChannels::new(cfg));
        }
        Self {
            nz,
            zwbots,
            zbots:  ChannelPool::new(&PoolType::Zone, nz),
            cfg:    simplex(),
            sbots:  ChannelPool::new(&PoolType::Server, cfg.num_sbots()),
            sup:    simplex(),
            sets:   Arc::new(Mutex::new(SetRegistry::default())),
            marks:  DurableMarks::default(),
            fault:  Fault::default(),
            held:   Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn pending_sets(&self) -> &PendingSets { &self.sets }
    pub fn marks(&self) -> &DurableMarks { &self.marks }
    pub fn fault(&self) -> &Fault { &self.fault }
    pub fn held_effects(&self) -> &Arc<AtomicUsize> { &self.held }

    /// Returns channels for all worker pools, for the given zone.
    pub fn get_all_workers_in_zone(
        &self,
        zind: &ZoneInd,
    )
        -> Outcome<ZoneWorkerChannels<UIDL, UID, ENC, KH>>
    {
        res!(self.check_zone_index(**zind));
        Ok(self.zwbots[**zind].clone())
    }

    /// Returns channels for the worker pool of the given type, for the given zone.
    pub fn get_workers_of_type_in_zone(
        &self,
        wtyp:   &WorkerType,
        zind:   &ZoneInd,
    )
        -> Outcome<ChannelPool<UIDL, UID, ENC, KH>>
    {
        res!(self.check_zone_index(**zind));
        Ok(self.zwbots[**zind][wtyp].clone())
    }

    /// Returns channels for all worker pools of the given type, across all zones.
    pub fn get_all_workers_of_type(
        &self,
        wtyp: &WorkerType,
    )
        -> Vec<ChannelPool<UIDL, UID, ENC, KH>>
    {
        let mut pools = Vec::new();
        for z in 0..self.nz {
            pools.push(self.zwbots[z][wtyp].clone());           
        }
        pools
    }

    pub fn all_zwbots(&self)    -> &Vec<ZoneWorkerChannels<UIDL, UID, ENC, KH>>  { &self.zwbots }
    pub fn all_zbots(&self)     -> &ChannelPool<UIDL, UID, ENC, KH>              { &self.zbots }
    pub fn cfg(&self)           -> &Simplex<OzoneMsg<UIDL, UID, ENC, KH>>        { &self.cfg }
    pub fn all_sbots(&self)     -> &ChannelPool<UIDL, UID, ENC, KH>              { &self.sbots }
    pub fn sup(&self)           -> &Simplex<OzoneMsg<UIDL, UID, ENC, KH>>        { &self.sup }

    pub fn get_sbot(&self, sind: &BotPoolInd) -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        self.sbots.get_bot(**sind)
    }

    pub fn get_zbot(&self, zind: &ZoneInd) -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        self.zbots.get_bot(**zind)
    }

    pub fn get_zwbots(&self, zind: &ZoneInd) -> Outcome<&ZoneWorkerChannels<UIDL, UID, ENC, KH>> {
        if **zind > self.zwbots.len() {
            return Err(err!(
                "Index {} exceeds number of zones {}.", **zind, self.zwbots.len();
                Index, TooBig));
        }
        Ok(&self.zwbots[**zind])
    }

    pub fn get_bot(
        &self,
        ozid: &OzoneBotId,
    )
        -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>
    {
        Ok(match ozid {
            // Solo bots
            OzoneBotId::ConfigBot(..) => self.cfg(),
            OzoneBotId::Supervisor(..) => self.sup(),
            OzoneBotId::ServerBot(_, bpind) => res!(self.get_sbot(bpind)),
            OzoneBotId::ZoneBot(_, zind) => res!(self.get_zbot(zind)),
            OzoneBotId::CacheBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Cache].get_bot(**bpind)),
            OzoneBotId::FileBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::File].get_bot(**bpind)),
            OzoneBotId::InitGarbageBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::InitGarbage].get_bot(**bpind)),
            OzoneBotId::ReaderBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Reader].get_bot(**bpind)),
            OzoneBotId::ScanBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Scan].get_bot(**bpind)),
            OzoneBotId::WriterBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Writer].get_bot(**bpind)),
            _ => return Err(err!(
                "Cannot return channel for {:?}.", ozid;
                Bug, Invalid)),
        })
    }

    // Mutate
    pub fn zbots_mut(&mut self) -> &mut ChannelPool<UIDL, UID, ENC, KH>          { &mut self.zbots }
    pub fn cfg_mut(&mut self)   -> &mut Simplex<OzoneMsg<UIDL, UID, ENC, KH>>    { &mut self.cfg }
    pub fn sbots_mut(&mut self) -> &mut ChannelPool<UIDL, UID, ENC, KH>          { &mut self.sbots }
    pub fn sup_mut(&mut self)   -> &mut Simplex<OzoneMsg<UIDL, UID, ENC, KH>>    { &mut self.sup }

    pub fn set_sbot(
        &mut self,
        bpind:  &BotPoolInd,
        chan:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        self.sbots.set_bot(**bpind, chan)
    }

    pub fn set_zbot(
        &mut self,
        zind: &ZoneInd,
        chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        self.zbots.set_bot(**zind, chan)
    }

    pub fn set_cfg(&mut self, chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>)   { self.cfg = chan; }
    pub fn set_sup(&mut self, chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>)   { self.sup = chan; }

    pub fn set_worker_bot(
        &mut self,
        wtyp:   &WorkerType,
        wind:   &WorkerInd,
        chan:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        res!(self.check_zone_index(wind.z()));
        self.zwbots[wind.z()][wtyp].set_bot(wind.b(), chan)
    }

    fn check_zone_index(&self, ind: usize) -> Outcome<()> {
        if ind > self.nz {
            return Err(err!(
                "Zone index {} into BotChannels exceeds number of zones {}.",
                ind, self.nz;
                Index, TooBig));
        }
        Ok(())
    }

    /// Sends a `Finish` to every bot but the supervisor: stage by stage in `FINISH_ORDER`, the
    /// servers and then each zone's workers, then the zone bots and the config bot.  Before each
    /// stage it waits until `ended` says every bot of the stage before it has ended.  It
    /// stops waiting at `until` and returns the stage it was waiting on, already finished, for
    /// `finish_from` to carry on from; `None` once every bot has been sent its `Finish`.
    pub fn finish_all<F: Fn(FinishStage) -> bool>(
        &self,
        ended:  F,
        until:  Instant,
    )
        -> Outcome<Option<usize>>
    {
        warn!(sync_log::stream(), "Shutdown: Finishing the bots in order, waiting up to {:?} for \
            them.", until.saturating_duration_since(Instant::now()));
        res!(self.finish_stage(0));
        self.finish_from(0, ended, until)
    }

    /// Carries on the `finish_all` that stopped waiting on `stage`.  An `ended` that is always
    /// true finishes everything left at once.
    pub fn finish_from<F: Fn(FinishStage) -> bool>(
        &self,
        stage:  usize,
        ended:  F,
        until:  Instant,
    )
        -> Outcome<Option<usize>>
    {
        let mut stage = stage;
        while stage + 1 < FINISH_ORDER.len() {
            let left = until.saturating_duration_since(Instant::now());
            let (start, timed_out) = res!(oxedyne_fe2o3_core::time::wait_for_true(
                FINISH_CHECK_INTERVAL.min(left),
                left,
                || ended(FINISH_ORDER[stage]),
            ));
            if timed_out {
                // Counted, not listed: a channel can only be read by taking its messages, and
                // taken to be listed here, the records the writers had just released were
                // destroyed, and their callers waited out the durability deadline for writes that
                // had landed (2026-09-23).
                warn!(sync_log::stream(), "Shutdown: The {:?} bots had not all ended after {:?}, \
                    so those after them wait; pending: {:?}",
                    FINISH_ORDER[stage], start.elapsed(), self.msg_count());
                return Ok(Some(stage));
            }
            stage += 1;
            res!(self.finish_stage(stage));
        }
        res!(self.zbots.finish_all());
        res!(self.cfg().send(OzoneMsg::Finish));
        warn!(sync_log::stream(), "Shutdown: Every bot has been sent its completion request.");
        Ok(None)
    }

    fn finish_stage(&self, stage: usize) -> Outcome<()> {
        match FINISH_ORDER[stage] {
            FinishStage::Servers => {
                res!(self.sbots.finish_all());
            },
            FinishStage::Workers(typs) => for zone in &self.zwbots {
                res!(zone.finish(typs));
            },
        }
        Ok(())
    }

    pub fn fwd_to_all_zones(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        self.zbots.send_to_all(msg)
    }

    pub fn send_to_all(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        let mut count = 0;
        for zwbot in &self.zwbots {
            count += res!(zwbot.send_to_all(msg.clone()));
        }
        count += res!(self.zbots.send_to_all(msg.clone()));
        {
            res!(self.cfg.send(msg.clone()));
            count += 1;
        }
        count += res!(self.sbots.send_to_all(msg.clone()));
        {
            res!(self.sup.send(msg.clone()));
            count += 1;
        }
        Ok(count)
    }

    pub fn msg_count(&self) -> OzoneMsgCount {
        let mut zone_counts = Vec::new();
        for zone in &self.zwbots {
            zone_counts.push(zone.msg_count());
        }
        OzoneMsgCount {
            nz:     self.nz,
            zwbots: zone_counts,
            zbots:  self.zbots.msg_count(),
            cfg:    self.cfg.len(),
            sbots:  self.sbots.msg_count(),
            sup:    self.sup.len(),
        }
    }

    pub fn dump_pending_messages(
        lines:  Vec<String>, // obtain using drain_messages
        label:  &str,
        z:      Option<usize>,
        b:      Option<usize>,
    ) {
        match (z, b) {
            (Some(z), Some(b)) => debug!(sync_log::stream(), " Z{} B{} {} messages ({}):", z, b, label, lines.len()),
            (Some(z), None) => debug!(sync_log::stream(), " Z{} {} messages ({}):", z, label, lines.len()),
            (None, None) => debug!(sync_log::stream(), " {} messages ({}):", label, lines.len()),
            _ => (),
        }
        if lines.len() > 0 {
            for line in lines {
                debug!(sync_log::stream(), "  {}", line);
            }
        }
    }
}
