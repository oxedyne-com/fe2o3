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

use oxedyne_fe2o3_core::{
    channels::Simplex,
    thread::Semaphore,
};
use oxedyne_fe2o3_jdat::id::NumIdDat;

/// Generates the shared accessor implementations ([`wind`](WorkerBot::wind),
/// [`wtyp`](WorkerBot::wtyp), [`zdir`](WorkerBot::zdir) and its setter) that
/// every worker bot needs, saving each bot struct from repeating them.
#[macro_export]
macro_rules! workerbot_methods { () => {
    fn wind(&self)      -> &WorkerInd       { &self.wind }
    fn wtyp(&self)      -> &WorkerType      { &self.wtyp }
    fn zdir(&self)      -> &ZoneDir         { &self.zdir }
    fn set_zdir(&mut self, zdir: ZoneDir) {
        self.zdir = zdir;
    }
} }

/// Behaviour shared by every zone worker bot (cache, file, reader, writer and
/// init-garbage). Extends [`OzoneBot`] with knowledge of the bot's worker
/// index, type and zone directory, and with helpers for reaching sibling bots
/// in the same zone.
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
    /// Returns the bot's worker index (zone and pool position).
    fn wind(&self)      -> &WorkerInd;
    /// Returns the bot's worker type.
    fn wtyp(&self)      -> &WorkerType;
    /// Returns the zone directory the bot operates within.
    fn zdir(&self)      -> &ZoneDir;
    /// Replaces the bot's zone directory, e.g. after a rezoning update.
    fn set_zdir(&mut self, zdir: ZoneDir);

    /// Returns the channel to this zone's zone bot, or `None` if it cannot be
    /// resolved (the error is logged).
    fn zbot(&self) -> Option<&Simplex<OzoneMsg<UIDL, UID, ENC, KH>>> {
        match self.chans().get_zbot(self.wind().zind()) {
            Err(e) => {
                self.error(e);
                None
            },
            Ok(chan) => Some(chan),
        }
    }

    /// Returns the cache-bot channel pool for this bot's zone.
    fn cbots(&self) -> Outcome<ChannelPool<UIDL, UID, ENC, KH>> {
        self.chans().get_workers_of_type_in_zone(&WorkerType::Cache, self.wind().zind())
    }
    /// Returns the file-bot channel pool for this bot's zone.
    fn fbots(&self) -> Outcome<ChannelPool<UIDL, UID, ENC, KH>> {
        self.chans().get_workers_of_type_in_zone(&WorkerType::File, self.wind().zind())
    }
    /// Returns the init-garbage-bot channel pool for this bot's zone.
    fn igbots(&self) -> Outcome<ChannelPool<UIDL, UID, ENC, KH>> {
        self.chans().get_workers_of_type_in_zone(&WorkerType::InitGarbage, self.wind().zind())
    }

    /// Handles the messages common to all workers, absorbing a zone-directory
    /// update and returning any other message unhandled for the caller to
    /// dispatch.
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
    /// Holds the in-memory value cache and file-location index.
    Cache,
    /// Owns open file handles and services file reads and writes.
    File,
    /// Performs cache initialisation and background garbage collection.
    InitGarbage,
    /// Services value reads.
    Reader,
    /// Owns a live data file and services value writes.
    Writer,
}

/// The bundle of identity, communication and API dependencies handed to a zone
/// worker bot when it is started.
pub struct ZoneWorkerInitArgs<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    // Identity
    /// The bot's worker index (zone and pool position).
    pub wind:       WorkerInd,
    /// The bot's worker type.
    pub wtyp:       WorkerType,
    // Bot
    /// Semaphore used to signal readiness and shutdown to the supervisor.
    pub sem:            Semaphore,
    /// Log stream identifier for this bot's output.
    pub log_stream_id:  String,
    // Comms
    /// The bot's inbound message channel.
    pub chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    /// The database API handle the bot issues operations through.
    pub api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
}
