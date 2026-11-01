//! `sync::query` against Steel over a real socket: a search of From and Subject that matches
//! nothing else, a raw read and a read by part that leave `\Seen` unset, and a read that fetches
//! `BODYSTRUCTURE` first and holds its byte cap by part.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_mail::{
    maildir::MaildirStore,
    sync::{
        query::{
            self,
            Limits,
            Range,
            Role,
        },
        Roles,
        Target,
    },
};
use oxedyne_fe2o3_net::{
    imap::{
        client::{
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
const PASS: &str = "correct-horse"; // allowlist secret

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
        if !address.eq_ignore_ascii_case(USER) { return Ok(None); }
        Ok(Some(MailUser {
            local:          fmt!("alice"),
            domain:         fmt!("test.local"),
            delivery_key:   fmt!("test.local/alice"),
            send_as:        Vec::new(),
        }))
    }
}

fn plain(from: &str, to: &str, subj: &str, body: &str) -> Vec<u8> {
    fmt!("From: {}\r\nTo: {}\r\nSubject: {}\r\nMessage-ID: <{}@test>\r\n\r\n{}\r\n",
        from, to, subj, subj.len(), body).into_bytes()
}

/// A body in an alternative, then an attachment far larger than the read cap.
fn with_attachment() -> Vec<u8> {
    let pdf = "QUJD".repeat(5000); // 20,000 octets of base64
    fmt!("From: Shop <shop@example.org>\r\n\
        To: alice@test.local\r\n\
        Subject: Invoice\r\n\
        MIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"mix\"\r\n\
        \r\n\
        --mix\r\n\
        Content-Type: multipart/alternative; boundary=\"alt\"\r\n\
        \r\n\
        --alt\r\n\
        Content-Type: text/plain; charset=utf-8\r\n\
        \r\n\
        Your invoice is attached. Total 42 dollars.\r\n\
        --alt\r\n\
        Content-Type: text/html; charset=utf-8\r\n\
        \r\n\
        <p>Your invoice is attached.</p>\r\n\
        --alt--\r\n\
        --mix\r\n\
        Content-Type: application/pdf; name=\"invoice.pdf\"\r\n\
        Content-Transfer-Encoding: base64\r\n\
        Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n\
        \r\n\
        {}\r\n\
        --mix--\r\n", pdf).into_bytes()
}

/// Steel with five messages in INBOX, logged in, with the roles listed.
async fn steel(name: &str) -> Outcome<(ImapClient, Roles, PathBuf)> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_mail_query_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    let user = res!(res!(OneUser.lookup(USER)).ok_or_else(|| err!("No test user."; Test, Bug)));
    res!(store.ensure_user(&user));
    let inbox = FolderName::new("INBOX");
    for m in [
        plain("Umart Online <sales@umart.com.au>", USER, "Order 1001", "Thanks."),
        plain("Bob <bob@example.org>", USER, "Your UMART receipt", "Attached."),
        plain("Bob <bob@example.org>", "umart@example.org", "lunch", "Did umart call?"),
        plain("Carol <carol@example.org>", USER, "unrelated", "Nothing here."),
        with_attachment(),
    ] {
        res!(store.append(&user, &inbox, &m, MessageFlags::default(), None));
    }
    let listener = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
    let addr = res!(listener.local_addr(), IO, Network);
    let server = ImapServer {
        store:      store.clone(),
        users:      OneUser,
        hostname:   std::sync::Arc::new(fmt!("test.local")),
    };
    tokio::spawn(async move {
        while let Ok((sock, peer)) = listener.accept().await {
            let srv = server.clone();
            tokio::spawn(async move {
                let _ = srv.run(sock, peer).await;
            });
        }
    });
    let cfg = ImapConfig::new(fmt!("127.0.0.1"), addr.port(), Security::Plain)
        .with_timeout(Duration::from_secs(10));
    let mut c = res!(ImapClient::connect(&cfg).await);
    res!(c.login(USER, PASS).await);
    let lim = limits(1000);
    let (roles, _) = res!(query::folders(&mut c, &lim).await);
    Ok((c, roles, root))
}

fn limits(read: u32) -> Limits {
    Limits { max_rows: 50, max_read_bytes: read, max_folders: 20 }
}

async fn seen(c: &mut ImapClient, roles: &Roles) -> Outcome<Vec<u32>> {
    let l = res!(query::list(c, roles, &Target::Inbox, &Range::default(), &limits(1000)).await);
    Ok(l.rows.iter().filter(|h| h.seen()).map(|h| h.uid).collect())
}

#[tokio::test]
async fn find_matches_from_and_subject_in_any_case_and_nothing_else() -> Outcome<()> {
    let (mut c, roles, root) = res!(steel("find").await);
    let l = res!(query::find(&mut c, &roles, &Target::Inbox, "umart", &Range::default(),
        &limits(1000)).await);
    let subjects: Vec<&str> = l.rows.iter().map(|h| h.subject.as_str()).collect();
    assert_eq!(subjects, vec!["Your UMART receipt", "Order 1001"],
        "From and Subject, newest first, and neither To nor the body");
    assert_eq!(l.total, 5);
    assert!(!l.more);
    let none = res!(query::find(&mut c, &roles, &Target::Inbox, "zebra", &Range::default(),
        &limits(1000)).await);
    assert!(none.rows.is_empty());
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn raw_leaves_seen_unset_and_holds_its_cap() -> Outcome<()> {
    let (mut c, roles, root) = res!(steel("raw").await);
    let whole = res!(res!(query::raw(&mut c, &roles, &Target::Inbox, 1, 1_000_000).await)
        .ok_or_else(|| err!("UID 1 was not read."; Test, Missing)));
    assert!(!whole.cut && whole.bytes.len() as u32 == whole.size, "the whole message: {:?}", whole);
    assert!(String::from_utf8_lossy(&whole.bytes).contains("Order 1001"));
    let part = res!(res!(query::raw(&mut c, &roles, &Target::Inbox, 5, 100).await)
        .ok_or_else(|| err!("UID 5 was not read."; Test, Missing)));
    assert!(part.cut && part.bytes.len() == 100, "cut at 100: {} of {}", part.bytes.len(), part.size);
    assert!(res!(query::raw(&mut c, &roles, &Target::Inbox, 99, 100).await).is_none());
    assert_eq!(res!(seen(&mut c, &roles).await), Vec::<u32>::new(), "a raw read set \\Seen");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn read_fetches_the_structure_first_and_holds_the_cap_by_part() -> Outcome<()> {
    let (mut c, roles, root) = res!(steel("read").await);
    // A cap far under the message's 20 KB, which a cut of the whole would spend on its head.
    let r = res!(res!(query::read(&mut c, &roles, &Target::Inbox, 5, &limits(2000)).await)
        .ok_or_else(|| err!("UID 5 was not read."; Test, Missing)));
    assert!(r.header.size > 20_000);
    assert_eq!(r.header.subject, "Invoice");
    assert_eq!(r.texts.len(), 1, "the plain form of the alternative only: {:?}", r.texts);
    assert_eq!((r.texts[0].section.as_str(), r.texts[0].media.as_str()), ("1.1", "text/plain"));
    assert!(r.texts[0].text.contains("Total 42 dollars") && !r.cut(), "{:?}", r.texts[0]);
    assert_eq!(r.attachments.len(), 1);
    let a = &r.attachments[0];
    assert_eq!((a.section.as_str(), a.media.as_str(), a.name.as_deref()),
        ("2", "application/pdf", Some("invoice.pdf")));
    assert!(a.size >= 20_000, "the attachment's size, unread: {}", a.size);

    // A cap under the body cuts the body, and says so.
    let r = res!(res!(query::read(&mut c, &roles, &Target::Inbox, 5, &limits(10)).await)
        .ok_or_else(|| err!("UID 5 was not read."; Test, Missing)));
    assert!(r.cut() && r.texts[0].text.len() <= 10, "{:?}", r.texts);

    // A message that is not multipart is its own part 1.
    let r = res!(res!(query::read(&mut c, &roles, &Target::Inbox, 1, &limits(2000)).await)
        .ok_or_else(|| err!("UID 1 was not read."; Test, Missing)));
    assert_eq!(r.texts.len(), 1);
    assert_eq!(r.texts[0].section, "1");
    assert!(r.texts[0].text.contains("Thanks."), "{:?}", r.texts);
    assert_eq!(res!(seen(&mut c, &roles).await), Vec::<u32>::new(), "a read set \\Seen");
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn folders_counts_and_names_the_inbox_on_steel() -> Outcome<()> {
    let (mut c, _, root) = res!(steel("folders").await);
    let (_, fs) = res!(query::folders(&mut c, &limits(1000)).await);
    let inbox = res!(fs.iter().find(|f| f.role == Some(Role::Inbox))
        .ok_or_else(|| err!("No inbox: {:?}", fs; Test, Missing)));
    assert_eq!(inbox.count, Some(5));
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
