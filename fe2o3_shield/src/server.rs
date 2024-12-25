use crate::{
    constant,
    core::Protocol,
    guard::{
        addr::{
            AddressGuard,
            AddressLog,
        },
        data::{
            AddressData,
            UserData,
        },
        user::{
            UserGuard,
            UserLog,
        },
    },
    msg::{
        external::{
            //HandshakeType,
            //IdentifiedMessage,
            //Message,
            MsgAssemblyParams,
            //MsgType,
            MsgAssembler,
            MsgState,
        },
        internal::ServerMsg,
        syntax::{
            HReq1,
            MsgFmt,
            MsgIds,
            MsgPow,
        },
    },
    packet::{
        PacketMeta,
        PacketValidationArtefactRelativeIndices,
        PacketValidator,
    },
    pow::{
        PowPristine,
        DifficultyParams,
    },
};

use oxedize_fe2o3_bot::{
    bot::{
        Bot,
        LoopBreak,
    },
    handles::Handle,
};
use oxedize_fe2o3_core::{
    prelude::*,
    byte::{
        FromBytes,
    },
    channels::{
        Simplex,
        Recv,
    },
    thread::{
        Semaphore,
        Sentinel,
    },
};
use oxedize_fe2o3_crypto::{
    sign::SignatureScheme,
    keys::PublicKey,
};
use oxedize_fe2o3_data::ring::RingTimer;
use oxedize_fe2o3_iop_crypto::{
    enc::{
        Encrypter,
    },
    keys::KeyManager,
    sign::{
        Signer,
        SignerDefAlt,
    },
};
use oxedize_fe2o3_hash::{
    hash::{
        HasherDefAlt,
        HashScheme,
    },
    pow::PowVars,
};
use oxedize_fe2o3_iop_hash::{
    api::{
        Hasher,
        HashForm,
    },
    csum::Checksummer,
};
use oxedize_fe2o3_jdat::id::{
    IdDat,
    NumIdDat,
};
use oxedize_fe2o3_namex::id::InNamex;
use oxedize_fe2o3_syntax::{
    core::SyntaxRef,
    msg::{
        Msg as SyntaxMsg,
    },
};
use oxedize_fe2o3_text::string::Stringer;

use std::{
    collections::BTreeMap,
    io,
    marker::PhantomData,
    net::{
        SocketAddr,
        UdpSocket,
    },
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

use tokio::task;

/// Capture all necessary information, and nothing more, allowing a thread to process an incoming
/// packet.  Rather than pass the entire struct atomically, use multiple interior atomic references
/// to reduce sharing wait times.
#[derive(Clone, Debug)]
pub struct RxEnv<
    const C: usize,
    const SIDL: usize,
    SID:    NumIdDat<SIDL>,
	POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
>{
    pub timer:          Arc<RwLock<RingTimer<{ constant::REQ_TIMER_LEN }>>>,
    // Address protection.
    pub agrd:           Arc<AddressGuard<
                            { constant::AGRD_SHARDMAP_INIT_SHARDS },
                            BTreeMap<
                                HashForm,
                                AddressLog<
                                    { constant::REQ_TIMER_LEN },
                                    { constant::MAX_ALLOWED_AVG_REQ_PER_SEC },
                                    AddressData,
                                >,
                            >,
                            HashScheme,
                            { constant::GUARD_SHARDMAP_SALT_LEN },
                            { constant::REQ_TIMER_LEN },
                            { constant::MAX_ALLOWED_AVG_REQ_PER_SEC },
                            AddressData,
                        >>,
    // User protection.
    pub ugrd:           Arc<UserGuard<
                            { constant::UGRD_SHARDMAP_INIT_SHARDS },
                            BTreeMap<
                                HashForm,
                                UserLog<UserData<SIDL, C, SID>>,
                            >,
                            HashScheme,
                            { constant::GUARD_SHARDMAP_SALT_LEN },
                            UserData<SIDL, C, SID>,
                        >>,
    // Packet validation.
    pub packval:        PacketValidator<
                            HasherDefAlt<HashScheme, POWH>,
                            SignerDefAlt<SignatureScheme, SGN>,
                        >,
    pub gpzparams:      DifficultyParams,
    // Message assembly.
    pub massembler:     Arc<MsgAssembler<
                            { constant::MSG_ASSEMBLY_SHARDS },
                            BTreeMap<HashForm, MsgState>,
                            HashScheme,
                            { constant::GUARD_SHARDMAP_SALT_LEN },
                        >>,
    pub ma_params:      MsgAssemblyParams,
    // Policy configuration.
    pub pow_time_horiz: u64,
    pub accept_unknown: bool,
}

impl<
    const C: usize,
    const SIDL: usize,
    SID:    NumIdDat<SIDL>,
	POWH:   Hasher,
	SGN:    Signer,
>
    RxEnv<C, SIDL, SID, POWH, SGN>
{
    pub async fn process<
        const MIDL: usize,
        const UIDL: usize,
        MID: NumIdDat<MIDL>,
        UID: NumIdDat<UIDL>,
    >(
        self,
        buf:        [u8; constant::UDP_BUFFER_SIZE], 
        n:          usize,
        src_addr:   SocketAddr,
        syntax:     SyntaxRef,
    )
        -> Outcome<()>
    {
        let src_addr_str = fmt!("{:?}", src_addr);
        match self.get_process_result::<MIDL, UIDL, MID, UID>(buf, n, src_addr, syntax) {//buf) {
            Err(e) => {
                let e2 = err!(e, fmt!("While processing incoming packet from {}.", src_addr_str));
                error!(e2.clone());
                Err(e2)
            },
            Ok(()) => {
                Ok(())
            },
        }
    }
    
    fn get_process_result<
        const MIDL: usize,
        const UIDL: usize,
        MID: NumIdDat<MIDL>,
        UID: NumIdDat<UIDL>,
    >(
        mut self,
        buf:        [u8; constant::UDP_BUFFER_SIZE], 
        n:          usize,
        src_addr:   SocketAddr,
        syntax:     SyntaxRef,
    )
        -> Outcome<()>
    {
        {
            let mut unlocked_timer = lock_write!(self.timer);
            unlocked_timer.update();
        }
        debug!("incoming [{}]:", n);
        for line in dump!(" {:02x}", &buf[..n], 32) {
            debug!("{}", line);
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
        let (meta, n1) = res!(PacketMeta::<MIDL, UIDL, MID, UID>::from_bytes(&buf[..n])); // Decode packet meta.
        debug!("meta [{}]:", n1);
        for line in Stringer::new(fmt!("{:?}", meta)).to_lines("  ") {
            debug!("{}", line);
        }
        //let uid = alias::Uid::from_be_bytes(
        //    res!(<[u8; constant::USER_ID_BYTE_LEN]>::try_from(&meta.uid), Decode, Bytes)
        //);
        //  pub struct PacketMeta<const U: usize> {
        //      pub typ:    MsgType,
        //      pub ver:    SemVer,
        //      pub mid:    MsgIds,
        //      pub uid:    [u8; U], // user id
        //      pub chnk:   PacketChunkState,
        //      pub tstamp: u64,
        //  }
        // 
        // 1. First line of defence: rate limiting and blacklisting against the source address.  We
        //    don't know if the sender of the packet is who they say they are, they could be
        //    address spoofing.  The threat of primary concern is DDOS, so we are looking for any
        //    excuse to drop a packet before committing more resources or degrading service for
        //    good users.  This check creates a new AddressLog entry if the source address is
        //    unknown and the request is an HREQ1.  This precedes validation because we want to
        //    collect any custom validation parameters for this address.
        if res!(self.agrd.drop_packet(meta.typ, &src_addr)) { // Accesses the address log.
            debug!("Address guard dropping packet.");
            return Ok(()); // Drop silently.
        }
        if res!(self.ugrd.drop_packet(&meta.uid, self.accept_unknown)) { // Accesses the user log.
            debug!("User guard dropping packet.");
            return Ok(()); // Drop silently.
        }
        debug!("");
        let n2 = n1 + (meta.chnk.chunk_size as usize);
        let (afact_rel_ind, _) =
            res!(PacketValidationArtefactRelativeIndices::from_bytes(&buf[n2..n]));
    
        // Get the (locked) shared address and user maps, and unlock them in tight scopes when we
        // need to read or write.
        let (akey, locked_amap) = res!(self.agrd.get_locked_map(&src_addr));
        let (ukey, locked_umap) = res!(self.ugrd.get_locked_map(&meta.uid));
        
        debug!("");
        // What are our proof of work requirements for the packet?
        let powvars = match self.packval.pow {
            Some(..) => {
                let zbits = {
                    let unlocked_amap = lock_read!(locked_amap);
                    if let Some(alog) = unlocked_amap.get(&akey) {
                        let unlocked_timer = lock_read!(self.timer);
                        let zbits = res!(
                            self.gpzparams.required_global_zbits(unlocked_timer.avg_rps() as u16),
                            IO,
                        );
                        if zbits >= alog.data.my_zbits {
                            zbits
                        } else {
                            alog.data.my_zbits
                        }
                    } else {
                        return Err(err!(errmsg!(
                            "No AddressLog entry for {:?}, which should have been created \
                            by the AddressGuard::drop_packet call.", src_addr,
                        ), Bug, Missing));
                    }
                };
                let code = {
                    let unlocked_umap = lock_read!(locked_umap);
                    if let Some(ulog) = unlocked_umap.get(&ukey) {
                        ulog.data.code.clone().unwrap_or([0; C])
                    } else {
                        return Err(err!(errmsg!(
                            "No UserLog entry for {:?}, which should have been created \
                            by the UserGuard::drop_packet call.", meta.uid,
                        ), Bug, Missing));
                    }
                };
                let pristine = res!(PowPristine::<
                    C,
                    {constant::POW_PREFIX_LEN},
                    {constant::POW_PREIMAGE_LEN},
                >::new_rx(
                    code,
                    src_addr.ip(),
                    self.pow_time_horiz, 
                ));
                trace!("POW Pristine rx:");
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
                                return Err(err!(errmsg!(
                                    "Local scheme id, {:?}, for public signing key of user, {:02x?}, does not \
                                    match the nid for the current packet signing scheme, {:?}.",
                                    sigtpk.sts.id, meta.uid, signer_nid,
                                )));
                            }
                            // Update the signer with the public key I have for you.
                            *signer = res!(signer.clone_with_keys(Some(&sigtpk.key[..]), None));
                        },
                        None => (),
                    }
                } else {
                    return Err(err!(errmsg!(
                        "No UserLog entry for {:02x?}, which should have been created \
                        by the UserGuard::drop_packet call.", meta.uid,
                    ), Bug, Missing));
                }
            },
            _ => (),
        }

        //////// Debugging only
        match &afact_rel_ind.pow {
            Some(range) => {
                let artefact = &buf[n2 + range.start..n2 + range.end];
                trace!("POW rx:");
                res!(self.packval.trace(
                    powvars.as_ref(),
                    artefact,
                ));
            },
            None => return Err(err!(errmsg!(
                "Proof of work validation missing artefact.",
            ), Bug, Configuration, Missing)),
        }
        ////////
        
        let validation = res!(self.packval.validate(
            &buf[..n],
            n2,
            afact_rel_ind,
            powvars,
            meta.typ,
        ));
        debug!("{:?}", validation);
        let validity = fmt!("pow {} sig {}", validation.pow_state(), validation.sig_state());

        match validation.is_valid() {
            // sigpk_opt = possible public signing key that may be included in the packet
            // validation artefact.
            Some((valid, sigpk_opt)) => if !valid {
                // TODO Take action on an invalid signature provided by this address and user id.
                trace!("Dropping packet: {}", validity);
                return Ok(()); // Drop silently.
            } else {
                // The packet signature was valid.
                debug!("The packet is valid: {}", validity);
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
                                        //ulog.data.sigtpk = Some(PublicKey {
                                        //    sts: SchemeTimestamp::now(nid),
                                        //    key: sigpk_given,
                                        //});
                                    } else {
                                        // The key you supplied perfectly matches the one I've got.
                                        match &ulog.data.sigtpk_opt_old {
                                            Some(_sigtpk_old) => {
                                                // I don't recognise the public key that you used.  It is possible
                                                // that I simply missed the key update.  So find the latest public
                                                // key I do have, in order to ask the peer to sign HReq2 using it,
                                                // so I can be sure this is the user I think it is.
                                                //if let Some(pk) = ulog.data.pack_sigpk_set.iter().next() {
                                                // TODO Replace line above with line below when
                                                // https://github.com/rust-lang/rust/issues/62924 stablises.
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
                                                    return Ok(());
                                                }
                                            },
                                        }
                                    }
                                },
                                None => (), // TODO FINISHME I can't remember what is supposed to happen here!!!
                            }
                        } else {
                            return Err(err!(errmsg!(
                                "No UserLog entry for {:?}, which should have been created \
                                by the UserGuard::drop_packet call.", meta.uid,
                            ), Bug, Missing));
                        }
                    },
                    None => (), // The packet signature was valid, using the public key I possess.
                }
            },
            None => (),
        }
        // Ok, we're almost done on a packet level.  Insert the message chunk into the AddressLog
        // partial message map, which returns the message when complete.  However, I may also have
        // to drop the packet if there is a problem.
        debug!("");
        match res!(self.massembler.get_msg( // Message checkpoint, drop the partial message?
            &meta,
            &buf[n1..n2], // payload + validator data
            &self.ma_params,
        )) { // Returns whether to drop the packet, and the potential syntax protocol message.
            (false, None) => return Ok(()), // Payload remains incomplete.
            (false, Some(msg_byts)) => { // We have a complete message.
                let msgrx = SyntaxMsg::new(syntax.clone());
                let mut msgrx = res!(msgrx.from_bytes(&msg_byts, None));
                debug!("msgrx [{}]: {}", msg_byts.len(), msgrx);
                // Gather the proof of work parameters required by the
                // client.
                let msgids: MsgIds<SIDL, UIDL, SID, UID> = res!(MsgIds::from_msg(
                    meta.uid,
                    &mut msgrx,
                ));
                let msgpow = res!(MsgPow::from_msg(&mut msgrx));
                // The MsgFmt captures the syntax protocol against which incoming and outgoing
                // messages are validated, and the encoding for any outgoing messages.
                let msgfmt = MsgFmt {
                    syntax: syntax.clone(),
                    encoding: constant::DEFAULT_MSG_ENCODING, // TODO allow client to change
                };
        
                // Multiple commands in a single message are permitted.
                for (cmd_name, mut msgcmd) in msgrx.cmds {
                    match cmd_name.as_str() {
                        "hreq1" => {
                            debug!("HREQ1");
                            let mut scmd: HReq1<SIDL, UIDL, SID, UID> = HReq1 {
                                fmt: msgfmt.clone(),
                                pow: msgpow.clone(),
                                mid: msgids.clone(),
                                ..Default::default()
                            };
                            // Each command type can implement its own
                            // custom process method, which captures
                            // only the parameters it needs.
                            let mut unlocked_amap = lock_write!(locked_amap);
                            if let Some(alog) = unlocked_amap.get_mut(&akey) {
                                //if let Some(mut alog_data) = alog.data.as_mut() {
                                    res!(scmd.server_process(
                                        &mut msgcmd,
                                        //alog.data.as_mut(), // For pow parameters.
                                        &mut alog.data, // For pow parameters.
                                        //&mut self.ugrd, // For user signing pk.
                                        // For sending HResp1.
                                        //&self.sock_addr,
                                        //Config::chunker(self.wschms.clone_chunk_config()),
                                        //&self.sock,
                                        //&self.src_addr,
                                    ));
                                //}
                            }
                        },
                    //    "hresp1" => {
                    //        let mut scmd = syntax::HResp1 {
                    //            fmt: msgfmt.clone(),
                    //            mid: msgid.clone(),
                    //            ..Default::default()
                    //        };
                    //        debug!("hresp1 recvd");
                    //        Ok(())
                    //        //scmd.process(
                    //        //    &mut msgcmd,
                    //        //    &mut self.ugrd, // For user signing pk.
                    //        //    // For sending.
                    //        //    self.sock_addr.clone(),
                    //        //    self.pack_size,
                    //        //    &self.sock,
                    //        //    &src_addr,
                    //        //)
                    //    },
                    //    //"hreq2" => {
                    //    //    let mut scmd = syntax::HReq1 {
                    //    //        fmt: msgfmt.clone(),
                    //    //        mid: msgid.clone(),
                    //    //        ..Default::default()
                    //    //    };
                    //    //    // Each command type can implement its own
                    //    //    // custom process method, which captures
                    //    //    // only the parameters it needs.
                    //    //    scmd.server_process(
                    //    //        &mut msgcmd,
                    //    //        &mut alog.data, // For pow parameters.
                    //    //        &mut self.ugrd, // For user signing pk.
                    //    //        // For sending HResp1.
                    //    //        self.sock_addr.clone(),
                    //    //        self.pack_size,
                    //    //        &self.sock,
                    //    //        &src_addr,
                    //    //    )
                    //    //},
                        _ => return Err(err!(errmsg!(
                            "Unrecognised message command '{}'.", cmd_name,
                        ), Bug, Unimplemented)),
                    }
                }
            }, // Read payload.
            (true, _) => { // Drop the message completely.
                res!(self.massembler.remove(&meta.mid));
            },
        } // Assemble payload packets.
        Ok(())
    }
}

/// Ozone servers maintain an address map, tracking the communication requirements for Internet
/// Protocol (IP) addresses of peers, and a user map, tracking trust level of users authenticated
/// via Digital Signatures.  Server user data is persisted to and from the Ozone database itself.
///
/// # Denial of Service (DOS)
///
/// The server is address-agnostic, allowing any address to start a session via a handshake request
/// (HREQ1).  This however leaves it exposed to DOS attack, which commonly relies on address
/// spoofing.  The main defence is that every packet received must be prepended with a valid Proof
/// of Work (POW), using a hash function that is very fast to validate.  The POW difficulty is
/// determined by the number of leading zero bits in the hash.  The server calculates a global
/// difficulty in real time as a function of the average requests per second.  Should this climb,
/// indicating a heightened DOS risk, the global difficulty automatically increases.  At the same
/// time, each address that has established a session can have a varying difficulty reflecting the
/// address behaviour and trust in the user.  The difficulty required by the server for any packet
/// is the higher of the global and session values.
///
/// The POW contains a code, which also has a global and session value.  The global value can be
/// changed in response to a DOS attack, and shared to peers via sessions, and/or via a side
/// channel to users.  The global value is only used in initial session-establishing HREQ1
/// messages, and thus is called a prime code.
///
/// The server listens to both external UDP messages (defined by the `Syntax`) and internal
/// messages of type `M` via a `Simplex` channel. 
#[derive(Debug)]
pub struct ServerBot<
    const BIDL: usize,
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    BID:    NumIdDat<BIDL>,
    MID:    NumIdDat<MIDL>,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
	WENC:   Encrypter,
	WCS:    Checksummer,
	POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter,
> {
    // Bot
    pub id:         IdDat<BIDL, BID>,
    pub protocol:   Protocol<WENC, WCS, POWH, SGN, HS>,
    pub sem:        Semaphore,
    pub errc:       Arc<Mutex<usize>>,
    pub inited:     bool,
    pub chan_in:    Simplex<ServerMsg>,
    //pack_sigkeys:   Arc<BTreeMap<Vec<u8>, SecretKey>>, // My packet digital signature keys.
    //msg_sigkeys:    Arc<BTreeMap<Vec<u8>, SecretKey>>, // My message digital signature keys.
    pub rxenv:      RxEnv<C, SIDL, SID, POWH, SGN>,
    pub pack_size:  usize,
    pub sock:       UdpSocket,
    pub sock_addr:  SocketAddr,
    pub ma_gc_last: Instant,
    pub ma_gc_int:  Duration,
    pub phantom1:   PhantomData<MID>,
    pub phantom2:   PhantomData<UID>,
}

impl<
    const BIDL: usize,
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    BID:    NumIdDat<BIDL> + 'static,
    MID:    NumIdDat<MIDL> + 'static,
    SID:    NumIdDat<SIDL> + 'static,
    UID:    NumIdDat<UIDL> + 'static,
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
	POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    Bot<BIDL, BID, ServerMsg> for
    ServerBot<BIDL, C, MIDL, SIDL, UIDL, BID, MID, SID, UID, WENC, WCS, POWH, SGN, HS>
{
    fn id(&self)        -> BID                  { *self.id }
    fn errc(&self)      -> &Arc<Mutex<usize>>   { &self.errc }
    fn chan_in(&self)   -> &Simplex<ServerMsg>  { &self.chan_in }
    fn label(&self)     -> String               { fmt!("Shield Server {:?}", self.id()) }
    fn err_count_warning(&self) -> usize        { constant::SERVER_BOT_ERROR_COUNT_WARNING }

    fn set_chan_in(&mut self, chan_in: Simplex<ServerMsg>) {
        self.chan_in = chan_in;
    }


    fn init(&mut self) -> Outcome<()> {
        info!("{}: Now listening on UDP at {:?}.",
            self.label(), res!(self.sock.local_addr()),
        );
        self.inited = true;
        Ok(())
    }

    fn go(&mut self) {
        if !self.inited {
            error!(err!(errmsg!(
                "Attempt to start {} before running init().", self.label(),
            ), Init, Missing));
        } else {
            debug!("HELLO");
            match tokio::runtime::Runtime::new() {
                Err(e) => error!(err!(e, errmsg!(
                    "Failed to start Tokio runtime.",
                ), Init)),
                Ok(rt) => rt.block_on(async {
                    self.now_listening();
                    loop {
                        if self.async_listen().await.must_end() { break; }
                    }
                }),
            }
        }
    }

}

impl<
    const BIDL: usize,
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    BID:    NumIdDat<BIDL> + 'static,
    MID:    NumIdDat<MIDL> + 'static,
    SID:    NumIdDat<SIDL> + 'static,
    UID:    NumIdDat<UIDL> + 'static,
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
	POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    ServerBot<BIDL, C, MIDL, SIDL, UIDL, BID, MID, SID, UID, WENC, WCS, POWH, SGN, HS>
{
    pub fn start(mut self, sentinel: Sentinel) -> Outcome<Handle<()>> {
        let id_string = self.id().to_string();
        let builder = thread::Builder::new()
            .name(id_string.clone())
            .stack_size(constant::STACK_SIZE);
        Ok(Handle::new(
            id_string,
            res!(builder.spawn(move || { self.go(); })),
            sentinel,
        ))
    }

    pub async fn async_listen(&mut self) -> LoopBreak {
        // INTERNAL
        match self.chan_in().recv_timeout(constant::SERVER_INT_CHANNEL_CHECK_INTERVAL) {
            Recv::Empty => (),
            Recv::Result(Err(e)) => self.err_cannot_receive(e),
            Recv::Result(Ok(ServerMsg::Finish)) => {
                trace!("Finish message received, {:?} finishing now.", self.id());
                //if let Err(e) = self.chan_in().rev().send(ServerMsg::Finish) {
                //    self.err_cannot_send(e, errmsg!("Attempt to return a finish message failed"));
                //}
                return LoopBreak(true);
            },
            Recv::Result(Ok(ServerMsg::Ready)) => {
                info!("{:?} ready to receive messages now.", self.id());
            },
            //Recv::Result(Ok(ServerMsg::Marco(id::Mid::randef())) => {
            //    trace!("{}: Ping received from owner, replying...", self.id());
            //    if let Err(e) = self.chans().rev().send(ServerMsg::Polo) {
            //        self.err_cannot_send(e, errmsg!("Attempt to return a ping failed"));
            //    }
            //},
            //Recv::Result(Ok(ServerMsg::Polo(mid))) => {
            //    trace!("{}: Ping received from owner, replying...", self.id());
            //    if let Err(e) = self.chans().rev().send(ServerMsg::Polo) {
            //        self.err_cannot_send(e, errmsg!("Attempt to return a ping failed"));
            //    }
            //},
            //Recv::Result(Ok(msg)) => error!(err!(fmt!(
            //    "{}: Message {:?} not recognised.", self.id(), msg,
            //), Invalid, Input)),
        }

        let mut buf = [0u8; constant::UDP_BUFFER_SIZE]; 
        // EXTERNAL
        match self.sock.recv_from(&mut buf) { // Receive udp packet, non-blocking.
            Err(e) => {
                //match self.timer.write() {
                //    Err(e) => self.error(err!(errmsg!(
                //        "While locking timer for writing: {}.", e), Poisoned)),
                //    Ok(mut unlocked_timer) => { unlocked_timer.update(); },
                //}
                ////self.timer.update();
                match e.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::InvalidInput => (),
                    _ => self.err_cannot_receive(Error::from(e)),
                }
            },
            Ok((n, src_addr)) => {
                let rxenv = self.rxenv.clone();
                let handle = task::spawn(rxenv.process::<MIDL, UIDL, MID, UID>(
                    buf,
                    n,
                    src_addr,
                    self.protocol.schms.syntax.clone(), // Arc
                ));
                match handle.await {
                    Ok(result) => self.result(&result),
                    Err(e) => self.error(
                        err!(e, errmsg!("While waiting for request processor to finish"))),
                }
            },
        } // Receive udp packet.

        // Message assembly garbage collection.
        if self.ma_gc_last.elapsed() > self.ma_gc_int {
            let result = self
                .rxenv.massembler
                .message_assembly_garbage_collection(&self.rxenv.ma_params);
            self.result(&result);
            self.ma_gc_last = Instant::now();
        }

        LoopBreak(false)
    }
}
