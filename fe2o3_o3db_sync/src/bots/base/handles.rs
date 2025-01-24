use crate::{
    prelude::*,
    bots::worker::bot::WorkerType,
    base::{
        id::OzoneBotId,
        index::{
            BotPoolInd,
            WorkerInd,
            ZoneInd,
        },
    },
    comm::{
        msg::OzoneMsg,
        response::{
            Responder,
            Wait,
        },
    },
};

use oxedize_fe2o3_core::{
    channels::Simplex,
    thread::Sentinel,
};
use oxedize_fe2o3_jdat::id::NumIdDat;

use std::{
    time::Duration,
};

use crossbeam_utils::sync::WaitGroup;


#[derive(Debug)]
pub struct Handle<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
    THD,
> {
    ozid:       Option<OzoneBotId>,
    pub thread: Option<std::thread::JoinHandle<THD>>,
    sentinel:   Sentinel,
    chan:       Option<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    THD,
>
    Default for Handle<UIDL, UID, ENC, KH, THD>
{
    fn default() -> Self {
        Self {
            ozid: None,
            thread: None,
            sentinel: Sentinel::default(),
            chan: None,
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    THD,
>
    Handle<UIDL, UID, ENC, KH, THD>
{
    pub fn new(
        ozid:       Option<OzoneBotId>,
        hand:       std::thread::JoinHandle<THD>,
        sentinel:   Sentinel,
        chan:       Option<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>>,
    )
        -> Self
    {
        Self {
            ozid,
            thread: Some(hand),
            sentinel,
            chan,
        }
    }

    pub fn ozid(&self)      -> &Option<OzoneBotId>  { &self.ozid }
    pub fn sentinel(&self)  -> &Sentinel            { &self.sentinel }
    pub fn chan(&self)      -> &Option<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        &self.chan
    }
    pub fn some_ozid(&self) -> Outcome<OzoneBotId> {
        match &self.ozid {
            Some(ozid) => Ok(ozid.clone()),
            None => Err(err!(
                "Handle contains no id as expected.";
                Identifier, Missing)),
        }
    }
    pub fn some_chan(&self) -> Outcome<Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        match &self.chan {
            Some(chan) => Ok(chan.clone()),
            None => Err(err!(
                "Handle contains no channel as expected.";
                Channel, Missing)),
        }
    }
}

/// Contains handles for all the bots (except the `Supervisor`), for use by the `Supervisor`.
#[derive(Debug)]
pub struct BotHandles<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    nz:     usize,
    cbots:  Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>>,
    fbots:  Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>>,
    igbots: Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>>,
    rbots:  Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>>,
    wbots:  Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>>,
    zbots:  Vec<Handle<UIDL, UID, ENC, KH, ()>>,
    cfg:    Handle<UIDL, UID, ENC, KH, ()>,
    sbots:  Vec<Handle<UIDL, UID, ENC, KH, ()>>,
    wait_init: WaitGroup,
    wait_end:  WaitGroup,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    Default for BotHandles<UIDL, UID, ENC, KH>
{
    fn default() -> Self {
        Self {
            nz: 0,
            cbots:      Vec::new(),
            fbots:      Vec::new(),
            igbots:     Vec::new(),
            rbots:      Vec::new(),
            wbots:      Vec::new(),
            zbots:      Vec::new(),
            cfg:        Handle::default(),
            sbots:      Vec::new(),
            wait_init:  WaitGroup::new(),
            wait_end:   WaitGroup::new(),
        }
    }
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
>
    BotHandles<UIDL, UID, ENC, KH>
{
    /// Create a full set of empty handles according to the given configuration.
    pub fn new(cfg: &OzoneConfig) -> Self {
        let nz = cfg.num_zones();
        let ns = cfg.num_sbots();
        let nc = cfg.num_bots_per_zone(&WorkerType::Cache);
        let nf = cfg.num_bots_per_zone(&WorkerType::File);
        let nig = cfg.num_bots_per_zone(&WorkerType::InitGarbage);
        let nr = cfg.num_bots_per_zone(&WorkerType::Reader);
        let nw = cfg.num_bots_per_zone(&WorkerType::Writer);
        let mut cbots = Vec::new();
        for _ in 0..nz {
            let mut bots = Vec::new();
            for _ in 0..nc {
                bots.push(Handle::<UIDL, UID, ENC, KH, ()>::default());
            }
            cbots.push(bots);
        }
        let mut fbots = Vec::new();
        for _ in 0..nz {
            let mut bots = Vec::new();
            for _ in 0..nf {
                bots.push(Handle::<UIDL, UID, ENC, KH, ()>::default());
            }
            fbots.push(bots);
        }
        let mut igbots = Vec::new();
        for _ in 0..nz {
            let mut bots = Vec::new();
            for _ in 0..nig {
                bots.push(Handle::<UIDL, UID, ENC, KH, ()>::default());
            }
            igbots.push(bots);
        }
        let mut rbots = Vec::new();
        for _ in 0..nz {
            let mut bots = Vec::new();
            for _ in 0..nr {
                bots.push(Handle::<UIDL, UID, ENC, KH, ()>::default());
            }
            rbots.push(bots);
        }
        let mut wbots = Vec::new();
        for _ in 0..nz {
            let mut bots = Vec::new();
            for _ in 0..nw {
                bots.push(Handle::<UIDL, UID, ENC, KH, ()>::default());
            }
            wbots.push(bots);
        }
        let mut zbots = Vec::new();
        for _ in 0..nz {
            zbots.push(Handle::<UIDL, UID, ENC, KH, ()>::default());
        }
        let mut sbots = Vec::new();
        for _ in 0..ns {
            sbots.push(Handle::<UIDL, UID, ENC, KH, ()>::default());
        }
        Self {
            nz,
            cbots,
            fbots,
            igbots,
            rbots,
            wbots,
            zbots,
            sbots,
            ..Default::default()
        }
    }

    // Use
    pub fn all_cbots(&self)     -> &Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &self.cbots }
    pub fn all_fbots(&self)     -> &Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &self.fbots }
    pub fn all_igbots(&self)    -> &Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &self.igbots }
    pub fn all_rbots(&self)     -> &Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &self.rbots }
    pub fn all_wbots(&self)     -> &Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &self.wbots }
    pub fn all_zbots(&self)     -> &Vec<Handle<UIDL, UID, ENC, KH, ()>>      { &self.zbots }
    pub fn cfg(&self)           -> &Handle<UIDL, UID, ENC, KH, ()>           { &self.cfg }
    pub fn all_sbots(&self)     -> &Vec<Handle<UIDL, UID, ENC, KH, ()>>      { &self.sbots }
    pub fn wait_init_ref(&self) -> &WaitGroup                       { &self.wait_init }
    pub fn wait_end_ref(&self)  -> &WaitGroup                       { &self.wait_end }

    pub fn get_zbot(&self, zind: &ZoneInd) -> Outcome<&Handle<UIDL, UID, ENC, KH, ()>> {
        res!(self.check_zone_index(**zind));
        Ok(&self.zbots[**zind])
    }

    // Mutate
    pub fn cbots_mut(&mut self)     -> &mut Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &mut self.cbots }
    pub fn fbots_mut(&mut self)     -> &mut Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &mut self.fbots }
    pub fn igbots_mut(&mut self)    -> &mut Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &mut self.igbots }
    pub fn rbots_mut(&mut self)     -> &mut Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &mut self.rbots }
    pub fn wbots_mut(&mut self)     -> &mut Vec<Vec<Handle<UIDL, UID, ENC, KH, ()>>> { &mut self.wbots }
    pub fn zbots_mut(&mut self)     -> &mut Vec<Handle<UIDL, UID, ENC, KH, ()>>      { &mut self.zbots }
    pub fn cfg_mut(&mut self)       -> &mut Handle<UIDL, UID, ENC, KH, ()>           { &mut self.cfg }
    pub fn sbots_mut(&mut self)     -> &mut Vec<Handle<UIDL, UID, ENC, KH, ()>>      { &mut self.sbots }

    pub fn set_sbot(&mut self, bpind: &BotPoolInd, hand: Handle<UIDL, UID, ENC, KH, ()>) -> Outcome<()> {
        self.sbots[**bpind] = hand;
        Ok(())
    }
    pub fn set_zbot(&mut self, zind: &ZoneInd, hand: Handle<UIDL, UID, ENC, KH, ()>) -> Outcome<()> {
        res!(self.check_zone_index(**zind));
        self.zbots[**zind] = hand;
        Ok(())
    }
    pub fn set_cfg(&mut self, hand: Handle<UIDL, UID, ENC, KH, ()>) { self.cfg = hand; }

    pub fn wait_init(self) {
        self.wait_init.wait();
    }
    pub fn wait_end(self) {
        self.wait_end.wait();
    }

    pub fn set_worker_bot(
        &mut self,
        wtyp:   &WorkerType,
        wind:   &WorkerInd,
        hand:   Handle<UIDL, UID, ENC, KH, ()>,
    )
        -> Outcome<()>
    {
        res!(self.check_zone_index(wind.z()));
        match wtyp {
            WorkerType::Cache       => self.cbots[wind.z()][wind.b()] = hand,
            WorkerType::File        => self.fbots[wind.z()][wind.b()] = hand,
            WorkerType::InitGarbage => self.igbots[wind.z()][wind.b()] = hand,
            WorkerType::Reader      => self.rbots[wind.z()][wind.b()] = hand,
            WorkerType::Writer      => self.wbots[wind.z()][wind.b()] = hand,
        }
        Ok(())
    }

    fn check_zone_index(&self, ind: usize) -> Outcome<()> {
        if ind > self.nz {
            return Err(err!(
                "Zone index {} into BotHandles exceeds number of zones {}.",
                ind, self.nz;
                Bug, Excessive));
        }
        Ok(())
    }

    pub fn report_status(&self) {
        for z in 0..self.nz {
            for pool in [
                &self.cbots[z],
                &self.fbots[z],
                &self.igbots[z],
                &self.rbots[z],
                &self.wbots[z],
            ] {
                for h in pool {
                    if !h.sentinel().is_finished() {
                        msg!("{:?} bot is not finished", h.ozid());
                    }
                }
            }
        }
        for h in &self.zbots {
            if !h.sentinel().is_finished() {
                msg!("{:?} bot is not finished", h.ozid());
            }
        }
        for h in &self.sbots {
            if !h.sentinel().is_finished() {
                msg!("{:?} bot is not finished", h.ozid());
            }
        }
        for h in [&self.cfg] {
            if !h.sentinel().is_finished() {
                msg!("{:?} bot is not finished", h.ozid());
            }
        }
    }

    /// Returns the ids of bots threads that have finished.
    pub fn get_dead_bots(&self) -> Outcome<Vec<OzoneBotId>> {

        let mut finished = Vec::new();

        for z in 0..self.nz {
            for pool in [
                &self.cbots[z],
                &self.fbots[z],
                &self.igbots[z],
                &self.rbots[z],
                &self.wbots[z],
            ] {
                for h in pool {
                    if h.sentinel().is_finished() {
                        finished.push(res!(h.some_ozid().clone()));
                    }
                }
            }
        }
        for h in &self.zbots {
            if h.sentinel().is_finished() {
                finished.push(res!(h.some_ozid().clone()));
            }
        }
        for h in &self.sbots {
            if h.sentinel().is_finished() {
                finished.push(res!(h.some_ozid().clone()));
            }
        }
        for h in [&self.cfg] {
            if h.sentinel().is_finished() {
                finished.push(res!(h.some_ozid().clone()));
            }
        }

        Ok(finished)
    }

    /// Returns the ids of bots that fail to respond to a ping within the specified timeout.
    ///  
    /// # Arguments
    /// * `timeout` - The duration to wait for a response from each bot.
    /// 
    /// Returns a list of `OzoneBotIds` for bots that did not respond in time.
    pub fn get_unresponsive_bots(
        &self,
        timeout: Duration,
    )
        -> Outcome<(usize, Vec<OzoneBotId>)>
    {
        let wait = Wait {
            max_wait: timeout.clone(),
            check_interval: constant::CHECK_INTERVAL,
        };
        let resp = Responder::new(None);
        let mut all_bot_ids = Vec::new();
    
        // Send pings to all bots.
        for handle in self.iter() {
            if let Some(chan) = handle.chan() {
                let ozid = res!(handle.some_ozid());
                match chan.send(OzoneMsg::Ping(ozid.clone(), resp.clone())) {
                    Ok(_) => {
                        all_bot_ids.push(ozid);
                    }
                    Err(e) => error!(err!(e,
                        "While sending ping to bot {:?}", ozid;
                        Channel, Write))
                }
            }
        }
        let expected = all_bot_ids.len();
    
        // Track responses and build set of responsive bots.
        let (_, responsive) = res!(resp.recv_pongs(wait));
        if responsive.len() > expected {
            error!(err!(
                "Expecting {} messages via responder, received {} after \
                {:?}.", responsive.len(), expected, timeout;
                Input, Mismatch, Size));
        }
    
        // Collect unresponsive bot IDs by comparing against those that responded.
        let mut unresponsive = Vec::new();
        for ozid in &all_bot_ids {
            if !responsive.contains(ozid) {
                unresponsive.push(ozid.clone());
            }
        }
    
        Ok((expected, unresponsive))
    }
}

/// Immutable iterator over all bot handles in a BotHandles collection.
#[derive(Debug)]
pub struct BotHandlesIter<'a, const UIDL: usize, UID, ENC, KH>
where
    UID: NumIdDat<UIDL>,
    ENC: Encrypter,
    KH: Hasher,
{
    handles: &'a BotHandles<UIDL, UID, ENC, KH>,
    zone_index: usize,
    pool_type: usize,    // Index into the pool types (cbots, fbots, etc.).
    bot_index: usize,    // Index within the current pool.
    stage: IterStage,    // Tracks which group of bots we're iterating over.
}

/// Tracks the current stage of iteration through different bot types.
#[derive(Debug, PartialEq)]
enum IterStage {
    Workers,    // Iterating through worker bot pools (cbots, fbots, etc.).
    ZoneBots,   // Iterating through zone bots.
    StoreBots,  // Iterating through store bots.
    Config,     // Iterating through the config bot.
    Done,       // Iteration complete.
}

impl<'a, const UIDL: usize, UID, ENC, KH> BotHandlesIter<'a, UIDL, UID, ENC, KH>
where
    UID: NumIdDat<UIDL> + 'static,
    ENC: Encrypter + 'static,
    KH: Hasher + 'static,
{
    fn new(handles: &'a BotHandles<UIDL, UID, ENC, KH>) -> Self {
        Self {
            handles,
            zone_index: 0,
            pool_type: 0,
            bot_index: 0,
            stage: IterStage::Workers,
        }
    }

    /// Returns the next worker bot handle, if any remain in the current zone.
    fn next_worker(&mut self) -> Option<&'a Handle<UIDL, UID, ENC, KH, ()>> {
        let pools = [
            self.handles.all_cbots(),
            self.handles.all_fbots(),
            self.handles.all_igbots(),
            self.handles.all_rbots(),
            self.handles.all_wbots(),
        ];

        // Ensure we haven't exceeded available pools.
        if self.pool_type >= pools.len() {
            return None;
        }

        let current_pool = &pools[self.pool_type][self.zone_index];

        // If we've exhausted the current pool.
        if self.bot_index >= current_pool.len() {
            self.bot_index = 0;
            self.pool_type += 1;
            return self.next_worker();
        }

        // If we've exhausted the current zone.
        if self.zone_index >= self.handles.nz {
            self.zone_index = 0;
            self.pool_type += 1;
            return self.next_worker();
        }

        let handle = &current_pool[self.bot_index];
        self.bot_index += 1;
        Some(handle)
    }
}

impl<'a, const UIDL: usize, UID, ENC, KH> Iterator for BotHandlesIter<'a, UIDL, UID, ENC, KH>
where
    UID: NumIdDat<UIDL> + 'static,
    ENC: Encrypter + 'static,
    KH: Hasher + 'static,
{
    type Item = &'a Handle<UIDL, UID, ENC, KH, ()>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.stage {
            IterStage::Workers => {
                if let Some(handle) = self.next_worker() {
                    return Some(handle);
                }
                self.stage = IterStage::ZoneBots;
                self.zone_index = 0;
                self.next()
            }
            IterStage::ZoneBots => {
                if self.zone_index < self.handles.nz {
                    let handle = &self.handles.all_zbots()[self.zone_index];
                    self.zone_index += 1;
                    Some(handle)
                } else {
                    self.stage = IterStage::StoreBots;
                    self.bot_index = 0;
                    self.next()
                }
            }
            IterStage::StoreBots => {
                if self.bot_index < self.handles.all_sbots().len() {
                    let handle = &self.handles.all_sbots()[self.bot_index];
                    self.bot_index += 1;
                    Some(handle)
                } else {
                    self.stage = IterStage::Config;
                    self.next()
                }
            }
            IterStage::Config => {
                self.stage = IterStage::Done;
                Some(self.handles.cfg())
            }
            IterStage::Done => None,
        }
    }
}

impl<const UIDL: usize, UID, ENC, KH> BotHandles<UIDL, UID, ENC, KH>
where
    UID: NumIdDat<UIDL> + 'static,
    ENC: Encrypter + 'static,
    KH: Hasher + 'static,
{
    /// Returns an iterator over references to all bot handles.
    pub fn iter(&self) -> BotHandlesIter<UIDL, UID, ENC, KH> {
        BotHandlesIter::new(self)
    }
}
