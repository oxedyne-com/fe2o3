use crate::srv::{
    schemes::{
        WireSchemes,
        WireSchemeTypes,
    },
    msg::{
        packet::{
            PacketChunkState,
            PacketCount,
            PacketMeta,
            PacketValidator,
        },
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::{
        ToBytes,
        ToByteArray,
    },
    map::MapMut,
    rand::RanDef,
};
use oxedize_fe2o3_iop_crypto::sign::Signer;
use oxedize_fe2o3_jdat::{
    chunk::{
        Chunker,
        ChunkConfig,
    },
    id::{
        IdDat,
        NumIdDat,
    },
    version::SemVer,
};
use oxedize_fe2o3_hash::{
    map::ShardMap,
    pow::{
        PowCreateParams,
        Pristine,
    },
};
use oxedize_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};

use std::{
    clone::Clone,
    collections::BTreeMap,
    fmt::Debug,
    net::{
        SocketAddr,
        UdpSocket,
    },
    sync::RwLock,
    time::{
        Duration,
        Instant,
    },
};


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

pub struct MsgBuilder<
    // Proof of work validator.
    H: Hasher + Send + 'static, // Proof of work hasher.
    const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
    const P1: usize, // Length of pristine bytes (i.e. included in artefact).
    PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
    // Digital signature validation.
    S: Signer,
> {
    // Structure
    pub chunk_cfg:  ChunkConfig,
    // Delivery
    pub src_sock:   UdpSocket,
    pub trg_addr:   SocketAddr,
    // Validation
    pub validator:  PacketValidator<H, S>,
    pub powparams:  PowCreateParams<P0, P1, PRIS>,
}

pub struct Message;

impl Message {

    /// Breaks the message into equally sized payload chunks, prepends a `PacketMeta` and appends
    /// the specified set of `PacketValidator`s.
    pub fn create<
        const MIDL: usize,
        const UIDL: usize,
        MID: NumIdDat<MIDL>,
        UID: NumIdDat<UIDL>,
        // Proof of work validator.
        H: Hasher + Send + 'static, // Proof of work hasher.
        const N: usize, // Pristine + Nonce size.
        const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
        const P1: usize, // Length of pristine bytes (i.e. included in artefact).
        PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
        // Digital signature validation.
        S: Signer,
    >(
        msg_name:   &'static str, // for debugging only
        msg_byts:   Vec<u8>,
        // Metadata
        typ:        MsgType,
        ver:        &SemVer,
        uid:        IdDat<UIDL, UID>,
        //tstamp:     u64,
        // Structure
        chunker:    &Chunker,
        // Validation
        validator:  &PacketValidator<H, S>,
        powparams:  &PowCreateParams<P0, P1, PRIS>,
        inc_sigpk:  bool,
    )
        -> Outcome<(Vec<Vec<u8>>, Option<String>)>
    {
        trace!("{:?}", chunker);
        let size = chunker.cfg.chunk_size;
        let meta_len = PacketMeta::<MIDL, UIDL, MID, UID>::BYTE_LEN;
        let warning = if 2 * meta_len > size {
            Some(errmsg!("Message meta of length {} bytes is more than half \
                the specified packet size of {}. Consider increasing the \
                packet size.", meta_len, size,
            ))
        } else {
            None
        };

        let msg_len = msg_byts.len();
        let (mut chunks, _) = res!(chunker.chunk(&msg_byts));
        let nc = chunks.len();
        if nc > PacketCount::MAX as usize {
            return Err(err!("Message type {} of length {} bytes, \
                when broken into chunks of {} bytes creates {} packets, \
                exceeding the limit of {}.  Reduce the message length or \
                increase the packet size.",
                msg_name, msg_byts.len(), size, nc, PacketCount::MAX;
            Invalid, Configuration));
        }

        let mut packets = Vec::new();
        let mid = IdDat::<MIDL, MID>::randef();
        for i in 0..nc {
            let chunk_len = chunks[i].len();
            let chnk = PacketChunkState {
                index:      res!(i.try_into()),
                num_chunks: res!(nc.try_into()),
                //chunk_size: res!(chunker.config().chunk_size().try_into()),
                chunk_size: res!(chunk_len.try_into()),
                pad_last:   chunker.cfg.pad_last,
            };
            let meta = PacketMeta {
                typ,
                ver: *ver,
                mid,
                uid,
                chnk,
                //tstamp,
            };
            // 1. Header
            let mut packet = res!(meta.to_bytes(Vec::new()));
            let meta_len = packet.len();
            // 2. Data chunk
            packet.append(&mut chunks[i]);
            let len = packet.len();
            // 3. Validators
            packet = res!(validator.to_bytes::<N, P0, P1, PRIS>(
                packet,
                powparams,
                //powparams.clone(),
                inc_sigpk,
            ));
            let validator_len = packet.len() - len;
            trace!("Packet {} lengths: msg {}, meta {} chunk {} valid {} total {}",
                i, msg_len, meta_len, chunk_len, validator_len, packet.len(),
            );
            trace!("  Chunk:      {}", chunks[i].len());
            packets.push(packet);
        }
    
        Ok((packets, warning))
    }

}

/// Rather than a generic and possibly more complex callback mechanism, the processing of server
/// command is customised so as to access only parameters needed from the server loop scope.
/// Incoming server commands are encoded in a `oxedize_fe2o3_syntax::msg::MsgCmd` using the `Syntax`
/// defined in `oxedize_fe2o3_shield::syntax`.  Each must be associated with a `struct` below that
/// is accessed in `oxedize_fe2o3_o3db::bots::bot_server`.  This must capture some basic information (i.e.
/// `MsgFmt` and `MsgIds`) as well as the command-specific data.  The associated `struct` must have
/// its own custom method for processing the incoming command (e.g.
/// `oxedize_fe2o3_o3db::comm::wire::HReq1::process`), and should implement `ShieldCommand` in order to
/// access supporting methods.  There are plenty of examples to copy and modify.
pub trait ShieldCommand<
    const ML: usize,
    const SL: usize,
    const UL: usize,
    ID: IdTypes<ML, SL< UL>,
>:
    Default
    + IdentifiedMessage
    + IntoBytes
{
    fn fmt(&self)       -> &MsgFmt;
    fn pow(&self)       -> &MsgPow;
    fn mid(&self)       -> &MsgIds<SL, UL, ID::S, ID::U>;
    fn syntax(&self)    -> &SyntaxRef   { &self.fmt().syntax }
    fn encoding(&self)  -> &Encoding    { &self.fmt().encoding }
    fn uid(&self)       -> ID::U        { self.mid().uid.clone() }
    fn sid_opt(&self)   -> Option<ID::S> {
        self.mid().sid_opt.as_ref().clone().copied()
    }
    fn pow_zbits(&self) -> ZeroBits { self.pow().zbits }
    fn pad_last(&self)  -> bool     { true }
    //fn pow_code(&self)  -> Option<[u8; constant::POW_CODE_LEN]> { self.pow().code }
    fn inc_sigpk(&self) -> bool; // Include signature public key in outgoing validator?
    fn deconstruct(&mut self, _mcmd: &mut SyntaxMsgCmd) -> Outcome<()> { Ok(()) }
    fn construct(self)  -> Outcome<SyntaxMsg>;

    fn build<
        // Proof of work validator.
        const N: usize,         // Pristine + Nonce size.
        const P0: usize,        // Length of pristine prefix bytes (i.e. not included in artefact).
        const P1: usize,        // Length of pristine bytes (i.e. included in artefact).
        PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
        W: WireSchemeTypes,     // Contains the chunker, the pow hasher and the signer.
    >(
        self,
        src_addr:   IpAddr,
        trg_addr:   IpAddr,
        schms:      WireSchemes<W>,
    )
        -> Outcome<Vec<Vec<u8>>>
    {
        // Copy some self parameters before consumption by into_bytes
        let msg_name = self.name();
        let msg_typ = self.typ();
        //let uid = res!(self.uid().to_bytes(Vec::new()));
        let inc_sigpk = self.inc_sigpk();
        let pad_last = self.pad_last();

        let uid = self.uid().clone();

        let msg_byts = res!(self.into_bytes(Vec::new()));
        //let tstamp = res!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)).as_secs();

        let pristine = PowPristine::<C, P0, P1> {
            code,
            src_addr,
            timestamp: res!(SystemTime::now().duration_since(UNIX_EPOCH)),
            time_horiz: constant::POW_TIME_HORIZON_SEC,
        };
        trace!("POW Pristine tx:");
        res!(pristine.trace());

        let validator = PacketValidator {
            pow: Some(res!(ProofOfWork::new(schms.powh.clone()))),
            sig: Some(schms.sign.clone()),
        },
        powparams: PowCreateParams {
            pvars: PowVars {
                zbits,
                pristine,
            },
            time_lim: constant::POW_CREATE_TIMEOUT,
            count_lim: constant::POW_CREATE_COUNT_LIM,
        },

        let (packets, warning) = res!(Message::create::<
            MIDL,
            UIDL,
            MID,
            UID,
            H,
            {constant::POW_INPUT_LEN},
            P0,
            P1,
            PRIS,
            S,
        >(
            msg_name,
            msg_byts,
            // Metadata
            msg_typ,
            &constant::VERSION,
            uid,
            //tstamp,
            &ServerConfig::chunker(builder.chunk_cfg.clone().set_pad_last(pad_last)),
            &builder.validator,
            &builder.powparams,
            inc_sigpk,
        ));

        let chunker = ServerConfig::chunker(builder.chunk_cfg.set_pad_last(self.pad_last()));
        trace!("{:?}", chunker);
        let size = chunker.cfg.chunk_size;
        let meta_len = PacketMeta::<MIDL, UIDL, MID, UID>::BYTE_LEN;
        let warning = if 2 * meta_len > size {
            Some(errmsg!("Message meta of length {} bytes is more than half \
                the specified packet size of {}. Consider increasing the \
                packet size.", meta_len, size,
            ))
        } else {
            None
        };

        let msg_len = msg_byts.len();
        let (mut chunks, _) = res!(chunker.chunk(&msg_byts));
        let nc = chunks.len();
        if nc > PacketCount::MAX as usize {
            return Err(err!("Message type {} of length {} bytes, \
                when broken into chunks of {} bytes creates {} packets, \
                exceeding the limit of {}.  Reduce the message length or \
                increase the packet size.",
                msg_name, msg_byts.len(), size, nc, PacketCount::MAX;
            Invalid, Configuration));
        }

        let mut packets = Vec::new();
        let mid = IdDat::<MIDL, MID>::randef();
        for i in 0..nc {
            let chunk_len = chunks[i].len();
            let chnk = PacketChunkState {
                index:      res!(i.try_into()),
                num_chunks: res!(nc.try_into()),
                //chunk_size: res!(chunker.config().chunk_size().try_into()),
                chunk_size: res!(chunk_len.try_into()),
                pad_last:   chunker.cfg.pad_last,
            };
            let meta = PacketMeta {
                typ,
                ver: *ver,
                mid,
                uid,
                chnk,
                //tstamp,
            };
            // 1. Header
            let mut packet = res!(meta.to_bytes(Vec::new()));
            let meta_len = packet.len();
            // 2. Data chunk
            packet.append(&mut chunks[i]);
            let len = packet.len();
            // 3. Validators
            packet = res!(validator.to_bytes::<N, P0, P1, PRIS>(
                packet,
                powparams,
                //powparams.clone(),
                inc_sigpk,
            ));
            let validator_len = packet.len() - len;
            trace!("Packet {} lengths: msg {}, meta {} chunk {} valid {} total {}",
                i, msg_len, meta_len, chunk_len, validator_len, packet.len(),
            );
            trace!("  Chunk:      {}", chunks[i].len());
            packets.push(packet);
        }


        if let Some(warning) = warning {
            warn!("{}", warning);
        }
        Ok(packets)
    }

    fn send_udp(
        src_sock:   &UdpSocket,
        trg_addr:   &SocketAddr,
        packets:    Vec<Vec<u8>>,
    )
        -> Outcome<()>
    {
        for packet in packets {
            res!(src_sock.send_to(&packet, &trg_addr));
        }
        Ok(())
    }

    fn send<
        const C: usize,
    >(
        self,
        trg_addr:   &SocketAddr,
        src:        Arc<UdpSocket>,
        packets:    Vec<Vec<u8>>,
        code:       [u8; C],
    )
        -> Outcome<()>
    {
        let src_addr = res!(src.local_addr()).ip();
        let packets = res!(self.build::<
            C,
            {constant::POW_PREFIX_LEN},
            {constant::POW_PREIMAGE_LEN},
        >(
            src_addr,
            trg_addr.ip(),
        ));
        for packet in packets {
            res!(src.send_to(&packet, trg_addr));
        }
        Ok(())
    }
}
