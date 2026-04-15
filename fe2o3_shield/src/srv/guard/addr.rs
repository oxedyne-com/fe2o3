//! SHIELD-side handshake layer on top of the generic
//! [`oxedyne_fe2o3_net::guard::addr::AddressGuard`].
//!
//! The rate-limiter, throttle, and blacklist state machine now lives generically in
//! `fe2o3_net`. This module keeps only the parts that are specific to the SHIELD UDP wire
//! protocol -- the three-step HReq1 then HReq2 then HReq3 handshake-sequence check -- and
//! layers them onto the generic guard via [`AddressGuard::update_log`].

use crate::srv::{
    guard::data::AddressData,
    msg::{
        core::MsgType,
        handshake::HandshakeType,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedyne_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};
use oxedyne_fe2o3_net::guard::addr::{
    AddressGuard,
    AddressLog,
};

use std::{
    fmt::Debug,
    net::SocketAddr,
    time::{
        Duration,
        SystemTime,
    },
};

/// Run the combined rate-limit + handshake-sequence check for an incoming SHIELD packet.
///
/// Returns `true` if the packet must be dropped. The generic guard decides on rate-limit
/// and blacklist grounds; this function adds the SHIELD-specific constraint that the first
/// packet from a new address must be an `HReq1` and that subsequent packets must follow the
/// recorded handshake sequence within `hreq_exp`.
pub fn drop_packet<
    const C: usize,
    M: MapMut<HashForm, AddressLog<N, AddressData>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static,
    const S: usize,
    const N: usize,
>(
    guard:      &AddressGuard<C, M, H, S, N, AddressData>,
    hreq_exp:   Duration,
    msg_typ:    MsgType,
    src_addr:   &SocketAddr,
)
    -> Outcome<bool>
{
    let htyp = HandshakeType::from(msg_typ);
    if htyp == HandshakeType::Unknown {
        return Ok(false);
    }

    let ip = src_addr.ip();

    // Pre-check: an unknown address may only contact us with an HReq1. Drop otherwise
    // without polluting the guard with a fresh log entry.
    if res!(guard.peek(&ip)).is_none() && htyp != HandshakeType::Req1 {
        return Ok(true);
    }

    // Feed the generic state machine. Under the same shard lock, validate the handshake
    // sequence on the shield-specific `AddressData`.
    let (decision, hs_drop) = res!(guard.update_log(&ip, |log, _was_new| {
        let pending = log.data.pending;
        match pending {
            Some((typ, when)) => {
                match when.elapsed() {
                    Ok(wait) if wait > hreq_exp => {
                        // Pending step expired: treat this packet as a fresh sequence start.
                        log.data.pending = None;
                        if htyp == HandshakeType::Req1 {
                            log.data.pending =
                                Some((HandshakeType::Req1, SystemTime::now()));
                        }
                    },
                    _ => match typ {
                        HandshakeType::Req1 => {
                            if !htyp.is_hreq2() {
                                return Ok(true);
                            }
                            log.data.pending = Some((htyp, SystemTime::now()));
                        },
                        HandshakeType::Req2 => {
                            if htyp != HandshakeType::Req3 {
                                return Ok(true);
                            }
                            log.data.pending = None;
                        },
                        _ => (),
                    },
                }
            },
            None => match htyp {
                HandshakeType::Req1 => {
                    log.data.pending = Some((HandshakeType::Req1, SystemTime::now()));
                },
                HandshakeType::Req2 | HandshakeType::Req3 => return Ok(true),
                _ => (),
            },
        }
        Ok(false)
    }));

    Ok(decision.should_drop() || hs_drop)
}
