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

    let inbox = FolderName::new("INBOX");
    let seeds: Vec<Vec<u8>> = vec![
        fmt!("From: bank@example.org\r\n\
              To: {}\r\n\
              Subject: Your statement is ready\r\n\
              Date: Mon, 06 Jul 2026 09:15:00 +0000\r\n\
              \r\n\
              Your July statement is available.\r\n", USER).into_bytes(),
        fmt!("From: bob@example.org\r\n\
              To: {}\r\n\
              Subject: Lunch on Thursday?\r\n\
              Date: Tue, 07 Jul 2026 12:30:00 +0000\r\n\
              \r\n\
              Are you free Thursday? There is a new place on Bourke Street.\r\n\
              \r\n\
              ) A stray close paren, and {{17}} a fake literal, to break a\r\n\
              line-oriented parser.\r\n", USER).into_bytes(),
        fmt!("From: newsletter@example.net\r\n\
              To: {}\r\n\
              Subject: Weekly digest\r\n\
              Date: Wed, 08 Jul 2026 06:00:00 +0000\r\n\
              \r\n\
              This week: nothing happened.\r\n", USER).into_bytes(),
    ];
    for s in &seeds {
        res!(store.append(&user, &inbox, s, MessageFlags::default(), None));
    }

    let addr = fmt!("127.0.0.1:{}", port);
    let listener = res!(TcpListener::bind(&addr).await, IO, Network);
    println!("IMAP fixture: {} messages for {} (password {}) on {}",
        seeds.len(), USER, PASS, addr);

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
