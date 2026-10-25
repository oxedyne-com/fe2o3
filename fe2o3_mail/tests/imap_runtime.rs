//! A session waiting on a Maildir folder lock must not hold the async runtime.
//!
//! Every folder read in `MaildirStore` waits on the folder's flock. Run on a runtime
//! worker, that wait takes the worker with it, and on a single-worker runtime nothing
//! else moves: not another session on another folder, not a plain timer. Steel serves
//! its HTTPS side from the same runtime, so the whole server stalls.

use oxedyne_fe2o3_core::{
    file::ExclusiveLock,
    prelude::*,
};
use oxedyne_fe2o3_mail::maildir::MaildirStore;
use oxedyne_fe2o3_net::{
    imap::server::ImapServer,
    mail::{
        store::{
            FolderName,
            MailStore,
            MailUser,
        },
        user::UserStore,
    },
};

use std::{
    fs,
    path::PathBuf,
    sync::mpsc,
    time::Duration,
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

/// Sends `text` under `tag` and reads to the tagged reply, which it returns.
async fn command(
    r:      &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    w:      &mut tokio::net::tcp::OwnedWriteHalf,
    tag:    &str,
    text:   &str,
)
    -> Outcome<String>
{
    res!(w.write_all(fmt!("{} {}\r\n", tag, text).as_bytes()).await, IO, Network);
    loop {
        let mut line = String::new();
        let n = res!(r.read_line(&mut line).await, IO, Network);
        if n == 0 {
            return Err(err!("The server closed before answering {}.", tag; IO, Network));
        }
        if line.starts_with(&fmt!("{} ", tag)) {
            return Ok(line.trim_end().to_string());
        }
    }
}

async fn connect(addr: std::net::SocketAddr)
    -> Outcome<(BufReader<tokio::net::tcp::OwnedReadHalf>, tokio::net::tcp::OwnedWriteHalf)>
{
    let sock = res!(TcpStream::connect(addr).await, IO, Network);
    let (r, w) = sock.into_split();
    let mut r = BufReader::new(r);
    let mut greeting = String::new();
    res!(r.read_line(&mut greeting).await, IO, Network);
    Ok((r, w))
}

#[derive(Debug)]
enum Event {
    Ticked,                 // a plain timer task ran
    OtherFolder(String),    // the second session's STATUS INBOX reply
    LockedFolder(String),   // the first session's SELECT reply, once the lock is gone
    Failed(String),
}

/// Session A's SELECT, the second session and the timer all share one runtime thread.
fn serve_on_one_thread(store: MaildirStore, tx: mpsc::Sender<Event>) -> Outcome<()> {
    let rt = res!(tokio::runtime::Builder::new_current_thread().enable_all().build(), IO);
    rt.block_on(async move {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await, IO, Network);
        let addr = res!(listener.local_addr(), IO, Network);
        let server = ImapServer {
            store,
            users:      OneUser,
            hostname:   std::sync::Arc::new(fmt!("test.local")),
        };
        tokio::spawn(async move {
            while let Ok((sock, peer)) = listener.accept().await {
                let srv = server.clone();
                tokio::spawn(async move { let _ = srv.run(sock, peer).await; });
            }
        });

        // Session A logs in and then waits on the locked folder.
        let (mut ra, mut wa) = res!(connect(addr).await);
        res!(command(&mut ra, &mut wa, "a1", &fmt!("LOGIN {} {}", USER, PASS)).await);
        let tx_a = tx.clone();
        let a = tokio::spawn(async move {
            let ev = match command(&mut ra, &mut wa, "a2", "SELECT Projects").await {
                Ok(line)    => Event::LockedFolder(line),
                Err(e)      => Event::Failed(fmt!("session A: {}", e)),
            };
            let _ = tx_a.send(ev);
        });
        // Give A's SELECT time to reach the store and block.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let tx_t = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = tx_t.send(Event::Ticked);
        });
        let ev = match connect(addr).await {
            Ok((mut rb, mut wb)) => {
                match command(&mut rb, &mut wb, "b1", &fmt!("LOGIN {} {}", USER, PASS)).await {
                    Ok(_)   => match command(&mut rb, &mut wb, "b2", "STATUS INBOX (MESSAGES)").await {
                        Ok(line)    => Event::OtherFolder(line),
                        Err(e)      => Event::Failed(fmt!("session B STATUS: {}", e)),
                    },
                    Err(e)  => Event::Failed(fmt!("session B LOGIN: {}", e)),
                }
            }
            Err(e) => Event::Failed(fmt!("session B connect: {}", e)),
        };
        let _ = tx.send(ev);
        let _ = a.await;
        Ok(())
    })
}

#[test]
fn a_session_waiting_on_a_folder_lock_holds_no_runtime_worker() -> Outcome<()> {
    let root = std::env::temp_dir().join(fmt!("fe2o3_imap_runtime_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    res!(fs::create_dir_all(&root), IO, File);
    let store = res!(MaildirStore::new(PathBuf::from(&root), "test.local"));
    let user = match res!(OneUser.lookup(USER)) {
        Some(u) => u,
        None    => return Err(err!("The test user did not resolve."; Test, Bug)),
    };
    res!(store.ensure_user(&user));
    let projects = FolderName::new("Projects");
    res!(store.create_folder(&user, &projects));

    // Another holder of the folder lock: a delivery or another process, as the store sees it.
    let lock_path = root.join("test.local").join("alice").join(".Projects").join("hematite-folder.lock");
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
            Ok(Event::OtherFolder(l))   => other = Some(l),
            Ok(ev)                      => { failures.push(fmt!("while locked: {:?}", ev)); break; }
            Err(_)                      => {
                failures.push(fmt!("while the folder was locked, nothing else ran \
                    (timer ticked: {}, other session: {:?})", ticked, other));
                break;
            }
        }
    }
    if let Some(line) = &other {
        if !line.contains(" OK ") {
            failures.push(fmt!("STATUS INBOX while Projects was locked: {}", line));
        }
    }

    // Once the lock goes, the waiting session finishes its SELECT.
    drop(lock);
    let mut selected = false;
    while !selected {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Event::LockedFolder(l)) => {
                if !l.contains(" OK ") { failures.push(fmt!("SELECT Projects: {}", l)); }
                selected = true;
            }
            Ok(Event::Failed(e))    => { failures.push(e); break; }
            Ok(_)                   => (),
            Err(_)                  => { failures.push(fmt!("SELECT never finished after the lock went")); break; }
        }
    }
    let _ = worker.join();
    let _ = fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
