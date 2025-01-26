use crate::{
    prelude::*,
    srv::{
        constant,
        cfg::ServerConfig,
        schemes::{
            WireSchemes,
            WireSchemeTypes,
        },
        msg::{
            core::{
                IdentifiedMessage,
                IdTypes,
                MsgFmt,
                MsgIds,
                MsgPow,
            },
            packet::{
                PacketChunkState,
                PacketCount,
                PacketMeta,
                PacketValidator,
            },
        },
        pow::{
            PowPristine,
        },
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    byte::{
        Encoding,
        IntoBytes,
        ToBytes,
    },
    rand::RanDef,
};
use oxedize_fe2o3_hash::{
    pow::{
        PowCreateParams,
        PowVars,
        Pristine,
        ProofOfWork,
        ZeroBits,
    },
};
use oxedize_fe2o3_syntax::{
    SyntaxRef,
    msg::{
        Msg,
        MsgCmd,
    },
};

use std::{
    clone::Clone,
    net::{
        IpAddr,
        SocketAddr,
        UdpSocket,
    },
    sync::Arc,
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};


/// Rather than a generic and possibly more complex callback mechanism, the processing of server
/// command is customised so as to access only parameters needed from the server loop scope.
/// Incoming server commands are encoded in a `oxedize_fe2o3_syntax::msg::MsgCmd` using the `Syntax`
/// defined in `oxedize_fe2o3_shield::syntax`.  Each must be associated with a `struct` below that
/// is accessed in `oxedize_fe2o3_o3db_sync::bots::bot_server`.  This must capture some basic information (i.e.
/// `MsgFmt` and `MsgIds`) as well as the command-specific data.  The associated `struct` must have
/// its own custom method for processing the incoming command (e.g.
/// `oxedize_fe2o3_o3db_sync::comm::wire::HReq1::process`), and should implement `ShieldCommand` in order to
/// access supporting methods.  There are plenty of examples to copy and modify.
pub trait ShieldCommand<
    const ML: usize,
    const SL: usize,
    const UL: usize,
    ID: IdTypes<ML, SL, UL>,
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
    fn deconstruct(&mut self, _mcmd: &mut MsgCmd) -> Outcome<()> { Ok(()) }
    fn construct(self)  -> Outcome<Msg>;

    fn build<
        const C: usize,
        // Proof of work validator.
        const N: usize,         // Hash pre-image size = pristine + nonce sizes.
        const P0: usize,        // Length of private prefix bytes (i.e. not included in artefact).
        const P1: usize,        // Length of pristine bytes (i.e. included in artefact).
        PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
        W: WireSchemeTypes + 'static, // Contains the chunker, the pow hasher and the signer.
    >(
        self,
        src_addr:   IpAddr,
        trg_addr:   IpAddr,
        code:       [u8; C],
        schms:      WireSchemes<W>,
    )
        -> Outcome<Vec<Vec<u8>>>
    {
        // Copy some self parameters before consumption by into_bytes
        let msg_name = self.name();
        //let uid = res!(self.uid().to_bytes(Vec::new()));
        let inc_sigpk = self.inc_sigpk();
        let pad_last = self.pad_last();

        let uid = self.uid().clone();

        let zbits = self.pow_zbits();
        let typ = self.typ();
        let msg_byts = res!(self.into_bytes(Vec::new()));
        //let tstamp = res!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)).as_secs();

        let pristine = PowPristine::<C, P0, P1> {
            code,
            src_addr,
            trg_addr,
            timestamp: res!(SystemTime::now().duration_since(UNIX_EPOCH)),
            time_horiz: constant::POW_TIME_HORIZON_SEC,
        };
        trace!(log_stream(), "POW Pristine tx:");
        res!(pristine.trace());

        let validator = PacketValidator {
            pow: Some(res!(ProofOfWork::new(schms.powh.clone()))),
            sig: Some(schms.sign.clone()),
        };

        let powparams = PowCreateParams {
            pvars: PowVars {
                zbits,
                pristine,
            },
            time_lim: constant::POW_CREATE_TIMEOUT,
            count_lim: constant::POW_CREATE_COUNT_LIM,
        };

        let chunk_cfg = schms.chnk.clone();
        let chunker = ServerConfig::chunker(chunk_cfg.set_pad_last(pad_last));
        trace!(log_stream(), "{:?}", chunker);

        let size = chunker.cfg.chunk_size;
        let meta_len = PacketMeta::<ML, UL, ID::M, ID::U>::BYTE_LEN;
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
        let mid = ID::M::randef();
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
                ver: constant::VERSION,
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
            packet = res!(validator.to_bytes::<N, P0, P1, PowPristine<C, P0, P1>>(
                packet,
                &powparams,
                //powparams.clone(),
                inc_sigpk,
            ));
            let validator_len = packet.len() - len;
            trace!(log_stream(), "Packet {} lengths: msg {}, meta {} chunk {} valid {} total {}",
                i, msg_len, meta_len, chunk_len, validator_len, packet.len(),
            );
            trace!(log_stream(), "  Chunk:      {}", chunks[i].len());
            packets.push(packet);
        }

        if let Some(warning) = warning {
            warn!(log_stream(), "{}", warning);
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
        W: WireSchemeTypes + 'static,
    >(
        self,
        src:        Arc<UdpSocket>,
        trg_addr:   &SocketAddr,
        code:       [u8; C],
        schms:      WireSchemes<W>,
    )
        -> Outcome<()>
    {
        let packets = res!(self.build::<
            C,
            {constant::POW_INPUT_LEN},      // N
            {constant::POW_PREFIX_LEN},     // P0
            {constant::POW_PREIMAGE_LEN},   // P1
            PowPristine<
                C,
                {constant::POW_PREFIX_LEN},
                {constant::POW_PREIMAGE_LEN},
            >,
            W,
        >(
            res!(src.local_addr()).ip(),
            trg_addr.ip(),
            code,
            schms,
        ));
        for packet in packets {
            res!(src.send_to(&packet, trg_addr));
        }
        Ok(())
    }
}
