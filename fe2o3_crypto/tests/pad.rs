//! Padmé padding (`oxedyne_fe2o3_crypto::pad`) against the shared vectors in
//! `tests/data/padme_vectors.txt`, which the JS twin (`js/test/pad.test.mjs`) checks too, and the
//! overhead bound of the PURBs paper (arXiv 1806.03160 §4.4, equation 1).

use oxedyne_fe2o3_crypto::pad::{
    self,
    PadConfig,
    PadRule,
};

use oxedyne_fe2o3_core::prelude::*;

const VECTORS: &str = include_str!("data/padme_vectors.txt");

fn rule_of(name: &str, floor: u64, block: u64) -> Outcome<PadRule> {
    let cfg = PadConfig { rule: name.to_string(), floor, block };
    cfg.rule()
}

#[test]
fn padme_len_matches_the_shared_vectors() -> Outcome<()> {
    let mut seen = 0;
    for line in VECTORS.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        if f.first() != Some(&"padme") {
            continue;
        }
        let l: u64 = res!(f[1].parse().map_err(|e| err!("{:?}: {}", line, e; Invalid, Input)));
        let want: u64 = res!(f[2].parse().map_err(|e| err!("{:?}: {}", line, e; Invalid, Input)));
        let got = res!(pad::padme_len(l));
        if got != want {
            return Err(err!("padme_len({}) is {}, the vectors say {}.", l, got, want; Mismatch));
        }
        seen += 1;
    }
    if seen < 3000 {
        return Err(err!("Only {} padme vectors were read.", seen; Missing));
    }
    Ok(())
}

#[test]
fn framed_length_matches_the_shared_vectors() -> Outcome<()> {
    let mut seen = 0;
    for line in VECTORS.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        if f.first() != Some(&"frame") {
            continue;
        }
        let num = |i: usize| -> Outcome<u64> {
            f[i].parse::<u64>().map_err(|e| err!("{:?}: {}", line, e; Invalid, Input))
        };
        let rule = res!(rule_of(f[1], res!(num(2)), res!(num(3))));
        let n = res!(num(4));
        let want = res!(num(5));
        let got = res!(rule.framed_len(n));
        if got != want {
            return Err(err!("{:?} frames {} body bytes in {}, the vectors say {}.",
                rule, n, got, want; Mismatch));
        }
        // Small bodies are padded for real, and come back whole.
        if n < 5000 {
            let body: Vec<u8> = (0..n).map(|i| (i * 7 + 1) as u8).collect();
            let framed = res!(rule.pad(&body));
            if framed.len() as u64 != want {
                return Err(err!("pad gave {} bytes, framed_len {}.", framed.len(), want; Mismatch));
            }
            let back = res!(pad::unpad(&framed));
            if back != &body[..] {
                return Err(err!("unpad did not return the body of {} bytes.", n; Mismatch));
            }
        }
        seen += 1;
    }
    if seen < 1000 {
        return Err(err!("Only {} frame vectors were read.", seen; Missing));
    }
    Ok(())
}

#[test]
fn padme_overhead_is_within_the_paper_bound() -> Outcome<()> {
    // Equation 1: the added bytes are below 2^(E-S), and the overhead under 12% everywhere. The
    // paper's text quotes +11.11% (9 bytes to 10) as the maximum, but its own Algorithm 1 pads 129
    // bytes to 144 (+11.63%), so the ceiling asserted is its "always less than 12%".
    let mut worst = 0.0f64;
    let mut l: u64 = 2;
    while l < (1u64 << 22) {
        let p = res!(pad::padme_len(l));
        if p < l {
            return Err(err!("padme_len({}) = {} shrinks.", l, p; Invalid));
        }
        let e = 63 - l.leading_zeros() as u64;
        let s = 64 - e.leading_zeros() as u64;
        if p - l >= (1u64 << (e - s)).max(1) {
            return Err(err!("padme_len({}) = {} adds 2^(E-S) or more.", l, p; Invalid));
        }
        let over = (p - l) as f64 / l as f64;
        if over >= 0.12 {
            return Err(err!("padme_len({}) = {}, overhead {} over 12%.", l, p, over; Invalid));
        }
        if l > 256 {
            worst = worst.max(over);
        }
        l += if l < 70_000 { 1 } else { 997 };
    }
    if worst <= 0.0 {
        return Err(err!("No overhead was measured."; Missing));
    }
    Ok(())
}

#[test]
fn unpad_refuses_a_frame_that_lies() -> Outcome<()> {
    let rule = res!(rule_of("padme", 64, 0));
    let mut framed = res!(rule.pad(b"hello"));
    // Shorter than its prefix.
    if pad::unpad(&framed[..3]).is_ok() {
        return Err(err!("A 3-byte frame was accepted."; Invalid));
    }
    // A prefix claiming more than the frame holds.
    framed[0] = 0xff;
    if pad::unpad(&framed).is_ok() {
        return Err(err!("A prefix past the end was accepted."; Invalid));
    }
    // Non-zero fill.
    let mut framed = res!(rule.pad(b"hello"));
    let last = framed.len() - 1;
    framed[last] = 1;
    if pad::unpad(&framed).is_ok() {
        return Err(err!("A frame with non-zero fill was accepted."; Invalid));
    }
    Ok(())
}

#[test]
fn config_names_the_rule() -> Outcome<()> {
    if rule_of("block", 0, 0).is_ok() {
        return Err(err!("A block rule of zero bytes was accepted."; Invalid));
    }
    if rule_of("constant-ish", 0, 0).is_ok() {
        return Err(err!("An unknown rule name was accepted."; Invalid));
    }
    let mut cfg = PadConfig::default();
    if res!(cfg.rule()) != (PadRule::Padme { floor: cfg.floor }) {
        return Err(err!("The default rule is not Padmé."; Invalid));
    }
    cfg.rule = "pow2".to_string();
    match res!(cfg.rule()) {
        PadRule::PowerOfTwo { .. } => Ok(()),
        other => Err(err!("pow2 read as {:?}.", other; Invalid)),
    }
}
