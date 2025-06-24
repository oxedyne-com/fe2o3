use oxedyne_fe2o3_core::{
    debug,
    rand::Rand,
};

use criterion::{
    criterion_group,
    criterion_main,
    Criterion,
    BenchmarkId,
    Throughput,
};

fn vu64_to_bytes_baseline(input: (Vec<u8>, Vec<u64>)) {
    let (mut buf, seq) = input;
    for num in seq {
        let byts: [u8; 8] = num.to_be_bytes();
        let mut count: u8 = 8;
        for byt in &byts[..] {
            if *byt == 0 {
                count -= 1;
            } else {
                break;
            }
        }
        buf.push(count);
        while count > 0 {
            buf.push(byts[(8-count) as usize]);
            count -= 1;
        }
    }
}

fn vu64_to_bytes_alt_1(input: (Vec<u8>, Vec<u64>)) {
    let (mut buf, seq) = input;
    for num in seq {
        let byts: [u8; 8] = num.to_be_bytes();
        let mut count: usize = 8;
        for byt in &byts[..] {
            if *byt == 0 {
                count -= 1;
            } else {
                break;
            }
        }
        buf.push(count as u8);
        while count > 0 {
            buf.push(byts[8-count]);
            count -= 1;
        }
    }
}

fn vu64_to_bytes_alt_2(input: (Vec<u8>, Vec<u64>)) {
    let (mut buf, seq) = input;
    for num in seq {
        let byts: [u8; 8] = num.to_be_bytes();
        let mut c_zeros: u8 = 0;
        let mut mask: u64 = 0x_FF_FF_FF_FF_FF_FF_FF_FF;
        // example
        // c = 0
        //   MSB     big endian    LSB
        // 0x_00_00_00_FF_FF_FF_FF_FF num
        // 0x_FF_FF_FF_FF_FF_FF_FF_FF mask
        // 0x_FF_FF_FF_00_00_00_00_00 xor
        // c = 1
        // 0x_00_00_00_FF_FF_FF_FF_FF num
        // 0x_00_FF_FF_FF_FF_FF_FF_FF mask
        // 0x_00_FF_FF_00_00_00_00_00 xor
        // c = 2
        // 0x_00_00_00_FF_FF_FF_FF_FF num
        // 0x_00_00_FF_FF_FF_FF_FF_FF mask
        // 0x_00_00_FF_00_00_00_00_00 xor
        // c = 3
        // 0x_00_00_00_FF_FF_FF_FF_FF num
        // 0x_00_00_00_FF_FF_FF_FF_FF mask
        // 0x_00_00_00_00_00_00_00_00 xor
        //
        // xor = 0 so exit with c = 3
        //
        while c_zeros > 0 && (num ^ mask) != 0 {
            mask = mask >> 8;
            c_zeros += 1;
        }
        buf.push(8-c_zeros);
        //buf.extend_from_slice(&byts[(c_zeros as usize)..8]);
        while c_zeros < 8 {
            buf.push(byts[c_zeros as usize]);
            c_zeros += 1;
        }
    }
}

fn bench_vu64_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("C64 encoding");

    let mut seq = vec![0u64];
    for _ in 0..10000 {
        seq.push(Rand::rand_u64() & 0x_00_FF_FF_FF_FF_FF_FF_FF);
    }
    debug!("Length of input sequence: {}", seq.len());

    group.throughput(Throughput::Elements(seq.len() as u64));

    let seq0 = seq.clone();
    let seq1 = seq.clone();
    let seq2 = seq.clone();
    let mut buf: Vec<u8> = Vec::new();
    group.bench_with_input(
        BenchmarkId::new("Baseline C64 encoding", seq0.len()),
        &(buf, seq0),
        |b, (buf, seq0)| {
            b.iter(|| vu64_to_bytes_baseline((buf.to_vec(), seq0.to_vec())));
        }
    );
    let mut buf: Vec<u8> = Vec::new();
    group.bench_with_input(
        BenchmarkId::new("Alternative #1 C64 encoding", seq1.len()),
        &(buf, seq1),
        |b, (buf, seq1)| {
            b.iter(|| vu64_to_bytes_alt_1((buf.to_vec(), seq1.to_vec())));
        }
    );
    let mut buf: Vec<u8> = Vec::new();
    group.bench_with_input(
        BenchmarkId::new("Alternative #2 C64 encoding", seq2.len()),
        &(buf, seq2),
        |b, (buf, seq2)| {
            b.iter(|| vu64_to_bytes_alt_2((buf.to_vec(), seq2.to_vec())));
        }
    );
    
    group.finish();
}

criterion_group!(benches, bench_vu64_encoding);
criterion_main!(benches);
