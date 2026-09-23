//! The tile route: what it answers, what it will not answer, and what it cannot read.
//!
//! The archive here is held in memory behind the same `TileArchive` interface the PMTiles reader
//! will implement, so everything the route decides is exercised without an archive file. Requests
//! go through the real wire parser, because the privacy claims are about bytes that arrived from
//! outside, cookies included.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_net::http::{
    encoding::{
        self,
        ContentCoding,
    },
    msg::HttpMessage,
};
use oxedyne_fe2o3_steel::srv::{
    cfg::{
        TileConfig,
        VhostConfig,
    },
    tiles::{
        TileArchive,
        TileInfo,
        TileKind,
        TileRequest,
        TileService,
        TileSource,
    },
};

use std::{
    collections::BTreeMap,
    path::PathBuf,
    pin::Pin,
};

use tokio::io::AsyncWriteExt;


const APP: &str = "https://oxegen.io";
const PLAIN: &[u8] = b"a vector tile, pretending";

struct MemArchive {
    tiles:  BTreeMap<(u8, u32, u32), Vec<u8>>,
    broken: bool,
}

impl TileArchive for MemArchive {
    fn info(&self) -> TileInfo {
        TileInfo {
            kind:       TileKind::Mvt,
            coding:     ContentCoding::Gzip,
            min_zoom:   0,
            max_zoom:   15,
            bounds_e7:  [1_129_000_000, -437_000_000, 1_537_000_000, -106_000_000],
        }
    }
    fn tile(&self, z: u8, x: u32, y: u32) -> Outcome<Option<Vec<u8>>> {
        if self.broken {
            return Err(err!("The test archive is broken on purpose."; Test, IO));
        }
        Ok(self.tiles.get(&(z, x, y)).cloned())
    }
}

fn config() -> TileConfig {
    let mut builds = BTreeMap::new();
    builds.insert("20260922".to_string(), PathBuf::from("/srv/tiles/20260922.pmtiles"));
    TileConfig {
        prefix:         "/t".to_string(),
        current:        "20260922".to_string(),
        builds,
        allow_origins:  vec![APP.to_string()],
        attribution:    "© OpenStreetMap".to_string(),
    }
}

fn service(broken: bool) -> Outcome<TileService<MemArchive>> {
    let stored = res!(encoding::gzip(PLAIN));
    TileService::new(&config(), "tiles.oxegen.io", 63_072_000, |_, _| {
        let mut tiles = BTreeMap::new();
        tiles.insert((14, 13_433, 9_798), stored.clone());
        tiles.insert((3, 7, 4), Vec::new());
        Ok(MemArchive { tiles, broken })
    })
}

async fn parse_request(raw: &str) -> Outcome<HttpMessage> {
    let (mut near, mut far) = tokio::io::duplex(8192);
    let bytes = raw.as_bytes().to_vec();
    tokio::spawn(async move {
        let _ = far.write_all(&bytes).await;
        let _ = far.flush().await;
    });
    match res!(HttpMessage::read::<1024, 1024, _>(
        Pin::new(&mut near), &Vec::new(), Some(true), None).await)
    {
        (Some(msg), _) => Ok(msg),
        (None, _) => Err(err!("The test request did not parse."; Test, Invalid, Input)),
    }
}

// The answer, as the lower-cased head and the body, the way a client would receive it.
struct Answer {
    head: String,
    body: Vec<u8>,
}

impl Answer {
    fn status(&self) -> &str {
        self.head.get(9..12).unwrap_or("")
    }
    fn field(&self, name: &str) -> Option<String> {
        let want = fmt!("{}: ", name);
        self.head.split("\r\n")
            .find(|l| l.starts_with(&want))
            .map(|l| l[want.len()..].to_string())
    }
}

async fn ask(svc: &TileService<MemArchive>, raw: &str) -> Outcome<Option<Answer>> {
    let msg = res!(parse_request(raw).await);
    let req = res!(TileRequest::of(&msg).ok_or_else(|| err!(
        "The test message is not a request."; Test, Invalid)));
    let resp = match svc.respond(req).await {
        Some(r) => r,
        None    => return Ok(None),
    };
    let mut wire: Vec<u8> = Vec::new();
    res!(resp.write_all(&mut wire).await);
    let split = res!(wire.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| err!(
        "The answer has no end of head."; Test, Invalid)));
    Ok(Some(Answer {
        head: String::from_utf8_lossy(&wire[..split]).to_lowercase(),
        body: wire[split + 4..].to_vec(),
    }))
}

async fn must(svc: &TileService<MemArchive>, raw: &str) -> Outcome<Answer> {
    res!(ask(svc, raw).await).ok_or_else(|| err!(
        "The route did not claim a path under its prefix."; Test, Missing))
}

fn get(path: &str, extra: &str) -> String {
    fmt!("GET {} HTTP/1.1\r\nHost: tiles.oxegen.io\r\n{}\r\n", path, extra)
}

const TILE: &str = "/t/20260922/14/13433/9798.mvt";

#[tokio::test]
async fn stored_gzip_is_passed_through_and_cached_for_good() -> Outcome<()> {
    let svc = res!(service(false));
    let a = res!(must(&svc, &get(TILE,
        "Origin: https://oxegen.io\r\nAccept-Encoding: gzip, br\r\n")).await);
    assert_eq!(a.status(), "200", "{}", a.head);
    assert_eq!(a.field("content-encoding").as_deref(), Some("gzip"));
    assert_eq!(a.field("content-type").as_deref(), Some("application/vnd.mapbox-vector-tile"));
    assert_eq!(res!(encoding::gunzip(&a.body)), PLAIN.to_vec(),
        "the stored bytes must reach the client as they are");
    assert_eq!(a.field("cache-control").as_deref(),
        Some("public, max-age=31536000, immutable"));
    assert_eq!(a.field("access-control-allow-origin").as_deref(), Some(APP));
    assert_eq!(a.field("vary").as_deref(), Some("origin, accept-encoding"));
    assert_eq!(a.field("cross-origin-resource-policy").as_deref(), Some("same-site"));
    assert_eq!(a.field("strict-transport-security").as_deref(),
        Some("max-age=63072000; includesubdomains"));
    assert!(a.field("access-control-allow-credentials").is_none());
    Ok(())
}

#[tokio::test]
async fn a_client_refusing_gzip_gets_the_plain_tile() -> Outcome<()> {
    let svc = res!(service(false));
    let a = res!(must(&svc, &get(TILE, "Accept-Encoding: identity\r\n")).await);
    assert_eq!(a.status(), "200", "{}", a.head);
    assert!(a.field("content-encoding").is_none(), "{}", a.head);
    assert_eq!(a.body, PLAIN.to_vec());
    // No Origin: served, but with nothing granting a cross-origin read.
    assert!(a.field("access-control-allow-origin").is_none(), "{}", a.head);
    Ok(())
}

#[tokio::test]
async fn an_empty_tile_is_204_and_cached_like_a_full_one() -> Outcome<()> {
    let svc = res!(service(false));
    for path in ["/t/20260922/3/7/4.mvt", "/t/20260922/5/1/1.mvt"] {
        let a = res!(must(&svc, &get(path, "Origin: https://oxegen.io\r\n")).await);
        assert_eq!(a.status(), "204", "{}: {}", path, a.head);
        assert!(a.body.is_empty());
        assert!(a.field("content-length").is_none(), "{}", a.head);
        assert_eq!(a.field("cache-control").as_deref(),
            Some("public, max-age=31536000, immutable"));
        assert_eq!(a.field("access-control-allow-origin").as_deref(), Some(APP));
    }
    Ok(())
}

#[tokio::test]
async fn paths_naming_no_tile_are_404_and_not_stored() -> Outcome<()> {
    let svc = res!(service(false));
    for path in [
        "/t",
        "/t/",
        "/t/20260922",
        "/t/20260101/14/13433/9798.mvt",    // unknown build
        "/t/20260922/16/0/0.mvt",           // beyond the archive's zooms
        "/t/20260922/3/8/0.mvt",            // off the edge at z3
        "/t/20260922/3/0/8.mvt",
        "/t/20260922/14/13433/9798.png",    // not the archive's kind
        "/t/20260922/14/013433/9798.mvt",   // a second spelling of one tile
        "/t/20260922/14/+13433/9798.mvt",
        "/t/20260922/14/13433/9798",
        "/t/20260922/14/13433/9798.mvt/x",
        "/t/../t/20260922/14/13433/9798.mvt",
        "/t/20260922/99999/0/0.mvt",
    ] {
        let a = res!(must(&svc, &get(path, "")).await);
        assert_eq!(a.status(), "404", "{}: {}", path, a.head);
        assert_eq!(a.field("cache-control").as_deref(), Some("no-store"), "{}", path);
    }
    Ok(())
}

#[tokio::test]
async fn paths_outside_the_prefix_are_not_claimed() -> Outcome<()> {
    let svc = res!(service(false));
    for path in ["/", "/tx/20260922/14/13433/9798.mvt", "/tiles.json", "/index.html"] {
        assert!(res!(ask(&svc, &get(path, "")).await).is_none(), "{} was claimed", path);
    }
    Ok(())
}

#[tokio::test]
async fn an_unlisted_origin_is_refused() -> Outcome<()> {
    let svc = res!(service(false));
    for origin in ["https://evil.example", "http://oxegen.io", "https://oxegen.io.evil.example",
        "null"]
    {
        let a = res!(must(&svc, &get(TILE, &fmt!("Origin: {}\r\n", origin))).await);
        assert_eq!(a.status(), "403", "{}: {}", origin, a.head);
        assert!(a.field("access-control-allow-origin").is_none(), "{}", a.head);
        assert!(a.body.len() < 32, "a refused origin must not be handed the tile");
    }
    Ok(())
}

#[tokio::test]
async fn cookies_and_credentials_change_nothing_and_none_are_set() -> Outcome<()> {
    let svc = res!(service(false));
    let bare = res!(must(&svc, &get(TILE,
        "Origin: https://oxegen.io\r\nAccept-Encoding: gzip\r\n")).await);
    let loaded = res!(must(&svc, &get(TILE,
        "Origin: https://oxegen.io\r\nAccept-Encoding: gzip\r\n\
        Cookie: sid=0123456789abcdef; member=alice\r\n\
        Authorization: Bearer abc.def\r\nX-Member-Id: 42\r\n\
        Referer: https://oxegen.io/map?at=-31.95,115.86\r\n")).await);
    assert_eq!(bare.head, loaded.head, "the request's identity reached the answer");
    assert_eq!(bare.body, loaded.body);
    for a in [&bare, &loaded] {
        assert!(!a.head.contains("set-cookie"), "{}", a.head);
    }
    // And the index, the one answer that is not a tile.
    let idx = res!(must(&svc, &get("/t/tiles.json", "Cookie: sid=1\r\n")).await);
    assert!(!idx.head.contains("set-cookie"), "{}", idx.head);
    Ok(())
}

#[tokio::test]
async fn preflight_head_and_other_methods() -> Outcome<()> {
    let svc = res!(service(false));
    let a = res!(must(&svc, &fmt!(
        "OPTIONS {} HTTP/1.1\r\nHost: tiles.oxegen.io\r\nOrigin: https://oxegen.io\r\n\
        Access-Control-Request-Method: GET\r\n\r\n", TILE)).await);
    assert_eq!(a.status(), "204", "{}", a.head);
    assert_eq!(a.field("access-control-allow-origin").as_deref(), Some(APP));
    assert_eq!(a.field("access-control-allow-methods").as_deref(), Some("get, head, options"));

    let full = res!(must(&svc, &get(TILE, "Accept-Encoding: gzip\r\n")).await);
    let head = res!(must(&svc, &fmt!(
        "HEAD {} HTTP/1.1\r\nHost: tiles.oxegen.io\r\nAccept-Encoding: gzip\r\n\r\n",
        TILE)).await);
    assert_eq!(head.status(), "200");
    assert!(head.body.is_empty(), "a HEAD answer carried a body");
    assert_eq!(head.head, full.head, "a HEAD must say what the GET would");

    let a = res!(must(&svc, &fmt!(
        "POST {} HTTP/1.1\r\nHost: tiles.oxegen.io\r\nContent-Length: 0\r\n\r\n",
        TILE)).await);
    assert_eq!(a.status(), "405", "{}", a.head);
    assert_eq!(a.field("allow").as_deref(), Some("get, head, options"));
    Ok(())
}

#[tokio::test]
async fn the_index_names_the_current_build() -> Outcome<()> {
    let svc = res!(service(false));
    let a = res!(must(&svc, &get("/t/tiles.json", "Origin: https://oxegen.io\r\n")).await);
    assert_eq!(a.status(), "200", "{}", a.head);
    assert_eq!(a.field("cache-control").as_deref(), Some("public, max-age=300"));
    let dat = res!(Dat::decode_string(String::from_utf8_lossy(&a.body).to_string()));
    let m = match dat {
        Dat::Map(m) => m,
        other => return Err(err!("The index is not a map: {:?}", other; Test, Invalid)),
    };
    assert_eq!(m.get(&dat!("build")), Some(&dat!("20260922")));
    assert_eq!(m.get(&dat!("attribution")), Some(&dat!("© OpenStreetMap")));
    let body = String::from_utf8_lossy(&a.body).to_string();
    assert!(body.contains(
        "\"https://tiles.oxegen.io/t/20260922/{z}/{x}/{y}.mvt\""), "{}", body);
    assert!(body.contains("\"maxzoom\":15"), "{}", body);
    Ok(())
}

#[tokio::test]
async fn a_failed_read_is_500_and_not_stored() -> Outcome<()> {
    let svc = res!(service(true));
    let a = res!(must(&svc, &get(TILE, "")).await);
    assert_eq!(a.status(), "500", "{}", a.head);
    assert_eq!(a.field("cache-control").as_deref(), Some("no-store"));
    Ok(())
}

// ── Config ──────────────────────────────────────────────────────────────────

fn vhost(tiles: &str, access_log: &str) -> Outcome<VhostConfig> {
    let text = fmt!("{{\"hostnames\": [\"tiles.oxegen.io\"], {} {}}}", tiles, access_log);
    match res!(Dat::decode_string(text)) {
        Dat::Map(m) => VhostConfig::from_datmap(&m),
        _ => Err(err!("The test config is not a map."; Test, Invalid)),
    }
}

const TILES: &str = "\"tiles\": {\"current\": \"20260922\", \
    \"builds\": {\"20260922\": \"/srv/tiles/20260922.pmtiles\"}, \
    \"allow_origins\": [\"https://oxegen.io\"]},";

#[test]
fn a_tile_vhost_must_turn_its_access_log_off() -> Outcome<()> {
    let e = match vhost(TILES, "") {
        Ok(_) => return Err(err!("A tile vhost with its access log on was accepted."; Test)),
        Err(e) => e,
    };
    assert!(fmt!("{}", e).contains("access_log"), "{}", e);
    assert!(vhost(TILES, "\"access_log\": true").is_err());
    let v = res!(vhost(TILES, "\"access_log\": false"));
    assert!(!v.access_log);
    let t = res!(v.tiles.ok_or_else(|| err!("The tiles block was dropped."; Test, Missing)));
    assert_eq!(t.prefix, "/t");
    assert_eq!(t.attribution, "© OpenStreetMap");
    // A vhost without tiles keeps its log unless told otherwise.
    assert!(res!(vhost("", "")).access_log);
    Ok(())
}

#[test]
fn allow_origins_reads_the_same_in_either_list_shape() -> Outcome<()> {
    let vek = "\"tiles\": {\"current\": \"20260922\", \
        \"builds\": {\"20260922\": \"/srv/tiles/20260922.pmtiles\"}, \
        \"allow_origins\": (vek|[\"https://oxegen.io\", \"https://test.oxegen.io\"])},";
    let v = res!(vhost(vek, "\"access_log\": false"));
    let t = res!(v.tiles.ok_or_else(|| err!("The tiles block was dropped."; Test, Missing)));
    assert_eq!(t.allow_origins, vec!["https://oxegen.io".to_string(), "https://test.oxegen.io".to_string()]);
    let v = res!(vhost(TILES, "\"access_log\": false"));
    let t = res!(v.tiles.ok_or_else(|| err!("The tiles block was dropped."; Test, Missing)));
    assert_eq!(t.allow_origins, vec!["https://oxegen.io".to_string()]);
    Ok(())
}

#[test]
fn tile_config_refuses_what_it_cannot_honour() -> Outcome<()> {
    for (why, tiles) in [
        ("relative path", "\"tiles\": {\"current\": \"a\", \"builds\": {\"a\": \"tiles/a.pmtiles\"}},"),
        ("current absent", "\"tiles\": {\"current\": \"b\", \"builds\": {\"a\": \"/srv/a.pmtiles\"}},"),
        ("no builds", "\"tiles\": {\"current\": \"a\"},"),
        ("wildcard origin", "\"tiles\": {\"current\": \"a\", \"builds\": {\"a\": \"/srv/a.pmtiles\"}, \"allow_origins\": [\"*\"]},"),
        ("origin with path", "\"tiles\": {\"current\": \"a\", \"builds\": {\"a\": \"/srv/a.pmtiles\"}, \"allow_origins\": [\"https://oxegen.io/\"]},"),
        ("slash in build", "\"tiles\": {\"current\": \"a/b\", \"builds\": {\"a/b\": \"/srv/a.pmtiles\"}},"),
        ("dot build", "\"tiles\": {\"current\": \"..\", \"builds\": {\"..\": \"/srv/a.pmtiles\"}},"),
        ("trailing slash prefix", "\"tiles\": {\"prefix\": \"/t/\", \"current\": \"a\", \"builds\": {\"a\": \"/srv/a.pmtiles\"}},"),
    ] {
        assert!(vhost(tiles, "\"access_log\": false").is_err(), "accepted: {}", why);
    }
    Ok(())
}

fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fe2o3_geom/tests/data/protomaps/sample.pmtiles")
}

#[test]
fn a_missing_archive_stops_start_up_and_a_real_one_opens() -> Outcome<()> {
    // The configured path does not exist, so start-up is refused, naming the build.
    let r = TileService::<TileSource>::new(&config(), "tiles.oxegen.io", 0, TileSource::open);
    let e = match r {
        Ok(_) => return Err(err!("An archive that is not there opened."; Test)),
        Err(e) => e,
    };
    assert!(fmt!("{:?}", e).contains("20260922"), "{:?}", e);
    // A real archive, written by the reference Python writer, opens and says what it holds.
    let src = res!(TileSource::open("sample", &sample()));
    let info = src.info();
    assert_eq!(info.kind, TileKind::Mvt);
    assert_eq!(info.coding, ContentCoding::Gzip);
    assert_eq!((info.min_zoom, info.max_zoom), (12, 15));
    assert_eq!(info.bounds_e7, [1_156_000_000, -326_000_000, 1_163_000_000, -316_000_000]);
    let stored = res!(src.tile(13, 6729, 4865));
    let plain = res!(encoding::gunzip(&res!(stored.ok_or_else(|| err!("No tile."; Test)))));
    let want = res!(std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fe2o3_geom/tests/data/protomaps/13_6729_4865.mvt")));
    assert_eq!(plain, want);
    assert_eq!(res!(src.tile(13, 0, 0)), None);
    Ok(())
}
