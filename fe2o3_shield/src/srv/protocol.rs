use crate::srv::{
    cfg::ServerConfig,
    constant,
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
            MsgAssembler,
            MsgAssemblyParams,
            MsgState,
        },
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
        DifficultyParams,
        PowPristine,
    },
    schemes::{
        WireSchemes,
        WireSchemesInput,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::FromBytes,
};
use oxedize_fe2o3_crypto::{
    keys::PublicKey,
    sign::SignatureScheme,
};
use oxedize_fe2o3_hash::{
    hash::{
        HasherDefAlt,
        HashScheme,
    },
    map::ShardMap,
    pow::{
        PowVars,
        ProofOfWork,
    },
};
use oxedize_fe2o3_data::ring::RingTimer;
use oxedize_fe2o3_iop_crypto::{
    enc::Encrypter,
    keys::KeyManager,
    sign::{
        Signer,
        SignerDefAlt,
    },
};
use oxedize_fe2o3_iop_hash::{
    api::{
        Hasher,
        HashForm,
    },
    csum::Checksummer,
};
use oxedize_fe2o3_jdat::id::NumIdDat;
use oxedize_fe2o3_namex::InNamex;
use oxedize_fe2o3_syntax::{
    core::SyntaxRef,
    msg::Msg,
};
use oxedize_fe2o3_text::string::Stringer;

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{
        Arc,
        RwLock,
    },
};


/// Capture all necessary information, and nothing more, allowing a thread to process an incoming
/// packet.  Rather than pass the entire struct atomically, use multiple interior atomic references
/// to reduce sharing wait times.
#[derive(Clone, Debug)]
pub struct Protocol<
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    MID:    NumIdDat<MIDL>,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
    // Data on the wire
	WENC:   Encrypter,      // Symmetric encryption of data on the wire.
	WCS:    Checksummer,    // Checks integrity of data on the wire.
    POWH:   Hasher,         // Packet validation proof of work hasher.
	SGN:    Signer,         // Digitally signs wire packets.
	HS:     Encrypter,      // Asymmetric encryption of symmetric encryption key during handshake.
>{
    // Let user define these generic parameters.
    _code_template:     [u8; C],
    _mid_template:      MID,
    _sid_template:      SID,
    _uid_template:      UID,

    pub test_mode:      bool,
    pub schms:          WireSchemes<WENC, WCS, POWH, SGN, HS>,

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
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    MID:    NumIdDat<MIDL>,
    SID:    NumIdDat<SIDL>,
    UID:    NumIdDat<UIDL>,
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
    POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    Protocol<C, MIDL, SIDL, UIDL, MID, SID, UID, WENC, WCS, POWH, SGN, HS>
{
    pub fn new(
        cfg:            &ServerConfig,
        schms_input:    WireSchemesInput<WENC, WCS, POWH, SGN, HS>,
        _code_template: [u8; C],
        _mid_template:  MID,
        _sid_template:  SID,
        _uid_template:  UID,
        test_mode:      bool,
    )
        -> Outcome<Self>
    {
        // Establish wire schemes.  The incoming WireSchemesInput uses Alt fields, allowing schemes
        // to be unspecified.  The protocol maintains a WireSchemes using DefAlt fields, which must
        // be specified.
        let no_chunker = schms_input.chnk.is_none();
        let mut schms = WireSchemes::from(schms_input);
        //// Initialise schemes using defaults.  Some of these can be updated in the config file.
        //schms.powh = HasherDefAlt(res!(ServerConfig::read_hash_scheme(
        //    &cfg.packet_pow_hash_scheme,
        //    &*HasherDefAlt::from(schms.powh),
        //    ServerConfig::default_packet_pow_hash_scheme,
        //    "packet_pow_hash_scheme",
        //)));
        //schms.sign = SignerDefAlt(res!(ServerConfig::read_signature_scheme(
        //    &cfg.packet_signature_scheme,
        //    &*SignerDefAlt::from(schms.sign),
        //    ServerConfig::default_packet_signature_scheme,
        //    "packet_signature_scheme",
        //)));
        if no_chunker {
            schms.chnk = cfg.chunk_config(); // Rather than ChunkCOnfig::default()
        }

        let agrd_map_init = BTreeMap::<
            HashForm,
            AddressLog<
                { constant::REQ_TIMER_LEN },
                { constant::MAX_ALLOWED_AVG_REQ_PER_SEC },
                AddressData,
            >,
        >::new();

        let agrd = Arc::new(AddressGuard {
            amap: res!(ShardMap::<
                {constant::AGRD_SHARDMAP_INIT_SHARDS},
                {constant::GUARD_SHARDMAP_SALT_LEN},
                AddressLog<
                    {constant::REQ_TIMER_LEN},
                    {constant::MAX_ALLOWED_AVG_REQ_PER_SEC},
                    AddressData,
                >,
                BTreeMap::<
                    HashForm,
                    AddressLog<
                        {constant::REQ_TIMER_LEN},
                        {constant::MAX_ALLOWED_AVG_REQ_PER_SEC},
                        AddressData,
                    >,
                >,
                HashScheme,
            >::new(
                constant::AGRD_SHARDMAP_INIT_SHARDS as u32,
                constant::SALT8,
                agrd_map_init,
                res!(HashScheme::try_from("Seahash")),
            )),
            // Monitor
            arps_max: constant::MAX_ALLOWED_AVG_REQ_PER_SEC,
            // Throttle
            tint_min: constant::THROTTLED_INTERVAL_MIN,   
            tsunset: (
                constant::ADDR_THROTTLE_SUNSET_SECS_MIN,
                constant::ADDR_THROTTLE_SUNSET_SECS_MAX,
            ),
            blist_cnt: constant::THROTTLE_COUNT_BEFORE_BLACKLIST,
            // Handshake
            hreq_exp: constant::SESSION_REQUEST_EXPIRY,
        });

        let ugrd_map_init = BTreeMap::<
            HashForm,
            UserLog<UserData<SIDL, C, SID>>,
        >::new();

        let ugrd = Arc::new(UserGuard {
            umap: res!(ShardMap::<
                {constant::UGRD_SHARDMAP_INIT_SHARDS},
                {constant::GUARD_SHARDMAP_SALT_LEN},
                UserLog<UserData<SIDL, C, SID>>,
                BTreeMap::<
                    HashForm,
                    UserLog<UserData<SIDL, C, SID>>,
                >,
                HashScheme,
            >::new(
                constant::UGRD_SHARDMAP_INIT_SHARDS as u32,
                constant::SALT8,
                ugrd_map_init,
                res!(HashScheme::try_from("Seahash")),
            )),
        });

        let packval = PacketValidator {
            pow: Some(res!(ProofOfWork::new(schms.powh.clone()))),
            sig: Some(schms.sign.clone()),
        };
        
        Ok(Self {
            _code_template,
            _mid_template,
            _sid_template,
            _uid_template,
            test_mode,
            schms,
            timer:          Arc::new(RwLock::new(RingTimer::<{ constant::REQ_TIMER_LEN }>::default())),
            agrd:           agrd.clone(),
            ugrd:           ugrd.clone(),
            packval,
            gpzparams:      DifficultyParams {
                                profile:    constant::POW_DIFFICULTY_PROFILE,
                                max:        constant::POW_MAX_ZERO_BITS,
                                min:        constant::POW_MIN_ZERO_BITS,
                                rps_max:    constant::MAX_ALLOWED_AVG_REQ_PER_SEC,
                            },
            massembler:     Arc::new(res!(MsgAssembler::<
                                { constant::MSG_ASSEMBLY_SHARDS },
                                _, _,
                                {constant::GUARD_SHARDMAP_SALT_LEN},
                            >::new(
                                constant::MSG_ASSEMBLY_SHARDS as u32,
                                constant::SALT8,
                                BTreeMap::<HashForm, MsgState>::new(),
                                res!(HashScheme::try_from("Seahash")),
                            ))),
            ma_params:      MsgAssemblyParams {
                                msg_sunset:     constant::MSG_ASSEMBLY_SUNSET,
                                idle_max:       constant::MSG_ASSEMBLY_IDLE_MAX,
                                rep_tot_lim:    constant::MSG_ASSEMBLY_REP_TOTAL_LIM,
                                rep_max_lim:    constant::MSG_ASSEMBLY_REP_PACKET_LIM,
                            },
            pow_time_horiz: constant::POW_TIME_HORIZON_SEC,
            accept_unknown: true,
        })
    }

    pub async fn handle(
        self,
        buf:        [u8; constant::UDP_BUFFER_SIZE], 
        n:          usize,
        src_addr:   SocketAddr,
        syntax:     SyntaxRef,
    )
        -> Outcome<()>
    {
        let src_addr_str = fmt!("{:?}", src_addr);
        match self.handler(buf, n, src_addr, syntax) {
            Err(e) => {
                let e2 = err!(e,
                    "While processing incoming packet from {}.", src_addr_str;
                    IO, Network);
                error!(e2.clone());
                Err(e2)
            },
            Ok(()) => {
                Ok(())
            },
        }
    }
    
    fn handler(
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
                trace!("POW rx:");
                res!(self.packval.trace(
                    powvars.as_ref(),
                    artefact,
                ));
            },
            None => return Err(err!(
                "Proof of work validation missing artefact.";
                Bug, Configuration, Missing)),
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
                let msgrx = Msg::new(syntax.clone());
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
                        _ => return Err(err!(
                            "Unrecognised message command '{}'.", cmd_name;
                            Bug, Unimplemented)),
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
