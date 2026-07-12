//! The IMAP client and the IMAP server, talking to each other.
//!
//! Neither side is mocked: a real [`MaildirStore`] on disk, the real
//! [`ImapServer`] over a real loopback socket, and the real
//! [`ImapClient`] reading it back. What is being tested is the wire --
//! literals, flags, UID sets, sequence numbers -- because that is where an
//! IMAP implementation is either right or subtly, silently wrong.
//!
//! The messages deliberately include one whose body contains a bare CRLF
//! and a `)`, which is exactly what breaks a client that parses FETCH
//! responses line by line.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::{
    imap::{
        client::{
            FetchWhat,
            FlagOp,
            ImapClient,
            ImapConfig,
            Security,
        },
        server::ImapServer,
    },
    mail::{
        store::{
            FolderName,
            MailStore,
            MailUser,
            MessageFlags,
        },
        user::UserStore,
    },
};

use std::{
    fs,
    path::PathBuf,
    time::Duration,
};

use tokio::net::TcpListener;


const USER: &str = "alice@test.local";
const PASS: &str = "correct-horse";

/// A one-user store, so the test exercises the protocol rather than a
/// password file.
#[derive(Clone, Debug)]
struct OneUser {
    /// Where this user's Maildir lives.
    delivery_key: String,
}

impl UserStore for OneUser {
    fn authenticate(&self, address: &str, password: &str) -> Outcome<Option<MailUser>> {
        if address.eq_ignore_ascii_case(USER) && password == PASS {
            return Ok(self.lookup(address).ok().flatten());
        }
        Ok(None)
    }

    fn lookup(&self, address: &str) -> Outcome<Option<MailUser>> {
        if !address.eq_ignore_ascii_case(USER) {
            return Ok(None);
        }
        Ok(Some(MailUser {
            local:        fmt!("alice"),
            domain:       fmt!("test.local"),
            delivery_key: self.delivery_key.clone(),
        }))
    }
}

/// A message whose body would break a line-oriented parser.
fn awkward_message() -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(b"From: bob@example.org\r\n");
    m.extend_from_slice(b"To: alice@test.local\r\n");
    m.extend_from_slice(b"Subject: literals\r\n");
    m.extend_from_slice(b"\r\n");
    m.extend_from_slice(b"A body with a blank line,\r\n");
    m.extend_from_slice(b"\r\n");
    m.extend_from_slice(b") a stray close paren, and {17} a fake literal.\r\n");
    m
}

fn plain_message(n: usize) -> Vec<u8> {
    fmt!("From: bob@example.org\r\n\
          To: alice@test.local\r\n\
          Subject: message {}\r\n\
          \r\n\
          Body of message {}.\r\n", n, n).into_bytes()
}

#[tokio::test]
async fn imap_client_reads_what_the_imap_server_serves() -> Outcome<()> {

    // ── A Maildir with three messages in it ──────────────────────
    let root = std::env::temp_dir().join(fmt!("fe2o3_imap_rt_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);

    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    let users = OneUser { delivery_key: fmt!("test.local/alice") };
    let user  = match res!(users.lookup(USER)) {
        Some(u) => u,
        None    => return Err(err!("The test user did not resolve."; Test, Bug)),
    };
    res!(store.ensure_user(&user));

    let inbox = FolderName::new("INBOX");
    let bodies = vec![plain_message(1), awkward_message(), plain_message(3)];
    for body in &bodies {
        res!(store.append(&user, &inbox, body, MessageFlags::default(), None));
    }

    // ── The server, on a loopback socket ─────────────────────────
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
    let addr = res!(listener.local_addr(), IO, Network);
    let server = ImapServer {
        store:    store.clone(),
        users:    users.clone(),
        hostname: std::sync::Arc::new(fmt!("test.local")),
    };
    tokio::spawn(async move {
        while let Ok((sock, peer)) = listener.accept().await {
            let srv = server.clone();
            tokio::spawn(async move {
                let _ = srv.run(sock, peer).await;
            });
        }
    });

    // ── The client ───────────────────────────────────────────────
    let cfg = ImapConfig::new(fmt!("127.0.0.1"), addr.port(), Security::Plain)
        .with_timeout(Duration::from_secs(10));
    let mut c = res!(ImapClient::connect(&cfg).await);
    assert!(c.has_cap("IMAP4REV1"), "the server did not advertise IMAP4rev1");

    res!(c.login(USER, PASS).await);

    let folders = res!(c.list("", "*").await);
    assert!(folders.iter().any(|f| f.name.eq_ignore_ascii_case("INBOX")),
        "INBOX was not in the LIST reply: {:?}", folders);

    let st = res!(c.select("INBOX").await);
    assert_eq!(st.exists, 3, "wrong message count");
    assert!(st.uid_validity > 0, "the server gave no UIDVALIDITY");
    assert!(st.uid_next > 3, "UIDNEXT should be past the messages we appended");

    let uids = res!(c.uid_search("ALL").await);
    assert_eq!(uids.len(), 3, "UID SEARCH ALL found {} of 3", uids.len());

    // ── The whole point: the bytes come back byte-identical ──────
    let msgs = res!(c.uid_fetch(&uids, FetchWhat::Full).await);
    assert_eq!(msgs.len(), 3, "UID FETCH returned {} of 3", msgs.len());
    for (i, msg) in msgs.iter().enumerate() {
        assert_eq!(msg.body, bodies[i],
            "message {} came back changed:\n--- got ---\n{}\n--- want ---\n{}",
            i,
            String::from_utf8_lossy(&msg.body),
            String::from_utf8_lossy(&bodies[i]));
        assert_eq!(msg.size as usize, bodies[i].len(), "RFC822.SIZE disagrees");
        assert!(msg.uid > 0, "message {} came back with no UID", i);
    }

    // Headers-only: the body stops at the blank line, and the size still
    // describes the whole message.
    let heads = res!(c.uid_fetch(&uids[..1], FetchWhat::Headers).await);
    assert_eq!(heads.len(), 1);
    let head = String::from_utf8_lossy(&heads[0].body).into_owned();
    assert!(head.contains("Subject: message 1"), "header block missing subject: {:?}", head);
    assert!(!head.contains("Body of message 1"), "headers-only fetch returned the body");
    assert_eq!(heads[0].size as usize, bodies[0].len());

    // ── Flags round-trip ─────────────────────────────────────────
    res!(c.uid_store_flags(&uids[..1], FlagOp::Add, &["\\Seen"]).await);
    let after = res!(c.uid_fetch(&uids[..1], FetchWhat::Meta).await);
    assert!(after[0].flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")),
        "\\Seen did not stick: {:?}", after[0].flags);

    // ── Append, and see it arrive ────────────────────────────────
    let sent = plain_message(4);
    res!(c.append("INBOX", &["\\Seen"], &sent).await);
    let st = res!(c.select("INBOX").await);
    assert_eq!(st.exists, 4, "the appended message did not land");

    res!(c.logout().await);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A wrong password must be a clean, named failure -- not a hang, and not
/// a success.
#[tokio::test]
async fn imap_client_reports_a_bad_password() -> Outcome<()> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_imap_auth_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);

    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    let users = OneUser { delivery_key: fmt!("test.local/alice") };
    if let Some(u) = res!(users.lookup(USER)) {
        res!(store.ensure_user(&u));
    }

    let listener = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
    let addr = res!(listener.local_addr(), IO, Network);
    let server = ImapServer {
        store,
        users,
        hostname: std::sync::Arc::new(fmt!("test.local")),
    };
    tokio::spawn(async move {
        while let Ok((sock, peer)) = listener.accept().await {
            let srv = server.clone();
            tokio::spawn(async move { let _ = srv.run(sock, peer).await; });
        }
    });

    let cfg = ImapConfig::new(fmt!("127.0.0.1"), addr.port(), Security::Plain)
        .with_timeout(Duration::from_secs(10));
    let mut c = res!(ImapClient::connect(&cfg).await);
    let outcome = c.login(USER, "wrong").await;
    assert!(outcome.is_err(), "a wrong password authenticated");

    let _ = fs::remove_dir_all(&root);
    Ok(())
}
