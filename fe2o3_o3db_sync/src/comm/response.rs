use crate::{
    prelude::*,
    base::id::{
        self,
        OzoneBotId,
        Ticket,
    },
    comm::msg::OzoneMsg,
    data::{
        core::Value,
    },
};

use oxedyne_fe2o3_core::{
    alt::Override,
    channels::{
        Recv,
        Simplex,
    },
};
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_iop_crypto::enc::EncrypterDefAlt;
use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
};

use std::{
    collections::HashSet,
    time::{
        Duration,
        Instant,
    },
};

#[derive(Clone, Debug)]
pub struct Responder<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    ozid:   Option<OzoneBotId>, // Source
    tik:    Ticket,
    chan:   Option<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    Responder<UIDL, UID, ENC, KH>
{
    pub fn new(ozid: Option<&OzoneBotId>) -> Self {
        Self {
            ozid:   ozid.map(|id| id.clone()),
            tik:    Ticket::new(),
            chan:   Some(Simplex::default()),
        }
    }

    pub fn make(
        ozid: Option<&OzoneBotId>,
        chan: Option<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>,
    )
        -> Self
    {
        Self {
            ozid:   ozid.map(|id| id.clone()),
            tik:    Ticket::new(),
            chan:   chan,
        }
    }

    pub fn none(ozid: Option<&OzoneBotId>) -> Self {
        Self {
            ozid:   ozid.map(|id| id.clone()),
            tik:    Ticket::new(),
            chan:   None,
        }
    }

    pub fn is_none(&self)   -> bool { self.chan.is_none() }
    pub fn is_some(&self)   -> bool { self.chan.is_some() }
    pub fn ozid(&self)      -> &Option<OzoneBotId> { &self.ozid }
    pub fn ticket(&self)    -> &Ticket { &self.tik }
    pub fn channel(&self)   -> Option<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> { self.chan.as_ref() }

    pub fn recv_block(&self) -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>> {
        match self.channel() {
            None => Err(err!("This responder does not have a channel."; Channel, Missing)),
            Some(ref simplex) => {
                match simplex.recv() {
                    Err(e) => return Err(err!(e,
                        "Could not read from responder channel";
                        Channel, Read)),
                    Ok(OzoneMsg::Error(e)) => return Err(e),
                    Ok(msg) => Ok(msg),
                }
            },
        }
    }

    pub fn send(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<()> {
        match self.channel() {
            Some(chan) => chan.send(msg),
            None => return Err(err!("This responder has no channel."; Channel, Missing)),
        }
    }

    /// A receiver waiting for a complete `Dat` wrapped byte vector.  Also returns whether
    /// garbage collection has just been performed on the read file, during which time it is
    /// possible the value may have been updated.  This method does not assemble a `Dat`
    /// wrapped byte vector from chunks, use `db::fetch_chunks` for that.
    pub fn recv_daticle(
        &self,
        enc:    &EncrypterDefAlt<EncryptionScheme, ENC>,
        or:     Option<&Override<EncryptionScheme, ENC>>,
    )
        -> Outcome<(Option<(Dat, Meta<UIDL, UID>)>, bool)>
    {
        match self.channel() {
            None => Err(err!("This responder does not have a channel."; Channel, Missing)),
            Some(ref simplex) => {
                match simplex.recv_timeout(constant::USER_REQUEST_TIMEOUT) {
                    Recv::Empty => Err(err!(
                        "Failed to receive a message via responder within {:.2} [s].",
                        constant::USER_REQUEST_TIMEOUT.as_secs_f32();
                        Missing, Data)),
                    Recv::Result(Err(e)) => return Err(err!(e,
                        "Could not read from responder channel";
                        Channel, Read)),
                    Recv::Result(Ok(msg)) => match msg {
                        OzoneMsg::Error(e) => return Err(e),
                        // A single record is decoded the same whether it was read under a Complete
                        // key or a chunk key: the chunk arm is reached when a chunk-data key is
                        // fetched directly -- which an orphan sweep does to read a chunk record's
                        // tombstone, and which also arises when a deleted chunked value's chunk key
                        // is scanned (its newest record is a deleted-kind marker and scans as a
                        // Tup5u64 main key).  Recognising the marker in both arms, not just the
                        // Complete one, is what lets such a key read back as absent rather than as
                        // an "unexpected" error.
                        OzoneMsg::Value(Value::Complete(Some((dat, meta)), postgc)) |
                        OzoneMsg::Value(Value::Chunk(Some((dat, meta)), _, postgc)) => {
                            // A deletion is stored as a tombstone value under the deleted key, and
                            // the reader finds it exactly as it finds any other value.  A key whose
                            // newest value is a tombstone has no value, so say so here, once, for
                            // every caller: a deleted key reads as absent, indistinguishable from
                            // one that was never written.  The tombstone is deliberately left
                            // unencrypted so that it can be recognised without a key.
                            if let Dat::Usr(kind, _) = &dat {
                                if *kind == id::usr_kind_id_deleted() {
                                    return Ok((None, postgc));
                                }
                            }
                            let or_is_some = match or {
                                Some(or) => or.is_some(),
                                None => false,
                            };
                            if enc.is_none() && !or_is_some {
                                return Ok((Some((dat, meta)), postgc));
                            }
                            let val = try_extract_dat!(dat, BU8, BU16, BU32, BU64);
                            let plain = res!(enc.or_decrypt(&val, or));
                            match Dat::from_bytes(&plain) {
                                Err(e) => return Err(err!(e,
                                    "Could not form a Dat from the value bytes, \
                                    this could be due to the use of an encryption scheme \
                                    differing from the one provided ({}).", enc.or_debug(or);
                                    Decode, Bytes)),
                                Ok((dat, _)) => return Ok((Some((dat, meta)), postgc)),
                            }
                        },
                        OzoneMsg::Value(Value::Complete(None, _)) |
                        OzoneMsg::Value(Value::Chunk(None, ..)) => Ok((None, false)),
                        msg => return Err(err!(
                            "Expected a OzoneMsg::Value containing a Value::Complete \
                            wrapping a Dat::BU64 but received a {:?}.", msg;
                            Unexpected, Input)),
                    }
                }
            },
        }
    }

    /// Collect one and only one reply within a given time, otherwise return an error.
    pub fn recv_timeout(&self, timeout: Duration) -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>> {
        match self.channel() {
            None => Err(err!("This responder does not have a channel."; Channel, Missing)),
            Some(simplex) => {
                match simplex.recv_timeout(timeout) {
                    Recv::Empty => Err(err!(
                        "Failed to receive a message via responder within {:.2} [s].",
                        timeout.as_secs_f32();
                        Missing, Data)),
                    Recv::Result(Err(e)) => Err(err!(e,
                        "Could not read from responder channel.";
                        Channel, Read)),
                    Recv::Result(Ok(msg)) => Ok(msg),
                }
            },
        }
    }

    /// Waits for the answers to a store dispatched with this responder: the count of records it
    /// was split into, then each record confirmed written and all of them confirmed durable.
    /// Returns whether the key already held a value, and the number of records.
    pub fn recv_store_ack(&self) -> Outcome<(bool, usize)> {
        // The count is sent before anything is dispatched, so it is waiting already.
        let n = match res!(self.recv_timeout(constant::USER_REQUEST_TIMEOUT)) {
            OzoneMsg::Chunks(n) => n,
            OzoneMsg::Error(e) => return Err(e),
            msg => return Err(err!(
                "Expected an OzoneMsg::Chunks counting the records of a store, received {:?}.", msg;
                Channel, Unexpected)),
        };
        // Passed on as it is, not wrapped: its tags are the caller's only way to tell a write
        // that landed unconfirmed (`Unconfirmed`) from one that did not, and a wrapper's tags are
        // all that `Error::tags` reports.
        let acks = ok!(self.recv_write_acks(
            n,
            constant::USER_REQUEST_TIMEOUT,
            constant::DURABILITY_TIMEOUT,
        ));
        let mut exists = false;
        for ack in acks {
            match ack {
                OzoneMsg::KeyExists(b)          => exists = b,
                OzoneMsg::KeyChunkExists(b, 0)  => exists = b, // the bunch key
                _ => (),
            }
        }
        Ok((exists, n))
    }

    /// Waits for the answer to a delete dispatched with this responder, and returns whether the
    /// key held a value.
    pub fn recv_delete_ack(&self) -> Outcome<bool> {
        // Passed on as it is, for its tags, as in `recv_store_ack`.
        let acks = ok!(self.recv_write_acks(
            1,
            constant::USER_REQUEST_TIMEOUT,
            constant::DURABILITY_TIMEOUT,
        ));
        match acks.first() {
            Some(OzoneMsg::KeyExists(b)) => Ok(*b),
            msg => Err(err!(
                "Expected an OzoneMsg::KeyExists answering a delete, received {:?}.", msg;
                Channel, Unexpected)),
        }
    }

    /// Collects the final answers to `n` records written under this responder.  Each record is
    /// first confirmed written, which is the writer's own work and so is held to `liveness`, and
    /// then confirmed durable and readable, which waits on the disk and is held to `durability`.
    /// Both deadlines measure silence, counted from the last answer of any kind: a value of many
    /// chunks is not failed for taking long while its writers keep answering, nor a busy disk
    /// for being slow while its barriers keep completing.
    ///
    /// Expiry of the first says a writer did not answer, so whether its record lands is unknown.
    /// Expiry of the second says every record is written but not all are confirmed durable, which
    /// is not a failure: the records are in the files and become durable, and readable, when the
    /// disk completes them.  That, and every error a store sends about a record after writing it,
    /// is tagged `Unconfirmed`, which nothing that fails a write before it lands is.
    pub fn recv_write_acks(
        &self,
        n:          usize,
        liveness:   Duration,
        durability: Duration,
    )
        -> Outcome<Vec<OzoneMsg<UIDL, UID, ENC, KH>>>
    {
        let chan = match self.channel() {
            Some(chan) => chan,
            None => return Err(err!("This responder does not have a channel."; Channel, Missing)),
        };
        let mut heard = Instant::now(); // the last answer, or the call
        let mut written = 0;
        let mut acks = Vec::with_capacity(n);
        while acks.len() < n {
            let deadline = if written < n { liveness } else { durability };
            let left = deadline.saturating_sub(heard.elapsed());
            if left.is_zero() {
                if written < n {
                    return Err(err!(
                        "{} of {} records of this write were confirmed written, and the writer \
                        holding the rest has said nothing for {:?}, so whether they land is not \
                        known.", written, n, liveness;
                        Channel, Timeout));
                }
                return Err(err!(
                    "All {} records of this write were written, but {} of them were not \
                    confirmed durable, the disk having completed nothing for {:?}.  The write \
                    has not failed: its records are in the store's files and become durable, \
                    and readable, when the disk completes them, unless the machine stops first.",
                    n, n - acks.len(), durability;
                    Write, Timeout, Unconfirmed));
            }
            match chan.recv_timeout(left) {
                Recv::Empty => (), // Out of time, which the next pass reports.
                Recv::Result(Err(e)) => return Err(err!(e,
                    "Could not read from responder channel.";
                    Channel, Read)),
                Recv::Result(Ok(msg)) => match msg {
                    OzoneMsg::Written => {
                        written += 1;
                        heard = Instant::now();
                    },
                    OzoneMsg::KeyExists(_) |
                    OzoneMsg::KeyChunkExists(..) => {
                        acks.push(msg);
                        heard = Instant::now();
                    },
                    OzoneMsg::Finish => (),
                    OzoneMsg::Error(e) => return Err(e),
                    msg => return Err(err!(
                        "Expected the answer to a write, received {:?}.", msg;
                        Channel, Unexpected)),
                },
            }
        }
        Ok(acks)
    }

    /// Collect replies within a given time.
    pub fn recv_number(
        &self,
        n: usize,
        wait: Wait,
    )
        -> Outcome<(Instant, Vec<OzoneMsg<UIDL, UID, ENC, KH>>)>
    {
        let mut msgs = Vec::new();
        let start = Instant::now();
        let mut count: usize = 0;
        match self.channel() {
            None => return Err(err!(
                "This responder does not have a channel.";
            Missing, Data)),
            Some(chan) => {
                loop {
                    match chan.recv_timeout(wait.check_interval) {
                        Recv::Empty => (),
                        Recv::Result(Err(e)) => return Err(err!(e,
                            "Could not read from responder channel.";
                            Channel, Read)),
                        // Neither is an answer: `Finish` trails one, and `Written` precedes a
                        // write's final answer, which is what a caller counting answers wants.
                        Recv::Result(Ok(OzoneMsg::Finish)) |
                        Recv::Result(Ok(OzoneMsg::Written)) => {
                            continue;
                        }
                        Recv::Result(Ok(msg)) => {
                            msgs.push(msg);
                            count += 1;
                            if count == n {
                                break;
                            }
                        }
                    }
                    if start.elapsed() > wait.max_wait {
                        if count < n {
                            return Err(err!(
                                "Expecting {} messages via responder, received {} when \
                                timed out after {:?}.", n, count, wait.max_wait;
                            Input, Mismatch, Size));
                        } else {
                            break;
                        }
                    }
                    if count > n {
                        return Err(err!(
                            "Expecting {} messages via responder, received {} after \
                            {:?}.", count, n, start.elapsed();
                        Input, Mismatch, Size));
                    }
                }
            },
        }
        Ok((start, msgs))
    }

    /// Collect pong messages within a given time, from a dedicated ping/pong `Responder`.
    pub fn recv_pongs(
        &self,
        wait: Wait,
    )
        -> Outcome<(Instant, HashSet<OzoneBotId>)>
    {
        let mut ozids = HashSet::new();
        let start = Instant::now();
        match self.channel() {
            None => return Err(err!(
                "This responder does not have a channel.";
            Missing, Data)),
            Some(chan) => {
                loop {
                    match chan.recv_timeout(wait.check_interval) {
                        Recv::Empty => {}
                        Recv::Result(Err(e)) => return Err(err!(e,
                            "Could not read from responder channel.";
                            Channel, Read)),
                        Recv::Result(Ok(OzoneMsg::Pong(ozid, _errs))) => {
                            ozids.insert(ozid);
                        }
                        Recv::Result(Ok(msg)) => {
                            error!(sync_log::stream(), err!(
                                "Expecting an OzoneMsg::Pong, received a {:?}.", msg;
                            Input, Mismatch, Unexpected));
                        }
                    }
                    if start.elapsed() > wait.max_wait {
                        break;
                    }
                }
            },
        }
        Ok((start, ozids))
    }

    /// Collect replies within a given time, until an `OzoneMsg::Finish` message is received.
    pub fn recv_all(
        &self,
        wait: Wait,
    )
        -> Outcome<(Instant, bool, Vec<OzoneMsg<UIDL, UID, ENC, KH>>)>
    {
        let mut complete = false;
        let mut msgs = Vec::new();
        let start = Instant::now();
        match self.channel() {
            None => return Err(err!(
                "This responder does not have a channel.";
            Missing, Data)),
            Some(chan) => {
                loop {
                    match chan.recv_timeout(wait.check_interval) {
                        Recv::Empty => (),
                        Recv::Result(Err(e)) => return Err(err!(e,
                            "Could not read from responder channel.";
                            Channel, Read)),
                        Recv::Result(Ok(OzoneMsg::Finish)) => {
                            complete = true;
                            break;
                        }
                        Recv::Result(Ok(msg)) => {
                            msgs.push(msg);
                        }
                    }
                    if start.elapsed() > wait.max_wait {
                        break;
                    }
                }
            },
        }
        Ok((start, complete, msgs))
    }
}

pub struct Wait {
    pub max_wait:       Duration,
    pub check_interval: Duration,
}

impl Default for Wait {
    fn default() -> Self {
        Self {
            max_wait:       constant::USER_REQUEST_TIMEOUT,
            check_interval: constant::CHECK_INTERVAL,
        }
    }
}

impl Wait {

    pub fn new(
        max_wait:       Duration,
        check_interval: Duration,
    )
        -> Outcome<Self>
    {
        if check_interval > max_wait {
            return Err(err!(
                "The given check interval, {:?}, should not be larger than the \
                given max wait, {:?}.", check_interval, max_wait;
            Invalid, Input));
        }
        Ok(Self {
            max_wait,
            check_interval,
        })
    }

    pub const fn new_default() -> Self {
        Self {
            max_wait:       constant::USER_REQUEST_TIMEOUT,
            check_interval: constant::CHECK_INTERVAL,
        }
    }

    pub fn timeout(
        max_wait: Duration,
    )
        -> Self
    {
        Self {
            max_wait,
            check_interval: Duration::default(),
        }
    }
}
