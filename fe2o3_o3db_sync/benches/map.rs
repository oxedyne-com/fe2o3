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
use oxedyne_fe2o3_crypto::{
    hash::HashScheme,
};

use test::Bencher;

use std::{
    collections::{
        HashMap,
        BTreeMap,
    },
};

const VAL_LEN: usize = 1_000;
const KEY_LEN: usize = 32;

fn gen_val() -> Vec<u8> {
    vec![42u8; VAL_LEN]
}

fn gen_key() -> Vec<u8> {
    vec![42u8; KEY_LEN]
}

//-----------------------------------------------------------------------------------------------//
//      Create maps
//-----------------------------------------------------------------------------------------------//
#[bench]
fn bench_create_u32_hashmap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: HashMap<u32, Vec<u8>> = HashMap::new();
        });
    });
    Ok(())
}

#[bench]
fn bench_create_u64_hashmap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: HashMap<u64, Vec<u8>> = HashMap::new();
        });
    });
    Ok(())
}

#[bench]
fn bench_create_u128_hashmap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: HashMap<u128, Vec<u8>> = HashMap::new();
        });
    });
    Ok(())
}

#[bench]
fn bench_create_vecu8_hashmap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        });
    });
    Ok(())
}

#[bench]
fn bench_create_u32_btreemap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
        });
    });
    Ok(())
}

#[bench]
fn bench_create_u64_btreemap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
        });
    });
    Ok(())
}

#[bench]
fn bench_create_u128_btreemap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: BTreeMap<u128, Vec<u8>> = BTreeMap::new();
        });
    });
    Ok(())
}

#[bench]
fn bench_create_vecu8_btreemap(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box({
            let mut map: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        });
    });
    Ok(())
}

//-----------------------------------------------------------------------------------------------//
//      Map insertion
//-----------------------------------------------------------------------------------------------//

#[bench]
fn bench_insert_u32_hashmap(b: &mut Bencher) -> Result<()> {
    let mut map: HashMap<u32, Vec<u8>> = HashMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(42u32, val);
        });
    });
    Ok(())
}

#[bench]
fn bench_insert_u64_hashmap(b: &mut Bencher) -> Result<()> {
    let mut map: HashMap<u64, Vec<u8>> = HashMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(42u64, val);
        });
    });
    Ok(())
}

#[bench]
fn bench_insert_u128_hashmap(b: &mut Bencher) -> Result<()> {
    let mut map: HashMap<u128, Vec<u8>> = HashMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(42u128, val);
        });
    });
    Ok(())
}

#[bench]
fn bench_insert_vecu8_hashmap(b: &mut Bencher) -> Result<()> {
    let mut map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(key, val);
        });
    });
    Ok(())
}

#[bench]
fn bench_insert_u32_btreemap(b: &mut Bencher) -> Result<()> {
    let mut map: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(42u32, val);
        });
    });
    Ok(())
}

#[bench]
fn bench_insert_u64_btreemap(b: &mut Bencher) -> Result<()> {
    let mut map: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(42u64, val);
        });
    });
    Ok(())
}

#[bench]
fn bench_insert_u128_btreemap(b: &mut Bencher) -> Result<()> {
    let mut map: BTreeMap<u128, Vec<u8>> = BTreeMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(42u128, val);
        });
    });
    Ok(())
}

#[bench]
fn bench_insert_vecu8_btreemap(b: &mut Bencher) -> Result<()> {
    let mut map: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    b.iter(|| {
        let key = gen_key();
        let val = gen_val();
        test::black_box({
            map.insert(key, val);
        });
    });
    Ok(())
}

//-----------------------------------------------------------------------------------------------//
//      Map retrieval
//-----------------------------------------------------------------------------------------------//

#[bench]
fn bench_get_u32_hashmap(b: &mut Bencher) -> Result<()> {
    let val = gen_val();
    let mut map: HashMap<u32, Vec<u8>> = HashMap::new();
    map.insert(42u32, val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&42u32);
        });
    });
    Ok(())
}

#[bench]
fn bench_get_u64_hashmap(b: &mut Bencher) -> Result<()> {
    let val = gen_val();
    let mut map: HashMap<u64, Vec<u8>> = HashMap::new();
    map.insert(42u64, val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&42u64);
        });
    });
    Ok(())
}

#[bench]
fn bench_get_u128_hashmap(b: &mut Bencher) -> Result<()> {
    let val = gen_val();
    let mut map: HashMap<u128, Vec<u8>> = HashMap::new();
    map.insert(42u128, val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&42u128);
        });
    });
    Ok(())
}

#[bench]
fn bench_get_vecu8_hashmap(b: &mut Bencher) -> Result<()> {
    let key = gen_key();
    let val = gen_val();
    let mut map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    map.insert(key.clone(), val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&key);
        });
    });
    Ok(())
}

#[bench]
fn bench_get_u32_btreemap(b: &mut Bencher) -> Result<()> {
    let val = gen_val();
    let mut map: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    map.insert(42u32, val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&42u32);
        });
    });
    Ok(())
}

#[bench]
fn bench_get_u64_btreemap(b: &mut Bencher) -> Result<()> {
    let val = gen_val();
    let mut map: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    map.insert(42u64, val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&42u64);
        });
    });
    Ok(())
}

#[bench]
fn bench_get_u128_btreemap(b: &mut Bencher) -> Result<()> {
    let val = gen_val();
    let mut map: BTreeMap<u128, Vec<u8>> = BTreeMap::new();
    map.insert(42u128, val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&42u128);
        });
    });
    Ok(())
}

#[bench]
fn bench_get_vecu8_btreemap(b: &mut Bencher) -> Result<()> {
    let key = gen_key();
    let val = gen_val();
    let mut map: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    map.insert(key.clone(), val);
    b.iter(|| {
        test::black_box({
            let val_opt = map.get(&key);
        });
    });
    Ok(())
}

