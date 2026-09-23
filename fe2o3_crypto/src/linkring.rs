//! A scoped linkable ring signature, `linkring/1`, whose proof grows with the
//! logarithm of the ring.
//!
//! A signer holding one secret key of a ring of public keys proves that fact
//! without saying which key is theirs, and publishes a *tag* `τ = k·H_p(scope)`
//! that is fixed by the pair (key, scope) alone. The same key signing twice
//! under one scope gives one tag, so a verifier can count signers, while tags
//! under two scopes cannot be linked without the key (decisional
//! Diffie-Hellman). The proof also binds the tag to the key, so no signer can
//! produce the tag of another ring member.
//!
//! # Construction
//!
//! A Groth-Kohlweiss one-out-of-many proof in the radix-`n` form of Bootle et
//! al. (ESORICS 2015), with `n = 16`, over ristretto255, plus a second relation
//! on the base `U = H_p(scope)` that reuses the same response `z`:
//!
//! ```text
//!     Σ_i p_i(x)·P_i − Σ_k x^k·G_k = z·G       (ring)
//!     x^m·τ          − Σ_k x^k·Y_k = z·U       (tag, Y_k = ρ_k·U)
//! ```
//!
//! The ring is padded to `16^m` entries by repeating its last key, which costs
//! nothing: since `Σ_i p_i(x) = x^m`, the padded entries fold into one
//! coefficient on that key. The prover computes the `G_k` by expanding each
//! `p_i` over the digit positions where `i` agrees with the signer's index, so
//! signing costs about `(1 + 1/16)^m` ring multi-scalar multiplications rather
//! than `m` of them.
//!
//! `H_p` is RFC 9380's `hash_to_ristretto255` (expand_message_xmd with SHA-512,
//! then the RFC 9496 one-way map). The ring digest is SHA-256 over the
//! concatenated 32-byte keys, in ring order. The Hematite User Guide gives the
//! sizes and costs at 10^5 to 10^7 keys.
//!
//! # Encoding
//!
//! The body is `m ‖ A ‖ B ‖ C ‖ D ‖ G_0..G_{m-1} ‖ Y_0..Y_{m-1} ‖ f ‖ z_A ‖ z_C ‖ z`,
//! with `f` the `m·15` responses `f_{j,1..15}`, every point a compressed
//! ristretto255 encoding and every scalar canonical little-endian. Its first
//! byte fixes its length, `1 + 32·(7 + 17m)`. It carries no key and no index.
//! The tag travels beside it. A change to any of this is a new algorithm name.
//!
//! # What a false means
//!
//! As in `p256`, a malformed body or tag is a verification failure,
//! `Ok(false)`, not an error. Only a malformed ring is an error, because the
//! ring is the verifier's own input.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::sha256;

use std::sync::Mutex;

use curve25519_dalek::{
    constants::RISTRETTO_BASEPOINT_POINT,
    ristretto::{
        CompressedRistretto,
        RistrettoPoint,
    },
    scalar::Scalar,
    traits::{
        Identity,
        IsIdentity,
        MultiscalarMul,
        VartimeMultiscalarMul,
    },
};
use rand_core::{
    OsRng,
    RngCore,
};
use sha2::{
    Digest,
    Sha512,
};
use zeroize::Zeroize;

pub const ALG:          &str    = "linkring/1";
pub const RADIX:        usize   = 16;
pub const KEY_LEN:      usize   = 32;
pub const TAG_LEN:      usize   = 32;
pub const MAX_DIGITS:   usize   = 8;    // rings up to 16^8 = 2^32 keys

// Domain separation
const DST_SCOPE:        &[u8]   = b"linkring/1:scope";
const DST_GEN:          &[u8]   = b"linkring/1:gen";
const DST_SECRET:       &[u8]   = b"linkring/1:secret";
const DST_NONCE:        &[u8]   = b"linkring/1:nonce";

const CHUNK:            usize   = 1 << 16;  // points per multi-scalar multiplication

/// The number of radix-16 digits `m` a ring of `n` keys uses.
pub fn digits(n: usize) -> usize {
    let mut m = 1;
    let mut cap = RADIX;
    while cap < n {
        m += 1;
        cap = cap.saturating_mul(RADIX);
    }
    m
}

/// The body length for `m` digits.
pub fn body_len(m: usize) -> usize {
    1 + 32 * (7 + 17 * m)
}

/// SHA-256 over the concatenated 32-byte keys, in ring order.
pub fn ring_digest(list: &[u8]) -> [u8; 32] {
    sha256::digest(list)
}

/// RFC 9380 `hash_to_ristretto255`: expand_message_xmd with SHA-512 to 64
/// bytes, then the RFC 9496 one-way map.
pub fn hash_to_group(dst: &[u8], msg: &[u8]) -> Outcome<RistrettoPoint> {
    if dst.is_empty() || dst.len() > 255 {
        return Err(err!("hash_to_group: domain tag length {} is outside 1..=255.",
            dst.len(); Invalid, Input));
    }
    let dst_len = [dst.len() as u8];
    let mut h = Sha512::new();
    h.update([0u8; 128]);                   // Z_pad, the SHA-512 block size
    h.update(msg);
    h.update([0u8, 64u8]);                  // l_i_b_str, 64 bytes wanted
    h.update([0u8]);
    h.update(dst);
    h.update(dst_len);
    let b0 = h.finalize();
    let mut h = Sha512::new();
    h.update(b0);
    h.update([1u8]);
    h.update(dst);
    h.update(dst_len);
    let b1 = h.finalize();
    let mut uniform = [0u8; 64];
    uniform.copy_from_slice(&b1);
    Ok(RistrettoPoint::from_uniform_bytes(&uniform))
}

/// The tag base for a scope, `U = H_p(scope)`.
fn scope_base(scope: &[u8]) -> Outcome<RistrettoPoint> {
    hash_to_group(DST_SCOPE, scope)
}

/// The vector commitment generators `H_{j,i}`, at `j·16 + i`.
fn generators(m: usize) -> Outcome<Vec<RistrettoPoint>> {
    let mut gens = Vec::with_capacity(m * RADIX);
    for idx in 0..(m * RADIX) as u32 {
        gens.push(res!(hash_to_group(DST_GEN, &idx.to_le_bytes())));
    }
    Ok(gens)
}

fn wide(bytes: &[u8]) -> Scalar {
    let mut w = [0u8; 64];
    w.copy_from_slice(bytes);
    let s = Scalar::from_bytes_mod_order_wide(&w);
    w.zeroize();
    s
}

// ── Keys ────────────────────────────────────────────────────────────────────

/// A ring secret key, a non-zero ristretto255 scalar. Zeroed on drop.
#[derive(Clone)]
pub struct SecretKey(Scalar);

impl Drop for SecretKey {
    fn drop(&mut self) { self.0.zeroize(); }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretKey(..)")
    }
}

impl SecretKey {
    /// Derives a key from seed material as the scalar `SHA-512(DST ‖ seed)`
    /// reduced modulo the group order, so a key can be restored from the same
    /// seed on any device.
    pub fn from_seed(seed: &[u8]) -> Outcome<Self> {
        let mut h = Sha512::new();
        h.update(DST_SECRET);
        h.update(seed);
        let s = wide(&h.finalize());
        if s == Scalar::ZERO {
            return Err(err!("linkring: the seed reduces to the zero scalar."; Invalid, Input));
        }
        Ok(Self(s))
    }

    /// Reads a canonical, non-zero scalar.
    pub fn from_bytes(bytes: &[u8; 32]) -> Outcome<Self> {
        let s: Option<Scalar> = Scalar::from_canonical_bytes(*bytes).into();
        match s {
            Some(s) if s != Scalar::ZERO => Ok(Self(s)),
            _ => Err(err!("linkring: the secret key is not a canonical non-zero scalar.";
                Invalid, Input)),
        }
    }

    pub fn random() -> Outcome<Self> {
        let mut seed = [0u8; 64];
        OsRng.fill_bytes(&mut seed);
        let key = Self::from_seed(&seed);
        seed.zeroize();
        key
    }

    pub fn to_bytes(&self) -> [u8; 32] { self.0.to_bytes() }

    pub fn public_key(&self) -> [u8; KEY_LEN] {
        RistrettoPoint::mul_base(&self.0).compress().to_bytes()
    }
}

/// The tag of `key` under `scope`, `k·H_p(scope)`.
pub fn tag(key: &SecretKey, scope: &[u8]) -> Outcome<[u8; TAG_LEN]> {
    let u = res!(scope_base(scope));
    Ok((u * key.0).compress().to_bytes())
}

// ── Ring ────────────────────────────────────────────────────────────────────

/// A decoded ring with its digest, built once and reused for every signature
/// over it. Holds about 200 bytes per key.
pub struct Ring {
    keys:   Vec<[u8; KEY_LEN]>,
    points: Vec<RistrettoPoint>,
    digest: [u8; 32],
}

impl Ring {
    /// Decodes a ring list, the concatenation of 32-byte keys.
    pub fn from_list(list: &[u8]) -> Outcome<Self> {
        Self::from_list_par(list, 1)
    }

    /// As `from_list`, decoding on `threads` threads.
    pub fn from_list_par(list: &[u8], threads: usize) -> Outcome<Self> {
        if list.is_empty() || list.len() % KEY_LEN != 0 {
            return Err(err!("linkring: a ring list of {} bytes is not a non-empty \
                multiple of {}.", list.len(), KEY_LEN; Invalid, Input, Size));
        }
        let n = list.len() / KEY_LEN;
        if n > 1usize << 32 {
            return Err(err!("linkring: a ring of {} keys exceeds 2^32.", n; Invalid, Input, Size));
        }
        let mut keys = Vec::with_capacity(n);
        for c in list.chunks_exact(KEY_LEN) {
            let mut k = [0u8; KEY_LEN];
            k.copy_from_slice(c);
            keys.push(k);
        }
        let points = res!(decode_all(&keys, threads.max(1)));
        Ok(Self { keys, points, digest: ring_digest(list) })
    }

    pub fn from_keys(keys: &[[u8; KEY_LEN]]) -> Outcome<Self> {
        let mut list = Vec::with_capacity(keys.len() * KEY_LEN);
        for k in keys {
            list.extend_from_slice(k);
        }
        Self::from_list(&list)
    }

    pub fn len(&self) -> usize { self.keys.len() }
    pub fn is_empty(&self) -> bool { self.keys.is_empty() }
    pub fn digest(&self) -> &[u8; 32] { &self.digest }
    pub fn keys(&self) -> &[[u8; KEY_LEN]] { &self.keys }

    /// The first position of `key` in the ring.
    pub fn position(&self, key: &[u8; KEY_LEN]) -> Option<usize> {
        self.keys.iter().position(|k| k == key)
    }
}

fn decode_one(pos: usize, k: &[u8; KEY_LEN]) -> Outcome<RistrettoPoint> {
    match CompressedRistretto(*k).decompress() {
        Some(p) if !p.is_identity() => Ok(p),
        Some(_) => Err(err!("linkring: ring key {} is the identity, whose secret is zero.",
            pos; Invalid, Input)),
        None => Err(err!("linkring: ring key {} is not a ristretto255 encoding.",
            pos; Invalid, Input, Decode)),
    }
}

fn decode_all(keys: &[[u8; KEY_LEN]], threads: usize) -> Outcome<Vec<RistrettoPoint>> {
    if threads <= 1 || keys.len() < threads * 256 {
        let mut pts = Vec::with_capacity(keys.len());
        for (pos, k) in keys.iter().enumerate() {
            pts.push(res!(decode_one(pos, k)));
        }
        return Ok(pts);
    }
    let per = (keys.len() + threads - 1) / threads;
    let failed: Mutex<Option<Error<ErrTag>>> = Mutex::new(None);
    let mut parts: Vec<Vec<RistrettoPoint>> = Vec::new();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (t, part) in keys.chunks(per).enumerate() {
            let failed = &failed;
            handles.push(s.spawn(move || {
                let mut pts = Vec::with_capacity(part.len());
                for (i, k) in part.iter().enumerate() {
                    match decode_one(t * per + i, k) {
                        Ok(p) => pts.push(p),
                        Err(e) => {
                            if let Ok(mut f) = failed.lock() {
                                if f.is_none() { *f = Some(e); }
                            }
                            break;
                        }
                    }
                }
                pts
            }));
        }
        for h in handles {
            parts.push(h.join().unwrap_or_default());
        }
    });
    let failed = res!(failed.into_inner().map_err(|_| err!(
        "linkring: a ring decode thread poisoned its lock."; Thread, Poisoned)));
    if let Some(e) = failed {
        return Err(e);
    }
    let mut pts = Vec::with_capacity(keys.len());
    for p in parts {
        pts.extend(p);
    }
    if pts.len() != keys.len() {
        return Err(err!("linkring: {} of {} ring keys decoded; a decode thread died.",
            pts.len(), keys.len(); Thread));
    }
    Ok(pts)
}

// ── Guards ──────────────────────────────────────────────────────────────────

// Each check in `verify` has a bit here. The unit tests switch one off, on
// their own thread only, to prove the attack it stops then succeeds.
const G_MSG:        u32 = 1;    // message in the transcript
const G_DIGEST:     u32 = 2;    // ring digest in the transcript
const G_DIGITS_AB:  u32 = 4;    // A + x·B = Com(f; z_A)
const G_DIGITS_CD:  u32 = 8;    // x·C + D = Com(f(x − f); z_C)
const G_TAG:        u32 = 16;   // tag relation
const G_RING:       u32 = 32;   // ring relation

#[cfg(test)]
thread_local! {
    static SKIP: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn on(g: u32) -> bool { SKIP.with(|s| s.get() & g == 0) }

#[cfg(not(test))]
#[inline(always)]
fn on(_g: u32) -> bool { true }

// ── Transcript ──────────────────────────────────────────────────────────────

struct Commit {
    a:  RistrettoPoint,
    b:  RistrettoPoint,
    c:  RistrettoPoint,
    d:  RistrettoPoint,
    g:  Vec<RistrettoPoint>,    // G_k
    y:  Vec<RistrettoPoint>,    // Y_k
}

fn challenge(
    ring:   &Ring,
    m:      usize,
    scope:  &[u8],
    tau:    &[u8; TAG_LEN],
    msg:    &[u8],
    com:    &Commit,
)
    -> Scalar
{
    let mut h = Sha512::new();
    h.update(ALG.as_bytes());
    h.update((RADIX as u32).to_le_bytes());
    h.update((m as u32).to_le_bytes());
    h.update((ring.len() as u64).to_le_bytes());
    if on(G_DIGEST) {
        h.update(ring.digest);
    }
    h.update((scope.len() as u64).to_le_bytes());
    h.update(scope);
    h.update(tau);
    if on(G_MSG) {
        h.update((msg.len() as u64).to_le_bytes());
        h.update(msg);
    }
    for p in [&com.a, &com.b, &com.c, &com.d].into_iter().chain(com.g.iter()).chain(com.y.iter()) {
        h.update(p.compress().as_bytes());
    }
    wide(&h.finalize())
}

// ── Sign ────────────────────────────────────────────────────────────────────

/// Signs `msg` under `scope` as a hidden member of `ring`, returning the tag
/// and the body. The key must be in the ring.
pub fn sign(
    ring:   &Ring,
    key:    &SecretKey,
    scope:  &[u8],
    msg:    &[u8],
)
    -> Outcome<([u8; TAG_LEN], Vec<u8>)>
{
    let mut aux = [0u8; 32];
    OsRng.fill_bytes(&mut aux);
    sign_with_aux(ring, key, scope, msg, &aux)
}

/// As `sign`, with the prover's randomness derived from the key, the inputs
/// and `aux` alone, so equal inputs give an equal body.  `aux` hedges against
/// a fault; empty is safe, since the nonces still depend on everything signed.
pub fn sign_with_aux(
    ring:   &Ring,
    key:    &SecretKey,
    scope:  &[u8],
    msg:    &[u8],
    aux:    &[u8],
)
    -> Outcome<([u8; TAG_LEN], Vec<u8>)>
{
    let pk = key.public_key();
    let l = match ring.position(&pk) {
        Some(l) => l,
        None => return Err(err!("linkring: the signing key is not in the ring of {} keys.",
            ring.len(); Missing, Input)),
    };
    let u = res!(scope_base(scope));
    let tau = u * key.0;
    let body = res!(prove(ring, l, &key.0, &tau, &u, scope, msg, aux));
    Ok((tau.compress().to_bytes(), body))
}

struct Nonces {
    seed:   [u8; 64],
    ctr:    u32,
}

impl Nonces {
    fn next(&mut self) -> Scalar {
        let mut h = Sha512::new();
        h.update(self.seed);
        h.update(self.ctr.to_le_bytes());
        self.ctr += 1;
        wide(&h.finalize())
    }
}

impl Drop for Nonces {
    fn drop(&mut self) { self.seed.zeroize(); }
}

#[allow(clippy::too_many_arguments)]
fn prove(
    ring:   &Ring,
    l:      usize,
    k:      &Scalar,
    tau:    &RistrettoPoint,
    u:      &RistrettoPoint,
    scope:  &[u8],
    msg:    &[u8],
    aux:    &[u8],
)
    -> Outcome<Vec<u8>>
{
    let n_ring = ring.len();
    let m = digits(n_ring);
    if m > MAX_DIGITS {
        return Err(err!("linkring: a ring of {} keys needs {} digits, over {}.",
            n_ring, m, MAX_DIGITS; Invalid, Input, Size));
    }
    let g = RISTRETTO_BASEPOINT_POINT;
    let gens = res!(generators(m));
    let tau_bytes = tau.compress().to_bytes();

    let mut nonces = {
        let mut h = Sha512::new();
        h.update(DST_NONCE);
        h.update(k.as_bytes());
        h.update(ring.digest);
        h.update((n_ring as u64).to_le_bytes());
        h.update((l as u64).to_le_bytes());
        h.update((scope.len() as u64).to_le_bytes());
        h.update(scope);
        h.update((msg.len() as u64).to_le_bytes());
        h.update(msg);
        h.update((aux.len() as u64).to_le_bytes());
        h.update(aux);
        let mut seed = [0u8; 64];
        seed.copy_from_slice(&h.finalize());
        Nonces { seed, ctr: 0 }
    };

    // Signer's digits, least significant first.
    let ld: Vec<usize> = (0..m).map(|j| (l >> (4 * j)) & (RADIX - 1)).collect();

    // a_{j,i}: random for i ≥ 1, a_{j,0} makes each row sum to zero.
    let mut a = vec![Scalar::ZERO; m * RADIX];
    for j in 0..m {
        let mut sum = Scalar::ZERO;
        for i in 1..RADIX {
            let s = nonces.next();
            a[j * RADIX + i] = s;
            sum += s;
        }
        a[j * RADIX] = -sum;
    }
    let r_a = nonces.next();
    let r_b = nonces.next();
    let r_c = nonces.next();
    let r_d = nonces.next();
    let rho: Vec<Scalar> = (0..m).map(|_| nonces.next()).collect();

    // Digit commitments, constant time since they hold secrets.
    let sigma = |j: usize, i: usize| if ld[j] == i { Scalar::ONE } else { Scalar::ZERO };
    let mut s_a = Vec::with_capacity(m * RADIX + 1);
    let mut s_b = Vec::with_capacity(m * RADIX + 1);
    let mut s_c = Vec::with_capacity(m * RADIX + 1);
    let mut s_d = Vec::with_capacity(m * RADIX + 1);
    s_a.push(r_a);
    s_b.push(r_b);
    s_c.push(r_c);
    s_d.push(r_d);
    for j in 0..m {
        for i in 0..RADIX {
            let aji = a[j * RADIX + i];
            let sji = sigma(j, i);
            s_a.push(aji);
            s_b.push(sji);
            s_c.push(aji * (Scalar::ONE - sji - sji));
            s_d.push(-(aji * aji));
        }
    }
    let bases: Vec<RistrettoPoint> = std::iter::once(g).chain(gens.iter().copied()).collect();
    let com_a = RistrettoPoint::multiscalar_mul(&s_a, &bases);
    let com_b = RistrettoPoint::multiscalar_mul(&s_b, &bases);
    let com_c = RistrettoPoint::multiscalar_mul(&s_c, &bases);
    let com_d = RistrettoPoint::multiscalar_mul(&s_d, &bases);
    for v in [&mut s_a, &mut s_b, &mut s_c, &mut s_d] {
        v.zeroize();
    }

    // G_k = Σ_i p_{i,k}·P_i, gathered by the subsets T of digit positions that
    // contribute a constant; positions outside T are fixed to the signer's.
    let mut gk = vec![RistrettoPoint::identity(); m];
    let mut coef_sum = vec![Scalar::ZERO; m];
    let mut idx_buf: Vec<u32> = Vec::new();
    let mut sc_buf: Vec<Scalar> = Vec::new();
    for mask in 1usize..(1 << m) {
        let pos: Vec<usize> = (0..m).filter(|j| mask & (1 << j) != 0).collect();
        let t = pos.len();
        let deg = m - t;
        let mut base = 0usize;
        for j in 0..m {
            if mask & (1 << j) == 0 {
                base += ld[j] << (4 * j);
            }
        }
        // Mixed-radix counter over T, least significant position fastest, so
        // the index only rises and the enumeration stops at the ring's end.
        let mut d = vec![0usize; t];
        let mut prod = vec![Scalar::ONE; t + 1];   // prod[q] = Π_{q' ≥ q} a
        for q in (0..t).rev() {
            prod[q] = prod[q + 1] * a[pos[q] * RADIX];
        }
        idx_buf.clear();
        sc_buf.clear();
        loop {
            let mut idx = base;
            for q in 0..t {
                idx += d[q] << (4 * pos[q]);
            }
            if idx >= n_ring {
                break;
            }
            idx_buf.push(idx as u32);
            sc_buf.push(prod[0]);
            coef_sum[deg] += prod[0];
            if idx_buf.len() == CHUNK {
                gk[deg] += msm_indexed(&sc_buf, &idx_buf, &ring.points);
                idx_buf.clear();
                sc_buf.clear();
            }
            // Increment.
            let mut q = 0;
            while q < t {
                d[q] += 1;
                if d[q] < RADIX {
                    break;
                }
                d[q] = 0;
                q += 1;
            }
            if q == t {
                break;
            }
            for q2 in (0..=q).rev() {
                prod[q2] = prod[q2 + 1] * a[pos[q2] * RADIX + d[q2]];
            }
        }
        if !idx_buf.is_empty() {
            gk[deg] += msm_indexed(&sc_buf, &idx_buf, &ring.points);
        }
    }
    sc_buf.zeroize();
    // Padded entries repeat the last key; since every coefficient below x^m
    // sums to zero over all 16^m entries, theirs is minus the sum seen.
    let last = ring.points[n_ring - 1];
    let mut ym = Vec::with_capacity(m);
    for kk in 0..m {
        gk[kk] += RistrettoPoint::multiscalar_mul(
            [-coef_sum[kk], rho[kk]],
            [last, g],
        );
        ym.push(u * rho[kk]);
    }
    coef_sum.zeroize();

    let com = Commit { a: com_a, b: com_b, c: com_c, d: com_d, g: gk, y: ym };
    let x = challenge(ring, m, scope, &tau_bytes, msg, &com);
    if x == Scalar::ZERO {
        return Err(err!("linkring: the challenge is zero."; Invalid));
    }

    // Responses.
    let mut body = Vec::with_capacity(body_len(m));
    body.push(m as u8);
    for p in [&com.a, &com.b, &com.c, &com.d].into_iter().chain(com.g.iter()).chain(com.y.iter()) {
        body.extend_from_slice(p.compress().as_bytes());
    }
    for j in 0..m {
        for i in 1..RADIX {
            let f = sigma(j, i) * x + a[j * RADIX + i];
            body.extend_from_slice(f.as_bytes());
        }
    }
    let z_a = r_b * x + r_a;
    let z_c = r_c * x + r_d;
    let mut xp = Scalar::ONE;
    let mut z = Scalar::ZERO;
    for kk in 0..m {
        z -= rho[kk] * xp;
        xp *= x;
    }
    z += k * xp;
    body.extend_from_slice(z_a.as_bytes());
    body.extend_from_slice(z_c.as_bytes());
    body.extend_from_slice(z.as_bytes());
    a.zeroize();
    Ok(body)
}

fn msm_indexed(scalars: &[Scalar], idx: &[u32], points: &[RistrettoPoint]) -> RistrettoPoint {
    RistrettoPoint::vartime_multiscalar_mul(
        scalars.iter(),
        idx.iter().map(|&i| &points[i as usize]),
    )
}

// ── Verify ──────────────────────────────────────────────────────────────────

/// Does `body` prove that a member of `ring` signed `msg` under `scope` with
/// tag `tau`?
pub fn verify(
    ring:   &Ring,
    scope:  &[u8],
    msg:    &[u8],
    tau:    &[u8],
    body:   &[u8],
)
    -> Outcome<bool>
{
    verify_par(ring, scope, msg, tau, body, 1)
}

/// As `verify`, spreading the ring sum over `threads` threads.  Use 1 on
/// wasm32, which has no threads.
pub fn verify_par(
    ring:       &Ring,
    scope:      &[u8],
    msg:        &[u8],
    tau:        &[u8],
    body:       &[u8],
    threads:    usize,
)
    -> Outcome<bool>
{
    let m = digits(ring.len());
    if m > MAX_DIGITS || body.len() != body_len(m) || body[0] as usize != m {
        return Ok(false);
    }
    if tau.len() != TAG_LEN {
        return Ok(false);
    }
    let mut tau_bytes = [0u8; TAG_LEN];
    tau_bytes.copy_from_slice(tau);
    let tau_pt = match CompressedRistretto(tau_bytes).decompress() {
        Some(p) if !p.is_identity() => p,
        _ => return Ok(false),
    };

    // Parse.
    let mut off = 1;
    let mut pts = Vec::with_capacity(4 + 2 * m);
    for _ in 0..(4 + 2 * m) {
        match CompressedRistretto::from_slice(&body[off..off + 32]).ok().and_then(|c| c.decompress()) {
            Some(p) => pts.push(p),
            None => return Ok(false),
        }
        off += 32;
    }
    let mut scs = Vec::with_capacity(m * (RADIX - 1) + 3);
    for _ in 0..(m * (RADIX - 1) + 3) {
        let mut b = [0u8; 32];
        b.copy_from_slice(&body[off..off + 32]);
        let s: Option<Scalar> = Scalar::from_canonical_bytes(b).into();
        match s {
            Some(s) => scs.push(s),
            None => return Ok(false),
        }
        off += 32;
    }
    let com = Commit {
        a: pts[0],
        b: pts[1],
        c: pts[2],
        d: pts[3],
        g: pts[4..4 + m].to_vec(),
        y: pts[4 + m..4 + 2 * m].to_vec(),
    };
    let z_a = scs[m * (RADIX - 1)];
    let z_c = scs[m * (RADIX - 1) + 1];
    let z = scs[m * (RADIX - 1) + 2];

    let x = challenge(ring, m, scope, &tau_bytes, msg, &com);
    if x == Scalar::ZERO {
        return Ok(false);
    }

    // Full response matrix, f_{j,0} = x − Σ_{i≥1} f_{j,i}.
    let mut f = vec![Scalar::ZERO; m * RADIX];
    for j in 0..m {
        let mut sum = Scalar::ZERO;
        for i in 1..RADIX {
            let v = scs[j * (RADIX - 1) + i - 1];
            f[j * RADIX + i] = v;
            sum += v;
        }
        f[j * RADIX] = x - sum;
    }

    // Digit checks: A + x·B = Com(f; z_A) and x·C + D = Com(f(x − f); z_C).
    let g = RISTRETTO_BASEPOINT_POINT;
    let gens = res!(generators(m));
    let mut s1 = Vec::with_capacity(m * RADIX + 3);
    let mut s2 = Vec::with_capacity(m * RADIX + 3);
    s1.push(z_a);
    s2.push(z_c);
    for v in &f {
        s1.push(*v);
        s2.push(v * (x - v));
    }
    s1.push(-Scalar::ONE);
    s1.push(-x);
    s2.push(-x);
    s2.push(-Scalar::ONE);
    let p1 = std::iter::once(&g).chain(gens.iter()).chain([&com.a, &com.b]);
    let p2 = std::iter::once(&g).chain(gens.iter()).chain([&com.c, &com.d]);
    if on(G_DIGITS_AB) && !RistrettoPoint::vartime_multiscalar_mul(&s1, p1).is_identity() {
        return Ok(false);
    }
    if on(G_DIGITS_CD) && !RistrettoPoint::vartime_multiscalar_mul(&s2, p2).is_identity() {
        return Ok(false);
    }

    // Tag check: x^m·τ − Σ x^k·Y_k − z·U = 0.
    let u = res!(scope_base(scope));
    let mut xpow = Vec::with_capacity(m + 1);
    let mut xp = Scalar::ONE;
    for _ in 0..=m {
        xpow.push(xp);
        xp *= x;
    }
    let mut st = Vec::with_capacity(m + 2);
    st.push(xpow[m]);
    for kk in 0..m {
        st.push(-xpow[kk]);
    }
    st.push(-z);
    let pt = std::iter::once(&tau_pt).chain(com.y.iter()).chain(std::iter::once(&u));
    if on(G_TAG) && !RistrettoPoint::vartime_multiscalar_mul(&st, pt).is_identity() {
        return Ok(false);
    }

    // Ring check: Σ p_i(x)·P_i + pad·P_{N−1} − Σ x^k·G_k − z·G = 0.
    if !on(G_RING) {
        return Ok(true);
    }
    let (sum_pt, sum_p) = res!(ring_sum(ring, &f, m, threads.max(1)));
    let mut sr = Vec::with_capacity(m + 2);
    sr.push(xpow[m] - sum_p);
    for kk in 0..m {
        sr.push(-xpow[kk]);
    }
    sr.push(-z);
    let last = &ring.points[ring.len() - 1];
    let pr = std::iter::once(last).chain(com.g.iter()).chain(std::iter::once(&g));
    Ok((sum_pt + RistrettoPoint::vartime_multiscalar_mul(&sr, pr)).is_identity())
}

/// `Σ_{i<N} p_i(x)·P_i` and `Σ_{i<N} p_i(x)`, where `p_i(x) = Π_j f_{j,i_j}`.
fn ring_sum(
    ring:       &Ring,
    f:          &[Scalar],
    m:          usize,
    threads:    usize,
)
    -> Outcome<(RistrettoPoint, Scalar)>
{
    let n = ring.len();
    if threads <= 1 || n < threads * 256 {
        return Ok(ring_sum_range(ring, f, m, 0, n));
    }
    let per = ((n + threads - 1) / threads).max(1);
    let mut out = Vec::new();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        let mut lo = 0;
        while lo < n {
            let hi = (lo + per).min(n);
            handles.push(s.spawn(move || ring_sum_range(ring, f, m, lo, hi)));
            lo = hi;
        }
        for h in handles {
            out.push(h.join());
        }
    });
    let mut pt = RistrettoPoint::identity();
    let mut sc = Scalar::ZERO;
    for r in out {
        match r {
            Ok((p, s)) => {
                pt += p;
                sc += s;
            }
            Err(_) => return Err(err!("linkring: a ring sum thread panicked."; Thread)),
        }
    }
    Ok((pt, sc))
}

fn ring_sum_range(
    ring:   &Ring,
    f:      &[Scalar],
    m:      usize,
    lo:     usize,
    hi:     usize,
)
    -> (RistrettoPoint, Scalar)
{
    let mut d: Vec<usize> = (0..m).map(|j| (lo >> (4 * j)) & (RADIX - 1)).collect();
    let mut prod = vec![Scalar::ONE; m + 1];    // prod[j] = Π_{j' ≥ j} f_{j', d_j'}
    for j in (0..m).rev() {
        prod[j] = prod[j + 1] * f[j * RADIX + d[j]];
    }
    let mut pt = RistrettoPoint::identity();
    let mut total = Scalar::ZERO;
    let mut buf = Vec::with_capacity(CHUNK.min(hi - lo));
    let mut start = lo;
    for i in lo..hi {
        buf.push(prod[0]);
        total += prod[0];
        if buf.len() == CHUNK {
            pt += RistrettoPoint::vartime_multiscalar_mul(buf.iter(), ring.points[start..=i].iter());
            buf.clear();
            start = i + 1;
        }
        // Next index.
        let mut j = 0;
        while j < m {
            d[j] += 1;
            if d[j] < RADIX {
                break;
            }
            d[j] = 0;
            j += 1;
        }
        let top = j.min(m - 1);
        for j2 in (0..=top).rev() {
            prod[j2] = prod[j2 + 1] * f[j2 * RADIX + d[j2]];
        }
    }
    if !buf.is_empty() {
        pt += RistrettoPoint::vartime_multiscalar_mul(buf.iter(), ring.points[start..hi].iter());
    }
    (pt, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_of(n: usize) -> Outcome<(Vec<SecretKey>, Ring)> {
        let mut keys = Vec::with_capacity(n);
        for i in 0..n {
            keys.push(res!(SecretKey::from_seed(fmt!("k{}", i).as_bytes())));
        }
        let pubs: Vec<[u8; 32]> = keys.iter().map(|k| k.public_key()).collect();
        let ring = res!(Ring::from_keys(&pubs));
        Ok((keys, ring))
    }

    fn with_skip<T>(g: u32, f: impl FnOnce() -> T) -> T {
        SKIP.with(|s| s.set(g));
        let out = f();
        SKIP.with(|s| s.set(0));
        out
    }

    fn scalar_at(body: &[u8], off: usize) -> Scalar {
        let mut b = [0u8; 32];
        b.copy_from_slice(&body[off..off + 32]);
        Scalar::from_bytes_mod_order(b)
    }

    fn put_scalar(body: &mut [u8], off: usize, s: Scalar) {
        body[off..off + 32].copy_from_slice(s.as_bytes());
    }

    // Offsets of z_A, z_C and z in a body of m digits.
    fn z_offsets(m: usize) -> (usize, usize, usize) {
        let base = 1 + 32 * (4 + 2 * m + 15 * m);
        (base, base + 32, base + 64)
    }

    // Guard G_MSG: the message is bound only by the transcript.
    #[test]
    fn test_guard_message() -> Outcome<()> {
        let (keys, ring) = res!(ring_of(5));
        let (t, b) = res!(with_skip(G_MSG, || sign_with_aux(&ring, &keys[2], b"s", b"m1", b"")));
        req!(res!(with_skip(G_MSG, || verify(&ring, b"s", b"m2", &t, &b))), true, "guard off");
        let (t, b) = res!(sign_with_aux(&ring, &keys[2], b"s", b"m1", b""));
        req!(res!(verify(&ring, b"s", b"m2", &t, &b)), false, "guard on");
        Ok(())
    }

    // Guards G_DIGITS_AB and G_DIGITS_CD: z_A and z_C sit in no other check.
    #[test]
    fn test_guard_digit_checks() -> Outcome<()> {
        let (keys, ring) = res!(ring_of(5));
        let (t, b) = res!(sign_with_aux(&ring, &keys[1], b"s", b"m", b""));
        let (oa, oc, _) = z_offsets(1);
        for (g, off) in [(G_DIGITS_AB, oa), (G_DIGITS_CD, oc)] {
            let mut bad = b.clone();
            put_scalar(&mut bad, off, scalar_at(&b, off) + Scalar::ONE);
            req!(res!(with_skip(g, || verify(&ring, b"s", b"m", &t, &bad))), true, "guard {} off", g);
            req!(res!(verify(&ring, b"s", b"m", &t, &bad)), false, "guard {} on", g);
        }
        Ok(())
    }

    // Guard G_TAG, and the no-frame property: a member cannot carry another
    // member's tag, even one it has seen, because the tag relation extracts
    // the signer's own key.
    #[test]
    fn test_guard_tag_no_frame() -> Outcome<()> {
        let (keys, ring) = res!(ring_of(20));
        let scope = b"present/1:https://app.example";
        let msg = b"m";
        let u = res!(scope_base(scope));
        let tau_b = u * keys[7].0;
        let tb = tau_b.compress().to_bytes();
        let body = res!(prove(&ring, 3, &keys[3].0, &tau_b, &u, scope, msg, b""));
        req!(res!(with_skip(G_TAG, || verify(&ring, scope, msg, &tb, &body))), true, "guard off");
        req!(res!(verify(&ring, scope, msg, &tb, &body)), false, "guard on");
        // The tag's own key proves it.
        let body = res!(prove(&ring, 7, &keys[7].0, &tau_b, &u, scope, msg, b""));
        req!(res!(verify(&ring, scope, msg, &tb, &body)), true, "honest");
        Ok(())
    }

    // Guard G_RING: without the ring relation anyone signs with no key at
    // all, by proving a made-up secret against the tag base alone.
    #[test]
    fn test_guard_ring_keyless_forgery() -> Outcome<()> {
        let (_, ring) = res!(ring_of(20));
        let fake = res!(SecretKey::from_seed(b"not in the ring"));
        let u = res!(scope_base(b"s"));
        let tau = u * fake.0;
        let tb = tau.compress().to_bytes();
        let body = res!(prove(&ring, 4, &fake.0, &tau, &u, b"s", b"m", b""));
        req!(res!(with_skip(G_RING, || verify(&ring, b"s", b"m", &tb, &body))), true, "guard off");
        req!(res!(verify(&ring, b"s", b"m", &tb, &body)), false, "guard on");
        Ok(())
    }

    // Guard G_DIGEST: without the ring in the transcript a forger fixes the
    // proof first and then solves for a ring key that satisfies it.
    #[test]
    fn test_guard_digest_ring_after_challenge() -> Outcome<()> {
        let (keys, _) = res!(ring_of(2));
        let p0 = keys[0].public_key();
        let q = res!(SecretKey::from_seed(b"placeholder")).public_key();
        let fake = res!(SecretKey::from_seed(b"forger"));
        let u = res!(scope_base(b"s"));
        let tau = u * fake.0;
        let tb = tau.compress().to_bytes();
        let forge = |guard_off: bool| -> Outcome<bool> {
            let skip = if guard_off { G_DIGEST } else { 0 };
            with_skip(skip, || {
                let placeholder = res!(Ring::from_keys(&[p0, q]));
                let body = res!(prove(&placeholder, 1, &fake.0, &tau, &u, b"s", b"m", b""));
                // Recompute x as the verifier will, then solve for P_1 in
                // f_0·P_0 + (Σ_{i≥1} f_i)·P_1 − G_0 = z·G.
                let mut pts = Vec::new();
                for i in 0..6 {
                    let c = res!(CompressedRistretto::from_slice(&body[1 + 32 * i..33 + 32 * i])
                        .map_err(|_| err!("slice"; Test)));
                    pts.push(res!(c.decompress().ok_or_else(|| err!("point"; Test))));
                }
                let com = Commit { a: pts[0], b: pts[1], c: pts[2], d: pts[3],
                    g: vec![pts[4]], y: vec![pts[5]] };
                let x = challenge(&placeholder, 1, b"s", &tb, b"m", &com);
                let mut rest = Scalar::ZERO;
                for i in 0..15 {
                    rest += scalar_at(&body, 1 + 32 * 6 + 32 * i);
                }
                let f0 = x - rest;
                let (_, _, oz) = z_offsets(1);
                let z = scalar_at(&body, oz);
                let p0_pt = placeholder.points[0];
                let p1 = (RISTRETTO_BASEPOINT_POINT * z + com.g[0] - p0_pt * f0) * rest.invert();
                let ring = res!(Ring::from_keys(&[p0, p1.compress().to_bytes()]));
                verify(&ring, b"s", b"m", &tb, &body)
            })
        };
        req!(res!(forge(true)), true, "guard off");
        req!(res!(forge(false)), false, "guard on");
        Ok(())
    }

    // The prover's subset expansion must equal the plain coefficient sum, at
    // every index and every ring size across a digit boundary.
    #[test]
    fn test_sizes_across_digit_boundaries() -> Outcome<()> {
        for n in [1usize, 2, 15, 16, 17, 255, 256, 257, 300] {
            let (keys, ring) = res!(ring_of(n));
            for l in [0, n / 2, n - 1] {
                let (t, b) = res!(sign_with_aux(&ring, &keys[l], b"s", b"m", b""));
                req!(b.len(), body_len(digits(n)));
                req!(res!(verify(&ring, b"s", b"m", &t, &b)), true, "n={} l={}", n, l);
            }
        }
        Ok(())
    }
}
