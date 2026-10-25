#![cfg(feature = "async")]
//! Submitting a message through a provider, which means proving the account is ours first.
//!
//! Delivery to a recipient's MX needs no credential: the receiving server takes the message because
//! it is responsible for the recipient. Submission through the *sender's own* provider is the
//! opposite -- the provider carries nothing until the sender authenticates -- and it is the
//! conversation every mail client actually has. The client could not have it at all until `submit`
//! existed, so this drives it end to end against a server that demands a login.
//!
//! The server here is a stand-in, spoken to over loopback in the clear, because what is under test
//! is the client's half of the exchange: does it read the mechanism list, choose one it can speak,
//! encode the credential correctly, and refuse to go on when the login is rejected.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_net::{
    imap::client::Security,
    mail::{
        store::MailUser,
        user::UserStore,
    },
    smtp::{
        client::{
            OutboundClient,
            SubmissionConfig,
            SubmissionFault,
        },
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
    tls::BoundedTlsAcceptor,
};

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        Mutex,
    },
    time::Duration,
};

use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncWriteExt,
        BufReader,
    },
    net::TcpListener,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ClientConfig,
        RootCertStore,
        ServerConfig,
        pki_types::{
            CertificateDer,
            PrivateKeyDer,
            PrivatePkcs8KeyDer,
        },
    },
};


// What the stand-in provider will accept.
const USER: &str = "alice@example.com";
const PASS: &str = "app-password-not-the-real-one";


/// A provider that demands a login, and remembers the conversation so the test can read it back.
/// `mechanisms` is advertised after `AUTH` in the `EHLO` reply, e.g. `"PLAIN LOGIN"`.
async fn fake_provider(
    mechanisms: &'static str,
    accept:     bool,
)
    -> Outcome<(SocketAddr, Arc<Mutex<Vec<String>>>)>
{
    let listener = res!(TcpListener::bind("127.0.0.1:0").await
        .map_err(|e| err!(e, "Binding the stand-in provider."; IO, Network)));
    let addr = res!(listener.local_addr()
        .map_err(|e| err!(e, "Reading the stand-in provider's address."; IO, Network)));

    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();

    tokio::spawn(async move {
        let (sock, _) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => return,
        };
        let (r, mut w) = sock.into_split();
        let mut lines = BufReader::new(r).lines();

        let _ = w.write_all(b"220 provider.example.com ESMTP\r\n").await;

        let mut in_data = false;
        let mut await_user = false;
        let mut await_pass = false;

        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(mut g) = log.lock() {
                g.push(line.clone());
            }

            if in_data {
                if line == "." {
                    in_data = false;
                    let _ = w.write_all(b"250 2.0.0 Ok: queued as STANDIN1\r\n").await;
                }
                continue;
            }
            if await_user {
                await_user = false;
                await_pass = true;
                let _ = w.write_all(b"334 UGFzc3dvcmQ6\r\n").await;   // "Password:"
                continue;
            }
            if await_pass {
                await_pass = false;
                let _ = w.write_all(if accept {
                    &b"235 2.7.0 Accepted\r\n"[..]
                } else {
                    &b"535 5.7.8 Username and Password not accepted\r\n"[..]
                }).await;
                continue;
            }

            let upper = line.to_uppercase();
            if upper.starts_with("EHLO") {
                let _ = w.write_all(
                    fmt!("250-provider.example.com\r\n250-SIZE 35882577\r\n250-AUTH {}\r\n250 8BITMIME\r\n",
                        mechanisms).as_bytes()).await;
            } else if upper.starts_with("AUTH PLAIN") {
                let _ = w.write_all(if accept {
                    &b"235 2.7.0 Accepted\r\n"[..]
                } else {
                    &b"535 5.7.8 Username and Password not accepted\r\n"[..]
                }).await;
            } else if upper.starts_with("AUTH LOGIN") {
                await_user = true;
                let _ = w.write_all(b"334 VXNlcm5hbWU6\r\n").await;   // "Username:"
            } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                let _ = w.write_all(b"250 2.1.0 Ok\r\n").await;
            } else if upper.starts_with("DATA") {
                in_data = true;
                let _ = w.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n").await;
            } else if upper.starts_with("QUIT") {
                let _ = w.write_all(b"221 2.0.0 Bye\r\n").await;
                return;
            } else {
                let _ = w.write_all(b"250 2.0.0 Ok\r\n").await;
            }
        }
    });

    Ok((addr, seen))
}

/// One body line deliberately begins with a full stop, so dot-stuffing is under test on every case.
fn body() -> Vec<u8> {
    let mut s = String::new();
    s.push_str("From: Alice <alice@example.com>\r\n");
    s.push_str("To: Bob <bob@example.net>\r\n");
    s.push_str("Subject: Hello\r\n");
    s.push_str("\r\n");
    s.push_str("A line.\r\n");
    s.push_str(".A line that begins with a full stop, which must survive dot-stuffing.\r\n");
    s.into_bytes()
}

/// A runtime per case. `test_it` takes a closure that outlives this function, so the runtime is
/// built inside each one rather than borrowed from around them.
fn runtime() -> Outcome<tokio::runtime::Runtime> {
    tokio::runtime::Runtime::new()
        .map_err(|e| err!(e, "Building a runtime."; IO, Init))
}

fn cfg(addr: SocketAddr) -> SubmissionConfig {
    SubmissionConfig::new("provider.example.com", addr.port(), Security::Plain, USER, PASS)
        .with_addr(addr)
        .with_timeout(Duration::from_secs(10))
}


// ── fe2o3's own submission server, for the account check ──

/// Counts what reaches it, so a test can prove nothing was submitted.
#[derive(Clone, Default)]
struct Counting {
    submitted: Arc<Mutex<usize>>,
}

impl SmtpHandler for Counting {
    fn deliver_inbound(&self, _txn: SmtpTransaction) -> Outcome<HandlerOutcome> {
        Ok(HandlerOutcome::RejectPermanent(fmt!("not this port")))
    }
    fn submit_outbound(&self, _txn: SmtpTransaction) -> Outcome<HandlerOutcome> {
        match self.submitted.lock() {
            Ok(mut n) => *n += 1,
            Err(_)    => return Err(err!("The submission count was poisoned."; Lock, Poisoned)),
        }
        Ok(HandlerOutcome::Accepted(fmt!("REAL1")))
    }
    fn rcpt_acceptable(&self, _address: &str) -> bool { true }
}

/// One account, `USER` with `PASS`.
#[derive(Clone)]
struct OneUser;

impl UserStore for OneUser {
    fn authenticate(&self, address: &str, password: &str) -> Outcome<Option<MailUser>> {
        if !address.eq_ignore_ascii_case(USER) || password != PASS {
            return Ok(None);
        }
        self.lookup(address)
    }
    fn lookup(&self, _address: &str) -> Outcome<Option<MailUser>> {
        Ok(Some(MailUser {
            local:          fmt!("alice"),
            domain:         fmt!("example.com"),
            delivery_key:   fmt!("alice"),
            send_as:        Vec::new(),
        }))
    }
}

/// fe2o3's `SmtpServer` in submission mode, STARTTLS before AUTH, with a certificate for
/// `localhost` that the returned client trusts and no public root would.
async fn real_submission() -> Outcome<(SocketAddr, OutboundClient, Arc<Mutex<usize>>)> {
    oxedyne_fe2o3_net::tls::ensure_crypto_provider();
    let cert = res!(rcgen::generate_simple_self_signed(vec![fmt!("localhost")])
        .map_err(|e| err!(e, "Making a test certificate."; Test, Init)));
    let der  = res!(cert.serialize_der()
        .map_err(|e| err!(e, "Serialising the test certificate."; Test, Encode)));
    let key  = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.serialize_private_key_der()));
    let scfg = res!(ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(der.clone())], key)
        .map_err(|e| err!(e, "Building the test server config."; Test, Init)));
    let mut roots = RootCertStore::empty();
    res!(roots.add(CertificateDer::from(der))
        .map_err(|e| err!(e, "Trusting the test certificate."; Test, Init)));
    let ccfg = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();

    let handler = Counting::default();
    let submitted = handler.submitted.clone();
    let server = SmtpServer {
        handler,
        users:          OneUser,
        tls_acceptor:   Some(BoundedTlsAcceptor::unbounded(TlsAcceptor::from(Arc::new(scfg)))),
        hostname:       Arc::new(fmt!("localhost")),
        mode:           SmtpMode::Submission,
    };
    let listener = res!(TcpListener::bind("127.0.0.1:0").await
        .map_err(|e| err!(e, "Binding the submission server."; Test, IO)));
    let addr = res!(listener.local_addr()
        .map_err(|e| err!(e, "Reading its address."; Test, IO)));
    tokio::spawn(async move {
        while let Ok((sock, peer)) = listener.accept().await {
            let s = server.clone();
            tokio::spawn(async move { let _ = s.run(sock, peer).await; });
        }
    });
    let mut client = res!(OutboundClient::with_system_roots("daimond.test"));
    client.tls_config = Arc::new(ccfg);
    Ok((addr, client, submitted))
}

fn real_cfg(addr: SocketAddr, password: &str) -> SubmissionConfig {
    SubmissionConfig::new("localhost", addr.port(), Security::StartTls, USER, password)
        .with_addr(addr)
        .with_timeout(Duration::from_secs(10))
}

fn count(n: &Arc<Mutex<usize>>) -> Outcome<usize> {
    match n.lock() {
        Ok(g)  => Ok(*g),
        Err(_) => Err(err!("The submission count was poisoned."; Lock, Poisoned)),
    }
}

pub fn test_smtp_submit(filter: &'static str) -> Outcome<()> {


    res!(test_it(filter, &["Submit with AUTH PLAIN", "all", "smtp", "submit"], || {
        res!(runtime()).block_on(async {
            let (addr, seen) = res!(fake_provider("PLAIN LOGIN", true).await);
            let client = res!(OutboundClient::with_system_roots("daimond.test"));
            let qid = res!(client.submit(
                &cfg(addr),
                "alice@example.com",
                &[fmt!("bob@example.net")],
                &body(),
            ).await);
            req!(true, qid.contains("STANDIN1"));

            let lines = match seen.lock() {
                Ok(g) => g.clone(),
                Err(_) => return Err(err!("The provider's log was poisoned."; Lock, Poisoned)),
            };
            // PLAIN is offered first and must be the one chosen, carrying the credential in the
            // command itself: an empty authorisation identity, the account, then the password.
            let auth = match lines.iter().find(|l| l.to_uppercase().starts_with("AUTH PLAIN")) {
                Some(l) => l.clone(),
                None => return Err(err!(
                    "The client never sent AUTH PLAIN. It said: {:?}", lines; Test, Missing)),
            };
            let b64 = auth["AUTH PLAIN ".len()..].trim().to_string();
            let raw = res!(base64::decode(&b64));
            let expect = fmt!("\0{}\0{}", USER, PASS);
            req!(expect.as_bytes().to_vec(), raw);

            // A line that begins with a full stop must reach the server doubled, or it would have
            // ended the message early.
            req!(true, lines.iter().any(|l| l.starts_with("..A line that begins")));
            Ok(())
        })
    }));

    res!(test_it(filter, &["Submit with AUTH LOGIN", "all", "smtp", "submit"], || {
        res!(runtime()).block_on(async {
            // A provider that offers only LOGIN. The client must fall back to it rather than give
            // up because its first choice was absent.
            let (addr, seen) = res!(fake_provider("LOGIN", true).await);
            let client = res!(OutboundClient::with_system_roots("daimond.test"));
            let qid = res!(client.submit(
                &cfg(addr),
                "alice@example.com",
                &[fmt!("bob@example.net")],
                &body(),
            ).await);
            req!(true, qid.contains("STANDIN1"));

            let lines = match seen.lock() {
                Ok(g) => g.clone(),
                Err(_) => return Err(err!("The provider's log was poisoned."; Lock, Poisoned)),
            };
            req!(true, lines.iter().any(|l| l.to_uppercase().starts_with("AUTH LOGIN")));
            // The account and the password each go over on their own line, base64 and nothing more.
            req!(true, lines.iter().any(|l| base64::decode(l.trim())
                .map(|b| b == USER.as_bytes()).unwrap_or(false)));
            req!(true, lines.iter().any(|l| base64::decode(l.trim())
                .map(|b| b == PASS.as_bytes()).unwrap_or(false)));
            Ok(())
        })
    }));

    res!(test_it(filter, &["A refused credential is an error, not a send", "all", "smtp", "submit"], || {
        res!(runtime()).block_on(async {
            let (addr, seen) = res!(fake_provider("PLAIN LOGIN", false).await);
            let client = res!(OutboundClient::with_system_roots("daimond.test"));
            let result = client.submit(
                &cfg(addr),
                "alice@example.com",
                &[fmt!("bob@example.net")],
                &body(),
            ).await;
            if result.is_ok() {
                return Err(err!(
                    "The provider refused the credential and the client reported success.";
                    Test, Invalid));
            }
            // And it must not have gone on to offer the message anyway.
            let lines = match seen.lock() {
                Ok(g) => g.clone(),
                Err(_) => return Err(err!("The provider's log was poisoned."; Lock, Poisoned)),
            };
            if lines.iter().any(|l| l.to_uppercase().starts_with("MAIL FROM")) {
                return Err(err!(
                    "The client sent MAIL FROM after its login was rejected.";
                    Test, Invalid));
            }
            Ok(())
        })
    }));

    res!(test_it(filter, &["A server offering no mechanism is refused", "all", "smtp", "submit"], || {
        res!(runtime()).block_on(async {
            // Advertise a mechanism this client cannot speak. Submitting anyway would mean sending
            // the password into a conversation that cannot use it.
            let (addr, _seen) = res!(fake_provider("XOAUTH2", true).await);
            let client = res!(OutboundClient::with_system_roots("daimond.test"));
            let result = client.submit(
                &cfg(addr),
                "alice@example.com",
                &[fmt!("bob@example.net")],
                &body(),
            ).await;
            if result.is_ok() {
                return Err(err!(
                    "The client claimed to submit through a server whose only mechanism it cannot \
                    speak."; Test, Invalid));
            }
            Ok(())
        })
    }));


    res!(test_it(filter, &["Verify against fe2o3's own submission server", "all", "smtp", "submit", "verify"], || {
        res!(runtime()).block_on(async {
            let (addr, client, submitted) = res!(real_submission().await);
            res!(client.verify(&real_cfg(addr, PASS)).await);
            req!(0, res!(count(&submitted)), "the check submitted a message");
            // The same account sends, so the check passed for the reason a send would.
            let qid = res!(client.submit(&real_cfg(addr, PASS), USER,
                &[fmt!("bob@example.net")], &body()).await);
            req!(true, qid.contains("REAL1"), "the queue id: {}", qid);
            req!(1, res!(count(&submitted)));
            Ok(())
        })
    }));

    res!(test_it(filter, &["Verify names a refused password as auth", "all", "smtp", "submit", "verify"], || {
        res!(runtime()).block_on(async {
            let (addr, client, submitted) = res!(real_submission().await);
            let e = match client.verify(&real_cfg(addr, "not-the-password")).await {
                Ok(())  => return Err(err!("A wrong password passed the check."; Test, Invalid)),
                Err(e)  => e,
            };
            req!(SubmissionFault::Auth, SubmissionFault::of(&e), "{:?}: {}", e.tags(), e);
            req!(0, res!(count(&submitted)));
            Ok(())
        })
    }));

    Ok(())
}
