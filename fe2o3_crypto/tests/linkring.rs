//! `linkring/1` against an independent implementation, `tools/linkring_oracle.py`,
//! both through vectors it produced and, where `python3` exists, live.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::linkring::{
    self,
    Ring,
    SecretKey,
};

use std::io::Write;
use std::process::{
    Command,
    Stdio,
};

use rand::RngCore;

struct Case {
    name:   String,
    seeds:  Vec<Vec<u8>>,
    signer: usize,
    scope:  Vec<u8>,
    msg:    Vec<u8>,
    aux:    Vec<u8>,
    fields: Vec<(String, Vec<u8>)>, // ring, digest, tag, body, ok
}

impl Case {
    fn get(&self, k: &str) -> Outcome<&Vec<u8>> {
        match self.fields.iter().find(|(key, _)| key == k) {
            Some((_, v)) => Ok(v),
            None => Err(err!("Case {} has no {}.", self.name, k; Missing, Test)),
        }
    }
}

fn unhex(s: &str) -> Outcome<Vec<u8>> {
    Ok(res!(hex::decode(s).map_err(|e| err!("Bad hex '{}': {}", s, e; Decode, Test))))
}

fn parse(text: &str) -> Outcome<Vec<Case>> {
    let mut out = Vec::new();
    let mut cur: Option<Case> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "case" => cur = Some(Case {
                name: val.to_string(), seeds: Vec::new(), signer: 0, scope: Vec::new(),
                msg: Vec::new(), aux: Vec::new(), fields: Vec::new(),
            }),
            "end" => {
                if let Some(c) = cur.take() {
                    out.push(c);
                }
            }
            _ => {
                let c = res!(cur.as_mut().ok_or_else(|| err!("'{}' outside a case.", key; Test)));
                match key {
                    "seeds"     => for s in val.split(',') { c.seeds.push(res!(unhex(s))); },
                    "signer"    => c.signer = res!(val.parse::<usize>().map_err(|_| err!(
                                    "Bad signer '{}'.", val; Decode, Test))),
                    "scope"     => c.scope = res!(unhex(val)),
                    "msg"       => c.msg = res!(unhex(val)),
                    "aux"       => c.aux = res!(unhex(val)),
                    "ok"        => c.fields.push((key.to_string(), val.as_bytes().to_vec())),
                    _           => c.fields.push((key.to_string(), res!(unhex(val)))),
                }
            }
        }
    }
    Ok(out)
}

fn keys_of(seeds: &[Vec<u8>]) -> Outcome<(Vec<SecretKey>, Ring)> {
    let mut keys = Vec::with_capacity(seeds.len());
    for s in seeds {
        keys.push(res!(SecretKey::from_seed(s)));
    }
    let pubs: Vec<[u8; 32]> = keys.iter().map(|k| k.public_key()).collect();
    let ring = res!(Ring::from_keys(&pubs));
    Ok((keys, ring))
}

fn ring_of(n: usize, label: &str) -> Outcome<(Vec<SecretKey>, Ring)> {
    let seeds: Vec<Vec<u8>> = (0..n).map(|i| fmt!("{}-{}", label, i).into_bytes()).collect();
    keys_of(&seeds)
}

fn oracle_path() -> String {
    fmt!("{}/tools/linkring_oracle.py", env!("CARGO_MANIFEST_DIR"))
}

fn run_oracle(mode: &str, input: &str) -> Outcome<Option<String>> {
    let child = Command::new("python3")
        .arg(oracle_path())
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    if let Some(mut stdin) = child.stdin.take() {
        res!(stdin.write_all(input.as_bytes()));
    }
    let out = res!(child.wait_with_output());
    if !out.status.success() {
        return Err(err!("The oracle exited with {}: {}", out.status,
            String::from_utf8_lossy(&out.stdout); Test));
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

// ── External oracle ─────────────────────────────────────────────────────────

#[test]
fn test_oracle_vectors() -> Outcome<()> {
    let text = include_str!("data/linkring_vectors.txt");
    let cases = res!(parse(text));
    req!((cases.len() >= 6), true, "vector count");
    for c in &cases {
        let (keys, ring) = res!(keys_of(&c.seeds));
        let list: Vec<u8> = ring.keys().concat();
        req!(&list, res!(c.get("ring")), "{} ring", c.name);
        req!(&ring.digest().to_vec(), res!(c.get("digest")), "{} digest", c.name);
        req!(&linkring::ring_digest(&list).to_vec(), res!(c.get("digest")), "{} digest fn", c.name);
        let key = &keys[c.signer];
        req!(&res!(linkring::tag(key, &c.scope)).to_vec(), res!(c.get("tag")), "{} tag()", c.name);
        let (tag, body) = res!(linkring::sign_with_aux(&ring, key, &c.scope, &c.msg, &c.aux));
        req!(&tag.to_vec(), res!(c.get("tag")), "{} tag", c.name);
        req!(&body, res!(c.get("body")), "{} body", c.name);
        req!(res!(linkring::verify(&ring, &c.scope, &c.msg, &tag, &body)), true, "{}", c.name);
    }
    Ok(())
}

#[test]
fn test_oracle_live() -> Outcome<()> {
    let mut rng = rand::thread_rng();
    let mut input = String::new();
    let mut made = Vec::new();
    for (ci, n) in [3usize, 16, 18, 29].into_iter().enumerate() {
        let seeds: Vec<Vec<u8>> = (0..n).map(|_| {
            let mut s = vec![0u8; 24];
            rng.fill_bytes(&mut s);
            s
        }).collect();
        let signer = (rng.next_u32() as usize) % n;
        let mut scope = vec![0u8; 12];
        let mut msg = vec![0u8; 40];
        let mut aux = vec![0u8; 8];
        rng.fill_bytes(&mut scope);
        rng.fill_bytes(&mut msg);
        rng.fill_bytes(&mut aux);
        let (keys, ring) = res!(keys_of(&seeds));
        let (tag, body) = res!(linkring::sign_with_aux(&ring, &keys[signer], &scope, &msg, &aux));
        let seeds_hex: Vec<String> = seeds.iter().map(hex::encode).collect();
        input.push_str(&fmt!("case c{}\nseeds {}\nsigner {}\nscope {}\nmsg {}\naux {}\nend\n",
            ci, seeds_hex.join(","), signer, hex::encode(&scope), hex::encode(&msg), hex::encode(&aux)));
        made.push((ring, scope, msg, tag, body));
    }
    let signed = match res!(run_oracle("sign", &input)) {
        Some(s) => s,
        None => {
            println!("python3 not found: the live oracle is skipped; the vectors still ran.");
            return Ok(());
        }
    };
    let cases = res!(parse(&signed));
    req!(cases.len(), made.len());
    // The oracle, signing independently, produces the same bytes.
    for (c, (ring, _, _, tag, body)) in cases.iter().zip(made.iter()) {
        req!(res!(c.get("ring")), &ring.keys().concat(), "{} ring", c.name);
        req!(res!(c.get("tag")), &tag.to_vec(), "{} tag", c.name);
        req!(res!(c.get("body")), body, "{} body", c.name);
    }
    // The oracle verifies Rust's bodies and refuses tampered ones.
    let mut vin = String::new();
    let mut want = Vec::new();
    for (i, (ring, scope, msg, tag, body)) in made.iter().enumerate() {
        let list = hex::encode(ring.keys().concat());
        let mut other = ring.keys().to_vec();
        other.swap(0, ring.len() - 1);
        let mut bad_body = body.clone();
        let last = bad_body.len() - 1;
        bad_body[last - 40] ^= 1;
        let mut bad_msg = msg.clone();
        bad_msg[0] ^= 1;
        let variants: Vec<(&str, String, &[u8], &[u8], &[u8], Vec<u8>, bool)> = vec![
            ("ok",    list.clone(),                 scope, msg,      tag, body.clone(), true),
            ("msg",   list.clone(),                 scope, &bad_msg, tag, body.clone(), false),
            ("scope", list.clone(),                 b"x",  msg,      tag, body.clone(), false),
            ("body",  list.clone(),                 scope, msg,      tag, bad_body,     false),
            ("ring",  hex::encode(other.concat()),  scope, msg,      tag, body.clone(), false),
        ];
        for (name, rl, sc, ms, tg, bd, ok) in variants {
            vin.push_str(&fmt!("case c{}-{}\nring {}\nscope {}\nmsg {}\ntag {}\nbody {}\nend\n",
                i, name, rl, hex::encode(sc), hex::encode(ms), hex::encode(tg), hex::encode(&bd)));
            want.push(ok);
        }
    }
    let verified = res!(res!(run_oracle("verify", &vin)).ok_or_else(|| err!("python3 vanished"; Test)));
    let got = res!(parse(&verified));
    req!(got.len(), want.len());
    for (c, ok) in got.iter().zip(want.iter()) {
        let expect: &[u8] = if *ok { b"true" } else { b"false" };
        req!(res!(c.get("ok")).as_slice(), expect, "{}", c.name);
    }
    Ok(())
}

// ── Refusals ────────────────────────────────────────────────────────────────

#[test]
fn test_tamper_each_refuses() -> Outcome<()> {
    let (keys, ring) = res!(ring_of(21, "tamper"));
    let scope = b"present/1:https://app.example";
    let msg = b"{\"v\":\"present/1\"}";
    let (tag, body) = res!(linkring::sign(&ring, &keys[9], scope, msg));
    req!(res!(linkring::verify(&ring, scope, msg, &tag, &body)), true, "honest");

    // Ring: a key replaced, two swapped, one dropped, one added.
    let mut ks = ring.keys().to_vec();
    ks[3] = res!(SecretKey::from_seed(b"outsider")).public_key();
    let r = res!(Ring::from_keys(&ks));
    req!(res!(linkring::verify(&r, scope, msg, &tag, &body)), false, "ring replaced");
    let mut ks = ring.keys().to_vec();
    ks.swap(2, 15);
    let r = res!(Ring::from_keys(&ks));
    req!(res!(linkring::verify(&r, scope, msg, &tag, &body)), false, "ring swapped");
    let ks = ring.keys()[..20].to_vec();
    let r = res!(Ring::from_keys(&ks));
    req!(res!(linkring::verify(&r, scope, msg, &tag, &body)), false, "ring dropped");
    let mut ks = ring.keys().to_vec();
    ks.push(res!(SecretKey::from_seed(b"extra")).public_key());
    let r = res!(Ring::from_keys(&ks));
    req!(res!(linkring::verify(&r, scope, msg, &tag, &body)), false, "ring added");

    // Message and scope.
    req!(res!(linkring::verify(&ring, scope, b"{\"v\":\"present/2\"}", &tag, &body)), false, "msg");
    req!(res!(linkring::verify(&ring, b"present/1:https://evil.example", msg, &tag, &body)), false,
        "scope");

    // Tag: another member's, the same key's under another scope, the key itself.
    let t2 = res!(linkring::tag(&keys[10], scope));
    req!(res!(linkring::verify(&ring, scope, msg, &t2, &body)), false, "other member's tag");
    let t3 = res!(linkring::tag(&keys[9], b"present/1:https://other.example"));
    req!(res!(linkring::verify(&ring, scope, msg, &t3, &body)), false, "other scope's tag");
    req!(res!(linkring::verify(&ring, scope, msg, &keys[9].public_key(), &body)), false, "pubkey");

    // Body: a valid point or scalar swapped in at every region.
    let m = linkring::digits(ring.len());
    let base_pt = keys[0].public_key();
    for i in 0..(4 + 2 * m) {
        let mut b = body.clone();
        b[1 + 32 * i..33 + 32 * i].copy_from_slice(&base_pt);
        req!(res!(linkring::verify(&ring, scope, msg, &tag, &b)), false, "point {}", i);
    }
    let first_sc = 1 + 32 * (4 + 2 * m);
    for i in 0..(15 * m + 3) {
        let mut b = body.clone();
        let off = first_sc + 32 * i;
        b[off..off + 32].copy_from_slice(&res!(SecretKey::from_seed(b"sc")).to_bytes());
        req!(res!(linkring::verify(&ring, scope, msg, &tag, &b)), false, "scalar {}", i);
    }

    // Malformed: wrong length, wrong digit count, non-canonical scalar, bad point.
    req!(res!(linkring::verify(&ring, scope, msg, &tag, &body[..body.len() - 1])), false, "short");
    let mut b = body.clone();
    b.push(0);
    req!(res!(linkring::verify(&ring, scope, msg, &tag, &b)), false, "long");
    let mut b = body.clone();
    b[0] = 3;
    req!(res!(linkring::verify(&ring, scope, msg, &tag, &b)), false, "m byte");
    let mut b = body.clone();
    let off = b.len() - 32;
    for x in &mut b[off..] { *x = 0xff; }
    req!(res!(linkring::verify(&ring, scope, msg, &tag, &b)), false, "non-canonical z");
    let mut b = body.clone();
    b[1..33].copy_from_slice(&[0xff; 32]);
    req!(res!(linkring::verify(&ring, scope, msg, &tag, &b)), false, "bad point");
    req!(res!(linkring::verify(&ring, scope, msg, &tag[..31], &body)), false, "short tag");
    req!(res!(linkring::verify(&ring, scope, msg, &[0u8; 32], &body)), false, "identity tag");
    Ok(())
}

#[test]
fn test_ring_refuses_bad_keys() -> Outcome<()> {
    let good = res!(SecretKey::from_seed(b"g")).public_key();
    req!(Ring::from_list(&[]).is_err(), true, "empty");
    req!(Ring::from_list(&[0u8; 33]).is_err(), true, "ragged");
    let mut list = good.to_vec();
    list.extend_from_slice(&[0u8; 32]);
    req!(Ring::from_list(&list).is_err(), true, "identity key");
    let mut list = good.to_vec();
    list.extend_from_slice(&[0xffu8; 32]);
    req!(Ring::from_list(&list).is_err(), true, "not an encoding");
    let ring = res!(Ring::from_list(&good));
    let outsider = res!(SecretKey::from_seed(b"o"));
    req!(linkring::sign(&ring, &outsider, b"s", b"m").is_err(), true, "signer not in ring");
    req!(SecretKey::from_bytes(&[0u8; 32]).is_err(), true, "zero secret");
    req!(SecretKey::from_bytes(&[0xffu8; 32]).is_err(), true, "non-canonical secret");
    let k = res!(SecretKey::from_seed(b"rt"));
    req!(res!(SecretKey::from_bytes(&k.to_bytes())).public_key(), k.public_key(), "round trip");
    Ok(())
}

// ── Tags ────────────────────────────────────────────────────────────────────

#[test]
fn test_tag_stable_per_key_and_scope() -> Outcome<()> {
    let (keys, ring) = res!(ring_of(7, "stable"));
    let scope = b"present/1:https://app.example";
    let (t1, b1) = res!(linkring::sign(&ring, &keys[4], scope, b"first"));
    let (t2, b2) = res!(linkring::sign(&ring, &keys[4], scope, b"second"));
    req!(t1, t2, "same key, same scope");
    req!((b1 != b2), true, "fresh bodies");
    req!(t1, res!(linkring::tag(&keys[4], scope)), "tag() agrees");
    // A second ring holding the key gives the same tag: the tag is the key's,
    // not the ring's.
    let (more, _) = res!(ring_of(12, "other-ring"));
    let mut ks: Vec<[u8; 32]> = more.iter().map(|k| k.public_key()).collect();
    ks.push(keys[4].public_key());
    let ring2 = res!(Ring::from_keys(&ks));
    let (t3, _) = res!(linkring::sign(&ring2, &keys[4], scope, b"third"));
    req!(t1, t3, "same tag over another ring");
    let (t4, _) = res!(linkring::sign(&ring, &keys[4], b"present/1:https://b.example", b"m"));
    req!((t1 != t4), true, "scopes differ");
    let (t5, _) = res!(linkring::sign(&ring, &keys[5], scope, b"m"));
    req!((t1 != t5), true, "keys differ");
    Ok(())
}

// Tags of one key under two scopes should look no more alike than the tags of
// two different keys. Bit 0 of byte 0 and bit 7 of byte 31 are fixed by the
// ristretto255 encoding and so are left out.
#[test]
fn test_tag_unlinkable_across_scopes() -> Outcome<()> {
    const K: usize = 400;
    let s1 = b"present/1:https://a.example";
    let s2 = b"present/1:https://b.example";
    let mut t1 = Vec::with_capacity(K);
    let mut t2 = Vec::with_capacity(K);
    let mut pk = Vec::with_capacity(K);
    for i in 0..K {
        let k = res!(SecretKey::from_seed(fmt!("unlink-{}", i).as_bytes()));
        t1.push(res!(linkring::tag(&k, s1)));
        t2.push(res!(linkring::tag(&k, s2)));
        pk.push(k.public_key());
    }
    let free_bit = |b: usize| b != 0 && b != 255;
    let bit = |x: &[u8; 32], b: usize| (x[b / 8] >> (b % 8)) & 1;
    let dist = |x: &[u8; 32], y: &[u8; 32]| (0..256).filter(|&b| free_bit(b) && bit(x, b) != bit(y, b)).count();
    // Mean Hamming distance over the 254 free bits: 127 expected, with a
    // standard error of about 8/√K ≈ 0.4.
    let mean = |pairs: &dyn Fn(usize) -> usize| (0..K).map(pairs).sum::<usize>() as f64 / K as f64;
    let same_key = mean(&|i| dist(&t1[i], &t2[i]));
    let diff_key = mean(&|i| dist(&t1[i], &t2[(i + 1) % K]));
    let vs_pub = mean(&|i| dist(&t1[i], &pk[i]));
    for (name, v) in [("same key", same_key), ("different keys", diff_key), ("tag vs key", vs_pub)] {
        req!(((v - 127.0).abs() < 3.0), true, "{} mean distance {}", name, v);
    }
    req!(((same_key - diff_key).abs() < 3.0), true, "linking gap {} vs {}", same_key, diff_key);
    // Per-bit agreement between the two scopes: K/2 expected, standard
    // deviation √K/2 = 10; 5 deviations allowed across 254 bits.
    for b in (0..256).filter(|&b| free_bit(b)) {
        let agree = (0..K).filter(|&i| bit(&t1[i], b) == bit(&t2[i], b)).count() as f64;
        req!(((agree - K as f64 / 2.0).abs() < 50.0), true, "bit {} agrees {} times", b, agree);
    }
    Ok(())
}

// ── Sizes and threads ───────────────────────────────────────────────────────

#[test]
fn test_sizes_at_scale() -> Outcome<()> {
    for (n, m, len) in [
        (256usize,      2usize, 1_313usize),
        (100_000,       5,      2_945),
        (1_000_000,     5,      2_945),
        (10_000_000,    6,      3_489),
    ] {
        req!(linkring::digits(n), m, "digits({})", n);
        req!(linkring::body_len(m), len, "body_len({})", m);
    }
    Ok(())
}

#[test]
fn test_threads_agree() -> Outcome<()> {
    let (keys, ring) = res!(ring_of(1100, "threads"));
    let list: Vec<u8> = ring.keys().concat();
    let ring4 = res!(Ring::from_list_par(&list, 4));
    req!(ring4.digest(), ring.digest());
    let (tag, body) = res!(linkring::sign(&ring, &keys[1099], b"s", b"m"));
    req!(res!(linkring::verify_par(&ring4, b"s", b"m", &tag, &body, 4)), true, "4 threads");
    req!(res!(linkring::verify_par(&ring4, b"s", b"x", &tag, &body, 4)), false, "4 threads, bad msg");
    Ok(())
}
