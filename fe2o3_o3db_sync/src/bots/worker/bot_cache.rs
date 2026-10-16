use crate::{
    prelude::*,
    base::id,
    bots::{
        base::bot_deps::*,
        worker::{
            bot_reader::ReadResult,
            worker_deps::*,
        },
    },
    comm::{
        msg::{
            HeadTicket,
            StoreRole,
        },
        response::decode_stored,
    },
    data::{
        cache::{
            Cache,
            Presence,
            Prior,
            Superseded,
            ValueOrLocation,
            is_chunk_key,
        },
        core::Key,
    },
    file::{
        floc::{
            FileLocation,
            FileNum,
        },
        stored::RecordDigest,
    },
    test::hooks,
};

use oxedyne_fe2o3_core::channels::Recv;
use oxedyne_fe2o3_iop_db::api::{
    Meta,
    RestSchemesOverride,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    chunk::PartKey,
    id::NumIdDat,
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    mem,
    sync::{
        Arc,
        atomic::Ordering,
    },
    time::Instant,
};

// What a store's head displaced, which is what its set retire is resolved by.
enum Displaced<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
> {
    Itself,                                         // a newer record was already cached
    Record { meta: Meta<UIDL, UID>, prior: Prior }, // the cached record, now old
}

// A set to retire: its part key, the store's metadata and its schemes.
type Retire<const UIDL: usize, UID, ENC, KH> =
    (PartKey, Meta<UIDL, UID>, Option<RestSchemesOverride<ENC, KH>>);

/// Waits on a barrier covering its winner (F1, R1).
#[derive(Debug)]
enum Held<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
> {
    Effect {                                            // what a supersession discards
        old:    (FileLocation, RecordDigest),           // the loser, to mark old
        retire: Option<Retire<UIDL, UID, ENC, KH>>,     // the set a store's head displaced
    },
    Answer(Responder<UIDL, UID, ENC, KH>, OzoneMsg<UIDL, UID, ENC, KH>), // a store's chunk
}

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
    held:       BTreeMap<(FileNum, u64), Vec<Held<UIDL, UID, ENC, KH>>>, // by the winner's end
    owed:       BTreeSet<FileNum>,  // files whose writer was asked for a barrier since their last
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
                                self.cache().pinned_heads(),
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
                        OzoneMsg::Settle(resp) => {
                            // Everything this bot has inserted has had its `UpdateData` sent, so
                            // the file bots reach the request after it.
                            let result = self.fbots().and_then(|bots| {
                                bots.send_to_all(OzoneMsg::Settle(resp.clone()))
                            });
                            if let Err(e) = result {
                                self.error(e.clone());
                                self.respond(Err(e), &resp);
                            }
                        },
                        OzoneMsg::SetCacheSizeLimit(size_lim) => {
                            self.cache_mut().set_lim(size_lim);
                        },
                        // WRITE
                        OzoneMsg::GcCacheUpdateRequest(buf, resp_g1) => {
                            let mut old_flocs = Vec::new();
                            for (key, floc, meta) in buf {
                                if let Some(old_floc) = self.cache_mut().reanchor(&key, &floc, &meta) {
                                    match RecordDigest::new(&key, &meta) {
                                        Ok(rid) => old_flocs.push((old_floc, rid)),
                                        // Its move entry stays, and keeps the file from being
                                        // collected again; the location itself is right.
                                        Err(e) => self.error(err!(e,
                                            "{}: Naming a record a collection re-anchored.", self.ozid();
                                            Data, Encode)),
                                    }
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
                        OzoneMsg::RecordsGone(fnum, rids) => {
                            let result = match self.cache_mut().records_gone(fnum, &rids) {
                                Ok(dropped) => self.retire_tombs(dropped),
                                Err(e) => Err(e),
                            };
                            self.result(&result);
                        },
                        OzoneMsg::Durable(z, fnum) => {
                            let result = self.drain(z, fnum);
                            self.result(&result);
                        },
                        OzoneMsg::ReplayDone => {
                            let result = match self.cache_mut().replay_done() {
                                Ok(dropped) => self.retire_tombs(dropped),
                                Err(e) => Err(e),
                            };
                            self.result(&result);
                        },
                        OzoneMsg::Insert(key, val, cind, floc, ilen, meta, resp_w1, unconfirmed, role) => {
                            let result = self.insert(key, val, cind, floc, ilen, meta, resp_w1, unconfirmed, role);
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
                            let result = self.read(key, resp_r2);
                            self.result(&result);
                        },
                        OzoneMsg::RecordsOf(keys, resp) => {
                            let held = keys.iter()
                                .map(|k| self.cache.record_at(k).map(|(m, f)| (m.clone(), f.clone())))
                                .collect();
                            self.respond(Ok(OzoneMsg::RecordsAt(held)), &resp);
                        },
                        OzoneMsg::PresenceOf(keys, resp) => {
                            let result = self.presence(&keys).map(OzoneMsg::Presences);
                            self.respond(result, &resp);
                        },
                        OzoneMsg::HeadsStamped(metas, set_ids, resp) => {
                            let heads = self.cache.heads_stamped(&metas, &set_ids);
                            self.respond(Ok(OzoneMsg::HeadsAt(heads)), &resp);
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
            held:       BTreeMap::new(),
            owed:       BTreeSet::new(),
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

    /// Flags each dropped tombstone's own record old, so that its bytes are collected in turn.
    fn retire_tombs(&mut self, dropped: Vec<(FileLocation, RecordDigest)>) -> Outcome<()> {
        if dropped.is_empty() {
            return Ok(());
        }
        let bots = res!(self.fbots());
        for (floc, rid) in dropped {
            let (bot, _) = bots.choose_bot(&ChooseBot::ByFile(floc.file_number()));
            res!(bot.send(OzoneMsg::ScheduleOld(floc, rid, self.ozid().clone())));
        }
        Ok(())
    }

    /// Enters a written record, and answers its caller at once, but a store's chunk only once a
    /// barrier covers it (R1).  What the record's supersession discards, the loser's bytes and
    /// for a store's head the chunk set it displaced, waits on the winner's mark: a winner that
    /// a power cut or a failed sync could remove would otherwise take its loser with it (F1).
    pub fn insert(
        &mut self,
        key:        Vec<u8>,
        val:        Option<Vec<u8>>,
        cind:       Option<usize>,
        floc:       FileLocation,
        ilen:       usize,
        meta:       Meta<UIDL, UID>,
        resp_w1:    Responder<UIDL, UID, ENC, KH>,
        unconfirmed: Option<Error<ErrTag>>,
        role:       Option<StoreRole<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        hooks::insert_delay();
        hooks::chunk_tombstone_delay(&key, cind);
        hooks::chunk_insert_delay(cind);
        // [12] Insert the data into the key-chosen zone cache.
        let floc_new = floc.clone();
        let noted = match is_chunk_key(&key) {
            true    => Some((key.clone(), meta.clone())),
            false   => None,
        };
        let is_head = matches!(role, Some(StoreRole::Head(_)));
        let inserted = if is_head && hooks::head_insert_fails() {
            Err(err!("A store's head could not be entered in the cache (test::hooks)."; Data, Write))
        } else {
            self.cache.insert(key, val, cind, floc, meta)
        };
        let sup = match inserted {
            Ok(sup) => sup,
            Err(e) => {
                // The caller is waiting on this answer.  Only logged, a failure here reached it
                // as an expired durability deadline, which says the write is on its way.  A
                // store's head keeps its set held: the record is in the files and comes back at
                // the next replay naming it.
                let e = err!(e,
                    "{}: A written record could not be entered in the cache.", self.ozid();
                    Data, Write, Unconfirmed);
                self.respond(Err(e.clone()), &resp_w1);
                return Err(e);
            },
        };
        // A tombstone noted early is only kept longer, which is safe.
        if let Some((k, m)) = noted {
            self.cache.note(&k, cind, &floc_new, &m, &sup);
        }
        let key_present = sup.old().is_some();
        // The head is in the cache, so the sweep sees its set live, or retires it as an orphan
        // only once the sweep's own barrier has made the winner durable.
        let (chunk, ticket) = match role {
            Some(StoreRole::Head(t)) => {
                self.release_set(t.set);
                (false, Some(t))
            },
            Some(StoreRole::Chunk)  => (true, None),
            None                    => (false, None),
        };

        // [13] Inform the caller of the file write and the cache insertion, or of the barrier
        //      that failed after the write.  A store's chunk is told only once a barrier covers
        //      it, so that its head never becomes durable before it.
        match unconfirmed {
            Some(e) => {
                self.respond(Err(e), &resp_w1);
                self.respond(Ok(OzoneMsg::Finish), &resp_w1);
            },
            None => {
                let answer = match cind {
                    Some(cind)  => OzoneMsg::KeyChunkExists(key_present, cind),
                    None        => OzoneMsg::KeyExists(key_present),
                };
                res!(self.hold_or_fire(&floc_new, match chunk {
                    true    => Some(Held::Answer(resp_w1, answer)),
                    false   => {
                        self.respond(Ok(answer), &resp_w1);
                        self.respond(Ok(OzoneMsg::Finish), &resp_w1);
                        None
                    },
                }));
            },
        }

        // [14] What the supersession discards waits on the winner: the offered record when it
        //      won, the cached one when the offered record lost.
        let bots = res!(self.fbots());
        let (bot, _) = bots.choose_bot(
            &ChooseBot::ByFile(floc_new.file_number())
        );
        let (gate, old, displaced) = match sup {
            Superseded::None => {
                res!(bot.send(OzoneMsg::UpdateData {
                    floc_new:       floc_new.clone(),
                    ilen,
                    floc_old_opt:   None,
                    from_id:        self.ozid().clone(),
                }));
                return Ok(());
            },
            Superseded::Offered { floc, rid, winner } =>
                (winner, (floc, rid), Displaced::Itself),
            Superseded::Cached { floc, rid, meta, prior } =>
                (floc_new.clone(), (floc, rid), Displaced::Record { meta, prior }),
        };
        let retire = match &ticket {
            Some(t) => self.resolve_retire(displaced, t),
            None    => None,
        };

        // [15] Insert the new data into the file state data map, via a file-selected fbot.  The
        //      old-marking rides in it when a barrier already covers the winner, so the fbot
        //      applies both before it weighs a collection; sent apart, a collection could start
        //      between them and carry the loser into the rewritten file as current.
        let now = res!(self.api().chans().marks().covers(self.wind.z(), &gate));
        res!(bot.send(OzoneMsg::UpdateData {
            floc_new:       floc_new.clone(),
            ilen,
            floc_old_opt:   if now { Some(old) } else { None },
            from_id:        self.ozid().clone(),
        }));
        if now {
            return self.fire_retire(retire);
        }
        self.hold_or_fire(&gate, Some(Held::Effect { old, retire }))
    }

    // Fires `item` if a barrier covers `gate`, or holds it until one does.
    fn hold_or_fire(
        &mut self,
        gate: &FileLocation,
        item: Option<Held<UIDL, UID, ENC, KH>>,
    )
        -> Outcome<()>
    {
        let item = match item {
            Some(item)  => item,
            None        => return Ok(()),
        };
        if res!(self.api().chans().marks().covers(self.wind.z(), gate)) {
            return self.fire(item);
        }
        if let Held::Answer(resp, _) = &item {
            // No mark moves after a failed barrier, so nothing would ever release it.
            if self.api().chans().fault().is_set() {
                self.fail_answer(resp);
                return Ok(());
            }
        }
        if let Held::Effect { .. } = item {
            self.api().chans().held_effects().fetch_add(1, Ordering::SeqCst);
            // Nothing else is bound to move the mark under `Never` or `EveryN`, so the writer
            // that owns the file is asked for a barrier, once until it has given one.  A held
            // chunk answer has its own request, from the store that waits on it.
            if !self.owed.contains(&gate.fnum) {
                let marks = self.api().chans().marks();
                if let Some(b) = res!(marks.owner(self.wind.z(), gate.fnum)) {
                    res!(self.api().owe(self.wind.zind(), b));
                    self.owed.insert(gate.fnum);
                }
            }
        }
        let end = gate.start + gate.klen + gate.vlen;
        self.held.entry((gate.fnum, end)).or_default().push(item);
        Ok(())
    }

    // The mark of `fnum` has moved: fires everything it now covers.  After a failed barrier no
    // mark moves again, so a store's chunk still waiting is told it failed; held discards stay
    // held, and the restart's cut and replay settle them.
    fn drain(&mut self, z: usize, fnum: FileNum) -> Outcome<()> {
        if z != self.wind.z() {
            return Ok(());
        }
        self.owed.remove(&fnum);
        let upto = match res!(self.api().chans().marks().get(z, fnum)) {
            Some((d, _))    => d,
            None            => u64::MAX,
        };
        let keys: Vec<(FileNum, u64)> = self.held.range((fnum, 0)..=(fnum, upto))
            .map(|(k, _)| *k)
            .collect();
        let mut result = Ok(());
        for items in keys.iter().filter_map(|k| self.held.remove(k)).collect::<Vec<_>>() {
            for item in items {
                if let Held::Effect { .. } = item {
                    self.api().chans().held_effects().fetch_sub(1, Ordering::SeqCst);
                }
                if let Err(e) = self.fire(item) {
                    result = Err(e);
                }
            }
        }
        if self.api().chans().fault().is_set() {
            let held = mem::take(&mut self.held);
            for (k, items) in held {
                let mut keep = Vec::new();
                for item in items {
                    match item {
                        Held::Answer(resp, _)   => self.fail_answer(&resp),
                        effect                  => keep.push(effect),
                    }
                }
                if !keep.is_empty() {
                    self.held.insert(k, keep);
                }
            }
        }
        result
    }

    fn fail_answer(&self, resp: &Responder<UIDL, UID, ENC, KH>) {
        let (when, cause) = match self.api().chans().fault().get() {
            Some(f) => (format!("{:?}", f.when), f.error),
            None    => (String::from("?"), String::new()),
        };
        self.respond(Err(err!(
            "{}: A chunk of this store was written, but the store stopped writing after a failed \
            sync at {} ({}) before a barrier covered it, so the value was not stored.  Restart \
            the store; it recovers from the disk.", self.ozid(), when, cause;
            Write, Fatal, Unconfirmed)), resp);
    }

    // A chunk's answer goes to its store; a discard marks the loser old, then retires the set a
    // head displaced.
    fn fire(&mut self, item: Held<UIDL, UID, ENC, KH>) -> Outcome<()> {
        match item {
            Held::Answer(resp, answer) => {
                self.respond(Ok(answer), &resp);
                self.respond(Ok(OzoneMsg::Finish), &resp);
            },
            Held::Effect { old: (floc, rid), retire } => {
                let bots = res!(self.fbots());
                let (bot, _) = bots.choose_bot(&ChooseBot::ByFile(floc.file_number()));
                res!(bot.send(OzoneMsg::ScheduleOld(floc, rid, self.ozid().clone())));
                res!(self.fire_retire(retire));
            },
        }
        Ok(())
    }

    // Sends the tombstones of the set a head displaced, stamped now and waited on by no one.  A
    // tombstone that fails leaves an orphan for the sweep.
    fn fire_retire(&mut self, retire: Option<Retire<UIDL, UID, ENC, KH>>) -> Outcome<()> {
        if let Some((pkey, mut tmeta, schms2)) = retire {
            hooks::trace(hooks::Step::Durable);
            if hooks::retire_dropped() {
                return Ok(());
            }
            let none = Responder::none(Some(self.ozid()));
            let sent = crate::api::retire_stamp().and_then(|t| {
                tmeta.time = t;
                self.api().send_retires(&pkey, &tmeta, schms2.as_ref(), &none)
            });
            if let Err(e) = sent {
                self.not_retired(&pkey, e);
            }
        }
        Ok(())
    }

    // The set a store's head displaced, or its own when a newer record was already cached.  A
    // set a durable record has displaced is named by no current head again, since no write
    // reuses a set.  A legacy key-derived set is retired once, by the first ticketed head over
    // it, and never by a store whose own set it is.  A delete that loses has no set.
    fn resolve_retire(
        &self,
        displaced:  Displaced<UIDL, UID>,
        t:          &HeadTicket<UIDL, UID, ENC, KH>,
    )
        -> Option<Retire<UIDL, UID, ENC, KH>>
    {
        let own = t.own.as_ref();
        let pkey = match displaced {
            Displaced::Itself => t.own.clone(),
            Displaced::Record { meta, prior } => match prior {
                Prior::NotHead      => None,
                Prior::Head(bytes)  => match self.decode_head(&bytes, t) {
                    Ok(pkey) => pkey.filter(|p| Some(p) != own),
                    Err(e) => {
                        self.not_retired(&bytes, e);
                        None
                    },
                },
                // A replayed head, whose bytes are not resident: it can only be the value the
                // store read before it began, if the metadata agrees.
                Prior::HeadUnread => match &t.pred {
                    Some((pkey, pmeta)) if *pmeta == meta && Some(pkey) != own =>
                        Some(pkey.clone()),
                    _ => None,
                },
            },
        };
        pkey.map(|p| (p, t.tmeta.clone(), t.schms2.clone()))
    }

    // The part key a displaced head named, if it named one.
    fn decode_head(
        &self,
        bytes:  &[u8],
        t:      &HeadTicket<UIDL, UID, ENC, KH>,
    )
        -> Outcome<Option<PartKey>>
    {
        let (dat, _) = res!(Dat::from_bytes(bytes));
        let or = t.schms2.as_ref().map(|s| s.encrypter());
        Ok(match res!(decode_stored(dat, self.api().schemes().encrypter(), or)) {
            Some(Dat::Tup5u64(tup)) => Some(PartKey(tup)),
            _                       => None,
        })
    }

    // The winner is durable; the old chunks are left to the orphan sweep.
    fn not_retired<T: std::fmt::Debug>(&self, what: &T, e: Error<ErrTag>) {
        warn!(sync_log::stream(),
            "{}: A durable write's displaced chunks, {:?}, of the value it replaced or its own \
            after a newer write won, were not all retired, and wait for the orphan sweep: {}",
            self.ozid(), what, e);
    }

    fn release_set(&self, set: Option<u64>) {
        if let Some(id) = set {
            if let Ok(mut held) = self.api().chans().pending_sets().lock() {
                held.release(&id);
            }
        }
    }

    /// Whether each stored key holds a value, from the index alone.  A stopped store answers only
    /// for records a barrier proved, as `read` serves only those.
    pub fn presence(&self, keys: &[Vec<u8>]) -> Outcome<Vec<Presence>> {
        let tomb = res!(Dat::Usr(id::usr_kind_id_deleted(), Some(Box::new(Dat::Empty))).as_bytes());
        let csum_len = res!(self.api().schemes().checksummer().len());
        let chans = self.api().chans();
        let fault = chans.fault().get();
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            if let Some(fault) = &fault {
                if let Some(floc) = self.cache.located(k) {
                    if !res!(chans.marks().covers(self.wind.z(), floc)) {
                        return Err(err!(
                            "{}: {}, and the record cached for a key asked after, at {:?}, was \
                            never proven durable, so its presence is not answered.  Restart the \
                            store; it recovers from the disk.", self.ozid(), fault.stopped(), floc;
                            Read, Fatal));
                    }
                }
            }
            out.push(self.cache.presence(k, &tomb, csum_len));
        }
        Ok(out)
    }

    pub fn read(
        &mut self,
        key:        Key,
        resp_r2:    Responder<UIDL, UID, ENC, KH>,
    )
        -> Outcome<()>
    {
        // A stopped store goes on serving reads, but only of records a barrier
        // proved: anything past a mark is cut at the restart, and must not be read and acted on
        // in the meantime (R4b).
        let chans = self.api().chans();
        if let Some(fault) = chans.fault().get() {
            if let Some(floc) = self.cache.located(key.as_bytes()) {
                if !res!(chans.marks().covers(self.wind.z(), floc)) {
                    let e = err!(
                        "{}: {}, and the record cached for this key, at {:?}, was never proven \
                        durable, so it is not served.  Restart the store; it recovers from the \
                        disk.", self.ozid(), fault.stopped(), floc;
                        Read, Fatal);
                    res!(resp_r2.send(OzoneMsg::Error(e)));
                    return Ok(());
                }
            }
        }
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
                        // The key goes with the location, so that a move entry at the same
                        // offset is taken only if it is this record's.
                        res!(bot.send(
                            OzoneMsg::ReadFileRequest(
                                fnum,
                                key.into_bytes(),
                                mloc,
                                resp_r2,
                        )));
                    },
                    // <6> Send result directly back to rbot.
                    ValueOrLocation::Value(val, meta) =>
                        res!(resp_r2.send(OzoneMsg::ReadResult(ReadResult::Value(val, meta)))),
                }
                
            },
            // <6> Send result directly back to rbot.
            None => res!(resp_r2.send(OzoneMsg::ReadResult(ReadResult::None))),
        }
        Ok(())
    }
}
