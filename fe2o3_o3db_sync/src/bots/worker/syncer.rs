//! A writer's durability barrier, run on a thread of its own.
//!
//! A writer bot appends a record and goes straight on to the next; its syncer makes each record
//! durable as the store's sync policy asks and only then releases it to its cache bot, which makes
//! it readable and gives the caller its final answer.  The barrier used to run inside the writer,
//! so one fsync held up by the rest of the machine's traffic -- eleven seconds was measured on
//! 2026-09-23 -- held every record queued behind it past the caller's six-second deadline, and each
//! was reported as failed although each went on to land.
//!
//! Records are released in the order they were written.  The unconditional barrier on sealing a
//! full live file stays with the writer, which holds the file.

use crate::{
    prelude::*,
    base::{
        cfg::OzoneConfig,
        constant,
    },
    comm::{
        msg::OzoneMsg,
        response::Responder,
    },
};

use oxedyne_fe2o3_core::channels::Simplex;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    collections::VecDeque,
    fs::File,
    sync::mpsc::{
        self,
        Receiver,
        RecvTimeoutError,
        Sender,
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

/// What a writer hands its syncer, in the order it wrote.
pub enum Handed<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    // The live data and index files every record after this one is appended to.
    Pair(File, File),
    // One appended record, and the insert that releases it to its cache bot.
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
    tx:     Sender<Handed<UIDL, UID, ENC, KH>>,
    handle: Option<JoinHandle<()>>,
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
        let builder = thread::Builder::new()
            .name(fmt!("{}.sync", label))
            .stack_size(constant::STACK_SIZE);
        let handle = res!(builder.spawn(move || {
            sync_log::set_stream(log_stream_id);
            Barrier::new(label, rx).run();
        }));
        Ok(Self {
            tx,
            handle: Some(handle),
        })
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
    since:      u32,                // records released since the last barrier
    unsynced:   bool,               // a record went out with no barrier behind it yet
    last:       Option<Instant>,    // when the last barrier completed
    period:     Option<Duration>,   // the interval policy's, while it is in force
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
        label:  String,
        rx:     Receiver<Handed<UIDL, UID, ENC, KH>>,
    )
        -> Self
    {
        Self {
            label,
            rx,
            queue:      VecDeque::new(),
            pair:       None,
            since:      0,
            unsynced:   false,
            last:       None,
            period:     None,
        }
    }

    fn run(mut self) {
        loop {
            if self.queue.is_empty() {
                // The interval policy owes the disk a barrier on its period whether or not another
                // record arrives: the writes that end a burst are made durable too.
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
                        // Whatever went out from the pair being left is made durable first.
                        if self.unsynced {
                            self.barrier_or_log();
                        }
                        self.pair = Some((dat, ind));
                    },
                    Handed::Record { cbot, insert, resp, policy } =>
                        self.records(cbot, insert, resp, policy),
                    Handed::Finish => return self.end(),
                }
            }
        }
    }

    /// Releases one record, and with it the run queued behind it where every record waits on a
    /// barrier of its own: one barrier covers every record appended before it begins.
    fn records(
        &mut self,
        cbot:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
        insert: OzoneMsg<UIDL, UID, ENC, KH>,
        resp:   Responder<UIDL, UID, ENC, KH>,
        policy: SyncPolicy,
    ) {
        let mut run = vec![(cbot, insert, resp)];
        if policy == SyncPolicy::EveryWrite {
            while let Some(Handed::Record { policy: SyncPolicy::EveryWrite, .. }) = self.queue.front() {
                if let Some(Handed::Record { cbot, insert, resp, .. }) = self.queue.pop_front() {
                    run.push((cbot, insert, resp));
                }
            }
        }
        self.since = self.since.saturating_add(run.len() as u32);
        let due = match policy {
            SyncPolicy::EveryWrite => {
                self.period = None;
                true
            },
            SyncPolicy::EveryN(n) => {
                self.period = None;
                self.since >= n
            },
            SyncPolicy::Interval(p) => {
                self.period = Some(p);
                match self.last {
                    None        => true,
                    Some(last)  => last.elapsed() >= p,
                }
            },
            SyncPolicy::Never => {
                self.period = None;
                false
            },
        };
        let synced = if due { self.barrier() } else { Ok(()) };
        for (cbot, insert, resp) in run {
            if let Err(e) = &synced {
                // The caller hears that the barrier failed.  The record is released all the same:
                // it is in the files whatever the barrier said, and holding it back would leave
                // the cache disagreeing with them and its bytes accounted to no one, where no
                // collection could ever reclaim them.
                Self::tell(&resp, e.clone());
            }
            if let Err(e) = cbot.send(insert) {
                let e = err!(e,
                    "{}: A written record could not be released to its cache bot.", self.label;
                    Channel, Write);
                error!(sync_log::stream(), e.clone());
                Self::tell(&resp, e);
            }
        }
        if !due && policy != SyncPolicy::Never {
            self.unsynced = true;
        }
    }

    /// Forces the current pair to stable storage.
    fn barrier(&mut self) -> Outcome<()> {
        if let Some((dat, ind)) = &self.pair {
            if let Err(e) = dat.sync_data() {
                return Err(err!(e,
                    "{}: sync_data on the live data file failed, so records written to it are not \
                    confirmed durable.", self.label;
                    IO, File, Write));
            }
            if let Err(e) = ind.sync_data() {
                return Err(err!(e,
                    "{}: sync_data on the live index file failed, so records written to it are \
                    not confirmed durable.", self.label;
                    IO, File, Write));
            }
        }
        self.since = 0;
        self.unsynced = false;
        self.last = Some(Instant::now());
        Ok(())
    }

    /// A barrier nobody is waiting on, whose failure can only be logged.
    fn barrier_or_log(&mut self) {
        if let Err(e) = self.barrier() {
            error!(sync_log::stream(), e);
        }
    }

    /// How long until the interval policy owes a barrier, while it owes one.
    fn owed_in(&self) -> Option<Duration> {
        match (self.unsynced, self.period, self.last) {
            (true, Some(p), Some(last))    => Some(p.saturating_sub(last.elapsed())),
            (true, Some(_), None)          => Some(Duration::ZERO),
            _                              => None,
        }
    }

    fn end(&mut self) {
        if self.unsynced {
            self.barrier_or_log();
        }
    }

    fn tell(resp: &Responder<UIDL, UID, ENC, KH>, e: Error<ErrTag>) {
        // A caller that has already given up has dropped its end, and there is no one else to
        // tell.  A responder with no channel is an internal write nobody waits on.
        if resp.is_some() {
            let _ = resp.send(OzoneMsg::Error(e));
        }
    }
}
