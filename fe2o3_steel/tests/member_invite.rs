//! `srv::member::invite` against a real Ozone store, under the target directory rather than
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
            member::{
                Clock,
                Token,
                invite::{
                    InviteCfg,
                    Invites,
                },
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
            .join(fmt!("member-invite-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        res!(std::fs::create_dir_all(&root), IO, File);
        let mut db = res!(new_db(&root, &[3u8; 32], false));
        res!(db.start("member_invite_test"));
        res!(ok!(db.updated_api()).activate_gc(true));
        std::thread::sleep(Duration::from_millis(200));
        res!(db.api().ping_bots(app_const::GET_DATA_WAIT));
        Ok(Store { db: (Arc::new(RwLock::new(db)), Uid::default()), root })
    }

    fn cfg() -> InviteCfg {
        InviteCfg {
            prefix:     fmt!("t/"),
            ttl_secs:   100,
            max_open:   3,
        }
    }

    fn clock() -> (Clock, Arc<AtomicU64>) {
        let t = Arc::new(AtomicU64::new(T0));
        (Clock::Manual(t.clone()), t)
    }

    fn keys(s: &Store) -> Outcome<Vec<String>> {
        let dbr = lock_read!(s.db.0);
        let rows = res!(dbr.scan(&ScanOpts::with_str_prefix("t/i/"), None));
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

    fn person(name: &str) -> Dat {
        let mut m = DaticleMap::new();
        m.insert(dat!("person"),    dat!(name.to_string()));
        m.insert(dat!("kind"),      dat!("add-device"));
        Dat::Map(m)
    }

    fn is_for(d: &Dat, name: &str) -> bool {
        match d {
            Dat::Map(m) => m.get(&dat!("person")) == Some(&dat!(name.to_string())),
            _           => false,
        }
    }

    #[test]
    fn issue_and_take_once() -> Outcome<()> {
        let s = res!(store("once"));
        let (clk, _) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, cfg(), clk));
        let tok = res!(iv.issue(&s.db, person("fred")));
        assert_eq!(res!(iv.open_count()), 1);
        // A peek shows it and leaves it open.
        let seen = res!(res!(iv.peek(&tok)).ok_or_else(|| err!("A fresh invite must peek."; Test)));
        assert_eq!(seen.payload, person("fred"));
        assert_eq!(seen.exp, T0 + 100);
        // The link form round-trips, and a token never formats as its bytes.
        let link = tok.to_hex();
        let back = res!(Token::from_hex(&link));
        let shown = fmt!("{:?} {:?} {:?}", tok, seen, iv.list());
        assert!(!shown.to_lowercase().contains(&link.to_lowercase()), "{}", shown);

        assert_eq!(res!(iv.take(&s.db, &back)), Some(person("fred")));
        assert_eq!(res!(iv.take(&s.db, &tok)), None, "a second take");
        assert!(res!(iv.peek(&tok)).is_none());
        assert_eq!(res!(iv.open_count()), 0);
        assert!(res!(keys(&s)).is_empty());
        // Nor does it come back after a restart.
        let (clk, _) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, cfg(), clk));
        assert_eq!(res!(iv.take(&s.db, &tok)), None);
        let stranger = res!(Token::from_hex(&"cd".repeat(32)));
        assert_eq!(res!(iv.take(&s.db, &stranger)), None);
        Ok(())
    }

    #[test]
    fn an_invite_expires() -> Outcome<()> {
        let s = res!(store("expiry"));
        let (clk, t) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, cfg(), clk));
        let a = res!(iv.issue(&s.db, person("a")));
        t.store(T0 + 50, Ordering::SeqCst);
        let _b = res!(iv.issue(&s.db, person("b")));
        t.store(T0 + 100, Ordering::SeqCst);
        assert!(res!(iv.peek(&a)).is_none(), "expired at issued + ttl");
        assert_eq!(res!(iv.open_count()), 1);
        assert_eq!(res!(iv.take(&s.db, &a)), None);
        assert_eq!(res!(keys(&s)).len(), 1, "the expired record went with the take");
        t.store(T0 + 150, Ordering::SeqCst);
        assert_eq!(res!(iv.sweep(&s.db)), 1);
        assert!(res!(keys(&s)).is_empty());
        Ok(())
    }

    #[test]
    fn max_open_refuses_until_one_goes() -> Outcome<()> {
        let s = res!(store("cap"));
        let (clk, t) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, cfg(), clk));
        let first = res!(iv.issue(&s.db, person("a")));
        let _ = res!(iv.issue(&s.db, person("b")));
        let _ = res!(iv.issue(&s.db, person("c")));
        assert!(iv.issue(&s.db, person("d")).is_err(), "a fourth past max_open 3");
        assert_eq!(res!(keys(&s)).len(), 3);
        assert!(res!(iv.take(&s.db, &first)).is_some());
        let _ = res!(iv.issue(&s.db, person("d")));
        // Expired invites hold no place: they are swept before the count.
        t.store(T0 + 100, Ordering::SeqCst);
        for n in ["e", "f", "g"] {
            let _ = res!(iv.issue(&s.db, person(n)));
        }
        assert_eq!(res!(keys(&s)).len(), 3);
        assert!(InviteCfg { max_open: 0, ..cfg() }.check().is_err());
        assert!(InviteCfg { ttl_secs: 0, ..cfg() }.check().is_err());
        Ok(())
    }

    #[test]
    fn purge_by_payload_and_revoke_by_id() -> Outcome<()> {
        let s = res!(store("purge"));
        let (clk, _) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, cfg(), clk));
        let f1 = res!(iv.issue(&s.db, person("fred")));
        let f2 = res!(iv.issue(&s.db, person("fred")));
        let jo = res!(iv.issue(&s.db, person("jo")));
        assert_eq!(res!(iv.purge(&s.db, |d| is_for(d, "fred"))), 2);
        assert_eq!(res!(iv.take(&s.db, &f1)), None);
        assert_eq!(res!(iv.take(&s.db, &f2)), None);
        assert_eq!(res!(keys(&s)).len(), 1);
        // The purge holds across a restart.
        let (clk, _) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, cfg(), clk));
        assert_eq!(res!(iv.open_count()), 1);
        let id = res!(iv.list())[0].id.clone();
        assert!(res!(iv.revoke(&s.db, &id)));
        assert!(!res!(iv.revoke(&s.db, &id)));
        assert!(!res!(iv.revoke(&s.db, "not-an-id")));
        assert_eq!(res!(iv.take(&s.db, &jo)), None);
        assert!(res!(keys(&s)).is_empty());
        Ok(())
    }

    #[test]
    fn two_threads_taking_one_token_one_wins() -> Outcome<()> {
        let s = res!(store("race"));
        let (clk, _) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, InviteCfg { max_open: 50, ..cfg() }, clk));
        for round in 0..20 {
            let tok = res!(iv.issue(&s.db, person("fred")));
            let link = tok.to_hex();
            let wins: Vec<bool> = std::thread::scope(|sc| {
                let hs: Vec<_> = (0..2).map(|_| {
                    let link = link.clone();
                    let (iv, db) = (&iv, &s.db);
                    sc.spawn(move || -> bool {
                        match Token::from_hex(&link) {
                            Ok(t)   => matches!(iv.take(db, &t), Ok(Some(_))),
                            Err(_)  => false,
                        }
                    })
                }).collect();
                hs.into_iter().map(|h| h.join().unwrap_or(false)).collect()
            });
            assert_eq!(wins.iter().filter(|w| **w).count(), 1, "round {}: {:?}", round, wins);
        }
        assert!(res!(keys(&s)).is_empty());
        Ok(())
    }

    #[test]
    fn the_store_holds_the_hash_and_never_the_token() -> Outcome<()> {
        let s = res!(store("hash"));
        let (clk, _) = clock();
        let iv = res!(Invites::load_with_clock(&s.db, cfg(), clk));
        let tok = res!(iv.issue(&s.db, person("fred")));
        let id = res!(iv.list())[0].id.clone();
        let link = tok.to_hex();
        assert_eq!(res!(keys(&s)), vec![fmt!("t/i/{}", id)]);
        assert_ne!(id.to_lowercase(), link.to_lowercase());
        let raw = res!(oxedyne_fe2o3_text::base2x::HEX.from_str(&link));
        assert!(!res!(files_contain(&s.root, &raw)), "the raw token is in the store's files");
        assert!(!res!(files_contain(&s.root, link.as_bytes())), "the token's text is in the store's files");
        Ok(())
    }
}
