use crate::srv::{
    cfg::ServerConfig,
    constant,
    guard::{
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
        assemble::{
            MsgAssembler,
            MsgAssemblyParams,
            MsgState,
        },
        core::{
            DefaultIdTypes,
            IdTypes,
        },
        packet::PacketValidator,
    },
    pow::DifficultyParams,
    schemes::{
        DefaultWireSchemes,
        WireSchemes,
        WireSchemesInput,
        WireSchemeTypes,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
};
use oxedyne_fe2o3_crypto::{
    sign::SignatureScheme,
};
use oxedyne_fe2o3_hash::{
    hash::{
        HasherDefAlt,
        HashScheme,
    },
    map::ShardMap,
    pow::ProofOfWork,
};
use oxedyne_fe2o3_data::ring::RingTimer;
use oxedyne_fe2o3_iop_crypto::sign::SignerDefAlt;
use oxedyne_fe2o3_iop_hash::api::HashForm;
use oxedyne_fe2o3_net::guard::addr::{
    AddressGuard,
    AddressLog,
};

use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc,
        RwLock,
    },
    time::Duration,
};


/// Operating mode of a protocol instance, selecting production, development or
/// test behaviour.
#[derive(Clone, Debug)]
pub enum ProtocolMode {
    /// Live production deployment.
    Production,
    /// Local development.
    Dev,
    /// Automated testing.
    Test,
}

/// Bundles the identifier and wire-scheme type families used to parameterise a
/// [`Protocol`].
pub trait ProtocolTypes<
    const ML: usize,
    const SL: usize,
    const UL: usize,
>:
    Clone
    + fmt::Debug
{
    /// Identifier types for messages, sessions and users.
    type ID: IdTypes<ML, SL, UL>;
    /// Cryptographic wire-scheme types.
    type W: WireSchemeTypes;
}

/// Default [`ProtocolTypes`] binding using the standard identifier and
/// wire-scheme families.
#[derive(Clone, Debug, Default)]
pub struct DefaultProtocolTypes<
    const ML: usize,
    const SL: usize,
    const UL: usize,
>;

impl<
    const ML: usize,
    const SL: usize,
    const UL: usize,
>
    ProtocolTypes<ML, SL, UL> for DefaultProtocolTypes<ML, SL, UL>
    where DefaultIdTypes<ML, SL, UL>: IdTypes<ML, SL, UL>,
{
    type ID = DefaultIdTypes<ML, SL, UL>;
    type W = DefaultWireSchemes;
}

/// Capture all necessary information, and nothing more, allowing a thread to process an incoming
/// packet.  Rather than pass the entire struct atomically, use multiple interior atomic references
/// to reduce sharing wait times.
#[derive(Clone, Debug)]
pub struct Protocol<
    const C: usize,
    const ML: usize,
    const SL: usize,
    const UL: usize,
    P: ProtocolTypes<ML, SL, UL>,
> {
    // Let user define these generic parameters.
    _code_template:     [u8; C],
    _mid_template:      <P::ID as IdTypes<ML, SL, UL>>::M,
    _sid_template:      <P::ID as IdTypes<ML, SL, UL>>::S,
    _uid_template:      <P::ID as IdTypes<ML, SL, UL>>::U,

    /// Operating mode of this protocol instance.
    pub mode:           ProtocolMode,
    /// Cryptographic schemes applied to the wire.
    pub schms:          WireSchemes<P::W>,

    /// Ring timer tracking recent request timestamps for rate estimation.
    pub timer:          Arc<RwLock<RingTimer<{ constant::REQ_TIMER_LEN }>>>,
    // Address protection.
    /// Per-address guard enforcing rate limiting and blacklisting.
    pub agrd:           Arc<AddressGuard<
                            { constant::AGRD_SHARDMAP_INIT_SHARDS },
                            BTreeMap<
                                HashForm,
                                AddressLog<
                                    { constant::REQ_TIMER_LEN },
                                    AddressData,
                                >,
                            >,
                            HashScheme,
                            { constant::GUARD_SHARDMAP_SALT_LEN },
                            { constant::REQ_TIMER_LEN },
                            AddressData,
                        >>,
    /// Handshake request expiry window enforced by the SHIELD sequence check.
    pub hreq_exp:       Duration,
    // User protection.
    /// Per-user guard holding trust state and key material.
    pub ugrd:           Arc<UserGuard<
                            { constant::UGRD_SHARDMAP_INIT_SHARDS },
                            BTreeMap<
                                HashForm,
                                UserLog<UserData<SL, C, <P::ID as IdTypes<ML, SL, UL>>::S>>,
                            >,
                            HashScheme,
                            { constant::GUARD_SHARDMAP_SALT_LEN },
                            UserData<SL, C, <P::ID as IdTypes<ML, SL, UL>>::S>,
                        >>,
    // Packet validation.
    /// Validator applying proof-of-work and signature checks to packets.
    pub packval:        PacketValidator<
                            HasherDefAlt<HashScheme, <P::W as WireSchemeTypes>::POWH>,
                            SignerDefAlt<SignatureScheme, <P::W as WireSchemeTypes>::SGN>,
                        >,
    /// Parameters governing the global proof-of-work difficulty curve.
    pub gpzparams:      DifficultyParams,
    // Message assembly.
    /// Reassembles multi-packet messages from incoming chunks.
    pub massembler:     Arc<MsgAssembler<
                            { constant::MSG_ASSEMBLY_SHARDS },
                            BTreeMap<HashForm, MsgState>,
                            HashScheme,
                            { constant::GUARD_SHARDMAP_SALT_LEN },
                        >>,
    /// Limits and timeouts applied during message assembly.
    pub ma_params:      MsgAssemblyParams,
    // Policy configuration.
    /// Window, in seconds, within which a proof-of-work timestamp is valid.
    pub pow_time_horiz: u64,
    /// Whether to accept packets from previously unknown users.
    pub accept_unknown: bool,
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
    /// Constructs a protocol instance from server configuration and wire
    /// schemes, initialising the address guard, user guard, packet validator
    /// and message assembler with the crate's compile-time constants. The
    /// identifier and code template arguments fix the concrete generic types.
    pub fn new(
        cfg:            &ServerConfig,
        schms_input:    WireSchemesInput<P::W>,
        _code_template: [u8; C],
        _mid_template:  <P::ID as IdTypes<ML, SL, UL>>::M,
        _sid_template:  <P::ID as IdTypes<ML, SL, UL>>::S,
        _uid_template:  <P::ID as IdTypes<ML, SL, UL>>::U,
        mode:           ProtocolMode,
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
                AddressData,
            >,
        >::new();

        let agrd = Arc::new(AddressGuard {
            amap: res!(ShardMap::<
                {constant::AGRD_SHARDMAP_INIT_SHARDS},
                {constant::GUARD_SHARDMAP_SALT_LEN},
                AddressLog<
                    {constant::REQ_TIMER_LEN},
                    AddressData,
                >,
                BTreeMap::<
                    HashForm,
                    AddressLog<
                        {constant::REQ_TIMER_LEN},
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
            arps_max:       constant::MAX_ALLOWED_AVG_REQ_PER_SEC,
            // Throttle
            tint_min:       constant::THROTTLED_INTERVAL_MIN,
            tsunset_base:   Duration::from_secs(constant::ADDR_THROTTLE_SUNSET_SECS_MIN),
            tsunset_spread: Duration::from_secs(
                                constant::ADDR_THROTTLE_SUNSET_SECS_MAX
                                    .saturating_sub(constant::ADDR_THROTTLE_SUNSET_SECS_MIN)
                            ),
            blist_cnt:      constant::THROTTLE_COUNT_BEFORE_BLACKLIST,
        });

        let ugrd_map_init = BTreeMap::<
            HashForm,
            UserLog<UserData<SL, C, <P::ID as IdTypes<ML, SL, UL>>::S>>,
        >::new();

        let ugrd = Arc::new(UserGuard {
            umap: res!(ShardMap::<
                {constant::UGRD_SHARDMAP_INIT_SHARDS},
                {constant::GUARD_SHARDMAP_SALT_LEN},
                UserLog<UserData<SL, C, <P::ID as IdTypes<ML, SL, UL>>::S>>,
                BTreeMap::<
                    HashForm,
                    UserLog<UserData<SL, C, <P::ID as IdTypes<ML, SL, UL>>::S>>,
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
            mode,
            schms,
            timer:          Arc::new(RwLock::new(RingTimer::<{ constant::REQ_TIMER_LEN }>::default())),
            agrd:           agrd.clone(),
            hreq_exp:       constant::SESSION_REQUEST_EXPIRY,
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
}
