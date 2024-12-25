// To switch to nightly to run these benchmarks:
// > rustup override set nightly
// > clear;clear;cargo bench
// To switch back to stable:
// > rustup override set stable
//
// Run this with
// > cargo bench
#![feature(test)]
extern crate test;

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_crypto::{
    hash::HashScheme,
};

use test::Bencher;

const INPUT_LEN: usize = 32;
const CRYPTO_HASH_LEN: usize = 32;

fn gen_input() -> [u8; INPUT_LEN] {
    [42u8; INPUT_LEN]
}

#[bench]
fn bench_seahash(b: &mut Bencher) -> Result<()> {
    let input = gen_input();
    b.iter(|| {
        test::black_box({
            let hash = seahash::hash(&input);
            hash.to_be_bytes();
        });
    });
    Ok(())
}

#[bench]
fn bench_sha_256(b: &mut Bencher) -> Result<()> {
    let input = gen_input();
    let mut output = [0u8; CRYPTO_HASH_LEN];
    b.iter(|| {
        test::black_box({
            let mut hasher = HashScheme::new_sha3_256();
            hasher.update(&input);
            hasher.finalize(&mut output);
        });
    });
    Ok(())
}
