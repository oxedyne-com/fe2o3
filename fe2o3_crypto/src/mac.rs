//! HMAC-SHA256 (RFC 2104) and HKDF-SHA256 (RFC 5869), in pure Rust.
//!
//! Both rest on [`oxedyne_fe2o3_hash::sha256`], so nothing here asks for a C toolchain or the
//! operating system and the module compiles to wasm32 with the rest of the pure-Rust subset.
//! That is the reason it exists beside `fe2o3_net::hmac`, which wraps `ring` and is native only:
//! a key derived in a browser by WebCrypto's HKDF and the same key derived by a native process
//! must be the same bytes, and only a derivation both sides can run gives that.
//!
//! The tag comparison in [`verify_hmac_sha256`] runs over every byte whatever it finds, so the
//! time it takes says nothing about how much of a forged tag was right.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::sha256::{
    self,
    Sha256,
};

/// The SHA-256 block, which is the width a key is padded or hashed to.
const BLOCK: usize = 64;

/// The width of every tag and of every pseudorandom key here.
pub const HMAC_SHA256_LEN: usize = sha256::DIGEST_LEN;

/// The most HKDF-SHA256 can expand to: 255 blocks of output (RFC 5869 §2.3).
pub const HKDF_SHA256_MAX: usize = 255 * HMAC_SHA256_LEN;

/// HMAC-SHA256 of `msg` under `key`. A key of any length is legal: one longer than a block is
/// hashed first, as RFC 2104 says.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; HMAC_SHA256_LEN] {
    hmac_sha256_parts(key, &[msg])
}

/// HMAC-SHA256 over the concatenation of `parts`, without making the concatenation.
pub fn hmac_sha256_parts(key: &[u8], parts: &[&[u8]]) -> [u8; HMAC_SHA256_LEN] {
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let d = sha256::digest(key);
        k[..d.len()].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    for p in parts {
        inner.update(p);
    }
    let ih = inner.finish();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&ih);
    let tag = outer.finish();
    // The padded key is the key: it is not left lying in a stack slot for the next caller.
    for b in k.iter_mut().chain(ipad.iter_mut()).chain(opad.iter_mut()) {
        *b = 0;
    }
    tag
}

/// Whether `tag` is the HMAC-SHA256 of `msg` under `key`, compared in constant time.
pub fn verify_hmac_sha256(key: &[u8], msg: &[u8], tag: &[u8]) -> bool {
    let want = hmac_sha256(key, msg);
    if tag.len() != want.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..want.len() {
        diff |= want[i] ^ tag[i];
    }
    diff == 0
}

/// HKDF-Extract: the pseudorandom key from input keying material and a salt. An empty salt is
/// a block of zeros, as RFC 5869 §2.2 says, which HMAC's padding makes the same thing.
pub fn hkdf_sha256_extract(salt: &[u8], ikm: &[u8]) -> [u8; HMAC_SHA256_LEN] {
    hmac_sha256(salt, ikm)
}

/// HKDF-Expand: `out.len()` bytes of output keying material from a pseudorandom key and an
/// `info` string. Refuses more than [`HKDF_SHA256_MAX`] bytes, which the counter cannot name.
pub fn hkdf_sha256_expand(prk: &[u8], info: &[u8], out: &mut [u8]) -> Outcome<()> {
    if out.len() > HKDF_SHA256_MAX {
        return Err(err!(
            "HKDF-SHA256 was asked for {} bytes, more than the {} its one-byte counter can name.",
            out.len(), HKDF_SHA256_MAX;
        Invalid, Input, TooBig));
    }
    let mut prev: [u8; HMAC_SHA256_LEN] = [0u8; HMAC_SHA256_LEN];
    let mut have_prev = false;
    let mut at = 0usize;
    let mut counter = 1u8;
    while at < out.len() {
        let t = if have_prev {
            hmac_sha256_parts(prk, &[&prev[..], info, &[counter]])
        } else {
            hmac_sha256_parts(prk, &[info, &[counter]])
        };
        let take = std::cmp::min(HMAC_SHA256_LEN, out.len() - at);
        out[at..at + take].copy_from_slice(&t[..take]);
        at += take;
        prev = t;
        have_prev = true;
        counter = counter.wrapping_add(1);
    }
    for b in prev.iter_mut() {
        *b = 0;
    }
    Ok(())
}

/// HKDF-SHA256, extract then expand, into `out`. This is what WebCrypto's
/// `deriveKey({ name: 'HKDF', hash: 'SHA-256', salt, info }, ..)` computes for a key of
/// `out.len()` bytes.
pub fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8], out: &mut [u8]) -> Outcome<()> {
    let mut prk = hkdf_sha256_extract(salt, ikm);
    let result = hkdf_sha256_expand(&prk, info, out);
    for b in prk.iter_mut() {
        *b = 0;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len() / 2);
        let mut i = 0;
        while i + 1 < b.len() {
            let hi = (b[i] as char).to_digit(16).unwrap_or(0) as u8;
            let lo = (b[i + 1] as char).to_digit(16).unwrap_or(0) as u8;
            out.push((hi << 4) | lo);
            i += 2;
        }
        out
    }

    // RFC 4231 test cases 1, 2, 6 (a key longer than a block) and 7.
    #[test]
    fn test_hmac_sha256_rfc4231() -> Outcome<()> {
        let cases: [(&str, &[u8], &str); 4] = [
            ("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b", b"Hi There",
             "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"),
            ("4a656665", b"what do ya want for nothing?",
             "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"),
            (&"aa".repeat(131), b"Test Using Larger Than Block-Size Key - Hash Key First",
             "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"),
            (&"aa".repeat(131),
             b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.",
             "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2"),
        ];
        for (k, m, t) in cases.iter() {
            let tag = hmac_sha256(&unhex(k), m);
            req!(tag.to_vec(), unhex(t));
            req!(verify_hmac_sha256(&unhex(k), m, &unhex(t)), true);
            let mut bad = unhex(t);
            bad[31] ^= 1;
            req!(verify_hmac_sha256(&unhex(k), m, &bad), false);
        }
        Ok(())
    }

    // RFC 5869 test cases 1, 2 and 3 (empty salt and info).
    #[test]
    fn test_hkdf_sha256_rfc5869() -> Outcome<()> {
        let ikm1 = unhex("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
        let mut okm = [0u8; 42];
        res!(hkdf_sha256(&ikm1, &unhex("000102030405060708090a0b0c"),
            &unhex("f0f1f2f3f4f5f6f7f8f9"), &mut okm));
        req!(okm.to_vec(), unhex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"));

        let ikm2: Vec<u8> = (0x00u8..=0x4f).collect();
        let salt2: Vec<u8> = (0x60u8..=0xaf).collect();
        let info2: Vec<u8> = (0xb0u8..=0xff).collect();
        let mut okm2 = [0u8; 82];
        res!(hkdf_sha256(&ikm2, &salt2, &info2, &mut okm2));
        req!(okm2.to_vec(), unhex(concat!(
            "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c",
            "59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71",
            "cc30c58179ec3e87c14c01d5c1f3434f1d87")));

        let mut okm3 = [0u8; 42];
        res!(hkdf_sha256(&ikm1, &[], &[], &mut okm3));
        req!(okm3.to_vec(), unhex("8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"));
        Ok(())
    }

    #[test]
    fn test_hkdf_sha256_refuses_too_long() -> Outcome<()> {
        let mut big = vec![0u8; HKDF_SHA256_MAX + 1];
        req!(hkdf_sha256(b"k", b"s", b"i", &mut big).is_err(), true);
        let mut max = vec![0u8; HKDF_SHA256_MAX];
        res!(hkdf_sha256(b"k", b"s", b"i", &mut max));
        Ok(())
    }
}
