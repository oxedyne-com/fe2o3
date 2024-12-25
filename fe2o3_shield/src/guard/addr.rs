use crate::{
    msg::external::{
        //self,
        HandshakeType,
        //MsgId,
        //MsgState,
        MsgType,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedize_fe2o3_data::ring::RingTimer;
use oxedize_fe2o3_hash::map::ShardMap;
use oxedize_fe2o3_iop_hash::api::{
    Hasher,
    HashForm,
};

use std::{
    clone::Clone,
    //collections::BTreeMap,
    fmt::Debug,
    net::{
        IpAddr,
        SocketAddr,
    },
    sync::RwLock,
    time::{
        Duration,
        SystemTime,
    },
};

use rand::{
    self,
    Rng,
};

#[derive(Clone, Debug)]
pub enum AddressState<
    const N: usize, // length of ring buffer
    const R: u64, // maximum rate per second
> {
    Monitor(RingTimer<N>, u64), // Observing, not actively dropping packets.
    Throttle{ // Actively dropping packets.
        reqs:       RingTimer<N>, // Record time of requests.
        tint_min:   Duration, // Drop packet if time interval since last request
                              // is less than this minimum.
        start:      SystemTime, // Record start time of throttling.
        sunset:     Duration, // Turn off throttling after this duration.
    },
    Blacklist, // No soup for you.
    Whitelist, // Come on through.
}

impl<
    const N: usize, // length of ring buffer
    const R: u64, // maximum rate per second
> Default for AddressState<N, R> {
    fn default() -> Self {
        Self::Monitor(
            RingTimer::default(),
            R,
        )
    }
}

impl<
    const N: usize, // length of ring buffer
    const R: u64, // maximum rate per second
>
    AddressState<N, R>
{
    pub fn new_monitor(avg_rps_lim: u64) -> Self {
        Self::Monitor(
            RingTimer::default(),
            avg_rps_lim,
        )
    }

    pub fn new_throttle(tint_min: Duration, sunset: Duration) -> Self {
        Self::Throttle{
            reqs:   RingTimer::default(),
            tint_min,
            start:  SystemTime::now(),
            sunset,
        }
    }

    //pub fn update(&mut self) -> Option<Duration> {
    //    match self {
    //        Self::Monitor(reqs, _) |
    //        Self::Throttle(reqs, _) => Some(reqs.update()), 
    //        Self::Blacklist |
    //        Self::Whitelist => None,
    //    }
    //}
}

#[derive(Clone, Debug, Default)]
pub struct AddressLog<
    // AddressState
    const N: usize, // length of ring buffer
    const R: u64, // maximum rate per second
    // AddressData
    D: Clone + Debug + Default, // user supplied data container
> {
    pub state:          AddressState<N, R>,
    pub throttle_cnt:   u16,
    // Handshake
    pub pending:        Option<(HandshakeType, SystemTime)>,
    //pub msgs:           BTreeMap<MsgId, MsgState>,
    // Data
    pub data:           D,
}

//impl<
//    // AddressState
//    const N: usize, // length of ring buffer
//    const R: u64, // maximum rate per second
//    // AddressData
//    D: Clone + Debug + Default, // user supplied data container
//>
//    AddressLog<N, R, D>
//{
//    /// Insert the packet message chunk into the incomplete message map, and return the message if
//    /// it is complete.
//    pub fn get_msg<const U: usize>(
//        &mut self,
//        meta:       &PacketMeta<U>,
//        _src_addr:  &SocketAddr,
//        buf:        &[u8],
//        params:     &msg::MsgAssemblyParams,
//    )
//        -> (bool, Option<Vec<u8>>)
//    {
//        if !self.msgs.contains_key(&meta.mid) {
//            self.msgs.insert(meta.mid, MsgState::new(meta.chnk.num_chunks));
//        }
//        let (drop, msg_byt_opt) = match self.msgs.get_mut(&meta.mid) {
//            Some(mstat) => mstat.insert_part(
//                meta,
//                buf,
//                params,
//            ),
//            None => return (true, None),
//        };
//        if drop || msg_byt_opt.is_some() {
//            self.msgs.remove(&meta.mid);
//        }
//        (drop, msg_byt_opt)
//    }
//
//}

#[derive(Debug)]
pub struct AddressGuard<
    // ShardMap
    const C: usize, // Capacity (maximum number of bins).
    M: MapMut<HashForm, AddressLog<N, R, D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
    // AddressState
    const N: usize, // Length of ring buffer.
    const R: u64, // Maximum rate per second.
    // AddressData
    D: Clone + Debug + Default, // User supplied data container.
> {
    pub amap:       ShardMap<C, S, AddressLog<N, R, D>, M, H>,
    // Monitor
    pub arps_max:   u64,
    // Throttle
    pub tint_min:   Duration, // Drop packet if duration since last request is
                              // less than this minimum.
    pub tsunset:    (u64, u64), // Range for randomisation of sunset durations.
    pub blist_cnt:  u16, // Blacklist after this many throttling episodes.
    // Handshake
    pub hreq_exp:   Duration, // Set expiry window for handshake messages.
}

impl<
    // ShardMap
    const C: usize, // Capacity (maximum number of bins).
    M: MapMut<HashForm, AddressLog<N, R, D>> + Clone + Debug,
    H: Hasher + Send + Sync + 'static, // Key hasher.
    const S: usize, // Key hasher salt length.
    // AddressState
    const N: usize, // Length of ring buffer.
    const R: u64, // Maximum rate per second.
    // AddressData
    D: Clone + Debug + Default, // user supplied data container
>
    AddressGuard<C, M, H, S, N, R, D>
{
    fn ip_addr_to_bytes(addr: &IpAddr) -> Vec<u8> {
        match addr {
            IpAddr::V4(addr) => addr.octets().to_vec(),
            IpAddr::V6(addr) => addr.octets().to_vec(),
        }
    }

    /// Updates state for given address and returns whether the packet should be dropped.
    pub fn drop_packet(
        &self,
        msg_typ:    MsgType,
        src_addr:   &SocketAddr,
    )
        -> Outcome<bool>
    {
        let htyp = HandshakeType::from(msg_typ);
        if htyp == HandshakeType::Unknown {
            return Ok(false);
        }
        //let ip_addr = src_addr.ip();
        //let addr_key = Self::ip_addr_to_bytes(&ip_addr);
        let (key, locked_map) = res!(self.get_locked_map(&src_addr));
        let mut new = false;
        {
            let mut unlocked_map = lock_write!(locked_map);
            match unlocked_map.get_mut(&key) {
                Some(alog) => {
                    // Check the rate of incoming requests.
                    match &mut alog.state {
                        AddressState::Monitor(ref mut reqs, _avg_rps_lim) => {
                            reqs.update();
                            if reqs.avg_rps() > self.arps_max {
                                // Downgrade treatment of address.
                                if alog.throttle_cnt >= self.blist_cnt {
                                    // Blacklist after too many throttling episodes.
                                    alog.state = AddressState::Blacklist;
                                    alog.throttle_cnt = alog.throttle_cnt + 1;
                                    return Ok(true);
                                } else {
                                    // Downgrade to throttled state.
                                    alog.state = AddressState::new_throttle(
                                        self.tint_min,
                                        Duration::from_secs(rand::thread_rng().gen_range(
                                            self.tsunset.0..self.tsunset.1
                                        )),
                                    );
                                    alog.throttle_cnt = alog.throttle_cnt + 1;
                                }
                            }
                        },
                        AddressState::Throttle{
                            ref mut reqs, ..
                            //tint_min,
                            //start,
                            //sunset,
                        } => {
                            reqs.update();
                            if reqs.last_duration() < self.tint_min {
                                return Ok(true);
                            }
                        },
                        AddressState::Blacklist => return Ok(true),
                        AddressState::Whitelist => (),
                    }
                    // Impose sequence order on session requests.
                    match alog.pending {
                        Some((typ, when)) => {
                            match when.elapsed() {
                                Ok(wait) => if wait > self.hreq_exp {
                                    alog.pending = None;
                                } else {
                                    match typ {
                                        HandshakeType::Req1 => { // Waiting for a HREQ2.
                                            if !htyp.is_hreq2() {
                                                return Ok(true);
                                            } else {
                                                alog.pending = Some((
                                                    htyp,
                                                    SystemTime::now(),
                                                ));
                                            }
                                        },
                                        HandshakeType::Req2 => { // Waiting for a HREQ3.
                                            if htyp != HandshakeType::Req3 {
                                                return Ok(true);
                                            } else {
                                                alog.pending = None;
                                            }
                                        },
                                        _ => (),
                                    }
                                }
                                Err(_) => (),
                            }
                        },
                        None => {
                            match htyp {
                                HandshakeType::Req1 => alog.pending = Some((
                                    HandshakeType::Req1,
                                    SystemTime::now(),
                                )),
                                HandshakeType::Req2 |
                                HandshakeType::Req3 => return Ok(true), // Must be preceded by a HREQ1.
                                _ => (),
                            }
                        },
                    }
                },
                None => new = true,
            }
        } // Release write lock on scr_addr shard.

        if new {
            // If we have no record of the address, the only acceptable request is a
            // HREQ1.
            if htyp != HandshakeType::Req1 {
                return Ok(true);
            }
            let alog = AddressLog {
                pending: Some((
                    HandshakeType::Req1,
                    SystemTime::now(),
                )),
                ..Default::default()
            };
            res!(self.amap.insert_using_hash(key, alog));
        }

        Ok(false)
    }

    pub fn get_locked_map(
        &self,
        addr: &SocketAddr,
    )
        -> Outcome<(HashForm, &RwLock<M>)>
    {
        let ip_addr = addr.ip();
        let key = self.amap.key(&Self::ip_addr_to_bytes(&ip_addr));
        let locked_map = res!(self.amap.get_shard_using_hash(&key));
        Ok((key, locked_map))
    }
}
