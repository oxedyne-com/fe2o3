//! A real signal, sent to a real Steel, and what it leaves behind.
//!
//! Everything else in this directory asks the code a question and reads the
//! code's answer. This asks the operating system, because the fault it is here
//! for cannot be seen from inside: `Server::start` ends in an accept loop with
//! no way out, so `systemctl stop`, a reboot, and a Ctrl-C at a terminal all
//! reach a process that hears nothing and is killed where it stands -- with a
//! vhost's Ozone instance still open.
//!
//! What is checked is that Steel now hears them, and that hearing one costs
//! nothing:
//!
//! 1. `SIGTERM` -- what a service manager and every reboot send -- ends the
//!    process by its own choice, with a nought, rather than felling it.
//! 2. `SIGINT` -- Ctrl-C -- does the same.
//! 3. The store was *shut*, not merely left readable, and it opens again after.
//!
//! The signal is sent by `kill`, the operating system's own tool, to a process
//! identifier this test holds. Nothing here matches on a command line: a live
//! Steel may be running on this machine, and anything that matched by name could
//! take a real server with it.
//!
//! Unix only. Windows has no `kill` and no signals; the three console events the
//! same listener answers there cannot be sent from here.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

#![cfg(unix)]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::keystore::{
    DEFAULT_WALLET_KDF_NAME,
    Wallet,
};
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    file::JdatFile,
    string::enc::EncoderConfig,
};
use oxedyne_fe2o3_steel::{
    app::constant as app_const,
    srv::{
        context::new_db,
        id,
    },
};

use std::{
    collections::BTreeMap,
    net::{
        IpAddr,
        Ipv4Addr,
        SocketAddr,
        TcpListener,
        TcpStream,
    },
    os::unix::process::ExitStatusExt,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Child,
        Command,
        ExitStatus,
        Stdio,
    },
    time::{
        Duration,
        Instant,
    },
};

use secrecy::ExposeSecret;

// The wallet passphrase, in the clear and on purpose: it protects a wallet that exists for a few
// seconds in a scratch directory and holds one key to one empty database. The rig beside this one
// says the same thing for the same reason.
const PASS: &str = "steel-stop-test-passphrase-not-a-secret"; // allowlist secret

const APP: &str = "stopsig";

const START_SECS: u64 = 120;

const STOP_SECS: u64 = 120;

/// Where a fixture goes.
///
/// A name per fixture, because the tests in this file run at the same time and a
/// shared directory is wiped by whichever reaches it last. Under `~/.cache`
/// rather than `/tmp`: an Ozone instance is not a small thing to hold in a
/// tmpfs, and this machine has been brought down that way before.
fn scratch(name: &str) -> Outcome<PathBuf> {
    let base = match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".cache"),
        Err(_) => std::env::temp_dir(),
    };
    let dir = base.join(fmt!("steel_stopsignal_{}", name));
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

fn in_use(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_millis(250),
    ).is_ok()
}

/// Everything a Steel needs to start: a config, a wallet, and the directories
/// the dev-mode refresh insists on.
///
/// The config is the test rig's, with the port substituted and the blocks a
/// stop has nothing to do with left out. The one thing it must have is a vhost
/// with a `db_dir_rel`, because a Steel with no store cannot demonstrate
/// anything about closing one.
fn lay_out(dir: &Path, port: u16) -> Outcome<Vec<u8>> {
    for sub in ["www/public", "www/src/styles", "www/logs"] {
        res!(std::fs::create_dir_all(dir.join(sub)), IO, File);
    }
    res!(std::fs::write(
        dir.join("www").join("public").join("index.html"),
        "<!doctype html><title>stop</title><p>here.\n",
    ), IO, File);

    let cfg = fmt!("{{
  \"app_description\":    \"Stop signal fixture\",
  \"app_human_name\":     \"Stopsig\",
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

    // The wallet, built here rather than through the shell. Creating one at the
    // prompt reads the passphrase in crossterm's raw mode, which needs a real
    // terminal and is why the older rig carries a pty in Python; the library
    // that shell calls is right here and takes bytes.
    let mut metadata = BTreeMap::new();
    metadata.insert(dat!("app_name"), dat!(APP));
    let (wallet, unlocked) = res!(Wallet::create_with_first_admin(
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
    // The wallet master key is the database encryption key, so the check that
    // reopens the store afterwards needs it.
    Ok(unlocked.master_key.expose_secret().clone())
}

/// Everything the server has said, wherever it said it.
///
/// Two files, because the two matter for different reasons: the log on disk is
/// what an operator reads after the fact, and the console capture is what
/// survives a failure early enough that the log was never configured.
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

/// The last few lines of what it said, for a failure that has to show its work.
fn tail_of(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let from = lines.len().saturating_sub(15);
    lines[from..].join("\n")
}

/// Kills the child, by its own handle and never by name.
fn put_down(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Sends one signal to one process, using the operating system's own tool.
///
/// `kill` rather than anything in this program: sending a signal from Rust means
/// `libc::kill`, which is an `extern "C"` call and would need an `unsafe` block
/// in a crate that forbids them. The external tool is also the better oracle --
/// it is what a person at a terminal or a service manager would use, rather than
/// Steel agreeing with itself.
fn send(signal: &str, child: &Child) -> Outcome<()> {
    let out = res!(Command::new("kill")
        .arg(fmt!("-{}", signal))
        .arg(fmt!("{}", child.id()))
        .output(), IO, File);
    if !out.status.success() {
        return Err(err!(
            "kill -{} {} answered {:?}: {}",
            signal, child.id(), out.status,
            String::from_utf8_lossy(&out.stderr).trim();
        Test, System));
    }
    Ok(())
}

fn wait_for_end(child: &mut Child) -> Outcome<ExitStatus> {
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(STOP_SECS) {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(err!(e,
                "The server could not be asked how it was."; Test, IO)),
        }
    }
    Err(err!(
        "The server was still running {} seconds after it was signalled.",
        STOP_SECS; Test, Timeout))
}

/// How a process ended, said the way a person would say it.
///
/// The distinction this whole file turns on: a process that *exited* chose to,
/// and a process that was *felled by a signal* did not.
fn how_it_ended(status: &ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => fmt!("it exited with {}", code),
        (None, Some(sig)) => fmt!(
            "it was felled by signal {}, which means it caught nothing", sig),
        (None, None) => fmt!("{:?}", status),
    }
}

/// Starts a Steel on the fixture and waits until its database is open.
///
/// Waiting for the port alone would not do. A Steel starts sealed and binds
/// before it opens anything, so a signal sent at that moment would find no store
/// to close and the test would pass while proving nothing. `STEEL_ADMIN_PASS`
/// unseals it at start-up, and the line waited for here is the one the opener
/// writes when every configured Ozone is up and attached.
fn serve_fixture(dir: &Path, port: u16) -> Outcome<Child> {
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
        // Kept open for the life of the child. The shell sits beside the
        // listener and an EOF on stdin ends the process, which would look
        // exactly like the clean stop this test is trying to observe.
        .stdin(Stdio::piped())
        .stdout(Stdio::from(console))
        .stderr(Stdio::from(errs))
        .spawn(), IO, File);

    let here = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let began = Instant::now();
    let mut bound = false;
    while began.elapsed() < Duration::from_secs(START_SECS) {
        if !bound && TcpStream::connect_timeout(&here, Duration::from_millis(500)).is_ok() {
            bound = true;
        }
        if bound && said(dir).contains("database(s) open and attached") {
            return Ok(child);
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

/// One signal, one clean stop, and a store that was shut and opens again.
///
/// The whole claim in one function, so that `SIGTERM` and `SIGINT` are held to
/// exactly the same standard rather than to two slightly different ones.
fn a_signal_stops_it_cleanly(signal: &str, name: &str) -> Outcome<()> {
    let dir = res!(scratch(name));
    let port = res!(free_port());
    req!(in_use(port), false,
        "Something is already listening on port {}. Every check below would be \
        answered by it rather than by the server this test starts.", port);
    let key = res!(lay_out(&dir, port));

    let mut child = res!(serve_fixture(&dir, port));

    if let Err(e) = send(signal, &child) {
        put_down(child);
        return Err(e);
    }

    let status = match wait_for_end(&mut child) {
        Ok(status) => status,
        Err(e) => {
            let log = said(&dir);
            put_down(child);
            return Err(err!(e, "It said:\n{}", tail_of(&log); Test));
        },
    };

    let log = said(&dir);

    // The teeth of this test. A process that catches nothing is felled by the
    // signal and reports no exit code at all; a process that was asked, and did
    // as it was asked, exits by itself with a nought. A service manager reads
    // anything else as a unit that failed -- and a Steel felled here is a Steel
    // killed with its Ozone open, which is the incident.
    req!(status.success(), true,
        "A {} did not stop the server cleanly: {}. It said:\n{}",
        signal, how_it_ended(&status), tail_of(&log));

    // The port is genuinely given back, so a restart is not refused.
    req!(in_use(port), false, "Something is still listening on port {}.", port);

    // The line at the end of `start_server` that this whole change exists to
    // make reachable.
    req!(log.contains("Server stopped gracefully."), true,
        "A {} ended the server without it ever reaching the end of \
        `start_server`. It said:\n{}", signal, tail_of(&log));

    // And the store was shut, which the survey below cannot tell on its own:
    // Ozone acknowledges each write before returning, so a killed process still
    // leaves a readable store. Being able to reopen it is necessary and not
    // sufficient; these two lines are the evidence that the close ran, the
    // second of them written by Ozone's own supervisor after every bot thread
    // had ended.
    req!(log.contains("Closed the database for vhost 'localhost'."), true,
        "A {} ended the server without it recording that it had closed the \
        vhost's database. It said:\n{}", signal, tail_of(&log));
    req!(log.contains("Shutdown: Verified."), true,
        "The database was never verified as shut down, so whatever closed it \
        did not finish. It said:\n{}", tail_of(&log));

    // And the store opens again, and works. A stop that left it unreadable
    // would be a worse fault than the one being fixed.
    {
        let db_dir = dir.join("o3db");
        let mut db = res!(new_db(&db_dir, &key));
        // The sequence the server itself runs, to the letter. Starting
        // without the rest of it leaves the bots up but not answering, and
        // every read then fails on a responder timeout that says nothing
        // about whether the store is sound.
        res!(db.start("db_reopen"));
        res!(ok!(db.updated_api()).activate_gc(true));
        std::thread::sleep(Duration::from_millis(200));
        let (_start, msgs) = res!(db.api().ping_bots(app_const::GET_DATA_WAIT));
        let answered = !msgs.is_empty();
        req!(answered, true,
            "The store left by a {} reopened but none of its bots answered.",
            signal);
        let uid = id::Uid::default();
        res!(db.insert(dat!("after/the/stop"), dat!("readable"), uid, None));
        let back = res!(db.get(&dat!("after/the/stop"), None));
        let found = match back {
            Some((val, _meta)) => val == dat!("readable"),
            None => false,
        };
        req!(found, true,
            "The store left by a {} would not take and return a value, so the \
            stop cost it something.", signal);
        res!(db.close());
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// What `systemctl stop` and every reboot send.
///
/// The signal from the incident: two live servers are stopped this way whenever
/// their machines are rebooted, and until now each was killed mid-flight with
/// its Ozone instances open.
#[test]
fn test_a_terminate_stops_the_server_cleanly_00() -> Outcome<()> {
    a_signal_stops_it_cleanly("TERM", "term")
}

/// Ctrl-C, which is what a person at a terminal sends.
#[test]
fn test_an_interrupt_stops_the_server_cleanly_01() -> Outcome<()> {
    a_signal_stops_it_cleanly("INT", "int")
}
