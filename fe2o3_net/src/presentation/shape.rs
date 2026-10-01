use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::linkring;
use oxedyne_fe2o3_hash::sha256;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    bdat::DecodeLimits,
};
use oxedyne_fe2o3_text::base64;


// Protocol words
pub const V_REQUEST:            &str    = "present-req/1";
pub const V_PRESENTATION:       &str    = "present/1";
pub const V_HEAD:               &str    = "head/1";
pub const V_INVOICE:            &str    = "invoice/1";
pub const V_SETTLEMENT:         &str    = "settle/1";
pub const SCOPE_PREFIX:         &str    = "present/1:";
pub const ALG_RING:             &str    = linkring::ALG;

// Clocks, in seconds
pub const T_G:                  u64     = 300;      // freshness of a head, a presentation and a nonce
pub const HEAD_LEAD:            u64     = 60;       // how far a head may postdate its presentation
pub const INVOICE_LIFE:         u64     = 3_600;    // longest an invoice may run

// Sizes
pub const NONCE_LEN:            usize   = 32;
pub const KEY_LEN:              usize   = 32;
pub const SIG_LEN:              usize   = 64;
pub const SALT_LEN:             usize   = 16;
pub const INVOICE_NONCE_LEN:    usize   = 16;
pub const KEY_ID_CHARS:         usize   = 10;
pub const MEMO_MAX_CHARS:       usize   = 64;
pub const WORD_MAX_CHARS:       usize   = 32;
pub const WORDS_MAX:            usize   = 16;
pub const JSON_MAX_BYTES:       usize   = 64 * 1024;
pub const MAX_INT:              u64     = 9_007_199_254_740_991;    // 2^53 - 1

const JSON_MAX_DEPTH:           usize   = 16;       // value nesting a parse descends to

// Members, by shape
const REQUEST_MEMBERS: [&str; 8] =
    ["v", "rp_id", "nonce", "modes", "predicates", "invoice", "return_to", "exp"];
const REQUEST_RESERVED: [&str; 2] = ["org", "osig"]; // ignored until origins are bound to names
const NAMED_MEMBERS: [&str; 10] =
    ["v", "mode", "rp_id", "nonce", "sub", "pub", "predicates", "head", "ts", "sig"];
const PAIRWISE_MEMBERS: [&str; 11] =
    ["v", "mode", "rp_id", "nonce", "sub", "tag", "predicates", "head", "ts", "proof", "sig"];
const PROOF_MEMBERS: [&str; 2] = ["alg", "body"];
const HEAD_MEMBERS: [&str; 11] = [
    "v", "head", "epoch", "prev", "ts", "salt", "members", "ring_n", "ring_digest", "signer", "sig",
];
const INVOICE_MEMBERS: [&str; 9] =
    ["v", "rp_id", "payee", "account", "oxes", "memo", "nonce", "expires", "ts"];
const SETTLEMENT_MEMBERS: [&str; 7] = ["v", "invoice", "entry", "oxes", "ts", "signer", "sig"];


// ── Rules ───────────────────────────────────────────────────────────────────

/// Checks that `origin` is an origin in RFC 6454 ASCII serialisation, spelt the
/// one way a verifier compares it, byte for byte: `scheme "://" host [":" port]`,
/// scheme and host lowercase, the default port omitted, and no path, trailing
/// slash or user information. The scheme is `https`, or `http` with a host of
/// `localhost` or `127.0.0.1` for development. An IPv6 literal is not accepted,
/// and an IPv4 address only as a dotted quad.
pub fn check_origin(origin: &str) -> Outcome<()> {
    let (scheme, rest) = match origin.split_once("://") {
        Some(parts) => parts,
        None => return Err(err!(
            "The origin '{}' has no '://' after its scheme.", origin;
            Invalid, Input)),
    };
    let default_port = match scheme {
        "https" => "443",
        "http"  => "80",
        _ => return Err(err!(
            "The origin '{}' has scheme '{}', where 'https' is required ('http' only for \
            localhost).", origin, scheme;
            Invalid, Input)),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    if host.is_empty() || host.len() > 253 {
        return Err(err!(
            "The origin '{}' has a host of {} characters, where 1 to 253 are allowed.",
            origin, host.len();
            Invalid, Input, Size));
    }
    // Lowercase letters, digits, hyphens and dots only, which also refuses a
    // path, a slash, user information, a query and an upper case letter.
    if let Some(c) = host.chars().find(|c| !(c.is_ascii_lowercase()
        || c.is_ascii_digit() || *c == '-' || *c == '.'))
    {
        return Err(err!(
            "The origin '{}' has '{}' in its host, which a serialised origin never \
            carries there.", origin, c.escape_default();
            Invalid, Input));
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') {
            return Err(err!(
                "The origin '{}' has the host label '{}', which is empty, longer than 63 \
                characters, or begins or ends with a hyphen.", origin, label;
                Invalid, Input));
        }
    }
    // A browser reads a host whose last label is a number, decimal or hex, as
    // an IPv4 address and serialises it as a dotted quad, so `https://127.1` is
    // `https://127.0.0.1` there (WHATWG URL, "ends in a number").
    if ends_in_a_number(host) && !is_dotted_quad(host) {
        return Err(err!(
            "The origin '{}' has a host that a browser reads as an IPv4 address, and \
            serialises otherwise than as written; only a dotted quad is.", origin;
            Invalid, Input));
    }
    if scheme == "http" && host != "localhost" && host != "127.0.0.1" {
        return Err(err!(
            "The origin '{}' is plain http on a host other than localhost or 127.0.0.1.",
            origin;
            Invalid, Input, Security));
    }
    if let Some(port) = port {
        // Digits only, since `parse` would also take a leading '+'.
        let digits = !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
        let number = match port.parse::<u32>() {
            Ok(n) if digits => n,
            _               => 0,
        };
        if port.starts_with('0') || number == 0 || number > 65_535 {
            return Err(err!(
                "The origin '{}' has the port '{}', which is not a number from 1 to 65535 \
                without leading zeros.", origin, port;
                Invalid, Input));
        }
        if port == default_port {
            return Err(err!(
                "The origin '{}' names its scheme's default port, which a serialised \
                origin omits.", origin;
                Invalid, Input));
        }
    }
    Ok(())
}

/// Does the host's last label read as a number, all decimal digits or `0x` and
/// hex digits?
fn ends_in_a_number(host: &str) -> bool {
    let last = match host.rsplit('.').next() {
        Some(last) => last,
        None => return false,
    };
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    match last.strip_prefix("0x") {
        Some(hex) => hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

/// Is the host four decimal numbers from 0 to 255, without leading zeros?
fn is_dotted_quad(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| {
        !p.is_empty()
            && p.bytes().all(|b| b.is_ascii_digit())
            && (p.len() == 1 || !p.starts_with('0'))
            && matches!(p.parse::<u16>(), Ok(n) if n <= 255)
    })
}

/// The key id a public key earns: the first ten lowercase hex characters of
/// SHA-256 over its raw bytes.
pub fn key_id(public: &[u8]) -> String {
    let digest = sha256::digest(public);
    let mut id = String::with_capacity(KEY_ID_CHARS);
    for b in &digest[..KEY_ID_CHARS / 2] {
        id.push_str(&fmt!("{:02x}", b));
    }
    id
}

/// Is `s` a key id, ten lowercase hex characters?
pub fn is_key_id(s: &str) -> bool {
    s.len() == KEY_ID_CHARS && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Is `s` a predicate word, 1 to 32 lowercase ASCII letters, digits and
/// underscores?
pub fn is_word(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= WORD_MAX_CHARS
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The scope a pairwise proof for `rp_id` links under: `"present/1:"` and the
/// origin, as UTF-8 bytes. One human has one tag per scope.
pub fn scope(rp_id: &str) -> Vec<u8> {
    let mut s = Vec::with_capacity(SCOPE_PREFIX.len() + rp_id.len());
    s.extend_from_slice(SCOPE_PREFIX.as_bytes());
    s.extend_from_slice(rp_id.as_bytes());
    s
}


// ── Modes ───────────────────────────────────────────────────────────────────

/// How a member presents: under a public name, or under a pseudonym and a tag
/// that only this relying party sees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Named,
    Pairwise,
}

impl Mode {
    pub fn word(&self) -> &'static str {
        match self {
            Self::Named     => "named",
            Self::Pairwise  => "pairwise",
        }
    }

    pub fn from_word(s: &str) -> Option<Self> {
        match s {
            "named"     => Some(Self::Named),
            "pairwise"  => Some(Self::Pairwise),
            _           => None,
        }
    }
}

/// The modes a request accepts. Pairwise is always among them, so a relying
/// party may accept a public name but cannot demand one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Accept {
    Pairwise,           // wire ["pairwise"]
    PairwiseOrNamed,    // wire ["pairwise", "named"]
}

impl Accept {
    /// Does a request carrying this accept a presentation in `mode`?
    pub fn admits(&self, mode: Mode) -> bool {
        match (self, mode) {
            (_, Mode::Pairwise)                 => true,
            (Self::PairwiseOrNamed, Mode::Named) => true,
            (Self::Pairwise, Mode::Named)       => false,
        }
    }

    fn to_dat(&self) -> Dat {
        match self {
            Self::Pairwise          => listdat![Mode::Pairwise.word()],
            Self::PairwiseOrNamed   => listdat![Mode::Pairwise.word(), Mode::Named.word()],
        }
    }

    fn from_dat(dat: &Dat) -> Outcome<Self> {
        let words: Vec<&str> = match dat {
            Dat::List(list) => list.iter().filter_map(|d| match d {
                Dat::Str(s) => Some(s.as_str()),
                _ => None,
            }).collect(),
            _ => Vec::new(),
        };
        match (dat, words.as_slice()) {
            (Dat::List(list), ["pairwise"]) if list.len() == 1 => Ok(Self::Pairwise),
            (Dat::List(list), ["pairwise", "named"]) if list.len() == 2 => Ok(Self::PairwiseOrNamed),
            _ => Err(err!(
                "A request's modes are [\"pairwise\"] or [\"pairwise\", \"named\"], not {:?}.",
                dat;
                Invalid, Input)),
        }
    }
}


// ── Request ─────────────────────────────────────────────────────────────────

/// A relying party's challenge to one browser session: its origin, a nonce,
/// what it accepts and asks, and when the challenge lapses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub rp_id:      String,
    pub nonce:      [u8; NONCE_LEN],
    pub accept:     Accept,
    pub predicates: Vec<String>,
    pub invoice:    Option<Invoice>,
    pub return_to:  Option<String>,     // for the redirect route only, a URL on `rp_id`
    pub exp:        u64,
}

impl Request {

    /// Checks what the shape alone cannot: the origin, the words, the return
    /// URL and any invoice.
    pub fn check(&self) -> Outcome<()> {
        res!(check_origin(&self.rp_id));
        res!(check_words(&self.predicates, "request"));
        if let Some(url) = &self.return_to {
            let on_origin = url.starts_with(&self.rp_id)
                && url.as_bytes().get(self.rp_id.len()) == Some(&b'/');
            if !on_origin || url.chars().any(|c| c == '#' || c.is_whitespace() || c.is_control()) {
                return Err(err!(
                    "The return URL '{}' is not a URL on {} without a fragment.", url, self.rp_id;
                    Invalid, Input));
            }
        }
        if let Some(invoice) = &self.invoice {
            res!(invoice.check());
            if invoice.rp_id != self.rp_id {
                return Err(err!(
                    "The invoice was issued by {}, and the request by {}.",
                    invoice.rp_id, self.rp_id;
                    Invalid, Input, Mismatch));
            }
        }
        Ok(())
    }

    pub fn to_dat(&self) -> Dat {
        let mut m = DaticleMap::new();
        m.insert(dat!("v"),             dat!(V_REQUEST));
        m.insert(dat!("rp_id"),         dat!(self.rp_id.clone()));
        m.insert(dat!("nonce"),         dat!(base64::encode_url(&self.nonce)));
        m.insert(dat!("modes"),         self.accept.to_dat());
        m.insert(dat!("predicates"),    words_dat(&self.predicates));
        if let Some(invoice) = &self.invoice {
            m.insert(dat!("invoice"),   invoice.to_dat());
        }
        if let Some(url) = &self.return_to {
            m.insert(dat!("return_to"), dat!(url.clone()));
        }
        m.insert(dat!("exp"),           Dat::U64(self.exp));
        Dat::Map(m)
    }

    pub fn to_json(&self) -> Outcome<String> {
        self.to_dat().json_canonical()
    }

    /// Reads a request strictly: an unknown member is refused, save `org` and
    /// `osig`, which are reserved and ignored.
    pub fn from_dat(dat: &Dat) -> Outcome<Self> {
        let o = res!(Obj::of(dat, "request"));
        res!(o.only(&REQUEST_MEMBERS, &REQUEST_RESERVED));
        res!(o.version(V_REQUEST));
        let invoice = match o.get("invoice") {
            Some(d) => Some(res!(Invoice::from_dat(d))),
            None    => None,
        };
        let return_to = match o.get("return_to") {
            Some(_) => Some(res!(o.text("return_to"))),
            None    => None,
        };
        let req = Self {
            rp_id:      res!(o.text("rp_id")),
            nonce:      res!(o.bytes::<NONCE_LEN>("nonce")),
            accept:     res!(Accept::from_dat(res!(o.must("modes")))),
            predicates: res!(o.words("predicates")),
            invoice,
            return_to,
            exp:        res!(o.uint("exp")),
        };
        res!(req.check());
        Ok(req)
    }

    pub fn parse(json: &str) -> Outcome<Self> {
        Self::from_dat(&res!(decode_json(json, "request")))
    }
}


// ── Presentation ────────────────────────────────────────────────────────────

/// Who presents, with what the mode needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Subject {
    Named {
        id:     String,             // the key id of `key`, on the wire as `sub`
        key:    [u8; KEY_LEN],      // Ed25519, on the wire as `pub`
    },
    Pairwise {
        key:    [u8; KEY_LEN],      // Ed25519 pseudonym for this relying party, `sub`
        tag:    [u8; KEY_LEN],      // linking tag under this relying party's scope
        proof:  Proof,
    },
}

/// A ring proof, named by its algorithm. Its bytes are the algorithm's own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proof {
    pub alg:    String,
    pub body:   Vec<u8>,
}

/// A member's answer to a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Presentation {
    pub rp_id:      String,
    pub nonce:      [u8; NONCE_LEN],
    pub subject:    Subject,
    pub predicates: Vec<String>,
    pub head:       [u8; 32],
    pub ts:         u64,
    pub sig:        [u8; SIG_LEN],
}

impl Presentation {

    pub fn mode(&self) -> Mode {
        match self.subject {
            Subject::Named { .. }       => Mode::Named,
            Subject::Pairwise { .. }    => Mode::Pairwise,
        }
    }

    fn signed_map(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        m.insert(dat!("v"),     dat!(V_PRESENTATION));
        m.insert(dat!("mode"),  dat!(self.mode().word()));
        m.insert(dat!("rp_id"), dat!(self.rp_id.clone()));
        m.insert(dat!("nonce"), dat!(base64::encode_url(&self.nonce)));
        match &self.subject {
            Subject::Named { id, key } => {
                m.insert(dat!("sub"),   dat!(id.clone()));
                m.insert(dat!("pub"),   dat!(base64::encode_url(key)));
            },
            Subject::Pairwise { key, tag, .. } => {
                m.insert(dat!("sub"),   dat!(base64::encode_url(key)));
                m.insert(dat!("tag"),   dat!(base64::encode_url(tag)));
            },
        }
        m.insert(dat!("predicates"),    words_dat(&self.predicates));
        m.insert(dat!("head"),          dat!(base64::encode_url(&self.head)));
        m.insert(dat!("ts"),            Dat::U64(self.ts));
        m
    }

    /// The bytes the signature and a pairwise proof both cover: the canonical
    /// JSON of every member but `proof` and `sig`, rebuilt from the parsed
    /// values, so the sender's own spelling of the object cannot move a byte.
    pub fn signed_bytes(&self) -> Outcome<Vec<u8>> {
        Ok(res!(Dat::Map(self.signed_map()).json_canonical()).into_bytes())
    }

    pub fn to_dat(&self) -> Dat {
        let mut m = self.signed_map();
        if let Subject::Pairwise { proof, .. } = &self.subject {
            m.insert(dat!("proof"), mapdat!{
                "alg"   => proof.alg.clone(),
                "body"  => base64::encode_url(&proof.body),
            });
        }
        m.insert(dat!("sig"), dat!(base64::encode_url(&self.sig)));
        Dat::Map(m)
    }

    pub fn to_json(&self) -> Outcome<String> {
        self.to_dat().json_canonical()
    }

    /// Reads a presentation strictly: exactly the members its mode carries, each
    /// of its stated form.
    pub fn from_dat(dat: &Dat) -> Outcome<Self> {
        let o = res!(Obj::of(dat, "presentation"));
        res!(o.version(V_PRESENTATION));
        let mode = match Mode::from_word(&res!(o.text("mode"))) {
            Some(mode) => mode,
            None => return Err(err!(
                "A presentation's mode is 'named' or 'pairwise'.";
                Invalid, Input)),
        };
        let subject = match mode {
            Mode::Named => {
                res!(o.only(&NAMED_MEMBERS, &[]));
                let id = res!(o.text("sub"));
                if !is_key_id(&id) {
                    return Err(err!(
                        "A named presentation's sub is a key id of ten lowercase hex \
                        characters, not '{}'.", id;
                        Invalid, Input));
                }
                Subject::Named { id, key: res!(o.bytes::<KEY_LEN>("pub")) }
            },
            Mode::Pairwise => {
                res!(o.only(&PAIRWISE_MEMBERS, &[]));
                let p = res!(Obj::of(res!(o.must("proof")), "proof"));
                res!(p.only(&PROOF_MEMBERS, &[]));
                let body = res!(base64::decode_url(&res!(p.text("body"))));
                if body.is_empty() {
                    return Err(err!("A proof's body is empty."; Invalid, Input, Missing));
                }
                Subject::Pairwise {
                    key:    res!(o.bytes::<KEY_LEN>("sub")),
                    tag:    res!(o.bytes::<KEY_LEN>("tag")),
                    proof:  Proof { alg: res!(p.text("alg")), body },
                }
            },
        };
        Ok(Self {
            rp_id:      res!(o.text("rp_id")),
            nonce:      res!(o.bytes::<NONCE_LEN>("nonce")),
            subject,
            predicates: res!(o.words("predicates")),
            head:       res!(o.bytes::<32>("head")),
            ts:         res!(o.uint("ts")),
            sig:        res!(o.bytes::<SIG_LEN>("sig")),
        })
    }

    pub fn parse(json: &str) -> Outcome<Self> {
        Self::from_dat(&res!(decode_json(json, "presentation")))
    }
}


// ── Head ────────────────────────────────────────────────────────────────────

/// A signed point in a network's history that a presentation is made against:
/// the member set's size and ring at a moment, under an issuer's key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Head {
    pub id:             [u8; 32],       // SHA-256 of the signed bytes, on the wire as `head`
    pub epoch:          u64,
    pub prev:           Option<[u8; 32]>,
    pub ts:             u64,
    pub salt:           [u8; SALT_LEN],
    pub members:        u64,
    pub ring_n:         u64,
    pub ring_digest:    [u8; 32],       // SHA-256 of the ring list
    pub signer:         [u8; KEY_LEN],  // Ed25519
    pub sig:            [u8; SIG_LEN],
}

impl Head {

    fn signed_map(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        m.insert(dat!("v"),             dat!(V_HEAD));
        m.insert(dat!("epoch"),         Dat::U64(self.epoch));
        m.insert(dat!("prev"), match &self.prev {
            Some(prev)  => dat!(base64::encode_url(prev)),
            None        => Dat::Empty,
        });
        m.insert(dat!("ts"),            Dat::U64(self.ts));
        m.insert(dat!("salt"),          dat!(base64::encode_url(&self.salt)));
        m.insert(dat!("members"),       Dat::U64(self.members));
        m.insert(dat!("ring_n"),        Dat::U64(self.ring_n));
        m.insert(dat!("ring_digest"),   dat!(base64::encode_url(&self.ring_digest)));
        m.insert(dat!("signer"),        dat!(base64::encode_url(&self.signer)));
        m
    }

    /// The bytes the signature covers and the id hashes: the canonical JSON of
    /// every member but `head` and `sig`, with `prev` a present null when there
    /// is none.
    pub fn signed_bytes(&self) -> Outcome<Vec<u8>> {
        Ok(res!(Dat::Map(self.signed_map()).json_canonical()).into_bytes())
    }

    /// The id this head's content earns, which its `id` must equal.
    pub fn compute_id(&self) -> Outcome<[u8; 32]> {
        Ok(sha256::digest(&res!(self.signed_bytes())))
    }

    pub fn to_dat(&self) -> Dat {
        let mut m = self.signed_map();
        m.insert(dat!("head"),  dat!(base64::encode_url(&self.id)));
        m.insert(dat!("sig"),   dat!(base64::encode_url(&self.sig)));
        Dat::Map(m)
    }

    pub fn to_json(&self) -> Outcome<String> {
        self.to_dat().json_canonical()
    }

    pub fn from_dat(dat: &Dat) -> Outcome<Self> {
        let o = res!(Obj::of(dat, "head"));
        res!(o.only(&HEAD_MEMBERS, &[]));
        res!(o.version(V_HEAD));
        Ok(Self {
            id:             res!(o.bytes::<32>("head")),
            epoch:          res!(o.uint("epoch")),
            prev:           res!(o.bytes_or_null::<32>("prev")),
            ts:             res!(o.uint("ts")),
            salt:           res!(o.bytes::<SALT_LEN>("salt")),
            members:        res!(o.uint("members")),
            ring_n:         res!(o.uint("ring_n")),
            ring_digest:    res!(o.bytes::<32>("ring_digest")),
            signer:         res!(o.bytes::<KEY_LEN>("signer")),
            sig:            res!(o.bytes::<SIG_LEN>("sig")),
        })
    }

    pub fn parse(json: &str) -> Outcome<Self> {
        Self::from_dat(&res!(decode_json(json, "head")))
    }
}


// ── Invoice and settlement ──────────────────────────────────────────────────

/// A relying party's bill to a member, paid once through the network's ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invoice {
    pub rp_id:      String,
    pub payee:      String,             // key id of the name being paid
    pub account:    String,             // key id of the account credited
    pub amount:     u64,                // on the wire as `oxes`, the ledger's unit
    pub memo:       String,
    pub nonce:      [u8; INVOICE_NONCE_LEN],
    pub expires:    u64,
    pub ts:         u64,
}

impl Invoice {

    /// Checks what the shape alone cannot. The memo is at most 64 characters
    /// once trimmed and carries no control character.
    pub fn check(&self) -> Outcome<()> {
        res!(check_origin(&self.rp_id));
        if !is_key_id(&self.payee) || !is_key_id(&self.account) {
            return Err(err!(
                "An invoice's payee and account are key ids of ten lowercase hex \
                characters, not '{}' and '{}'.", self.payee, self.account;
                Invalid, Input));
        }
        if self.amount == 0 || self.amount > MAX_INT {
            return Err(err!(
                "An invoice's amount is from 1 to 2^53 - 1, not {}.", self.amount;
                Invalid, Input));
        }
        let chars = self.memo.trim().chars().count();
        if chars > MEMO_MAX_CHARS || self.memo.chars().any(|c| c.is_control()) {
            return Err(err!(
                "An invoice's memo is at most {} characters once trimmed, with no control \
                character; this one has {} characters.", MEMO_MAX_CHARS, chars;
                Invalid, Input));
        }
        if self.expires < self.ts || self.expires - self.ts > INVOICE_LIFE {
            return Err(err!(
                "An invoice expires no earlier than it was issued and at most {} s after, \
                not {} s from {} to {}.", INVOICE_LIFE,
                self.expires as i128 - self.ts as i128, self.ts, self.expires;
                Invalid, Input));
        }
        Ok(())
    }

    pub fn to_dat(&self) -> Dat {
        mapdat!{
            "v"         => V_INVOICE,
            "rp_id"     => self.rp_id.clone(),
            "payee"     => self.payee.clone(),
            "account"   => self.account.clone(),
            "oxes"      => Dat::U64(self.amount),
            "memo"      => self.memo.clone(),
            "nonce"     => base64::encode_url(&self.nonce),
            "expires"   => Dat::U64(self.expires),
            "ts"        => Dat::U64(self.ts),
        }
    }

    pub fn to_json(&self) -> Outcome<String> {
        self.to_dat().json_canonical()
    }

    /// SHA-256 over the canonical JSON: what a payment's reference and a
    /// settlement name.
    pub fn id(&self) -> Outcome<[u8; 32]> {
        Ok(sha256::digest(res!(self.to_json()).as_bytes()))
    }

    pub fn from_dat(dat: &Dat) -> Outcome<Self> {
        let o = res!(Obj::of(dat, "invoice"));
        res!(o.only(&INVOICE_MEMBERS, &[]));
        res!(o.version(V_INVOICE));
        let invoice = Self {
            rp_id:      res!(o.text("rp_id")),
            payee:      res!(o.text("payee")),
            account:    res!(o.text("account")),
            amount:     res!(o.uint("oxes")),
            memo:       res!(o.text("memo")),
            nonce:      res!(o.bytes::<INVOICE_NONCE_LEN>("nonce")),
            expires:    res!(o.uint("expires")),
            ts:         res!(o.uint("ts")),
        };
        res!(invoice.check());
        Ok(invoice)
    }

    pub fn parse(json: &str) -> Outcome<Self> {
        Self::from_dat(&res!(decode_json(json, "invoice")))
    }
}

/// An issuer's signed word that an invoice was paid, naming no payer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settlement {
    pub invoice:    [u8; 32],       // the invoice's id
    pub entry:      [u8; 32],       // the ledger entry that paid it
    pub amount:     u64,            // on the wire as `oxes`
    pub ts:         u64,
    pub signer:     [u8; KEY_LEN],  // Ed25519
    pub sig:        [u8; SIG_LEN],
}

impl Settlement {

    fn signed_map(&self) -> DaticleMap {
        let mut m = DaticleMap::new();
        m.insert(dat!("v"),         dat!(V_SETTLEMENT));
        m.insert(dat!("invoice"),   dat!(base64::encode_url(&self.invoice)));
        m.insert(dat!("entry"),     dat!(base64::encode_url(&self.entry)));
        m.insert(dat!("oxes"),      Dat::U64(self.amount));
        m.insert(dat!("ts"),        Dat::U64(self.ts));
        m.insert(dat!("signer"),    dat!(base64::encode_url(&self.signer)));
        m
    }

    /// The bytes the signature covers: the canonical JSON of every member but
    /// `sig`.
    pub fn signed_bytes(&self) -> Outcome<Vec<u8>> {
        Ok(res!(Dat::Map(self.signed_map()).json_canonical()).into_bytes())
    }

    pub fn to_dat(&self) -> Dat {
        let mut m = self.signed_map();
        m.insert(dat!("sig"), dat!(base64::encode_url(&self.sig)));
        Dat::Map(m)
    }

    pub fn to_json(&self) -> Outcome<String> {
        self.to_dat().json_canonical()
    }

    pub fn from_dat(dat: &Dat) -> Outcome<Self> {
        let o = res!(Obj::of(dat, "settlement"));
        res!(o.only(&SETTLEMENT_MEMBERS, &[]));
        res!(o.version(V_SETTLEMENT));
        Ok(Self {
            invoice:    res!(o.bytes::<32>("invoice")),
            entry:      res!(o.bytes::<32>("entry")),
            amount:     res!(o.uint("oxes")),
            ts:         res!(o.uint("ts")),
            signer:     res!(o.bytes::<KEY_LEN>("signer")),
            sig:        res!(o.bytes::<SIG_LEN>("sig")),
        })
    }

    pub fn parse(json: &str) -> Outcome<Self> {
        Self::from_dat(&res!(decode_json(json, "settlement")))
    }
}


// ── Status ──────────────────────────────────────────────────────────────────

/// What a named-mode status lookup reports: the key a name holds now, and
/// whether it is live.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub key:    [u8; KEY_LEN],
    pub live:   bool,
}

impl Status {

    /// Reads `pub` and `live` from a status document and nothing else, since
    /// the document carries display fields a verifier has no use for.
    pub fn from_dat(dat: &Dat) -> Outcome<Self> {
        let o = res!(Obj::of(dat, "status"));
        Ok(Self {
            key:    res!(o.bytes::<KEY_LEN>("pub")),
            live:   res!(o.flag("live")),
        })
    }

    pub fn parse(json: &str) -> Outcome<Self> {
        Self::from_dat(&res!(decode_json(json, "status")))
    }
}


// ── Reading ─────────────────────────────────────────────────────────────────

/// Decodes a JSON document within the size and nesting these shapes need, as
/// RFC 8259 JSON and nothing else: none of JDAT's typed, hex or unquoted forms.
fn decode_json(json: &str, what: &str) -> Outcome<Dat> {
    if json.len() > JSON_MAX_BYTES {
        return Err(err!(
            "A {} of {} bytes exceeds the {} bytes one may take.", what, json.len(),
            JSON_MAX_BYTES;
            Invalid, Input, Size));
    }
    Ok(res!(Dat::decode_json_strict(json, &DecodeLimits::new(JSON_MAX_DEPTH, JSON_MAX_BYTES))))
}

fn words_dat(words: &[String]) -> Dat {
    Dat::List(words.iter().map(|w| dat!(w.clone())).collect())
}

fn check_words(words: &[String], what: &str) -> Outcome<()> {
    if words.len() > WORDS_MAX {
        return Err(err!(
            "A {} carries {} predicates, and at most {} are allowed.", what, words.len(), WORDS_MAX;
            Invalid, Input, Size));
    }
    for (i, w) in words.iter().enumerate() {
        if !is_word(w) {
            return Err(err!(
                "A {}'s predicate '{}' is not a word of 1 to {} lowercase letters, digits \
                and underscores.", what, w, WORD_MAX_CHARS;
                Invalid, Input));
        }
        if words[..i].contains(w) {
            return Err(err!(
                "A {} names the predicate '{}' twice.", what, w;
                Invalid, Input, Duplicate));
        }
    }
    Ok(())
}

/// The members of one JSON object, read strictly.
struct Obj<'a> {
    map:    &'a DaticleMap,
    what:   &'static str,   // what the object is, for errors
}

impl<'a> Obj<'a> {

    fn of(dat: &'a Dat, what: &'static str) -> Outcome<Self> {
        match dat {
            Dat::Map(map) => Ok(Self { map, what }),
            other => Err(err!(
                "A {} is a JSON object, not a {:?}.", what, other.kind();
                Invalid, Input, Decode)),
        }
    }

    /// Refuses any member outside `allowed`, save those in `ignored`.
    fn only(&self, allowed: &[&str], ignored: &[&str]) -> Outcome<()> {
        for key in self.map.keys() {
            match key {
                Dat::Str(s) if allowed.contains(&s.as_str()) || ignored.contains(&s.as_str()) => (),
                other => return Err(err!(
                    "A {} has no member {:?}.", self.what, other;
                    Invalid, Input, Unknown)),
            }
        }
        Ok(())
    }

    fn version(&self, v: &str) -> Outcome<()> {
        let got = res!(self.text("v"));
        if got != v {
            return Err(err!(
                "A {} carries v '{}', and this reads only '{}'.", self.what, got, v;
                Invalid, Input, Version));
        }
        Ok(())
    }

    fn get(&self, key: &str) -> Option<&'a Dat> {
        self.map.get(&dat!(key))
    }

    fn must(&self, key: &str) -> Outcome<&'a Dat> {
        match self.get(key) {
            Some(d) => Ok(d),
            None => Err(err!(
                "A {} lacks its '{}' member.", self.what, key;
                Invalid, Input, Missing)),
        }
    }

    fn text(&self, key: &str) -> Outcome<String> {
        match res!(self.must(key)) {
            Dat::Str(s) => Ok(s.clone()),
            other => Err(err!(
                "The '{}' of a {} is a string, not a {:?}.", key, self.what, other.kind();
                Invalid, Input, Mismatch)),
        }
    }

    fn flag(&self, key: &str) -> Outcome<bool> {
        match res!(self.must(key)) {
            Dat::Bool(b) => Ok(*b),
            other => Err(err!(
                "The '{}' of a {} is true or false, not a {:?}.", key, self.what, other.kind();
                Invalid, Input, Mismatch)),
        }
    }

    /// A non-negative integer no larger than 2^53 - 1, the largest a JSON
    /// number carries exactly.
    fn uint(&self, key: &str) -> Outcome<u64> {
        let d = res!(self.must(key));
        let n = match d {
            Dat::U8(n)              => *n as u64,
            Dat::U16(n)             => *n as u64,
            Dat::U32(n)             => *n as u64,
            Dat::U64(n)             => *n,
            Dat::I8(n)  if *n >= 0  => *n as u64,
            Dat::I16(n) if *n >= 0  => *n as u64,
            Dat::I32(n) if *n >= 0  => *n as u64,
            Dat::I64(n) if *n >= 0  => *n as u64,
            other => return Err(err!(
                "The '{}' of a {} is a non-negative integer, not {:?}.", key, self.what, other;
                Invalid, Input, Mismatch)),
        };
        if n > MAX_INT {
            return Err(err!(
                "The '{}' of a {} is {}, beyond 2^53 - 1, the largest integer JSON carries \
                exactly.", key, self.what, n;
                Invalid, Input, TooBig));
        }
        Ok(n)
    }

    /// Unpadded base64url of exactly `N` bytes.
    fn bytes<const N: usize>(&self, key: &str) -> Outcome<[u8; N]> {
        fixed::<N>(&res!(self.text(key)), key, self.what)
    }

    fn bytes_or_null<const N: usize>(&self, key: &str) -> Outcome<Option<[u8; N]>> {
        match res!(self.must(key)) {
            Dat::Empty => Ok(None),
            Dat::Opt(inner) if inner.is_none() => Ok(None),
            Dat::Str(s) => Ok(Some(res!(fixed::<N>(s, key, self.what)))),
            other => Err(err!(
                "The '{}' of a {} is base64url or null, not a {:?}.", key, self.what, other.kind();
                Invalid, Input, Mismatch)),
        }
    }

    fn words(&self, key: &str) -> Outcome<Vec<String>> {
        let list = match res!(self.must(key)) {
            Dat::List(list) => list,
            other => return Err(err!(
                "The '{}' of a {} is a list of words, not a {:?}.", key, self.what, other.kind();
                Invalid, Input, Mismatch)),
        };
        let mut words = Vec::with_capacity(list.len());
        for d in list {
            match d {
                Dat::Str(s) => words.push(s.clone()),
                other => return Err(err!(
                    "The '{}' of a {} holds a {:?} where a word belongs.", key, self.what,
                    other.kind();
                    Invalid, Input, Mismatch)),
            }
        }
        res!(check_words(&words, self.what));
        Ok(words)
    }
}

fn fixed<const N: usize>(b64: &str, key: &str, what: &str) -> Outcome<[u8; N]> {
    let v = res!(base64::decode_url(b64));
    if v.len() != N {
        return Err(err!(
            "The '{}' of a {} decodes to {} bytes, not {}.", key, what, v.len(), N;
            Invalid, Input, Size));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&v);
    Ok(out)
}
