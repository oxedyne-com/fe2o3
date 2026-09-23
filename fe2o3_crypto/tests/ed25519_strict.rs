//! Ed25519 verification held to the strict rules, against oracles this crate does
//! not define.
//!
//! - The C2SP CCTV edge-case vectors, `tests/data/ed25519vectors.json` (provenance
//!   in `tests/data/PROVENANCE.md`). Each vector is flagged with the edge cases it
//!   exercises, so the verdict a verifier owes it follows from its flags and the
//!   rules alone.
//! - RFC 8032 §7.1's own examples, which must still verify.
//! - `ed25519-dalek`'s `verify_strict`, differentially, over honest signatures and
//!   random mutations of them.
//!
//! The rules are RFC 8032 §5.1.7 read strictly: the public key and R must each be
//! the canonical encoding of a point (§5.1.3), S must lie below the group order,
//! the group equation is the cofactored one the RFC gives first, and a public key
//! or R of small order is refused whatever the equation says.
//!
//! Every check goes through `SignatureScheme::verify` or `verify_batch`, the path
//! every caller takes, so nothing here can pass while a caller is still exposed.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::sign::SignatureScheme;
use oxedyne_fe2o3_iop_crypto::{
    keys::KeyManager,
    sign::{
        BatchItem,
        Signer,
    },
};
use oxedyne_fe2o3_jdat::prelude::*;

use ed25519_dalek::{
    Signature,
    Signer as DalekSigner,
    SigningKey,
    VerifyingKey,
};
use rand::RngCore;

// The edge cases a strict verifier refuses outright. The other flags, a
// low-order *component* of A or R and a low-order residue that the cofactored
// equation absorbs, leave a signature to stand or fall by the equation.
const REFUSED: [&str; 4] = [
    "low_order_A",
    "low_order_R",
    "non_canonical_A",
    "non_canonical_R",
];

struct Vector {
    number: u64,
    key:    Vec<u8>,
    sig:    Vec<u8>,
    msg:    Vec<u8>,
    flags:  Vec<String>,
}

impl Vector {
    /// Does a strict verifier owe this vector acceptance?
    fn acceptable(&self) -> bool {
        !self.flags.iter().any(|f| REFUSED.contains(&f.as_str()))
    }
}

fn unhex(s: &str) -> Outcome<Vec<u8>> {
    Ok(res!(hex::decode(s).map_err(|e| err!("Bad hex '{}': {:?}", s, e; Decode, Test))))
}

fn vectors() -> Outcome<Vec<Vector>> {
    let dat = res!(Dat::decode_string(include_str!("data/ed25519vectors.json")));
    let list = match dat {
        Dat::List(list) => list,
        other => return Err(err!(
            "The CCTV vector file holds a {:?} where a list was expected.", other.kind();
            Decode, Test)),
    };
    let mut out = Vec::with_capacity(list.len());
    for item in list.iter() {
        let number = res!(item.map_get_u64(&dat!("number")));
        let flags = match res!(item.map_get(&dat!("flags"))) {
            Some(Dat::List(fs)) => {
                let mut flags = Vec::with_capacity(fs.len());
                for f in fs {
                    match f {
                        Dat::Str(s) => flags.push(s.clone()),
                        other => return Err(err!(
                            "Vector {} has a flag of kind {:?}.", number, other.kind();
                            Decode, Test)),
                    }
                }
                flags
            },
            _ => Vec::new(), // null, the one vector with no edge case
        };
        out.push(Vector {
            number,
            key:    res!(unhex(&res!(item.map_get_string(&dat!("key"))))),
            sig:    res!(unhex(&res!(item.map_get_string(&dat!("sig"))))),
            msg:    res!(item.map_get_string(&dat!("msg"))).into_bytes(),
            flags,
        });
    }
    Ok(out)
}

/// Checks one signature the way every caller checks one.
fn single(public: &[u8], msg: &[u8], sig: &[u8]) -> Outcome<bool> {
    let bound = res!(SignatureScheme::empty_ed25519().clone_with_keys(Some(public), None));
    bound.verify(msg, sig)
}

fn batch(items: &[BatchItem<'_>]) -> Outcome<bool> {
    SignatureScheme::empty_ed25519().verify_batch(items)
}

/// A fresh key pair, as `(signing key, public key bytes)`.
fn pair() -> (SigningKey, Vec<u8>) {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    (sk, pk)
}

/// The identity point as a public key, with R the identity and S zero, satisfies
/// the cofactorless equation `[S]B = R + [k]A` for every message, since every term
/// is the identity whatever k is. A verifier that takes this is a verifier any
/// stranger can sign for, which is why a strict verifier refuses a small-order
/// key before it looks at the equation.
#[test]
fn a_small_order_key_signs_nothing() -> Outcome<()> {
    let mut identity = [0u8; 32];
    identity[0] = 0x01;
    let mut forged = [0u8; 64];
    forged[0] = 0x01; // R, the identity; S stays zero.
    let msgs: [&[u8]; 4] = [
        b"",
        b"transfer everything to the forger",
        b"a message nobody signed",
        &[0xffu8; 200],
    ];
    for msg in msgs.iter() {
        req!(res!(single(&identity, msg, &forged)), false,
            "the identity key's forgery was accepted singly over {:?}", msg);
        req!(res!(batch(&[BatchItem { public: &identity, msg, sig: &forged }])), false,
            "the identity key's forgery was accepted in a batch over {:?}", msg);
    }
    // A forgery must not hide in a batch among sound signatures either.
    let (sk, pk) = pair();
    let sound = sk.sign(b"sound").to_bytes();
    req!(res!(batch(&[
        BatchItem { public: &pk,       msg: b"sound",   sig: &sound },
        BatchItem { public: &identity, msg: b"forged",  sig: &forged },
        BatchItem { public: &pk,       msg: b"sound",   sig: &sound },
    ])), false, "the forgery was accepted inside a batch of sound signatures");
    Ok(())
}

/// Every CCTV vector earns the strict verdict its flags imply.
#[test]
fn the_cctv_edge_cases_earn_the_strict_verdict() -> Outcome<()> {
    let vs = res!(vectors());
    req!(vs.len(), 914, "the CCTV file has 914 vectors");
    let mut wrong = Vec::new();
    for v in vs.iter() {
        let got = res!(single(&v.key, &v.msg, &v.sig));
        if got != v.acceptable() {
            wrong.push(fmt!("#{} {:?} gave {}", v.number, v.flags, got));
        }
    }
    req!(wrong.len(), 0, "{} vectors earned the wrong verdict: {:?}",
        wrong.len(), &wrong[..wrong.len().min(8)]);
    // The file's accepted set is not empty, so the test above is not passed by a
    // verifier that refuses everything.
    req!(vs.iter().filter(|v| v.acceptable()).count(), 106,
        "106 vectors carry only flags a strict verifier accepts");
    Ok(())
}

/// The batch accepts each CCTV vector exactly when the single check does, alone
/// and among sound signatures.
///
/// The two are different equations, and Ore's `Envelope::verify_all` promises its
/// callers that a batch accepts the set the single check accepts, envelope for
/// envelope. With the cofactorless equation that promise fails for a signature
/// whose R carries a low-order component: the single check refuses it, and a
/// batch accepts it whenever the batch's own coefficient happens to annihilate
/// that component, which a signer can arrange by trying messages.
#[test]
fn the_batch_agrees_with_the_single_check_on_every_edge_case() -> Outcome<()> {
    let (sk, pk) = pair();
    let sound_msg = b"a sound signature beside the vector";
    let sound = sk.sign(sound_msg).to_bytes();
    let mut wrong = Vec::new();
    for v in res!(vectors()).iter() {
        let one = res!(single(&v.key, &v.msg, &v.sig));
        let alone = res!(batch(&[BatchItem { public: &v.key, msg: &v.msg, sig: &v.sig }]));
        let among = res!(batch(&[
            BatchItem { public: &pk,    msg: sound_msg, sig: &sound },
            BatchItem { public: &v.key, msg: &v.msg,    sig: &v.sig },
            BatchItem { public: &pk,    msg: sound_msg, sig: &sound },
        ]));
        if alone != one || among != one {
            wrong.push(fmt!("#{} {:?}: single {}, alone {}, among {}",
                v.number, v.flags, one, alone, among));
        }
    }
    req!(wrong.len(), 0, "{} vectors split the batch from the single check: {:?}",
        wrong.len(), &wrong[..wrong.len().min(8)]);
    Ok(())
}

/// RFC 8032 §7.1's examples verify, alone and as one batch, and a signature
/// moved to another example's message does not.
#[test]
fn the_rfc8032_examples_verify() -> Outcome<()> {
    // (secret, public, message, signature), TEST 1, 2, 3 and SHA(abc).
    let cases: [(&str, &str, &str, &str); 4] = [
        (
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        ),
        (
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            "72",
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da\
             085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        ),
        (
            "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
            "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
            "af82",
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac\
             18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
        ),
        (
            "833fe62409237b9d62ec77587520911e9a759cec1d19755b7da901b96dca3d42",
            "ec172b93ad5e563bf4932c70e1245034c35467ef2efd4d64ebf819683467e2bf",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            "dc2a4459e7369633a52b1bf277839a00201009a3efbf3ecb69bea2186c26b589\
             09351fc9ac90b3ecfdfbc7c66431e0303dca179c138ac17ad9bef1177331a704",
        ),
    ];
    let mut parsed = Vec::with_capacity(cases.len());
    for (secret, public, msg, sig) in cases.iter() {
        let (secret, public, msg, sig) =
            (res!(unhex(secret)), res!(unhex(public)), res!(unhex(msg)), res!(unhex(sig)));
        // The scheme's own signer reproduces the RFC's signature, Ed25519 being
        // deterministic, so the vectors are read the way the RFC means them.
        let signer = res!(SignatureScheme::empty_ed25519()
            .clone_with_keys(Some(&public), Some(&secret)));
        req!(res!(signer.sign(&msg)), sig.clone(), "signing reproduces the RFC's signature");
        req!(res!(single(&public, &msg, &sig)), true, "an RFC 8032 example verifies");
        parsed.push((public, msg, sig));
    }
    let items: Vec<BatchItem<'_>> = parsed.iter()
        .map(|(public, msg, sig)| BatchItem { public, msg, sig })
        .collect();
    req!(res!(batch(&items)), true, "the RFC 8032 examples verify as one batch");
    req!(res!(single(&parsed[0].0, &parsed[1].1, &parsed[0].2)), false,
        "a signature moved to another message does not verify");
    Ok(())
}

/// Honest signatures verify, and random mutations of the key, the message and
/// each half of the signature earn the same verdict here as from
/// `ed25519-dalek`'s `verify_strict`, singly and in a batch.
///
/// The two verifiers differ only where a residue of small order survives, which
/// a random mutation reaches with negligible probability, so any disagreement
/// here is a fault in this crate.
#[test]
fn verdicts_match_dalek_strict_over_honest_and_mutated_signatures() -> Outcome<()> {
    let mut rng = rand::thread_rng();
    for round in 0..300 {
        let (sk, pk) = pair();
        let mut msg = vec![0u8; (rng.next_u32() % 96) as usize];
        rng.fill_bytes(&mut msg);
        let sig = sk.sign(&msg).to_bytes().to_vec();
        req!(res!(single(&pk, &msg, &sig)), true, "an honest signature verifies (round {})", round);

        // Mutate one of key, message, R or S, by one random bit.
        let (mut pk2, mut msg2, mut sig2) = (pk.clone(), msg.clone(), sig.clone());
        match round % 4 {
            0 => { let i = (rng.next_u32() % 32) as usize; pk2[i] ^= 1 << (rng.next_u32() % 8); },
            1 => {
                if msg2.is_empty() {
                    msg2.push(0x00);
                } else {
                    let i = rng.next_u32() as usize % msg2.len();
                    msg2[i] ^= 1 << (rng.next_u32() % 8);
                }
            },
            2 => { let i = (rng.next_u32() % 32) as usize; sig2[i] ^= 1 << (rng.next_u32() % 8); },
            _ => { let i = 32 + (rng.next_u32() % 32) as usize; sig2[i] ^= 1 << (rng.next_u32() % 8); },
        }
        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pk2);
        let dalek = match VerifyingKey::from_bytes(&pk_arr) {
            // A mutated key that is not a point at all is an error here, as it
            // always has been; there is nothing to compare.
            Err(_) => {
                req!(single(&pk2, &msg2, &sig2).is_err(), true,
                    "a key that is not a point is an error (round {})", round);
                continue;
            },
            Ok(vk) => match Signature::from_slice(&sig2) {
                Ok(s) => vk.verify_strict(&msg2, &s).is_ok(),
                Err(_) => false,
            },
        };
        let ours = res!(single(&pk2, &msg2, &sig2));
        req!(ours, dalek, "the verdict differs from dalek's verify_strict (round {}, case {})",
            round, round % 4);
        let batched = res!(batch(&[BatchItem { public: &pk2, msg: &msg2, sig: &sig2 }]));
        req!(batched, dalek, "the batch verdict differs from dalek's verify_strict (round {})",
            round);
    }
    Ok(())
}
