//! Sign and verify costs for `linkring/1` at 2^10 to 2^16 keys, the basis of the
//! figures at 10^5 to 10^7 in the Hematite User Guide. Release only:
//! `cargo test --release --test linkring_bench -- --ignored --nocapture`.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::linkring::{
    self,
    Ring,
    SecretKey,
};

use std::time::Instant;

#[test]
#[ignore]
fn bench_linkring() -> Outcome<()> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8);
    println!("threads for verify_par: {}", threads);
    // LINKRING_BENCH_N adds one direct size, such as 1000000.
    let mut sizes: Vec<usize> = [10u32, 12, 14, 16].iter().map(|e| 1usize << e).collect();
    if let Ok(v) = std::env::var("LINKRING_BENCH_N") {
        sizes = vec![res!(v.parse::<usize>().map_err(|_| err!("LINKRING_BENCH_N '{}'", v; Test)))];
    }
    for n in sizes {
        let mut list = Vec::with_capacity(n * 32);
        let mut signer = None;
        for i in 0..n {
            let k = res!(SecretKey::from_seed(fmt!("bench-{}", i).as_bytes()));
            list.extend_from_slice(&k.public_key());
            if i == n / 3 {
                signer = Some(k);
            }
        }
        let key = res!(signer.ok_or_else(|| err!("no signer"; Test)));
        let t = Instant::now();
        let ring = res!(Ring::from_list_par(&list, if n > 1 << 16 { threads } else { 1 }));
        let t_dec = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let (tag, body) = res!(linkring::sign(&ring, &key, b"present/1:https://app.example", b"msg"));
        let t_sign = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let ok = res!(linkring::verify(&ring, b"present/1:https://app.example", b"msg", &tag, &body));
        let t_ver = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let ok_p = res!(linkring::verify_par(&ring, b"present/1:https://app.example", b"msg", &tag, &body, threads));
        let t_par = t.elapsed().as_secs_f64();
        req!(ok && ok_p, true);
        let us = |s: f64| s * 1e6 / n as f64;
        println!("N={:<8} body {:>5} B | sign {:>8.1} ms ({:.2} us/key) | verify {:>8.1} ms ({:.2} us/key) \
            | verify x{} {:>7.1} ms ({:.2} us/key) | decode {:.2} us/key",
            n, body.len(), t_sign * 1e3, us(t_sign), t_ver * 1e3, us(t_ver),
            threads, t_par * 1e3, us(t_par), us(t_dec));
    }
    Ok(())
}
