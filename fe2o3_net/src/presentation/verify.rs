use crate::{
    presentation::shape::{
        Accept,
        HEAD_LEAD,
        Head,
        Invoice,
        NONCE_LEN,
        Presentation,
        Request,
        Settlement,
        Status,
        Subject,
        T_G,
        check_origin,
        key_id,
        scope,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedyne_fe2o3_crypto::{
    linkring::{
        self,
        Ring,
    },
    sign::verify_ed25519,
};
use oxedyne_fe2o3_hash::sha256;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        Mutex,
    },
};


/// What a verifier needs fetched: heads, the ring at a head, and the status of
/// a name. Each answer is the caller's to fetch and cache. A head and its ring
/// are content-addressed, so a ring decoded once serves every presentation made
/// against it. An error and a `None` both refuse the step that asked.
///
/// A head is checked here against its own hash and an issuer's signature, and a
/// ring against its head, so either may come from anywhere. A status carries no
/// signature, so it must come from a source the relying party trusts, such as
/// an issuer's own read API over TLS: whoever answers it decides which key a
/// name holds.
pub trait Lookup {
    fn head(&self, id: &[u8; 32]) -> Outcome<Option<Head>>;
    fn ring(&self, head: &Head) -> Outcome<Option<Arc<Ring>>>;
    fn status(&self, id: &str) -> Outcome<Option<Status>>;
}

/// The first check a presentation failed, as one of a fixed list of words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    Malformed,              // not the shape, or not `present/1`
    WrongAudience,          // `rp_id` is not this verifier's origin
    UnknownNonce,           // not issued here to this session, or past its `exp`
    Replayed,               // the nonce was spent before
    ModeNotAccepted,        // a mode the request did not accept
    PredicateNotOffered,    // a predicate the request did not ask for
    Stale,                  // `ts` more than T_G from now
    UnknownHead,            // not fetched, not its own hash, or not signed by an issuer
    StaleHead,              // older than T_G, or more than HEAD_LEAD after `ts`
    BadSignature,           // the signature fails, or a name is not its key's id
    NotLive,                // named: the name is not live with this key
    BadProof,               // pairwise: the ring proof fails over the head's whole ring
}

impl Refusal {
    pub fn word(&self) -> &'static str {
        match self {
            Self::Malformed             => "malformed",
            Self::WrongAudience         => "wrong_audience",
            Self::UnknownNonce          => "unknown_nonce",
            Self::Replayed              => "replayed",
            Self::ModeNotAccepted       => "mode_not_accepted",
            Self::PredicateNotOffered   => "predicate_not_offered",
            Self::Stale                 => "stale",
            Self::UnknownHead           => "unknown_head",
            Self::StaleHead             => "stale_head",
            Self::BadSignature          => "bad_signature",
            Self::NotLive               => "not_live",
            Self::BadProof              => "bad_proof",
        }
    }
}

/// What an accepted presentation establishes. A relying party mints its own
/// session from it; uniqueness and bans by tag are its policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verified {
    Named {
        id:         String,         // the name's key id, `sub`
        key:        [u8; 32],       // its Ed25519 key, `pub`
        predicates: Vec<String>,
    },
    Pairwise {
        key:        [u8; 32],       // the pseudonym key, `sub`, to re-challenge later
        tag:        [u8; 32],       // one per human at this relying party
        predicates: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
    Accepted(Verified),
    Refused(Refusal),
}

impl Verdict {
    /// The refusal, if this is one.
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            Self::Accepted(_)   => None,
            Self::Refused(r)    => Some(*r),
        }
    }
}

// ── Guards ──────────────────────────────────────────────────────────────────

// Each refusal that stands between an attacker and an accepted presentation
// has a bit here. The unit tests switch one off, on their own thread only, to
// prove the attack it stops then succeeds.
const G_AUDIENCE:   u32 = 1;    // rp_id is this verifier's own
const G_SESSION:    u32 = 2;    // the nonce was issued to this session
const G_REPLAY:     u32 = 4;    // the nonce is spent once
const G_ISSUER:     u32 = 8;    // the head is signed by a trusted issuer
const G_RING:       u32 = 16;   // the ring is the head's, by length and digest
const G_STATUS:     u32 = 32;   // a name is live with the presented key

#[cfg(test)]
thread_local! {
    static SKIP: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn on(g: u32) -> bool { SKIP.with(|s| s.get() & g == 0) }

#[cfg(not(test))]
#[inline(always)]
fn on(_g: u32) -> bool { true }

// ── Verifier ────────────────────────────────────────────────────────────────

struct Issued {
    session:    [u8; 32],   // SHA-256 of the session it was issued to
    req:        Request,
    spent:      bool,       // shown once already, pass or fail
}

struct Challenges {
    issued: HashMap<[u8; NONCE_LEN], Issued>,
}

impl Challenges {
    /// Drops the challenges whose requests have lapsed.
    fn evict(&mut self, now: u64) {
        self.issued.retain(|_, issued| issued.req.exp >= now);
    }
}

/// A relying party's presentation verifier, for one origin. Its methods take
/// `&self`, and the challenge store's lock is held only while a nonce is looked
/// up and spent, so a pass over a large ring never holds up another session.
pub struct Verifier<L: Lookup> {
    rp_id:      String,
    issuers:    Vec<[u8; 32]>,
    lookup:     L,
    threads:    usize,
    max_issued: usize,
    challenges: Mutex<Challenges>,
}

impl<L: Lookup> Verifier<L> {

    pub const MAX_ISSUED: usize = 1 << 16;

    /// A verifier for the relying party at `rp_id`, which trusts heads signed
    /// by `issuers`, Ed25519 keys.
    ///
    /// # Arguments
    ///
    /// * `threads` - how many threads a pairwise proof's pass over the ring may
    ///   use; 1 on wasm32.
    pub fn new(
        rp_id:      &str,
        issuers:    Vec<[u8; 32]>,
        lookup:     L,
        threads:    usize,
    )
        -> Outcome<Self>
    {
        res!(check_origin(rp_id));
        if issuers.is_empty() {
            return Err(err!(
                "A presentation verifier for {} with no issuer keys could accept nothing.",
                rp_id;
                Invalid, Input, Missing));
        }
        Ok(Self {
            rp_id:      rp_id.to_string(),
            issuers,
            lookup,
            threads:    threads.max(1),
            max_issued: Self::MAX_ISSUED,
            challenges: Mutex::new(Challenges { issued: HashMap::new() }),
        })
    }

    /// Caps the challenges outstanding at once. An issue beyond the cap is
    /// refused until older challenges lapse.
    pub fn with_max_issued(mut self, max_issued: usize) -> Self {
        self.max_issued = max_issued;
        self
    }

    pub fn rp_id(&self) -> &str { &self.rp_id }
    pub fn lookup(&self) -> &L { &self.lookup }

    /// Issues a challenge to one browser session: a fresh nonce, and the
    /// request that carries it, which lapses T_G after `now`.
    pub fn issue(
        &self,
        session:    &[u8],
        accept:     Accept,
        predicates: &[&str],
        invoice:    Option<Invoice>,
        return_to:  Option<&str>,
        now:        u64,
    )
        -> Outcome<Request>
    {
        let mut nonce = [0u8; NONCE_LEN];
        Rand::fill_u8(&mut nonce);
        let req = Request {
            rp_id:      self.rp_id.clone(),
            nonce,
            accept,
            predicates: predicates.iter().map(|w| w.to_string()).collect(),
            invoice,
            return_to:  return_to.map(|url| url.to_string()),
            exp:        now.saturating_add(T_G),
        };
        res!(req.check());
        let mut ch = lock_mutex!(self.challenges);
        ch.evict(now);
        if ch.issued.len() >= self.max_issued {
            return Err(err!(
                "{} challenges are outstanding at {}, the most this verifier holds, so no \
                more are issued until some lapse.", ch.issued.len(), self.rp_id;
                Excessive, Size));
        }
        ch.issued.insert(nonce, Issued {
            session:    sha256::digest(session),
            req:        req.clone(),
            spent:      false,
        });
        Ok(req)
    }

    /// Verifies a presentation that arrived in `session` at `now` (unix
    /// seconds). The nonce is spent by the first presentation to reach that
    /// check, whether or not it goes on to pass, so a presentation is accepted
    /// once at most. An error is this verifier's own fault, never the
    /// presentation's.
    pub fn verify(
        &self,
        session:    &[u8],
        body:       &[u8],
        now:        u64,
    )
        -> Outcome<Verdict>
    {
        let p = match std::str::from_utf8(body) {
            Ok(text) => match Presentation::parse(text) {
                Ok(p)   => p,
                Err(_)  => return Ok(Verdict::Refused(Refusal::Malformed)),
            },
            Err(_) => return Ok(Verdict::Refused(Refusal::Malformed)),
        };
        if on(G_AUDIENCE) && p.rp_id != self.rp_id {
            return Ok(Verdict::Refused(Refusal::WrongAudience));
        }
        let req = {
            let mut ch = lock_mutex!(self.challenges);
            ch.evict(now);
            let issued = match ch.issued.get_mut(&p.nonce) {
                Some(issued)    => issued,
                None            => return Ok(Verdict::Refused(Refusal::UnknownNonce)),
            };
            if (on(G_SESSION) && issued.session != sha256::digest(session)) || now > issued.req.exp {
                return Ok(Verdict::Refused(Refusal::UnknownNonce));
            }
            // The spent mark lives in the challenge, so it stands exactly as
            // long as the challenge does and a second showing reads as a
            // replay. A tracker windowed from the first showing could forget
            // it while the challenge still stood, whenever the caller's clock
            // stepped back between the issue and that showing.
            if on(G_REPLAY) && issued.spent {
                return Ok(Verdict::Refused(Refusal::Replayed));
            }
            issued.spent = true;
            issued.req.clone()
        };
        self.check_issued(&req, &p, now)
    }

    /// The checks after the nonce's, for a caller that keeps its own store of
    /// issued and spent nonces and has matched `p` to the request `req` it
    /// issued. The audience, the nonce and the request's lapse are asserted
    /// again here; spending the nonce once is the caller's.
    pub fn check_issued(
        &self,
        req:    &Request,
        p:      &Presentation,
        now:    u64,
    )
        -> Outcome<Verdict>
    {
        let refuse = |r| Ok(Verdict::Refused(r));
        if on(G_AUDIENCE) && (p.rp_id != self.rp_id || req.rp_id != self.rp_id) {
            return refuse(Refusal::WrongAudience);
        }
        if p.nonce != req.nonce || now > req.exp {
            return refuse(Refusal::UnknownNonce);
        }
        if !req.accept.admits(p.mode()) {
            return refuse(Refusal::ModeNotAccepted);
        }
        if p.predicates.iter().any(|w| !req.predicates.contains(w)) {
            return refuse(Refusal::PredicateNotOffered);
        }
        if now.abs_diff(p.ts) > T_G {
            return refuse(Refusal::Stale);
        }

        // The head: fetched, its own hash, signed by an issuer, and recent.
        let head = match self.lookup.head(&p.head) {
            Ok(Some(head))  => head,
            _               => return refuse(Refusal::UnknownHead),
        };
        let head_bytes = match head.signed_bytes() {
            Ok(bytes)   => bytes,
            Err(_)      => return refuse(Refusal::UnknownHead),
        };
        if head.id != p.head || sha256::digest(&head_bytes) != head.id {
            return refuse(Refusal::UnknownHead);
        }
        if on(G_ISSUER) && !self.issuers.contains(&head.signer) {
            return refuse(Refusal::UnknownHead);
        }
        if !matches!(verify_ed25519(&head.signer, &head_bytes, &head.sig), Ok(true)) {
            return refuse(Refusal::UnknownHead);
        }
        if head.ts.saturating_add(T_G) < now || head.ts > p.ts.saturating_add(HEAD_LEAD) {
            return refuse(Refusal::StaleHead);
        }

        let msg = match p.signed_bytes() {
            Ok(msg) => msg,
            Err(_)  => return refuse(Refusal::Malformed),
        };
        match &p.subject {
            Subject::Named { id, key } => {
                if key_id(key) != *id
                    || !matches!(verify_ed25519(key, &msg, &p.sig), Ok(true))
                {
                    return refuse(Refusal::BadSignature);
                }
                let status = match self.lookup.status(id) {
                    Ok(Some(status))    => status,
                    _                   => return refuse(Refusal::NotLive),
                };
                if on(G_STATUS) && (!status.live || status.key != *key) {
                    return refuse(Refusal::NotLive);
                }
                Ok(Verdict::Accepted(Verified::Named {
                    id:         id.clone(),
                    key:        *key,
                    predicates: p.predicates.clone(),
                }))
            },
            Subject::Pairwise { key, tag, proof } => {
                if !matches!(verify_ed25519(key, &msg, &p.sig), Ok(true)) {
                    return refuse(Refusal::BadSignature);
                }
                if proof.alg != linkring::ALG {
                    return refuse(Refusal::BadProof);
                }
                let ring = match self.lookup.ring(&head) {
                    Ok(Some(ring))  => ring,
                    _               => return refuse(Refusal::BadProof),
                };
                if on(G_RING)
                    && (ring.len() as u64 != head.ring_n || *ring.digest() != head.ring_digest)
                {
                    return refuse(Refusal::BadProof);
                }
                // The scope is this verifier's own origin, so a proof made
                // under any other audience fails here as well as above.
                let proved = linkring::verify_par(
                    &ring, &scope(&self.rp_id), &msg, tag, &proof.body, self.threads);
                if !matches!(proved, Ok(true)) {
                    return refuse(Refusal::BadProof);
                }
                Ok(Verdict::Accepted(Verified::Pairwise {
                    key:        *key,
                    tag:        *tag,
                    predicates: p.predicates.clone(),
                }))
            },
        }
    }

    /// Checks that `settlement` shows `invoice`, which this relying party
    /// issued, paid: signed by an issuer, naming the invoice's id and amount,
    /// and made no later than the invoice expired. A relying party credits its
    /// own ledger on this, never on the word of the member's page.
    pub fn verify_settlement(
        &self,
        settlement: &Settlement,
        invoice:    &Invoice,
    )
        -> Outcome<()>
    {
        if invoice.rp_id != self.rp_id {
            return Err(err!(
                "The invoice was issued by {}, not by {}.", invoice.rp_id, self.rp_id;
                Invalid, Input, Mismatch));
        }
        if !self.issuers.contains(&settlement.signer) {
            return Err(err!(
                "The settlement is signed by a key outside {}'s issuers.", self.rp_id;
                Invalid, Input, Security));
        }
        if !res!(verify_ed25519(&settlement.signer, &res!(settlement.signed_bytes()), &settlement.sig)) {
            return Err(err!(
                "The settlement's signature does not verify against its signer.";
                Invalid, Input, Security));
        }
        if settlement.invoice != res!(invoice.id()) {
            return Err(err!(
                "The settlement names another invoice.";
                Invalid, Input, Mismatch));
        }
        if settlement.amount != invoice.amount {
            return Err(err!(
                "The settlement pays {} where the invoice asks {}.",
                settlement.amount, invoice.amount;
                Invalid, Input, Mismatch));
        }
        if settlement.ts > invoice.expires {
            return Err(err!(
                "The settlement at {} is after the invoice expired at {}.",
                settlement.ts, invoice.expires;
                Invalid, Input));
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::shape::{
        KEY_LEN,
        Proof,
        SALT_LEN,
        SIG_LEN,
    };

    use oxedyne_fe2o3_crypto::{
        linkring::SecretKey,
        sign::SignatureScheme,
    };
    use oxedyne_fe2o3_iop_crypto::{
        keys::KeyManager,
        sign::Signer,
    };

    const NOW: u64 = 1_800_000_000;

    /// Runs `f` with the guard `g` switched off on this thread.
    fn without<T>(g: u32, f: impl FnOnce() -> T) -> T {
        SKIP.with(|s| s.set(g));
        let out = f();
        SKIP.with(|s| s.set(0));
        out
    }

    struct Ed {
        scheme: SignatureScheme,
        public: [u8; KEY_LEN],
    }

    impl Ed {
        fn new() -> Outcome<Self> {
            let scheme = SignatureScheme::new_ed25519();
            let mut public = [0u8; KEY_LEN];
            match res!(scheme.get_public_key()) {
                Some(pk) => public.copy_from_slice(pk),
                None => return Err(err!("A new Ed25519 key has no public half."; Bug, Missing)),
            }
            Ok(Self { scheme, public })
        }

        fn sign(&self, msg: &[u8]) -> Outcome<[u8; SIG_LEN]> {
            let sig = res!(self.scheme.sign(msg));
            let mut out = [0u8; SIG_LEN];
            out.copy_from_slice(&sig);
            Ok(out)
        }
    }

    #[derive(Default)]
    struct Book {
        heads:      HashMap<[u8; 32], Head>,
        rings:      HashMap<[u8; 32], Arc<Ring>>,   // by head id
        statuses:   HashMap<String, Status>,
    }

    impl Lookup for Book {
        fn head(&self, id: &[u8; 32]) -> Outcome<Option<Head>> {
            Ok(self.heads.get(id).cloned())
        }
        fn ring(&self, head: &Head) -> Outcome<Option<Arc<Ring>>> {
            Ok(self.rings.get(&head.id).cloned())
        }
        fn status(&self, id: &str) -> Outcome<Option<Status>> {
            Ok(self.statuses.get(id).cloned())
        }
    }

    /// A head at `ts` over `ring`, signed by `signer`, filed with its ring.
    fn mint(book: &mut Book, signer: &Ed, ring: &Arc<Ring>, ts: u64) -> Outcome<[u8; 32]> {
        let mut head = Head {
            id:             [0u8; 32],
            epoch:          1,
            prev:           None,
            ts,
            salt:           [7u8; SALT_LEN],
            members:        ring.len() as u64,
            ring_n:         ring.len() as u64,
            ring_digest:    *ring.digest(),
            signer:         signer.public,
            sig:            [0u8; SIG_LEN],
        };
        let bytes = res!(head.signed_bytes());
        head.sig = res!(signer.sign(&bytes));
        head.id = res!(head.compute_id());
        book.rings.insert(head.id, ring.clone());
        book.heads.insert(head.id, head.clone());
        Ok(head.id)
    }

    fn ring_of(keys: &[SecretKey]) -> Outcome<Arc<Ring>> {
        let mut list: Vec<[u8; 32]> = keys.iter().map(|k| k.public_key()).collect();
        list.sort();
        Ok(Arc::new(res!(Ring::from_keys(&list))))
    }

    fn named(req: &Request, member: &Ed, head: [u8; 32], rp_id: &str) -> Outcome<Vec<u8>> {
        let mut p = Presentation {
            rp_id:      rp_id.to_string(),
            nonce:      req.nonce,
            subject:    Subject::Named { id: key_id(&member.public), key: member.public },
            predicates: vec![],
            head,
            ts:         NOW,
            sig:        [0u8; SIG_LEN],
        };
        p.sig = res!(member.sign(&res!(p.signed_bytes())));
        Ok(res!(p.to_json()).into_bytes())
    }

    fn pairwise(
        req:    &Request,
        sub:    &Ed,
        key:    &SecretKey,
        ring:   &Ring,
        head:   [u8; 32],
        rp_id:  &str,
    )
        -> Outcome<Vec<u8>>
    {
        let scope = scope(rp_id);
        let mut p = Presentation {
            rp_id:      rp_id.to_string(),
            nonce:      req.nonce,
            subject:    Subject::Pairwise {
                key:    sub.public,
                tag:    res!(linkring::tag(key, &scope)),
                proof:  Proof { alg: linkring::ALG.to_string(), body: Vec::new() },
            },
            predicates: vec![],
            head,
            ts:         NOW,
            sig:        [0u8; SIG_LEN],
        };
        let msg = res!(p.signed_bytes());
        let (_, body) = res!(linkring::sign(ring, key, &scope, &msg));
        if let Subject::Pairwise { proof, .. } = &mut p.subject {
            proof.body = body;
        }
        p.sig = res!(sub.sign(&msg));
        Ok(res!(p.to_json()).into_bytes())
    }

    struct World {
        peer:   Ed,
        member: Ed,
        keys:   Vec<SecretKey>,
        ring:   Arc<Ring>,
        head:   [u8; 32],
    }

    fn world() -> Outcome<(World, Book)> {
        let mut book = Book::default();
        let peer = res!(Ed::new());
        let member = res!(Ed::new());
        let mut keys = Vec::new();
        for _ in 0..5 {
            keys.push(res!(SecretKey::random()));
        }
        let ring = res!(ring_of(&keys));
        let head = res!(mint(&mut book, &peer, &ring, NOW - 10));
        book.statuses.insert(key_id(&member.public), Status { key: member.public, live: true });
        Ok((World { peer, member, keys, ring, head }, book))
    }

    fn accepted(v: &Verdict) -> bool {
        matches!(v, Verdict::Accepted(_))
    }

    /// The nonce is spent once. With the replay guard off the same
    /// presentation is accepted a second time.
    #[test]
    fn replay_guard_is_load_bearing() -> Outcome<()> {
        let (w, book) = res!(world());
        let v = res!(Verifier::new("https://app.example", vec![w.peer.public], book, 1));
        let req = res!(v.issue(b"alice", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let body = res!(named(&req, &w.member, w.head, "https://app.example"));
        assert!(accepted(&res!(v.verify(b"alice", &body, NOW))), "the first showing passes");
        assert_eq!(res!(v.verify(b"alice", &body, NOW)).refusal(), Some(Refusal::Replayed));
        let again = without(G_REPLAY, || v.verify(b"alice", &body, NOW));
        assert!(accepted(&res!(again)), "without the guard the replay is accepted");
        Ok(())
    }

    /// A relying party that relays another's challenge to a member, under its
    /// own origin, and forwards the answer, is refused by the audience check.
    /// Named mode is the case to try, since a pairwise proof's scope refuses
    /// the foreign origin a second time.
    #[test]
    fn audience_guard_is_load_bearing() -> Outcome<()> {
        let (w, book) = res!(world());
        let v = res!(Verifier::new("https://app.example", vec![w.peer.public], book, 1));
        let req = res!(v.issue(b"alice", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let body = res!(named(&req, &w.member, w.head, "https://evil.example"));
        assert_eq!(res!(v.verify(b"alice", &body, NOW)).refusal(), Some(Refusal::WrongAudience));
        let relayed = without(G_AUDIENCE, || v.verify(b"alice", &body, NOW));
        assert!(accepted(&res!(relayed)), "without the guard the relayed answer is accepted");
        Ok(())
    }

    /// A presentation made for one session is refused in another.
    #[test]
    fn session_guard_is_load_bearing() -> Outcome<()> {
        let (w, book) = res!(world());
        let v = res!(Verifier::new("https://app.example", vec![w.peer.public], book, 1));
        let req = res!(v.issue(b"alice", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let body = res!(named(&req, &w.member, w.head, "https://app.example"));
        assert_eq!(res!(v.verify(b"mallory", &body, NOW)).refusal(), Some(Refusal::UnknownNonce));
        let moved = without(G_SESSION, || v.verify(b"mallory", &body, NOW));
        assert!(accepted(&res!(moved)), "without the guard another session completes it");
        Ok(())
    }

    /// A head the attacker mints over a ring of its own keys is refused for
    /// want of an issuer's signature.
    #[test]
    fn issuer_guard_is_load_bearing() -> Outcome<()> {
        let (w, mut book) = res!(world());
        let forger = res!(Ed::new());
        let own = vec![res!(SecretKey::random()), res!(SecretKey::random())];
        let own_ring = res!(ring_of(&own));
        let fake = res!(mint(&mut book, &forger, &own_ring, NOW - 10));
        let v = res!(Verifier::new("https://app.example", vec![w.peer.public], book, 1));
        let sub = res!(Ed::new());
        let req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
        let body = res!(pairwise(&req, &sub, &own[0], &own_ring, fake, "https://app.example"));
        assert_eq!(res!(v.verify(b"s", &body, NOW)).refusal(), Some(Refusal::UnknownHead));
        let req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
        let body = res!(pairwise(&req, &sub, &own[0], &own_ring, fake, "https://app.example"));
        let forged = without(G_ISSUER, || v.verify(b"s", &body, NOW));
        assert!(accepted(&res!(forged)), "without the guard a self-minted head is accepted");
        Ok(())
    }

    /// A ring served for a head whose digest it does not match is refused,
    /// even when the proof over it is sound.
    #[test]
    fn ring_guard_is_load_bearing() -> Outcome<()> {
        let (w, mut book) = res!(world());
        let own = vec![res!(SecretKey::random()), res!(SecretKey::random())];
        let own_ring = res!(ring_of(&own));
        // The issuer's head, with the attacker's ring filed against it.
        book.rings.insert(w.head, own_ring.clone());
        let v = res!(Verifier::new("https://app.example", vec![w.peer.public], book, 1));
        let sub = res!(Ed::new());
        let req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
        let body = res!(pairwise(&req, &sub, &own[1], &own_ring, w.head, "https://app.example"));
        assert_eq!(res!(v.verify(b"s", &body, NOW)).refusal(), Some(Refusal::BadProof));
        let req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
        let body = res!(pairwise(&req, &sub, &own[1], &own_ring, w.head, "https://app.example"));
        let swapped = without(G_RING, || v.verify(b"s", &body, NOW));
        assert!(accepted(&res!(swapped)), "without the guard a foreign ring is accepted");
        Ok(())
    }

    /// A name that is no longer live, or that now holds another key, is
    /// refused.
    #[test]
    fn status_guard_is_load_bearing() -> Outcome<()> {
        let (w, mut book) = res!(world());
        book.statuses.insert(key_id(&w.member.public), Status { key: w.member.public, live: false });
        let v = res!(Verifier::new("https://app.example", vec![w.peer.public], book, 1));
        let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let body = res!(named(&req, &w.member, w.head, "https://app.example"));
        assert_eq!(res!(v.verify(b"s", &body, NOW)).refusal(), Some(Refusal::NotLive));
        let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let body = res!(named(&req, &w.member, w.head, "https://app.example"));
        let erased = without(G_STATUS, || v.verify(b"s", &body, NOW));
        assert!(accepted(&res!(erased)), "without the guard an erased name is accepted");
        Ok(())
    }

    /// Every guard on, the honest cases pass in both modes, so the refusals
    /// above are about the attacks and not about the fixtures.
    #[test]
    fn the_honest_cases_pass_with_every_guard_on() -> Outcome<()> {
        let (w, book) = res!(world());
        let v = res!(Verifier::new("https://app.example", vec![w.peer.public], book, 1));
        let req = res!(v.issue(b"s", Accept::PairwiseOrNamed, &[], None, None, NOW));
        let body = res!(named(&req, &w.member, w.head, "https://app.example"));
        assert!(accepted(&res!(v.verify(b"s", &body, NOW))), "named");
        let sub = res!(Ed::new());
        let req = res!(v.issue(b"s", Accept::Pairwise, &[], None, None, NOW));
        let body = res!(pairwise(&req, &sub, &w.keys[2], &w.ring, w.head, "https://app.example"));
        assert!(accepted(&res!(v.verify(b"s", &body, NOW))), "pairwise");
        Ok(())
    }
}
