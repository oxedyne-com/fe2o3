//! A store in progress.  A chunked value is stored in two steps, its chunks and then its bunch
//! key, and the second waits on the first being written and readable: a disk's durability
//! round.  The server bot that began the store must not sit through it, or the bots of a
//! database hold one chunked put each and every other put queues behind them (A3 round 2a,
//! 2026-10-09: twelve concurrent chunked puts timed out against two server bots).  The store is
//! therefore held here as a state that is advanced by one answer at a time, and the bot goes on
//! with other requests between them.  A caller that has nothing else to do runs it to the end
//! with `run`, which is how `OzoneApi::store_dat_using_responder` waits.
//!
//! Since 2026-10-09 (B1, B2) every chunked write has a chunk set of its own.  Once its bunch key
//! is durable the store reads the key back: if its own value is there, the value it replaced is
//! retired; if a newer put or a delete won, it retires its own chunks instead.  Its set is held in
//! the database's pending sets until then, so the orphan sweep leaves it alone.

use crate::{
    prelude::*,
    base::{
        constant,
        index::ZoneInd,
    },
    comm::{
        channels::PendingSets,
        msg::OzoneMsg,
        response::{
            AckWait,
            Responder,
        },
    },
};

use oxedyne_fe2o3_core::channels::{
    self,
    Receiver,
    Recv,
};
use oxedyne_fe2o3_data::time::Timestamp;
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
    Chunks, // every chunk written and readable
    Head,   // the bunch key, after which the key is read back to choose what to retire
    Retire, // the tombstones of the chunks retired
}

// Holds a chunk set in the database's pending sets for as long as it lives.
struct SetHold {
    sets:   PendingSets,
    id:     Option<u64>,
}

impl SetHold {
    fn new(sets: &PendingSets, id: Option<u64>) -> Outcome<Self> {
        if let Some(id) = id {
            let mut held = lock_mutex!(sets);
            held.insert(id);
        }
        Ok(Self { sets: sets.clone(), id })
    }

    fn release(&mut self) {
        if let Some(id) = self.id.take() {
            if let Ok(mut held) = self.sets.lock() {
                held.remove(&id);
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
    meta:       Meta<UIDL, UID>,
    schms2:     Option<RestSchemesOverride<ENC, KH>>,
    resp:       Responder<UIDL, UID, ENC, KH>, // the caller's
    nchunks:    usize,                         // records in all, as the caller is told
    head:       Option<(OzoneMsg<UIDL, UID, ENC, KH>, ZoneInd)>,
    chunks:     Msgs<UIDL, UID, ENC, KH>,
    step:       Step,
    own:        Responder<UIDL, UID, ENC, KH>, // answers to the step in hand
    wait:       AckWait<UIDL, UID, ENC, KH>,
    held:       Vec<OzoneMsg<UIDL, UID, ENC, KH>>, // the bunch key's answers, told after the retire
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
    /// `datkeys`, the bunch key first.  Nothing is sent until `start`, but the new value's chunk
    /// set is held from here.
    pub fn new<PR: Hasher, CS: Checksummer>(
        api:        &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
        k:          Dat,
        pred:       Option<(PartKey, Meta<UIDL, UID>)>,
        datkeys:    &[Dat],
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
        let hold = res!(SetHold::new(api.chans().pending_sets(), set.as_ref().map(|p| p.set_id())));
        Ok(Self {
            k,
            pred,
            set,
            hold,
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
        AckWait::new(n, constant::USER_REQUEST_TIMEOUT, constant::DURABILITY_TIMEOUT)
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

    /// Counts the records to the caller and sends the chunks, each answering to the store's own
    /// responder; the caller hears of them when all are in.  A value without chunks goes straight
    /// to its bunch key.  Returns whether the store is already complete.
    pub fn start<PR: Hasher, CS: Checksummer>(
        &mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<bool>
    {
        if self.resp.is_some() {
            res!(self.resp.send(OzoneMsg::Chunks(self.nchunks)));
        }
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
            if let OzoneMsg::Write { resp, .. } = msg {
                *resp = self.own.clone();
            }
        }
        self.wait = Self::wait_for(chunks.len());
        res!(api.store_bytes(chunks));
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
                self.retire(api)
            },
            Step::Retire => self.answer(),
        }
    }

    // The bunch key goes out once every chunk of the value is written and readable, so that a
    // reader finds the value it replaces, or none, until the value that names its chunks has all
    // of them.  A chunked value's key answers to the store, which passes its final answer on, and
    // over a value of other geometry retires that value's chunks first; a value without chunks
    // and without a predecessor to retire is sent with the caller's own responder.
    fn publish<PR: Hasher, CS: Checksummer>(
        &mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<bool>
    {
        let mut head = match self.head.take() {
            Some(head) => head,
            None => return Err(err!("A store in progress has no bunch key left to send."; Bug, Missing)),
        };
        if self.pred.is_none() && self.set.is_none() && !self.promised {
            res!(api.store_bytes(vec![head]));
            return Ok(true);
        }
        self.own = api.responder();
        if let (OzoneMsg::Write { resp, .. }, _) = &mut head {
            *resp = self.own.clone();
        }
        self.step = Step::Head;
        self.wait = Self::wait_for(1);
        res!(api.store_bytes(vec![head]));
        Ok(false)
    }

    // The bunch key is durable.  The key is read back, and every chunk set it does not name of
    // the two this store knows, the value it replaced and its own, is tombstoned, together under
    // a responder of their own and awaited as one step.  A set the key does not name is never
    // named again, since no write reuses a set.  A read that fails retires nothing.
    fn retire<PR: Hasher, CS: Checksummer>(
        &mut self,
        api: &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Outcome<bool>
    {
        if self.pred.is_none() && self.set.is_none() {
            return self.answer();
        }
        let cur = match api.chunk_set_of(&self.k, self.schms2.as_ref()) {
            Ok(cur) => cur.map(|(pkey, _)| pkey.set_id()),
            Err(e) => {
                self.not_retired(api, e);
                return self.answer();
            },
        };
        let mut retiring = Vec::new();
        if let Some((pkey, pmeta)) = &self.pred {
            if cur != Some(pkey.set_id()) {
                // Outranks the predecessor's records even after the clock has stepped back.
                let mut tmeta = self.meta.clone();
                tmeta.time = Self::later(&self.meta.time, &Self::just_after(&pmeta.time));
                retiring.push((pkey.clone(), tmeta));
            }
        }
        if let Some(own) = &self.set {
            if cur != Some(own.set_id()) {
                // A newer put or a delete won: these chunks are named by nothing.
                let mut tmeta = self.meta.clone();
                tmeta.time = Self::later(&res!(Timestamp::now()), &Self::just_after(&self.meta.time));
                retiring.push((own.clone(), tmeta));
            }
        }
        self.own = api.responder();
        let mut n = 0;
        for (pkey, tmeta) in &retiring {
            match api.send_retires(pkey, tmeta, self.schms2.as_ref(), Some(&self.own)) {
                Ok(waits) => n += waits.len(),
                Err(e) => {
                    self.not_retired(api, e);
                    break;
                },
            }
        }
        if n == 0 {
            return self.answer();
        }
        self.step = Step::Retire;
        self.wait = Self::wait_for(n);
        Ok(false)
    }

    fn just_after(t: &Timestamp) -> Timestamp {
        let d = **t + Duration::from_nanos(1);
        Timestamp::new(d.as_secs(), d.subsec_nanos())
    }

    fn later(a: &Timestamp, b: &Timestamp) -> Timestamp {
        if a >= b { a.clone() } else { b.clone() }
    }

    // The new value is stored and durable; the old chunks are left to the orphan sweep, and the
    // caller is not told its write failed.
    fn not_retired<PR: Hasher, CS: Checksummer>(
        &self,
        api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
        e:      Error<ErrTag>,
    ) {
        warn!(sync_log::stream(),
            "{}: The value stored at {:?} is durable, but the chunks it was to retire, of the \
            value it replaced or its own after a newer write won, were not all retired, and wait \
            for the orphan sweep: {}",
            api.ozid(), self.k, e);
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
    // written; over the bunch key it is the writer's error, as it stands; over the retire it is
    // nothing the caller need hear.
    fn fail<PR: Hasher, CS: Checksummer>(
        &mut self,
        api:    &OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
        e:      Error<ErrTag>,
    )
        -> Outcome<bool>
    {
        self.hold.release();
        let e = match self.step {
            Step::Retire => {
                self.not_retired(api, e);
                return self.answer();
            },
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
