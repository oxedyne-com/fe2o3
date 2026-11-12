//! The pure-Rust MACs and key derivations against published vectors: HMAC-SHA256 (RFC 4231),
//! HKDF-SHA256 (RFC 5869) and PBKDF2-HMAC-SHA256 (RFC 7914 §11 and the widely used RFC
//! 6070-style set for SHA-256). The 16,777,216-round vector is ignored by default; run it with
//! `cargo test --release -p oxedyne_fe2o3_crypto --test mac -- --ignored`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::mac::{
    hkdf_sha256,
    hmac_sha256,
    pbkdf2_hmac_sha256,
};


fn unhex(s: &str) -> Outcome<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        out.push(res!(u8::from_str_radix(&s[i..i + 2], 16), Decode));
    }
    Ok(out)
}

fn check(pass: &[u8], salt: &[u8], rounds: u32, want: &str) -> Outcome<()> {
    let want = res!(unhex(want));
    let mut got = vec![0u8; want.len()];
    res!(pbkdf2_hmac_sha256(pass, salt, rounds, &mut got));
    if got != want {
        return Err(err!(
            "PBKDF2-HMAC-SHA256({:?}, {:?}, {}) gave {:02x?}, expected {:02x?}.",
            String::from_utf8_lossy(pass), String::from_utf8_lossy(salt), rounds, got, want;
        Test, Mismatch));
    }
    Ok(())
}

/// RFC 4231 cases 2 and 6, the second with a key longer than a block.
#[test]
fn test_hmac_sha256_rfc4231_00() -> Outcome<()> {
    req!(hmac_sha256(b"Jefe", b"what do ya want for nothing?").to_vec(), res!(unhex(
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")));
    req!(hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")
        .to_vec(), res!(unhex(
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54")));
    Ok(())
}

/// RFC 5869 test case 1.
#[test]
fn test_hkdf_sha256_rfc5869_00() -> Outcome<()> {
    let salt: Vec<u8> = (0x00..=0x0c).collect();
    let info: Vec<u8> = (0xf0..=0xf9).collect();
    let mut okm = [0u8; 42];
    res!(hkdf_sha256(&[0x0b; 22], &salt, &info, &mut okm));
    req!(okm.to_vec(), res!(unhex(
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")));
    Ok(())
}

#[test]
fn test_pbkdf2_rfc7914_one_round_two_blocks_00() -> Outcome<()> {
    check(b"passwd", b"salt", 1,
        "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc
         49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783")
}

#[test]
fn test_pbkdf2_rfc7914_eighty_thousand_rounds_00() -> Outcome<()> {
    check(b"Password", b"NaCl", 80_000,
        "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56
         a1d425a1225833549adb841b51c9b3176a272bdebba1d078478f62b397f33c8d")
}

#[test]
fn test_pbkdf2_sha256_counts_00() -> Outcome<()> {
    res!(check(b"password", b"salt", 1,
        "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"));
    res!(check(b"password", b"salt", 2,
        "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"));
    check(b"password", b"salt", 4_096,
        "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a")
}

/// Output that ends part way through a block, a password longer than the 32-byte digest, and
/// embedded zero bytes.
#[test]
fn test_pbkdf2_sha256_partial_block_and_zero_bytes_00() -> Outcome<()> {
    res!(check(b"passwordPASSWORDpassword", b"saltSALTsaltSALTsaltSALTsaltSALTsalt", 4_096,
        "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1c635518c7dac47e9"));
    check(b"pass\0word", b"sa\0lt", 4_096, "89b69d0516f829893c696226650a8687")
}

#[test]
#[ignore = "16,777,216 rounds: run with --release --ignored"]
fn test_pbkdf2_sha256_sixteen_million_rounds_00() -> Outcome<()> {
    check(b"password", b"salt", 16_777_216,
        "cf81c66fe8cfc04d1f31ecb65dab4089f7f179e89b3b0bcb17ad10e3ac6eba46")
}

#[test]
fn test_pbkdf2_refuses_zero_rounds_00() {
    let mut out = [0u8; 32];
    assert!(pbkdf2_hmac_sha256(b"p", b"s", 0, &mut out).is_err());
}

#[test]
fn test_pbkdf2_empty_output_is_nothing_to_do_00() -> Outcome<()> {
    let mut out: [u8; 0] = [];
    res!(pbkdf2_hmac_sha256(b"p", b"s", 1, &mut out));
    assert!(pbkdf2_hmac_sha256(b"p", b"s", 0, &mut out).is_err(), "zero rounds still refused");
    Ok(())
}
