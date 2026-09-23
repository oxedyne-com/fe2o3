//! A PMTiles archive read over HTTP by range, from a local server holding only the byte ranges
//! the reference `pmtiles` reader read from the Protomaps planet (see
//! `fe2o3_geom/tests/data/protomaps/NOTICE.md`).

use oxedyne_fe2o3_geom::tile::pmtiles::{
    Archive,
    RangeSource,
};
use oxedyne_fe2o3_net::http::range_source::HttpRangeSource;

use oxedyne_fe2o3_core::prelude::*;

use std::{
    io::{
        Read,
        Write,
    },
    net::TcpListener,
    path::PathBuf,
    thread,
};

fn ranges() -> Outcome<Vec<(u64, Vec<u8>)>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fe2o3_geom/tests/data/protomaps/ocean_ranges.bin");
    let b = res!(std::fs::read(&path), File, Read);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 12 <= b.len() {
        let mut o = [0u8; 8];
        o.copy_from_slice(&b[i..i + 8]);
        let n = u32::from_le_bytes([b[i + 8], b[i + 9], b[i + 10], b[i + 11]]) as usize;
        out.push((u64::from_le_bytes(o), b[i + 12..i + 12 + n].to_vec()));
        i += 12 + n;
    }
    Ok(out)
}

/// Serves `n` requests: a `Range` within the known ranges is answered `206` on `/planet`, and
/// `/whole` answers `200` with a body, as a server ignoring `Range` does.
fn serve(n: usize) -> Outcome<u16> {
    let data = res!(ranges());
    let listener = res!(TcpListener::bind("127.0.0.1:0"), Network, Init);
    let port = res!(listener.local_addr(), Network, Init).port();
    thread::spawn(move || {
        for _ in 0..n {
            let (mut s, _) = match listener.accept() {
                Ok(c) => c,
                Err(_) => return,
            };
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => req.extend_from_slice(&buf[..k]),
                }
            }
            let text = String::from_utf8_lossy(&req).to_string();
            let whole = text.starts_with("GET /whole ");
            let range = text.lines()
                .find_map(|l| l.strip_prefix("Range: bytes="))
                .and_then(|r| r.split_once('-'))
                .and_then(|(a, b)| Some((a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?)));
            let reply = match (whole, range) {
                (true, _) => {
                    let body = vec![7u8; 64];
                    let mut r = fmt!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
                    r.extend_from_slice(&body);
                    r
                },
                (false, Some((a, b))) => {
                    let hit = data.iter().find(|(o, d)| a >= *o && b < *o + d.len() as u64);
                    match hit {
                        Some((o, d)) => {
                            let body = &d[(a - o) as usize..=(b - o) as usize];
                            let mut r = fmt!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\n\
                                Content-Range: bytes {}-{}/*\r\n\r\n", body.len(), a, b).into_bytes();
                            r.extend_from_slice(body);
                            r
                        },
                        None => b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\r\n".to_vec(),
                    }
                },
                (false, None) => b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n".to_vec(),
            };
            let _ = s.write_all(&reply);
        }
    });
    Ok(port)
}

fn fnv1a64(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    fmt!("{:016x}", h)
}

#[test]
fn test_an_archive_reads_over_http_ranges_00() -> Outcome<()> {
    // Header, root, one leaf and one tile for the first; the second's leaf and the shared tile.
    let port = res!(serve(8));
    let src = res!(HttpRangeSource::new(&fmt!("http://127.0.0.1:{}/planet", port), None));
    let a = res!(Archive::open(src));
    req!(a.header().max_zoom, 15);
    // The two open-ocean tiles, and the reference reader's reading of them.
    for (z, x, y) in [(12u8, 2958u32, 2545u32), (15, 5152, 21497)] {
        let t = res!(res!(a.tile_decoded(z, x, y)).ok_or_else(|| err!("{}/{}/{} is missing.", z, x, y; Test)));
        req!(t.len(), 75);
        req!(fnv1a64(&t), "b36d34914f4c291c".to_string(), "{}/{}/{} read other bytes.", z, x, y);
    }
    Ok(())
}

#[test]
fn test_a_server_ignoring_range_is_refused_01() -> Outcome<()> {
    let port = res!(serve(1));
    let src = res!(HttpRangeSource::new(&fmt!("http://127.0.0.1:{}/whole", port), None));
    let refused = src.read(0, 16).is_err();
    req!(refused, true, "A 200 with the whole file was read as a range.");
    req!(HttpRangeSource::new("ftp://example.com/x", None).is_err(), true, "An ftp URL was taken.");
    req!(HttpRangeSource::new("https://example.com/x", None).is_err(), true,
        "An https URL was taken with no TLS configuration.");
    Ok(())
}
