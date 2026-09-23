//! `presentation` from outside: a fixture signed by a real WebCrypto Ed25519
//! key in headless Chromium (`tools/presentation_fixture.cjs`), the refusal
//! each attack earns, and the module's silence about any network by name.
//!
//! The verifier's guards are proven load-bearing inside the module, where a
//! test can switch one off; see `src/presentation/verify.rs`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::{
    linkring::{
        self,
        Ring,
        SecretKey,
    },
    sign::SignatureScheme,
};
use oxedyne_fe2o3_hash::sha256;
use oxedyne_fe2o3_iop_crypto::{
    keys::KeyManager,
    sign::Signer,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::presentation::{
    shape::{
        self,
        Accept,
        Head,
        Invoice,
        Presentation,
        Proof,
        Request,
        Settlement,
        Status,
        Subject,
        HEAD_LEAD,
        T_G,
    },
    verify::{
        Lookup,
        Refusal,
        Verdict,
        Verified,
        Verifier,
    },
};
use oxedyne_fe2o3_text::base64;

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::Arc,
};

const NOW:  u64     = 1_800_000_000;
const APP:  &str    = "https://app.example";
const OTHER:&str    = "https://other.example";

// ── Fixtures ────────────────────────────────────────────────────────────────

struct Ed {
    scheme: SignatureScheme,
    public: [u8; 32],
}

impl Ed {
    fn new() -> Outcome<Self> {
        let scheme = SignatureScheme::new_ed25519();
        let mut public = [0u8; 32];
        match res!(scheme.get_public_key()) {
            Some(pk) => public.copy_from_slice(pk),
            None => return Err(err!("A new Ed25519 key has no public half."; Test, Missing)),
        }
        Ok(Self { scheme, public })
    }

    fn sign(&self, msg: &[u8]) -> Outcome<[u8; 64]> {
        let sig = res!(self.scheme.sign(msg));
        let mut out = [0u8; 64];
        out.copy_from_slice(&sig);
        Ok(out)
    }
}

#[derive(Default)]
struct Book {
    heads:      RefCell<HashMap<[u8; 32], Head>>,
    rings:      RefCell<HashMap<[u8; 32], Arc<Ring>>>,  // by head id
    statuses:   RefCell<HashMap<String, Status>>,
}

impl Lookup for Book {
    fn head(&self, id: &[u8; 32]) -> Outcome<Option<Head>> {
        Ok(self.heads.borrow().get(id).cloned())
    }
    fn ring(&self, head: &Head) -> Outcome<Option<Arc<Ring>>> {
        Ok(self.rings.borrow().get(&head.id).cloned())
    }
    fn status(&self, id: &str) -> Outcome<Option<Status>> {
        Ok(self.statuses.borrow().get(id).cloned())
    }
}

fn ring_of(keys: &[SecretKey]) -> Outcome<Arc<Ring>> {
    let mut list: Vec<[u8; 32]> = keys.iter().map(|k| k.public_key()).collect();
    list.sort(); // The ring list is sorted ascending by its keys' bytes.
    Ok(Arc::new(res!(Ring::from_keys(&list))))
}

/// A head at `ts` over `ring`, signed by `signer` and filed in `book`.
fn mint(book: &Book, signer: &Ed, ring: &Arc<Ring>, ts: u64) -> Outcome<Head> {
    let mut head = Head {
        id:             [0u8; 32],
        epoch:          1,
        prev:           None,
        ts,
        salt:           [9u8; 16],
        members:        ring.len() as u64,
        ring_n:         ring.len() as u64,
        ring_digest:    *ring.digest(),
        signer:         signer.public,
        sig:            [0u8; 64],
    };
    head.sig = res!(signer.sign(&res!(head.signed_bytes())));
    head.id = res!(head.compute_id());
    book.rings.borrow_mut().insert(head.id, ring.clone());
    book.heads.borrow_mut().insert(head.id, head.clone());
    Ok(head)
}

struct World {
    peer:   Ed,
    member: Ed,         // a name's key
    keys:   Vec<SecretKey>,
    ring:   Arc<Ring>,
    head:   Head,
}

fn world() -> Outcome<(World, Book)> {
    let book = Book::default();
    let peer = res!(Ed::new());
    let member = res!(Ed::new());
    let mut keys = Vec::new();
    for _ in 0..7 {
        keys.push(res!(SecretKey::random()));
    }
    let ring = res!(ring_of(&keys));
    let head = res!(mint(&book, &peer, &ring, NOW - 20));
    book.statuses.borrow_mut().insert(
        shape::key_id(&member.public), Status { key: member.public, live: true });
    Ok((World { peer, member, keys, ring, head }, book))
}

fn verifier(w: &World, book: Book) -> Outcome<Verifier<Book>> {
    Verifier::new(APP, vec![w.peer.public], book, 2)
}

/// A named presentation answering `req`, signed by `member`.
fn named(req: &Request, member: &Ed, head: &Head) -> Outcome<Presentation> {
    let mut p = Presentation {
        rp_id:      req.rp_id.clone(),
        nonce:      req.nonce,
        subject:    Subject::Named { id: shape::key_id(&member.public), key: member.public },
        predicates: req.predicates.clone(),
        head:       head.id,
        ts:         NOW,
        sig:        [0u8; 64],
    };
    p.sig = res!(member.sign(&res!(p.signed_bytes())));
    Ok(p)
}

/// A pairwise presentation answering `req`: pseudonym key `sub`, ring key
/// `key`, proof over `ring` under `rp_id`'s scope.
fn pairwise(
    req:    &Request,
    sub:    &Ed,
    key:    &SecretKey,
    ring:   &Ring,
    head:   &Head,
    rp_id:  &str,
)
    -> Outcome<Presentation>
{
    let scope = shape::scope(rp_id);
    let mut p = Presentation {
        rp_id:      req.rp_id.clone(),
        nonce:      req.nonce,
        subject:    Subject::Pairwise {
            key:    sub.public,
            tag:    res!(linkring::tag(key, &scope)),
            proof:  Proof { alg: linkring::ALG.to_string(), body: Vec::new() },
        },
        predicates: req.predicates.clone(),
        head:       head.id,
        ts:         NOW,
        sig:        [0u8; 64],
    };
    res!(prove(&mut p, sub, key, ring, &scope));
    Ok(p)
}

/// Makes the ring proof and the signature afresh over `p` as it now stands.
fn prove(p: &mut Presentation, sub: &Ed, key: &SecretKey, ring: &Ring, scope: &[u8]) -> Outcome<()> {
    let msg = res!(p.signed_bytes());
    let (tag, body) = res!(linkring::sign(ring, key, scope, &msg));
    if let Subject::Pairwise { tag: t, proof, .. } = &mut p.subject {
        req!(*t, tag, "the proof's tag is the tag the body carries");
        proof.body = body;
    }
    p.sig = res!(sub.sign(&msg));
    Ok(())
}

/// Signs `p` again with `key`, after a change to what the signature covers.
fn resign(p: &mut Presentation, key: &Ed) -> Outcome<()> {
    p.sig = res!(key.sign(&res!(p.signed_bytes())));
    Ok(())
}

fn body(p: &Presentation) -> Outcome<Vec<u8>> {
    Ok(res!(p.to_json()).into_bytes())
}

fn refusal(v: Outcome<Verdict>) -> Outcome<Option<Refusal>> {
    Ok(res!(v).refusal())
}

// ── Acceptance ──────────────────────────────────────────────────────────────

#[test]
fn a_named_presentation_verifies() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let req = res!(v.issue(b"session", Accept::PairwiseOrNamed, &["adult"], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    match res!(v.verify(b"session", &res!(body(&p)), NOW)) {
        Verdict::Accepted(Verified::Named { id, key, predicates }) => {
            req!(id, shape::key_id(&w.member.public));
            req!(key, w.member.public);
            req!(predicates, vec!["adult".to_string()]);
        },
        other => return Err(err!("A sound named presentation earned {:?}.", other; Test)),
    }
    Ok(())
}

/// Chromium made both keys, signed the head and the presentation over its own
/// canonical JSON, and serialised the presentation with its members in the
/// order it built them. The verifier accepts it only by rebuilding the
/// browser's bytes and verifying the browser's Ed25519 signatures.
#[test]
fn a_named_presentation_signed_in_webcrypto_verifies() -> Outcome<()> {
    let fx = res!(Dat::decode_string(include_str!("data/presentation_webcrypto.json")));
    let now = res!(fx.map_get_u64(&dat!("now")));
    let req = res!(Request::from_dat(res!(fx.map_get_must(&dat!("request")))));
    let head = res!(Head::from_dat(res!(fx.map_get_must(&dat!("head")))));
    let status = res!(Status::from_dat(res!(fx.map_get_must(&dat!("status")))));
    let text = res!(fx.map_get_string(&dat!("presentation")));
    let p = res!(Presentation::parse(&text));
    req!(res!(fx.map_get_string(&dat!("rp_id"))), APP.to_string());
    req!(head.ring_n, 0, "a named presentation needs no ring");

    let book = Book::default();
    book.heads.borrow_mut().insert(head.id, head.clone());
    let id = match &p.subject {
        Subject::Named { id, .. } => id.clone(),
        other => return Err(err!("The fixture is named, not {:?}.", other; Test)),
    };
    book.statuses.borrow_mut().insert(id.clone(), status);
    let v = res!(Verifier::new(APP, vec![head.signer], book, 1));
    match res!(v.check_issued(&req, &p, now)) {
        Verdict::Accepted(Verified::Named { id: got, .. }) => req!(got, id),
        other => return Err(err!("The WebCrypto presentation earned {:?}.", other; Test)),
    }

    // One byte of the browser's signature changed, and it no longer verifies.
    let mut bad = p.clone();
    bad.sig[10] ^= 0x01;
    let got = res!(refusal(v.check_issued(&req, &bad, now)));
    req!(got, Some(Refusal::BadSignature));
    Ok(())
}

/// A pairwise presentation over the whole ring at the head verifies, and its
/// tag is the member's `linkring` tag under this origin's scope, whatever
/// pseudonym key presents it.
#[test]
fn a_pairwise_presentation_over_the_whole_ring_verifies() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let want = res!(linkring::tag(&w.keys[3], &shape::scope(APP)));
    for _ in 0..2 {
        let sub = res!(Ed::new());
        let req = res!(v.issue(b"session", Accept::Pairwise, &[], None, None, NOW));
        let p = res!(pairwise(&req, &sub, &w.keys[3], &w.ring, &w.head, APP));
        match res!(v.verify(b"session", &res!(body(&p)), NOW)) {
            Verdict::Accepted(Verified::Pairwise { key, tag, .. }) => {
                req!(key, sub.public);
                req!(tag, want, "one human, one tag at one relying party");
            },
            other => return Err(err!("A sound pairwise presentation earned {:?}.", other; Test)),
        }
    }
    Ok(())
}

// ── Audience, nonce and freshness ───────────────────────────────────────────

/// A presentation made for one relying party is refused at another, whether
/// it arrives as it was made or with its origin rewritten.
#[test]
fn a_cross_origin_replay_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let other_book = Book::default();
    other_book.heads.borrow_mut().insert(w.head.id, w.head.clone());
    other_book.rings.borrow_mut().insert(w.head.id, w.ring.clone());
    other_book.statuses.borrow_mut().insert(
        shape::key_id(&w.member.public), Status { key: w.member.public, live: true });
    let a = res!(verifier(&w, book));
    let b = res!(Verifier::new(OTHER, vec![w.peer.public], other_book, 1));

    let req = res!(a.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(b.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::WrongAudience));
    let mut moved = p.clone();
    moved.rp_id = OTHER.to_string();
    let got = res!(refusal(b.verify(b"s", &res!(body(&moved)), NOW)));
    req!(got, Some(Refusal::UnknownNonce));

    // The same for a pairwise presentation, which carries A's tag.
    let sub = res!(Ed::new());
    let req = res!(a.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
    let p = res!(pairwise(&req, &sub, &w.keys[0], &w.ring, &w.head, APP));
    let got = res!(refusal(b.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::WrongAudience));

    // And A still accepts what was made for it.
    let got = res!(refusal(a.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, None::<Refusal>);
    Ok(())
}

#[test]
fn an_unknown_nonce_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    // Never issued.
    let fake = Request {
        rp_id:      APP.to_string(),
        nonce:      [5u8; 32],
        accept:     Accept::PairwiseOrNamed,
        predicates: vec![],
        invoice:    None,
        return_to:  None,
        exp:        NOW + T_G,
    };
    let p = res!(named(&fake, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownNonce));
    // Issued to another session.
    let req = res!(v.issue(b"alice", Accept::PairwiseOrNamed, &[], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"mallory", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownNonce));
    // Past its request's expiry, though the presentation itself is fresh.
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW - T_G - 1));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownNonce));
    Ok(())
}

/// A nonce is spent by the first presentation to reach it, even one that
/// then fails, so nothing presented a second time is accepted.
#[test]
fn a_nonce_spent_twice_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, None::<Refusal>);
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::Replayed));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW + 100)));
    req!(got, Some(Refusal::Replayed));

    // Spent by a first attempt that failed on its signature.
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    let mut spoilt = p.clone();
    spoilt.sig[0] ^= 0x01;
    let got = res!(refusal(v.verify(b"s", &res!(body(&spoilt)), NOW)));
    req!(got, Some(Refusal::BadSignature));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::Replayed));
    Ok(())
}

/// A nonce stays spent for as long as its challenge stands, whatever the
/// caller's clock does in between. Here the clock steps back 100 s between the
/// issue and the first showing, and the replay comes 250 s after the issue,
/// while the challenge still stands.
#[test]
fn a_replay_is_refused_after_the_clock_steps_back() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW - 100)));
    req!(got, None::<Refusal>, "the first showing passes");
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW + 250)));
    req!(got, Some(Refusal::Replayed));
    Ok(())
}

/// A presentation shown in the wrong session is refused without spending the
/// nonce, so whoever intercepts one cannot spoil it for the session it was
/// made for.
#[test]
fn a_showing_in_another_session_spends_nothing() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let req = res!(v.issue(b"alice", Accept::PairwiseOrNamed, &[], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"mallory", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownNonce));
    let got = res!(refusal(v.verify(b"alice", &res!(body(&p)), NOW)));
    req!(got, None::<Refusal>);
    Ok(())
}

/// A caller that keeps its own store of challenges still has the lapse of the
/// request asserted, as well as its audience and nonce.
#[test]
fn check_issued_refuses_a_lapsed_request() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW - T_G - 1));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.check_issued(&req, &p, NOW)));
    req!(got, Some(Refusal::UnknownNonce), "lapsed at {}", req.exp);
    let got = res!(refusal(v.check_issued(&req, &p, req.exp)));
    req!(got, None::<Refusal>, "still standing at {}", req.exp);
    Ok(())
}

/// A presentation whose `ts` is more than T_G from now is stale, on either
/// side, and both bounds are exact. Each is made against a head no later than
/// HEAD_LEAD after its `ts`, so only the presentation's own clock is at issue.
#[test]
fn a_stale_presentation_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    for (ts, want) in [
        (NOW - T_G - 1, Some(Refusal::Stale)),
        (NOW - T_G,     None),
        (NOW + T_G,     None),
        (NOW + T_G + 1, Some(Refusal::Stale)),
    ] {
        let head = res!(mint(v.lookup(), &w.peer, &w.ring, ts.min(NOW)));
        let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let mut p = res!(named(&req, &w.member, &head));
        p.ts = ts;
        res!(resign(&mut p, &w.member));
        let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
        req!(got, want, "ts {}", ts);
    }
    Ok(())
}

/// A head older than T_G is stale, and so is one minted more than HEAD_LEAD
/// after the presentation that names it; both bounds are exact.
#[test]
fn a_stale_head_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    for (head_ts, want) in [
        (NOW - T_G - 1, Some(Refusal::StaleHead)),
        (NOW - T_G, None),
        (NOW + HEAD_LEAD + 1, Some(Refusal::StaleHead)),
        (NOW + HEAD_LEAD, None),
    ] {
        let head = res!(mint(v.lookup(), &w.peer, &w.ring, head_ts));
        let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let p = res!(named(&req, &w.member, &head));
        let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
        req!(got, want, "head at {}", head_ts);
    }
    Ok(())
}

// ── Modes and predicates ────────────────────────────────────────────────────

#[test]
fn the_request_decides_mode_and_predicates() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    // A request that accepts pairwise only.
    let req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::ModeNotAccepted));
    // A predicate the request did not ask for.
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &["adult"], None, None, NOW));
    let mut p = res!(named(&req, &w.member, &w.head));
    p.predicates = vec!["adult".to_string(), "established".to_string()];
    res!(resign(&mut p, &w.member));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::PredicateNotOffered));
    // Fewer than were asked is the member's choice.
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &["adult"], None, None, NOW));
    let mut p = res!(named(&req, &w.member, &w.head));
    p.predicates = vec![];
    res!(resign(&mut p, &w.member));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, None::<Refusal>);
    Ok(())
}

// ── Tampering ───────────────────────────────────────────────────────────────

/// Each part of a pairwise presentation changed, by a member who can re-sign
/// with the pseudonym key but cannot remake another's ring proof, is refused.
#[test]
fn each_pairwise_tamper_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let sub = res!(Ed::new());
    let fresh = |v: &Verifier<Book>| v.issue(b"s", Accept::Pairwise, &[], None, None, NOW);

    // Another member's tag, re-signed.
    let req = res!(fresh(&v));
    let mut p = res!(pairwise(&req, &sub, &w.keys[1], &w.ring, &w.head, APP));
    if let Subject::Pairwise { tag, .. } = &mut p.subject {
        *tag = res!(linkring::tag(&w.keys[2], &shape::scope(APP)));
    }
    res!(resign(&mut p, &sub));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "tag");

    // One bit of the proof body.
    let req = res!(fresh(&v));
    let mut p = res!(pairwise(&req, &sub, &w.keys[1], &w.ring, &w.head, APP));
    if let Subject::Pairwise { proof, .. } = &mut p.subject {
        proof.body[40] ^= 0x01;
    }
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "body");

    // An algorithm this verifier does not hold.
    let req = res!(fresh(&v));
    let mut p = res!(pairwise(&req, &sub, &w.keys[1], &w.ring, &w.head, APP));
    if let Subject::Pairwise { proof, .. } = &mut p.subject {
        proof.alg = "linkring/2".to_string();
    }
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "alg");

    // The message: a proof over one body, the signature over another.
    let req = res!(fresh(&v));
    let mut p = res!(pairwise(&req, &sub, &w.keys[1], &w.ring, &w.head, APP));
    p.ts = NOW - 1;
    res!(resign(&mut p, &sub));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "message");

    // The scope: a proof and tag made under another origin, presented here.
    let req = res!(fresh(&v));
    let p = res!(pairwise(&req, &sub, &w.keys[1], &w.ring, &w.head, OTHER));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "scope");

    // The ring: a proof over a ring with one key dropped, which is not the
    // whole ring at the head.
    let req = res!(fresh(&v));
    let short = res!(ring_of(&w.keys[..6]));
    let p = res!(pairwise(&req, &sub, &w.keys[1], &short, &w.head, APP));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "ring");

    // The ring served for the head is not the head's.
    let req = res!(fresh(&v));
    let p = res!(pairwise(&req, &sub, &w.keys[1], &w.ring, &w.head, APP));
    v.lookup().rings.borrow_mut().insert(w.head.id, short.clone());
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "served ring");
    v.lookup().rings.borrow_mut().insert(w.head.id, w.ring.clone());

    // No ring at all for the head.
    let req = res!(fresh(&v));
    let p = res!(pairwise(&req, &sub, &w.keys[1], &w.ring, &w.head, APP));
    v.lookup().rings.borrow_mut().remove(&w.head.id);
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadProof), "no ring");
    Ok(())
}

/// A pseudonym key of small order, whose "signature" anyone can make, is
/// refused however sound the ring proof beside it. Accepting it would let
/// anyone answer a later challenge made to that key alone.
#[test]
fn a_small_order_pseudonym_key_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
    let sub = res!(Ed::new());
    let mut p = res!(pairwise(&req, &sub, &w.keys[4], &w.ring, &w.head, APP));
    let mut identity = [0u8; 32];
    identity[0] = 0x01;
    if let Subject::Pairwise { key, .. } = &mut p.subject {
        *key = identity;
    }
    // Remake the proof over the new body, then forge the Ed25519 half.
    let scope = shape::scope(APP);
    let msg = res!(p.signed_bytes());
    let (_, proof_body) = res!(linkring::sign(&w.ring, &w.keys[4], &scope, &msg));
    if let Subject::Pairwise { proof, .. } = &mut p.subject {
        proof.body = proof_body;
    }
    let mut forged = [0u8; 64];
    forged[0] = 0x01;
    p.sig = forged;
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadSignature));
    Ok(())
}

#[test]
fn each_named_tamper_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let fresh = |v: &Verifier<Book>| v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW);

    // A name that is not its key's id, re-signed.
    let req = res!(fresh(&v));
    let mut p = res!(named(&req, &w.member, &w.head));
    if let Subject::Named { id, .. } = &mut p.subject {
        *id = "0123456789".to_string();
    }
    res!(resign(&mut p, &w.member));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadSignature), "sub");

    // Signed by a key other than the one presented.
    let req = res!(fresh(&v));
    let mut p = res!(named(&req, &w.member, &w.head));
    let other = res!(Ed::new());
    res!(resign(&mut p, &other));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::BadSignature), "key");

    // A name no longer live, one that holds another key now, and one unknown.
    let id = shape::key_id(&w.member.public);
    for status in [
        Some(Status { key: w.member.public, live: false }),
        Some(Status { key: other.public, live: true }),
        None,
    ] {
        match &status {
            Some(s) => { v.lookup().statuses.borrow_mut().insert(id.clone(), s.clone()); },
            None    => { v.lookup().statuses.borrow_mut().remove(&id); },
        }
        let req = res!(fresh(&v));
        let p = res!(named(&req, &w.member, &w.head));
        let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
        req!(got, Some(Refusal::NotLive),
            "status {:?}", status);
    }
    Ok(())
}

#[test]
fn each_head_tamper_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let fresh = |v: &Verifier<Book>| v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW);

    // A head whose content no longer hashes to its id.
    let mut altered = w.head.clone();
    altered.members += 1;
    v.lookup().heads.borrow_mut().insert(w.head.id, altered);
    let req = res!(fresh(&v));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownHead), "content");

    // A head whose signature fails.
    let mut spoilt = w.head.clone();
    spoilt.sig[5] ^= 0x01;
    v.lookup().heads.borrow_mut().insert(w.head.id, spoilt);
    let req = res!(fresh(&v));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownHead), "sig");

    // A head minted by a key outside the issuers.
    let forger = res!(Ed::new());
    let forged = res!(mint(v.lookup(), &forger, &w.ring, NOW - 20));
    let req = res!(fresh(&v));
    let p = res!(named(&req, &w.member, &forged));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownHead), "issuer");

    // A head nobody can fetch.
    v.lookup().heads.borrow_mut().remove(&w.head.id);
    let req = res!(fresh(&v));
    let p = res!(named(&req, &w.member, &w.head));
    let got = res!(refusal(v.verify(b"s", &res!(body(&p)), NOW)));
    req!(got, Some(Refusal::UnknownHead), "missing");
    Ok(())
}

// ── Shapes ──────────────────────────────────────────────────────────────────

/// Every departure from the shape is `malformed`, whatever else is right.
#[test]
fn malformed_presentations_are_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let sub = res!(Ed::new());
    let named_req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
    let named_text = res!(res!(named(&named_req, &w.member, &w.head)).to_json());
    let pair_req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
    let pair_text = res!(res!(pairwise(&pair_req, &sub, &w.keys[0], &w.ring, &w.head, APP)).to_json());
    let pub_b64 = base64::encode_url(&w.member.public);

    let edits: Vec<(&str, String)> = vec![
        ("not JSON",            "{\"v\":".to_string()),
        ("not an object",       "[1,2,3]".to_string()),
        ("another version",     named_text.replace("\"present/1\"", "\"present/2\"")),
        ("another mode",        named_text.replace("\"named\"", "\"public\"")),
        ("an unknown member",   named_text.replace("{", "{\"extra\":1,")),
        ("a missing member",    named_text.replace(&fmt!("\"pub\":\"{}\",", pub_b64), "")),
        ("padded base64url",    named_text.replace(&fmt!("\"{}\"", pub_b64), &fmt!("\"{}=\"", pub_b64))),
        ("a short key",         named_text.replace(&fmt!("\"{}\"", pub_b64), "\"AAAA\"")),
        ("a float ts",          named_text.replace(&fmt!("\"ts\":{}", NOW), &fmt!("\"ts\":{}.5", NOW))),
        ("a negative ts",       named_text.replace(&fmt!("\"ts\":{}", NOW), "\"ts\":-1")),
        ("a ts beyond 2^53",    named_text.replace(&fmt!("\"ts\":{}", NOW), "\"ts\":9007199254740992")),
        ("a string ts",         named_text.replace(&fmt!("\"ts\":{}", NOW), &fmt!("\"ts\":\"{}\"", NOW))),
        ("a repeated word",     named_text.replace("\"predicates\":[]", "\"predicates\":[\"adult\",\"adult\"]")),
        ("a word not a word",   named_text.replace("\"predicates\":[]", "\"predicates\":[\"Adult\"]")),
        ("a named sub in hex caps",
            named_text.replace(&shape::key_id(&w.member.public), &shape::key_id(&w.member.public).to_uppercase())),
        ("a named proof",       named_text.replace("{", "{\"proof\":{\"alg\":\"linkring/1\",\"body\":\"AA\"},")),
        ("a pairwise pub",      pair_text.replace("{", &fmt!("{{\"pub\":\"{}\",", pub_b64))),
        ("a proof member extra", pair_text.replace("\"alg\":", "\"x\":1,\"alg\":")),
        ("a duplicate member",  named_text.replace("{", &fmt!("{{\"ts\":{},", NOW))),
    ];
    for (what, text) in edits {
        let got = res!(refusal(v.verify(b"s", text.as_bytes(), NOW)));
        req!(got, Some(Refusal::Malformed), "{}", what);
    }
    let got = res!(refusal(v.verify(b"s", &[0xff, 0xfe, 0x00], NOW)));
    req!(got, Some(Refusal::Malformed), "not UTF-8");
    Ok(())
}

/// The signed bytes are the canonical JSON the contract describes, written out
/// here by hand rather than taken from the encoder that makes them.
#[test]
fn the_signed_bytes_are_the_canonical_json() -> Outcome<()> {
    let p = Presentation {
        rp_id:      APP.to_string(),
        nonce:      [0u8; 32],
        subject:    Subject::Named { id: "0123456789".to_string(), key: [0xffu8; 32] },
        predicates: vec!["adult".to_string()],
        head:       [0xfbu8; 32],
        ts:         1_700_000_000,
        sig:        [0u8; 64],
    };
    let want = "{\"head\":\"-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_v7-_s\",\"mode\":\"named\",\
        \"nonce\":\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\",\"predicates\":[\"adult\"],\
        \"pub\":\"__________________________________________8\",\
        \"rp_id\":\"https://app.example\",\"sub\":\"0123456789\",\"ts\":1700000000,\
        \"v\":\"present/1\"}";
    req!(String::from_utf8_lossy(&res!(p.signed_bytes())).to_string(), want.to_string());

    // A head's id is SHA-256 over exactly its signed bytes, prev a present null.
    let head = Head {
        id:             [0u8; 32],
        epoch:          3,
        prev:           None,
        ts:             5,
        salt:           [0u8; 16],
        members:        2,
        ring_n:         1,
        ring_digest:    [0u8; 32],
        signer:         [0u8; 32],
        sig:            [0u8; 64],
    };
    let want = "{\"epoch\":3,\"members\":2,\"prev\":null,\
        \"ring_digest\":\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\",\"ring_n\":1,\
        \"salt\":\"AAAAAAAAAAAAAAAAAAAAAA\",\
        \"signer\":\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\",\"ts\":5,\"v\":\"head/1\"}";
    req!(String::from_utf8_lossy(&res!(head.signed_bytes())).to_string(), want.to_string());
    req!(res!(head.compute_id()), sha256::digest(want.as_bytes()));
    Ok(())
}

#[test]
fn a_key_id_is_the_head_of_its_sha256() -> Outcome<()> {
    // SHA-256("abc") begins ba7816bf8f (FIPS 180-4).
    req!(shape::key_id(b"abc"), "ba7816bf8f".to_string());
    req!(shape::is_key_id("ba7816bf8f"), true);
    for bad in ["BA7816BF8F", "ba7816bf8", "ba7816bf8f0", "ba7816bf8g"] {
        req!(shape::is_key_id(bad), false, "{}", bad);
    }
    Ok(())
}

#[test]
fn origins_are_read_as_serialised() -> Outcome<()> {
    for good in [
        "https://app.example",
        "https://a.b-c.example:8443",
        "https://127.0.0.1",
        "http://localhost",
        "http://localhost:8080",
        "http://127.0.0.1:3000",
        "https://xn--bcher-kva.example",
    ] {
        if shape::check_origin(good).is_err() {
            return Err(err!("'{}' was refused.", good; Test));
        }
    }
    for bad in [
        "https://App.example",          // upper case
        "https://app.example/",         // trailing slash
        "https://app.example/path",     // path
        "https://app.example:443",      // default port
        "http://app.example",           // plain http off localhost
        "http://localhost:80",          // default port
        "ftp://app.example",            // scheme
        "https://user@app.example",     // user information
        "https://app.example:0",        // port zero
        "https://app.example:08443",    // leading zero
        "https://app.example:65536",    // port too large
        "https://app..example",         // empty label
        "https://-app.example",         // hyphen at a label's edge
        "https://[::1]",                // IPv6 literal
        "https://app.example?x=1",      // query
        "app.example",                  // no scheme
        "https://",                     // no host
    ] {
        if shape::check_origin(bad).is_ok() {
            return Err(err!("'{}' was accepted.", bad; Test));
        }
    }
    Ok(())
}

/// A request goes out and comes back as itself; the reserved members are
/// ignored and any other is refused.
#[test]
fn requests_round_trip_and_read_strictly() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let invoice = Invoice {
        rp_id:      APP.to_string(),
        payee:      "0123456789".to_string(),
        account:    "abcdef0123".to_string(),
        amount:     4_294_967_296,
        memo:       "One book".to_string(),
        nonce:      [3u8; 16],
        expires:    NOW + 600,
        ts:         NOW,
    };
    let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &["adult"], Some(invoice.clone()),
        Some("https://app.example/back?x=1"), NOW));
    let text = res!(req.to_json());
    req!(res!(Request::parse(&text)), req.clone());

    let reserved = text.replacen("{", "{\"org\":\"a\",\"osig\":\"b\",", 1);
    req!(res!(Request::parse(&reserved)), req.clone(), "org and osig are ignored");
    for (what, bad) in [
        ("an unknown member",       text.replacen("{", "{\"x\":1,", 1)),
        ("modes without pairwise",  text.replace("[\"pairwise\",\"named\"]", "[\"named\"]")),
        ("modes out of order",      text.replace("[\"pairwise\",\"named\"]", "[\"named\",\"pairwise\"]")),
        ("a return URL elsewhere",  text.replace("https://app.example/back", "https://evil.example/back")),
    ] {
        req!(Request::parse(&bad).is_err(), true, "{}", what);
    }
    // Issuing checks the same rules.
    let bad = v.issue(b"s", Accept::Pairwise, &["Adult"], None, None, NOW).is_err();
    req!(bad, true, "a bad word");
    let bad = v.issue(b"s", Accept::Pairwise, &[], None, Some("https://app.example#frag"), NOW).is_err();
    req!(bad,
        true, "a return URL with a fragment");
    let mut foreign = invoice.clone();
    foreign.rp_id = OTHER.to_string();
    let bad = v.issue(b"s", Accept::Pairwise, &[], Some(foreign), None, NOW).is_err();
    req!(bad, true,
        "another origin's invoice");
    Ok(())
}

#[test]
fn invoices_are_checked_and_named_by_their_hash() -> Outcome<()> {
    let invoice = Invoice {
        rp_id:      APP.to_string(),
        payee:      "0123456789".to_string(),
        account:    "abcdef0123".to_string(),
        amount:     10,
        memo:       "Tea".to_string(),
        nonce:      [0u8; 16],
        expires:    1_000 + 3_600,
        ts:         1_000,
    };
    res!(invoice.check());
    let want = "{\"account\":\"abcdef0123\",\"expires\":4600,\"memo\":\"Tea\",\
        \"nonce\":\"AAAAAAAAAAAAAAAAAAAAAA\",\"oxes\":10,\"payee\":\"0123456789\",\
        \"rp_id\":\"https://app.example\",\"ts\":1000,\"v\":\"invoice/1\"}";
    req!(res!(invoice.to_json()), want.to_string());
    req!(res!(invoice.id()), sha256::digest(want.as_bytes()));
    req!(res!(Invoice::parse(want)), invoice.clone());

    let cases: Vec<(&str, Invoice)> = vec![
        ("no amount",           Invoice { amount: 0, ..invoice.clone() }),
        ("a long memo",         Invoice { memo: "m".repeat(65), ..invoice.clone() }),
        ("a control in the memo", Invoice { memo: "a\tb".to_string(), ..invoice.clone() }),
        ("a payee not an id",   Invoice { payee: "someone".to_string(), ..invoice.clone() }),
        ("too long a life",     Invoice { expires: 1_000 + 3_601, ..invoice.clone() }),
        ("expiring before issue", Invoice { expires: 999, ..invoice.clone() }),
        ("a bad origin",        Invoice { rp_id: "https://app.example/".to_string(), ..invoice.clone() }),
    ];
    for (what, bad) in cases {
        req!(bad.check().is_err(), true, "{}", what);
    }
    // A memo of 64 characters after trimming is within the limit.
    res!(Invoice { memo: fmt!("  {}  ", "m".repeat(64)), ..invoice.clone() }.check());
    Ok(())
}

#[test]
fn a_settlement_verifies_and_each_fault_is_refused() -> Outcome<()> {
    let (w, book) = res!(world());
    let v = res!(verifier(&w, book));
    let invoice = Invoice {
        rp_id:      APP.to_string(),
        payee:      "0123456789".to_string(),
        account:    "abcdef0123".to_string(),
        amount:     500,
        memo:       String::new(),
        nonce:      [1u8; 16],
        expires:    NOW + 3_600,
        ts:         NOW,
    };
    let settle = |signer: &Ed, invoice_id: [u8; 32], amount: u64, ts: u64| -> Outcome<Settlement> {
        let mut s = Settlement {
            invoice:    invoice_id,
            entry:      [2u8; 32],
            amount,
            ts,
            signer:     signer.public,
            sig:        [0u8; 64],
        };
        s.sig = res!(signer.sign(&res!(s.signed_bytes())));
        Ok(s)
    };
    let id = res!(invoice.id());
    let good = res!(settle(&w.peer, id, 500, NOW + 60));
    res!(v.verify_settlement(&good, &invoice));
    req!(res!(Settlement::parse(&res!(good.to_json()))), good.clone());

    let forger = res!(Ed::new());
    let mut spoilt = good.clone();
    spoilt.sig[3] ^= 0x01;
    let mut other_invoice = invoice.clone();
    other_invoice.rp_id = OTHER.to_string();
    for (what, s, inv) in [
        ("an outside signer",   res!(settle(&forger, id, 500, NOW + 60)), &invoice),
        ("a bad signature",     spoilt, &invoice),
        ("another invoice",     res!(settle(&w.peer, [9u8; 32], 500, NOW + 60)), &invoice),
        ("another amount",      res!(settle(&w.peer, id, 499, NOW + 60)), &invoice),
        ("after expiry",        res!(settle(&w.peer, id, 500, NOW + 3_601)), &invoice),
        ("another's invoice",   good.clone(), &other_invoice),
    ] {
        req!(v.verify_settlement(&s, inv).is_err(), true, "{}", what);
    }
    Ok(())
}

// ── Vendor neutrality ───────────────────────────────────────────────────────

/// The module serves any network of confirmed humans, so no source file in it
/// names one, or one's vocabulary.
#[test]
fn the_presentation_module_names_no_network() -> Outcome<()> {
    let sources = [
        ("mod.rs",      include_str!("../src/presentation/mod.rs")),
        ("shape.rs",    include_str!("../src/presentation/shape.rs")),
        ("verify.rs",   include_str!("../src/presentation/verify.rs")),
    ];
    let banned = ["oxe".to_string() + "gen", "oxe".to_string() + "nym", "ox".to_string() + "id",
        "hu".to_string() + "ser"];
    for (name, text) in sources {
        let lower = text.to_lowercase();
        for word in banned.iter() {
            req!(lower.contains(word.as_str()), false, "src/presentation/{} says '{}'", name, word);
        }
    }
    Ok(())
}
