//! A throwaway IMAP server over a scratch Maildir, for exercising a client
//! without a real mailbox.
//!
//! Seeds a few messages, one of them containing the CRLF-and-parenthesis
//! body that breaks a line-oriented client, and serves them in the clear on
//! loopback. Plaintext, one user, no TLS: a fixture, never a deployment.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_mail --example imap_test_server -- <port> <maildir-root>
//! ```

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::{
    imap::server::ImapServer,
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
    sync::Arc,
};

use tokio::net::TcpListener;


const USER: &str = "alice@test.local";
const PASS: &str = "test-app-password";

/// A single hard-coded user, so the fixture needs no password file.
#[derive(Clone, Debug)]
struct OneUser;

impl UserStore for OneUser {
    fn authenticate(&self, address: &str, password: &str) -> Outcome<Option<MailUser>> {
        if address.eq_ignore_ascii_case(USER) && password == PASS {
            return self.lookup(address);
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
            delivery_key: fmt!("test.local/alice"),
        }))
    }
}

#[tokio::main]
async fn main() -> Outcome<()> {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1143);
    let root = PathBuf::from(args.get(2).cloned()
        .unwrap_or_else(|| fmt!("/tmp/fe2o3_imap_fixture")));

    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);

    let store = res!(MaildirStore::new(root.clone(), "test.local"));
    let users = OneUser;
    let user  = match res!(users.lookup(USER)) {
        Some(u) => u,
        None    => return Err(err!("The fixture user did not resolve."; Bug)),
    };
    res!(store.ensure_user(&user));

    // Every seeded message carries a `Message-ID`, because a client that answers one has to
    // point back at it -- a reply with no `In-Reply-To` arrives as an unrelated message with a
    // similar subject, and a fixture whose mail cannot be threaded cannot show that a client
    // threads. The second message names the first in `References`, so a fetched thread has a
    // chain in it and not merely two messages.
    let inbox = FolderName::new("INBOX");
    let seeds: Vec<Vec<u8>> = vec![
        fmt!("From: bank@example.org\r\n\
              To: {}\r\n\
              Subject: Your statement is ready\r\n\
              Message-ID: <stmt-202607@example.org>\r\n\
              Date: Mon, 06 Jul 2026 09:15:00 +0000\r\n\
              \r\n\
              Your July statement is available.\r\n", USER).into_bytes(),
        fmt!("From: bob@example.org\r\n\
              To: {}\r\n\
              Subject: Lunch on Thursday?\r\n\
              Message-ID: <lunch-1@example.org>\r\n\
              References: <stmt-202607@example.org>\r\n\
              Date: Tue, 07 Jul 2026 12:30:00 +0000\r\n\
              \r\n\
              Are you free Thursday? There is a new place on Bourke Street.\r\n\
              \r\n\
              ) A stray close paren, and {{17}} a fake literal, to break a\r\n\
              line-oriented parser.\r\n", USER).into_bytes(),
        fmt!("From: newsletter@example.net\r\n\
              To: {}\r\n\
              Subject: Weekly digest\r\n\
              Message-ID: <digest-28@example.net>\r\n\
              Date: Wed, 08 Jul 2026 06:00:00 +0000\r\n\
              \r\n\
              This week: nothing happened.\r\n", USER).into_bytes(),
    ];
    for s in &seeds {
        res!(store.append(&user, &inbox, s, MessageFlags::default(), None));
    }

    // A mailbox is not an inbox. Five of these are named exactly what the
    // server's SPECIAL-USE table recognises, so a client sees a role rather
    // than a name; the last is an ordinary folder with a space and a hierarchy
    // in it, which is what an ordinary folder actually looks like and what a
    // client that flattens names onto a filesystem has to survive.
    let others: Vec<(&str, Vec<Vec<u8>>)> = vec![
        ("Sent", vec![
            fmt!("From: {}\r\n\
                  To: bob@example.org\r\n\
                  Subject: Thursday works\r\n\
                  Message-ID: <sent-1@test.local>\r\n\
                  Date: Tue, 07 Jul 2026 13:05:00 +0000\r\n\
                  \r\n\
                  Bourke Street it is.\r\n", USER).into_bytes(),
        ]),
        ("Drafts",  vec![]),
        ("Archive", vec![
            fmt!("From: hr@example.org\r\n\
                  To: {}\r\n\
                  Subject: Your 2025 summary\r\n\
                  Message-ID: <arch-1@example.org>\r\n\
                  Date: Fri, 09 Jan 2026 08:00:00 +0000\r\n\
                  \r\n\
                  Filed for reference.\r\n", USER).into_bytes(),
        ]),
        ("Junk",  vec![]),
        ("Trash", vec![]),
        ("Projects/Bourke Street", vec![
            fmt!("From: bob@example.org\r\n\
                  To: {}\r\n\
                  Subject: The new place\r\n\
                  Message-ID: <proj-1@example.org>\r\n\
                  Date: Wed, 08 Jul 2026 10:00:00 +0000\r\n\
                  \r\n\
                  Booked for one.\r\n", USER).into_bytes(),
        ]),
    ];
    for (name, msgs) in &others {
        let folder = FolderName::new(*name);
        res!(store.create_folder(&user, &folder));
        res!(store.subscribe(&user, &folder));
        for m in msgs {
            res!(store.append(&user, &folder, m, MessageFlags::default(), None));
        }
    }

    let addr = fmt!("127.0.0.1:{}", port);
    let listener = res!(TcpListener::bind(&addr).await, IO, Network);
    println!("IMAP fixture: {} messages in INBOX and {} more folders for {} (password {}) on {}",
        seeds.len(), others.len(), USER, PASS, addr);

    let server = ImapServer {
        store,
        users,
        hostname: Arc::new(fmt!("test.local")),
    };
    loop {
        let (sock, peer) = res!(listener.accept().await, IO, Network);
        let srv = server.clone();
        tokio::spawn(async move {
            if let Err(e) = srv.run(sock, peer).await {
                println!("session with {} ended: {}", peer, e);
            }
        });
    }
}
