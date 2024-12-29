use crate::srv::{
    packet::{
        PacketChunkState,
        PacketCount,
        PacketMeta,
        PacketValidator,
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

pub type MsgType = u16;
//pub const MSG_TYPE_BYTE_LEN: usize = 2;
//pub const MSG_TYPE_USER_START: MsgType = 1_024;
//pub type MsgId = u64;
//pub const MSG_ID_BYTE_LEN: usize = 8;

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HandshakeType {
    Unknown = 0,
    Req1    = 1,
    Resp1   = 2,
    Req2    = 3,
    Resp2   = 4,
    Req3    = 5,
    Resp3   = 6,
}

impl From<MsgType> for HandshakeType {
    fn from(u: MsgType) -> Self {
        match u {
            1 =>    Self::Req1,
            2 =>    Self::Resp1,
            3 =>    Self::Req2,
            4 =>    Self::Resp2,
            5 =>    Self::Req3,
            6 =>    Self::Resp3,
            _ =>    Self::Unknown,
        }
    }
}

impl HandshakeType {
    pub fn is_hreq2(&self) -> bool {
        match self {
            Self::Req2 => true,
            _ => false,
        }
    }
}

pub trait IdentifiedMessage {
    fn typ(&self) -> MsgType;
    fn name(&self) -> &'static str;
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

#[derive(Debug)]
pub struct MsgAssembler<
    // ShardMap
    const C: usize, // Capacity (maximum number of shards).
    M: MapMut<HashForm, MsgState> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
> {
    pub msgs: ShardMap<C, S, MsgState, M, H>,
}

impl<
    // ShardMap
    const C: usize, // Capacity (maximum number of shards).
    M: MapMut<HashForm, MsgState> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
>
    MsgAssembler<C, M, H, S>
{
    pub fn new(
        n:          u32,
        salt:       [u8; S],
        init_map:   M,
        hasher:     H,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            msgs: res!(ShardMap::new(
                n,
                salt,
                init_map,
                hasher,
            )),
        })
    }

    pub fn get_locked_map<
        const MIDL: usize,
        MID: NumIdDat<MIDL>,
    >(
        &self,
        mid: &IdDat<MIDL, MID>,
    )
        -> Outcome<(HashForm, &RwLock<M>)>
    {
        let key = self.msgs.key(&mid.to_byte_array());
        let locked_map = res!(self.msgs.get_shard_using_hash(&key));
        Ok((key, locked_map))
    }

    /// Insert the packet message chunk into the message assembler map.  Returns whether the
    /// message should be dropped, and possibly the entire message when it is complete.
    pub fn get_msg<
        const MIDL: usize,
        const UIDL: usize,
        MID: NumIdDat<MIDL>,
        UID: NumIdDat<UIDL>,
    >(
        &self,
        meta:   &PacketMeta<MIDL, UIDL, MID, UID>,
        buf:    &[u8],
        params: &MsgAssemblyParams,
    )
        -> Outcome<(bool, Option<Vec<u8>>)>
    {
        let (key, locked_map) = res!(self.get_locked_map(&meta.mid));
        let mut unlocked_map = lock_write!(locked_map);
        if !unlocked_map.contains_key(&key) {
            unlocked_map.insert(key.clone(), MsgState::new(meta.chnk.num_chunks));
        }
        let (drop, msg_byt_opt) = match unlocked_map.get_mut(&key) {
            Some(mstat) => mstat.insert_part(
                meta,
                buf,
                params,
            ),
            None => return Ok((true, None)),
        };
        if drop || msg_byt_opt.is_some() {
            unlocked_map.remove(&key);
        }
        Ok((drop, msg_byt_opt))
    }

    pub fn remove<
        const MIDL: usize,
        MID: NumIdDat<MIDL>,
    >(
        &self,
        mid: &IdDat<MIDL, MID>,
    )
        -> Outcome<()>
    {
        let (key, locked_map) = res!(self.get_locked_map(mid));
        let mut unlocked_map = lock_write!(locked_map);
        unlocked_map.remove(&key);
        Ok(())
    }

    pub fn message_assembly_garbage_collection(
        &self,
        params: &MsgAssemblyParams,
    )
        -> Outcome<()>
    {
        for i in 0..self.msgs.n {
            if let Some(locked_map) = &self.msgs.shards[i] {
                let mut unlocked_map = lock_write!(locked_map);
                unlocked_map.retain(
                    |_key, mstat|
                    !mstat.drop_on_time_check(params)
                )
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct MsgAssemblyParams {
    pub msg_sunset:     Duration,
    pub idle_max:       Duration,
    pub rep_tot_lim:    u8,
    pub rep_max_lim:    u8,
}

#[derive(Clone, Debug)]
pub struct MsgState {
    parts:      BTreeMap<PacketCount, (Vec<u8>, u8)>, // The packet and how many times it has been written.
    tot:        PacketCount,
    cnt:        PacketCount, // Count of packets not yet received.
    first:      Instant,
    last:       Instant,
    rep_tot:    u8, // Total repetitions.
    rep_max:    u8, // Maximum repetition for any packet number.
}

impl Default for MsgState {
    fn default() -> Self {
        Self {
            parts:      BTreeMap::new(),
            tot:        0,
            cnt:        0,
            first:      Instant::now(),
            last:       Instant::now(),
            rep_tot:    0,
            rep_max:    0,
        }
    }
}

impl MsgState {

    pub fn new(total_packets: PacketCount) -> Self {
        Self {
            tot: total_packets,
            cnt: total_packets,
            ..Default::default()
        }
    }

    /// Inserts the packet payload into the message.  Returns whether the entire partial message
    /// should be dropped, and possibly the completed message.
    pub fn insert_part<
        const MIDL: usize,
        const UIDL: usize,
        MID: NumIdDat<MIDL>,
        UID: NumIdDat<UIDL>,
    >(
        &mut self,
        meta:   &PacketMeta<MIDL, UIDL, MID, UID>,
        buf:    &[u8],
        params: &MsgAssemblyParams,
    )
        -> (bool, Option<Vec<u8>>)
    {
        if self.cnt == self.tot {
            self.first = Instant::now();
        }
        if self.drop_on_time_check(params) {
            return (true, None);
        }
        self.last = Instant::now();
        match self.parts.get_mut(&meta.chnk.index) {
            Some((_part, n)) => { // Update repetition data but do not copy packet.
                match n.checked_add(1) {
                    Some(n2) => if n2 > params.rep_max_lim {
                        return (true, None);
                    } else {
                        *n = n2;
                        self.rep_max = n2;
                    }
                    None => return (true, None),
                }
                match self.rep_tot.checked_add(1) {
                    Some(n2) => if n2 > params.rep_tot_lim {
                        return (true, None);
                    } else {
                        self.rep_tot = n2;
                    },
                    None => return (true, None),
                }
                return (false, None);
            },
            None => (),
        }
        self.parts.insert(meta.chnk.index, (buf.to_vec(), 0));
        if self.cnt == 1 {
            // Assemble full message bytes.
            let mut v = Vec::new();
            for (_id, (part, _n)) in self.parts.iter_mut() {
                v.append(part);
            }
            return (false, Some(v));
        } else {
            self.cnt -= 1;
        }
        (false, None)
    }

    pub fn drop_on_time_check(
        &mut self,
        params: &MsgAssemblyParams,
    )
        -> bool
    {
        if self.first.elapsed() > params.msg_sunset ||
            self.last.elapsed() > params.idle_max
        {
            true
        } else {
            false
        }
    }
}
