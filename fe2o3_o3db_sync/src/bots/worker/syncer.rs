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

use crate::{
    prelude::*,
    base::cfg::OzoneConfig,
    comm::{
        msg::OzoneMsg,
        response::Responder,
    },
    test::hooks,
};

use oxedyne_fe2o3_core::channels::Simplex;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    collections::VecDeque,
    fs::File,
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


/// When a record has to be durable before it is released.
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
/// files every later record is appended to; the pair it replaces is sealed, made durable before
/// any record written to the new one is released.  A `Record` is one appended record with the
/// insert that releases it to its cache bot.
pub enum Handed<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    Pair(File, File),   // data, index
    Record {
        cbot:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
        insert: OzoneMsg<UIDL, UID, ENC, KH>,
        resp:   Responder<UIDL, UID, ENC, KH>,
        policy: SyncPolicy,
    },
    Finish,
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
            Barrier::new(label, rx, cleared).run();
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

    /// Releases everything handed over, with a last barrier where the policy owes one, and ends
    /// the thread.
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
    pair:       Option<(File, File)>,
    policy:     SyncPolicy,         // the latest record's
    dirty:      bool,               // the pair holds records no barrier has covered
    since:      u32,                // records released since the last barrier
    last:       Option<Instant>,    // when the last barrier completed
    failed:     Option<Instant>,    // when the last barrier failed, until one completes
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
    )
        -> Self
    {
        Self {
            label,
            rx,
            queue:      VecDeque::new(),
            pair:       None,
            policy:     SyncPolicy::Never,
            dirty:      false,
            since:      0,
            last:       None,
            failed:     None,
            running,
        }
    }

    fn run(mut self) {
        loop {
            if self.queue.is_empty() {
                // The interval policy owes the disk a barrier on its period whether or not another
                // record arrives, so the writes that end a burst are made durable too.
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
                    None => return self.end(),
                }
            }
            // Take whatever else is waiting, so that one barrier can cover all of it.
            while let Ok(item) = self.rx.try_recv() {
                self.queue.push_back(item);
            }
            while let Some(item) = self.queue.pop_front() {
                match item {
                    Handed::Pair(dat, ind) => {
                        // Sealing is unconditional, whatever the policy: a file that is no
                        // longer live is durable before anything written after it is released.
                        if self.dirty {
                            self.barrier_or_log();
                        }
                        self.pair = Some((dat, ind));
                    },
                    Handed::Record { cbot, insert, resp, policy } =>
                        self.records(cbot, insert, resp, policy),
                    Handed::Finish => return self.end(),
                }
            }
            // A syncer stopping as one that panicked would, with no last barrier.
            if hooks::syncer_stops() {
                return;
            }
        }
    }

    /// Releases one record, and with it the run queued behind it where every record waits on a
    /// barrier of its own: one barrier covers every record appended before it begins.  That is
    /// always so under `sync_on_write`, and under the interval and every-n policies while the last
    /// barrier has failed.  Batched only under the first, a disk failing its syncs slowly took one
    /// barrier per record under the others, one after another, and with writes arriving faster
    /// than it failed the queue grew without bound (2026-09-23, every-n 2026-09-24).
    fn records(
        &mut self,
        cbot:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
        insert: OzoneMsg<UIDL, UID, ENC, KH>,
        resp:   Responder<UIDL, UID, ENC, KH>,
        policy: SyncPolicy,
    ) {
        let mut run = vec![(cbot, insert, resp)];
        let alone = match policy {
            SyncPolicy::EveryWrite                          => true,
            SyncPolicy::EveryN(_) | SyncPolicy::Interval(_) => self.failed.is_some(),
            SyncPolicy::Never                               => false,
        };
        if alone {
            while let Some(Handed::Record { policy: next, .. }) = self.queue.front() {
                if *next != policy {
                    break;
                }
                if let Some(Handed::Record { cbot, insert, resp, .. }) = self.queue.pop_front() {
                    run.push((cbot, insert, resp));
                }
            }
        }
        self.policy = policy;
        self.dirty = true;
        self.since = self.since.saturating_add(run.len() as u32);
        // While the disk is failing a record waits on a barrier of its own, so that its caller
        // hears so rather than being answered as if its period will make it durable.
        let due = match policy {
            SyncPolicy::EveryWrite  => true,
            SyncPolicy::EveryN(n)   => self.since >= n,
            SyncPolicy::Interval(p) => self.failed.is_some() || match self.last {
                None        => true,
                Some(last)  => last.elapsed() >= p,
            },
            SyncPolicy::Never       => false,
        };
        let synced = if due { self.barrier() } else { Ok(()) };
        for (cbot, insert, resp) in run {
            // The caller hears when the barrier failed, from the cache bot once the record is
            // readable, as it hears of one that did not: told from here first, it could read its
            // write back and not find it.  The record is released all the same: it is in the
            // files whatever the barrier said, and holding it back would leave the cache
            // disagreeing with them and its bytes accounted to no one, where no collection could
            // ever reclaim them.
            let insert = match (&synced, insert) {
                (Ok(()), insert) => insert,
                (Err(e), OzoneMsg::Insert(k, v, c, f, i, m, r, _)) =>
                    OzoneMsg::Insert(k, v, c, f, i, m, r, Some(Self::written(e.clone()))),
                // Not an insert, so nothing the cache bot would answer with: told from here.
                (Err(e), other) => {
                    Self::tell(&resp, e.clone());
                    other
                },
            };
            if let Err(e) = cbot.send(insert) {
                let e = err!(e,
                    "{}: A written record could not be released to its cache bot.", self.label;
                    Channel, Write);
                error!(sync_log::stream(), e.clone());
                Self::tell(&resp, e);
            }
        }
    }

    /// Forces the current pair to stable storage.
    fn barrier(&mut self) -> Outcome<()> {
        if let Err(e) = self.sync_pair() {
            self.failed = Some(Instant::now());
            return Err(e);
        }
        self.dirty = false;
        self.since = 0;
        self.last = Some(Instant::now());
        self.failed = None;
        Ok(())
    }

    fn sync_pair(&self) -> Outcome<()> {
        if let Some((dat, ind)) = &self.pair {
            hooks::barrier_delay();
            if let Err(e) = Self::sync(dat) {
                return Err(err!(e,
                    "{}: sync_data on the live data file failed, so records written to it are not \
                    confirmed durable.", self.label;
                    IO, File, Write));
            }
            if let Err(e) = Self::sync(ind) {
                return Err(err!(e,
                    "{}: sync_data on the live index file failed, so records written to it are \
                    not confirmed durable.", self.label;
                    IO, File, Write));
            }
        }
        Ok(())
    }

    /// `File::sync_data`, or the failure a test has asked for (`test::hooks`).  A barrier stops at
    /// its first failed sync, so a barrier the hook fails is counted once.
    fn sync(file: &File) -> std::io::Result<()> {
        if hooks::sync_fails() {
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

    /// How long until the interval policy owes a barrier, while it owes one.  A failed barrier is
    /// retried a period after it failed: counted from the last one that completed, a disk refusing
    /// every sync had this thread retry it as fast as the disk could refuse, a million times in
    /// three seconds with an error logged for each (2026-09-23).
    fn owed_in(&self) -> Option<Duration> {
        match (self.dirty, self.policy, self.failed.or(self.last)) {
            (true, SyncPolicy::Interval(p), Some(t))    => Some(p.saturating_sub(t.elapsed())),
            (true, SyncPolicy::Interval(_), None)       => Some(Duration::ZERO),
            _                                           => None,
        }
    }

    fn end(&mut self) {
        if self.dirty && self.policy != SyncPolicy::Never {
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
