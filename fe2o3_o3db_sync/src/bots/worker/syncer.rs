//! A writer's durability barrier, run on a thread of its own.
//!
//! A writer bot appends a record and goes straight on to the next; its syncer makes each record
//! durable as the store's sync policy asks and only then releases it to its cache bot, which makes
//! it readable and gives the caller its final answer.  The barrier used to run inside the writer,
//! so one fsync held up by the rest of the machine's traffic -- eleven seconds was measured on
//! 2026-09-23 -- held every record queued behind it past the caller's six-second deadline, and each
//! was reported as failed although each went on to land.
//!
//! Records are released in the order they were written, so a record reaches its cache bot, and
//! its accounting reaches the file bots, only after every record written before it.  A file bot
//! collects a sealed file only once the file's accounted size has caught up with its size on
//! disk, so holding records here delays a collection and never races one.
//!
//! Since 2026-10-10 each successful barrier publishes how far it reached, the file's mark in
//! `DurableMarks`, and then tells every cache bot it released a record to since the last one.
//! Anything that discards an older record waits on its winner's mark (F1).  A failed barrier
//! poisons the syncer for good: after a failed fsync the kernel may have dropped the dirty pages
//! and a second fsync can succeed over the loss, so no later barrier proves anything (F2).  The
//! store is stopped, its fault recorded durably, and the writer cuts the files in doubt back to
//! their marks.

use crate::{
    prelude::*,
    base::cfg::OzoneConfig,
    comm::{
        channels::{
            Cut,
            DurableMarks,
            Fault,
            FaultCause,
        },
        msg::OzoneMsg,
        response::Responder,
    },
    file::floc::FileNum,
    test::hooks,
};

use oxedyne_fe2o3_core::channels::Simplex;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    collections::{
        BTreeMap,
        VecDeque,
    },
    fs::File,
    mem,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
        mpsc::{
            self,
            Receiver,
            RecvTimeoutError,
            Sender,
        },
    },
    thread::{
        self,
        JoinHandle,
    },
    time::{
        Duration,
        Instant,
    },
};


/// When a record has to be durable before it is released, and so how long the discards a
/// replacement makes wait: an older record is let go only once its winner is durable.  `Never`
/// and `EveryN` mean no barrier per write, not none at all: a discard held on a winner is owed a
/// barrier within `constant::HELD_BARRIER` under every policy, one for all held in that time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncPolicy {
    EveryWrite,         // `sync_on_write`
    EveryN(u32),        // `sync_every_n_writes`
    Interval(Duration), // `sync_interval_ms`
    Never,
}

impl SyncPolicy {
    /// The policy a configuration asks for, the strongest first where it asks for several.
    pub fn of(cfg: &OzoneConfig) -> Self {
        if cfg.sync_on_write {
            Self::EveryWrite
        } else if cfg.sync_every_n_writes > 0 {
            Self::EveryN(cfg.sync_every_n_writes)
        } else if cfg.sync_interval_ms > 0 {
            Self::Interval(Duration::from_millis(cfg.sync_interval_ms))
        } else {
            Self::Never
        }
    }
}

/// What a writer hands its syncer, in the order it wrote.  A `Pair` names the live data and index
/// files every later record is appended to, with their sizes, durable already; the pair it
/// replaces is sealed, made durable before any record written to the new one is released.  A
/// `Record` is one appended record with the insert that releases it to its cache bot, and the
/// data and index ends after it.  `SyncNow` asks for a barrier over everything handed before it,
/// `SyncOwed` for one within `constant::HELD_BARRIER`.
pub enum Handed<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    Pair(LiveFiles),
    Record {
        cbot:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
        cbpind: usize,
        insert: OzoneMsg<UIDL, UID, ENC, KH>,
        resp:   Responder<UIDL, UID, ENC, KH>,
        policy: SyncPolicy,
        ends:   (u64, u64),
    },
    SyncNow(Responder<UIDL, UID, ENC, KH>),
    SyncOwed,
    Finish,
}

/// A live pair as its syncer holds it.
pub struct LiveFiles {
    pub fnum:   FileNum,
    pub dat:    File,
    pub ind:    File,
    pub paths:  (PathBuf, PathBuf),
    pub sizes:  (u64, u64), // durable already: synced at open, or new
}

/// What a syncer shares with its store.
#[derive(Clone)]
pub struct Shared<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    pub zone:   usize,
    pub bot:    usize,              // the writer's, entered beside each mark it opens
    pub root:   PathBuf,
    pub marks:  DurableMarks,
    pub fault:  Fault,
    pub writer: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>, // told to cut its files at the fault
}

/// A writer's handle on its syncer.
pub struct Syncer<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    tx:         Sender<Handed<UIDL, UID, ENC, KH>>,
    handle:     Option<JoinHandle<()>>,
    running:    Arc<AtomicBool>,    // cleared by the thread's barrier as it goes, however it goes
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    Syncer<UIDL, UID, ENC, KH>
{
    pub fn start(
        label:          String,
        log_stream_id:  String,
        shared:         Shared<UIDL, UID, ENC, KH>,
    )
        -> Outcome<Self>
    {
        // A plain channel rather than a `Simplex`, whose two ends travel together: a writer must
        // learn from its own send that the syncer has gone, rather than queue records that nothing
        // will ever release.
        let (tx, rx) = mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let cleared = running.clone();
        let builder = thread::Builder::new()
            .name(fmt!("{}.sync", label))
            .stack_size(constant::STACK_SIZE);
        let handle = res!(builder.spawn(move || {
            sync_log::set_stream(log_stream_id);
            Barrier::new(label, rx, cleared, shared).run();
        }));
        Ok(Self {
            tx,
            handle:     Some(handle),
            running,
        })
    }

    /// Is the syncer still taking records?  Once it is not, nothing more written through its
    /// writer can be confirmed.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn hand(&self, item: Handed<UIDL, UID, ENC, KH>) -> Outcome<()> {
        match self.tx.send(item) {
            Ok(()) => Ok(()),
            Err(_) => Err(err!(
                "The durability barrier thread has stopped, so no write from here on can be \
                confirmed.";
                Thread, Missing)),
        }
    }

    /// Releases everything handed over, with a last barrier under every policy, and ends the
    /// thread.
    pub fn finish(&mut self) -> Outcome<()> {
        // A syncer that has already stopped cannot be sent this, and the join says why.
        let _ = self.tx.send(Handed::Finish);
        match self.handle.take() {
            None => Ok(()),
            Some(handle) => match handle.join() {
                Ok(()) => Ok(()),
                Err(_) => Err(err!(
                    "The durability barrier thread panicked.";
                    Thread, Panic)),
            },
        }
    }
}

struct Barrier<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    label:      String,
    rx:         Receiver<Handed<UIDL, UID, ENC, KH>>,
    queue:      VecDeque<Handed<UIDL, UID, ENC, KH>>,
    pair:       Option<LiveFiles>,
    taken:      (u64, u64),         // the ends of the last record taken in from the pair
    doubt:      Vec<(FileNum, (PathBuf, PathBuf))>, // this writer's files with a mark entry
    told:       BTreeMap<usize, Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>, // cache bots released to since the last barrier
    policy:     SyncPolicy,         // the latest record's
    dirty:      bool,               // the pair holds records no barrier has covered
    since:      u32,                // records released since the last barrier
    last:       Option<Instant>,    // when the last barrier completed
    owed:       Option<Instant>,    // a held discard's barrier is due by then
    poisoned:   Option<Error<ErrTag>>, // the failed barrier, for good
    shared:     Shared<UIDL, UID, ENC, KH>,
    running:    Arc<AtomicBool>,    // shared with the writer's handle
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    Barrier<UIDL, UID, ENC, KH>
{
    fn new(
        label:      String,
        rx:         Receiver<Handed<UIDL, UID, ENC, KH>>,
        running:    Arc<AtomicBool>,
        shared:     Shared<UIDL, UID, ENC, KH>,
    )
        -> Self
    {
        Self {
            label,
            rx,
            queue:      VecDeque::new(),
            pair:       None,
            taken:      (0, 0),
            doubt:      Vec::new(),
            told:       BTreeMap::new(),
            policy:     SyncPolicy::Never,
            dirty:      false,
            since:      0,
            last:       None,
            owed:       None,
            poisoned:   None,
            shared,
            running,
        }
    }

    fn run(mut self) {
        loop {
            if self.queue.is_empty() {
                // The interval policy owes the disk a barrier on its period whether or not another
                // record arrives, so the writes that end a burst are made durable too; a held
                // discard is owed one under every policy.
                let item = match self.owed_in() {
                    Some(wait) => match self.rx.recv_timeout(wait) {
                        Ok(item) => Some(item),
                        Err(RecvTimeoutError::Timeout) => {
                            self.barrier_or_log();
                            continue;
                        },
                        Err(RecvTimeoutError::Disconnected) => None,
                    },
                    None => self.rx.recv().ok(),
                };
                match item {
                    Some(item) => self.queue.push_back(item),
                    // The writer has gone without a word, so nothing more is coming.
                    None => return self.end(false),
                }
            }
            // Read as the batch begins, so that a batch already under way when a test arms the
            // stop, as the one releasing the write the test waited on can be, does not stop.
            let stop_armed = hooks::syncer_stop_armed();
            // Take whatever else is waiting, so that one barrier can cover all of it.
            while let Ok(item) = self.rx.try_recv() {
                self.queue.push_back(item);
            }
            while let Some(item) = self.queue.pop_front() {
                match item {
                    Handed::Pair(live) => self.seal(live),
                    Handed::Record { cbot, cbpind, insert, resp, policy, ends } =>
                        self.records(cbot, cbpind, insert, resp, policy, ends),
                    Handed::SyncNow(resp) => {
                        let result = match (&self.poisoned, self.dirty) {
                            (Some(e), _)    => Err(e.clone()),
                            (None, true)    => self.barrier(),
                            (None, false)   => Ok(()),
                        };
                        if resp.is_some() {
                            let _ = resp.send(match result {
                                Ok(())  => OzoneMsg::Ok,
                                Err(e)  => OzoneMsg::Error(e),
                            });
                        }
                    },
                    // Only the first in a period sets the time, so all of them share one barrier.
                    // A clean pair needs none: its mark already covers what is held.
                    Handed::SyncOwed => if self.dirty && self.owed.is_none() && !hooks::owed_dropped() {
                        self.owed = Some(Instant::now() + constant::HELD_BARRIER);
                    },
                    Handed::Finish => return self.end(true),
                }
            }
            // A syncer stopping as one that panicked would, with no last barrier.
            if stop_armed && hooks::syncer_stops() {
                return;
            }
            // A steady stream of records never lets the wait above time out, so a held discard's
            // barrier is also checked here, lest the writes that keep coming starve it.
            if let Some(due) = self.owed {
                if self.dirty && self.poisoned.is_none() && Instant::now() >= due {
                    self.barrier_or_log();
                }
            }
        }
    }

    /// Releases one record, and with it the run queued behind it under `sync_on_write`, where
    /// every record waits on a barrier and one barrier covers every record appended before it
    /// begins.  Once the syncer is poisoned no barrier runs, and every record is released with
    /// the failure as its caller's answer.
    fn records(
        &mut self,
        cbot:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
        cbpind: usize,
        insert: OzoneMsg<UIDL, UID, ENC, KH>,
        resp:   Responder<UIDL, UID, ENC, KH>,
        policy: SyncPolicy,
        ends:   (u64, u64),
    ) {
        let mut run = vec![(cbot, cbpind, insert, resp)];
        self.taken = ends;
        if policy == SyncPolicy::EveryWrite {
            while let Some(Handed::Record { policy: next, .. }) = self.queue.front() {
                if *next != policy {
                    break;
                }
                if let Some(Handed::Record { cbot, cbpind, insert, resp, ends, .. }) = self.queue.pop_front() {
                    run.push((cbot, cbpind, insert, resp));
                    self.taken = ends;
                }
            }
        }
        self.policy = policy;
        self.dirty = true;
        self.since = self.since.saturating_add(run.len() as u32);
        let due = match policy {
            SyncPolicy::EveryWrite  => true,
            SyncPolicy::EveryN(n)   => self.since >= n,
            SyncPolicy::Interval(p) => match self.last {
                None        => true,
                Some(last)  => last.elapsed() >= p,
            },
            SyncPolicy::Never       => false,
        };
        let synced = match (&self.poisoned, due) {
            (Some(e), _)    => Err(e.clone()),
            (None, true)    => self.barrier(),
            (None, false)   => Ok(()),
        };
        for (cbot, cbpind, insert, resp) in run {
            // The caller hears when the barrier failed, from the cache bot once the record is
            // readable, as it hears of one that did not: told from here first, it could read its
            // write back and not find it.  The record is released all the same: it is in the
            // files whatever the barrier said, and holding it back would leave the cache
            // disagreeing with them and its bytes accounted to no one.
            let insert = match (&synced, insert) {
                (Ok(()), insert) => insert,
                (Err(e), OzoneMsg::Insert(k, v, c, f, i, m, r, _, t)) =>
                    OzoneMsg::Insert(k, v, c, f, i, m, r, Some(Self::written(e.clone())), t),
                // Not an insert, so nothing the cache bot would answer with: told from here.
                (Err(e), other) => {
                    Self::tell(&resp, e.clone());
                    other
                },
            };
            match cbot.send(insert) {
                // Released before a barrier covers it: told when one does.
                Ok(()) => if self.dirty {
                    self.told.insert(cbpind, cbot);
                },
                Err(e) => {
                    let e = err!(e,
                        "{}: A written record could not be released to its cache bot.", self.label;
                        Channel, Write);
                    error!(sync_log::stream(), e.clone());
                    Self::tell(&resp, e);
                },
            }
        }
    }

    /// Seals the live pair and takes the next.  Sealing is unconditional, whatever the policy: a
    /// file that is no longer live is durable before anything written after it is released, and
    /// its mark entry goes, which makes every record in it durable.  The new pair is entered at
    /// the sizes it was opened with before anything of it is released.
    fn seal(&mut self, live: LiveFiles) {
        if self.dirty {
            self.barrier_or_log();
        }
        if self.poisoned.is_none() {
            if let Some(fnum) = self.pair.as_ref().map(|p| p.fnum) {
                if let Err(e) = self.shared.marks.remove(self.shared.zone, fnum) {
                    error!(sync_log::stream(), e);
                }
                self.doubt.retain(|(f, _)| *f != fnum);
                self.notify(fnum);
                // A sealed file held from collection only for its mark entry is looked at again
                // now, through the writer, which relays to the file's bot (R3 liveness).
                let _ = self.shared.writer.send(OzoneMsg::Durable(self.shared.zone, fnum));
            }
        }
        if let Err(e) = self.shared.marks.open(self.shared.zone, live.fnum, live.sizes, self.shared.bot) {
            error!(sync_log::stream(), e);
        }
        self.doubt.push((live.fnum, live.paths.clone()));
        self.taken = live.sizes;
        self.pair = Some(live);
        // The pair taken after the stop is in doubt from its first byte, and the cut set raised
        // at the stop could not name it, so it joins the record and is cut back to the sizes it
        // was opened with (QA F3).
        if let Some(e) = self.poisoned.clone() {
            if let Some(live) = &self.pair {
                let cut = Cut {
                    zone:       self.shared.zone,
                    fnum:       live.fnum,
                    data:       live.paths.0.clone(),
                    data_upto:  live.sizes.0,
                    index:      live.paths.1.clone(),
                    index_upto: live.sizes.1,
                };
                self.shared.fault.raise(&self.shared.root, FaultCause::Barrier, vec![cut.clone()], &e);
                if let Err(e) = self.shared.writer.send(OzoneMsg::Cut(vec![cut])) {
                    error!(sync_log::stream(), err!(e,
                        "{}: The writer could not be told to cut the file it rolled over to after \
                        a failed barrier; the next open cuts it.", self.label;
                        Channel, Write));
                }
            }
        }
    }

    // Tells every cache bot released to since the last barrier that `fnum`'s mark has moved.
    fn notify(&mut self, fnum: FileNum) {
        for (_, cbot) in mem::take(&mut self.told) {
            let _ = cbot.send(OzoneMsg::Durable(self.shared.zone, fnum));
        }
    }

    /// Forces the current pair to stable storage, publishes the mark it reached, and only then
    /// tells the cache bots.  A failure poisons the syncer and raises the store's fault.
    fn barrier(&mut self) -> Outcome<()> {
        if let Some(e) = &self.poisoned {
            return Err(e.clone());
        }
        let covered = self.taken; // only what was taken in before the sync began
        if let Err(e) = self.sync_pair() {
            self.poison(&e);
            return Err(e);
        }
        hooks::halt(hooks::Halt::Synced);
        self.dirty = false;
        self.since = 0;
        self.last = Some(Instant::now());
        self.owed = None;
        if let Some(fnum) = self.pair.as_ref().map(|p| p.fnum) {
            if let Err(e) = self.shared.marks.set(self.shared.zone, fnum, covered) {
                error!(sync_log::stream(), e);
            }
            self.notify(fnum);
        }
        Ok(())
    }

    /// No barrier counts from here on.  The store's fault names each file of this writer still
    /// in doubt with its mark, and the writer is told to cut them.
    fn poison(&mut self, e: &Error<ErrTag>) {
        self.poisoned = Some(err!(e.clone(),
            "{}: A durability barrier failed, so the store has stopped writing.  Restart the \
            store; it recovers from the disk.", self.label;
            Write, Fatal));
        let mut cuts = Vec::new();
        for (fnum, (data, index)) in &self.doubt {
            let mark = match self.shared.marks.get(self.shared.zone, *fnum) {
                Ok(Some(mark))  => mark,
                Ok(None)        => continue,
                Err(e)          => {
                    error!(sync_log::stream(), e);
                    continue;
                },
            };
            cuts.push(Cut {
                zone:       self.shared.zone,
                fnum:       *fnum,
                data:       data.clone(),
                data_upto:  mark.0,
                index:      index.clone(),
                index_upto: mark.1,
            });
        }
        error!(sync_log::stream(), err!(e.clone(),
            "{}: A durability barrier failed.  The store stops writing, and the files in doubt \
            are cut back to their last durable marks: {:?}", self.label, cuts;
            IO, File, Write, Fatal));
        self.shared.fault.raise(&self.shared.root, FaultCause::Barrier, cuts.clone(), e);
        // Every cache bot holding a chunk's answer behind a mark that will not move now hears
        // of the fault, rather than its store waiting out the durability timeout.
        if let Some(fnum) = self.pair.as_ref().map(|p| p.fnum) {
            self.notify(fnum);
        }
        if let Err(e) = self.shared.writer.send(OzoneMsg::Cut(cuts)) {
            error!(sync_log::stream(), err!(e,
                "{}: The writer could not be told to cut its files after a failed barrier; the \
                next open cuts them.", self.label;
                Channel, Write));
        }
    }

    fn sync_pair(&self) -> Outcome<()> {
        if let Some(LiveFiles { fnum, dat, ind, .. }) = &self.pair {
            hooks::barrier_delay();
            if let Err(e) = Self::sync(*fnum, dat) {
                return Err(err!(e,
                    "{}: sync_data on live data file {} failed, so records written to it are not \
                    confirmed durable.", self.label, fnum;
                    IO, File, Write));
            }
            if let Err(e) = Self::sync(*fnum, ind) {
                return Err(err!(e,
                    "{}: sync_data on live index file {} failed, so records written to it are \
                    not confirmed durable.", self.label, fnum;
                    IO, File, Write));
            }
        }
        Ok(())
    }

    /// `File::sync_data`, or the failure a test has asked for (`test::hooks`).  A barrier stops at
    /// its first failed sync, so a barrier the hook fails is counted once.
    fn sync(fnum: FileNum, file: &File) -> std::io::Result<()> {
        if hooks::sync_fails(fnum) {
            return Err(std::io::Error::other(
                "the disk failed the sync (test::hooks::set_barrier_failure)"));
        }
        file.sync_data()
    }

    /// A barrier nobody is waiting on, whose failure can only be logged.
    fn barrier_or_log(&mut self) {
        if let Err(e) = self.barrier() {
            error!(sync_log::stream(), e);
        }
    }

    /// How long until the interval policy or a held discard owes a barrier, while one is owed.
    /// A poisoned syncer owes nothing: no barrier of its can prove anything again.
    fn owed_in(&self) -> Option<Duration> {
        if self.poisoned.is_some() || !self.dirty {
            return None;
        }
        let period = match (self.policy, self.last) {
            (SyncPolicy::Interval(p), Some(t))  => Some(p.saturating_sub(t.elapsed())),
            (SyncPolicy::Interval(_), None)     => Some(Duration::ZERO),
            _                                   => None,
        };
        let held = self.owed.map(|t| t.saturating_duration_since(Instant::now()));
        match (period, held) {
            (Some(p), Some(h))  => Some(p.min(h)),
            (p, h)              => p.or(h),
        }
    }

    /// A close announced by `Finish` barriers under every policy, so that the discards held on
    /// the last records are let go; `Never` governs the write path, not the close.  A writer gone
    /// without a word keeps the old rule.
    fn end(&mut self, announced: bool) {
        if self.dirty && (announced || self.policy != SyncPolicy::Never) {
            self.barrier_or_log();
        }
    }

    fn tell(resp: &Responder<UIDL, UID, ENC, KH>, e: Error<ErrTag>) {
        // A caller that has already given up has dropped its end, and there is no one else to
        // tell.  A responder with no channel is an internal write nobody waits on.
        if resp.is_some() {
            let _ = resp.send(OzoneMsg::Error(Self::written(e)));
        }
    }

    /// What a written record's caller is told went wrong after the write: tagged `Unconfirmed`,
    /// so that it can be told from a write that never landed without reading the words.
    fn written(e: Error<ErrTag>) -> Error<ErrTag> {
        err!(e, "The record is written, but not confirmed durable."; Write, Unconfirmed)
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
>
    Drop for Barrier<UIDL, UID, ENC, KH>
{
    fn drop(&mut self) {
        // However the thread ends, a panic included, and before the receiver is dropped: a writer
        // never finds the channel gone while the flag still says the syncer is running.
        self.running.store(false, Ordering::SeqCst);
    }
}
