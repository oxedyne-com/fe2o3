//! Taking an incoming packet apart, in the order a hostile network makes
//! sensible.
//!
//! The work splits in two, and the split is what lets a peer that dials and a
//! peer that listens share it. [`Protocol::accept`] does everything that is
//! true of any packet whatever it turns out to say: rate-limit the address it
//! came from, look up what proof of work is being demanded of that address,
//! validate the artefacts, and hand the chunk to the assembler. What comes back
//! is either nothing -- the message is still incomplete, or the packet was
//! dropped -- or a whole message, which [`Protocol::read`] then parses against
//! the syntax. Deciding what to *do* with the commands in it is the caller's,
//! because a server answers requests and a client hears answers, and neither
//! wants the other's dispatch table.

use crate::{
    srv::{
        constant,
        msg::{
            core::{
                IdTypes,
                MsgFmt,
                MsgIds,
                MsgPow,
            },
            packet::{
                PacketMeta,
                PacketValidationArtefactRelativeIndices,
            },
            protocol::{
                Protocol,
                ProtocolTypes,
            },
        },
        pow::PowPristine,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::FromBytes,
};
use oxedyne_fe2o3_crypto::keys::PublicKey;
use oxedyne_fe2o3_hash::pow::PowVars;
use oxedyne_fe2o3_iop_crypto::keys::KeyManager;
use oxedyne_fe2o3_namex::InNamex;
use oxedyne_fe2o3_syntax::{
    core::SyntaxRef,
    msg::Msg,
};
use oxedyne_fe2o3_text::string::Stringer;

use std::net::{
    IpAddr,
    SocketAddr,
};


/// A message that arrived whole, together with the header of the packet that
/// completed it.
///
/// The header is kept because the parts of it that matter outside the packet
/// layer -- which message this was, who sent it, and what kind of message it
/// claims to be -- are exactly what an answer has to be addressed with.
#[derive(Clone, Debug)]
pub struct Accepted<
    const MIDL: usize,
    const UIDL: usize,
    MID: oxedyne_fe2o3_jdat::id::NumIdDat<MIDL>,
    UID: oxedyne_fe2o3_jdat::id::NumIdDat<UIDL>,
> {
    /// Header of the packet that completed the message.
    pub meta:   PacketMeta<MIDL, UIDL, MID, UID>,
    /// The assembled message bytes, ready to be parsed against a syntax.
    pub byts:   Vec<u8>,
}

/// A message parsed against the syntax, with the parts every command needs.
#[derive(Clone, Debug)]
pub struct Received<
    const SL: usize,
    const UL: usize,
    SID: oxedyne_fe2o3_jdat::id::NumIdDat<SL>,
    UID: oxedyne_fe2o3_jdat::id::NumIdDat<UL>,
> {
    /// Syntax the message was validated against, and the outgoing encoding.
    pub fmt:    MsgFmt,
    /// Proof-of-work parameters the sender declared.
    pub pow:    MsgPow,
    /// Session and user identifiers.
    pub ids:    MsgIds<SL, UL, SID, UID>,
    /// The parsed message, whose commands the caller dispatches.
    pub msg:    Msg,
}

impl<
    const C: usize,
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL> + 'static,
>
    Protocol<C, ML, SL, UL, P>
{
    /// Guard, validate and assemble one incoming packet.
    ///
    /// Returns the whole message when this packet was the one that completed
    /// it, and `None` whenever there is nothing yet to hand on: a packet
    /// dropped by a guard, a packet whose validation failed, or a chunk of a
    /// message still missing others. A dropped packet is not an error, because
    /// most of what arrives at a public UDP port is not addressed to anybody
    /// in good faith, and a loop that errored on each one would be a loop an
    /// attacker could fill with logging.
    ///
    /// `trg_ip` is the address the packet was received on, which the proof of
    /// work is bound to at both ends.
    pub fn accept(
        mut self,
        buf:        &[u8],
        src_addr:   SocketAddr,
        trg_ip:     IpAddr,
    )
        -> Outcome<Option<Accepted<
            ML,
            UL,
            <P::ID as IdTypes<ML, SL, UL>>::M,
            <P::ID as IdTypes<ML, SL, UL>>::U,
        >>>
    {
        let n = buf.len();
        {
            let mut unlocked_timer = lock_write!(self.timer);
            unlocked_timer.update();
        }
        debug!(async_log::stream(), "incoming [{}]:", n);
        for line in dump!(" {:02x}", &buf[..n], 32) {
            debug!(async_log::stream(), "{}", line);
        }
        // Packet:
        //                                                   validation
        //                                                   artefacts
        //                                                       |
        //               n1                               n2     |      n
        // +-------------+--------------------------------+-------------+
        //        |                        |                +----+ +----+
        //        |                        |
        //        |                        |                     |
        //       meta                   message              validation
        //                               chunk               artefacts
        //
        // 1. Read meta data.
        let (meta, n1) = res!(PacketMeta::<
            ML,
            UL,
            <P::ID as IdTypes<ML, SL, UL>>::M,
            <P::ID as IdTypes<ML, SL, UL>>::U,
        >::from_bytes(&buf[..n])); // Decode packet meta.
        debug!(async_log::stream(), "meta [{}]:", n1);
        for line in Stringer::new(fmt!("{:?}", meta)).to_lines("  ") {
            debug!(async_log::stream(), "{}", line);
        }
        //
        // 1. First line of defence: rate limiting and blacklisting against the source address.  We
        //    don't know if the sender of the packet is who they say they are, they could be
        //    address spoofing.  The threat of primary concern is DDOS, so we are looking for any
        //    excuse to drop a packet before committing more resources or degrading service for
        //    good users.  This check creates a new AddressLog entry if the source address is
        //    unknown and the request is an HREQ1.  This precedes validation because we want to
        //    collect any custom validation parameters for this address.
        if res!(crate::srv::guard::addr::drop_packet(
            &*self.agrd,
            self.hreq_exp,
            meta.typ,
            &src_addr,
        )) {
            debug!(async_log::stream(), "Address guard dropping packet.");
            return Ok(None); // Drop silently.
        }
        if res!(self.ugrd.drop_packet(&meta.uid, self.accept_unknown)) { // Accesses the user log.
            debug!(async_log::stream(), "User guard dropping packet.");
            return Ok(None); // Drop silently.
        }
        debug!(async_log::stream(), "");
        // A packet claiming a chunk it did not bring is a packet whose validation artefacts
        // would be read from somebody else's bytes. Refuse it here rather than slicing past
        // the end of the buffer.
        let n2 = n1 + (meta.chnk.chunk_size as usize);
        if n2 > n
            || n - n2 < PacketValidationArtefactRelativeIndices::BYTE_PREFIX_LEN
        {
            debug!(async_log::stream(),
                "Dropping packet of {} bytes: its header claims a {}-byte chunk after {} \
                bytes of metadata, leaving no room for the validation artefacts.",
                n, meta.chnk.chunk_size, n1);
            return Ok(None); // Drop silently.
        }
        let (afact_rel_ind, _) =
            res!(PacketValidationArtefactRelativeIndices::from_bytes(&buf[n2..n]));

        // Get the (locked) shared address and user maps, and unlock them in tight scopes when we
        // need to read or write.
        let (akey, locked_amap) = res!(self.agrd.get_locked_map(&src_addr));
        let (ukey, locked_umap) = res!(self.ugrd.get_locked_map(&meta.uid));

        debug!(async_log::stream(), "");
        // What are our proof of work requirements for the packet?
        let powvars = match self.packval.pow {
            Some(..) => {
                let zbits = {
                    let unlocked_amap = lock_read!(locked_amap);
                    if let Some(alog) = unlocked_amap.get(&akey) {
                        let unlocked_timer = lock_read!(self.timer);
                        let zbits = res!(
                            self.gpzparams.required_global_zbits(unlocked_timer.avg_rps()),
                            IO,
                        );
                        if zbits >= alog.data.my_zbits {
                            zbits
                        } else {
                            alog.data.my_zbits
                        }
                    } else {
                        return Err(err!(
                            "No AddressLog entry for {:?}, which should have been created \
                            by the AddressGuard::drop_packet call.", src_addr;
                            Bug, Missing));
                    }
                };
                let code = {
                    let unlocked_umap = lock_read!(locked_umap);
                    if let Some(ulog) = unlocked_umap.get(&ukey) {
                        ulog.data.code.clone().unwrap_or([0; C])
                    } else {
                        return Err(err!(
                            "No UserLog entry for {:?}, which should have been created \
                            by the UserGuard::drop_packet call.", meta.uid;
                            Bug, Missing));
                    }
                };
                let pristine = res!(PowPristine::<
                    C,
                    {constant::POW_PREFIX_LEN},
                    {constant::POW_PREIMAGE_LEN},
                >::new_rx(
                    code,
                    src_addr.ip(),
                    trg_ip,
                    self.pow_time_horiz,
                ));
                trace!(async_log::stream(), "POW Pristine rx:");
                res!(pristine.trace());

                Some(PowVars {
                    zbits,
                    pristine,
                })
            },
            _  => None,
        };
        // Insert my record of your public signing key into the packet signer for the purpose of
        // verification.
        match &mut self.packval.sig {
            Some(signer) => {
                let unlocked_umap = lock_read!(locked_umap);
                if let Some(ulog) = unlocked_umap.get(&ukey) {
                    let signer_nid = signer.local_id();
                    // The current signing scheme may differ from that for the public signing key I
                    // have on record, check it.
                    match &ulog.data.sigtpk_opt {
                        Some(sigtpk) => {
                            if sigtpk.sts.id != signer_nid {
                                return Err(err!(
                                    "Local scheme id, {:?}, for public signing key of user, {:02x?}, does not \
                                    match the nid for the current packet signing scheme, {:?}.",
                                    sigtpk.sts.id, meta.uid, signer_nid;
                                    Name, Mismatch));
                            }
                            // Update the signer with the public key I have for you.
                            *signer = res!(signer.clone_with_keys(Some(&sigtpk.key[..]), None));
                        },
                        None => (),
                    }
                } else {
                    return Err(err!(
                        "No UserLog entry for {:02x?}, which should have been created \
                        by the UserGuard::drop_packet call.", meta.uid;
                        Bug, Missing));
                }
            },
            _ => (),
        }

        //////// Debugging only
        match &afact_rel_ind.pow {
            Some(range) => {
                let artefact = &buf[n2 + range.start..n2 + range.end];
                trace!(async_log::stream(), "POW rx:");
                res!(self.packval.trace(
                    powvars.as_ref(),
                    artefact,
                ));
            },
            None => {
                debug!(async_log::stream(),
                    "Dropping packet from {}: proof of work required, none supplied.",
                    src_addr);
                return Ok(None); // Drop silently.
            },
        }
        ////////

        let validation = res!(self.packval.validate(
            &buf[..n],
            n2,
            afact_rel_ind,
            powvars,
            meta.typ,
        ));
        debug!(async_log::stream(), "{:?}", validation);
        let validity = fmt!("pow {} sig {}", validation.pow_state(), validation.sig_state());

        match validation.is_valid() {
            // sigpk_opt = possible public signing key that may be included in the packet
            // validation artefact.
            Some((valid, sigpk_opt)) => if !valid {
                // TODO Take action on an invalid signature provided by this address and user id.
                trace!(async_log::stream(), "Dropping packet: {}", validity);
                return Ok(None); // Drop silently.
            } else {
                // The packet signature was valid.
                debug!(async_log::stream(), "The packet is valid: {}", validity);
                match sigpk_opt {
                    Some((nid, sigpk_given)) => {
                        // A public signing key was supplied, and was used for verification.  My
                        // existing record of your public signing key, if it exists, was not used.
                        let mut unlocked_umap = lock_write!(locked_umap);
                        if let Some(ulog) = unlocked_umap.get_mut(&ukey) {
                            match &ulog.data.sigtpk_opt {
                                Some(sigtpk) => { // I have a record of your current public signing key.
                                    if sigtpk.key != sigpk_given {
                                        // The key you supplied doesn't match the one I've got.
                                        // I'll record the one I've got as old, and you'll be asked
                                        // to sign with it.  I won't regard the key you supplied as
                                        // genuine until you are validated using the old key.
                                        ulog.data.sigtpk_opt_old = Some(sigtpk.clone());
                                    } else {
                                        // The key you supplied perfectly matches the one I've got.
                                        match &ulog.data.sigtpk_opt_old {
                                            Some(_sigtpk_old) => {
                                                // I don't recognise the public key that you used.  It is possible
                                                // that I simply missed the key update.  So find the latest public
                                                // key I do have, in order to ask the peer to sign HReq2 using it,
                                                // so I can be sure this is the user I think it is.
                                                if let Some(pk) = ulog.data.pack_sigpk_set.first() {
                                                    ulog.data.sign_pack_this = Some(pk.key.clone());
                                                }
                                            },
                                            None => {
                                                // The earlier call to self.ugrd.drop_packet may have created a new
                                                // entry for an unrecognised uid, but with no public signing key,
                                                // I have no prior record of this user.  Whether I accept them as
                                                // a new user depends on our policy.
                                                if self.accept_unknown {
                                                    ulog.data.sigtpk_opt = Some(res!(PublicKey::now(
                                                        nid,
                                                        sigtpk.key.clone(),
                                                    )));
                                                } else {
                                                    // TODO If arranging for periodic garbage collection of users
                                                    // who lack packet public keys is more efficient, don't delete
                                                    // user just yet.
                                                    return Ok(None);
                                                }
                                            },
                                        }
                                    }
                                },
                                None => (), // TODO FINISHME I can't remember what is supposed to happen here!!!
                            }
                        } else {
                            return Err(err!(
                                "No UserLog entry for {:?}, which should have been created \
                                by the UserGuard::drop_packet call.", meta.uid;
                                Bug, Missing));
                        }
                    },
                    None => (), // The packet signature was valid, using the public key I possess.
                }
            },
            None => (),
        }
        // Ok, we're almost done on a packet level.  Insert the message chunk into the message
        // assembler, which returns the message when complete.  However, I may also have to drop
        // the packet if there is a problem.
        debug!(async_log::stream(), "");
        match res!(self.massembler.get_msg( // Message checkpoint, drop the partial message?
            &meta,
            &buf[n1..n2], // payload chunk
            &self.ma_params,
        )) { // Returns whether to drop the packet, and the potential syntax protocol message.
            (false, None) => Ok(None), // Payload remains incomplete.
            (false, Some(byts)) => Ok(Some(Accepted { meta, byts })),
            (true, _) => { // Drop the message completely.
                res!(self.massembler.remove(&meta.mid));
                Ok(None)
            },
        }
    }

    /// Parse an assembled message against the syntax, gathering the identifiers
    /// and proof-of-work parameters every command shares.
    pub fn read(
        &self,
        accepted:   &Accepted<
                        ML,
                        UL,
                        <P::ID as IdTypes<ML, SL, UL>>::M,
                        <P::ID as IdTypes<ML, SL, UL>>::U,
                    >,
        syntax:     SyntaxRef,
    )
        -> Outcome<Received<
            SL,
            UL,
            <P::ID as IdTypes<ML, SL, UL>>::S,
            <P::ID as IdTypes<ML, SL, UL>>::U,
        >>
    {
        let msgrx = Msg::new(syntax.clone());
        let mut msgrx = res!(msgrx.from_bytes(&accepted.byts, None));
        debug!(async_log::stream(), "msgrx [{}]: {}", accepted.byts.len(), msgrx);
        let ids: MsgIds<
            SL,
            UL,
            <P::ID as IdTypes<ML, SL, UL>>::S,
            <P::ID as IdTypes<ML, SL, UL>>::U,
        > = res!(MsgIds::from_msg(
            accepted.meta.uid,
            &mut msgrx,
        ));
        let pow = res!(MsgPow::from_msg(&mut msgrx));
        // The MsgFmt captures the syntax protocol against which incoming and outgoing
        // messages are validated, and the encoding for any outgoing messages.
        let fmt = MsgFmt {
            syntax,
            encoding: constant::DEFAULT_MSG_ENCODING, // TODO allow client to change
        };
        Ok(Received {
            fmt,
            pow,
            ids,
            msg: msgrx,
        })
    }
}
