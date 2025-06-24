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
use oxedyne_fe2o3_jdat::{
    prelude::*,
    daticle::Dat,
};

use test::Bencher;

//const SMALL_LEN: usize = 10;
//const LARGE_LEN: usize = 1000;
//const FILL_U8: u8 = 42;
//const FILL_U64: u64 = 42;
//
//fn u8_to_bytes(v: u8, buf: &mut Vec<u8>) {
//    buf.push(Dat::U8_CODE);
//    buf.extend_from_slice(&v.to_be_bytes());
//}
//
//fn u8_to_bytes2(v: u8, mut buf: Vec<u8>) -> Vec<u8> {
//    buf.push(Dat::U8_CODE);
//    buf.push(v.to_be_bytes()[0]);
//    buf
//}
//
//fn u64_to_bytes(v: u64, buf: &mut Vec<u8>) {
//    buf.push(Dat::U64_CODE);
//    buf.extend_from_slice(&v.to_be_bytes());
//}
//
//fn u64_to_bytes2(v: u64, mut buf: Vec<u8>) -> Vec<u8> {
//    buf.push(Dat::U64_CODE);
//    buf.append(&mut v.to_be_bytes().to_vec());
//    buf
//}
//
//fn u64_to_bytes3(v: u64, buf: &mut Vec<u8>) {
//    buf.push(Dat::U64_CODE);
//    for b in v.to_be_bytes() {
//        buf.push(b);
//    }
//}
//
//fn u64_to_bytes4(v: u64, mut buf: Vec<u8>) -> Vec<u8> {
//    let mut pre = vec![0u8;9];
//    pre[0] = Dat::U64_CODE;
//    let vbyts = v.to_be_bytes();
//    for i in 0..vbyts.len() {
//        pre[1 + i] = vbyts[i];
//    }
//    buf.append(&mut pre);
//    buf
//}
//
//fn c64_to_bytes(v: u64, buf: &mut Vec<u8>) {
//    let byts: [u8; 8] = v.to_be_bytes();
//    let mut count: usize = 8;
//    for byt in &byts[..] {
//        if *byt == 0 {
//            count -= 1;
//        } else {
//            break;
//        }
//    }
//    buf.push(Dat::C64_CODE_START + count as u8);
//    while count > 0 {
//        buf.push(byts[8-count]);
//        count -= 1;
//    }
//}
//
//fn daticle_bytes(b: &Vec<u8>, mut buf: &mut Vec<u8>) {
//    buf.push(Dat::BC64_CODE);
//    buf = Dat::C64(b.len() as u64).to_bytes(buf);
//    buf.extend_from_slice(b);
//}
//
//fn daticle_bytes2(mut b: Vec<u8>, mut buf: Vec<u8>) -> Vec<u8> {
//    buf.push(Dat::BU64_CODE);
//    //Dat::C64(b.len() as u64).to_bytes(&mut buf);
//    buf.extend_from_slice(&(b.len() as u64).to_be_bytes());
//    buf.append(&mut b);
//    buf
//}
//
//fn daticle_bytes3(b: &Vec<u8>, buf: &mut Vec<u8>) {
//    buf.push(Dat::BU64_CODE);
//    buf.extend_from_slice(&(b.len() as u64).to_be_bytes());
//    buf.extend_from_slice(b);
//}
//
//fn wrap_bytes(byts: &mut Vec<u8>) {
//    let mut pre = Vec::new();
//    pre.push(Dat::BC64_CODE);
//    Dat::C64(byts.len() as u64).to_bytes(&mut pre);
//    byts.splice(0..0, pre.drain(..));
//}
//
//fn wrap_bytes2(mut byts: Vec<u8>) -> Vec<u8> {
//    let mut pre = Vec::new();
//    pre.push(Dat::BC64_CODE);
//    Dat::C64(byts.len() as u64).to_bytes(&mut pre);
//    pre.append(&mut byts);
//    pre
//}
//
//#[bench]
//fn bench_small_init(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            vec![FILL_U8; SMALL_LEN]
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_large_init(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            vec![FILL_U8; LARGE_LEN]
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u8_to_bytes_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u8_to_bytes(FILL_U8, &mut vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u8_to_bytes2_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u8_to_bytes2(FILL_U8, vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u64_to_bytes_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u64_to_bytes(FILL_U64, &mut vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u64_to_bytes2_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u64_to_bytes2(FILL_U64, vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u8_to_bytes_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u8_to_bytes(FILL_U8, &mut vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u8_to_bytes2_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u8_to_bytes2(FILL_U8, vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u64_to_bytes_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u64_to_bytes(FILL_U64, &mut vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u64_to_bytes2_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u64_to_bytes2(FILL_U64, vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u64_to_bytes3_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u64_to_bytes3(FILL_U64, &mut vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_u64_to_bytes4_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            u64_to_bytes4(FILL_U64, vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_daticle_bytes_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            daticle_bytes(&vec![FILL_U8; SMALL_LEN], &mut vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_daticle_bytes2_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            daticle_bytes2(vec![FILL_U8; SMALL_LEN], vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_daticle_bytes3_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            daticle_bytes3(&vec![FILL_U8; SMALL_LEN], &mut vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_daticle_bytes_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            daticle_bytes(&vec![FILL_U8; LARGE_LEN], &mut vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_daticle_bytes2_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            daticle_bytes2(vec![FILL_U8; LARGE_LEN], vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_daticle_bytes3_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            daticle_bytes3(&vec![FILL_U8; LARGE_LEN], &mut vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_wrap_bytes_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            wrap_bytes(&mut vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_wrap_bytes2_small(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            wrap_bytes2(vec![FILL_U8; SMALL_LEN])
//        );
//    });
//    Ok(())
//}
//
//
//#[bench]
//fn bench_wrap_bytes_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            wrap_bytes(&mut vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_wrap_bytes2_large(b: &mut Bencher) -> Result<()> {
//    b.iter(|| {
//        test::black_box(
//            wrap_bytes2(vec![FILL_U8; LARGE_LEN])
//        );
//    });
//    Ok(())
//}
//
//fn vec_to_bytes(v: &Vec<Dat>, buf: &mut Vec<u8>) {
//    let mut buf2 = Vec::new();
//    for item in v {
//        item.to_bytes(&mut buf2);
//    }
//    buf.push(Dat::LIST_CODE);
//    Dat::C64(buf2.len() as u64).to_bytes(buf);
//    buf.extend_from_slice(&buf2);
//}
//
//fn vec_to_bytes2(v: &Vec<Dat>, mut buf: Vec<u8>) -> Vec<u8> {
//    let mut buf2 = Vec::new();
//    for item in v {
//        item.to_bytes(&mut buf2);
//    }
//    buf.push(Dat::LIST_CODE);
//    Dat::C64(buf2.len() as u64).to_bytes(&mut buf);
//    buf.append(&mut buf2);
//    buf
//}
//
//#[bench]
//fn bench_encode_list(b: &mut Bencher) -> Result<()> {
//    let v = vec![ listdat![
//        vec![42;1000], // 1
//        vec![42;1000], // 2
//        vec![42;1000], // 3
//        vec![42;1000], // 4
//        vec![42;1000], // 5
//        vec![42;1000], // 6
//        vec![42;1000], // 7
//        vec![42;1000], // 8
//        vec![42;1000], // 9
//        vec![42;1000], // 10
//    ]];
//    let mut buf = Vec::new();
//    b.iter(|| {
//        test::black_box(
//            vec_to_bytes(&v, &mut buf)
//        );
//    });
//    Ok(())
//}
//
//#[bench]
//fn bench_encode_list2(b: &mut Bencher) -> Result<()> {
//    let v = vec![ listdat![
//        vec![42;1000], // 1
//        vec![42;1000], // 2
//        vec![42;1000], // 3
//        vec![42;1000], // 4
//        vec![42;1000], // 5
//        vec![42;1000], // 6
//        vec![42;1000], // 7
//        vec![42;1000], // 8
//        vec![42;1000], // 9
//        vec![42;1000], // 10
//    ]];
//    b.iter(|| {
//        test::black_box(
//            vec_to_bytes2(&v, Vec::new())
//        );
//    });
//    Ok(())
//}

fn test_vec() -> Vec<u8> {
    vec![42; 200]
}

#[bench]
fn bench_wrap_bytes_placebo(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box(
            test_vec()
        );
    });
    Ok(())
}

#[bench]
fn bench_wrap_bytes2(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box(
            Dat::wrap_bytes2(test_vec())
        );
    });
    Ok(())
}

#[bench]
fn bench_wrap_bytes(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box(
            Dat::wrap_bytes(test_vec())
        );
    });
    Ok(())
}

#[bench]
fn bench_wrap_bytes_c64(b: &mut Bencher) -> Result<()> {
    b.iter(|| {
        test::black_box(
            Dat::wrap_bytes_c64(test_vec())
        );
    });
    Ok(())
}
