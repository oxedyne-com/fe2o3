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
            bot_file::{
                GcControl,
                Hold,
            },
            bot_reader::ReadResult,
        },
        bot_zone::ZoneState,
    },
    comm::{
        channels::{
            BotChannels,
            Cut,
        },
        response::Responder,
    },
    data::{
        cache::{
            Cache,
            MetaLocation,
            PinnedHeads,
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
        stored::RecordDigest,
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
    chunk::PartKey,
    id::NumIdDat,
};

use std::{
    collections::BTreeMap,
};

/// What a record sent by a `PendingStore` is to its store, which says how the cache bot answers.
#[derive(Clone, Debug)]
pub enum StoreRole<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    Chunk,                                  // answered once a barrier covers it (R1)
    Head(HeadTicket<UIDL, UID, ENC, KH>),   // answered at insert
}

/// A store's head.  It hands over the chunk set the store holds, released at insert, and carries
/// what the cache bot needs to retire the set the head displaces once the winner is durable.
#[derive(Clone, Debug)]
pub struct HeadTicket<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    pub set:    Option<u64>,                        // the hold handed over
    pub own:    Option<PartKey>,                    // the store's own chunk set
    pub pred:   Option<(PartKey, Meta<UIDL, UID>)>, // the value read before the store began
    pub schms2: Option<RestSchemesOverride<ENC, KH>>,
    pub tmeta:  Meta<UIDL, UID>,                    // the store's metadata, restamped to retire
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
>
    StoreRole<UIDL, UID, ENC, KH>
{
    /// The chunk set hold a head hands over, if any.
    pub fn set(&self) -> Option<u64> {
        match self {
            Self::Chunk     => None,
            Self::Head(t)   => t.set,
        }
    }
}

#[derive(Clone, Debug)]
pub enum OzoneMsg<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    None,
    // Advise
    CacheSize(usize, usize, usize, PinnedHeads),
    SetCacheSizeLimit(usize),
    Channels(BotChannels<UIDL, UID, ENC, KH>, Responder<UIDL, UID, ENC, KH>),
    ChannelsReceived(OzoneBotId),
    Config(OzoneConfig),
    //ConfigConfirm(OzoneBotId, Ticket),
    Cut(Vec<Cut>),          // syncer -> wbot, a barrier failed: cut these files to their marks
    Durable(usize, FileNum), // syncer -> cbot, the mark of (zone, file) has moved
    Finish,
    FileReplaced(FileNum, FileType), // a collection renamed a new file over this one, or compaction deleted it
    GcAborted(FileNum),     // a collection gave up before it replaced anything
    GcCompleted(FileNum, FileState, usize),
    InitTest,
    MessageCount(usize),
    NewFileStates(FileStateMap),
    ReadFinished(FileNum),
    RecordsGone(FileNum, Vec<RecordDigest>), // gbot/fbot -> cbot, old records durably off disk
    ReplayDone,             // zbot -> cbot, the start's replay has filled the cache
    ScheduleOld(FileLocation, RecordDigest, OzoneBotId),
    ShardFileSize(usize, usize),
    UpdateData {
        floc_new:       FileLocation,
        ilen:           usize,
        floc_old_opt:   Option<(FileLocation, RecordDigest)>, // the record superseded
        from_id:        OzoneBotId,
    },
    ZoneDir(ZoneInd, ZoneDir),
    ZoneInitTrigger(Responder<UIDL, UID, ENC, KH>), // sup -> cfg, each zone answers when ready
    ZoneInit(ZoneDir, ZoneConfig, Responder<UIDL, UID, ENC, KH>),
    ZoneState(usize, ZoneState),
    // Command
    GcControl(GcControl, Responder<UIDL, UID, ENC, KH>), // sup -> gbot, control gc activation
    ClearCache(Responder<UIDL, UID, ENC, KH>),
    Settle(Responder<UIDL, UID, ENC, KH>),      // api -> cbot -> fbot, the accounting barrier
    SettleEcho(Responder<UIDL, UID, ENC, KH>),  // fbot -> fbot, answered once the fbot reaches it
    SyncNow(Responder<UIDL, UID, ENC, KH>),     // api -> wbot -> syncer, a barrier over all before it
    SyncOwed,                                   // cbot -> wbot -> syncer, a held discard owes a barrier soon
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
    GcCacheUpdateRequest(Vec<(Vec<u8>, FileLocation, Meta<UIDL, UID>)>, Responder<UIDL, UID, ENC, KH>),
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
        Option<Error<ErrTag>>, // written, but its barrier failed: the caller's answer
        Option<StoreRole<UIDL, UID, ENC, KH>>, // a record of a store in progress
    ),
    NewLiveFile(Option<FileNum>, Responder<UIDL, UID, ENC, KH>), // Explicit file number for init, None for routine new file.
    NextLiveFile(Responder<UIDL, UID, ENC, KH>), // A routine request by a wbot to the zbot for the next live file.
    OzoneStateRequest(Responder<UIDL, UID, ENC, KH>),
    Ping(OzoneBotId, Responder<UIDL, UID, ENC, KH>),
    Pong(OzoneBotId, usize),
    Read(Key, usize, Responder<UIDL, UID, ENC, KH>),
    Ready,
    ReadCache(Key, Responder<UIDL, UID, ENC, KH>),
    ReadFileRequest(FileNum, Vec<u8>, MetaLocation<UIDL, UID>, Responder<UIDL, UID, ENC, KH>), // key bytes
    RecordsOf(Vec<Vec<u8>>, Responder<UIDL, UID, ENC, KH>), // stored keys, for the record each holds now
    HeadsStamped(Vec<Meta<UIDL, UID>>, Vec<u64>, Responder<UIDL, UID, ENC, KH>), // the current heads written with any of these metas, or under a key of these legacy set ids
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
        role:       Option<StoreRole<UIDL, UID, ENC, KH>>,
    },
    // Respond
    Chunks(usize), // Number of chunks.
    CompactReport {         // fbot -> caller of `compact_now`, one per file bot per round
        wind:       WorkerInd,
        started:    Vec<FileNum>,           // collections begun this round
        deleted:    Vec<FileNum>,           // files that held only old records, removed
        waiting:    Vec<(FileNum, Hold)>,   // files with old bytes, and what holds each
    },
    DumpCacheResponse(WorkerInd, Cache<UIDL, UID>),
    DumpFileStatesResponse(WorkerInd, FileStateMap),
    Error(Error<ErrTag>),
    Files(ZoneInd, BTreeMap<String, FileEntry>),
    GcCacheUpdateResponse(Vec<(FileLocation, RecordDigest)>), // re-anchored, from where
    KeyExists(bool),
    KeyChunkExists(bool, usize), // includes chunk index
    Ok,
    //OkFrom(OzoneBotId),
    OzoneStateResponse(Vec<ZoneState>),
    ScanEntries(Vec<(Dat, Dat, Meta<UIDL, UID>)>),
    //UserKeys(Vec<(u128, Dat)>),
    UseLiveFile(FileNum),
    Value(Value<UIDL, UID>),
    Written, // a write's record is appended; its durable answer follows
    ReadResult(ReadResult<UIDL, UID>),
    RecordsAt(Vec<Option<(Meta<UIDL, UID>, FileLocation)>>), // answers RecordsOf, in its order
    HeadsAt(Vec<(Vec<u8>, Meta<UIDL, UID>)>), // answers HeadsStamped: stored key and metadata
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
