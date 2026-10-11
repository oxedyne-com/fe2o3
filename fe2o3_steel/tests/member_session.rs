//! `srv::member::session` against a real Ozone store, under the target directory rather than
//! `/tmp`.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod member {

    use oxedyne_fe2o3_steel::{
        app::{
            constant as app_const,
            server::VhostDb,
        },
        srv::{
            context::new_db,
            id::Uid,
            member::session::{
                Clock,
                SessionCfg,
                Sessions,
                Token,
            },
        },
    };

    use oxedyne_fe2o3_core::prelude::*;
    use oxedyne_fe2o3_iop_db::api::{
        Database,
        ScanOpts,
    };
    use oxedyne_fe2o3_jdat::prelude::*;

    use std::{
        path::{
            Path,
            PathBuf,
        },
        sync::{
            atomic::{
                AtomicU64,
                Ordering,
            },
            Arc,
            RwLock,
        },
        time::Duration,
    };

    const T0: u64 = 1_800_000_000;

    struct Store {
        db:     (Arc<RwLock<VhostDb>>, Uid),
        root:   PathBuf,
    }

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn store(label: &str) -> Outcome<Store> {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(fmt!("member-session-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        res!(std::fs::create_dir_all(&root), IO, File);
        let mut db = res!(new_db(&root, &[3u8; 32], false));
        res!(db.start("member_session_test"));
        res!(ok!(db.updated_api()).activate_gc(true));
        std::thread::sleep(Duration::from_millis(200));
        res!(db.api().ping_bots(app_const::GET_DATA_WAIT));
        Ok(Store { db: (Arc::new(RwLock::new(db)), Uid::default()), root })
    }

    fn cfg() -> SessionCfg {
        SessionCfg {
            prefix:         fmt!("t/"),
            idle_secs:      100,
            absolute_secs:  1_000,
            per_principal:  3,
            touch_secs:     10,
        }
    }

    fn clock() -> (Clock, Arc<AtomicU64>) {
        let t = Arc::new(AtomicU64::new(T0));
        (Clock::Manual(t.clone()), t)
    }

    fn keys(s: &Store) -> Outcome<Vec<String>> {
        let dbr = lock_read!(s.db.0);
        let rows = res!(dbr.scan(&ScanOpts::with_str_prefix("t/s/"), None));
        let mut out = Vec::new();
        for (k, _, _) in rows {
            if let Dat::Str(name) = k {
                // A scan lists a deleted key whose tombstone it meets; only live ones count.
                if res!(dbr.get(&Dat::Str(name.clone()), None)).is_some() {
                    out.push(name);
                }
            }
        }
        Ok(out)
    }

    fn files_contain(dir: &Path, needle: &[u8]) -> Outcome<bool> {
        for entry in res!(std::fs::read_dir(dir), IO, File) {
            let p = res!(entry, IO, File).path();
            if p.is_dir() {
                if res!(files_contain(&p, needle)) {
                    return Ok(true);
                }
            } else {
                let bytes = res!(std::fs::read(&p), IO, File);
                if bytes.windows(needle.len()).any(|w| w == needle) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    #[test]
    fn start_check_end() -> Outcome<()> {
        let s = res!(store("basic"));
        let (clk, _) = clock();
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk));
        let tok = res!(ss.start(&s.db, b"alice", dat!("phone")));
        let got = res!(ss.check(&s.db, &tok));
        let sess = res!(got.ok_or_else(|| err!("A fresh session must check."; Test)));
        assert_eq!(sess.principal, b"alice".to_vec());
        assert_eq!(sess.data, dat!("phone"));
        assert_eq!(sess.created, T0);

        // The cookie form round-trips, and a token never formats as its bytes.
        let cookie = tok.to_cookie();
        assert_eq!(cookie.len(), 64);
        let back = res!(Token::from_cookie(&cookie));
        assert!(res!(ss.check(&s.db, &back)).is_some());
        assert!(Token::from_cookie("short").is_err());
        let shown = fmt!("{:?} {:?} {:?}", tok, sess, ss.list(b"alice"));
        assert!(!shown.to_lowercase().contains(&cookie.to_lowercase()), "{}", shown);
        assert_ne!(sess.id.to_lowercase(), cookie.to_lowercase());

        assert!(res!(ss.end(&s.db, &tok)));
        assert!(res!(ss.check(&s.db, &tok)).is_none());
        assert!(!res!(ss.end(&s.db, &tok)));
        assert!(res!(keys(&s)).is_empty());
        // A token nobody issued checks as nothing.
        let stranger = res!(Token::from_cookie(&"ab".repeat(32)));
        assert!(res!(ss.check(&s.db, &stranger)).is_none());
        Ok(())
    }

    #[test]
    fn idle_and_absolute_expiry() -> Outcome<()> {
        let s = res!(store("expiry"));
        let (clk, t) = clock();
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk));

        // Idle: unused for the whole idle window, it is gone.
        let idle = res!(ss.start(&s.db, b"bob", Dat::Empty));
        t.store(T0 + 99, Ordering::SeqCst);
        assert!(res!(ss.check(&s.db, &idle)).is_some(), "one second inside the idle window");
        t.store(T0 + 99 + 100, Ordering::SeqCst);
        assert!(res!(ss.check(&s.db, &idle)).is_none(), "a full idle window unused");

        // Absolute: kept busy, it still ends at the ceiling.
        let start = T0 + 500;
        t.store(start, Ordering::SeqCst);
        let busy = res!(ss.start(&s.db, b"bob", Dat::Empty));
        let mut now = start;
        while now + 50 < start + 1_000 {
            now += 50;
            t.store(now, Ordering::SeqCst);
            assert!(res!(ss.check(&s.db, &busy)).is_some(), "busy at {}", now - start);
        }
        t.store(start + 1_000, Ordering::SeqCst);
        assert!(res!(ss.check(&s.db, &busy)).is_none(), "past the absolute ceiling");

        // The sweep ends what nobody presents again.
        t.store(T0 + 2_000, Ordering::SeqCst);
        let _ = res!(ss.start(&s.db, b"carol", Dat::Empty));
        let _ = res!(ss.start(&s.db, b"dave", Dat::Empty));
        t.store(T0 + 2_050, Ordering::SeqCst);
        let kept = res!(ss.start(&s.db, b"erin", Dat::Empty));
        t.store(T0 + 2_100, Ordering::SeqCst);
        assert_eq!(res!(ss.sweep(&s.db)), 2);
        assert_eq!(res!(ss.len()), 1);
        assert!(res!(ss.check(&s.db, &kept)).is_some());
        assert_eq!(res!(keys(&s)).len(), 1);
        Ok(())
    }

    #[test]
    fn the_cap_drops_the_oldest() -> Outcome<()> {
        let s = res!(store("cap"));
        let (clk, t) = clock();
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk));
        let mut toks = Vec::new();
        for i in 0..4u64 {
            t.store(T0 + i, Ordering::SeqCst);
            toks.push(res!(ss.start(&s.db, b"fay", dat!(i))));
        }
        assert!(res!(ss.check(&s.db, &toks[0])).is_none(), "the oldest went past the cap");
        for tok in &toks[1..] {
            assert!(res!(ss.check(&s.db, tok)).is_some());
        }
        let listed = res!(ss.list(b"fay"));
        assert_eq!(listed.iter().map(|x| x.data.clone()).collect::<Vec<_>>(),
            vec![dat!(1u64), dat!(2u64), dat!(3u64)]);
        assert_eq!(res!(keys(&s)).len(), 3);
        Ok(())
    }

    #[test]
    fn end_all_ends_one_principal_only() -> Outcome<()> {
        let s = res!(store("end_all"));
        let (clk, t) = clock();
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk));
        let a1 = res!(ss.start(&s.db, b"gus", Dat::Empty));
        let a2 = res!(ss.start(&s.db, b"gus", Dat::Empty));
        let b1 = res!(ss.start(&s.db, b"hal", Dat::Empty));
        assert_eq!(res!(ss.end_all(&s.db, b"gus")), 2);
        assert!(res!(ss.check(&s.db, &a1)).is_none());
        assert!(res!(ss.check(&s.db, &a2)).is_none());
        assert!(res!(ss.check(&s.db, &b1)).is_some());
        assert_eq!(res!(ss.end_all(&s.db, b"gus")), 0);
        // One device revoked from another, by the id a listing shows.
        t.store(T0 + 1, Ordering::SeqCst); // so the listing puts b1 first
        let c1 = res!(ss.start(&s.db, b"hal", Dat::Empty));
        let id = res!(ss.list(b"hal"))[0].id.clone();
        assert!(res!(ss.end_id(&s.db, &id)));
        assert!(res!(ss.check(&s.db, &b1)).is_none());
        assert!(res!(ss.check(&s.db, &c1)).is_some());
        // Revocation holds across a restart.
        let (clk, t2) = clock();
        t2.store(T0 + 1, Ordering::SeqCst);
        let again = res!(Sessions::load_with_clock(&s.db, cfg(), clk));
        assert!(res!(again.check(&s.db, &a1)).is_none());
        assert!(res!(again.check(&s.db, &c1)).is_some());
        Ok(())
    }

    #[test]
    fn a_restart_keeps_live_sessions() -> Outcome<()> {
        let s = res!(store("restart"));
        let (clk, t) = clock();
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk));
        let old = res!(ss.start(&s.db, b"ivy", dat!("old")));
        t.store(T0 + 60, Ordering::SeqCst);
        let live = res!(ss.start(&s.db, b"ivy", dat!("live")));
        // Used at +95, past `touch_secs`, so `last_seen` is written through.
        t.store(T0 + 95, Ordering::SeqCst);
        assert!(res!(ss.check(&s.db, &old)).is_some());
        drop(ss);

        // At +150 the first was last seen at +95 and the second at +60, so both live.
        let (clk, t2) = clock();
        t2.store(T0 + 150, Ordering::SeqCst);
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk.clone()));
        assert_eq!(res!(ss.len()), 2);
        let got = res!(res!(ss.check(&s.db, &live)).ok_or_else(|| err!("Lost at restart."; Test)));
        assert_eq!(got.data, dat!("live"));
        assert_eq!(got.created, T0 + 60);
        let got = res!(res!(ss.check(&s.db, &old)).ok_or_else(|| err!("Lost at restart."; Test)));
        assert_eq!(got.data, dat!("old"));
        drop(ss);

        // Long after, a load drops what expired while nothing ran.
        let (clk, t3) = clock();
        t3.store(T0 + 5_000, Ordering::SeqCst);
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk));
        assert_eq!(res!(ss.len()), 0);
        assert!(res!(keys(&s)).is_empty());
        Ok(())
    }

    #[test]
    fn the_store_holds_the_hash_and_never_the_token() -> Outcome<()> {
        let s = res!(store("hash"));
        let (clk, _) = clock();
        let ss = res!(Sessions::load_with_clock(&s.db, cfg(), clk));
        let tok = res!(ss.start(&s.db, b"jo", dat!("laptop")));
        let id = res!(ss.list(b"jo"))[0].id.clone();
        let cookie = tok.to_cookie();
        assert_eq!(res!(keys(&s)), vec![fmt!("t/s/{}", id)]);
        // The hash of the token, not the token.
        assert_ne!(id, cookie);
        let raw = res!(oxedyne_fe2o3_text::base2x::HEX.from_str(&cookie));
        assert_eq!(raw.len(), 32);
        assert!(!res!(files_contain(&s.root, &raw)), "the raw token is in the store's files");
        assert!(!res!(files_contain(&s.root, cookie.as_bytes())), "the token's text is in the store's files");
        // And what the record holds names no token.
        let dbr = lock_read!(s.db.0);
        let val = res!(res!(dbr.get(&Dat::Str(fmt!("t/s/{}", id)), None))
            .ok_or_else(|| err!("The record is missing."; Test)));
        let shown = fmt!("{:?}", val.0);
        assert!(!shown.to_lowercase().contains(&cookie.to_lowercase()), "{}", shown);
        Ok(())
    }
}
