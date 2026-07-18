use crate::{
    prelude::*,
    base::{
        cfg::ZoneConfig,
        id::OzoneBotId,
        index::{
            WorkerInd,
            ZoneInd,
        },
    },
    bots::{
        worker::{
            bot_file::GcControl,
            bot_reader::ReadResult,
        },
        bot_zone::ZoneState,
    },
    comm::{
        channels::BotChannels,
        response::Responder,
    },
    data::{
        cache::{
            Cache,
            MetaLocation,
        },
        core::{
            Key,
            Value,
        },
    },
    file::{
        core::FileEntry,
        floc::{
            FileLocation,
            FileNum,
        },
        state::{
            FileState,
            FileStateMap,
        },
        zdir::ZoneDir,
    },
};

use oxedyne_fe2o3_bot::msg::BotMsg;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::{
    Meta,
    RestSchemesOverride,
    ScanOpts,
};
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
    Dat,
    id::NumIdDat,
};

use std::{
    collections::BTreeMap,
};

/// The single message type exchanged between every bot in the database.
///
/// Variants are grouped by role: advisory notifications, commands, requests
/// (which carry a [`Responder`] the recipient replies on), responses, and the
/// server-facing get/put pair. A bot's listen loop matches on this enum to
/// decide what work to do and what, if anything, to reply.
#[derive(Clone, Debug)]
pub enum OzoneMsg<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    /// A no-op / absent message.
    None,
    // Advise
    /// Reports a cache's entry count, size and size limit in bytes.
    CacheSize(usize, usize, usize),
    /// Instructs a cache bot to adopt a new size limit in bytes.
    SetCacheSizeLimit(usize),
    /// Distributes a refreshed bot channel set, acknowledged via the responder.
    Channels(BotChannels<UIDL, UID, ENC, KH>, Responder<UIDL, UID, ENC, KH>),
    /// Acknowledges receipt of an updated channel set by the named bot.
    ChannelsReceived(OzoneBotId),
    /// Distributes an updated database configuration.
    Config(OzoneConfig),
    //ConfigConfirm(OzoneBotId, Ticket),
    /// Signals a bot to wind down its current activity.
    Finish,
    /// Reports completion of garbage collection on a file, with its new state
    /// and the number of bytes reclaimed.
    GcCompleted(FileNum, FileState, usize),
    /// Triggers a self-test on start-up.
    InitTest,
    /// Reports a bot's pending inbound message count.
    MessageCount(usize),
    /// Delivers a fresh set of file states.
    NewFileStates(FileStateMap),
    /// Notifies that reading of a file has finished.
    ReadFinished(FileNum),
    /// Schedules a file location as old (superseded) for later collection.
    ScheduleOld(FileLocation, OzoneBotId),
    /// Reports the data and index sizes of a file shard.
    ShardFileSize(usize, usize),
    /// Advises a cache of a value's new file location, superseding an old one.
    UpdateData {
        /// New location of the value.
        floc_new:       FileLocation,
        /// Length of the stored index entry.
        ilen:           usize,
        /// Previous location, now superseded, if any.
        floc_old_opt:   Option<FileLocation>,
        /// Bot that originated the update.
        from_id:        OzoneBotId,
    },
    /// Distributes the current zone directory for a zone.
    ZoneDir(ZoneInd, ZoneDir),
    /// Triggers zone initialisation.
    ZoneInitTrigger,
    /// Instructs a zone to initialise from the given directory and config.
    ZoneInit(ZoneDir, ZoneConfig),
    /// Reports the state of a zone.
    ZoneState(usize, ZoneState),
    // Command
    GcControl(GcControl, Responder<UIDL, UID, ENC, KH>), // sup -> gbot, control gc activation
    /// Commands cache bots to clear their cached values.
    ClearCache(Responder<UIDL, UID, ENC, KH>),
    /// Commands a file bot to close the old live file and adopt a new one.
    CloseOldLiveFileState {
        /// File number being closed.
        fnum_old:       FileNum,
        /// File number becoming live.
        fnum_new:       FileNum,
        /// Size of the new data file.
        new_dat_size:   u64,
        /// Size of the new index file.
        new_ind_size:   u64,
        /// Reply channel.
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    /// Commands a file bot to open a new live file state.
    OpenNewLiveFileState {
        /// File number to open.
        fnum_new:       FileNum,
        /// Initial size of the new data file.
        new_dat_size:   u64,
        /// Initial size of the new index file.
        new_ind_size:   u64,
        /// Reply channel.
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    // Request
    /// Requests that a data file be loaded into the cache.
    CacheDataFile {
        /// File to cache.
        fnum:           FileNum,
        /// Size of the data file in bytes.
        dat_file_size:  usize,
        /// Reply channel.
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    /// Requests that an index file be loaded into the cache.
    CacheIndexFile{
        /// File to cache.
        fnum:           FileNum,
        /// Size of the data file in bytes.
        dat_file_size:  usize,
        /// Size of the index file in bytes.
        ind_file_size:  usize,
        /// Reply channel.
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    /// Requests garbage collection of a file.
    CollectGarbage {
        /// File to collect.
        fnum:           FileNum,
        /// Current state of the file.
        fstat:          FileState,
        /// Index of the file bot that owns it.
        fbot_index:     usize,
    },
    //Delete(KeyVal, Responder<UIDL, UID, ENC, KH>),
    /// Requests a dump of a cache's contents.
    DumpCacheRequest(Responder<UIDL, UID, ENC, KH>),
    /// Requests a directory listing of a zone's files.
    DumpFiles(Responder<UIDL, UID, ENC, KH>),
    /// Requests a dump of a zone's file states.
    DumpFileStatesRequest(Responder<UIDL, UID, ENC, KH>),
    /// Requests that a cache apply the given post-GC location updates.
    GcCacheUpdateRequest(Vec<(Vec<u8>, FileLocation)>, Responder<UIDL, UID, ENC, KH>),
    //GetUsers(Responder<UIDL, UID, ENC, KH>),
    /// Requests a zone's directory.
    GetZoneDir(Responder<UIDL, UID, ENC, KH>),
    /// Requests insertion of a stored key/value into a cache and index. Carries
    /// the stored key bytes, optional value bytes, optional chunk index, the
    /// file location, the stored index length, the metadata and a reply channel.
    Insert(
        Vec<u8>,
        Option<Vec<u8>>,
        Option<usize>,
        FileLocation,
        usize, // stored index length
        Meta<UIDL, UID>,
        Responder<UIDL, UID, ENC, KH>,
    ),
    NewLiveFile(Option<FileNum>, Responder<UIDL, UID, ENC, KH>), // Explicit file number for init, None for routine new file.
    NextLiveFile(Responder<UIDL, UID, ENC, KH>), // A routine request by a wbot to the zbot for the next live file.
    /// Requests the overall state of every zone.
    OzoneStateRequest(Responder<UIDL, UID, ENC, KH>),
    /// A liveness probe from the named bot.
    Ping(OzoneBotId, Responder<UIDL, UID, ENC, KH>),
    /// A liveness reply from the named bot.
    Pong(OzoneBotId),
    /// Requests a value read, given the key, the owning cache-bot pool index and a reply channel.
    Read(Key, usize, Responder<UIDL, UID, ENC, KH>),
    /// Signals that a bot is ready.
    Ready,
    /// Requests a read of a key directly from the cache.
    ReadCache(Key, Responder<UIDL, UID, ENC, KH>),
    /// Requests a read of a value from a file at the given metadata location.
    ReadFileRequest(FileNum, MetaLocation<UIDL, UID>, Responder<UIDL, UID, ENC, KH>),
    /// Walk a zone's index files and return every live entry that
    /// satisfies `opts`. Dispatched to an igbot per zone by the
    /// scan coordinator in `O3db::scan`. The igbot is responsible
    /// for chash-based deduplication within its zone; the
    /// coordinator stitches the per-zone results and applies the
    /// final prefix and limit filters.
    ///
    /// `schms2` supplies the per-call encryption scheme override
    /// that the igbot must use when decrypting value payloads for
    /// `opts.include_values == true`. Ignored when values are
    /// skipped.
    ScanRequest {
        opts:   ScanOpts,
        schms2: Option<RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
    },
    /// Requests an orderly shutdown, initiated by the named bot.
    Shutdown(OzoneBotId, Responder<UIDL, UID, ENC, KH>),
    /// Requests a write of a framed key/value record.
    Write {
        /// Stored (framed) key bytes.
        kstored:    Vec<u8>,
        /// Stored (framed) value bytes.
        vstored:    Vec<u8>,
        /// Length of the key as held in the cache.
        klen_cache: usize,
        /// Chunk index, if this record is a chunk.
        cind:       Option<usize>,
        /// Record metadata.
        meta:       Meta<UIDL, UID>,
        /// Cache-bot pool index owning the key.
        cbpind:     usize,
        /// Reply channel.
        resp:       Responder<UIDL, UID, ENC, KH>,
    },
    // Respond
    Chunks(usize), // Number of chunks.
    /// Carries a cache's contents in reply to a dump request.
    DumpCacheResponse(WorkerInd, Cache<UIDL, UID>),
    /// Carries a zone's file states in reply to a dump request.
    DumpFileStatesResponse(WorkerInd, FileStateMap),
    /// Reports an error back to the requester.
    Error(Error<ErrTag>),
    /// Carries a zone's directory listing in reply to a files request.
    Files(ZoneInd, BTreeMap<String, FileEntry>),
    /// Carries the file locations updated by a post-GC cache update.
    GcCacheUpdateResponse(Vec<FileLocation>),
    /// Reports whether a key already existed on a write.
    KeyExists(bool),
    KeyChunkExists(bool, usize), // includes chunk index
    /// A generic success acknowledgement.
    Ok,
    //OkFrom(OzoneBotId),
    /// Carries the per-zone states in reply to an ozone-state request.
    OzoneStateResponse(Vec<ZoneState>),
    /// A zone's contribution to a scan in progress. Emitted by an
    /// igbot in response to [`OzoneMsg::ScanRequest`]. Contains the
    /// live entries the zone's index walker found, after per-zone
    /// chash deduplication. Corrupt entries are logged at `warn!`
    /// level and elided from this vector in v1; a future richer
    /// return type can surface them explicitly.
    ScanEntries(Vec<(Dat, Dat, Meta<UIDL, UID>)>),
    //UserKeys(Vec<(u128, Dat)>),
    /// Instructs a writer bot to adopt the given file as its live file.
    UseLiveFile(FileNum),
    /// Carries a read value back to the requester.
    Value(Value<UIDL, UID>),
    /// Carries the full result of a cache read, including location metadata.
    ReadResult(ReadResult<UIDL, UID>),
    // Wrap
    /// Wraps a message buffered during garbage collection for later processing.
    ProcessGcBuffer(Box<OzoneMsg<UIDL, UID, ENC, KH>>),
    // Server
    /// A server-level get request: fetch a value by key.
    Get {
        /// Key to fetch.
        key:    Dat,
        /// Optional scheme overrides.
        schms2: Option<RestSchemesOverride<ENC, KH>>,
        /// Reply channel.
        resp:   Responder<UIDL, UID, ENC, KH>,
    },
    /// Carries the result of a server-level get.
    GetResult(Option<(Dat, Meta<UIDL, UID>)>),
    /// A server-level put request: store a key/value for a user.
    Put {
        /// Key to store.
        key:    Dat,
        /// Value to store.
        val:    Dat,
        /// User the write is attributed to.
        user:   UID,
        /// Optional scheme overrides.
        schms2: Option<RestSchemesOverride<ENC, KH>>,
        /// Reply channel.
        resp:   Responder<UIDL, UID, ENC, KH>,
    },
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
>
    BotMsg<ErrTag> for OzoneMsg<UIDL, UID, ENC, KH> {}
