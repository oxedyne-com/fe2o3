use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
    rand::Rand,
};
use oxedyne_fe2o3_hash::{
    hash::HashScheme,
    map::ShardMap,
};
use oxedyne_fe2o3_iop_hash::{
    api::{
        Hasher,
        HashForm,
    },
};

use std::{
    collections::BTreeMap,
    fmt::Debug,
    thread,
};

const MAX_SHARDS:           usize = 20;
const N1:                   u32 = 4;
const N2:                   u32 = 13;
const N_THREADS:            usize = 5;
const KEY_LEN:              usize = 4;
const VALS_PER_THREAD:      u8 = 10;
const SALT_SIZE:            usize = 5;
const SALT:                 [u8; SALT_SIZE] = [1u8, 2, 3, 4, 5];
const STACK_SIZE:           usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TestStruct {
    ord: u8,
    thread: usize,
}

fn dump_shards<
    const C: usize,
    const S: usize,
    V: Clone + Debug,
    M: MapMut<HashForm, V> + Clone + Debug,
    H: Hasher + Send + Sync + 'static,
>(
    shardmap: &ShardMap<C, S, V, M, H>,
) {
    for i in 0..shardmap.n {
        if let Some(locked_map) = &shardmap.shards[i] {
            let unlocked_map = locked_map.read().unwrap();
            debug!("Map {}, len {}", i, unlocked_map.len());
            for (k, v) in (*unlocked_map).iter() {
                debug!(" k={:02x?} v={:?}", k, v);
            }
        }
    }
}

fn doer<
    const C: usize,
    const S: usize,
    M: MapMut<HashForm, TestStruct> + Clone + Debug,
    H: Hasher + Send + Sync + 'static,
>(
    t: usize,
    shardmap: Arc<ShardMap<C, S, TestStruct, M, H>>,
)
    -> Outcome<Vec<(Vec<u8>, TestStruct)>>
{
    let mut data = Vec::new();
    for j in 0..VALS_PER_THREAD {
        let mut p = [0u8; KEY_LEN];
        Rand::fill_u8(&mut p);
        let v = TestStruct { ord: j, thread: t };
        res!((*shardmap).insert(&p, v.clone()));
        data.push((p.to_vec(), v));
    }
    test!("Thread {} finished doing.", t);
    Ok(data)
}

pub fn test_map(filter: &'static str) -> Outcome<()> {

    match filter {
        "all" | "shardmap" => {
            let shardmap = Arc::new(res!(ShardMap::<MAX_SHARDS, SALT_SIZE, _, _, _>::new(
                N1,
                SALT,
                BTreeMap::<_, TestStruct>::new(),
                HashScheme::new_seahash(),
            )));
            let mut handles = Vec::new();
            for t in 0..N_THREADS {
                let builder = thread::Builder::new()
                    .name(fmt!("maps_test"))
                    .stack_size(STACK_SIZE);
                let shardmap_clone = shardmap.clone();
                handles.push(res!(builder.spawn( move || doer(t, shardmap_clone) )));
            }

            let mut data = Vec::new();
            for handle in handles {
                let thread_data = handle.join().unwrap().unwrap(); // too much effort to handle errors
                for pair in thread_data {
                    data.push(pair);
                }
            }

            dump_shards(&shardmap);
            for (p, v) in data {
                let locked_map = res!(shardmap.get_shard(&p));
                let unlocked_map = lock_read!(locked_map);
                req!(unlocked_map.get(&res!(shardmap.key(&p))), Some(&v));
            }
            let maps2 = res!(shardmap.remap(N2));
            test!("Remap from {} to {} shards...", N1, N2);
            dump_shards(&maps2);
        },
        _ => (),
    }

    Ok(())
}
