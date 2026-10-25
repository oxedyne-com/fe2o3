//! An SMTP delivery waiting on a Maildir folder lock must not hold the async runtime.
//!
//! Steel's handler files an inbound message with `MailStore::append`, which waits on the
//! folder's flock. Called on a runtime worker, that wait takes the worker with it, and on a
//! single-worker runtime no other session, and no plain timer, moves until the lock goes.

use oxedyne_fe2o3_core::{
    file::ExclusiveLock,
    prelude::*,
};
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::{
    mail::{
        store::{
            FolderName,
            MailStore,
            MailUser,
            MessageFlags,
        },
        user::UserStore,
    },
    smtp::{
        handler::{
            HandlerOutcome,
            SmtpHandler,
            SmtpTransaction,
        },
        server::{
            SmtpMode,
            SmtpServer,
        },
    },
};

use std::{
    fs,
    path::PathBuf,
    sync::{
        mpsc,
        Arc,
    },
    time::Duration,
};

use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncWriteExt,
        BufReader,
    },
    net::{
        tcp::{
            OwnedReadHalf,
            OwnedWriteHalf,
        },
        TcpListener,
        TcpStream,
    },
};


const RCPT: &str = "alice@test.local";

fn alice() -> MailUser {
    MailUser {
        local:          fmt!("alice"),
        domain:         fmt!("test.local"),
        delivery_key:   fmt!("test.local/alice"),
        send_as:        Vec::new(),
    }
}

#[derive(Clone, Debug)]
struct OneUser;

impl UserStore for OneUser {
    fn authenticate(&self, _address: &str, _password: &str) -> Outcome<Option<MailUser>> {
        Ok(None)
    }

    fn lookup(&self, address: &str) -> Outcome<Option<MailUser>> {
        Ok(if address.eq_ignore_ascii_case(RCPT) { Some(alice()) } else { None })
    }
}

/// Files every inbound message in alice's INBOX, as Steel's handler does.
#[derive(Clone)]
struct Deliver(MaildirStore);

impl SmtpHandler for Deliver {
    fn deliver_inbound(&self, txn: SmtpTransaction) -> Outcome<HandlerOutcome> {
        let flags = MessageFlags { recent: true, ..MessageFlags::default() };
        res!(self.0.append(&alice(), &FolderName::new("INBOX"), &txn.raw_message, flags, None));
        Ok(HandlerOutcome::Accepted(fmt!("T1")))
    }

    fn submit_outbound(&self, _txn: SmtpTransaction) -> Outcome<HandlerOutcome> {
        Ok(HandlerOutcome::RejectPermanent(fmt!("not a submission server")))
    }

    fn rcpt_acceptable(&self, address: &str) -> bool {
        address.eq_ignore_ascii_case(RCPT)
    }
}

/// Reads one reply, multi-line included, and returns its last line.
async fn reply(r: &mut BufReader<OwnedReadHalf>) -> Outcome<String> {
    loop {
        let mut line = String::new();
        let n = res!(r.read_line(&mut line).await, IO, Network);
        if n == 0 {
            return Err(err!("The server closed mid-reply."; IO, Network));
        }
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok(line.trim_end().to_string());
        }
    }
}

async fn say(r: &mut BufReader<OwnedReadHalf>, w: &mut OwnedWriteHalf, text: &str) -> Outcome<String> {
    res!(w.write_all(text.as_bytes()).await, IO, Network);
    reply(r).await
}

async fn connect(addr: std::net::SocketAddr) -> Outcome<(BufReader<OwnedReadHalf>, OwnedWriteHalf)> {
    let sock = res!(TcpStream::connect(addr).await, IO, Network);
    let (r, w) = sock.into_split();
    let mut r = BufReader::new(r);
    res!(reply(&mut r).await);
    Ok((r, w))
}

#[derive(Debug)]
enum Event {
    Ticked,                 // a plain timer task ran
    OtherSession(String),   // the second session's reply to EHLO
    Delivered(String),      // the first session's reply to the end of DATA
    Failed(String),
}

/// The waiting delivery, the second session and the timer all share one runtime thread.
fn serve_on_one_thread(store: MaildirStore, tx: mpsc::Sender<Event>) -> Outcome<()> {
    let rt = res!(tokio::runtime::Builder::new_current_thread().enable_all().build(), IO);
    rt.block_on(async move {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
        let addr = res!(listener.local_addr(), IO, Network);
        let server = SmtpServer {
            handler:        Deliver(store),
            users:          OneUser,
            tls_acceptor:   None,
            hostname:       Arc::new(fmt!("mx.test.local")),
            mode:           SmtpMode::Receive,
        };
        tokio::spawn(async move {
            while let Ok((sock, peer)) = listener.accept().await {
                let srv = server.clone();
                tokio::spawn(async move { let _ = srv.run(sock, peer).await; });
            }
        });

        // Session A delivers into the locked INBOX, and waits there.
        let (mut ra, mut wa) = res!(connect(addr).await);
        for line in ["EHLO a.example\r\n", "MAIL FROM:<bob@a.example>\r\n",
            "RCPT TO:<alice@test.local>\r\n", "DATA\r\n"]
        {
            res!(say(&mut ra, &mut wa, line).await);
        }
        let tx_a = tx.clone();
        let a = tokio::spawn(async move {
            let msg = "From: bob@a.example\r\nTo: alice@test.local\r\nSubject: Hi\r\n\r\nHello.\r\n.\r\n";
            let ev = match say(&mut ra, &mut wa, msg).await {
                Ok(line)    => Event::Delivered(line),
                Err(e)      => Event::Failed(fmt!("session A: {}", e)),
            };
            let _ = tx_a.send(ev);
        });
        // Give A's delivery time to reach the store and block.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let tx_t = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = tx_t.send(Event::Ticked);
        });
        let ev = match connect(addr).await {
            Ok((mut rb, mut wb)) => match say(&mut rb, &mut wb, "EHLO b.example\r\n").await {
                Ok(line)    => Event::OtherSession(line),
                Err(e)      => Event::Failed(fmt!("session B EHLO: {}", e)),
            },
            Err(e) => Event::Failed(fmt!("session B connect: {}", e)),
        };
        let _ = tx.send(ev);
        let _ = a.await;
        Ok(())
    })
}

#[test]
fn a_delivery_waiting_on_a_folder_lock_holds_no_runtime_worker() -> Outcome<()> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_smtp_runtime_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    res!(store.ensure_user(&alice()));

    // Another holder of INBOX's lock: an IMAP session or another delivery, as the store sees it.
    let lock_path = root.join("test.local").join("alice").join("hematite-folder.lock");
    let lock = res!(ExclusiveLock::acquire(&lock_path));

    let (tx, rx) = mpsc::channel::<Event>();
    let worker = std::thread::spawn(move || serve_on_one_thread(store, tx));

    // While the lock is held, the other session and the timer must both get through.
    let mut ticked = false;
    let mut other: Option<String> = None;
    let mut failures: Vec<String> = Vec::new();
    while !(ticked && other.is_some()) {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Event::Ticked)           => ticked = true,
            Ok(Event::OtherSession(l))  => other = Some(l),
            Ok(ev)                      => { failures.push(fmt!("while locked: {:?}", ev)); break; }
            Err(_)                      => {
                failures.push(fmt!("while INBOX was locked, nothing else ran \
                    (timer ticked: {}, other session: {:?})", ticked, other));
                break;
            }
        }
    }
    if let Some(line) = &other {
        if !line.starts_with("250") {
            failures.push(fmt!("EHLO while INBOX was locked: {}", line));
        }
    }

    // Once the lock goes, the waiting delivery is filed and accepted.
    drop(lock);
    let mut delivered = false;
    while !delivered {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Event::Delivered(l)) => {
                if !l.starts_with("250") { failures.push(fmt!("end of DATA: {}", l)); }
                delivered = true;
            }
            Ok(Event::Failed(e))    => { failures.push(e); break; }
            Ok(_)                   => (),
            Err(_)                  => { failures.push(fmt!("the delivery never finished after the lock went")); break; }
        }
    }
    let _ = worker.join();
    let _ = fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
