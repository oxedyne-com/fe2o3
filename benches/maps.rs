use std::alloc;
use cap::Cap;

#[global_allocator]
static ALLOCATOR: Cap<alloc::System> = Cap::new(alloc::System, usize::max_value());

use oxedyne_fe2o3_sand::treemap::{
    ByteKey,
    TreeMap,
};
use oxedyne_fe2o3_core::{debug};
use oxedyne_fe2o3_core::data::{Stack};

use criterion::{
    criterion_group,
    criterion_main,
    Criterion,
    BenchmarkId,
    Throughput,
};

use std::collections::{
    BTreeMap,
    HashMap,
};

use rand::Rng;
use rand::distributions::{
    Distribution,
    Uniform,
};

fn create_keys(n: usize, len: usize) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    for _ in 0..n {
        result.push((0..len).map(|_| { rand::random::<u8>() }).collect());
    }
    result
}

fn create_data(len: usize) -> Vec<u8> {
    (0..len).map(|_| { rand::random::<u8>() }).collect()
}

fn hashmap_insertions<'a>(input: (&'a Vec<Vec<u8>>, &'a Vec<u8>)) -> (HashMap<Vec<u8>, &'a Vec<u8>>, usize) {
    let (keys, data) = input;
    let mem = ALLOCATOR.allocated();
    let mut map = HashMap::new();
    for key in keys {
        map.insert(key.clone(), data);
    }
    (map, ALLOCATOR.allocated() - mem)
}

fn hashmap_reads<'a>(input: (&'a Vec<Vec<u8>>, &'a Vec<u8>), map: HashMap<Vec<u8>, &'a Vec<u8>>) {
    let (keys, _) = input;
    for key in keys {
        map.get(&key.clone());
    }
}

fn btreemap_insertions<'a>(input: (&'a Vec<Vec<u8>>, &'a Vec<u8>)) -> (BTreeMap<Vec<u8>, &'a Vec<u8>>, usize) {
    let (keys, data) = input;
    let mem = ALLOCATOR.allocated();
    let mut map = BTreeMap::new();
    for key in keys {
        map.insert(key.clone(), data);
    }
    (map, ALLOCATOR.allocated() - mem)
}

fn btreemap_reads<'a>(input: (&'a Vec<Vec<u8>>, &'a Vec<u8>), map: BTreeMap<Vec<u8>, &'a Vec<u8>>) {
    let (keys, _) = input;
    for key in keys {
        map.get(&key.clone());
    }
}

fn treemap_insertions<'a>(input: (&'a Vec<Vec<u8>>, &'a Vec<u8>)) -> (TreeMap<Stack<&'a Vec<u8>>>, usize) {
    let (keys, data) = input;
    let mem = ALLOCATOR.allocated();
    let mut map = TreeMap::new();
    for key in keys {
        map.insert_data(ByteKey::new(key.clone()), data);
    }
    (map, ALLOCATOR.allocated() - mem)
}

fn treemap_reads<'a>(input: (&'a Vec<Vec<u8>>, &'a Vec<u8>), map: TreeMap<Stack<&'a Vec<u8>>>) {
    let (keys, _) = input;
    for key in keys {
        map.get_data_ref(ByteKey::new(key.clone()));
    }
}

fn bench_map_insertions(c: &mut Criterion) {
    // Set the limit to 500 [MiB].
    debug!("*********************************************");
    debug!("*         BENCHMARK MAP INSERTIONS          *");
    debug!("*********************************************");
    ALLOCATOR.set_limit(500 * 1024 * 1024).unwrap();
    debug!("Currently allocated: {} [B]", ALLOCATOR.allocated());
    static N: usize = 1000;
    let mut group = c.benchmark_group("Maps");

    for len in [10, 50, 100].iter() {
        let keys = create_keys(N, *len);
        let data = create_data(10);

        // Estimate memory usage
        debug!("Insertions: {}, key length: {}", N, len);
        let (hashmap, mem) = hashmap_insertions((&keys, &data));
        debug!("std::collections::HashMap memory: {} [KB]", mem/1024);
        let (btreemap, mem) = btreemap_insertions((&keys, &data));
        debug!("std::collections::BTreeMap memory: {} [KB]", mem/1024);
        let (treemap, mem) = treemap_insertions((&keys, &data));
        debug!("fe2o3::treemap::TreeMap memory: {} [KB]", mem/1024);

        group.throughput(Throughput::Bytes(*len as u64));
        group.bench_with_input(
            BenchmarkId::new("std::collections::HashMap", len),
            &(&keys, &data),
            |b, (keys, data)| {
                b.iter(|| hashmap_insertions((&keys, &data)));
            }
        );
        group.bench_with_input(
            BenchmarkId::new("std::collections::BTreeMap", len),
            &(&keys, &data),
            |b, (keys, data)| {
                b.iter(|| btreemap_insertions((&keys, &data)));
            }
        );
        group.bench_with_input(
            BenchmarkId::new("fe2o3::treemap::TreeMap", len),
            &(&keys, &data),
            |b, (keys, data)| {
                b.iter(|| treemap_insertions((&keys, &data)));
            }
        );
    }
    group.finish();
}

fn bench_map_retrievals(c: &mut Criterion) {
    // Set the limit to 500 [MiB].
    debug!("*********************************************");
    debug!("*         BENCHMARK MAP RETRIEVALS          *");
    debug!("*********************************************");
    ALLOCATOR.set_limit(500 * 1024 * 1024).unwrap();
    debug!("Currently allocated: {} [B]", ALLOCATOR.allocated());
    static N: usize = 1000;
    let mut group = c.benchmark_group("Maps");

    for len in [6, 10, 50, 100].iter() {
        let keys = create_keys(N, *len);
        let data = create_data(10);

        // Estimate memory usage
        debug!("Retrievals: {}, key length: {}", N, len);
        let (mut hashmap, mem) = hashmap_insertions((&keys, &data));
        debug!("std::collections::HashMap memory: {} [KB]", mem/1024);
        let (mut btreemap, mem) = btreemap_insertions((&keys, &data));
        debug!("std::collections::BTreeMap memory: {} [KB]", mem/1024);
        let (mut treemap, mem) = treemap_insertions((&keys, &data));
        debug!("fe2o3::treemap::TreeMap memory: {} [KB]", mem/1024);

        group.throughput(Throughput::Bytes(*len as u64));
        group.bench_with_input(
            BenchmarkId::new("std::collections::HashMap", len),
            &(&keys, &data),
            |b, (keys, data)| {
                b.iter(|| hashmap_reads(
                        (&keys, &data),
                        std::mem::replace(
                            &mut hashmap,
                            HashMap::new(),
                        ),
                ));
            }
        );
        group.bench_with_input(
            BenchmarkId::new("std::collections::BTreeMap", len),
            &(&keys, &data),
            |b, (keys, data)| {
                b.iter(|| btreemap_reads(
                        (&keys, &data),
                        std::mem::replace(
                            &mut btreemap,
                            BTreeMap::new(),
                        ),
                ));
            }
        );
        group.bench_with_input(
            BenchmarkId::new("fe2o3::treemap::TreeMap", len),
            &(&keys, &data),
            |b, (keys, data)| {
                b.iter(|| treemap_reads(
                        (&keys, &data),
                        std::mem::replace(
                            &mut treemap,
                            TreeMap::new(),
                        ),
                ));
            }
        );
    }
    group.finish();
}

criterion_group!(benches, bench_map_insertions, bench_map_retrievals);
criterion_main!(benches);
