use crate::{
    prelude::*,
    bots::{
        base::bot_deps::*,
        worker::{
            bot_reader::ReadResult,
            worker_deps::*,
        },
    },
    data::{
        cache::{
            Cache,
            ValueOrLocation,
        },
        core::Key,
    },
    file::floc::FileLocation,
};

use oxedyne_fe2o3_core::channels::Recv;
use oxedyne_fe2o3_iop_db::api::Meta;
use oxedyne_fe2o3_jdat::id::NumIdDat;

use std::{
    sync::Arc,
    time::Instant,
};

#[derive(Debug)]
pub struct CacheBot<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
	PR:     Hasher,
    CS:     Checksummer,
>{
    // Identity
    wind:       WorkerInd,
    wtyp:       WorkerType,
    // Bot
    sem:            Semaphore,
    errc:           Arc<Mutex<usize>>,
    log_stream_id:  String,
    // Config
    zdir:       ZoneDir,
    // Comms
    chan_in:    Simplex<OzoneMsg<UIDL, UID, ENC, KH>>,
    // API
    api:        OzoneApi<UIDL, UID, ENC, KH, PR, CS>,
    // State
    active:     bool,
    cache:      Cache<UIDL, UID>,
    inited:     bool,
    trep:       Instant,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    WorkerBot<UIDL, UID, ENC, KH, PR, CS> for CacheBot<UIDL, UID, ENC, KH, PR, CS>
{
    workerbot_methods!();
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    OzoneBot<UIDL, UID, ENC, KH, PR, CS> for CacheBot<UIDL, UID, ENC, KH, PR, CS>
{
    ozonebot_methods!();
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    Bot<{ BID_LEN }, Bid, OzoneMsg<UIDL, UID, ENC, KH>> for CacheBot<UIDL, UID, ENC, KH, PR, CS>
{
    bot_methods!();

    fn go(&mut self) {

        sync_log::set_stream(self.log_stream_id());

        if self.no_init() { return; }
        self.now_listening();
        loop {
            if self.wind().b() < self.cfg().num_bots_per_zone(self.wtyp()) {

                if self.trep.elapsed() > self.cfg().zone_state_update_interval() {
                    // Automated state reporting.
                    self.trep = Instant::now();
                    if let Some(zbot) = self.zbot() {
                        if let Err(e) = zbot.send(
                            OzoneMsg::CacheSize(
                                self.wind().b(),
                                self.cache().get_size(),
                                self.cache().get_ancillary_size(),
                            )
                        ) {
                            self.result(&Err(err!(e,
                                "{}: Cannot send cache size update to zbot.", self.ozid();
                                Channel, Write)));
                        }
                    }
                }
            
                if self.listen().must_end() { break; }

            } else {
                // This bot is to be terminated. Forward incoming messages to the remaining bots of
                // this type.
            }
        }
    }

    fn listen(&mut self) -> LoopBreak {
        match self.chan_in().recv_timeout(self.cfg().zone_state_update_interval()) {
            Recv::Result(Err(e)) => self.err_cannot_receive(err!(e,
                "{}: Waiting for message.", self.ozid();
                IO, Channel)),
            Recv::Result(Ok(msg)) => {
                if let Some(msg) = self.listen_worker(msg) {
                    match msg {
                        // COMMAND
                        OzoneMsg::ClearCache(resp) => {
                            self.cache_mut().clear_all_values(); 
                            self.respond(Ok(OzoneMsg::Ok), &resp);
                        },
                        OzoneMsg::SetCacheSizeLimit(size_lim) => {
                            self.cache_mut().set_lim(size_lim);
                        },
                        // WRITE
                        OzoneMsg::GcCacheUpdateRequest(buf, resp_g1) => {
                            let mut old_flocs = Vec::new();
                            for (key, floc) in buf {
                                if let Some(old_floc) = self.cache_mut().update_if_same_fnum(&key, &floc) {
                                    old_flocs.push(old_floc);
                                }
                            }
                            if let Err(e) = resp_g1.send(
                                OzoneMsg::GcCacheUpdateResponse(old_flocs)
                            ) {
                                self.err_cannot_send(err!(e,
                                    "{}: Sending cache update response back to gbot.", self.ozid();
                                    IO, Channel));
                            }
                        },
                        OzoneMsg::Insert(key, val, cind, floc, ilen, meta, resp_w1) => {
                            let result = self.insert(key, val, cind, floc, ilen, meta, resp_w1);
                            self.result(&result);
                        },
                        // READ
                        OzoneMsg::DumpCacheRequest(resp) => {
                            if let Err(e) = resp.send(OzoneMsg::DumpCacheResponse(
                                self.wind().clone(),
                                self.cache().clone(),
                            )) {
                                self.err_cannot_send(err!(e,
                                    "{}: Responding to {:?} with cache dump.", self.ozid(), resp.ozid();
                                    Data, IO, Channel));
                            }
                        },
                        //OzoneMsg::GetUsers(resp) => {
                        //    let mut kuserdat = Vec::new();
                        //    let prefix = constant::USER_DAT.byte_prefix();
                        //    for (k, _) in self.cache().map() {
                        //        if k.len() > UsrKindId::CODE_BYTE_LEN {
                        //            if k[0] == Dat::USR_CODE {
                        //                if UsrKindId::prefix_matches(prefix, &k[1..]) {
                        //                    let kdat = match Dat::from_bytes(&k) {
                        //                        Ok((dat, _)) => dat,
                        //                        _ => continue,
                        //                    };
                        //                    if let Dat::Usr(_, optboxdat) = &kdat {
                        //                        match optboxdat {
                        //                            Some(boxdat) => match **boxdat {
                        //                                Dat::U128(id) => {
                        //                                    kuserdat.push((id, kdat.clone()));
                        //                                },
                        //                                dat => self.error(err!(
                        //                                    "Custom Usr daticle should contain a \
                        //                                    Dat::U128 but found {:?}.", dat,
                        //                                ), Bug, Invalid, Input)),
                        //                            },
                        //                            None => self.error(err!(
                        //                                "Custom Usr daticle should contain \
                        //                                something but None was found.",
                        //                            ), Bug, Invalid, Input)),
                        //                        }
                        //                    }
                        //                }
                        //            }
                        //        }
                        //    }
                        //    self.respond(Ok(OzoneMsg::UserKeys(kuserdat)), &resp);
                        //},
                        OzoneMsg::ReadCache(key, resp_r2) => {
                            let result = self.read(&key, resp_r2);
                            self.result(&result);
                        },
                        _ => return self.listen_more(msg),
                    }
                }
            },
            Recv::Empty => (),
        }
        LoopBreak(false)
    }

}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
	PR:     Hasher,
    CS:     Checksummer,
>
    CacheBot<UIDL, UID, ENC, KH, PR, CS>
{
    pub fn new(
        args: ZoneWorkerInitArgs<UIDL, UID, ENC, KH, PR, CS>,
    )
        -> Self
    {
        let cache = Cache::new(Some(&args.api.ozid));
        Self {
            // Identity
            wind:       args.wind,
            wtyp:       args.wtyp,
            // Bot
            sem:            args.sem,
            errc:           Arc::new(Mutex::new(0)),
            log_stream_id:  args.log_stream_id,
            // Config
            zdir:       ZoneDir::default(),
            // Comms    
            chan_in:    args.chan_in,
            // API
            api:        args.api,
            // State
            active:     false,
            cache,
            inited:     false,
            trep:       Instant::now(),
        }
    }

    fn cache(&self)         -> &Cache<UIDL, UID>       { &self.cache }
    fn cache_mut(&mut self) -> &mut Cache<UIDL, UID>   { &mut self.cache }

    pub fn max_file_len(&self) -> usize {
        self.cfg().data_file_max_bytes as usize
    }

    pub fn activate(mut self) -> Self {
        self.active = true;
        self
    }

    pub fn insert(
        &mut self,
        key:        Vec<u8>,
        val:        Option<Vec<u8>>,
        cind:       Option<usize>,
        floc:       FileLocation,
        ilen:       usize,
        meta:       Meta<UIDL, UID>,
        resp_w1:    Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        // [12] Insert the data into the key-chosen zone cache.
        let floc_new = floc.clone();
        let floc_old_opt = res!(self.cache.insert(
            key,
            val,
            floc,
            meta,
        ));

        let key_present = floc_old_opt.is_some();
        
        // [13] Inform the caller of successful file write and cache insertion.
        match cind {
            Some(cind) => self.respond(Ok(OzoneMsg::KeyChunkExists(key_present, cind)), &resp_w1),
            None => self.respond(Ok(OzoneMsg::KeyExists(key_present)), &resp_w1),
        }
        self.respond(Ok(OzoneMsg::Finish), &resp_w1);

        // [14] Insert the new data into the file state data map, via a file-selected cbot.
        let bots = res!(self.fbots());
        let (bot, _) = bots.choose_bot(
            &ChooseBot::ByFile(floc_new.file_number())
        );
        res!(bot.send(OzoneMsg::UpdateData {
            floc_new,
            ilen,
            floc_old_opt,
            from_id: self.ozid().clone(),
        }));

        Ok(())
    }

    pub fn read(
        &mut self,
        key:        &Key,
        resp_r2:    Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        // <3> The cbot accesses its cache.
        match res!(self.cache.get(key.as_bytes())) {
            Some(vloc) => {
                match vloc {
                    ValueOrLocation::Location(mloc) => {
                        // <4> Only the file location is available, so send a request to the
                        // appropriate fbot, forwarding the responder.
                        let fnum = mloc.file_number();
                        let bots = res!(self.fbots());
                        let (bot, _) = bots.choose_bot(&ChooseBot::ByFile(fnum));
                        res!(bot.send(
                            OzoneMsg::ReadFileRequest(
                                fnum,
                                mloc.clone(),
                                resp_r2,
                        )));
                    },
                    // <6> Send result directly back to rbot.
                    ValueOrLocation::Value(val, meta) =>
                        res!(resp_r2.send(OzoneMsg::ReadResult(ReadResult::Value(val, meta)))),
                    ValueOrLocation::Deleted(meta) =>
                        res!(resp_r2.send(OzoneMsg::ReadResult(ReadResult::Deleted(meta)))),
                }
                
            },
            // <6> Send result directly back to rbot.
            None => res!(resp_r2.send(OzoneMsg::ReadResult(ReadResult::None))),
        }
        Ok(())
    }
}
