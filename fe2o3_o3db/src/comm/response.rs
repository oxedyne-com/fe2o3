use crate::{
    prelude::*,
    base::id::{
        OzoneBotId,
        Ticket,
    },
    comm::msg::OzoneMsg,
    data::{
        core::Value,
    },
};

use oxedize_fe2o3_core::{
    alt::Override,
    channels::{
        Recv,
        Simplex,
    },
};
use oxedize_fe2o3_crypto::enc::EncryptionScheme;
use oxedize_fe2o3_iop_crypto::enc::EncrypterDefAlt;
use oxedize_fe2o3_iop_db::api::Meta;
use oxedize_fe2o3_jdat::{
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
            None => Err(err!(errmsg!("This responder does not have a channel."))),
            Some(ref simplex) => {
                match simplex.recv() {
                    Err(e) => return Err(err!(e, errmsg!(
                        "Could not read from responder channel",
                    ), Channel, Read)),
                    Ok(OzoneMsg::Error(e)) => return Err(e),
                    Ok(msg) => Ok(msg),
                }
            },
        }
    }

    //pub fn recv_message_count(&self, emsg: String) -> Outcome<usize> {
    //    match res!(self.recv()) {
    //        OzoneMsg::Error(e) => Err(err!(e, errmsg!("{}", emsg))),
    //        OzoneMsg::MessageCount(n) => Ok(n),
    //        msg => Err(err!(errmsg!("{}: {:?}", emsg, msg))),
    //    }
    //}

    pub fn send(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<()> {
        match self.channel() {
            Some(chan) => chan.send(msg),
            None => return Err(err!(errmsg!("This responder has no channel."), Channel, Write)),
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
            None => Err(err!(errmsg!("This responder does not have a channel."), Missing, Data)),
            Some(ref simplex) => {
                match simplex.recv_timeout(constant::USER_REQUEST_TIMEOUT) {
                    Recv::Empty => Err(err!(errmsg!(
                        "Failed to receive a message via responder within {:.2} [s].",
                        constant::USER_REQUEST_TIMEOUT.as_secs_f32(),
                    ), Missing, Data)),
                    Recv::Result(Err(e)) => return Err(err!(e, errmsg!(
                        "Could not read from responder channel",
                    ), Channel, Read)),
                    Recv::Result(Ok(msg)) => match msg {
                        OzoneMsg::Error(e) => return Err(e),
                        OzoneMsg::Value(Value::Complete(Some((dat, meta)), postgc)) => {
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
                                Err(e) => return Err(err!(e, errmsg!(
                                    "Could not form a Dat from the value bytes, \
                                    this could be due to the use of an encryption scheme \
                                    differing from the one provided ({}).", enc.or_debug(or),
                                ), Decode, Bytes)),
                                Ok((dat, _)) => return Ok((Some((dat, meta)), postgc)),
                            }
                        },
                        OzoneMsg::Value(Value::Complete(None, _)) |
                        OzoneMsg::Value(Value::Chunk(None, ..)) => Ok((None, false)),
                        msg => return Err(err!(errmsg!(
                            "Expected a OzoneMsg::Value containing a Value::Complete \
                            wrapping a Dat::BU64 but received a {:?}.", msg,
                        ), Unexpected, Input)),
                    }
                }
            },
        }
    }

    /// Collect one and only one reply within a given time, otherwise return an error.
    pub fn recv_timeout(&self, timeout: Duration) -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>> {
        match self.channel() {
            None => Err(err!(errmsg!("This responder does not have a channel."))),
            Some(simplex) => {
                match simplex.recv_timeout(timeout) {
                    Recv::Empty => Err(err!(errmsg!(
                        "Failed to receive a message via responder within {:.2} [s].",
                        timeout.as_secs_f32(),
                    ), Missing, Data)),
                    Recv::Result(Err(e)) => Err(err!(e, errmsg!(
                        "Could not read from responder channel.",
                    ), Channel, Read)),
                    Recv::Result(Ok(msg)) => Ok(msg),
                }
            },
        }
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
            None => return Err(err!(errmsg!(
                "This responder does not have a channel.",
            ), Missing, Data)),
            Some(chan) => {
                loop {
                    match chan.recv_timeout(wait.check_interval) {
                        Recv::Empty => (),
                        Recv::Result(Err(e)) => return Err(err!(e, errmsg!(
                            "Could not read from responder channel.",
                        ), Channel, Read)),
                        Recv::Result(Ok(OzoneMsg::Finish)) => {
                            continue; // Don't count Finish messages.
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
                            return Err(err!(errmsg!(
                                "Expecting {} messages via responder, received {} when \
                                timed out after {:?}.", n, count, wait.max_wait,
                            ), Input, Mismatch, Size));
                        } else {
                            break;
                        }
                    }
                    if count > n {
                        return Err(err!(errmsg!(
                            "Expecting {} messages via responder, received {} after \
                            {:?}.", count, n, start.elapsed(),
                        ), Input, Mismatch, Size));
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
            None => return Err(err!(errmsg!(
                "This responder does not have a channel.",
            ), Missing, Data)),
            Some(chan) => {
                loop {
                    match chan.recv_timeout(wait.check_interval) {
                        Recv::Empty => {}
                        Recv::Result(Err(e)) => return Err(err!(e, errmsg!(
                            "Could not read from responder channel.",
                        ), Channel, Read)),
                        Recv::Result(Ok(OzoneMsg::Pong(ozid))) => {
                            ozids.insert(ozid);
                        }
                        Recv::Result(Ok(msg)) => {
                            error!(err!(errmsg!(
                                "Expecting an OzoneMsg::Pong, received a {:?}.", msg,
                            ), Input, Mismatch, Unexpected));
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
            None => return Err(err!(errmsg!(
                "This responder does not have a channel.",
            ), Missing, Data)),
            Some(chan) => {
                loop {
                    match chan.recv_timeout(wait.check_interval) {
                        Recv::Empty => (),
                        Recv::Result(Err(e)) => return Err(err!(e, errmsg!(
                            "Could not read from responder channel.",
                        ), Channel, Read)),
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

    ///// Wait to receive all all replies associated with chunked data, but respond with only a
    ///// single message.  This really only makes sense for responses to a Write request.
    //pub fn recv_write_response(&self, num_chunks: usize) -> Outcome<OzoneMsg<UIDL, UID, ENC, KH>> {
    //    let mut msg = OzoneMsg::None;
    //    match self.channel() {
    //        None => return Err(err!(errmsg!("This responder does not have a channel."))),
    //        Some(ref simplex) => {
    //            for _ in 0..num_chunks+1 {
    //                match simplex.recv() {
    //                    Err(e) => return Err(err!(e, errmsg!(
    //                        "Could not read from responder channel",
    //                    ), Channel, Read)),
    //                    Ok(OzoneMsg::Error(e)) => return Err(err!(e, errmsg!(
    //                        "While gathering data chunks",
    //                    ), "ozone", "response")),
    //                    //Ok(OzoneMsg::KeyExists(b, cind)) => msg = OzoneMsg::KeyExists(b, cind),
    //                    //Ok(OzoneMsg::Value(optdatmeta, cind)) => msg = OzoneMsg::Value(optdatmeta, cind),
    //                    _ => (),
    //                }
    //            }
    //        },
    //    }
    //    Ok(msg)
    //}
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
            return Err(err!(errmsg!(
                "The given check interval, {:?}, should not be larger than the \
                given max wait, {:?}.", check_interval, max_wait,
            ), Invalid, Input));
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
