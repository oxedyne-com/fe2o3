//! Steel's IMAP SEARCH against a real Maildir over a real socket.
//!
//! A client filter, a move and the send-and-file step all trust SEARCH to
//! name exactly the messages that match. A criterion the server does not
//! understand must be refused with a tagged BAD: answering it with the whole
//! mailbox turns a narrow action into one on every message.

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
            MessageUid,
        },
        user::UserStore,
    },
};

use std::{
    fs,
    path::PathBuf,
    time::{
        Duration,
        UNIX_EPOCH,
    },
};

use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncWriteExt,
        BufReader,
    },
    net::{
        TcpListener,
        TcpStream,
    },
};


const USER: &str = "alice@test.local";
const PASS: &str = "correct-horse";

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

/// One message to file, and the INTERNALDATE (days after the epoch) it is given.
struct Msg {
    from:   &'static str,
    to:     &'static str,
    cc:     &'static str,
    subj:   &'static str,
    mid:    &'static str,
    body:   &'static str,
    day:    u64,
    flags:  MessageFlags,
}

impl Msg {
    fn bytes(&self) -> Vec<u8> {
        let mut s = fmt!("From: {}\r\nTo: {}\r\n", self.from, self.to);
        if !self.cc.is_empty() { s.push_str(&fmt!("Cc: {}\r\n", self.cc)); }
        s.push_str(&fmt!("Subject: {}\r\nMessage-ID: {}\r\n\r\n{}\r\n", self.subj, self.mid, self.body));
        s.into_bytes()
    }
}

fn corpus() -> Vec<Msg> {
    let seen = MessageFlags { seen: true, ..MessageFlags::default() };
    let ans = MessageFlags { answered: true, seen: true, ..MessageFlags::default() };
    let draft = MessageFlags { draft: true, ..MessageFlags::default() };
    vec![
        // 2024-01-01 is day 19723 after the epoch.
        Msg { from: "Bob <bob@example.org>", to: "alice@test.local", cc: "",
            subj: "Quarterly Report", mid: "<a1@example.org>",
            body: "The figures are attached.", day: 19723, flags: seen },
        Msg { from: "carol@example.net", to: "alice@test.local", cc: "dave@example.com",
            subj: "Lunch", mid: "<b2@example.net>",
            body: "Shall we meet at noon? The REPORT can wait.", day: 19724, flags: MessageFlags::default() },
        Msg { from: "Bob <bob@example.org>", to: "team@test.local", cc: "",
            subj: "re: lunch", mid: "<c3@example.org>",
            body: "Yes, noon suits me. xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
            day: 19725, flags: ans },
        Msg { from: "erin@example.com", to: "alice@test.local", cc: "bob@example.org",
            subj: "Draft notes", mid: "<d4@example.com>",
            body: "Nothing to see.", day: 19726, flags: draft },
    ]
}

async fn read_until_tagged(r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>, tag: &str)
    -> Outcome<Vec<String>>
{
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        let n = res!(r.read_line(&mut line).await, IO, Network);
        if n == 0 { return Err(err!("The server closed the connection."; IO, Network)); }
        let done = line.starts_with(&fmt!("{} ", tag));
        lines.push(line.trim_end().to_string());
        if done { return Ok(lines); }
    }
}

struct Conn {
    r:      BufReader<tokio::net::tcp::OwnedReadHalf>,
    w:      tokio::net::tcp::OwnedWriteHalf,
    n:      usize,
}

impl Conn {
    async fn cmd(&mut self, text: &str) -> Outcome<Vec<String>> {
        self.n += 1;
        let tag = fmt!("t{}", self.n);
        res!(self.w.write_all(fmt!("{} {}\r\n", tag, text).as_bytes()).await, IO, Network);
        read_until_tagged(&mut self.r, &tag).await
    }

    /// The UIDs a `UID SEARCH` names, or the tagged status when it is not OK.
    async fn uid_search(&mut self, criteria: &str) -> Outcome<Result<Vec<u32>, String>> {
        let lines = res!(self.cmd(&fmt!("UID SEARCH {}", criteria)).await);
        let last = lines.last().cloned().unwrap_or_default();
        if !last.contains(" OK ") { return Ok(Err(last)); }
        let mut out = Vec::new();
        for l in &lines {
            if let Some(rest) = l.strip_prefix("* SEARCH") {
                for t in rest.split_whitespace() {
                    out.push(res!(t.parse::<u32>(), Decode));
                }
            }
        }
        out.sort();
        Ok(Ok(out))
    }
}

/// A Maildir holding `msgs` (bytes, flags, INTERNALDATE in days after the epoch), served over
/// a socket, logged in with INBOX selected; with the UIDs given, in order, and the root.
async fn serve(name: &str, msgs: Vec<(Vec<u8>, MessageFlags, u64)>) -> Outcome<(Conn, Vec<u32>, PathBuf)> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_imap_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    let user = match res!(OneUser.lookup(USER)) {
        Some(u) => u,
        None    => return Err(err!("The test user did not resolve."; Test, Bug)),
    };
    res!(store.ensure_user(&user));
    let inbox = FolderName::new("INBOX");
    let mut uids: Vec<u32> = Vec::new();
    for (bytes, flags, day) in msgs {
        let t = UNIX_EPOCH + Duration::from_secs(day * 86_400 + 3_600);
        let MessageUid(u) = res!(store.append(&user, &inbox, &bytes, flags, Some(t)));
        uids.push(u);
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
            tokio::spawn(async move { let _ = srv.run(sock, peer).await; });
        }
    });
    let sock = res!(TcpStream::connect(addr).await, IO, Network);
    let (r, w) = sock.into_split();
    let mut c = Conn { r: BufReader::new(r), w, n: 0 };
    let mut greeting = String::new();
    res!(c.r.read_line(&mut greeting).await, IO, Network);
    res!(c.cmd(&fmt!("LOGIN {} {}", USER, PASS)).await);
    res!(c.cmd("SELECT INBOX").await);
    Ok((c, uids, root))
}

#[tokio::test]
async fn search_finds_exactly_what_matches_and_refuses_what_it_cannot() -> Outcome<()> {
    let msgs = corpus().iter().map(|m| (m.bytes(), m.flags, m.day)).collect();
    let (mut c, uids, root) = res!(serve("search", msgs).await);
    // Every UID, in corpus order, so `pick(&[0, 2])` is messages 1 and 3.
    let pick = |ix: &[usize]| -> Vec<u32> {
        let mut v: Vec<u32> = ix.iter().map(|i| uids[*i]).collect();
        v.sort();
        v
    };

    let uid_range = fmt!("UID {}:{}", uids[1], uids[2]);
    let cases: Vec<(&str, Vec<u32>)> = vec![
        ("ALL",                                         pick(&[0, 1, 2, 3])),
        ("HEADER Message-ID \"<c3@example.org>\"",      pick(&[2])),
        ("HEADER message-id <b2@EXAMPLE.net>",          pick(&[1])),
        ("HEADER Message-ID \"<nowhere@example>\"",     pick(&[])),
        ("HEADER Cc \"\"",                              pick(&[1, 3])),
        ("FROM bob",                                    pick(&[0, 2])),
        ("TO team@",                                    pick(&[2])),
        ("CC example.com",                              pick(&[1])),
        ("BCC anyone",                                  pick(&[])),
        ("SUBJECT LUNCH",                               pick(&[1, 2])),
        ("BODY report",                                 pick(&[1])),
        ("TEXT report",                                 pick(&[0, 1])),
        ("SINCE 3-Jan-2024",                            pick(&[2, 3])),
        ("BEFORE 3-Jan-2024",                           pick(&[0, 1])),
        ("ON \"2-Jan-2024\"",                           pick(&[1])),
        ("LARGER 200",                                  pick(&[2])),
        ("SMALLER 200",                                 pick(&[0, 1, 3])),
        ("NOT FROM bob",                                pick(&[1, 3])),
        ("OR FROM carol FROM erin",                     pick(&[1, 3])),
        ("FROM bob SUBJECT lunch",                      pick(&[2])),
        ("(FROM bob) (OR SEEN DRAFT)",                  pick(&[0, 2])),
        ("SEEN",                                        pick(&[0, 2])),
        ("UNSEEN",                                      pick(&[1, 3])),
        ("ANSWERED",                                    pick(&[2])),
        ("UNANSWERED",                                  pick(&[0, 1, 3])),
        ("DRAFT",                                       pick(&[3])),
        ("UNDRAFT",                                     pick(&[0, 1, 2])),
        ("UNFLAGGED",                                   pick(&[0, 1, 2, 3])),
        ("UNDELETED",                                   pick(&[0, 1, 2, 3])),
        ("CHARSET UTF-8 SUBJECT report",                pick(&[0])),
        (uid_range.as_str(),                            pick(&[1, 2])),
        ("2:3",                                         pick(&[1, 2])),
    ];
    let mut failures = Vec::new();
    for (q, want) in &cases {
        match res!(c.uid_search(q).await) {
            Ok(got) if &got == want => (),
            other => failures.push(fmt!("{}: want {:?}, got {:?}", q, want, other)),
        }
    }
    for q in ["FOO", "HEADER Message-ID", "SINCE notadate", "OR FROM bob", "LARGER x",
        "KEYWORD", "(FROM bob", "CHARSET KOI8-R FROM bob"]
    {
        match res!(c.uid_search(q).await) {
            Err(line) if line.contains(" BAD ") || line.contains(" NO ") => (),
            other => failures.push(fmt!("{}: want BAD, got {:?}", q, other)),
        }
    }
    // Every RFC 3501 key parses and is answered OK, alone and as the client sends it.
    for q in ["ALL", "ANSWERED", "BCC x", "BEFORE 1-Jan-2030", "BODY x", "CC x", "DELETED",
        "DRAFT", "FLAGGED", "FROM x", "HEADER X-Foo y", "KEYWORD $Junk", "LARGER 1", "NEW",
        "NOT SEEN", "OLD", "ON 1-Jan-2024", "OR SEEN FLAGGED", "RECENT", "SEEN",
        "SENTBEFORE 1-Jan-2030", "SENTON 1-Jan-2024", "SENTSINCE 1-Jan-2000", "SINCE 1-Jan-2000",
        "SMALLER 10000", "SUBJECT x", "TEXT x", "TO x", "UID 1:*", "UNANSWERED", "UNDELETED",
        "UNDRAFT", "UNFLAGGED", "UNKEYWORD $Junk", "UNSEEN", "1:*", "1,2:3", "(SEEN FLAGGED)",
        "CHARSET US-ASCII ALL", "charset utf-8 not deleted", "uid 1:* undeleted"]
    {
        if let Err(line) = res!(c.uid_search(q).await) {
            failures.push(fmt!("{}: a standard key was refused: {}", q, line));
        }
    }
    // A synchronising literal, as Thunderbird sends a non-ASCII string.
    res!(c.w.write_all(b"lit UID SEARCH SUBJECT {5}\r\n").await, IO, Network);
    let mut cont = String::new();
    res!(c.r.read_line(&mut cont).await, IO, Network);
    res!(c.w.write_all(b"LUNCH\r\n").await, IO, Network);
    let lines = res!(read_until_tagged(&mut c.r, "lit").await);
    let want = fmt!("* SEARCH {} {}", uids[1], uids[2]);
    if !cont.starts_with('+') || !lines.iter().any(|l| *l == want) {
        failures.push(fmt!("SUBJECT literal: continuation {:?}, then {:?}", cont, lines));
    }
    let lines = res!(c.cmd("SEARCH FROM carol").await);
    if !lines.iter().any(|l| l == "* SEARCH 2") {
        failures.push(fmt!("SEARCH FROM carol by sequence: {:?}", lines));
    }
    assert!(failures.is_empty(), "SEARCH got these wrong:\n{}", failures.join("\n"));
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

/// A non-ASCII subject or sender name travels as RFC 2047 encoded-words, and a search must
/// match what the reader sees, not the encoding.
#[tokio::test]
async fn search_matches_header_values_with_their_encoded_words_decoded() -> Outcome<()> {
    let plain = MessageFlags::default();
    let msgs = vec![
        // Subject "Grüße aus Köln", base64.
        (b"From: Bob <bob@example.org>\r\nTo: alice@test.local\r\n\
            Subject: =?UTF-8?B?R3LDvMOfZSBhdXMgS8O2bG4=?=\r\n\r\nHallo.\r\n".to_vec(), plain, 19723),
        // From "Renée Dupont", Q-encoded.
        (b"From: =?utf-8?Q?Ren=C3=A9e_Dupont?= <renee@example.fr>\r\nTo: alice@test.local\r\n\
            Subject: Lunch\r\n\r\nMidi ?\r\n".to_vec(), plain, 19724),
        (b"From: carol@example.net\r\nTo: alice@test.local\r\n\
            Subject: Plain\r\n\r\nNothing encoded.\r\n".to_vec(), plain, 19725),
    ];
    let (mut c, uids, root) = res!(serve("search2047", msgs).await);
    let mut failures = Vec::new();

    // A non-ASCII needle arrives as a literal, as Thunderbird sends it.
    for (needle, want) in [("grüße", uids[0]), ("KÖLN", uids[0])] {
        let n = needle.as_bytes().len();
        res!(c.w.write_all(fmt!("lit UID SEARCH CHARSET UTF-8 SUBJECT {{{}}}\r\n", n).as_bytes()).await,
            IO, Network);
        let mut cont = String::new();
        res!(c.r.read_line(&mut cont).await, IO, Network);
        res!(c.w.write_all(needle.as_bytes()).await, IO, Network);
        res!(c.w.write_all(b"\r\n").await, IO, Network);
        let lines = res!(read_until_tagged(&mut c.r, "lit").await);
        if !lines.iter().any(|l| *l == fmt!("* SEARCH {}", want)) {
            failures.push(fmt!("SUBJECT {}: continuation {:?}, then {:?}", needle, cont, lines));
        }
    }
    let cases: Vec<(&str, Vec<u32>)> = vec![
        ("FROM \"Dupont\"",                    vec![uids[1]]),
        ("FROM \"e dupont\"",                  vec![uids[1]]),   // `_` is a space in Q
        ("HEADER From \"Renee Dupont\"",       vec![]),
        ("TEXT \"aus k\"",                     vec![uids[0]]),
        ("SUBJECT \"R3LD\"",                   vec![]),
        ("SUBJECT plain",                       vec![uids[2]]),
    ];
    for (q, want) in &cases {
        match res!(c.uid_search(q).await) {
            Ok(got) if &got == want => (),
            other => failures.push(fmt!("{}: want {:?}, got {:?}", q, want, other)),
        }
    }
    assert!(failures.is_empty(), "SEARCH missed decoded headers:\n{}", failures.join("\n"));
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[tokio::test]
async fn sent_keys_read_a_date_whose_weekday_has_no_comma() -> Outcome<()> {
    let plain = MessageFlags::default();
    let msgs = vec![
        (b"From: bob@example.org\r\nTo: alice@test.local\r\nSubject: comma\r\n\
            Date: Tue, 27 Oct 2026 10:00:00 +0000\r\n\r\nA.\r\n".to_vec(), plain, 20388),
        // Not RFC 5322, but sent by some clients.
        (b"From: bob@example.org\r\nTo: alice@test.local\r\nSubject: bare\r\n\
            Date: Tue 27 Oct 2026 10:00:00 +0000\r\n\r\nB.\r\n".to_vec(), plain, 20388),
        (b"From: bob@example.org\r\nTo: alice@test.local\r\nSubject: none\r\n\
            Date: 27 Oct 2026 10:00:00 +0000\r\n\r\nC.\r\n".to_vec(), plain, 20388),
    ];
    let (mut c, uids, root) = res!(serve("sentweekday", msgs).await);
    let mut failures = Vec::new();
    for q in ["SENTON 27-Oct-2026", "SENTSINCE 27-Oct-2026", "SENTBEFORE 28-Oct-2026"] {
        match res!(c.uid_search(q).await) {
            Ok(got) if got == uids => (),
            other => failures.push(fmt!("{}: want {:?}, got {:?}", q, uids, other)),
        }
    }
    assert!(failures.is_empty(), "SENT* missed a Date:\n{}", failures.join("\n"));
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
