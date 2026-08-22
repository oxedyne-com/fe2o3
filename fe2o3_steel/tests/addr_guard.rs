//! The address guard, asked by a real Steel over a real socket.
//!
//! # Why this is not a unit test
//!
//! `guard.rs` builds the guard, `cfg.rs` parses its settings, and
//! `fe2o3_net::guard::addr` holds the state machine, and every one of those has
//! tests of its own. None of them can say whether a running Steel *asks* it. That
//! question has had a wrong answer here before: `RingTimer::update` wrote its
//! timestamp into a `Copy` of the ring, so the rate was always zero and a guard
//! that read as configured refused nothing whatever. A limiter that records
//! nothing is indistinguishable from one that works, right up until the day it
//! matters.
//!
//! So a Steel is started, and connections are made to it, and what is asserted is
//! what a caller on the far side of the socket can see.
//!
//! # Where the guard sits, and what that means
//!
//! [`srv::server`] checks it in the TCP accept loop -- before the TLS handshake,
//! before the SNI is read, before any vhost is chosen. Two things follow, and
//! both matter to an operator:
//!
//! - It covers **every** vhost, proxied ones included. There is no per-vhost
//!   configuration of it and no way for a vhost to be missed.
//! - It counts **connections**, not requests. A client that keeps one connection
//!   alive and sends a thousand requests down it is one event to this guard. What
//!   that is worth depends on the route: Steel answers a proxied route with
//!   `Connection: close`, so there the two are the same number, while a static
//!   vhost keeps the connection and there they are not.
//!
//! The second point is why a per-request limiter still earns its place upstream
//! of an application, and why `auth_path_prefixes` exists as a second tier that
//! *is* consulted per request.
//!
//! # A guard that refuses a browser is worse than none
//!
//! One test here floods and one does not, and the second is the one that would
//! cost more if it were wrong: a page load opens several connections at once, and
//! a threshold that treats that as an attack breaks every visitor rather than
//! stopping one.
//!
//! Unix only, for the same reason as `stopping_signal.rs`: the fixture runs a
//! real server as a child process.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

#![cfg(unix)]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::keystore::{
    DEFAULT_WALLET_KDF_NAME,
    Wallet,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    file::JdatFile,
    string::enc::EncoderConfig,
};
use oxedyne_fe2o3_steel::{
    app::constant as app_const,
    srv::admin::guard::{
        DEFAULT_RPS_MAX,
        GUARD_RING,
    },
};

use std::{
    collections::BTreeMap,
    io::Read,
    net::{
        IpAddr,
        Ipv4Addr,
        SocketAddr,
        TcpListener,
        TcpStream,
    },
    path::{
        Path,
        PathBuf,
    },
    process::{
        Child,
        Command,
        Stdio,
    },
    time::{
        Duration,
        Instant,
    },
};

// The wallet passphrase, in the clear and on purpose: it protects a wallet that
// exists for a few seconds in a scratch directory and holds one key to one empty
// database. `stopping_signal.rs` beside this says the same thing for the same
// reason.
const PASS: &str = "steel-addr-guard-test-passphrase-not-a-secret"; // allowlist secret

const APP: &str = "addrguard";

const START_SECS: u64 = 120;

// How long the whole burst is given to settle before any of it is read. Every
// connection the guard refused has had its close queued by then, so a read that
// blocks after this is a connection the server is still holding.
const SETTLE_MS: u64 = 500;

// How long one settled connection is given to say something. Short on purpose:
// the server has already decided by now, so this is the cost of asking and not a
// wait for an answer. It must not be long enough to matter, because a burst read
// one socket at a time at 400 ms each was what made the first version of this
// test open its connections at two and a half a second and conclude, wrongly,
// that a guard set to fifty a second had never fired.
const ASK_MS: u64 = 40;

// A browser's worst honest moment: a page whose assets open this many
// connections at once. Under `GUARD_RING`, so the rate machine has not even
// filled its ring, which is the arithmetic the assertion rests on.
const BROWSERS_BURST: usize = 24;


/// One connection's fate, as the far end sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fate {
    Kept,       // the server is holding it open, waiting for a handshake
    Dropped,    // closed with nothing said, which is what the guard does
    Refused,    // the connection never opened at all
}


#[test]
fn test_the_guard_refuses_a_flood_of_connections_00() -> Outcome<()> {
    let dir = res!(scratch("flood"));
    let port = res!(free_port());
    res!(lay_out(&dir, port));
    let (child, probes) = res!(serve_fixture(&dir, port));

    // Far past the ring, as fast as loopback allows, which is thousands a second
    // against a ceiling of `DEFAULT_RPS_MAX`.
    let fates = res!(burst(port, GUARD_RING * 4));
    put_down(child);

    let dropped = fates.iter().filter(|f| **f == Fate::Dropped).count();
    let kept = fates.iter().filter(|f| **f == Fate::Kept).count();
    // Asserted as a count rather than as "no error", because a guard that
    // recorded nothing would also raise no error. The number is what tells a
    // working guard from an ornamental one.
    assert!(dropped > 0,
        "{} connections were opened as fast as a socket allows, against a ceiling \
        of {} a second, and every one of them was served. The guard is either not \
        consulted in the accept path or is not counting.",
        fates.len(), DEFAULT_RPS_MAX);
    // And it did not start refusing before it had anything to go on. `avg_rps`
    // reports zero until the ring is full, so a ring's worth must be served
    // first; a guard refusing sooner than that is refusing on no evidence.
    //
    // The probes count. Waiting for the server to bind opens a connection each
    // time round, and those are connections from this address like any other --
    // which is how this assertion first failed, at 63 served against a ring of
    // 64, and it was right to.
    assert!(kept + probes >= GUARD_RING,
        "{} connections were served, after {} while waiting for the port, and the \
        guard's ring holds {}. It cannot have measured a rate on fewer than a \
        ring, so it is refusing on nothing.", kept, probes, GUARD_RING);
    Ok(())
}

#[test]
fn test_a_page_load_is_not_a_flood_01() -> Outcome<()> {
    let dir = res!(scratch("browser"));
    let port = res!(free_port());
    res!(lay_out(&dir, port));
    let (child, _probes) = res!(serve_fixture(&dir, port));

    // A browser opening every connection a page load would, back to back and with
    // no pause, which is faster than any browser really manages.
    let fates = res!(burst(port, BROWSERS_BURST));
    put_down(child);

    let dropped = fates.iter().filter(|f| **f == Fate::Dropped).count();
    assert_eq!(dropped, 0,
        "{} of a page load's {} connections were dropped by the address guard. A \
        limit that refuses a browser fetching one page has cost more than any \
        attacker would.", dropped, BROWSERS_BURST);
    Ok(())
}


/// Opens `n` connections as fast as the socket allows, then reports what the
/// server did with each.
///
/// The two halves are apart for a reason worth keeping. The whole test is about a
/// rate, so the connections have to be opened at a rate a rate limiter would
/// object to; reading each one as it is opened puts a timeout between every pair
/// of them and turns a flood into a trickle. The first version of this test did
/// exactly that, offered two and a half connections a second against a ceiling of
/// fifty, and reported that the guard never fired.
///
/// Saying nothing on the connections is the method. Steel peeks for the first
/// bytes of a TLS record before it decides anything, so a connection it means to
/// serve simply waits; one the guard refused is closed at once with nothing sent.
/// The two are told apart by whether a read returns end-of-file or nothing at
/// all.
fn burst(port: u16, n: usize) -> Outcome<Vec<Fate>> {
    let here = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut held = Vec::with_capacity(n);
    let began = Instant::now();
    for _ in 0..n {
        held.push(TcpStream::connect_timeout(&here, Duration::from_secs(2)));
    }
    let took = began.elapsed();
    // The claim rests on the offered rate, so it is checked rather than assumed.
    // A machine slow enough to have offered less than the ceiling would fail the
    // assertions below for a reason that has nothing to do with the guard.
    let rate = (n as f64) / took.as_secs_f64().max(f64::MIN_POSITIVE);
    if rate < (DEFAULT_RPS_MAX as f64) {
        return Err(err!(
            "{} connections took {:?}, which is {:.0} a second and below the \
            guard's ceiling of {}. Nothing was offered that it should have \
            refused, so neither answer would mean anything.",
            n, took, rate, DEFAULT_RPS_MAX;
        Test, Timeout));
    }
    std::thread::sleep(Duration::from_millis(SETTLE_MS));
    let mut fates = Vec::with_capacity(n);
    for opened in held.iter_mut() {
        fates.push(match opened {
            Err(_) => Fate::Refused,
            Ok(stream) => {
                if stream.set_read_timeout(
                    Some(Duration::from_millis(ASK_MS))).is_err()
                {
                    Fate::Refused
                } else {
                    let mut buf = [0u8; 1];
                    match stream.read(&mut buf) {
                        Ok(0)   => Fate::Dropped,
                        Ok(_)   => Fate::Kept,  // it spoke, so it is serving us
                        Err(_)  => Fate::Kept,  // silent and open: waiting
                    }
                }
            },
        });
    }
    Ok(fates)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE FIXTURE                                                               │
// └───────────────────────────────────────────────────────────────────────────┘
//
// The same shape as `stopping_signal.rs`, and deliberately not shared with it: a
// fixture that two test files steer is a fixture neither of them can change.

/// A name per fixture, because the tests in this file run at the same time.
/// Under `~/.cache` rather than `/tmp`, which is a tmpfs on this machine and has
/// brought it down before.
fn scratch(name: &str) -> Outcome<PathBuf> {
    let base = match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".cache"),
        Err(_) => std::env::temp_dir(),
    };
    let dir = base.join(fmt!("steel_addrguard_{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    res!(std::fs::create_dir_all(&dir), IO, File);
    Ok(dir)
}

/// A port nothing is listening on, asked of the operating system rather than
/// picked.
fn free_port() -> Outcome<u16> {
    let probe = res!(TcpListener::bind("127.0.0.1:0"), IO, Network);
    let port = res!(probe.local_addr(), IO, Network).port();
    drop(probe);
    Ok(port)
}

/// Everything a Steel needs to start.
///
/// The `addr_guard` block is left empty on purpose. That is the deployed
/// configuration on the host this was written for, and an empty block is not an
/// absent guard: every field falls back to the `DEFAULT_*` constants, which is
/// exactly what these tests are asserting about.
fn lay_out(dir: &Path, port: u16) -> Outcome<()> {
    for sub in ["www/public", "www/src/styles", "www/logs"] {
        res!(std::fs::create_dir_all(dir.join(sub)), IO, File);
    }
    res!(std::fs::write(
        dir.join("www").join("public").join("index.html"),
        "<!doctype html><title>guard</title><p>here.\n",
    ), IO, File);

    let cfg = fmt!("{{
  \"app_description\":    \"Address guard fixture\",
  \"app_human_name\":     \"Addrguard\",
  \"app_log_level\":      \"info\",
  \"app_name\":           \"{app}\",
  \"app_root\":           \"current\",
  \"enc_name\":           \"AES-256-GCM\",
  \"kdf_name\":           \"Argon2id_v0x13\",
  \"dev_cfg\": {{
    \"src_path_rel\":             \"./www/src\",
    \"js_bundles_rel\":           {{}},
    \"js_import_aliases_rel\":    {{}},
    \"css_source_dir_rel\":       \"./www/src/styles\",
    \"css_bundle_rel\":           \"./www/public/styles.css\"
  }},
  \"server_cfg\": {{
    \"log_level\":                    \"info\",
    \"num_server_bots\":              (u16|1),
    \"server_address\":               \"127.0.0.1\",
    \"server_port_tcp\":              (u16|{port}),
    \"server_port_tcp_plaintext\":    (u16|0),
    \"hsts_max_age_secs\":            (u32|0),
    \"admin_local_port\":             (u16|0),
    \"session_expiry_default_secs\":  (u32|604800),
    \"ws_ping_interval_secs\":        (u8|30),
    \"server_max_errors_allowed\":    (u8|30),
    \"allow_anonymous_sessions\":     (true),
    \"http_max_header_bytes\":        (u64|16384),
    \"http_max_body_bytes\":          (u64|8388608),
    \"http_header_read_timeout_ms\":  (u64|15000),
    \"security_headers_enabled\":     (true),
    \"content_security_policy\":      \"\",
    \"addr_guard\":                   {{}},
    \"auth_path_prefixes\":           (vek|[\"/admin/login\"]),
    \"auth_rps_max\":                 (u64|100),
    \"tls_dir_rel\":                  \"./tls\",
    \"acme\": {{
      \"enabled\":        (false),
      \"contact_email\":  \"\",
      \"directory_url\":  \"https://acme-staging-v02.api.letsencrypt.org/directory\",
      \"cache_dir_rel\":  \"./tls/acme\"
    }},
    \"mail\": {{}},
    \"vhosts\": [
      {{
        \"hostnames\":                (vek|[\"localhost\",\"localhost.\"]),
        \"public_dir_rel\":           \"./www/public\",
        \"static_route_paths_rel\":   {{}},
        \"default_index_files\":      (vek|[\"index.html\",\"index.htm\"]),
        \"redirects\":                [],
        \"db_dir_rel\":               \"./o3db\"
      }}
    ]
  }}
}}
", app = APP, port = port);
    res!(std::fs::write(dir.join("config.jdat"), cfg), IO, File);

    let mut metadata = BTreeMap::new();
    metadata.insert(dat!("app_name"), dat!(APP));
    let (wallet, _unlocked) = res!(Wallet::create_with_first_admin(
        metadata,
        fmt!("operator"),
        PASS.as_bytes(),
        DEFAULT_WALLET_KDF_NAME,
    ));
    res!(wallet.save(
        &dir.join(app_const::WALLET_NAME),
        "  ",
        Some(EncoderConfig::<(), ()>::default()),
    ));
    Ok(())
}

/// Everything the server has said, wherever it said it.
fn said(dir: &Path) -> String {
    let mut all = String::new();
    for path in [
        dir.join("console.log"),
        dir.join("www").join("logs").join(fmt!("{}.log", APP)),
    ] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            all.push_str(&text);
        }
    }
    all
}

/// Starts a Steel and waits until it is answering.
///
/// It must be *unsealed* before the tests run, because the guard is reached
/// through the admin state and a Steel with none skips it entirely. Waiting for
/// the port alone would start the flood against a server that had bound and not
/// yet built one.
fn serve_fixture(dir: &Path, port: u16) -> Outcome<(Child, usize)> {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_steel"));
    let console = res!(std::fs::File::create(dir.join("console.log")), IO, File);
    let errs = res!(console.try_clone(), IO, File);
    let child = res!(Command::new(&exe)
        .current_dir(dir)
        .arg("server")
        // `-d` or nothing: without it a first run refuses production mode and
        // exits 0 in silence. Dev mode also generates the self-signed
        // certificate this fixture would otherwise have to carry.
        .arg("-d")
        .env("STEEL_ADMIN_PASS", PASS)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(console))
        .stderr(Stdio::from(errs))
        .spawn(), IO, File);

    let here = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let began = Instant::now();
    let mut bound = false;
    // Every one of these is a connection from this address, and the guard counts
    // it like any other. The count is handed back so a test measuring the ring
    // can allow for what waiting cost it.
    let mut probes = 0;
    while began.elapsed() < Duration::from_secs(START_SECS) {
        if !bound {
            probes += 1;
            if TcpStream::connect_timeout(&here, Duration::from_millis(500)).is_ok() {
                bound = true;
            }
        }
        if bound && said(dir).contains("database(s) open and attached") {
            return Ok((child, probes));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let log = said(dir);
    put_down(child);
    Err(err!(
        "The server did not bind {} and open its database within {} seconds \
        (bound: {}). It said:\n{}",
        here, START_SECS, bound, tail_of(&log);
    Test, Timeout))
}

/// The last of what a server said, for a message that has to carry a reason.
fn tail_of(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let from = lines.len().saturating_sub(40);
    lines[from..].join("\n")
}

fn put_down(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}
