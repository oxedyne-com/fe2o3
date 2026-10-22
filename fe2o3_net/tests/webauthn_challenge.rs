//! `webauthn::challenge`: stateless challenges, each accepted once.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

mod webauthn {

    use oxedyne_fe2o3_net::{
        acme::jose::{
            base64url_decode,
            base64url_encode,
        },
        webauthn::challenge::{
            Challenges,
            Purpose,
            TTL_SECS,
        },
    };

    use oxedyne_fe2o3_core::prelude::*;

    const T0: u64 = 1_800_000_000;

    #[test]
    fn a_challenge_checks_once() -> Outcome<()> {
        let ch = Challenges::new();
        let (c, cookie) = ch.mint(Purpose::Get, T0);
        assert_eq!(res!(ch.used_len()), 0, "a mint records nothing");
        res!(ch.check(&cookie, &c, Purpose::Get, T0 + 5));
        assert_eq!(res!(ch.used_len()), 1);
        assert!(ch.check(&cookie, &c, Purpose::Get, T0 + 6).is_err(), "a replay");
        assert!(ch.check(&cookie, &c, Purpose::Get, T0 + TTL_SECS - 1).is_err(), "a late replay");
        // A second challenge, minted alike, is its own.
        let (c2, cookie2) = ch.mint(Purpose::Create, T0);
        assert_ne!(c, c2);
        res!(ch.check(&cookie2, &c2, Purpose::Create, T0 + 1));
        Ok(())
    }

    #[test]
    fn bad_challenges_are_refused() -> Outcome<()> {
        let ch = Challenges::new();
        let (c, cookie) = ch.mint(Purpose::Create, T0);

        // Expired.
        assert!(ch.check(&cookie, &c, Purpose::Create, T0 + TTL_SECS).is_err());
        // For the other ceremony.
        assert!(ch.check(&cookie, &c, Purpose::Get, T0 + 1).is_err());
        // The browser signed another challenge.
        let mut other = c;
        other[0] ^= 1;
        assert!(ch.check(&cookie, &other, Purpose::Create, T0 + 1).is_err());
        // Tampered: each field flipped in turn, the expiry pushed out, the purpose switched.
        let b = res!(base64url_decode(&cookie));
        for i in [0usize, 1, 8, 9, 40, 41, 72] {
            let mut t = b.clone();
            t[i] ^= 1;
            assert!(ch.check(&base64url_encode(&t), &c, Purpose::Create, T0 + 1).is_err(),
                "byte {} flipped", i);
        }
        let mut late = b.clone();
        late[1..9].copy_from_slice(&(T0 + 10_000).to_be_bytes());
        assert!(ch.check(&base64url_encode(&late), &c, Purpose::Create, T0 + 1).is_err());
        let mut get = b.clone();
        get[0] = b'g';
        assert!(ch.check(&base64url_encode(&get), &c, Purpose::Get, T0 + 1).is_err());
        // Junk and the wrong length.
        assert!(ch.check("not a cookie!", &c, Purpose::Create, T0 + 1).is_err());
        assert!(ch.check(&base64url_encode(&b[..40]), &c, Purpose::Create, T0 + 1).is_err());
        // Minted by another process, or before a restart.
        let foreign = Challenges::new();
        let (fc, fcookie) = foreign.mint(Purpose::Create, T0);
        assert!(ch.check(&fcookie, &fc, Purpose::Create, T0 + 1).is_err());

        // None of those refusals spent the real one.
        assert_eq!(res!(ch.used_len()), 0);
        res!(ch.check(&cookie, &c, Purpose::Create, T0 + 1));
        Ok(())
    }

    #[test]
    fn ten_thousand_mints_hold_no_state() -> Outcome<()> {
        let ch = Challenges::new().with_max_used(100);
        let mut minted = Vec::with_capacity(10_000);
        for i in 0..10_000u64 {
            minted.push(ch.mint(Purpose::Get, T0 + i / 1_000));
        }
        assert_eq!(res!(ch.used_len()), 0, "a mint keeps nothing");
        // The earliest is still checkable: no flood of mints evicts it.
        res!(ch.check(&minted[0].1, &minted[0].0, Purpose::Get, T0 + 10));
        // Checks fill the used-set to its bound and no further.
        let mut refused = 0usize;
        for (c, cookie) in minted.iter().skip(1).take(200) {
            if ch.check(cookie, c, Purpose::Get, T0 + 10).is_err() {
                refused += 1;
            }
        }
        assert_eq!(res!(ch.used_len()), 100);
        assert_eq!(refused, 101);
        // And it empties as the window passes.
        let (c, cookie) = ch.mint(Purpose::Get, T0 + 10 + TTL_SECS + 1);
        res!(ch.check(&cookie, &c, Purpose::Get, T0 + 10 + TTL_SECS + 2));
        assert_eq!(res!(ch.used_len()), 1);
        Ok(())
    }
}
