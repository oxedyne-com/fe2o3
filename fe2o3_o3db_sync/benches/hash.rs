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

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_hash::api::Hasher;

use test::Bencher;

const INPUT_LEN: usize = 32;

fn gen_input() -> [u8; INPUT_LEN] {
    [42u8; INPUT_LEN]
}

#[bench]
fn bench_seahash(b: &mut Bencher) -> Outcome<()> {
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
fn bench_sha3_256(b: &mut Bencher) -> Outcome<()> {
    let input = gen_input();
    b.iter(|| {
        test::black_box(HashScheme::new_sha3_256().hash(&[&input], []));
    });
    Ok(())
}
