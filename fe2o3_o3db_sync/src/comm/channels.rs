use crate::{
    prelude::*,
    base::{
        id::{
            OzoneBotId,
            //OzoneBotType,
        },
        index::{
            BotPoolInd,
            WorkerInd,
            ZoneInd,
        },
    },
    bots::{
        worker::bot::WorkerType,
    },
    comm::msg::OzoneMsg,
};

use oxedyne_fe2o3_core::{
    channels::{
        simplex,
        Simplex,
    },
};
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    ops::{
        Index,
        IndexMut,
    },
    time::{
        Duration,
        Instant,
    },
};

use rand::Rng;

const FINISH_CHECK_INTERVAL: Duration = Duration::from_millis(5); // finer than CHECK_INTERVAL: a close waits on it

// The order in which a close finishes each zone's workers.  A stage is sent its `Finish` only
// once every worker of the stage before it has ended, since those are the ones that send it work
// and wait on its answers.  Finished together, a read or collection still queued behind the
// `Finish` of its reader or collector met a cache bot that had already ended, and waited out
// `BOT_REQUEST_TIMEOUT` for an answer nobody would send; and a record a syncer released late, a
// failed barrier's error among them, reached a cache bot that had ended, so its caller waited out
// the durability deadline (2026-09-24).
const FINISH_ORDER: [&[WorkerType]; 4] = [
    &[WorkerType::Reader, WorkerType::Scan, WorkerType::InitGarbage],   // ask the others
    &[WorkerType::Writer],                                              // release to the caches
    &[WorkerType::Cache],                                               // tell the file bots
    &[WorkerType::File],
];


#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PoolType {
    Cache,
    File,
    InitGarbage,
    Reader,
    Scan,
    Writer,
    Zone,
    Server,
}

impl From<&WorkerType> for PoolType {
    fn from(wtyp: &WorkerType) -> Self {
        match wtyp {
            WorkerType::Cache       => PoolType::Cache,
            WorkerType::File        => PoolType::File,
            WorkerType::InitGarbage => PoolType::InitGarbage,
            WorkerType::Reader      => PoolType::Reader,
            WorkerType::Scan        => PoolType::Scan,
            WorkerType::Writer      => PoolType::Writer,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ChooseBot {
    Randomly,
    ByFile(u32),
}

#[derive(Clone, Debug)]
pub struct ChannelPool<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    typ:    PoolType,
    pool:   Vec<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    ChannelPool<UIDL, UID, ENC, KH>
{
    pub fn new(typ: &PoolType, n: usize) -> Self {
        let mut pool = Vec::new();
        for _ in 0..n {
            pool.push(simplex());
        }
        Self {
            typ: typ.clone(),
            pool: pool,
        }
    }

    pub fn make(typ: &PoolType, pool: Vec<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>) -> Self {
        Self {
            typ: typ.clone(),
            pool: pool,
        }
    }

    fn check_index(&self, ind: usize) -> Outcome<()> {
        if ind > self.pool.len() {
            return Err(err!("Index {} exceeds pool size {}.", ind, self.pool.len(); Index, TooBig));
        }
        Ok(())
    }

    pub fn len(&self) -> usize { self.pool.len() }

    pub fn get_bot(&self, ind: usize) -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        res!(self.check_index(ind));
        Ok(&self.pool[ind])
    }

    pub fn choose_bot(
        &self,
        how: &ChooseBot,
    )
        -> (&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>, BotPoolInd)
    {
        let n = self.pool.len();
        let i = match how {
            ChooseBot::Randomly => {
                let mut rng = rand::thread_rng();
                rng.gen_range(0..n)
            },
            ChooseBot::ByFile(fnum) => (*fnum as usize) % n,
        };
        (&self.pool[i], BotPoolInd::new(i))
    }

    pub fn set_bot(&mut self, ind: usize, chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>) -> Outcome<()> {
        res!(self.check_index(ind));
        self.pool[ind] = chan;
        Ok(())
    }

    pub fn finish_all(&self) -> Outcome<()> {
        for chan in &self.pool {
            res!(chan.send(OzoneMsg::Finish));
        }
        Ok(())
    }

    pub fn msg_count(&self) -> Vec<usize> {
        let mut queues = Vec::new();
        for chan in &self.pool {
            queues.push(chan.len());
        }
        queues
    }

    pub fn msg_count_total(&self) -> usize {
        let mut total: usize = 0;
        for chan in &self.pool {
            total += chan.len();
        }
        total
    }

    pub fn msg_count_non_zero(&self) -> bool {
        let mut pending = false;
        for chan in &self.pool {
            pending = pending | (chan.len() > 0);
        }
        pending
    }

    /// Returns the number of messages sent.
    pub fn send_to_all(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        for chan in &self.pool {
            res!(chan.send(msg.clone()));
        }
        Ok(self.len())
    }
}

/// Channel message queue lengths for all worker bots in a zone.
#[derive(Clone, Debug)]
pub struct ZoneMsgCount {
    cbots:  Vec<usize>,
    fbots:  Vec<usize>,
    igbots: Vec<usize>,
    rbots:  Vec<usize>,
    scbots: Vec<usize>,
    wbots:  Vec<usize>,
}

impl ZoneMsgCount {
    pub fn total(&self) -> usize {
        let mut total = 0;
        total += self.cbots.iter().sum::<usize>();
        total += self.fbots.iter().sum::<usize>();
        total += self.igbots.iter().sum::<usize>();
        total += self.rbots.iter().sum::<usize>();
        total += self.scbots.iter().sum::<usize>();
        total += self.wbots.iter().sum::<usize>();
        total
    }
}

/// Channels for all worker bots in a zone.
#[derive(Clone, Debug)]
pub struct ZoneWorkerChannels<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    cbots:  ChannelPool<UIDL, UID, ENC, KH>,
    fbots:  ChannelPool<UIDL, UID, ENC, KH>,
    igbots: ChannelPool<UIDL, UID, ENC, KH>,
    rbots:  ChannelPool<UIDL, UID, ENC, KH>,
    scbots: ChannelPool<UIDL, UID, ENC, KH>,
    wbots:  ChannelPool<UIDL, UID, ENC, KH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
>
    Index<&WorkerType> for ZoneWorkerChannels<UIDL, UID, ENC, KH> {
    type Output = ChannelPool<UIDL, UID, ENC, KH>;

    fn index(&self, typ: &WorkerType) -> &Self::Output {
        match typ {
            WorkerType::Cache       => &self.cbots,
            WorkerType::File        => &self.fbots,
            WorkerType::InitGarbage => &self.igbots,
            WorkerType::Reader      => &self.rbots,
            WorkerType::Scan        => &self.scbots,
            WorkerType::Writer      => &self.wbots,
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
>
    IndexMut<&WorkerType> for ZoneWorkerChannels<UIDL, UID, ENC, KH>
{
    fn index_mut(&mut self, typ: &WorkerType) -> &mut Self::Output {
        match typ {
            WorkerType::Cache       => &mut self.cbots,
            WorkerType::File        => &mut self.fbots,
            WorkerType::InitGarbage => &mut self.igbots,
            WorkerType::Reader      => &mut self.rbots,
            WorkerType::Scan        => &mut self.scbots,
            WorkerType::Writer      => &mut self.wbots,
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    ZoneWorkerChannels<UIDL, UID, ENC, KH>
{
    /// Create a full set of worker channels for a zone, according to the given configuration.
    pub fn new(cfg: &OzoneConfig) -> Self {
        let nc = cfg.num_bots_per_zone(&WorkerType::Cache);
        let nf = cfg.num_bots_per_zone(&WorkerType::File);
        let nig = cfg.num_bots_per_zone(&WorkerType::InitGarbage);
        let nr = cfg.num_bots_per_zone(&WorkerType::Reader);
        let nsc = cfg.num_bots_per_zone(&WorkerType::Scan);
        let nw = cfg.num_bots_per_zone(&WorkerType::Writer);
        Self {
            cbots:  ChannelPool::new(&PoolType::Cache,      nc),
            fbots:  ChannelPool::new(&PoolType::File,       nf),
            igbots: ChannelPool::new(&PoolType::InitGarbage,nig),
            rbots:  ChannelPool::new(&PoolType::Reader,     nr),
            scbots: ChannelPool::new(&PoolType::Scan,       nsc),
            wbots:  ChannelPool::new(&PoolType::Writer,     nw),
        }
    }

    fn msg_count_non_zero(&self) -> bool {
        self.cbots.msg_count_non_zero()   |
        self.fbots.msg_count_non_zero()   |
        self.igbots.msg_count_non_zero()  |
        self.rbots.msg_count_non_zero()   |
        self.scbots.msg_count_non_zero()  |
        self.wbots.msg_count_non_zero()
    }

    /// Finishes the zone's workers of the given types.
    pub fn finish(&self, typs: &[WorkerType]) -> Outcome<()> {
        for typ in typs {
            res!(self[typ].finish_all());
        }
        Ok(())
    }

    pub fn msg_count(&self) -> ZoneMsgCount {
        ZoneMsgCount {
            cbots:  self.cbots.msg_count(),
            fbots:  self.fbots.msg_count(),
            igbots: self.igbots.msg_count(),
            rbots:  self.rbots.msg_count(),
            scbots: self.scbots.msg_count(),
            wbots:  self.wbots.msg_count(),
        }
    }

    pub fn total_bot_count(&self) -> usize {
        let mut count = 0;
        count += self.cbots.len();
        count += self.fbots.len();
        count += self.igbots.len();
        count += self.rbots.len();
        count += self.scbots.len();
        count += self.wbots.len();
        count
    }

    pub fn send_to_all(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        let mut count = 0;
        count += res!(self.cbots.send_to_all(msg.clone()));
        count += res!(self.fbots.send_to_all(msg.clone()));
        count += res!(self.igbots.send_to_all(msg.clone()));
        count += res!(self.rbots.send_to_all(msg.clone()));
        count += res!(self.scbots.send_to_all(msg.clone()));
        count += res!(self.wbots.send_to_all(msg.clone()));
        Ok(count)
    }
}

/// Message queue lengths for all channels.
#[derive(Clone, Debug)]
pub struct OzoneMsgCount {
    nz:     usize,
    zwbots: Vec<ZoneMsgCount>,
    zbots:  Vec<usize>,
    cfg:    usize,
    sbots:  Vec<usize>,
    sup:    usize,
}

impl OzoneMsgCount {

    pub fn total(&self) -> usize {
        let mut total = 0;
        self.zwbots.iter().for_each(|x| total += x.total());
        self.zbots.iter().for_each(|x| total += x);
        total += self.cfg;
        self.sbots.iter().for_each(|x| total += x);
        total += self.sup;
        total
    }

    pub fn total_zone(&self) -> usize {
        let mut total = 0;
        self.zwbots.iter().for_each(|x| total += x.total());
        self.zbots.iter().for_each(|x| total += x);
        total
    }
}

/// Channels for all bots in all zones.  Rather than sharing references to these channels, clone them.  Unlike `bots::base::handles::BotHandles`, this includes the `Supervisor`.
#[derive(Clone, Debug)]
pub struct BotChannels<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    nz:     usize,
    zwbots: Vec<ZoneWorkerChannels<UIDL, UID, ENC, KH>>,
    zbots:  ChannelPool<UIDL, UID, ENC, KH>,
    cfg:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    sbots:  ChannelPool<UIDL, UID, ENC, KH>,
    sup:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    BotChannels<UIDL, UID, ENC, KH>
{
    /// Create a full set of functioning channels according to the given configuration.
    pub fn new(cfg: &OzoneConfig) -> Self {
        let nz = cfg.num_zones();
        let mut zwbots = Vec::new();
        for _ in 0..nz {
            zwbots.push(ZoneWorkerChannels::new(cfg));
        }
        Self {
            nz,
            zwbots,
            zbots:  ChannelPool::new(&PoolType::Zone, nz),
            cfg:    simplex(),
            sbots:  ChannelPool::new(&PoolType::Server, cfg.num_sbots()),
            sup:    simplex(),
        }
    }

    /// Returns channels for all worker pools, for the given zone.
    pub fn get_all_workers_in_zone(
        &self,
        zind: &ZoneInd,
    )
        -> Outcome<ZoneWorkerChannels<UIDL, UID, ENC, KH>>
    {
        res!(self.check_zone_index(**zind));
        Ok(self.zwbots[**zind].clone())
    }

    /// Returns channels for the worker pool of the given type, for the given zone.
    pub fn get_workers_of_type_in_zone(
        &self,
        wtyp:   &WorkerType,
        zind:   &ZoneInd,
    )
        -> Outcome<ChannelPool<UIDL, UID, ENC, KH>>
    {
        res!(self.check_zone_index(**zind));
        Ok(self.zwbots[**zind][wtyp].clone())
    }

    /// Returns channels for all worker pools of the given type, across all zones.
    pub fn get_all_workers_of_type(
        &self,
        wtyp: &WorkerType,
    )
        -> Vec<ChannelPool<UIDL, UID, ENC, KH>>
    {
        let mut pools = Vec::new();
        for z in 0..self.nz {
            pools.push(self.zwbots[z][wtyp].clone());           
        }
        pools
    }

    pub fn all_zwbots(&self)    -> &Vec<ZoneWorkerChannels<UIDL, UID, ENC, KH>>  { &self.zwbots }
    pub fn all_zbots(&self)     -> &ChannelPool<UIDL, UID, ENC, KH>              { &self.zbots }
    pub fn cfg(&self)           -> &Simplex<OzoneMsg<UIDL, UID, ENC, KH>>        { &self.cfg }
    pub fn all_sbots(&self)     -> &ChannelPool<UIDL, UID, ENC, KH>              { &self.sbots }
    pub fn sup(&self)           -> &Simplex<OzoneMsg<UIDL, UID, ENC, KH>>        { &self.sup }

    pub fn get_sbot(&self, sind: &BotPoolInd) -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        self.sbots.get_bot(**sind)
    }

    pub fn get_zbot(&self, zind: &ZoneInd) -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        self.zbots.get_bot(**zind)
    }

    pub fn get_zwbots(&self, zind: &ZoneInd) -> Outcome<&ZoneWorkerChannels<UIDL, UID, ENC, KH>> {
        if **zind > self.zwbots.len() {
            return Err(err!(
                "Index {} exceeds number of zones {}.", **zind, self.zwbots.len();
                Index, TooBig));
        }
        Ok(&self.zwbots[**zind])
    }

    pub fn get_bot(
        &self,
        ozid: &OzoneBotId,
    )
        -> Outcome<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>
    {
        Ok(match ozid {
            // Solo bots
            OzoneBotId::ConfigBot(..) => self.cfg(),
            OzoneBotId::Supervisor(..) => self.sup(),
            OzoneBotId::ServerBot(_, bpind) => res!(self.get_sbot(bpind)),
            OzoneBotId::ZoneBot(_, zind) => res!(self.get_zbot(zind)),
            OzoneBotId::CacheBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Cache].get_bot(**bpind)),
            OzoneBotId::FileBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::File].get_bot(**bpind)),
            OzoneBotId::InitGarbageBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::InitGarbage].get_bot(**bpind)),
            OzoneBotId::ReaderBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Reader].get_bot(**bpind)),
            OzoneBotId::ScanBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Scan].get_bot(**bpind)),
            OzoneBotId::WriterBot(_, zind, bpind) =>
                res!(res!(self.get_zwbots(zind))[&WorkerType::Writer].get_bot(**bpind)),
            _ => return Err(err!(
                "Cannot return channel for {:?}.", ozid;
                Bug, Invalid)),
        })
    }

    // Mutate
    pub fn zbots_mut(&mut self) -> &mut ChannelPool<UIDL, UID, ENC, KH>          { &mut self.zbots }
    pub fn cfg_mut(&mut self)   -> &mut Simplex<OzoneMsg<UIDL, UID, ENC, KH>>    { &mut self.cfg }
    pub fn sbots_mut(&mut self) -> &mut ChannelPool<UIDL, UID, ENC, KH>          { &mut self.sbots }
    pub fn sup_mut(&mut self)   -> &mut Simplex<OzoneMsg<UIDL, UID, ENC, KH>>    { &mut self.sup }

    pub fn set_sbot(
        &mut self,
        bpind:  &BotPoolInd,
        chan:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        self.sbots.set_bot(**bpind, chan)
    }

    pub fn set_zbot(
        &mut self,
        zind: &ZoneInd,
        chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        self.zbots.set_bot(**zind, chan)
    }

    pub fn set_cfg(&mut self, chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>)   { self.cfg = chan; }
    pub fn set_sup(&mut self, chan: Simplex<OzoneMsg<UIDL, UID, ENC, KH>>)   { self.sup = chan; }

    pub fn set_worker_bot(
        &mut self,
        wtyp:   &WorkerType,
        wind:   &WorkerInd,
        chan:   Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        res!(self.check_zone_index(wind.z()));
        self.zwbots[wind.z()][wtyp].set_bot(wind.b(), chan)
    }

    fn check_zone_index(&self, ind: usize) -> Outcome<()> {
        if ind > self.nz {
            return Err(err!(
                "Zone index {} into BotChannels exceeds number of zones {}.",
                ind, self.nz;
                Index, TooBig));
        }
        Ok(())
    }

    /// Sends a `Finish` to every bot but the supervisor: the servers first, then each zone's
    /// workers stage by stage in `FINISH_ORDER`, then the zone bots and the config bot.  Before
    /// each stage it waits until `ended` says every worker of the stage before it has ended.  It
    /// stops waiting at `until` and returns the stage it was waiting on, already finished, for
    /// `finish_from` to carry on from; `None` once every bot has been sent its `Finish`.
    pub fn finish_all<F: Fn(&[WorkerType]) -> bool>(
        &self,
        ended:  F,
        until:  Instant,
    )
        -> Outcome<Option<usize>>
    {
        // Starve servers.
        res!(self.sbots.send_to_all(OzoneMsg::Finish));
        warn!(sync_log::stream(), "Shutdown: Completion request sent to server, finishing the \
            other bots in order, waiting up to {:?} for them.",
            until.saturating_duration_since(Instant::now()));
        res!(self.finish_stage(0));
        self.finish_from(0, ended, until)
    }

    /// Carries on the `finish_all` that stopped waiting on `stage`.  An `ended` that is always
    /// true finishes everything left at once.
    pub fn finish_from<F: Fn(&[WorkerType]) -> bool>(
        &self,
        stage:  usize,
        ended:  F,
        until:  Instant,
    )
        -> Outcome<Option<usize>>
    {
        let mut stage = stage;
        while stage + 1 < FINISH_ORDER.len() {
            let left = until.saturating_duration_since(Instant::now());
            let (start, timed_out) = res!(oxedyne_fe2o3_core::time::wait_for_true(
                FINISH_CHECK_INTERVAL.min(left),
                left,
                || ended(FINISH_ORDER[stage]),
            ));
            if timed_out {
                // Counted, not listed: a channel can only be read by taking its messages, and
                // taken to be listed here, the records the writers had just released were
                // destroyed, and their callers waited out the durability deadline for writes that
                // had landed (2026-09-23).
                warn!(sync_log::stream(), "Shutdown: The {:?} bots had not all ended after {:?}, \
                    so those after them wait; pending: {:?}",
                    FINISH_ORDER[stage], start.elapsed(), self.msg_count());
                return Ok(Some(stage));
            }
            stage += 1;
            res!(self.finish_stage(stage));
        }
        res!(self.zbots.finish_all());
        res!(self.cfg().send(OzoneMsg::Finish));
        warn!(sync_log::stream(), "Shutdown: Every bot has been sent its completion request.");
        Ok(None)
    }

    fn finish_stage(&self, stage: usize) -> Outcome<()> {
        for zone in &self.zwbots {
            res!(zone.finish(FINISH_ORDER[stage]));
        }
        Ok(())
    }

    pub fn fwd_to_all_zones(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        self.zbots.send_to_all(msg)
    }

    pub fn send_to_all(&self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Outcome<usize> {
        let mut count = 0;
        for zwbot in &self.zwbots {
            count += res!(zwbot.send_to_all(msg.clone()));
        }
        count += res!(self.zbots.send_to_all(msg.clone()));
        {
            res!(self.cfg.send(msg.clone()));
            count += 1;
        }
        count += res!(self.sbots.send_to_all(msg.clone()));
        {
            res!(self.sup.send(msg.clone()));
            count += 1;
        }
        Ok(count)
    }

    pub fn msg_count(&self) -> OzoneMsgCount {
        let mut zone_counts = Vec::new();
        for zone in &self.zwbots {
            zone_counts.push(zone.msg_count());
        }
        OzoneMsgCount {
            nz:     self.nz,
            zwbots: zone_counts,
            zbots:  self.zbots.msg_count(),
            cfg:    self.cfg.len(),
            sbots:  self.sbots.msg_count(),
            sup:    self.sup.len(),
        }
    }

    pub fn dump_pending_messages(
        lines:  Vec<String>, // obtain using drain_messages
        label:  &str,
        z:      Option<usize>,
        b:      Option<usize>,
    ) {
        match (z, b) {
            (Some(z), Some(b)) => debug!(sync_log::stream(), " Z{} B{} {} messages ({}):", z, b, label, lines.len()),
            (Some(z), None) => debug!(sync_log::stream(), " Z{} {} messages ({}):", z, label, lines.len()),
            (None, None) => debug!(sync_log::stream(), " {} messages ({}):", label, lines.len()),
            _ => (),
        }
        if lines.len() > 0 {
            for line in lines {
                debug!(sync_log::stream(), "  {}", line);
            }
        }
    }
}
