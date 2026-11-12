//! One Ed25519 key used for X25519 too: the public key mapped to its Montgomery `u` and the seed
//! to the clamped first half of its SHA-512 must be a pair, and two such pairs must agree.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::agree::AgreementScheme;


fn unhex(s: &str) -> Outcome<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        out.push(res!(u8::from_str_radix(&s[i..i + 2], 16), Decode));
    }
    Ok(out)
}

// RFC 8032 §7.1, tests 1 and 2: secret seed and public key.
const KEYS: [(&str, &str); 2] = [
    ("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
     "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"),
    ("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
     "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"),
];

#[test]
fn test_the_converted_secret_and_public_key_are_a_pair_00() -> Outcome<()> {
    for (seed, pk) in KEYS {
        let sk = res!(AgreementScheme::x25519_secret_of_ed25519(&res!(unhex(seed))));
        if sk[0] & 7 != 0 || sk[31] & 0x80 != 0 || sk[31] & 0x40 == 0 {
            return Err(err!("The secret from seed {} is not clamped.", seed; Test, Invalid));
        }
        let from_ed = res!(AgreementScheme::x25519_public_of_ed25519(&res!(unhex(pk))));
        let from_sk = res!(AgreementScheme::x25519_public_of(&sk[..]));
        req!(from_ed, from_sk);
    }
    Ok(())
}

#[test]
fn test_two_converted_keys_agree_00() -> Outcome<()> {
    let a_sk = res!(AgreementScheme::x25519_secret_of_ed25519(&res!(unhex(KEYS[0].0))));
    let b_sk = res!(AgreementScheme::x25519_secret_of_ed25519(&res!(unhex(KEYS[1].0))));
    let a_pk = res!(AgreementScheme::x25519_public_of_ed25519(&res!(unhex(KEYS[0].1))));
    let b_pk = res!(AgreementScheme::x25519_public_of_ed25519(&res!(unhex(KEYS[1].1))));
    let ab = res!(AgreementScheme::x25519_agree(&a_sk[..], &b_pk));
    let ba = res!(AgreementScheme::x25519_agree(&b_sk[..], &a_pk));
    req!(ab, ba);
    Ok(())
}

#[test]
fn test_keys_that_cannot_be_converted_are_refused_00() -> Outcome<()> {
    // The identity point, of small order, and a y with no x on the curve (y = 2).
    let mut identity = [0u8; 32];
    identity[0] = 1;
    let mut off_curve = [0u8; 32];
    off_curve[0] = 2;
    assert!(AgreementScheme::x25519_public_of_ed25519(&identity).is_err());
    assert!(AgreementScheme::x25519_public_of_ed25519(&off_curve).is_err());
    assert!(AgreementScheme::x25519_public_of_ed25519(&[9u8; 31]).is_err());
    assert!(AgreementScheme::x25519_secret_of_ed25519(&[9u8; 33]).is_err());
    Ok(())
}
