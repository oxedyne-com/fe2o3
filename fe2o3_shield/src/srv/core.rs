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
            MsgBuilder,
            MsgState,
        },
        internal::{
            ServerMsg,
        },
    },
    packet::PacketValidator,
    pow::{
        DifficultyParams,
        PowPristine,
    },
    schemes::{
        WireSchemes,
        WireSchemesInput,
    },
    server::{
        RxEnv,
        ServerBot,
    },
};

use oxedize_fe2o3_bot::{
    bot::Bot,
};
use oxedize_fe2o3_core::{
    prelude::*,
    alt::Alt,
    channels::{
        simplex,
        Simplex,
    },
    thread::{
        Sentinel,
        thread_channel,
    },
};
use oxedize_fe2o3_crypto::{
    sign::SignatureScheme,
};
use oxedize_fe2o3_data::ring::RingTimer;
use oxedize_fe2o3_iop_crypto::{
    enc::Encrypter,
    sign::{
        Signer,
        SignerDefAlt,
    },
};
use oxedize_fe2o3_jdat::{
    cfg::Config,
    chunk::ChunkConfig,
    file::JdatMapFile,
    id::{
        IdDat,
        NumIdDat,
    },
};
use oxedize_fe2o3_hash::{
    hash::{
        HasherDefAlt,
        HashScheme,
    },
    map::ShardMap,
    pow::{
        PowCreateParams,
        PowVars,
        ProofOfWork,
    },
};
use oxedize_fe2o3_iop_hash::{
    api::{
        Hasher,
        HashForm,
    },
    csum::Checksummer,
};
use oxedize_fe2o3_syntax::core::SyntaxRef;

use std::{
    collections::BTreeMap,
    marker::PhantomData,
    net::{
        IpAddr,
        SocketAddr,
        UdpSocket,
    },
    path::Path,
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
    time::{
        Duration,
        Instant,
        SystemTime,
        UNIX_EPOCH,
    },
};

use local_ip_address::local_ip;


impl<
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter,
>
    Protocol<WENC, WCS, POWH, SGN, HS>
{
    pub fn new<P: AsRef<Path>>(
        cfg_opt:        Option<P>,
        schms_input:    WireSchemesInput<WENC, WCS, POWH, SGN, HS>,
    )
        -> Outcome<Self>
    {
        // Check constants.
        res!(ServerConfig::check_constants());

        let mut cfg = match cfg_opt {
            Some(path) => res!(ServerConfig::load(&path.as_ref())),
            None => {
                info!("No path to config file supplied: using default config.");
                ServerConfig::default()
            },
        };

        // Check configuration.
        res!(cfg.check_and_fix());

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

        Ok(Self {
            cfg,
            schms,
        })
    }

}

pub struct ShieldParams<
    const BIDL: usize,
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    BID:    NumIdDat<BIDL> + 'static,
    MID:    NumIdDat<MIDL>,
    SID:    NumIdDat<SIDL> + 'static,
    UID:    NumIdDat<UIDL>,
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
	POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
> {
    _code_template:     [u8; C],
    _mid_template:      IdDat<MIDL, MID>,
    _sid_template:      IdDat<SIDL, SID>,
    _uid_template:      IdDat<UIDL, UID>,
    pub bid:            IdDat<BIDL, BID>,
    pub schms_input:    WireSchemesInput<WENC, WCS, POWH, SGN, HS>,
}

impl<
    const BIDL: usize,
    const C: usize,
    const MIDL: usize,
    const SIDL: usize,
    const UIDL: usize,
    BID:    NumIdDat<BIDL> + 'static,
    MID:    NumIdDat<MIDL>,
    SID:    NumIdDat<SIDL> + 'static,
    UID:    NumIdDat<UIDL>,
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
	POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    ShieldParams<BIDL, C, MIDL, SIDL, UIDL, BID, MID, SID, UID, WENC, WCS, POWH, SGN, HS>
{
    pub fn new(
        _code_template: [u8; C],
        _mid_template:  IdDat<MIDL, MID>,
        _sid_template:  IdDat<SIDL, SID>,
        _uid_template:  IdDat<UIDL, UID>,
        bid:            IdDat<BIDL, BID>,
        enc:            Option<WENC>,
        csum:           Option<WCS>,
        powh:           Option<POWH>,
        sign:           Option<SGN>,
        hsenc:          Option<HS>,
        chnk:           Option<ChunkConfig>,
        syntax:         SyntaxRef,
    )
        -> Self
    {
        Self {
            _code_template,
            _mid_template,
            _sid_template,
            _uid_template,
            bid,
            schms_input: WireSchemesInput {
                enc:    Alt::from(enc),
                csum:   Alt::from(csum),
                powh:   Alt::from(powh),
                sign:   Alt::from(sign),
                hsenc:  Alt::from(hsenc),
                chnk,
                syntax,
            },
        }
    }
}

pub struct Shield<
    const C: usize,
    const SIDL: usize,
    SID:    NumIdDat<SIDL>,
	WENC:   Encrypter + 'static,             // Symmetric encryption of data on the wire.               
	WCS:    Checksummer + 'static,// Checks integrity of data on the wire.                   
	POWH:   Hasher + 'static,     // Packet validation proof of work hasher.                 
	SGN:    Signer + 'static,                // Digitally signs wire packets.                           
	HS:     Encrypter + 'static,             // Asymmetric encryption of symmetric encryption key during
> {
    pub addr:       IpAddr,
    pub addr_guard: Arc<AddressGuard<
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
    pub chan_in:    Simplex<ServerMsg>,
    pub port:       u16,
    pub protocol:   Protocol<WENC, WCS, POWH, SGN, HS>,
    pub sentinel:   Sentinel,
    pub socket:     UdpSocket,
    pub user_guard: Arc<UserGuard<
                        { constant::UGRD_SHARDMAP_INIT_SHARDS },
                        BTreeMap<
                            HashForm,
                            UserLog<UserData<SIDL, C, SID>>,
                        >,
                        HashScheme,
                        { constant::GUARD_SHARDMAP_SALT_LEN },
                        UserData<SIDL, C, SID>,
                    >>,
}

impl<
    const C: usize,
    const SIDL: usize,
    SID:    NumIdDat<SIDL> + 'static,
	WENC:   Encrypter + 'static,
	WCS:    Checksummer + 'static,
	POWH:   Hasher + 'static,
	SGN:    Signer + 'static,
	HS:     Encrypter + 'static,
>
    Shield<C, SIDL, SID, WENC, WCS, POWH, SGN, HS>
{
    pub fn params<
        const BIDL: usize,
        const MIDL: usize,
        const UIDL: usize,
        BID: NumIdDat<BIDL> + 'static,
        MID: NumIdDat<MIDL>,
        UID: NumIdDat<UIDL>,
    >(
        code_template:  [u8; C],
        mid_template:   IdDat<MIDL, MID>,
        sid_template:   IdDat<SIDL, SID>,
        uid_template:   IdDat<UIDL, UID>,
        bid:    IdDat<BIDL, BID>,
        enc:    Option<WENC>,
        csum:   Option<WCS>,
        powh:   Option<POWH>,
        sign:   Option<SGN>,
        hsenc:  Option<HS>,
        chnk:   Option<ChunkConfig>,
        syntax: SyntaxRef,
    )
        -> ShieldParams<BIDL, C, MIDL, SIDL, UIDL, BID, MID, SID, UID, WENC, WCS, POWH, SGN, HS>
    {
        ShieldParams::new(
            code_template,
            mid_template,
            sid_template,
            uid_template,
            bid,
            enc,
            csum,
            powh,
            sign,
            hsenc,
            chnk,
            syntax,
        )
    }

    pub fn new<
        const BIDL: usize,
        const MIDL: usize,
        const UIDL: usize,
        BID:    NumIdDat<BIDL> + 'static,
        MID:    NumIdDat<MIDL>,
        UID:    NumIdDat<UIDL>,
        P:      AsRef<Path>,
    >(
        cfg_opt:    Option<P>,
        params:     ShieldParams<BIDL, C, MIDL, SIDL, UIDL, BID, MID, SID, UID, WENC, WCS, POWH, SGN, HS>,
    )
        -> Outcome<(
            Shield<C, SIDL, SID, WENC, WCS, POWH, SGN, HS>,
            ServerBot<BIDL, C, MIDL, SIDL, UIDL, BID, MID, SID, UID, WENC, WCS, POWH, SGN, HS>,
        )>
    {
        let protocol = res!(Protocol::new(cfg_opt, params.schms_input));

        let port                = protocol.cfg.server_port_udp;
        let server_ip_addr      = res!(local_ip());
        let server_sock_addr    = SocketAddr::new(server_ip_addr.clone(), port);

        info!("Server ip address = {}", server_ip_addr);

        let server_sock = res!(UdpSocket::bind(server_sock_addr));

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

        let rxenv = RxEnv {
            timer:          Arc::new(RwLock::new(RingTimer::<{ constant::REQ_TIMER_LEN }>::default())),
            agrd:           agrd.clone(),
            ugrd:           ugrd.clone(),
            // Packet validation.
            packval:        PacketValidator {
                                pow: Some(res!(ProofOfWork::new(protocol.schms.powh.clone()))),
                                sig: Some(protocol.schms.sign.clone()),
                            },
            gpzparams:      DifficultyParams {
                                profile:    constant::POW_DIFFICULTY_PROFILE,
                                max:        constant::POW_MAX_ZERO_BITS,
                                min:        constant::POW_MIN_ZERO_BITS,
                                rps_max:    constant::MAX_ALLOWED_AVG_REQ_PER_SEC,
                            },
            // Message assembly.
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
            // Policy configuration.
            pow_time_horiz: constant::POW_TIME_HORIZON_SEC,
            accept_unknown: true,
        };

        // Create a semaphore and a sentinel to maintain awareness of the server thread state, and
        // to exercise some control over it.  The semaphore is given to the ServerBot and the user
        // keeps the sentinel.
        let (sem, sentinel) = thread_channel();

        let chan_in = simplex::<ServerMsg>();

        let server = ServerBot {
            id:         params.bid,
            protocol:   protocol.clone(),
            sem,
            errc:       Arc::new(Mutex::new(0)),
            inited:     false,
            chan_in:    chan_in.clone(),
            rxenv,
            pack_size:  constant::UDP_BUFFER_SIZE,
            sock:       res!(server_sock.try_clone()),
            sock_addr:  server_sock_addr.clone(),
            ma_gc_last: Instant::now(),
            ma_gc_int:  Duration::from_secs(300),
            phantom1:   PhantomData,
            phantom2:   PhantomData,
        };

        Ok((
            Shield {
                addr:       server_ip_addr,
                addr_guard: agrd,
                chan_in,
                port,
                protocol,
                socket:     server_sock,
                sentinel,
                user_guard: ugrd,
            },
            server,
        ))
    }

    pub fn start<
        const BIDL: usize,
        const MIDL: usize,
        const UIDL: usize,
        BID:    NumIdDat<BIDL> + 'static,
        MID:    NumIdDat<MIDL> + 'static,
        UID:    NumIdDat<UIDL> + 'static,
    >(
        &self,
        mut server: ServerBot<
            BIDL, C, MIDL, SIDL, UIDL, BID, MID, SID, UID, WENC, WCS, POWH, SGN, HS,
        >,
    )
        -> Outcome<()>
    {
        res!(server.init());
        res!(server.start(self.sentinel.clone()));
        Ok(())
    }

    pub fn msg_builder_default (
        &self,
        code:   [u8; C],
        zbits:  u16,
    )
        -> Outcome<MsgBuilder<
            HasherDefAlt<HashScheme, POWH>,
            {constant::POW_PREFIX_LEN},
            {constant::POW_PREIMAGE_LEN},
            PowPristine<
                C,
                {constant::POW_PREFIX_LEN},
                {constant::POW_PREIMAGE_LEN},
            >,
            SignerDefAlt<SignatureScheme, SGN>,
        >>
    {
        let src_addr            = self.addr.clone();
        let server_sock_addr    = SocketAddr::new(src_addr.clone(), self.port);
        let server_sock         = res!(self.socket.try_clone());

        let pristine = PowPristine::<
            C,
            {constant::POW_PREFIX_LEN},
            {constant::POW_PREIMAGE_LEN},
        >{
            code,
            src_addr,
            timestamp: res!(SystemTime::now().duration_since(UNIX_EPOCH)),
            time_horiz: constant::POW_TIME_HORIZON_SEC,
        };
        trace!("POW Pristine tx:");
        res!(pristine.trace());

        Ok(MsgBuilder {
            chunk_cfg: ChunkConfig {
                threshold_bytes:    1_500,
                chunk_size:         1_000,
                dat_wrap:           false,
                pad_last:           true,
            },
            src_sock: res!(server_sock.try_clone()),
            trg_addr: server_sock_addr,
            validator: PacketValidator {
                pow: Some(res!(ProofOfWork::new(self.protocol.schms.powh.clone()))),
                sig: Some(self.protocol.schms.sign.clone()),
            },
            powparams: PowCreateParams {
                pvars: PowVars {
                    zbits,
                    pristine,
                },
                time_lim: constant::POW_CREATE_TIMEOUT,
                count_lim: constant::POW_CREATE_COUNT_LIM,
            },
        })
    }

}
