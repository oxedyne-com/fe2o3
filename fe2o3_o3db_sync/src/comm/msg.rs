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

#[derive(Clone, Debug)]
pub enum OzoneMsg<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    None,
    // Advise
    CacheSize(usize, usize, usize),
    SetCacheSizeLimit(usize),
    Channels(BotChannels<UIDL, UID, ENC, KH>, Responder<UIDL, UID, ENC, KH>),
    ChannelsReceived(OzoneBotId),
    Config(OzoneConfig),
    //ConfigConfirm(OzoneBotId, Ticket),
    Finish,
    GcCompleted(FileNum, FileState, usize),
    InitTest,
    MessageCount(usize),
    NewFileStates(FileStateMap),
    ReadFinished(FileNum),
    ScheduleOld(FileLocation, OzoneBotId),
    ShardFileSize(usize, usize),
    UpdateData {
        floc_new:       FileLocation,
        ilen:           usize,
        floc_old_opt:   Option<FileLocation>,
        from_id:        OzoneBotId,
    },
    ZoneDir(ZoneInd, ZoneDir),
    ZoneInitTrigger,
    ZoneInit(ZoneDir, ZoneConfig),
    ZoneState(usize, ZoneState),
    // Command
    GcControl(GcControl, Responder<UIDL, UID, ENC, KH>), // sup -> gbot, control gc activation
    ClearCache(Responder<UIDL, UID, ENC, KH>),
    CloseOldLiveFileState {
        fnum_old:       FileNum,
        fnum_new:       FileNum,
        new_dat_size:   u64,
        new_ind_size:   u64,
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    OpenNewLiveFileState {
        fnum_new:       FileNum,
        new_dat_size:   u64,
        new_ind_size:   u64,
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    // Request
    CacheDataFile {
        fnum:           FileNum,
        dat_file_size:  usize,
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    CacheIndexFile{
        fnum:           FileNum,
        dat_file_size:  usize,
        ind_file_size:  usize,
        resp:           Responder<UIDL, UID, ENC, KH>,
    },
    CollectGarbage {
        fnum:           FileNum,
        fstat:          FileState,
        fbot_index:     usize,
    },
    //Delete(KeyVal, Responder<UIDL, UID, ENC, KH>),
    DumpCacheRequest(Responder<UIDL, UID, ENC, KH>),
    DumpFiles(Responder<UIDL, UID, ENC, KH>),
    DumpFileStatesRequest(Responder<UIDL, UID, ENC, KH>),
    GcCacheUpdateRequest(Vec<(Vec<u8>, FileLocation)>, Responder<UIDL, UID, ENC, KH>), 
    //GetUsers(Responder<UIDL, UID, ENC, KH>),
    GetZoneDir(Responder<UIDL, UID, ENC, KH>),
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
    OzoneStateRequest(Responder<UIDL, UID, ENC, KH>),
    Ping(OzoneBotId, Responder<UIDL, UID, ENC, KH>),
    Pong(OzoneBotId),
    Read(Key, usize, Responder<UIDL, UID, ENC, KH>),
    Ready,
    ReadCache(Key, Responder<UIDL, UID, ENC, KH>),
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
    Shutdown(OzoneBotId, Responder<UIDL, UID, ENC, KH>),
    Write {
        kstored:    Vec<u8>,
        vstored:    Vec<u8>,
        klen_cache: usize,
        cind:       Option<usize>,
        meta:       Meta<UIDL, UID>,
        cbpind:     usize,
        resp:       Responder<UIDL, UID, ENC, KH>,
    },
    // Respond
    Chunks(usize), // Number of chunks.
    DumpCacheResponse(WorkerInd, Cache<UIDL, UID>),
    DumpFileStatesResponse(WorkerInd, FileStateMap),
    Error(Error<ErrTag>),
    Files(ZoneInd, BTreeMap<String, FileEntry>),
    GcCacheUpdateResponse(Vec<FileLocation>),
    KeyExists(bool),
    KeyChunkExists(bool, usize), // includes chunk index
    Ok,
    //OkFrom(OzoneBotId),
    OzoneStateResponse(Vec<ZoneState>),
    /// A zone's contribution to a scan in progress. Emitted by an
    /// igbot in response to [`OzoneMsg::ScanRequest`]. Contains the
    /// live entries the zone's index walker found, after per-zone
    /// chash deduplication. Corrupt entries are logged at `warn!`
    /// level and elided from this vector in v1; a future richer
    /// return type can surface them explicitly.
    ScanEntries(Vec<(Dat, Dat, Meta<UIDL, UID>)>),
    //UserKeys(Vec<(u128, Dat)>),
    UseLiveFile(FileNum),
    Value(Value<UIDL, UID>),
    ReadResult(ReadResult<UIDL, UID>),
    // Wrap
    ProcessGcBuffer(Box<OzoneMsg<UIDL, UID, ENC, KH>>),
    // Server
    Get {
        key:    Dat,
        schms2: Option<RestSchemesOverride<ENC, KH>>,
        resp:   Responder<UIDL, UID, ENC, KH>,
    },
    GetResult(Option<(Dat, Meta<UIDL, UID>)>),
    Put {
        key:    Dat,
        val:    Dat,
        user:   UID,
        schms2: Option<RestSchemesOverride<ENC, KH>>,
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
