use crate::{
    constant,
    msg::syntax,
    //packet::PacketValidator,
    //schemes::{
    //    WireSchemes,
    //},
};

use oxedize_fe2o3_core::{
    prelude::*,
    //alt::DefAlt,
};
use oxedize_fe2o3_crypto::sign::SignatureScheme;
//use oxedize_fe2o3_iop_crypto::{
//    sign::{
//        Signer,
//        SignerDefAlt,
//    },
//    enc::Encrypter,
//};
use oxedize_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
    chunk::{
        Chunker,
        ChunkConfig,
    },
    //file::JdatFile,
};
use oxedize_fe2o3_hash::{
    hash::{
        HashScheme,
        //HasherDefAlt,
    },
    //pow::ProofOfWork,
};
//use oxedize_fe2o3_iop_hash::{
//    api::Hasher,
//    csum::Checksummer,
//};
use oxedize_fe2o3_syntax::core::SyntaxRef;

use std::{
    collections::BTreeMap,
    sync::Arc,
};


#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct ShieldConfig {
    // Schemes
    pub schemes_db_path:                String,
    // Chunking
    pub wire_chunk_threshold:           u64, // applies only to values
    pub wire_chunk_bytes:               u64,
    // Server
    pub server_address:                 String,
    pub server_port_udp:                u16,
    pub server_port_tcp:                u16,
    pub server_rps_zbits_profile:       u8, // 0 = linear, ..
    pub server_pow_zbits_min:           u16, // min zero bits for all packet pows
    pub server_pow_zbits_max:           u16, // when rps reaches max, the reqd pow zbit reaches this level
    pub server_pow_time_horiz_secs:     u64, // timestamp must be no older in seconds to now
    pub server_rps_max:                 u16, // the requests per second corresponding to maximum pow zbits
    //pub packet_pow_hash_scheme:         String,
    //pub packet_signature_scheme:        String,
    pub addr_guard_map_bins:            u32, // Number of bins in shared map of incoming addresses.
    pub user_guard_map_bins:            u32, // Number of bins in shared map of users.
    pub msg_assembler_map_bins:         u32, // Number of bins in shared map of message pieces.
    // Server policy
    // An attacker can flood us with HReq1 messages with random uids and public keys.  A reasonable
    // defence is to set a relatively high difficulty for HReq1.
    pub server_accept_unknown_users:    bool,
}

impl Config for ShieldConfig {

    fn check_and_fix(&mut self) -> Outcome<()> {
        // Checks that read only.
        res!(self.check_wire_chunk_config(&self.chunk_config()));
        Ok(())
    }
}

impl Default for ShieldConfig {
    fn default() -> Self {
        Self {
            // Schemes
            schemes_db_path:                fmt!("../oxedize_fe2o3_namex/namex.jdat"),
            // Chunking
            wire_chunk_threshold:           1_500,
            wire_chunk_bytes:               1_000,
            // Server
            server_address:                 fmt!("127.0.0.1"),
            server_port_udp:                60000, // numeric keypad mapping for "o3db"
            server_port_tcp:                60000,
            server_rps_zbits_profile:       0, // 0 = linear, ..
            server_pow_zbits_min:           2, // all packets must have a proof of work with at least this many zero bits
            server_pow_zbits_max:           15, // when rps reaches max, the reqd pow zbit reaches this level
            server_pow_time_horiz_secs:     600, // timestamp must be no older in seconds to now
            server_rps_max:                 30_000, // the requests per second corresponding to maximum pow zbits
            //packet_pow_hash_scheme:         fmt!("Seahash"),
            //packet_signature_scheme:        fmt!("Ed25519"), // try SIKE
            addr_guard_map_bins:            128, // arbitrary
            user_guard_map_bins:            128, // arbitrary
            msg_assembler_map_bins:         128, // arbitrary
            // Server policy.
            server_accept_unknown_users:    false,
        }
    }
}

impl ShieldConfig {

    pub fn try_default() -> Outcome<Self> {
        Ok(Self::default())
    }

    pub fn syntax_default() -> Outcome<SyntaxRef> {
        let syntax = SyntaxRef(Arc::new(res!(syntax::build())));
        Ok(syntax)
    }

    /// Hard-wired default proof of work hash scheme.
    pub fn default_packet_pow_hash_scheme() -> Option<HashScheme> {
        Some(HashScheme::new_seahash())
    }

    /// Hard-wired default signature scheme.
    pub fn default_packet_signature_scheme() -> Option<SignatureScheme> {
        Some(SignatureScheme::empty_ed25519())
    }

    //// Data scheme updaters.
    //pub fn update_packet_validator<
	//    WENC: Encrypter,
	//    WCS: Checksummer,
    //    POWH: Hasher,
	//    SGN: Signer,
	//    HS: Encrypter,
    //    //// Proof of work validation.
    //    //const N: usize, // Pristine + Nonce size.
    //    //const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
    //    //const P1: usize, // Length of pristine bytes (i.e. included in artefact).
    //    //PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
    //>(
    //    &self,
    //    wschms: &WireSchemes<WENC, WCS, POWH, SGN, HS>,
    //)
    //    -> Outcome<PacketValidator<
    //        HasherDefAlt<HashScheme, POWH>,
    //        SignerDefAlt<SignatureScheme, SGN>,
    //    >>
    //{
    //    let pow_def_alt = res!(Self::read_hash_scheme(
    //        &self.packet_pow_hash_scheme,
    //        &*wschms.powh,
    //        Self::default_packet_pow_hash_scheme,
    //        "packet_pow_hash_scheme",
    //    ));
    //    let sig_def_alt = res!(Self::read_signature_scheme(
    //        &self.packet_signature_scheme,
    //        &*wschms.sign,
    //        Self::default_packet_signature_scheme,
    //        "packet_signature_scheme",
    //    ));
    //    Ok(PacketValidator { 
    //        pow: match pow_def_alt {
    //            DefAlt::Given(..) | DefAlt::Default(..) =>
    //                Some(ProofOfWork::new(HasherDefAlt(pow_def_alt))),
    //            DefAlt::None => None,
    //        },
    //        sig: match sig_def_alt {
    //            DefAlt::Given(..) | DefAlt::Default(..) =>
    //                Some(SignerDefAlt(sig_def_alt)),
    //            DefAlt::None => None,
    //        },
    //    })
    //}

    /// Return a rest chunk configuration from the database configuration.
    pub fn chunk_config(&self) -> ChunkConfig {
        ChunkConfig {
            threshold_bytes:    self.wire_chunk_threshold as usize,
            chunk_size:         self.wire_chunk_bytes as usize, 
            dat_wrap:           false,
            pad_last:           true,
        }
    }

    pub fn chunker(cfg: ChunkConfig) -> Chunker {
        Chunker::default().set_config(cfg)
    }

    pub fn wire_chunk_size(&self)           -> usize { self.wire_chunk_bytes as usize }
    pub fn wire_chunking_threshold(&self)   -> usize { self.wire_chunk_threshold as usize }

    pub fn new_chunk_cfg(
        threshold_bytes:    usize,
        chunk_size:         usize,
        dat_wrap:           bool,
        pad_last:           bool,
    )
        -> ChunkConfig
    {
        ChunkConfig {
            threshold_bytes,
            chunk_size,
            dat_wrap,
            pad_last,
        }
    }

    pub fn chunker_default(&self) -> ChunkConfig {
        ChunkConfig {
            threshold_bytes:    self.wire_chunk_threshold as usize,
            chunk_size:         self.wire_chunk_bytes as usize,
            dat_wrap:           false,
            pad_last:           true,
        }
    }

    pub fn check_wire_chunk_config(&self, chunk_cfg: &ChunkConfig) -> Outcome<()> {
        if chunk_cfg.chunk_size > u16::MAX as usize {
            return Err(err!(
                "Chunk size of {} is less than the current minimum of {}.",
                chunk_cfg.chunk_size, constant::MIN_CHUNK_SIZE;
            TooBig, Configuration));
        }
        Ok(())
    }

    pub fn dump(self) -> Outcome<()> {
        let dat = Self::to_datmap(self);
        for line in dat.to_lines("    ", true) {
            info!("{}", line);
        }
        Ok(())
    }

    // Servers.
    pub fn addr_guard_map_bins(&self) -> u32 {
        self.addr_guard_map_bins
    }

    pub fn user_guard_map_bins(&self) -> u32 {
        self.user_guard_map_bins
    }

    pub fn msg_assembler_map_bins(&self) -> u32 {
        self.msg_assembler_map_bins
    }

    ///// Build the `PacketValidator`
    //pub fn packet_validator<
    //    // Proof of work validator.
    //    H: Hasher + Send + 'static, // Proof of work hasher.
    //    const N: usize, // Pristine + Nonce size.
    //    const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
    //    const P1: usize, // Length of pristine bytes (i.e. included in artefact).
    //    PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
    //    // Digital signature validation.
    //    S: Signer,
    //>(
    //    &self,
    //    pow_opt:    Option<(ProofOfWork<HasherDefAlt<HashScheme, POWH>>, PowParams<P0, P1, PRIS>)>,
    //    sigpk_opt:  Option<Vec<u8>>,
    //)
    //    -> Outcome<PacketValidator>
    //{
    //    Ok(PacketValidator {
    //        pow: match self.packet_pow_hash_schemevalidation {
    //            true => match pvars_opt {
    //                Some(pvars) => (
    //                    ProofOfWork::new(HashScheme::new_seahash()),
    //                    PowParams {
    //                        pvars,
    //                        time_lim: constant::POW_CREATE_TIMEOUT,
    //                        count_lim: constant::POW_CREATE_COUNT_LIM,
    //                    },
    //                ),
    //                None => return Err(err!(
    //                    "The configuration currently requires a proof of work validator, \
    //                    but the PowVars supplied is None.",
    //                ), Bug, Configuration)),
    //            },
    //            false => None,
    //        },
    //        sig: match self.packet_sign_validation {
    //            true => match sigpk_opt {
    //                Some(sigpk) => (
    //                    ProofOfWork::new(HashScheme::new_seahash()),
    //                    PowParams {
    //                        pvars,
    //                        time_lim: constant::POW_CREATE_TIMEOUT,
    //                        count_lim: constant::POW_CREATE_COUNT_LIM,
    //                    },
    //                ),
    //                None => return Err(err!(
    //                    "The configuration currently requires a proof of work validator, \
    //                    but the PowVars supplied is None.",
    //                ), Bug, Configuration)),

    //            },
    //            false => None,
    //        }
    //    })
    //}
}
