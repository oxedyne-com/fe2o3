use crate::{
    prelude::*,
    base::index::WorkerInd,
    bots::base::bot::OzoneBot,
    comm::{
        channels::ChannelPool,
        msg::OzoneMsg,
    },
    file::zdir::ZoneDir,
};

use oxedize_fe2o3_core::{
    channels::Simplex,
    thread::Semaphore,
};
use oxedize_fe2o3_jdat::id::NumIdDat;

#[macro_export]
macro_rules! workerbot_methods { () => {
    fn wind(&self)      -> &WorkerInd       { &self.wind }
    fn wtyp(&self)      -> &WorkerType      { &self.wtyp }
    fn zdir(&self)      -> &ZoneDir         { &self.zdir }
    fn set_zdir(&mut self, zdir: ZoneDir) {
        self.zdir = zdir;
    }
} }

pub trait WorkerBot<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>:
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> + Send
{
    fn wind(&self)      -> &WorkerInd;
    fn wtyp(&self)      -> &WorkerType;
    fn zdir(&self)      -> &ZoneDir;
    fn set_zdir(&mut self, zdir: ZoneDir);

    fn zbot(&self) -> Option<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        match self.chans().get_zbot(self.wind().zind()) {
            Err(e) => {
                self.error(e);
                None
            },
            Ok(chan) => Some(chan),
        }
    }

    fn cbots(&self) -> Outcome<ChannelPool<UIDL, UID, ENC, KH>> {
        self.chans().get_workers_of_type_in_zone(&WorkerType::Cache, self.wind().zind())
    }
    fn fbots(&self) -> Outcome<ChannelPool<UIDL, UID, ENC, KH>> {
        self.chans().get_workers_of_type_in_zone(&WorkerType::File, self.wind().zind())
    }
    fn igbots(&self) -> Outcome<ChannelPool<UIDL, UID, ENC, KH>> {
        self.chans().get_workers_of_type_in_zone(&WorkerType::InitGarbage, self.wind().zind())
    }

    fn listen_worker(&mut self, msg: OzoneMsg<UIDL, UID, ENC, KH>) -> Option<OzoneMsg<UIDL, UID, ENC, KH>> {
        match msg {
            OzoneMsg::ZoneDir(_, zdir) => {
                self.set_zdir(zdir);
                None
            },
            _ => Some(msg),
        }    
    }
}

/// A way to identify the bots that do most of the work in Ozone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerType {
    Cache,
    File,
    InitGarbage,
    Reader,
    Writer,
}

pub struct ZoneWorkerInitArgs<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    // Identity
    pub wind:       WorkerInd,
    pub wtyp:       WorkerType,
    // Bot
    pub sem:        Semaphore,
    // Comms
    pub chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    pub api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
}
