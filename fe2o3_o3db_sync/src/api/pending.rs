//! A store in progress.  A chunked value is stored in two steps, its chunks and then its bunch
//! key, and the second waits on the first being written, readable and durable.  The server bot
//! that began the store must not sit through it, or the bots of a database hold one chunked put
//! each and every other put queues behind them (A3 round 2a, 2026-10-09: twelve concurrent
//! chunked puts timed out against two server bots).  The store is therefore held here as a state
//! that is advanced by one answer at a time, and the bot goes on with other requests between
//! them.  A caller that has nothing else to do runs it to the end with `run`, which is how
//! `OzoneApi::store_dat_using_responder` waits.
//!
//! Since 2026-10-09 (B1, B2) every chunked write has a chunk set of its own, held in the
//! database's pending sets from the moment its id is drawn until its head is in the cache, so
//! the orphan sweep leaves it alone.  Since 2026-10-10 (durable mark, R1) the head goes out only
//! once a barrier covers every chunk, which may sit in another writer's files: each writer that
//! took a chunk is asked for a barrier at once, so the wait is one fsync.  A head is never
//! durable before its chunks.  The cache bot that enters the head retires the set it displaced,
//! or the store's own when a newer record was already there, once the winner is durable; the
//! store does not wait for it.  A crash before then leaves orphans for the sweep, never a loss.
//!
//! A delete is a store with no chunks (fix A, 2026-10-09).  Its tombstone is the head, and
//! retires the set of the head it displaces.

use crate::{
    prelude::*,
    base::{
        constant,
        index::ZoneInd,
    },
    comm::{
        channels::PendingSets,
        msg::{
            HeadTicket,
            OzoneMsg,
            StoreRole,
        },
        response::{
            AckWait,
            Responder,
        },
    },
    test::hooks,
};

use oxedyne_fe2o3_core::channels::{
    self,
    Receiver,
    Recv,
};
use oxedyne_fe2o3_iop_db::api::{
    Meta,
    RestSchemesOverride,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    chunk::PartKey,
    id::NumIdDat,
};

use std::{
    mem,
    time::Duration,
};

// Which answers the store is waiting for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Step {
    Chunks, // every chunk written, readable and durable
    Head,   // the bunch key
}

// Holds a chunk set in the database's pending sets for as long as it lives, or until it is handed
// over with the head.
pub(crate) struct SetHold {
    sets:   PendingSets,
    id:     Option<u64>,
}

impl SetHold {
    pub(crate) fn new(sets: &PendingSets, id: Option<u64>) -> Outcome<Self> {
        if let Some(id) = id {
            let mut held = lock_mutex!(sets);
            held.hold(id);
        }
        Ok(Self { sets: sets.clone(), id })
    }

    // The id, for the head to carry; whoever answers the head releases it.
    fn hand_over(&mut self) -> Option<u64> { self.id.take() }

    fn release(&mut self) {
        if let Some(id) = self.id.take() {
            if let Ok(mut held) = self.sets.lock() {
                held.release(&id);
            }
        }
    }
}

impl Drop for SetHold {
    fn drop(&mut self) { self.release(); }
}

type Msgs<const UIDL: usize, UID, ENC, KH> = Vec<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>;

pub struct PendingStore<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    k:          Dat,
    pred:       Option<(PartKey, Meta<UIDL, UID>)>, // the chunked value replaced, with its bunch key's metadata
    set:        Option<PartKey>,                    // the new value's own chunks
    hold:       SetHold,
    ticketed:   bool,                               // is the head answered with what it displaced?
    meta:       Meta<UIDL, UID>,
    schms2:     Option<RestSchemesOverride<ENC, KH>>,
    resp:       Responder<UIDL, UID, ENC, KH>, // the caller's
    nchunks:    usize,                         // records in all, as the caller is told
    head:       Option<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>,
    chunks:     Msgs<UIDL, UID, ENC, KH>,
    step:       Step,
    own:        Responder<UIDL, UID, ENC, KH>, // answers to the step in hand
    wait:       AckWait<UIDL, UID, ENC, KH>,
    held:       Vec<OzoneMsg<UIDL, UID, ENC, KH>>, // the bunch key's answers, told last
    promised:   bool, // has the caller been told the bunch key is written?
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    PendingStore<UIDL, UID, ENC, KH>
{
    /// Holds the prepared write `head` and `chunks`, whose keys beyond the main key are
    /// `datkeys`, the bunch key first, and `hold` on the new value's chunk set.  Nothing is sent
    /// until `start`.  An unticketed store, the forcing path's, retires nothing.
    pub(crate) fn new<PR: Hasher, CS: Checksummer>(
        api:        &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
        k:          Dat,
        pred:       Option<(PartKey, Meta<UIDL, UID>)>,
        datkeys:    &[Dat],
        hold:       SetHold,
        ticketed:   bool,
        meta:       Meta<UIDL, UID>,
        schms2:     Option<&RestSchemesOverride<ENC, KH>>,
        resp:       Responder<UIDL, UID, ENC, KH>,
        head:       (OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd),
        chunks:     Msgs<UIDL, UID, ENC, KH>,
    )
        -> Outcome<Self>
    {
        let set = match datkeys.first() {
            Some(Dat::Tup5u64(tup)) => Some(PartKey(*tup)),
            Some(other) => return Err(err!(
                "The bunch key of a prepared write of {:?} is {:?}, not a part key.", k, other;
                Bug, Unexpected)),
            None => None,
        };
        if ticketed && hold.id != set.as_ref().map(|p| p.set_id()) {
            return Err(err!(
                "The chunk set held for a prepared write of {:?} is {:?}, but its bunch key \
                names {:?}.", k, hold.id, set;
                Bug, Mismatch));
        }
        Ok(Self {
            k,
            pred,
            set,
            hold,
            ticketed,
            meta,
            schms2:     schms2.cloned(),
            resp,
            nchunks:    chunks.len() + 1,
            head:       Some(head),
            chunks,
            step:       Step::Chunks,
            own:        api.responder(),
            wait:       Self::wait_for(0),
            held:       Vec::new(),
            promised:   false,
        })
    }

    fn wait_for(n: usize) -> AckWait<UIDL, UID, ENC, KH> {
        let durability = match hooks::durability_timeout() {
            Some(d) => d,
            None    => constant::DURABILITY_TIMEOUT,
        };
        AckWait::new(n, constant::USER_REQUEST_TIMEOUT, durability)
    }

    pub fn nchunks(&self) -> usize { self.nchunks }

    /// The channel the answers to the step in hand arrive on.
    pub fn rx(&self) -> Outcome<&Receiver<OzoneMsg<UIDL, UID, ENC, KH>>> {
        match self.own.channel() {
            Some(chan) => Ok(chan.rx()),
            None => Err(err!("The responder of a store in progress has no channel."; Bug, Missing)),
        }
    }

    /// The time left before the silence that fails the step in hand.
    pub fn left(&self) -> Duration { self.wait.left() }

    /// Sends the chunks, each answering to the store's own responder; the caller hears of them
    /// when all are in.  A value without chunks goes straight to its bunch key.  A store tells its
    /// caller the count of records before this, and a delete, whose caller expects one record's
    /// answers, does not.  Returns whether the store is already complete.
    pub fn start<PR: Hasher, CS: Checksummer>(
        &mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<bool>
    {
        let mut chunks = mem::take(&mut self.chunks);
        if chunks.is_empty() {
            return self.publish(api);
        }
        // The bunch key is not sent until the chunks are readable, which waits on the disk, and
        // a caller counts the silence before the last record is written against a short
        // deadline.  The store answers for the bunch key's liveness itself, as its own wait for
        // it is held to the same deadline, and the caller waits on the disk as it would have.
        if self.resp.is_some() {
            res!(self.resp.send(OzoneMsg::Written));
            self.promised = true;
        }
        for (msg, _) in chunks.iter_mut() {
            if let OzoneMsg::Write { resp, role, .. } = msg {
                *resp = self.own.clone();
                *role = Some(StoreRole::Chunk);
            }
        }
        self.wait = Self::wait_for(chunks.len());
        let took = res!(api.store_bytes(chunks));
        res!(api.nudge(&took));
        Ok(false)
    }

    /// Takes in one answer to the step in hand.  Returns whether the store is complete, in which
    /// case the caller has been told everything it is to be told.  An error comes back only when
    /// the caller has no responder to be told it on.
    pub fn advance<PR: Hasher, CS: Checksummer>(
        &mut self,
        api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
        msg:    OzoneMsg<UIDL, UID, ENC, KH>,
    )
        -> Outcome<bool>
    {
        // A failure to send the next step, the bunch key among them, is the caller's to hear:
        // left to the bot's log, a caller already told `Written` waited out the durability
        // deadline and was then told its records were in the files (A3 QA m1, 2026-10-09).
        match self.take_in(api, msg) {
            Err(e)  => self.fail(api, e),
            done    => done,
        }
    }

    fn take_in<PR: Hasher, CS: Checksummer>(
        &mut self,
        api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
        msg:    OzoneMsg<UIDL, UID, ENC, KH>,
    )
        -> Outcome<bool>
    {
        // The chunks' answers are passed to the caller as they come, and not when the last is
        // in: its deadlines measure silence, and a chunk that is written but not yet readable
        // is not silence.  The bunch key's follow from its own writer.
        if self.step == Step::Head && !self.promised && self.resp.is_some() {
            // A value without chunks over one that has them: its record is written, which is all
            // the caller's short deadline asks, and what follows waits on the disk.
            if matches!(&msg, OzoneMsg::Written) {
                res!(self.resp.send(OzoneMsg::Written));
                self.promised = true;
            }
        }
        if self.step == Step::Chunks && self.resp.is_some() {
            match &msg {
                OzoneMsg::Written               => res!(self.resp.send(OzoneMsg::Written)),
                OzoneMsg::KeyExists(b)          => res!(self.resp.send(OzoneMsg::KeyExists(*b))),
                OzoneMsg::KeyChunkExists(b, i)  => res!(self.resp.send(OzoneMsg::KeyChunkExists(*b, *i))),
                _ => (),
            }
        }
        match self.wait.feed(msg) {
            Err(e)      => self.fail(api, e),
            Ok(false)   => Ok(false),
            Ok(true)    => self.complete(api),
        }
    }

    /// The silence the step in hand is allowed has run out.
    pub fn expire<PR: Hasher, CS: Checksummer>(
        &mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<bool>
    {
        let e = self.wait.expired();
        self.fail(api, e)
    }

    /// Waits on the store's answers, one step after another, until it is complete.
    pub fn run<PR: Hasher, CS: Checksummer>(
        mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<()>
    {
        loop {
            let left = self.left();
            if left.is_zero() {
                if res!(self.expire(api)) { return Ok(()); }
                continue;
            }
            let got = match self.own.channel() {
                Some(chan) => chan.recv_timeout(left),
                None => return Err(err!(
                    "The responder of a store in progress has no channel."; Bug, Missing)),
            };
            match got {
                Recv::Empty => (), // Out of time, which the next pass reports.
                Recv::Result(Err(e)) => return Err(err!(e,
                    "{}: Waiting on the answers to a store of {:?}.", api.ozid(), self.k;
                    Channel, Read)),
                Recv::Result(Ok(msg)) => if res!(self.advance(api, msg)) { return Ok(()); },
            }
        }
    }

    // The step in hand has all its answers.
    fn complete<PR: Hasher, CS: Checksummer>(
        &mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<bool>
    {
        let acks = mem::replace(&mut self.wait, Self::wait_for(0)).into_acks();
        match self.step {
            Step::Chunks => self.publish(api),
            Step::Head => {
                self.held = acks;
                self.answer()
            },
        }
    }

    // The bunch key goes out once every chunk of the value is written, readable and durable, so
    // that a reader finds the value it replaces, or none, until the value that names its chunks
    // has all of them, and no restart finds the head without them.  The head carries the set's
    // hold, handed over once it is sent, and what the cache bot retires by; until then a failure
    // here leaves the hold with the store, which releases it.
    fn publish<PR: Hasher, CS: Checksummer>(
        &mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<bool>
    {
        if self.set.is_some() {
            hooks::head_hold();
        }
        let mut head = match self.head.take() {
            Some(head) => head,
            None => return Err(err!("A store in progress has no bunch key left to send."; Bug, Missing)),
        };
        self.own = api.responder();
        if let (OzoneMsg::Write { resp, role: ticket, .. }, _) = &mut head {
            *resp = self.own.clone();
            if self.ticketed {
                *ticket = Some(StoreRole::Head(HeadTicket {
                    set:    self.hold.id,
                    own:    self.set.clone(),
                    pred:   self.pred.take(),
                    schms2: self.schms2.clone(),
                    tmeta:  self.meta.clone(),
                }));
            }
        }
        self.step = Step::Head;
        self.wait = Self::wait_for(1);
        res!(api.store_bytes(vec![head]));
        if self.ticketed {
            self.hold.hand_over();
        }
        Ok(false)
    }

    // Tells the caller the bunch key is stored.
    fn answer(&mut self) -> Outcome<bool> {
        self.hold.release();
        if self.resp.is_some() {
            if !self.promised {
                res!(self.resp.send(OzoneMsg::Written));
            }
            for ack in mem::take(&mut self.held) {
                res!(self.resp.send(ack));
            }
        }
        Ok(true)
    }

    // A step has failed.  Over the chunks that is the end of the store, with no bunch key
    // written; over the bunch key it is the writer's error, as it stands.
    fn fail<PR: Hasher, CS: Checksummer>(
        &mut self,
        api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
        e:      Error<ErrTag>,
    )
        -> Outcome<bool>
    {
        self.hold.release();
        let e = match self.step {
            Step::Head => e,
            // A new error, not a wrapper: the tags of a chain are all reported, and the
            // `Unconfirmed` of a chunk that was written but not confirmed would say a write that
            // publishes nothing may yet land.
            Step::Chunks => if e.tags().contains(&ErrTag::Timeout) {
                err!("{}: Storing a chunked value, its {} chunks were not all written and \
                    readable in time, so its bunch key was not written and nothing was \
                    stored.  Chunks that landed wait for the orphan sweep: {}",
                    api.ozid(), self.nchunks - 1, e;
                    Write, Timeout)
            } else {
                err!("{}: Storing a chunked value, a chunk failed, so its bunch key was not \
                    written and nothing was stored.  Chunks that landed wait for the orphan \
                    sweep: {}", api.ozid(), e;
                    Write)
            },
        };
        if self.resp.is_none() {
            return Err(e);
        }
        res!(self.resp.send(OzoneMsg::Error(e)));
        Ok(true)
    }
}

/// Waits on the channels of the stores in progress, and on `inbox`, for the next thing to do.
/// Returns the position of the store that was answered, with the answer, or `None` for a message
/// from `inbox`, or `None` altogether when the time ran out.
pub fn next_event<const UIDL: usize, UID, ENC, KH>(
    inbox:      &Receiver<OzoneMsg<UIDL, UID, ENC, KH>>,
    pending:    &[PendingStore<UIDL, UID, ENC, KH>],
)
    -> Outcome<Event<UIDL, UID, ENC, KH>>
where
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
{
    let mut rxs = vec![inbox];
    let mut timeout = None;
    for p in pending {
        rxs.push(res!(p.rx()));
        let left = p.left();
        timeout = Some(match timeout {
            Some(t) if t < left => t,
            _ => left,
        });
    }
    Ok(match res!(channels::recv_any(&rxs, timeout)) {
        Some((0, msg))  => Event::Inbox(msg),
        Some((i, msg))  => Event::Answer(i - 1, msg),
        None            => Event::Quiet,
    })
}

pub enum Event<const UIDL: usize, UID: NumIdDat<UIDL>, ENC: Encrypter, KH: Hasher> {
    Inbox(OzoneMsg<UIDL, UID, ENC, KH>),            // a request for the bot
    Answer(usize, OzoneMsg<UIDL, UID, ENC, KH>),    // to the store at this position
    Quiet,                                          // a deadline has passed, somewhere
}
