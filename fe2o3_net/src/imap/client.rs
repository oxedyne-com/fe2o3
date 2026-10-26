//! Client-side IMAP4rev1 (RFC 3501) for reading a remote mailbox.
//!
//! The counterpart to [`crate::imap::server`]: where that serves a local
//! Maildir to a mail reader, this connects *out* to somebody else's IMAP
//! server and pulls messages down. It is what a program needs in order to
//! treat a hosted mailbox (Gmail, Fastmail, a corporate Dovecot) as a
//! source of raw RFC 5322 messages.
//!
//! What it does:
//!
//! - Connects with implicit TLS (the 993 case), STARTTLS (143), or plain.
//! - `LOGIN` with a password, or `AUTHENTICATE XOAUTH2` with a bearer
//!   token, whichever the account requires.
//! - `CAPABILITY`, `LIST`, `SELECT`/`EXAMINE`, `UID SEARCH`, `UID FETCH`,
//!   `UID STORE` (as `+FLAGS`/`-FLAGS` deltas only), `UID COPY`, `UID MOVE`,
//!   `UID EXPUNGE`, `APPEND` (with `APPENDUID`), `LOGOUT`, over a range-compressed
//!   [`UidSet`].
//!
//! A failure says what kind it is in its tags, so a caller explaining it to a
//! person reads the tags and never the words, which for a dropped connection
//! and a refused certificate both mention TLS (D-20261006-09):
//!
//! - `Unauthorised`: the credential was refused, or password login is disabled.
//! - `NotFound`: the mailbox does not exist (a `NO` to `SELECT`/`EXAMINE`).
//! - `Unreachable`: nothing would take the connection.
//! - `Interrupted`: the connection dropped, or the server said it was briefly
//!   unavailable. A fresh attempt may succeed; [`is_interrupted`] asks.
//! - `Security` with `Init`: a certificate was not accepted in the handshake.
//! - `Timeout`: the server went quiet past the per-IO deadline.
//!
//! A dropped connection is met with one fresh one, once per client
//! ([`ImapConfig::retries`]): the client reconnects, logs in again, re-opens the
//! mailbox it had open, and repeats the command, so the caller sees only the
//! answer. Only commands that change nothing are repeated: `CAPABILITY`,
//! `LOGIN`, `AUTHENTICATE`, `LIST`, `SELECT`/`EXAMINE`, `UID SEARCH`, `UID FETCH`
//! (which peeks). `UID STORE` and `APPEND` are never repeated, because whether a
//! dropped write took effect cannot be known. A mailbox re-opened under a new
//! `UIDVALIDITY` has renumbered its messages, and the retry refuses with
//! `Mismatch` rather than answer a UID question on the wrong numbering. A
//! timeout is not retried, so the worst case stays one deadline per IO.
//!
//! What it does not do: `IDLE`, `CONDSTORE`, `QRESYNC`, compression. A
//! caller wanting to know what changed polls, which is what a caller
//! without a long-lived socket has to do anyway.
//!
//! The awkward part of IMAP is the wire format, and it is handled once,
//! here. A response is a line, except when it is a line with a literal
//! (`{1234}` followed by exactly that many raw bytes, which may contain
//! anything at all including CRLF) spliced into the middle of it. So the
//! reader assembles a *logical* line: the text with each literal lifted
//! out into a side queue, which the tokeniser then puts back in order.
//! Parsing the text as a line and hoping no message body contains a CRLF
//! is the classic way to write an IMAP client that works until somebody
//! sends you an attachment.
//!
//! # Example
//!
//! ```no_run
//! # use oxedyne_fe2o3_core::prelude::*;
//! # use oxedyne_fe2o3_net::imap::client::{FetchWhat, ImapClient, ImapConfig, Security, UidSet};
//! # async fn f() -> Outcome<()> {
//! let cfg = ImapConfig::new("imap.example.com", 993, Security::ImplicitTls);
//! let mut c = res!(ImapClient::connect(&cfg).await);
//! res!(c.login("alice@example.com", "app-password").await);
//! let inbox = res!(c.select("INBOX").await);
//! let uids  = res!(c.uid_search(&fmt!("UID {}:*", inbox.uid_next.saturating_sub(10))).await);
//! for msg in res!(c.uid_fetch(&UidSet::from(&uids), FetchWhat::Full).await) {
//!     println!("{} bytes, flags {:?}", msg.body.len(), msg.flags);
//! }
//! res!(c.logout().await);
//! # Ok(())
//! # }
//! ```
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    email,
    imap::uid_set,
    tls::{
        self,
        ClientStream,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use secrecy::{
    ExposeSecret,
    SecretString,
};

use tokio::{
    io::{
        AsyncBufReadExt,
        AsyncReadExt,
        AsyncWriteExt,
        BufReader,
    },
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::rustls::ClientConfig;


pub use crate::imap::uid_set::UidSet;


// Generous, because a large `UID FETCH` against a slow mailbox is a legitimate
// multi-second read.
pub const IMAP_CLIENT_TIMEOUT: Duration = Duration::from_secs(60);

// A server announcing a 4 GB literal is either broken or hostile, and either way
// the client should not try to allocate for it.
pub const MAX_LITERAL_BYTES: usize = 64 * 1024 * 1024;

// One fresh connection per client for a dropped one. A second drop is the server's or the
// network's state, not chance, and the caller is told.
pub const IMAP_CLIENT_RETRIES: u8 = 1;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CONFIGURATION                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// How the connection is protected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Security {
    ImplicitTls,    // TLS from the first byte, the usual case on port 993
    // In the clear, then `STARTTLS` before authenticating, on port 143. Where the
    // upgrade fails the client refuses to send the credential.
    StartTls,
    Plain,          // a test server on loopback, and nothing else
}

/// Where to connect and how.
#[derive(Clone, Debug)]
pub struct ImapConfig {
    pub host:       String,     // also the name the certificate is validated against
    pub port:       u16,        // conventionally 993 (implicit TLS) or 143 (STARTTLS)
    pub security:   Security,
    pub timeout:    Duration,   // per IO
    // Dialled instead of resolving `host`; the certificate is still validated against `host`, so
    // pinning the address weakens nothing. A server connecting to a host its *user* named must
    // resolve the name, satisfy itself the answer is somewhere it is willing to go (see
    // crate::addr::resolve_public), and then connect to that address -- not re-resolve the name
    // and hope for the same answer twice. This field is how it does the last part.
    pub addr:       Option<SocketAddr>,
    pub retries:    u8,         // fresh connections a client may make for dropped ones
    // The roots the server's certificate is checked against; `None` is what the operating system
    // trusts (tls::default_client_config, which honours SSL_CERT_FILE and SSL_CERT_DIR). A
    // private CA is added with tls::ClientRoots or tls::client_config_trusting.
    pub tls:        Option<Arc<ClientConfig>>,
}

impl ImapConfig {

    /// The default timeout, and no pinned address.
    pub fn new<S: Into<String>>(host: S, port: u16, security: Security) -> Self {
        Self {
            host:     host.into(),
            port,
            security,
            timeout:  IMAP_CLIENT_TIMEOUT,
            addr:     None,
            retries:  IMAP_CLIENT_RETRIES,
            tls:      None,
        }
    }

    pub fn with_tls(mut self, tls_cfg: Arc<ClientConfig>) -> Self {
        self.tls = Some(tls_cfg);
        self
    }

    /// Zero turns the retry of a dropped connection off.
    pub fn with_retries(mut self, retries: u8) -> Self {
        self.retries = retries;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_addr(mut self, addr: SocketAddr) -> Self {
        self.addr = Some(addr);
        self
    }
}


pub use crate::imap::types::{
    AppendUid,
    CopyUid,
    FetchChanged,
    FetchWhat,
    FetchedMessage,
    FolderStatus,
    MailboxInfo,
    MailboxStatus,
    SelectWith,
    SpecialUse,
};

/// The completion status of a tagged command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Status {
    Ok,
    No,     // refused
    Bad,    // not understood
}

/// One logical response line: the text, with every literal lifted out
/// into `literals` in the order it appeared.
#[derive(Clone, Debug)]
struct RawLine {
    text:       String,         // each literal appears only as its `{n}` marker
    literals:   Vec<Vec<u8>>,   // the payloads, in order
}

/// Everything one tagged command produced. Only a successful command
/// yields one: a `NO` or a `BAD` becomes an error carrying the server's
/// own words, so there is no status to carry here.
#[derive(Clone, Debug)]
struct Response {
    untagged:   Vec<RawLine>,   // the `*` lines that arrived before completion
    text:       String,         // the completion line, which may carry a response code
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CLIENT                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// A connected IMAP client. One connection, one mailbox selected at a
/// time, exactly as the protocol is.
pub struct ImapClient {
    // Buffered, because literal reads need byte precision. An `Option` only so that a STARTTLS
    // upgrade can move the socket out, wrap it and put it back; it is `None` for no other reason,
    // and any use of a `None` stream is a failed upgrade and a dead connection.
    stream:     Option<BufReader<ClientStream>>,
    tag:        u32,            // monotonic, never reused
    caps:       Vec<String>,    // as last advertised, upper-cased
    timeout:    Duration,       // per IO, from the config
    host:       String,         // kept for error messages and the TLS upgrade
    // How to stand the conversation up again after a drop.
    cfg:        ImapConfig,
    tls_cfg:    Arc<ClientConfig>,
    auth:       Option<Auth>,   // as last accepted
    enabled:    Vec<String>,    // RFC 5161 extensions the server said it enabled, upper-cased
    opened:     Option<Opened>, // the mailbox last opened
    retries:    u8,             // fresh connections left
}

/// The credential a reconnect logs in with again. Held as a secret, so it is wiped on drop and
/// never printed.
enum Auth {
    Login   { user: String, pass: SecretString },
    XOAuth2 { user: String, token: SecretString },
}

/// The mailbox a reconnect re-opens, and the numbering its UIDs were read under.
struct Opened {
    name:           String,
    read_only:      bool,           // as asked: `EXAMINE` rather than `SELECT`
    with:           SelectWith,     // repeated as asked
    uid_validity:   Option<u32>,    // None until an open has answered
    modseq:         bool,           // the open answered a HIGHESTMODSEQ, so a fetch asks MODSEQ
}

/// Is this a transient failure -- a dropped connection, a server briefly unavailable -- that a
/// fresh attempt may cure?
pub fn is_interrupted(e: &Error<ErrTag>) -> bool {
    e.tags().contains(&ErrTag::Interrupted)
}

impl ImapClient {

    /// Protects the connection as configured and reads the greeting. Does not authenticate. The
    /// certificate is checked against [`ImapConfig::tls`] where set, and what the operating system
    /// trusts otherwise.
    pub async fn connect(cfg: &ImapConfig) -> Outcome<Self> {
        let tls_cfg = match &cfg.tls {
            Some(t) => t.clone(),
            None    => Arc::new(res!(tls::default_client_config())),
        };
        Self::connect_with(cfg, tls_cfg).await
    }

    /// Connect using a caller-supplied rustls config, for a private CA or a pinned root. It
    /// overrides [`ImapConfig::tls`].
    pub async fn connect_with(
        cfg:        &ImapConfig,
        tls_cfg:    Arc<ClientConfig>,
    )
        -> Outcome<Self>
    {
        let mut left = cfg.retries;
        loop {
            match Self::open(cfg, tls_cfg.clone()).await {
                Ok(mut c) => {
                    c.retries = left;
                    return Ok(c);
                },
                Err(e) if left > 0 && is_interrupted(&e) => {
                    warn!("IMAP {}: the connection dropped while opening, trying once more: {}",
                        cfg.host, e);
                    left -= 1;
                },
                Err(e) => return Err(e),
            }
        }
    }

    /// One attempt at a connection, with no retry of its own.
    async fn open(
        cfg:        &ImapConfig,
        tls_cfg:    Arc<ClientConfig>,
    )
        -> Outcome<Self>
    {
        let addr = match cfg.addr {
            Some(a) => a.to_string(),
            None    => fmt!("{}:{}", cfg.host, cfg.port),
        };
        let plain = match timeout(cfg.timeout, TcpStream::connect(&addr)).await {
            Ok(Ok(s))  => s,
            Ok(Err(e)) => return Err(tls::dial_fault(e,
                fmt!("Connecting to IMAP server {}", addr))),
            Err(_)     => return Err(err!(
                "Timeout connecting to IMAP server {}.", addr;
                IO, Network, Timeout)),
        };

        let stream = match cfg.security {
            Security::ImplicitTls =>
                res!(tls::upgrade(plain, &cfg.host, tls_cfg.clone(), cfg.timeout).await),
            Security::StartTls | Security::Plain =>
                ClientStream::Plain(plain),
        };

        let mut client = Self {
            stream:  Some(BufReader::new(stream)),
            tag:     0,
            caps:    Vec::new(),
            timeout: cfg.timeout,
            host:    cfg.host.clone(),
            cfg:     cfg.clone(),
            tls_cfg: tls_cfg.clone(),
            auth:    None,
            enabled: Vec::new(),
            opened:  None,
            retries: 0,
        };

        // The greeting: an untagged OK, PREAUTH or BYE. It may carry a
        // CAPABILITY list, saving a round trip.
        let greeting = res!(client.read_line().await);
        let up = greeting.text.to_uppercase();
        if up.starts_with("* BYE") {
            return Err(err!(
                "IMAP server {} refused the connection: {}", cfg.host, greeting.text;
                IO, Network, Wire));
        }
        if !up.starts_with("* OK") && !up.starts_with("* PREAUTH") {
            return Err(err!(
                "Expected an IMAP greeting from {}, got: {}", cfg.host, greeting.text;
                IO, Network, Wire));
        }
        client.absorb_capabilities(&greeting.text);

        if cfg.security == Security::StartTls {
            res!(client.starttls(tls_cfg).await);
        }
        if client.caps.is_empty() {
            res!(client.capability_once().await);
        }
        Ok(client)
    }

    /// Upgrade a plain connection in place. The credentials have not been
    /// sent yet, and if the upgrade fails they never will be.
    async fn starttls(&mut self, tls_cfg: Arc<ClientConfig>) -> Outcome<()> {
        if self.caps.is_empty() {
            res!(self.capability_once().await);
        }
        if !self.has_cap("STARTTLS") {
            return Err(err!(
                "IMAP server {} does not offer STARTTLS, and the connection \
                is not otherwise protected.", self.host;
                Security, Unimplemented));
        }
        res!(self.command("STARTTLS").await);

        // Anything the server pipelined behind its STARTTLS response is
        // sitting in the read buffer, unprotected, and would be indis-
        // tinguishable from what the TLS peer says next. That is a
        // downgrade attack (RFC 2595 §3.1), so refuse rather than discard.
        let buffered = res!(self.take_stream());
        if !buffered.buffer().is_empty() {
            return Err(err!(
                "IMAP server {} sent data after its STARTTLS response, before \
                the handshake. Refusing to continue.", self.host;
                IO, Network, Security));
        }
        let plain = match buffered.into_inner().into_plain() {
            Some(s) => s,
            None    => return Err(err!(
                "STARTTLS issued on an already-protected connection.";
                Invalid, Bug)),
        };
        let upgraded = res!(tls::upgrade(plain, &self.host, tls_cfg, self.timeout).await);
        self.stream = Some(BufReader::new(upgraded));

        // Capabilities before and after TLS are allowed to differ, and the
        // pre-TLS set must not be trusted: re-ask.
        self.caps.clear();
        res!(self.capability_once().await);
        Ok(())
    }

    /// The live stream, or an error if a failed upgrade has left none.
    fn stream_mut(&mut self) -> Outcome<&mut BufReader<ClientStream>> {
        match self.stream.as_mut() {
            Some(s) => Ok(s),
            None    => Err(err!(
                "The IMAP connection to {} was dropped by a failed TLS \
                upgrade.", self.host;
                IO, Network, Missing)),
        }
    }

    /// Move the stream out, for the one operation that needs to own it.
    fn take_stream(&mut self) -> Outcome<BufReader<ClientStream>> {
        match self.stream.take() {
            Some(s) => Ok(s),
            None    => Err(err!(
                "The IMAP connection to {} was dropped by a failed TLS \
                upgrade.", self.host;
                IO, Network, Missing)),
        }
    }

    /// A refresh replaces rather than accumulates: the set a server offers
    /// legitimately changes across a STARTTLS or a login, and a stale
    /// entry left in the list is a capability the client believes in and
    /// the server has withdrawn.
    pub async fn capability(&mut self) -> Outcome<&[String]> {
        let resp = res!(self.read_command("CAPABILITY").await);
        self.take_capabilities(&resp);
        Ok(&self.caps)
    }

    /// As [`Self::capability`] with no retry, for use inside a conversation still being stood up.
    async fn capability_once(&mut self) -> Outcome<()> {
        let resp = res!(self.command("CAPABILITY").await);
        self.take_capabilities(&resp);
        Ok(())
    }

    fn take_capabilities(&mut self, resp: &Response) {
        let mut caps: Vec<String> = Vec::new();
        for line in &resp.untagged {
            caps.extend(parse_capabilities(&line.text));
        }
        caps.sort();
        caps.dedup();
        self.caps = caps;
    }

    /// Does the server advertise this capability? Compared case-insensitively.
    pub fn has_cap(&self, cap: &str) -> bool {
        let want = cap.to_uppercase();
        self.caps.iter().any(|c| *c == want)
    }

    /// For a consumer mailbox the password is an app password, not the account password.
    pub async fn login(&mut self, user: &str, pass: &str) -> Outcome<()> {
        let auth = Auth::Login {
            user: user.to_string(),
            pass: SecretString::new(pass.to_string()),
        };
        match self.login_once(user, pass).await {
            Ok(()) => {
                self.auth = Some(auth);
                Ok(())
            },
            Err(e) if self.may_retry(&e) => {
                // The reconnect logs in with what it is given, which is this.
                self.auth = Some(auth);
                res!(self.reopen(e).await);
                Ok(())
            },
            Err(e) => Err(e),
        }
    }

    async fn login_once(&mut self, user: &str, pass: &str) -> Outcome<()> {
        if self.has_cap("LOGINDISABLED") {
            return Err(err!(
                "IMAP server {} has disabled password login on this \
                connection.", self.host;
                Unauthorised, Security));
        }
        let cmd = fmt!("LOGIN {} {}", quoted(user), quoted(pass));
        // The password is in the command, so keep it out of any error.
        let resp = res!(self.command_hushed(&cmd, "LOGIN").await);
        self.absorb_capabilities_from(&resp);
        if self.caps.is_empty() {
            res!(self.capability_once().await);
        }
        Ok(())
    }

    /// SASL `XOAUTH2`, the mechanism the large providers require of a registered application.
    pub async fn authenticate_xoauth2(&mut self, user: &str, token: &str) -> Outcome<()> {
        let auth = Auth::XOAuth2 {
            user:  user.to_string(),
            token: SecretString::new(token.to_string()),
        };
        match self.xoauth2_once(user, token).await {
            Ok(()) => {
                self.auth = Some(auth);
                Ok(())
            },
            Err(e) if self.may_retry(&e) => {
                self.auth = Some(auth);
                res!(self.reopen(e).await);
                Ok(())
            },
            Err(e) => Err(e),
        }
    }

    async fn xoauth2_once(&mut self, user: &str, token: &str) -> Outcome<()> {
        if !self.has_cap("AUTH=XOAUTH2") {
            return Err(err!(
                "IMAP server {} does not offer XOAUTH2.", self.host;
                Unimplemented, Mismatch));
        }
        let raw = fmt!("user={}\u{1}auth=Bearer {}\u{1}\u{1}", user, token);
        let cmd = fmt!("AUTHENTICATE XOAUTH2 {}", base64::encode(raw.as_bytes()));
        let resp = res!(self.command_hushed(&cmd, "AUTHENTICATE XOAUTH2").await);
        self.absorb_capabilities_from(&resp);
        if self.caps.is_empty() {
            res!(self.capability_once().await);
        }
        Ok(())
    }

    /// List mailboxes under `reference` matching `pattern` (`"*"` for all,
    /// `"%"` for one level).
    pub async fn list(&mut self, reference: &str, pattern: &str) -> Outcome<Vec<MailboxInfo>> {
        // Ask for the RFC 6154 roles when the server offers them, so a
        // localised "[Gmail]/Gesendet" still carries `\Sent`. A server
        // without the capability gets the plain command it expects.
        let cmd = if self.has_cap("SPECIAL-USE") {
            fmt!("LIST {} {} RETURN (SPECIAL-USE)", quoted(reference), quoted(pattern))
        } else {
            fmt!("LIST {} {}", quoted(reference), quoted(pattern))
        };
        let resp = res!(self.read_command(&cmd).await);
        let mut out = Vec::new();
        for line in &resp.untagged {
            if let Some(mb) = res!(parse_list_line(line)) {
                out.push(mb);
            }
        }
        Ok(out)
    }

    pub async fn select(&mut self, mailbox: &str) -> Outcome<MailboxStatus> {
        self.select_impl(mailbox, false, &SelectWith::Plain).await
    }

    /// Read-only, so nothing the client does can change a flag. The safe choice for a sync that
    /// must not disturb the mailbox.
    pub async fn examine(&mut self, mailbox: &str) -> Outcome<MailboxStatus> {
        self.select_impl(mailbox, true, &SelectWith::Plain).await
    }

    // ── Extensions ───────────────────────────────────────────────

    /// RFC 5161 `ENABLE`, refused before anything is sent to a server without it. The server
    /// names what it turned on, which may be less than was asked and is returned; what it turned
    /// on is enabled again on any fresh connection.
    pub async fn enable(&mut self, exts: &[&str]) -> Outcome<Vec<String>> {
        if !self.has_cap("ENABLE") {
            return Err(err!(
                "IMAP server {} does not offer ENABLE, so it cannot enable {:?}.", self.host, exts;
                Unimplemented, Mismatch));
        }
        if exts.is_empty() {
            return Ok(Vec::new());
        }
        let exts: Vec<String> = exts.iter().map(|e| e.to_uppercase()).collect();
        match self.enable_once(&exts).await {
            Err(e) if self.may_retry(&e) => {
                res!(self.reopen(e).await);
                self.enable_once(&exts).await
            },
            other => other,
        }
    }

    async fn enable_once(&mut self, exts: &[String]) -> Outcome<Vec<String>> {
        let resp = res!(self.command(&fmt!("ENABLE {}", exts.join(" "))).await);
        let mut got: Vec<String> = Vec::new();
        for line in &resp.untagged {
            let up = line.text.to_uppercase();
            if let Some(rest) = up.strip_prefix("* ENABLED") {
                got.extend(rest.split_whitespace().map(|s| s.to_string()));
            }
        }
        for g in &got {
            if !self.enabled.contains(g) {
                self.enabled.push(g.clone());
            }
        }
        Ok(got)
    }

    /// Has the server said it enabled this extension? `QRESYNC` brings `CONDSTORE` with it
    /// (RFC 7162 §3.2.3).
    pub fn is_enabled(&self, ext: &str) -> bool {
        let want = ext.to_uppercase();
        self.enabled.iter().any(|e| *e == want || (want == "CONDSTORE" && e == "QRESYNC"))
    }

    // ── Mailboxes ────────────────────────────────────────────────

    /// As [`Self::select`], with RFC 7162 parameters, which a reconnect repeats. A QRESYNC open
    /// enables QRESYNC first where it is not yet on, and either is refused before anything is
    /// sent to a server that does not offer it.
    pub async fn select_with(&mut self, mailbox: &str, with: &SelectWith) -> Outcome<MailboxStatus> {
        self.select_impl(mailbox, false, with).await
    }

    /// As [`Self::examine`], with RFC 7162 parameters, as [`Self::select_with`].
    pub async fn examine_with(&mut self, mailbox: &str, with: &SelectWith) -> Outcome<MailboxStatus> {
        self.select_impl(mailbox, true, with).await
    }

    /// RFC 3501 `STATUS`, for a mailbox's counts without opening it; `HIGHESTMODSEQ` too where
    /// CONDSTORE is offered.
    pub async fn status(&mut self, mailbox: &str) -> Outcome<FolderStatus> {
        let modseq = self.has_cap("CONDSTORE") || self.has_cap("QRESYNC");
        let cmd = fmt!("STATUS {} (MESSAGES UIDNEXT UIDVALIDITY UNSEEN{})",
            quoted(mailbox), if modseq { " HIGHESTMODSEQ" } else { "" });
        let resp = res!(self.read_command(&cmd).await);
        for line in &resp.untagged {
            if let Some(st) = res!(parse_status_line(line)) {
                return Ok(st);
            }
        }
        Err(err!("IMAP server {} answered STATUS of '{}' with no STATUS line.", self.host, mailbox;
            IO, Network, Missing))
    }

    /// Creates `mailbox`, a hierarchy the server needs to hold it included (RFC 3501 6.3.3).
    pub async fn create(&mut self, mailbox: &str) -> Outcome<()> {
        res!(self.command(&fmt!("CREATE {}", quoted(mailbox))).await);
        Ok(())
    }

    pub async fn subscribe(&mut self, mailbox: &str) -> Outcome<()> {
        res!(self.read_command(&fmt!("SUBSCRIBE {}", quoted(mailbox))).await);
        Ok(())
    }

    pub async fn unsubscribe(&mut self, mailbox: &str) -> Outcome<()> {
        res!(self.read_command(&fmt!("UNSUBSCRIBE {}", quoted(mailbox))).await);
        Ok(())
    }

    /// The subscribed mailboxes under `reference` matching `pattern`, as [`Self::list`].
    pub async fn lsub(&mut self, reference: &str, pattern: &str) -> Outcome<Vec<MailboxInfo>> {
        let cmd  = fmt!("LSUB {} {}", quoted(reference), quoted(pattern));
        let resp = res!(self.read_command(&cmd).await);
        let mut out = Vec::new();
        for line in &resp.untagged {
            if let Some(mb) = res!(parse_list_line(line)) {
                out.push(mb);
            }
        }
        Ok(out)
    }

    /// RFC 2971 `ID`: the client's fields go, the server's come back, a `None` value where it
    /// sent `NIL`. No fields sends `ID NIL`. Refused before anything is sent to a server without
    /// it.
    pub async fn id(&mut self, fields: &[(&str, &str)]) -> Outcome<Vec<(String, Option<String>)>> {
        if !self.has_cap("ID") {
            return Err(err!("IMAP server {} does not offer ID.", self.host; Unimplemented, Mismatch));
        }
        let cmd = if fields.is_empty() {
            fmt!("ID NIL")
        } else {
            let parts: Vec<String> = fields.iter()
                .map(|(k, v)| fmt!("{} {}", quoted(k), quoted(v)))
                .collect();
            fmt!("ID ({})", parts.join(" "))
        };
        let resp = res!(self.read_command(&cmd).await);
        for line in &resp.untagged {
            if !line.text.to_uppercase().starts_with("* ID ") {
                continue;
            }
            let toks = res!(tokenise(&line.text, &line.literals));
            let items = match toks.get(2) {
                Some(Tok::List(items)) => items,
                _                      => return Ok(Vec::new()), // `* ID NIL`
            };
            let mut out = Vec::new();
            for pair in items.chunks(2) {
                let key = String::from_utf8_lossy(&pair[0].as_bytes()).into_owned();
                let val = match pair.get(1) {
                    Some(Tok::Atom(a)) if a.eq_ignore_ascii_case("NIL") => None,
                    Some(t) => Some(String::from_utf8_lossy(&t.as_bytes()).into_owned()),
                    None    => None,
                };
                out.push((key, val));
            }
            return Ok(out);
        }
        Ok(Vec::new())
    }

    async fn select_impl(
        &mut self,
        mailbox:    &str,
        read_only:  bool,
        with:       &SelectWith,
    )
        -> Outcome<MailboxStatus>
    {
        match with {
            SelectWith::Plain => (),
            SelectWith::CondStore => if !self.has_cap("CONDSTORE") {
                return Err(err!(
                    "IMAP server {} does not offer CONDSTORE, so '{}' cannot be opened with it.",
                    self.host, mailbox; Unimplemented, Mismatch));
            },
            // A QRESYNC parameter before `ENABLE QRESYNC` is a protocol error (RFC 7162 §3.2.5).
            SelectWith::QResync { .. } => if !self.is_enabled("QRESYNC") {
                if !self.has_cap("QRESYNC") || !self.has_cap("ENABLE") {
                    return Err(err!(
                        "IMAP server {} does not offer QRESYNC, so '{}' cannot be opened with it.",
                        self.host, mailbox; Unimplemented, Mismatch));
                }
                res!(self.enable(&["QRESYNC"]).await);
                if !self.is_enabled("QRESYNC") {
                    return Err(err!(
                        "IMAP server {} offers QRESYNC but did not enable it.", self.host;
                        Unimplemented, Mismatch));
                }
            },
        }
        // A new open starts a new numbering, so no earlier UIDVALIDITY binds it.
        self.opened = Some(Opened {
            name:           mailbox.to_string(),
            read_only,
            with:           with.clone(),
            uid_validity:   None,
            modseq:         false,
        });
        let st = match self.select_once(mailbox, read_only, with).await {
            Ok(st) => st,
            Err(e) if self.may_retry(&e) => {
                match res!(self.reopen(e).await) {
                    Some(st) => st,
                    None     => return Err(err!(
                        "The reconnect to {} did not re-open '{}'.", self.host, mailbox;
                        Bug, Missing)),
                }
            },
            Err(e) => {
                // A failed open leaves no mailbox open (RFC 3501 6.3.1).
                self.opened = None;
                return Err(e);
            },
        };
        if let Some(o) = self.opened.as_mut() {
            o.uid_validity = Some(st.uid_validity);
            o.modseq       = st.highest_modseq.is_some();
        }
        Ok(st)
    }

    async fn select_once(
        &mut self,
        mailbox:    &str,
        read_only:  bool,
        with:       &SelectWith,
    )
        -> Outcome<MailboxStatus>
    {
        let verb  = if read_only { "EXAMINE" } else { "SELECT" };
        let param = match with {
            SelectWith::Plain     => String::new(),
            SelectWith::CondStore => fmt!(" (CONDSTORE)"),
            SelectWith::QResync { uid_validity, modseq, known } => match known {
                Some(k) if !k.is_empty() => fmt!(" (QRESYNC ({} {} {}))", uid_validity, modseq, k),
                _                        => fmt!(" (QRESYNC ({} {}))", uid_validity, modseq),
            },
        };
        let cmd  = fmt!("{} {}{}", verb, quoted(mailbox), param);
        let resp = res!(self.command(&cmd).await);

        let mut st = MailboxStatus {
            name:      mailbox.to_string(),
            read_only,
            ..Default::default()
        };
        // QRESYNC's part of the reply: what vanished, and what changed. Only there does every
        // FETCH carry its UID, so only there is one read.
        let qresync = matches!(with, SelectWith::QResync { .. });
        for line in &resp.untagged {
            if let Some(v) = res!(parse_vanished(&line.text)) {
                st.vanished = st.vanished.union(&v);
            } else if let Some(msg) = if qresync { res!(parse_fetch_line(line)) } else { None } {
                st.changed.push(msg);
            } else {
                res!(absorb_select_line(&mut st, &line.text));
            }
        }
        // A server may downgrade a SELECT to read-only, and says so in the
        // completion line's response code.
        if resp.text.to_uppercase().contains("[READ-ONLY]") {
            st.read_only = true;
        }
        Ok(st)
    }

    /// `criteria` is the raw IMAP search key, e.g. `ALL`, `UID 1234:*`, `UNSEEN`, or
    /// `SINCE 01-Jan-2026`.
    pub async fn uid_search(&mut self, criteria: &str) -> Outcome<Vec<u32>> {
        let cmd  = fmt!("UID SEARCH {}", criteria);
        let resp = res!(self.read_command(&cmd).await);
        let mut uids: Vec<u32> = Vec::new();
        for line in &resp.untagged {
            let up = line.text.to_uppercase();
            if !up.starts_with("* SEARCH") { continue; }
            for tok in line.text.split_whitespace().skip(2) {
                if let Ok(n) = tok.parse::<u32>() {
                    uids.push(n);
                }
            }
        }
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
    }

    /// An empty set is a no-op rather than a command, because `UID FETCH ` with no set is a
    /// syntax error and a caller passing an empty search result is not doing anything wrong.
    pub async fn uid_fetch(
        &mut self,
        uids:   &UidSet,
        what:   FetchWhat,
    )
        -> Outcome<Vec<FetchedMessage>>
    {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        // `BODY.PEEK`, so a repeat changes nothing and is safe after a drop.
        let cmd  = fmt!("UID FETCH {} ({})", uids, self.fetch_items(what, false));
        let resp = res!(self.read_command(&cmd).await);
        let mut out: Vec<FetchedMessage> = Vec::new();
        for line in &resp.untagged {
            if let Some(msg) = res!(parse_fetch_line(line)) {
                out.push(msg);
            }
        }
        Ok(out)
    }

    /// What changed since mod-sequence `since` among `uids` (RFC 7162 `CHANGEDSINCE`), and, with
    /// QRESYNC enabled, which of them have vanished. Refused before anything is sent to a server
    /// without CONDSTORE.
    pub async fn uid_fetch_changed(
        &mut self,
        uids:   &UidSet,
        what:   FetchWhat,
        since:  u64,
    )
        -> Outcome<FetchChanged>
    {
        if !self.has_cap("CONDSTORE") && !self.has_cap("QRESYNC") {
            return Err(err!(
                "IMAP server {} does not offer CONDSTORE, so it cannot fetch what changed.",
                self.host; Unimplemented, Mismatch));
        }
        if uids.is_empty() {
            return Ok(FetchChanged::default());
        }
        let vanished = if self.is_enabled("QRESYNC") { " VANISHED" } else { "" };
        let cmd = fmt!("UID FETCH {} ({}) (CHANGEDSINCE {}{})",
            uids, self.fetch_items(what, true), since, vanished);
        let resp = res!(self.read_command(&cmd).await);
        let mut out = FetchChanged::default();
        for line in &resp.untagged {
            if let Some(v) = res!(parse_vanished(&line.text)) {
                out.vanished = out.vanished.union(&v);
            } else if let Some(msg) = res!(parse_fetch_line(line)) {
                out.messages.push(msg);
            }
        }
        Ok(out)
    }

    /// The fetch items for `what`: `MODSEQ` where asked or where the open mailbox keeps
    /// mod-sequences, and Gmail's where the server offers them, but nothing beyond what
    /// `MessageId` names.
    fn fetch_items(&self, what: FetchWhat, modseq: bool) -> String {
        let mut items = what.items().to_string();
        if what != FetchWhat::MessageId {
            if modseq || self.opened.as_ref().map(|o| o.modseq).unwrap_or(false) {
                items.push_str(" MODSEQ");
            }
            if self.has_cap("X-GM-EXT-1") {
                items.push_str(" X-GM-MSGID X-GM-LABELS");
            }
        }
        items
    }

    /// The messages whose `Message-ID` is `mid`, with or without its angle brackets. The server's
    /// `SEARCH` is only a candidate list, because a server may match a substring, another field
    /// or nothing in particular: each candidate's `Message-ID` field is fetched, a few bytes, and
    /// only those it names exactly are kept, with their UIDs and sizes. An empty result is no
    /// match, never an error.
    pub async fn uid_search_message_id(&mut self, mid: &str) -> Outcome<Vec<FetchedMessage>> {
        let bare = bare_message_id(mid);
        if bare.is_empty() || bare.chars().any(|c| c.is_control() || c == '"' || c == '\\') {
            return Err(err!(
                "The Message-ID {:?} cannot be searched for: empty, or with a control character, \
                quote or backslash.", mid; Invalid, Input));
        }
        let uids = res!(self.uid_search(&fmt!("HEADER Message-ID {}", quoted(bare))).await);
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let found = res!(self.uid_fetch(&UidSet::from(&uids), FetchWhat::MessageId).await);
        Ok(found.into_iter().filter(|m| header_message_id(&m.body).as_deref() == Some(bare)).collect())
    }

    /// `None` where the server does not return it: expunged between the search and the fetch,
    /// which is a race the caller cannot prevent and should not treat as an error.
    pub async fn uid_fetch_one(&mut self, uid: u32) -> Outcome<Option<FetchedMessage>> {
        let mut msgs = res!(self.uid_fetch(&UidSet::from([uid]), FetchWhat::Full).await);
        Ok(if msgs.is_empty() { None } else { Some(msgs.remove(0)) })
    }

    /// Adds `add` and removes `remove`, leaving every other flag as it is: a `+FLAGS` and a
    /// `-FLAGS`, each sent only when it names a flag. There is deliberately no absolute set,
    /// which would overwrite a flag another client has changed since this one looked. A UID no
    /// longer in the mailbox is passed over by the server, which is not an error.
    pub async fn uid_store_flags(
        &mut self,
        uids:   &UidSet,
        add:    &[&str],
        remove: &[&str],
    )
        -> Outcome<()>
    {
        if uids.is_empty() {
            return Ok(());
        }
        // `.SILENT` throughout: the client already knows what it asked for, and the untagged
        // FETCH the server would otherwise send back is a round trip spent on nothing.
        for (item, flags) in [("+FLAGS.SILENT", add), ("-FLAGS.SILENT", remove)] {
            if flags.is_empty() {
                continue;
            }
            let cmd = fmt!("UID STORE {} {} ({})", uids, item, flags.join(" "));
            res!(self.command(&cmd).await);
        }
        Ok(())
    }

    /// `None` from a server without `UIDPLUS`, which says nothing of where the copies went.
    pub async fn uid_copy(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>> {
        if uids.is_empty() {
            return Ok(None);
        }
        let resp = res!(self.command(&fmt!("UID COPY {} {}", uids, quoted(to))).await);
        copy_uid_of(&resp, uids)
    }

    /// RFC 6851 `MOVE`, refused before anything is sent to a server that does not offer it: the
    /// fallback (`UID COPY`, `+FLAGS \Deleted`, `UID EXPUNGE`) is the caller's to choose.
    pub async fn uid_move(&mut self, uids: &UidSet, to: &str) -> Outcome<Option<CopyUid>> {
        if !self.has_cap("MOVE") {
            return Err(err!(
                "IMAP server {} does not offer MOVE.", self.host;
                Unimplemented, Mismatch));
        }
        if uids.is_empty() {
            return Ok(None);
        }
        let resp = res!(self.command(&fmt!("UID MOVE {} {}", uids, quoted(to))).await);
        copy_uid_of(&resp, uids)
    }

    /// Removes exactly these UIDs where they are `\Deleted`. Refused before anything is sent to a
    /// server without `UIDPLUS`, because the only other expunge removes every `\Deleted` message
    /// in the mailbox, including ones another client marked and meant to keep a while.
    pub async fn uid_expunge(&mut self, uids: &UidSet) -> Outcome<()> {
        if !self.has_cap("UIDPLUS") {
            return Err(err!(
                "IMAP server {} does not offer UIDPLUS, so it cannot expunge by UID.", self.host;
                Unimplemented, Mismatch));
        }
        if uids.is_empty() {
            return Ok(());
        }
        res!(self.command(&fmt!("UID EXPUNGE {}", uids)).await);
        Ok(())
    }

    /// `body` is a raw RFC 5322 message: this is how a sent message is filed in `Sent` after
    /// SMTP has delivered it. The new UID where the server has `UIDPLUS`, and `None` otherwise.
    pub async fn append(
        &mut self,
        mailbox:    &str,
        flags:      &[&str],
        body:       &[u8],
    )
        -> Outcome<Option<AppendUid>>
    {
        let tag  = self.next_tag();
        let flag_part = if flags.is_empty() {
            String::new()
        } else {
            fmt!(" ({})", flags.join(" "))
        };
        let cmd = fmt!("{} APPEND {}{} {{{}}}\r\n",
            tag, quoted(mailbox), flag_part, body.len());
        res!(self.write_all(cmd.as_bytes()).await);

        // The server must answer a synchronising literal with a `+`
        // continuation before the bytes may be sent.
        let cont = res!(self.read_line().await);
        if !cont.text.starts_with('+') {
            return Err(err!(
                "APPEND to '{}' was refused: {}", mailbox, cont.text;
                IO, Network, Wire));
        }
        res!(self.write_all(body).await);
        res!(self.write_all(b"\r\n").await);

        let resp = res!(self.read_until_tag(&tag, "APPEND").await);
        let code = match response_code(&resp.text, "APPENDUID") {
            Some(c) => c,
            None    => return Ok(None),
        };
        let mut it = code.split_whitespace();
        match (it.next().map(|v| v.parse::<u32>()), it.next().map(|u| u.parse::<u32>())) {
            (Some(Ok(uid_validity)), Some(Ok(uid))) => Ok(Some(AppendUid { uid_validity, uid })),
            _ => Err(err!(
                "IMAP server {} sent a malformed APPENDUID: {}", self.host, resp.text;
                IO, Network, Decode)),
        }
    }

    pub async fn logout(&mut self) -> Outcome<()> {
        res!(self.command("LOGOUT").await);
        if let Some(s) = self.stream.as_mut() {
            let _ = s.get_mut().shutdown().await;
        }
        Ok(())
    }

    // ── Retry ────────────────────────────────────────────────────

    /// May this failure be met with a fresh connection?
    fn may_retry(&self, e: &Error<ErrTag>) -> bool {
        self.retries > 0 && is_interrupted(e)
    }

    /// A command that changes nothing on the server, so that after a drop it is asked again on a
    /// fresh connection.
    async fn read_command(&mut self, cmd: &str) -> Outcome<Response> {
        match self.command(cmd).await {
            Err(e) if self.may_retry(&e) => {
                res!(self.reopen(e).await);
                self.command(cmd).await
            },
            other => other,
        }
    }

    /// Spend a retry: stand the conversation up again on a fresh connection -- connected, logged
    /// in and with the same mailbox open -- and carry on with it. The status of the re-opened
    /// mailbox, where one was open, is returned.
    async fn reopen(&mut self, cause: Error<ErrTag>) -> Outcome<Option<MailboxStatus>> {
        self.retries = self.retries.saturating_sub(1);
        warn!("IMAP {}: the connection dropped, trying once more on a fresh one: {}",
            self.host, cause);

        // The server took this conversation a moment ago, so a fresh connection that cannot be
        // made says nothing new about it: the drop is the fault, and the caller hears that rather
        // than an "unreachable" that would send them checking a name and port that worked.
        let mut fresh = match Self::open(&self.cfg, self.tls_cfg.clone()).await {
            Ok(c)  => c,
            Err(e) => {
                warn!("IMAP {}: the fresh connection failed too: {}", self.host, e);
                return Err(cause);
            },
        };
        match &self.auth {
            Some(Auth::Login { user, pass }) =>
                res!(fresh.login_once(user, pass.expose_secret()).await),
            Some(Auth::XOAuth2 { user, token }) =>
                res!(fresh.xoauth2_once(user, token.expose_secret()).await),
            None => (),
        }
        // What was enabled is enabled again before the mailbox is re-opened, which a QRESYNC
        // open needs.
        if !self.enabled.is_empty() {
            let exts = self.enabled.clone();
            res!(fresh.enable_once(&exts).await);
        }
        let mut status = None;
        if let Some(o) = &self.opened {
            let st = res!(fresh.select_once(&o.name, o.read_only, &o.with).await);
            if let Some(v) = o.uid_validity {
                if st.uid_validity != v {
                    return Err(err!(
                        "IMAP {}: '{}' re-opened after a dropped connection under UIDVALIDITY {} \
                        where it was {}, so its messages were renumbered and the UIDs in hand \
                        name others. Read it again from scratch.",
                        self.host, o.name, st.uid_validity, v;
                        IO, Network, Mismatch));
                }
            }
            status = Some(st);
        }
        self.stream = fresh.stream.take();
        self.tag    = fresh.tag;
        self.caps   = std::mem::take(&mut fresh.caps);
        self.enabled = std::mem::take(&mut fresh.enabled);
        Ok(status)
    }

    // ── Command plumbing ─────────────────────────────────────────

    fn next_tag(&mut self) -> String {
        self.tag += 1;
        fmt!("a{:04}", self.tag)
    }

    /// A `NO` or a `BAD` is an error carrying the server's own words, which are usually the most
    /// useful thing anyone will say about the failure.
    async fn command(&mut self, cmd: &str) -> Outcome<Response> {
        let verb = cmd.split_whitespace().next().unwrap_or(cmd).to_string();
        self.command_hushed(cmd, &verb).await
    }

    /// As [`Self::command`], but naming the command in errors rather than
    /// echoing it -- for commands whose text contains a credential.
    async fn command_hushed(&mut self, cmd: &str, label: &str) -> Outcome<Response> {
        let tag  = self.next_tag();
        let line = fmt!("{} {}\r\n", tag, cmd);
        res!(self.write_all(line.as_bytes()).await);
        self.read_until_tag(&tag, label).await
    }

    /// Read untagged lines until the one carrying `tag`, then judge it.
    async fn read_until_tag(&mut self, tag: &str, label: &str) -> Outcome<Response> {
        let mut untagged: Vec<RawLine> = Vec::new();
        loop {
            let line = res!(self.read_line().await);
            if line.text.starts_with("* ") || line.text == "*" {
                untagged.push(line);
                continue;
            }
            if line.text.starts_with('+') {
                return Err(err!(
                    "IMAP server asked for a literal in reply to {}, which \
                    sends none.", label;
                    IO, Network, Wire));
            }
            let rest = match line.text.strip_prefix(tag) {
                Some(r) => r.trim_start(),
                None    => {
                    // A tag we did not send: the connection is out of step
                    // and nothing read after this can be trusted.
                    return Err(err!(
                        "IMAP response carried tag other than '{}': {}",
                        tag, line.text;
                        IO, Network, Wire));
                }
            };
            let (status, text) = res!(parse_completion(rest));
            return match status {
                Status::Ok => Ok(Response { untagged, text }),
                Status::No => Err(refusal(label, &text)),
                Status::Bad => Err(err!(
                    "IMAP server rejected {} as malformed: {}", label, text;
                    IO, Network, Wire)),
            };
        }
    }

    /// Merge any `[CAPABILITY ...]` response code carried on a command's
    /// completion line into the cached set.
    fn absorb_capabilities_from(&mut self, resp: &Response) {
        let text = resp.text.clone();
        self.absorb_capabilities(&text);
    }

    /// Pull a `CAPABILITY` list out of a line, whether it arrived as an
    /// untagged `* CAPABILITY ...` or as a `[CAPABILITY ...]` response
    /// code inside a greeting or completion.
    fn absorb_capabilities(&mut self, text: &str) {
        self.caps.extend(parse_capabilities(text));
        self.caps.sort();
        self.caps.dedup();
    }

    // ── Wire ─────────────────────────────────────────────────────

    async fn write_all(&mut self, bytes: &[u8]) -> Outcome<()> {
        let host     = self.host.clone();
        let deadline = self.timeout;
        let w = res!(self.stream_mut()).get_mut();
        match timeout(deadline, w.write_all(bytes)).await {
            Ok(Ok(()))  => (),
            Ok(Err(e))  => return Err(tls::wire_fault(e,
                fmt!("Writing to IMAP server {}", host), true)),
            Err(_)      => return Err(err!(
                "Timeout writing to IMAP server {}.", host;
                IO, Network, Timeout)),
        }
        match timeout(deadline, w.flush()).await {
            Ok(Ok(()))  => Ok(()),
            Ok(Err(e))  => Err(tls::wire_fault(e,
                fmt!("Flushing to IMAP server {}", host), true)),
            Err(_)      => Err(err!(
                "Timeout flushing to IMAP server {}.", host;
                IO, Network, Timeout)),
        }
    }

    /// Read one *logical* response line: CRLF-terminated text, except that
    /// a trailing `{n}` is a literal whose `n` raw bytes follow, after
    /// which the line continues. The literals are lifted out; what returns
    /// is the text with its `{n}` markers still in place, and the payloads
    /// alongside in order.
    async fn read_line(&mut self) -> Outcome<RawLine> {
        let mut text     = String::new();
        let mut literals = Vec::new();
        loop {
            let chunk = res!(self.read_crlf_line().await);
            text.push_str(&chunk);
            let n = match trailing_literal_len(&chunk) {
                Some(n) => n,
                None    => break,
            };
            if n > MAX_LITERAL_BYTES {
                return Err(err!(
                    "IMAP server {} announced a {}-byte literal, over the \
                    {}-byte limit.", self.host, n, MAX_LITERAL_BYTES;
                    IO, Network, Excessive));
            }
            let mut buf  = vec![0u8; n];
            let host     = self.host.clone();
            let deadline = self.timeout;
            let rd       = res!(self.stream_mut());
            match timeout(deadline, rd.read_exact(&mut buf)).await {
                Ok(Ok(_))  => (),
                Ok(Err(e)) => return Err(tls::wire_fault(e,
                    fmt!("Reading a {}-byte literal from IMAP server {}", n, host), false)),
                Err(_)     => return Err(err!(
                    "Timeout reading a {}-byte literal from IMAP server {}.",
                    n, host;
                    IO, Network, Timeout)),
            }
            literals.push(buf);
        }
        Ok(RawLine { text, literals })
    }

    /// Without its terminator.
    async fn read_crlf_line(&mut self) -> Outcome<String> {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let host     = self.host.clone();
        let deadline = self.timeout;
        let rd       = res!(self.stream_mut());
        let n = match timeout(deadline, rd.read_until(b'\n', &mut buf)).await {
            Ok(Ok(n))  => n,
            Ok(Err(e)) => return Err(tls::wire_fault(e,
                fmt!("Reading from IMAP server {}", host), false)),
            Err(_)     => return Err(err!(
                "Timeout reading from IMAP server {}.", host;
                IO, Network, Timeout)),
        };
        if n == 0 {
            return Err(err!(
                "IMAP server {} closed the connection.", host;
                IO, Network, Read, Interrupted));
        }
        while buf.last() == Some(&b'\n') || buf.last() == Some(&b'\r') {
            buf.pop();
        }
        // Response text is 7-bit ASCII plus, in practice, whatever a
        // server puts in a mailbox name. Lossy is right: a malformed byte
        // in a mailbox name must not fail the sync.
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PARSING                                                                   │
// └───────────────────────────────────────────────────────────────────────────┘

/// One element of an IMAP response.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Tok {
    Atom(String),       // a bare word: a number, a flag, `NIL`, a `BODY[...]` item name
    Quoted(String),     // unescaped
    Literal(Vec<u8>),   // the payload, spliced back in from the reader's side queue
    List(Vec<Tok>),     // parenthesised
}

impl Tok {
    /// An atom or a quoted string only.
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Atom(s) | Self::Quoted(s) => Some(s),
            _                               => None,
        }
    }

    /// Whether it arrived as a literal or a string. `NIL` comes back empty, which is what a
    /// server means by it here.
    fn as_bytes(&self) -> Vec<u8> {
        match self {
            Self::Literal(b)  => b.clone(),
            Self::Quoted(s)   => s.as_bytes().to_vec(),
            Self::Atom(s)     => if s.eq_ignore_ascii_case("NIL") {
                                     Vec::new()
                                 } else {
                                     s.as_bytes().to_vec()
                                 },
            Self::List(_)     => Vec::new(),
        }
    }
}

/// Tokenise a response line, splicing each `{n}` marker back into the
/// literal that followed it on the wire.
fn tokenise(text: &str, literals: &[Vec<u8>]) -> Outcome<Vec<Tok>> {
    let chars: Vec<char> = text.chars().collect();
    let mut queue: VecDeque<Vec<u8>> = literals.iter().cloned().collect();
    let mut pos = 0usize;
    let toks = res!(tokenise_until(&chars, &mut pos, &mut queue, None));
    Ok(toks)
}

/// Tokenise until `close` (or the end of the input when `close` is
/// `None`). Recursive, because IMAP lists nest.
fn tokenise_until(
    chars:  &[char],
    pos:    &mut usize,
    queue:  &mut VecDeque<Vec<u8>>,
    close:  Option<char>,
)
    -> Outcome<Vec<Tok>>
{
    let mut out: Vec<Tok> = Vec::new();
    while *pos < chars.len() {
        let c = chars[*pos];
        if c.is_whitespace() {
            *pos += 1;
            continue;
        }
        if Some(c) == close {
            *pos += 1;
            return Ok(out);
        }
        match c {
            '(' => {
                *pos += 1;
                let inner = res!(tokenise_until(chars, pos, queue, Some(')')));
                out.push(Tok::List(inner));
            }
            ')' => {
                // An unbalanced close: the caller wanted end-of-input.
                return Err(err!(
                    "Unbalanced ')' in IMAP response at character {}.", pos;
                    Invalid, Input, Decode));
            }
            '"' => {
                *pos += 1;
                let mut s = String::new();
                let mut closed = false;
                while *pos < chars.len() {
                    let d = chars[*pos];
                    *pos += 1;
                    if d == '\\' && *pos < chars.len() {
                        s.push(chars[*pos]);
                        *pos += 1;
                        continue;
                    }
                    if d == '"' { closed = true; break; }
                    s.push(d);
                }
                if !closed {
                    return Err(err!(
                        "Unterminated quoted string in IMAP response.";
                        Invalid, Input, Decode));
                }
                out.push(Tok::Quoted(s));
            }
            '{' => {
                // A literal marker. Its payload was read off the wire and
                // is waiting in the queue, in order.
                while *pos < chars.len() && chars[*pos] != '}' {
                    *pos += 1;
                }
                if *pos >= chars.len() {
                    return Err(err!(
                        "Unterminated literal marker in IMAP response.";
                        Invalid, Input, Decode));
                }
                *pos += 1;                              // past the '}'
                match queue.pop_front() {
                    Some(bytes) => out.push(Tok::Literal(bytes)),
                    None => return Err(err!(
                        "IMAP response has more literal markers than \
                        literals were read.";
                        Invalid, Input, Decode)),
                }
            }
            _ => {
                // An atom, which may embed a bracketed section --
                // `BODY[HEADER.FIELDS (FROM TO)]` is one token, spaces and
                // parentheses and all.
                let mut s = String::new();
                let mut depth = 0usize;
                while *pos < chars.len() {
                    let d = chars[*pos];
                    if depth == 0 {
                        if d.is_whitespace() || d == '(' || d == ')' { break; }
                        if Some(d) == close { break; }
                    }
                    if d == '[' { depth += 1; }
                    if d == ']' { depth = depth.saturating_sub(1); }
                    s.push(d);
                    *pos += 1;
                }
                out.push(Tok::Atom(s));
            }
        }
    }
    if close.is_some() {
        return Err(err!(
            "IMAP response ended inside a parenthesised list.";
            Invalid, Input, Decode));
    }
    Ok(out)
}

/// If a line ends with a synchronising or non-synchronising literal
/// marker (`{123}` or `{123+}`), the length it announces.
fn trailing_literal_len(line: &str) -> Option<usize> {
    let trimmed = line.trim_end();
    if !trimmed.ends_with('}') { return None; }
    let open = ok!(trimmed.rfind('{'));
    let inner = &trimmed[open + 1..trimmed.len() - 1];
    let digits = inner.strip_suffix('+').unwrap_or(inner);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<usize>().ok()
}

/// A completion line is `OK ...`, `NO ...` or `BAD ...`.
fn parse_completion(rest: &str) -> Outcome<(Status, String)> {
    let mut it = rest.splitn(2, char::is_whitespace);
    let word = it.next().unwrap_or("");
    let text = it.next().unwrap_or("").trim().to_string();
    let status = match word.to_uppercase().as_str() {
        "OK"  => Status::Ok,
        "NO"  => Status::No,
        "BAD" => Status::Bad,
        other => return Err(err!(
            "IMAP completion line has unknown status '{}'.", other;
            Invalid, Input, Decode)),
    };
    Ok((status, text))
}

/// A `NO`, tagged by what it refused. The RFC 5530 response code says why where the server gives
/// one; otherwise the command does: a `NO` to `LOGIN` is a refused credential, and a `NO` to
/// `SELECT` or `EXAMINE` a mailbox that cannot be opened.
fn refusal(label: &str, text: &str) -> Error<ErrTag> {
    let up = text.to_uppercase();
    if up.contains("[UNAVAILABLE]") {
        err!("IMAP server refused {} for now: {}", label, text; IO, Network, Interrupted)
    } else if up.contains("[AUTHENTICATIONFAILED]")
        || up.contains("[AUTHORIZATIONFAILED]")
        || label == "LOGIN"
        || label.starts_with("AUTHENTICATE")
    {
        err!("IMAP server refused {}: {}", label, text; IO, Network, Unauthorised)
    } else if up.contains("[NONEXISTENT]") || label == "SELECT" || label == "EXAMINE" {
        err!("IMAP server refused {}: {}", label, text; IO, Network, NotFound)
    } else {
        err!("IMAP server refused {}: {}", label, text; IO, Network, Invalid)
    }
}

/// Pull capability names out of `* CAPABILITY ...` or a `[CAPABILITY ...]`
/// response code, upper-cased.
fn parse_capabilities(text: &str) -> Vec<String> {
    let up = text.to_uppercase();
    let body = if let Some(i) = up.find("[CAPABILITY ") {
        let start = i + "[CAPABILITY ".len();
        match up[start..].find(']') {
            Some(e) => &up[start..start + e],
            None    => return Vec::new(),
        }
    } else if let Some(i) = up.find("* CAPABILITY ") {
        &up[i + "* CAPABILITY ".len()..]
    } else {
        return Vec::new();
    };
    body.split_whitespace().map(|s| s.to_string()).collect()
}

/// Reads `* LIST (\HasNoChildren) "/" "INBOX"`; `None` for an untagged line that is not a
/// `LIST` reply.
fn parse_list_line(line: &RawLine) -> Outcome<Option<MailboxInfo>> {
    let up = line.text.to_uppercase();
    if !up.starts_with("* LIST") && !up.starts_with("* LSUB") {
        return Ok(None);
    }
    let toks = res!(tokenise(&line.text, &line.literals));
    // `*`, `LIST`, (attrs), delimiter, name
    if toks.len() < 5 {
        return Ok(None);
    }
    let attrs = match &toks[2] {
        Tok::List(items) => items.iter()
            .filter_map(|t| t.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    };
    let delimiter = toks[3].as_str()
        .filter(|s| !s.eq_ignore_ascii_case("NIL"))
        .and_then(|s| s.chars().next());
    let name = String::from_utf8_lossy(&toks[4].as_bytes()).into_owned();
    if name.is_empty() {
        return Ok(None);
    }
    Ok(Some(MailboxInfo { name, delimiter, attrs }))
}

/// Fold one untagged line of a `SELECT`/`EXAMINE` reply into the status.
/// Unrecognised lines are ignored -- a server is free to volunteer more
/// than the client asked for.
fn absorb_select_line(st: &mut MailboxStatus, text: &str) -> Outcome<()> {
    let up = text.to_uppercase();
    let parts: Vec<&str> = up.split_whitespace().collect();

    // `* 42 EXISTS` / `* 3 RECENT`
    if parts.len() >= 3 && parts[0] == "*" {
        if let Ok(n) = parts[1].parse::<u32>() {
            match parts[2] {
                "EXISTS" => { st.exists = n; return Ok(()); }
                "RECENT" => { st.recent = n; return Ok(()); }
                _        => (),
            }
        }
    }
    // `* OK [UIDVALIDITY 1234]` / `* OK [UIDNEXT 5678]`
    if let Some(v) = bracket_value(&up, "UIDVALIDITY") {
        st.uid_validity = v;
    }
    if let Some(v) = bracket_value(&up, "UIDNEXT") {
        st.uid_next = v;
    }
    // `* OK [HIGHESTMODSEQ 715194045007]`; `[NOMODSEQ]` leaves it `None`.
    if let Some(v) = bracket_u64(&up, "HIGHESTMODSEQ") {
        st.highest_modseq = Some(v);
    }
    // `* FLAGS (\Answered \Flagged ...)`
    if up.starts_with("* FLAGS") {
        if let (Some(a), Some(b)) = (text.find('('), text.rfind(')')) {
            if b > a {
                st.flags = text[a + 1..b]
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
            }
        }
    }
    if up.contains("[READ-ONLY]") {
        st.read_only = true;
    }
    Ok(())
}

/// The number inside a `[NAME 123]` response code, if present.
fn bracket_value(up: &str, name: &str) -> Option<u32> {
    let pat = fmt!("[{} ", name);
    let i = ok!(up.find(&pat));
    let start = i + pat.len();
    let end = ok!(up[start..].find(']'));
    up[start..start + end].trim().parse::<u32>().ok()
}

/// As [`bracket_value`], for a 63-bit mod-sequence.
fn bracket_u64(up: &str, name: &str) -> Option<u64> {
    let pat = fmt!("[{} ", name);
    let i = ok!(up.find(&pat));
    let start = i + pat.len();
    let end = ok!(up[start..].find(']'));
    up[start..start + end].trim().parse::<u64>().ok()
}

/// The UIDs of a `* VANISHED (EARLIER) 41,43:116` line (RFC 7162 §3.2.10), with or without
/// `(EARLIER)`; `None` for any other line.
fn parse_vanished(text: &str) -> Outcome<Option<UidSet>> {
    let rest = match text.get(..11) {
        Some(head) if head.eq_ignore_ascii_case("* VANISHED ") => text[11..].trim(),
        _ => return Ok(None),
    };
    let rest = match rest.get(..9) {
        Some(head) if head.eq_ignore_ascii_case("(EARLIER)") => rest[9..].trim(),
        _ => rest,
    };
    Ok(Some(res!(UidSet::parse(rest))))
}

/// Reads `* STATUS "INBOX" (MESSAGES 231 UIDNEXT 44292)`; `None` for any other line.
fn parse_status_line(line: &RawLine) -> Outcome<Option<FolderStatus>> {
    if !line.text.to_uppercase().starts_with("* STATUS ") {
        return Ok(None);
    }
    let toks = res!(tokenise(&line.text, &line.literals));
    let items = match toks.get(3) {
        Some(Tok::List(items)) => items,
        _ => return Err(err!("IMAP STATUS reply has no data list: {}", line.text;
            Invalid, Input, Decode)),
    };
    let mut st = FolderStatus::default();
    for pair in items.chunks(2) {
        let (key, val) = match (pair[0].as_str(), pair.get(1).and_then(|t| t.as_str())) {
            (Some(k), Some(v)) => (k.to_uppercase(), v),
            _                  => continue,
        };
        let n = res!(val.parse::<u64>().map_err(|e| err!(e,
            "IMAP STATUS item {} is not a number: {}", key, line.text; Invalid, Input, Decode)));
        if key == "HIGHESTMODSEQ" {
            st.highest_modseq = Some(n);
            continue;
        }
        let small = res!(u32::try_from(n).map_err(|e| err!(e,
            "IMAP STATUS item {} is out of range: {}", key, line.text; Invalid, Input, Decode)));
        match key.as_str() {
            "MESSAGES"      => st.messages = small,
            "UIDNEXT"       => st.uid_next = small,
            "UIDVALIDITY"   => st.uid_validity = small,
            "UNSEEN"        => st.unseen = small,
            _               => (),
        }
    }
    Ok(Some(st))
}

/// A Message-ID without its angle brackets or surrounding space.
fn bare_message_id(mid: &str) -> &str {
    let t = mid.trim();
    t.strip_prefix('<').and_then(|t| t.strip_suffix('>')).unwrap_or(t).trim()
}

/// The `Message-ID` a header block names, without its angle brackets; `None` where it names
/// none, or cannot be read as RFC 5322 reads it, which is no confirmation.
fn header_message_id(block: &[u8]) -> Option<String> {
    let (head, _) = email::header::split_headers_body(block);
    let fields = ok!(email::header::header_fields(head).ok());
    fields.into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Message-ID"))
        .map(|(_, v)| bare_message_id(&v).to_string())
}

/// Reads `* 12 FETCH (UID 345 FLAGS (\Seen) ... BODY[] {4523}...)`; `None` for an untagged line
/// that is not a `FETCH` reply.
fn parse_fetch_line(line: &RawLine) -> Outcome<Option<FetchedMessage>> {
    let toks = res!(tokenise(&line.text, &line.literals));
    if toks.len() < 3 {
        return Ok(None);
    }
    if toks[0].as_str() != Some("*") {
        return Ok(None);
    }
    match toks[1].as_str().map(|s| s.parse::<u32>()) {
        Some(Ok(_)) => (),
        _           => return Ok(None),
    }
    if !toks[2].as_str().map(|s| s.eq_ignore_ascii_case("FETCH")).unwrap_or(false) {
        return Ok(None);
    }
    let seq = match toks[1].as_str().and_then(|s| s.parse::<u32>().ok()) {
        Some(n) => n,
        None    => return Ok(None),
    };
    let items = match toks.get(3) {
        Some(Tok::List(items)) => items,
        _ => return Err(err!(
            "IMAP FETCH reply has no data list: {}", line.text;
            Invalid, Input, Decode)),
    };

    let mut msg = FetchedMessage { seq, ..Default::default() };
    let mut i = 0usize;
    while i < items.len() {
        let key = match items[i].as_str() {
            Some(s) => s.to_uppercase(),
            None    => { i += 1; continue; }
        };
        let val = match items.get(i + 1) {
            Some(v) => v,
            None    => break,
        };
        i += 2;
        match key.as_str() {
            "UID" => {
                if let Some(n) = val.as_str().and_then(|s| s.parse::<u32>().ok()) {
                    msg.uid = n;
                }
            }
            "RFC822.SIZE" => {
                if let Some(n) = val.as_str().and_then(|s| s.parse::<u32>().ok()) {
                    msg.size = n;
                }
            }
            "FLAGS" => {
                if let Tok::List(fs) = val {
                    msg.flags = fs.iter()
                        .filter_map(|t| t.as_str().map(|s| s.to_string()))
                        .collect();
                }
            }
            "INTERNALDATE" => {
                if let Some(s) = val.as_str() {
                    msg.internal_date = s.to_string();
                }
            }
            // `MODSEQ (20060115194045000)`, a list of one.
            "MODSEQ" => {
                if let Tok::List(v) = val {
                    msg.modseq = v.first().and_then(|t| t.as_str()).and_then(|s| s.parse::<u64>().ok());
                }
            }
            "X-GM-MSGID" => {
                msg.gm_msgid = val.as_str().and_then(|s| s.parse::<u64>().ok());
            }
            // Atoms, strings or literals, a label being any mailbox name.
            "X-GM-LABELS" => {
                if let Tok::List(ls) = val {
                    msg.gm_labels = ls.iter()
                        .map(|t| String::from_utf8_lossy(&t.as_bytes()).into_owned())
                        .collect();
                }
            }
            _ => {
                // Every body-ish item -- `BODY[]`, `BODY[HEADER]`,
                // `RFC822`, `RFC822.HEADER` -- carries the bytes we want,
                // and the first one that does wins. Anything else is an
                // item the caller did not ask for, and is skipped.
                if key.starts_with("BODY[") || key == "RFC822" || key == "RFC822.HEADER" {
                    if msg.body.is_empty() {
                        msg.body = val.as_bytes();
                    }
                }
            }
        }
    }
    if msg.uid == 0 {
        // Without a UID the message cannot be addressed again, so it is
        // useless to a synchronising caller. A server that omits it after
        // being asked for it is broken.
        return Err(err!(
            "IMAP FETCH reply for sequence {} carried no UID.", seq;
            Invalid, Input, Missing));
    }
    Ok(Some(msg))
}

/// The text of the `[NAME ...]` response code in `text`, after the name; `Some("")` for a bare
/// `[NAME]`.
fn response_code(text: &str, name: &str) -> Option<String> {
    let up  = text.to_uppercase();
    let pat = fmt!("[{}", name);
    let mut from = 0usize;
    while let Some(i) = up[from..].find(&pat) {
        let start = from + i + pat.len();
        match up[start..].chars().next() {
            Some(' ') | Some(']') => {
                let end = ok!(text[start..].find(']'));
                return Some(text[start..start + end].trim().to_string());
            },
            _ => from = start,
        }
    }
    None
}

/// The `COPYUID` of a `COPY` or `MOVE` of `sent`, on the completion line or, for a `MOVE`, an
/// untagged `OK`. Its source UIDs are a subset of `sent` (RFC 4315), so a list longer than `sent`
/// is not believed and is never expanded.
fn copy_uid_of(resp: &Response, sent: &UidSet) -> Outcome<Option<CopyUid>> {
    let max = usize::try_from(sent.len()).unwrap_or(usize::MAX);
    let code = resp.untagged.iter().map(|l| l.text.as_str())
        .chain(std::iter::once(resp.text.as_str()))
        .find_map(|t| response_code(t, "COPYUID"));
    let code = match code {
        Some(c) => c,
        None    => return Ok(None),
    };
    let parts: Vec<&str> = code.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(err!("A malformed COPYUID: [COPYUID {}]", code; Invalid, Input, Decode));
    }
    let uid_validity = res!(parts[0].parse::<u32>().map_err(|e| err!(e,
        "A malformed COPYUID: [COPYUID {}]", code; Invalid, Input, Decode)));
    let src = res!(uid_set::uid_list(parts[1], max));
    let dst = res!(uid_set::uid_list(parts[2], max));
    if src.len() != dst.len() {
        return Err(err!(
            "A COPYUID naming {} source and {} destination UIDs: [COPYUID {}]",
            src.len(), dst.len(), code; Invalid, Input, Decode));
    }
    Ok(Some(CopyUid { uid_validity, pairs: src.into_iter().zip(dst).collect() }))
}

/// An IMAP quoted-string, with the two characters that need escaping escaped.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use tokio::net::TcpListener;

    fn line(text: &str, lits: Vec<Vec<u8>>) -> RawLine {
        RawLine { text: text.to_string(), literals: lits }
    }


    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ A SCRIPTED SERVER                                                     │
    // └───────────────────────────────────────────────────────────────────────┘

    // The round-trip against fe2o3's own ImapServer lives in
    // fe2o3_mail/tests/imap_roundtrip.rs and proves the happy path. What it
    // cannot pose is a server behaving badly -- a BYE greeting, a literal larger
    // than the client will allocate for, a tag the client never sent, bytes
    // pipelined behind a STARTTLS response. Those are the cases where this client
    // either refuses or is quietly compromised, so they are scripted here.

    const USER: &str = "alice@example.com";
    const PASS: &str = "app-password-not-a-real-one";

    /// Every line the server was sent, in order, plus a `<literal N>` marker and
    /// its payload wherever the client sent one.
    type Transcript = Arc<Mutex<Vec<String>>>;

    /// Serve `greeting`, then answer each command the client sends with the next
    /// entry of `script`, verbatim. `%T%` in an entry becomes the tag the client
    /// used on the command being answered, so a script need not track them.
    ///
    /// A command ending in a `{n}` literal marker is answered as scripted; where
    /// that answer begins with `+`, the `n` bytes and their CRLF are read off and
    /// logged before the next command. Running off the end of the script closes
    /// the connection, which is a legitimate thing for a server to do and one the
    /// client must not hang on.
    async fn scripted(
        greeting:   &'static str,
        script:     Vec<&'static str>,
    )
        -> Outcome<(SocketAddr, Transcript)>
    {
        scripted_seq(vec![(greeting, script)]).await
    }

    /// As [`scripted`], but one session per entry, each served on the next connection accepted, so
    /// a client that reconnects meets the next script. Each accepted connection logs a
    /// `<connection N>` marker, counting from one.
    async fn scripted_seq(
        sessions:   Vec<(&'static str, Vec<&'static str>)>,
    )
        -> Outcome<(SocketAddr, Transcript)>
    {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await
            .map_err(|e| err!(e, "Binding the scripted IMAP server."; IO, Network)));
        let addr = res!(listener.local_addr()
            .map_err(|e| err!(e, "Reading its address."; IO, Network)));

        let seen: Transcript = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();

        tokio::spawn(async move {
          for (n, (greeting, script)) in sessions.into_iter().enumerate() {
            let (sock, _) = match listener.accept().await {
                Ok(x)  => x,
                Err(_) => return,
            };
            if let Ok(mut g) = log.lock() {
                g.push(fmt!("<connection {}>", n + 1));
            }
            let (r, mut w) = sock.into_split();
            let mut rd = BufReader::new(r);

            let _ = w.write_all(greeting.as_bytes()).await;

            let mut script = script.into_iter();
            while let Some(reply) = script.next() {
                let mut buf: Vec<u8> = Vec::with_capacity(256);
                match rd.read_until(b'\n', &mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_)          => (),
                }
                while buf.last() == Some(&b'\n') || buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                let cmd = String::from_utf8_lossy(&buf).into_owned();
                if let Ok(mut g) = log.lock() {
                    g.push(cmd.clone());
                }
                let tag = cmd.split_whitespace().next().unwrap_or("").to_string();
                let out = reply.replace("%T%", &tag);
                let _ = w.write_all(out.as_bytes()).await;

                // A synchronising literal the client is now entitled to send. It
                // is not a command, so the entry that follows goes out without
                // waiting for another line -- the client is waiting on the
                // completion and will send nothing until it has it.
                if out.starts_with('+') {
                    if let Some(n) = trailing_literal_len(&cmd) {
                        let mut lit = vec![0u8; n];
                        if rd.read_exact(&mut lit).await.is_err() {
                            break;
                        }
                        let mut tail = Vec::new();
                        let _ = rd.read_until(b'\n', &mut tail).await;
                        if let Ok(mut g) = log.lock() {
                            g.push(fmt!("<literal {}>", n));
                            g.push(String::from_utf8_lossy(&lit).into_owned());
                        }
                        match script.next() {
                            Some(done) => {
                                let _ = w.write_all(
                                    done.replace("%T%", &tag).as_bytes()).await;
                            },
                            None => break,
                        }
                    }
                }
            }
          }
        });

        Ok((addr, seen))
    }

    fn cfg(addr: SocketAddr) -> ImapConfig {
        ImapConfig::new(fmt!("127.0.0.1"), addr.port(), Security::Plain)
            .with_addr(addr)
            .with_timeout(Duration::from_secs(10))
    }

    /// What the client sent, without the connection markers.
    fn lines_of(t: &Transcript) -> Outcome<Vec<String>> {
        match t.lock() {
            Ok(g)  => Ok(g.iter().filter(|l| !l.starts_with("<connection ")).cloned().collect()),
            Err(_) => Err(err!("The server's transcript was poisoned."; Lock, Poisoned)),
        }
    }

    /// How many connections the server accepted.
    fn connections_of(t: &Transcript) -> Outcome<usize> {
        match t.lock() {
            Ok(g)  => Ok(g.iter().filter(|l| l.starts_with("<connection ")).count()),
            Err(_) => Err(err!("The server's transcript was poisoned."; Lock, Poisoned)),
        }
    }

    /// A greeting that already carries the capability list, which is the common
    /// case and saves a round trip.
    const GREET: &str = "* OK [CAPABILITY IMAP4rev1 SPECIAL-USE AUTH=PLAIN] ready\r\n";

    // ── The greeting ──────────────────────────────────────────────

    /// A `BYE` greeting is a refusal, and the server's own words are the most
    /// useful thing anybody will say about it.
    #[tokio::test]
    async fn test_a_bye_greeting_is_a_refusal_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(
            "* BYE Too many connections from your IP\r\n", vec![]).await);
        let out = ImapClient::connect(&cfg(addr)).await;
        let msg = match out {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "A BYE greeting was treated as a usable connection."; Test, Invalid)),
        };
        req!(true, msg.contains("Too many connections"),
            "the server's own words were dropped: {}", msg);
        Ok(())
    }

    /// Anything that is not `OK` or `PREAUTH` is not an IMAP server, and talking
    /// on regardless is how a client sends a password to whatever answered.
    #[tokio::test]
    async fn test_a_greeting_that_is_not_a_greeting_is_refused_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted("+OK POP3 server ready\r\n", vec![]).await);
        req!(true, ImapClient::connect(&cfg(addr)).await.is_err(),
            "a POP3 greeting was accepted as IMAP");
        req!(true, res!(lines_of(&seen)).is_empty(),
            "the client sent a command to something that never greeted it");
        Ok(())
    }

    /// The greeting's `[CAPABILITY ...]` is taken at its word: a client that asks
    /// again anyway has spent a round trip on nothing, on every connection.
    #[tokio::test]
    async fn test_a_capability_in_the_greeting_saves_the_round_trip_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec![]).await);
        let c = res!(ImapClient::connect(&cfg(addr)).await);
        req!(true, c.has_cap("IMAP4REV1"));
        req!(true, c.has_cap("SPECIAL-USE"));
        req!(true, res!(lines_of(&seen)).is_empty(),
            "the client asked for CAPABILITY it had already been given");
        Ok(())
    }

    /// A greeting without one leaves the client not knowing what the server can
    /// do, so it must ask before doing anything that depends on the answer.
    #[tokio::test]
    async fn test_a_bare_greeting_is_followed_by_capability_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(
            "* OK ready\r\n",
            vec!["* CAPABILITY IMAP4rev1 STARTTLS\r\n%T% OK done\r\n"],
        ).await);
        let c = res!(ImapClient::connect(&cfg(addr)).await);
        req!(true, c.has_cap("STARTTLS"));
        let lines = res!(lines_of(&seen));
        req!(1, lines.len());
        req!(true, lines[0].to_uppercase().ends_with("CAPABILITY"),
            "the client did not ask for CAPABILITY: {:?}", lines);
        Ok(())
    }

    /// A refresh replaces the set rather than adding to it. A capability left
    /// behind is one the client believes in and the server has withdrawn --
    /// after a login, that is a command sent to a server that will reject it.
    #[tokio::test]
    async fn test_a_capability_refresh_replaces_rather_than_accumulates_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(
            "* OK [CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED] ready\r\n",
            vec!["* CAPABILITY IMAP4rev1 IDLE\r\n%T% OK done\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        req!(true, c.has_cap("LOGINDISABLED"));
        res!(c.capability().await);
        req!(false, c.has_cap("LOGINDISABLED"),
            "a withdrawn capability survived the refresh");
        req!(false, c.has_cap("STARTTLS"), "a withdrawn capability survived the refresh");
        req!(true, c.has_cap("IDLE"));
        Ok(())
    }

    // ── Authentication ────────────────────────────────────────────

    /// A refusal carries the server's words -- `[AUTHENTICATIONFAILED]` is what
    /// tells a person their app password is the problem -- and never the password.
    #[tokio::test]
    async fn test_a_refused_login_keeps_the_words_and_not_the_password_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(
            GREET,
            vec!["%T% NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let msg = match c.login(USER, PASS).await {
            Err(e) => fmt!("{}", e),
            Ok(())  => return Err(err!("A refused LOGIN was reported as a success.";
                Test, Invalid)),
        };
        req!(true, msg.contains("AUTHENTICATIONFAILED"),
            "the server's response code was dropped: {}", msg);
        req!(false, msg.contains(PASS), "the password leaked into the error: {}", msg);
        // It did go over the wire, which is what LOGIN is; the point is that the
        // error does not repeat it into a log file.
        req!(true, res!(lines_of(&seen)).iter().any(|l| l.contains(PASS)));
        Ok(())
    }

    /// `LOGINDISABLED` means the password will be refused, so it is not sent. A
    /// client that tries anyway has put the credential on the wire to learn
    /// something the server already told it.
    #[tokio::test]
    async fn test_logindisabled_withholds_the_password_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(
            "* OK [CAPABILITY IMAP4rev1 LOGINDISABLED STARTTLS] ready\r\n",
            vec!["%T% OK never reached\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        req!(true, c.login(USER, PASS).await.is_err(),
            "the client sent a password to a server that had disabled it");
        req!(false, res!(lines_of(&seen)).iter().any(|l| l.contains(PASS)),
            "the password crossed the wire anyway");
        Ok(())
    }

    /// XOAUTH2 is refused outright where the server does not offer it, rather
    /// than sent and rejected -- a bearer token spent on a server that cannot
    /// use it is a token in somebody's log.
    #[tokio::test]
    async fn test_xoauth2_is_not_offered_to_a_server_without_it_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec!["%T% OK never reached\r\n"]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        req!(true, c.authenticate_xoauth2(USER, "ya29.a-bearer-token").await.is_err());
        req!(false, res!(lines_of(&seen)).iter().any(|l| l.contains("ya29")),
            "the token crossed the wire to a server that does not speak XOAUTH2");
        Ok(())
    }

    /// STARTTLS was asked for and the server does not offer it. The credential is
    /// not sent in the clear as a fallback.
    #[tokio::test]
    async fn test_starttls_absent_refuses_the_connection_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(
            "* OK [CAPABILITY IMAP4rev1 AUTH=PLAIN] ready\r\n", vec![]).await);
        let c = ImapConfig::new(fmt!("127.0.0.1"), addr.port(), Security::StartTls)
            .with_addr(addr)
            .with_timeout(Duration::from_secs(10));
        let msg = match ImapClient::connect(&c).await {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "A connection that could not be secured was returned as usable.";
                Test, Invalid)),
        };
        req!(true, msg.contains("STARTTLS"), "the error did not name STARTTLS: {}", msg);
        req!(false, res!(lines_of(&seen)).iter().any(|l| l.to_uppercase().contains("LOGIN")),
            "the client began to log in over an unprotected connection");
        Ok(())
    }

    /// RFC 2595 §3.1: anything the server pipelines behind its `STARTTLS` reply
    /// arrived unprotected and is indistinguishable from what the TLS peer says
    /// next. Discarding it silently is the downgrade attack; the connection must
    /// be refused.
    #[tokio::test]
    async fn test_data_pipelined_behind_starttls_is_refused_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(
            "* OK [CAPABILITY IMAP4rev1 STARTTLS] ready\r\n",
            // The OK, and then an injected untagged line the real peer never sent.
            vec!["%T% OK Begin TLS negotiation now\r\n* OK [CAPABILITY IMAP4rev1 \
                AUTH=PLAIN LOGINDISABLED] injected\r\n"],
        ).await);
        let c = ImapConfig::new(fmt!("127.0.0.1"), addr.port(), Security::StartTls)
            .with_addr(addr)
            .with_timeout(Duration::from_secs(10));
        let msg = match ImapClient::connect(&c).await {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "Bytes injected before the handshake were accepted as the peer's.";
                Test, Invalid)),
        };
        req!(true, msg.contains("after its STARTTLS response"),
            "the refusal did not name the reason: {}", msg);
        Ok(())
    }

    // ── The wire, where a line is not a line ──────────────────────

    /// The reader's whole job: a literal announced mid-line, whose payload
    /// contains CRLF, a close paren and a `{n}` of its own. A client that reads
    /// FETCH replies line by line loses the message here, and the bug does not
    /// show until somebody sends an attachment.
    #[tokio::test]
    async fn test_a_literal_containing_crlf_survives_the_reader_00() -> Outcome<()> {
        // 62 bytes, counted: a body with two blank lines, a close paren and a
        // fake literal marker in it.
        let body = "Subject: awkward\r\n\r\n) not the end {17}\r\nlast line\r\n";
        req!(51, body.len(), "the scripted literal length must match the payload");
        let script = vec![
            "%T% OK [READ-WRITE] SELECT completed\r\n",
            "* 1 FETCH (UID 9 FLAGS (\\Seen) RFC822.SIZE 51 BODY[] {51}\r\n\
             Subject: awkward\r\n\r\n) not the end {17}\r\nlast line\r\n)\r\n\
             %T% OK FETCH completed\r\n",
        ];
        let (addr, _) = res!(scripted(GREET, script).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let msgs = res!(c.uid_fetch(&UidSet::from([9]), FetchWhat::Full).await);
        req!(1, msgs.len(), "the FETCH reply was lost");
        req!(body.as_bytes().to_vec(), msgs[0].body,
            "the literal came back changed: {:?}", String::from_utf8_lossy(&msgs[0].body));
        req!(9, msgs[0].uid);
        req!(vec![fmt!("\\Seen")], msgs[0].flags);
        Ok(())
    }

    /// A server announcing more than the client will allocate for is refused
    /// before the allocation, not after it.
    #[tokio::test]
    async fn test_an_oversized_literal_is_refused_before_allocating_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(
            GREET,
            vec![
                "%T% OK SELECT completed\r\n",
                // 64 MiB + 1, announced and never sent.
                "* 1 FETCH (UID 9 BODY[] {67108865}\r\n",
            ],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let msg = match c.uid_fetch(&UidSet::from([9]), FetchWhat::Full).await {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "A 64 MiB literal was accepted from a server that never sent it.";
                Test, Invalid)),
        };
        req!(true, msg.contains("67108865"), "the error did not name the size: {}", msg);
        Ok(())
    }

    /// A tag the client never sent means the connection is out of step, and
    /// nothing read after it belongs to the command that is waiting. Matching it
    /// loosely is how one command's reply is read as another's.
    #[tokio::test]
    async fn test_a_foreign_tag_takes_the_connection_out_of_step_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(
            GREET,
            vec!["a9999 OK SELECT completed\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let msg = match c.select("INBOX").await {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "A reply carrying somebody else's tag was accepted."; Test, Invalid)),
        };
        req!(true, msg.contains("a9999"), "the error did not name the tag seen: {}", msg);
        Ok(())
    }

    /// A server that hangs up mid-command is an error naming the close, not a
    /// hang and not an empty success.
    #[tokio::test]
    async fn test_a_closed_connection_is_named_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(GREET, vec![]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        // The one retry finds nothing listening, and the caller still hears the hang-up, not an
        // "unreachable" server that took the first connection.
        let e = match c.select("INBOX").await {
            Err(e) => e,
            Ok(_)  => return Err(err!(
                "A closed connection was reported as a SELECT."; Test, Invalid)),
        };
        let msg = fmt!("{}", e);
        req!(true, msg.contains("closed the connection"),
            "the error did not say the server hung up: {}", msg);
        req!(true, is_interrupted(&e), "a hang-up was not tagged Interrupted: {:?}", e.tags());
        req!(false, e.tags().contains(&ErrTag::Unreachable),
            "a hang-up was called unreachable: {:?}", e.tags());
        Ok(())
    }

    /// `NO` and `BAD` are different failures and are reported as such: one is a
    /// server refusing, the other a client having sent nonsense.
    #[tokio::test]
    async fn test_no_and_bad_are_reported_apart_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(GREET,
            vec!["%T% NO Mailbox does not exist\r\n"]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let msg = match c.select("nosuch").await {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!("A NO was a success."; Test, Invalid)),
        };
        req!(true, msg.contains("refused"), "a NO did not read as a refusal: {}", msg);

        let (addr, _) = res!(scripted(GREET, vec!["%T% BAD Command unrecognised\r\n"]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let msg = match c.select("INBOX").await {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!("A BAD was a success."; Test, Invalid)),
        };
        req!(true, msg.contains("malformed"), "a BAD did not read as malformed: {}", msg);
        Ok(())
    }

    // ── What the client puts on the wire ──────────────────────────

    /// A week's worth of consecutive UIDs is a command line a server is entitled
    /// to reject, so they go over collapsed into ranges. Asserted from what the
    /// server received, not from the function that builds it.
    #[tokio::test]
    async fn test_a_uid_set_reaches_the_server_collapsed_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(
            GREET,
            vec![
                "%T% OK SELECT completed\r\n",
                "%T% OK FETCH completed\r\n",
            ],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        res!(c.uid_fetch(&UidSet::from([1, 2, 3, 7, 9, 10]), FetchWhat::Meta).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.contains("UID FETCH 1:3,7,9:10")),
            "the UID set was not collapsed on the wire: {:?}", lines);
        Ok(())
    }

    /// An empty set is no command at all: `UID FETCH ` with nothing after it is a
    /// syntax error, and a caller passing an empty search result has done nothing
    /// wrong.
    #[tokio::test]
    async fn test_an_empty_fetch_sends_nothing_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec!["%T% OK SELECT completed\r\n"]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let before = res!(lines_of(&seen)).len();
        req!(true, res!(c.uid_fetch(&UidSet::new(), FetchWhat::Full).await).is_empty());
        res!(c.uid_store_flags(&UidSet::new(), &["\\Seen"], &[]).await);
        req!(before, res!(lines_of(&seen)).len(),
            "a command was sent for an empty UID set");
        Ok(())
    }

    /// `RETURN (SPECIAL-USE)` is asked for only where the server said it
    /// understands it. Sent to a server without the capability it is a `LIST` the
    /// server may reject outright, and then no folders are listed at all.
    #[tokio::test]
    async fn test_special_use_is_asked_for_only_when_offered_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET,
            vec!["* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n%T% OK LIST completed\r\n"]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.list("", "*").await);
        req!(true, res!(lines_of(&seen)).iter().any(|l| l.contains("RETURN (SPECIAL-USE)")),
            "SPECIAL-USE was offered and not asked for");

        let (addr, seen) = res!(scripted(
            "* OK [CAPABILITY IMAP4rev1] ready\r\n",
            vec!["* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n%T% OK LIST completed\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.list("", "*").await);
        req!(false, res!(lines_of(&seen)).iter().any(|l| l.contains("SPECIAL-USE")),
            "SPECIAL-USE was asked of a server that never offered it");
        Ok(())
    }

    /// `EXAMINE` rather than `SELECT`, because a sync that marks mail read is a
    /// sync somebody notices. The gateway's fetch path uses this one.
    #[tokio::test]
    async fn test_examine_is_read_only_on_the_wire_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(
            GREET,
            vec!["* 42 EXISTS\r\n* OK [UIDVALIDITY 1234567890]\r\n\
                  * OK [UIDNEXT 4321]\r\n%T% OK [READ-ONLY] EXAMINE completed\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let st = res!(c.examine("INBOX").await);
        req!(true, res!(lines_of(&seen)).iter().any(|l| l.contains("EXAMINE \"INBOX\"")),
            "EXAMINE was not the verb sent");
        req!(42,           st.exists);
        req!(1_234_567_890, st.uid_validity);
        req!(4_321,        st.uid_next);
        req!(true,         st.read_only);
        Ok(())
    }

    /// A server may downgrade a `SELECT` and say so only in the completion line's
    /// response code. A client that misses it believes it may set flags.
    #[tokio::test]
    async fn test_a_downgraded_select_is_read_only_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(
            GREET,
            vec!["* 3 EXISTS\r\n%T% OK [READ-ONLY] SELECT completed\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let st = res!(c.select("INBOX").await);
        req!(true, st.read_only, "the downgrade in the completion line was missed");
        Ok(())
    }

    /// `.SILENT` on every `STORE`: the client already knows what it asked for,
    /// and the untagged FETCH the server would send back is a round trip spent on
    /// nothing.
    #[tokio::test]
    async fn test_a_store_is_silent_on_the_wire_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(
            GREET,
            vec!["%T% OK SELECT completed\r\n", "%T% OK STORE completed\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        res!(c.uid_store_flags(&UidSet::from([4, 5]), &["\\Seen"], &[]).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.contains("UID STORE 4:5 +FLAGS.SILENT (\\Seen)")),
            "the STORE was not what was expected: {:?}", lines);
        Ok(())
    }

    // ── APPEND, the one command with a literal going the other way ──

    /// The body waits for the `+` continuation. Sending it before the server has
    /// asked is how a client and a server disagree about where a message ends.
    #[tokio::test]
    async fn test_append_waits_for_the_continuation_00() -> Outcome<()> {
        let body = b"Subject: filed\r\n\r\nA sent message.\r\n";
        let (addr, seen) = res!(scripted(
            GREET,
            vec!["+ Ready for literal data\r\n", "%T% OK [APPENDUID 1 12] APPEND completed\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.append("Sent", &["\\Seen"], body).await);

        let lines = res!(lines_of(&seen));
        // The command, then the marker the fixture logs when it reads the
        // literal, then the payload -- in that order, which is the point.
        let cmd = res!(lines.iter().position(|l| l.contains("APPEND \"Sent\""))
            .ok_or_else(|| err!("No APPEND was sent: {:?}", lines; Test, Missing)));
        let lit = res!(lines.iter().position(|l| l.starts_with("<literal "))
            .ok_or_else(|| err!("The body never arrived: {:?}", lines; Test, Missing)));
        req!(true, cmd < lit, "the body was sent before the command");
        req!(true, lines[cmd].contains(&fmt!("{{{}}}", body.len())),
            "the announced length was wrong: {}", lines[cmd]);
        req!(true, lines[cmd].contains("(\\Seen)"), "the flags were dropped");
        req!(fmt!("<literal {}>", body.len()), lines[lit]);
        req!(String::from_utf8_lossy(body).into_owned(), lines[lit + 1]);
        Ok(())
    }

    /// A refusal instead of a continuation stops the body: a server that will not
    /// take the message must not be sent it anyway.
    #[tokio::test]
    async fn test_a_refused_append_sends_no_body_00() -> Outcome<()> {
        let body = b"Subject: filed\r\n\r\nA sent message.\r\n";
        let (addr, seen) = res!(scripted(
            GREET,
            vec!["%T% NO [OVERQUOTA] Mailbox is full\r\n"],
        ).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let msg = match c.append("Sent", &[], body).await {
            Err(e) => fmt!("{}", e),
            Ok(_)   => return Err(err!("A refused APPEND was a success."; Test, Invalid)),
        };
        req!(true, msg.contains("OVERQUOTA"), "the server's words were dropped: {}", msg);
        req!(false, res!(lines_of(&seen)).iter().any(|l| l.starts_with("<literal ")),
            "the body was sent to a mailbox that had refused it");
        Ok(())
    }

    #[test]
    fn test_quoted_escapes() {
        assert_eq!(quoted("plain"),      "\"plain\"");
        assert_eq!(quoted("a\"b"),       "\"a\\\"b\"");
        assert_eq!(quoted("a\\b"),       "\"a\\\\b\"");
    }

    #[test]
    fn test_trailing_literal_len() {
        assert_eq!(trailing_literal_len("* 1 FETCH (BODY[] {42}"),  Some(42));
        assert_eq!(trailing_literal_len("* 1 FETCH (BODY[] {42+}"), Some(42));
        assert_eq!(trailing_literal_len("a001 OK done"),            None);
        assert_eq!(trailing_literal_len("* OK [UIDNEXT 5]"),        None);
    }

    #[test]
    fn test_tokenise_nested_and_bracketed() {
        let toks = tokenise(
            "* 1 FETCH (UID 9 FLAGS (\\Seen \\Answered) BODY[HEADER.FIELDS (FROM TO)] \"x\")",
            &[],
        ).unwrap();
        assert_eq!(toks[0], Tok::Atom(fmt!("*")));
        assert_eq!(toks[1], Tok::Atom(fmt!("1")));
        assert_eq!(toks[2], Tok::Atom(fmt!("FETCH")));
        let items = match &toks[3] {
            Tok::List(v) => v.clone(),
            other        => panic!("expected a list, got {:?}", other),
        };
        assert_eq!(items[0], Tok::Atom(fmt!("UID")));
        assert_eq!(items[1], Tok::Atom(fmt!("9")));
        assert_eq!(items[3], Tok::List(vec![
            Tok::Atom(fmt!("\\Seen")),
            Tok::Atom(fmt!("\\Answered")),
        ]));
        // The bracketed section stays one atom, spaces and all.
        assert_eq!(items[4], Tok::Atom(fmt!("BODY[HEADER.FIELDS (FROM TO)]")));
        assert_eq!(items[5], Tok::Quoted(fmt!("x")));
    }

    #[test]
    fn test_parse_fetch_with_literal_body() {
        let body = b"From: a@b.co\r\nSubject: hi\r\n\r\nbody\r\n".to_vec();
        let l = line(
            &fmt!("* 12 FETCH (UID 345 FLAGS (\\Seen) INTERNALDATE \
                \"01-Jan-2026 09:15:00 +0000\" RFC822.SIZE {} BODY[] {{{}}})",
                body.len(), body.len()),
            vec![body.clone()],
        );
        let msg = parse_fetch_line(&l).unwrap().unwrap();
        assert_eq!(msg.seq,           12);
        assert_eq!(msg.uid,           345);
        assert_eq!(msg.flags,         vec![fmt!("\\Seen")]);
        assert_eq!(msg.internal_date, "01-Jan-2026 09:15:00 +0000");
        assert_eq!(msg.size as usize, body.len());
        assert_eq!(msg.body,          body);
    }

    /// A body containing a CRLF and even a `)` must survive: this is
    /// precisely what a line-oriented parser gets wrong.
    #[test]
    fn test_literal_body_containing_crlf_and_paren() {
        let body = b"Subject: x\r\n\r\nline one\r\n) not the end\r\n".to_vec();
        let l = line(
            &fmt!("* 1 FETCH (UID 2 BODY[] {{{}}})", body.len()),
            vec![body.clone()],
        );
        let msg = parse_fetch_line(&l).unwrap().unwrap();
        assert_eq!(msg.body, body);
        assert_eq!(msg.uid,  2);
    }

    #[test]
    fn test_parse_fetch_headers_only() {
        let hdr = b"From: a@b.co\r\n\r\n".to_vec();
        let l = line(
            &fmt!("* 3 FETCH (UID 7 RFC822.SIZE 999 BODY[HEADER] {{{}}})", hdr.len()),
            vec![hdr.clone()],
        );
        let msg = parse_fetch_line(&l).unwrap().unwrap();
        assert_eq!(msg.body, hdr);
        assert_eq!(msg.size, 999);            // the whole message, not the fetch
    }

    #[test]
    fn test_parse_fetch_without_uid_is_an_error() {
        let l = line("* 1 FETCH (FLAGS (\\Seen))", vec![]);
        assert!(parse_fetch_line(&l).is_err());
    }

    #[test]
    fn test_non_fetch_untagged_line_is_skipped() {
        let l = line("* 42 EXISTS", vec![]);
        assert!(parse_fetch_line(&l).unwrap().is_none());
    }

    #[test]
    fn test_parse_list_line() {
        let l = line("* LIST (\\HasNoChildren) \"/\" \"INBOX\"", vec![]);
        let mb = parse_list_line(&l).unwrap().unwrap();
        assert_eq!(mb.name,      "INBOX");
        assert_eq!(mb.delimiter, Some('/'));
        assert_eq!(mb.attrs,     vec![fmt!("\\HasNoChildren")]);
        assert!(mb.selectable());

        let l = line("* LIST (\\Noselect \\HasChildren) \".\" \"[Gmail]\"", vec![]);
        let mb = parse_list_line(&l).unwrap().unwrap();
        assert_eq!(mb.name,      "[Gmail]");
        assert_eq!(mb.delimiter, Some('.'));
        assert!(!mb.selectable());
    }

    #[test]
    fn test_special_use_survives_localised_names() {
        // A German Gmail account: the NAME is localised, the ROLE is not.
        let l = line("* LIST (\\HasNoChildren \\Sent) \"/\" \"[Gmail]/Gesendet\"", vec![]);
        let mb = parse_list_line(&l).unwrap().unwrap();
        assert_eq!(mb.name,           "[Gmail]/Gesendet");
        assert_eq!(mb.special_use(),  Some(SpecialUse::Sent));

        let l = line("* LIST (\\HasNoChildren \\All) \"/\" \"[Gmail]/Alle Nachrichten\"", vec![]);
        let mb = parse_list_line(&l).unwrap().unwrap();
        assert_eq!(mb.special_use(),  Some(SpecialUse::All));

        // No role declared: an ordinary user folder.
        let l = line("* LIST (\\HasNoChildren) \"/\" \"receipts\"", vec![]);
        let mb = parse_list_line(&l).unwrap().unwrap();
        assert_eq!(mb.special_use(),  None);
    }

    #[test]
    fn test_select_lines_fold_into_status() {
        let mut st = MailboxStatus::default();
        absorb_select_line(&mut st, "* 42 EXISTS").unwrap();
        absorb_select_line(&mut st, "* 3 RECENT").unwrap();
        absorb_select_line(&mut st, "* OK [UIDVALIDITY 1234567890]").unwrap();
        absorb_select_line(&mut st, "* OK [UIDNEXT 4321]").unwrap();
        absorb_select_line(&mut st, "* FLAGS (\\Answered \\Seen)").unwrap();
        assert_eq!(st.exists,       42);
        assert_eq!(st.recent,       3);
        assert_eq!(st.uid_validity, 1_234_567_890);
        assert_eq!(st.uid_next,     4_321);
        assert_eq!(st.flags,        vec![fmt!("\\Answered"), fmt!("\\Seen")]);
        assert!(!st.read_only);
    }

    #[test]
    fn test_capabilities_from_greeting_and_untagged() {
        let a = parse_capabilities("* OK [CAPABILITY IMAP4rev1 STARTTLS AUTH=PLAIN] ready");
        assert!(a.contains(&fmt!("STARTTLS")));
        assert!(a.contains(&fmt!("IMAP4REV1")));
        let b = parse_capabilities("* CAPABILITY IMAP4rev1 IDLE AUTH=XOAUTH2");
        assert!(b.contains(&fmt!("AUTH=XOAUTH2")));
        assert!(parse_capabilities("* 12 EXISTS").is_empty());
    }

    #[test]
    fn test_parse_completion() {
        assert_eq!(parse_completion("OK LOGIN completed").unwrap().0,  Status::Ok);
        assert_eq!(parse_completion("NO [AUTHENTICATIONFAILED] bad").unwrap().0, Status::No);
        assert_eq!(parse_completion("BAD nonsense").unwrap().0,        Status::Bad);
        assert!(parse_completion("WAT something").is_err());
    }

    #[test]
    fn test_fetch_items_peek_not_seen() {
        // Fetching must not silently mark mail as read.
        assert!(FetchWhat::Full.items().contains("BODY.PEEK[]"));
        assert!(!FetchWhat::Full.items().contains("BODY[]"));
    }

    // ── Faults are tagged, and a dropped read is tried once more ──

    const LOGIN_OK:   &str = "%T% OK LOGIN completed\r\n";
    const EXAMINE_7:  &str = "* 2 EXISTS\r\n* OK [UIDVALIDITY 7] UIDs valid\r\n\
                              * OK [UIDNEXT 5] next\r\n%T% OK [READ-ONLY] EXAMINE completed\r\n";
    const EXAMINE_8:  &str = "* 2 EXISTS\r\n* OK [UIDVALIDITY 8] UIDs valid\r\n\
                              * OK [UIDNEXT 5] next\r\n%T% OK [READ-ONLY] EXAMINE completed\r\n";
    const SEARCH_3_4: &str = "* SEARCH 3 4\r\n%T% OK SEARCH completed\r\n";

    /// A server that says nothing and hangs up, which is what a dropped connection looks like from
    /// the client's side.
    const DROP: (&str, Vec<&str>) = ("", Vec::new());

    /// The connection drops under a `UID SEARCH`. The client opens a fresh one, logs in again,
    /// re-opens the mailbox and asks again, and the caller sees only the answer.
    #[tokio::test]
    async fn test_a_dropped_read_is_retried_once_on_a_fresh_connection_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted_seq(vec![
            (GREET, vec![LOGIN_OK, EXAMINE_7]),
            (GREET, vec![LOGIN_OK, EXAMINE_7, SEARCH_3_4]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.login(USER, PASS).await);
        res!(c.examine("INBOX").await);
        let uids = res!(c.uid_search("UID 1:*").await);
        req!(vec![3u32, 4], uids, "the retried search lost its answer");
        req!(2, res!(connections_of(&seen)), "the drop was not met with one fresh connection");
        let lines = res!(lines_of(&seen));
        req!(2, lines.iter().filter(|l| l.contains("EXAMINE \"INBOX\"")).count(),
            "the mailbox was not re-opened before the retry: {:?}", lines);
        Ok(())
    }

    /// One retry, not a loop: a second drop reaches the caller, tagged as a dropped connection,
    /// and no third connection is made.
    #[tokio::test]
    async fn test_a_second_drop_surfaces_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted_seq(vec![
            (GREET, vec![LOGIN_OK, EXAMINE_7]),
            (GREET, vec![LOGIN_OK, EXAMINE_7]),
            (GREET, vec![LOGIN_OK, EXAMINE_7, SEARCH_3_4]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.login(USER, PASS).await);
        res!(c.examine("INBOX").await);
        match c.uid_search("UID 1:*").await {
            Ok(u)  => return Err(err!(
                "A second drop was papered over with a third connection: {:?}", u; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Interrupted),
                "a dropped connection was not tagged Interrupted: {:?}", e.tags()),
        }
        req!(2, res!(connections_of(&seen)), "the retry was not bounded at one");
        Ok(())
    }

    /// A drop before the greeting is a drop like any other, and `connect` is tried once more.
    #[tokio::test]
    async fn test_a_drop_at_the_greeting_is_retried_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted_seq(vec![
            DROP,
            (GREET, vec![LOGIN_OK]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.login(USER, PASS).await);
        req!(2, res!(connections_of(&seen)));
        Ok(())
    }

    /// A mailbox re-opened under a different UIDVALIDITY has renumbered its messages, so a UID
    /// read on the old numbering would fetch the wrong ones. The retry refuses rather than answer.
    #[tokio::test]
    async fn test_a_changed_uidvalidity_is_not_papered_over_00() -> Outcome<()> {
        let (addr, _) = res!(scripted_seq(vec![
            (GREET, vec![LOGIN_OK, EXAMINE_7]),
            (GREET, vec![LOGIN_OK, EXAMINE_8, SEARCH_3_4]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.login(USER, PASS).await);
        res!(c.examine("INBOX").await);
        match c.uid_search("UID 1:*").await {
            Ok(u)  => return Err(err!(
                "A search across a UIDVALIDITY change was answered: {:?}", u; Test, Invalid)),
            Err(e) => {
                req!(true, e.tags().contains(&ErrTag::Mismatch),
                    "the renumbering was not called a mismatch: {:?}", e.tags());
                req!(false, e.tags().contains(&ErrTag::Interrupted),
                    "a renumbered mailbox read as a dropped connection: {:?}", e.tags());
            },
        }
        Ok(())
    }

    /// A write is not repeated: whether a dropped `UID STORE` took effect cannot be known, so the
    /// drop goes to the caller and no second connection is made.
    #[tokio::test]
    async fn test_a_dropped_write_is_not_retried_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted_seq(vec![
            (GREET, vec![LOGIN_OK, "* 2 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n%T% OK SELECT done\r\n"]),
            (GREET, vec![LOGIN_OK, EXAMINE_7, "%T% OK STORE done\r\n"]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.login(USER, PASS).await);
        res!(c.select("INBOX").await);
        match c.uid_store_flags(&UidSet::from([3]), &["\\Seen"], &[]).await {
            Ok(())  => return Err(err!("A dropped STORE was reported done."; Test, Invalid)),
            Err(e)  => req!(true, e.tags().contains(&ErrTag::Interrupted),
                "the dropped STORE was not tagged Interrupted: {:?}", e.tags()),
        }
        req!(1, res!(connections_of(&seen)), "a write was retried");
        Ok(())
    }

    /// A refused password is the user's to fix, so it says so in its tags, and it is not retried.
    #[tokio::test]
    async fn test_a_refused_login_is_tagged_unauthorised_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted_seq(vec![
            (GREET, vec!["%T% NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"]),
            (GREET, vec![LOGIN_OK]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        match c.login(USER, PASS).await {
            Ok(())  => return Err(err!("A refused LOGIN was a success."; Test, Invalid)),
            Err(e)  => req!(true, e.tags().contains(&ErrTag::Unauthorised),
                "a refused LOGIN was not tagged Unauthorised: {:?}", e.tags()),
        }
        req!(1, res!(connections_of(&seen)), "a refused LOGIN was retried");
        Ok(())
    }

    #[tokio::test]
    async fn test_a_missing_mailbox_is_tagged_not_found_00() -> Outcome<()> {
        let (addr, _) = res!(scripted_seq(vec![
            (GREET, vec![LOGIN_OK, "%T% NO [NONEXISTENT] Unknown Mailbox\r\n"]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.login(USER, PASS).await);
        match c.examine("Nope").await {
            Ok(_)  => return Err(err!("A missing mailbox was opened."; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::NotFound),
                "a missing mailbox was not tagged NotFound: {:?}", e.tags()),
        }
        Ok(())
    }

    /// Nothing listening is not a dropped connection: there was never one to drop.
    #[tokio::test]
    async fn test_a_closed_port_is_tagged_unreachable_00() -> Outcome<()> {
        let listener = res!(TcpListener::bind("127.0.0.1:0").await
            .map_err(|e| err!(e, "Binding a port to close."; IO, Network)));
        let addr = res!(listener.local_addr()
            .map_err(|e| err!(e, "Reading its address."; IO, Network)));
        drop(listener);
        match ImapClient::connect(&cfg(addr)).await {
            Ok(_)  => return Err(err!("A closed port answered."; Test, Invalid)),
            Err(e) => {
                req!(true, e.tags().contains(&ErrTag::Unreachable),
                    "a refused connection was not tagged Unreachable: {:?}", e.tags());
                req!(false, e.tags().contains(&ErrTag::Interrupted),
                    "a refused connection read as a dropped one: {:?}", e.tags());
            },
        }
        Ok(())
    }

    /// An implicit-TLS server vouched for by a private CA, which greets each of `n` connections.
    async fn private_tls_server(n: usize) -> Outcome<(SocketAddr, String)> {
        use tokio_rustls::{rustls::ServerConfig, TlsAcceptor};
        let mint = res!(tls::Mint::new(&[tls::NameRule::Exact(fmt!("imap.world.test"))], 1));
        let ca   = mint.ca_pem();
        let scfg = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(mint));
        let acceptor = TlsAcceptor::from(Arc::new(scfg));
        let listener = res!(TcpListener::bind("127.0.0.1:0").await
            .map_err(|e| err!(e, "Binding the test server."; Test, IO)));
        let addr = res!(listener.local_addr()
            .map_err(|e| err!(e, "Reading its address."; Test, IO)));
        tokio::spawn(async move {
            for _ in 0..n {
                if let Ok((sock, _)) = listener.accept().await {
                    if let Ok(mut s) = acceptor.accept(sock).await {
                        let _ = s.write_all(GREET.as_bytes()).await;
                        let _ = s.flush().await;
                    }
                }
            }
        });
        Ok((addr, ca))
    }

    /// The caller's roots reach the handshake through the config: the private CA is refused by
    /// the default roots and trusted once the config carries it.
    #[tokio::test]
    async fn test_a_config_carrying_a_private_root_connects_00() -> Outcome<()> {
        let (addr, ca) = res!(private_tls_server(2).await);
        let base = ImapConfig::new(fmt!("imap.world.test"), addr.port(), Security::ImplicitTls)
            .with_addr(addr)
            .with_retries(0)
            .with_timeout(Duration::from_secs(10));
        match ImapClient::connect(&base).await {
            Ok(_)  => return Err(err!("A private CA was trusted unasked."; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Security),
                "the refusal was not tagged Security: {:?}", e.tags()),
        }
        let tls_cfg = Arc::new(res!(tls::client_config_trusting(&tls::TrustConfig::default(), ca.as_bytes())));
        let c = res!(ImapClient::connect(&base.with_tls(tls_cfg)).await);
        req!(true, c.has_cap("SPECIAL-USE"), "the greeting was not read over TLS");
        Ok(())
    }

    /// A server that accepts and hangs up before the TLS handshake finishes has dropped the
    /// connection. That is not a certificate fault, and must not be tagged as one.
    #[tokio::test]
    async fn test_a_drop_in_the_tls_handshake_is_not_a_certificate_fault_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted_seq(vec![DROP, DROP]).await);
        let cfg = ImapConfig::new(fmt!("localhost"), addr.port(), Security::ImplicitTls)
            .with_addr(addr)
            .with_timeout(Duration::from_secs(10));
        match ImapClient::connect(&cfg).await {
            Ok(_)  => return Err(err!("A dropped handshake made a client."; Test, Invalid)),
            Err(e) => {
                req!(true, e.tags().contains(&ErrTag::Interrupted),
                    "a dropped handshake was not tagged Interrupted: {:?}", e.tags());
                req!(false, e.tags().contains(&ErrTag::Security),
                    "a dropped handshake was tagged as a security fault: {:?}", e.tags());
            },
        }
        req!(2, res!(connections_of(&seen)), "the dropped handshake was not tried once more");
        Ok(())
    }

    // ── Deltas, copies, moves and expunges (F1) ─────────────────────

    /// A greeting from a server with the extensions a modern Dovecot offers.
    const GREET_PLUS: &str = "* OK [CAPABILITY IMAP4rev1 SPECIAL-USE UIDPLUS MOVE ENABLE \
                              CONDSTORE QRESYNC ID] ready\r\n";
    const SELECT_OK: &str  = "%T% OK [READ-WRITE] SELECT completed\r\n";

    /// Flags go as deltas, one `+FLAGS` and one `-FLAGS`, never an absolute list, which would
    /// overwrite a flag set elsewhere since the client last looked.
    #[tokio::test]
    async fn test_a_flag_change_goes_as_deltas_only_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec![
            SELECT_OK, "%T% OK STORE done\r\n", "%T% OK STORE done\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        res!(c.uid_store_flags(&UidSet::from([3, 4]), &["\\Seen"], &["\\Flagged", "$Junk"]).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.ends_with("UID STORE 3:4 +FLAGS.SILENT (\\Seen)")),
            "no +FLAGS: {:?}", lines);
        req!(true, lines.iter().any(|l| l.ends_with("UID STORE 3:4 -FLAGS.SILENT (\\Flagged $Junk)")),
            "no -FLAGS: {:?}", lines);
        req!(false, lines.iter().any(|l| l.contains(" FLAGS.SILENT")),
            "an absolute FLAGS was sent: {:?}", lines);
        Ok(())
    }

    /// RFC 4315's example: `COPYUID` pairs each source UID with its new one, by position.
    #[tokio::test]
    async fn test_uid_copy_reads_copyuid_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            SELECT_OK, "%T% OK [COPYUID 38505 304,319:320 3956:3958] Done\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let got = res!(c.uid_copy(&res!(UidSet::parse("304,319:320")), "Archive/2026").await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.ends_with("UID COPY 304,319:320 \"Archive/2026\"")),
            "the COPY was not what was expected: {:?}", lines);
        let got = res!(got.ok_or_else(|| err!("No COPYUID was read."; Test, Missing)));
        req!(38505, got.uid_validity);
        req!(vec![(304, 3956), (319, 3957), (320, 3958)], got.pairs);
        Ok(())
    }

    /// Without `UIDPLUS` there is no `COPYUID`, which is no answer rather than an error.
    #[tokio::test]
    async fn test_a_copy_without_uidplus_has_no_copyuid_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(GREET, vec![SELECT_OK, "%T% OK COPY done\r\n"]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        req!(true, res!(c.uid_copy(&UidSet::from([5]), "Trash").await).is_none());
        Ok(())
    }

    /// RFC 6851's example: a `MOVE` carries `COPYUID` on an untagged `OK`, before the expunges.
    #[tokio::test]
    async fn test_uid_move_reads_copyuid_from_the_untagged_ok_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            SELECT_OK,
            "* OK [COPYUID 432432 42:43 11:12]\r\n* 22 EXPUNGE\r\n* 22 EXPUNGE\r\n\
             %T% OK Move completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let got = res!(c.uid_move(&UidSet::from([42, 43]), "Junk").await);
        let lines = res!(lines_of(&seen));
        req!(true, lines.iter().any(|l| l.ends_with("UID MOVE 42:43 \"Junk\"")),
            "the MOVE was not what was expected: {:?}", lines);
        let got = res!(got.ok_or_else(|| err!("No COPYUID was read."; Test, Missing)));
        req!(vec![(42, 11), (43, 12)], got.pairs);
        Ok(())
    }

    /// A server without `MOVE` is not sent one: the fallback is the caller's to choose.
    #[tokio::test]
    async fn test_a_move_is_not_sent_to_a_server_without_it_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec![SELECT_OK]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let before = res!(lines_of(&seen)).len();
        match c.uid_move(&UidSet::from([1]), "Trash").await {
            Ok(_)  => return Err(err!("A MOVE went to a server without it."; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Unimplemented), "{:?}", e.tags()),
        }
        req!(before, res!(lines_of(&seen)).len(), "a command was sent");
        Ok(())
    }

    /// `UID EXPUNGE` removes only the UIDs named. Without `UIDPLUS` it is refused, never replaced
    /// by a plain `EXPUNGE`, which would remove every `\Deleted` message in the mailbox.
    #[tokio::test]
    async fn test_uid_expunge_names_its_uids_or_is_refused_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            SELECT_OK, "* 3 EXPUNGE\r\n* 3 EXPUNGE\r\n* 3 EXPUNGE\r\n%T% OK Expunge completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        res!(c.uid_expunge(&res!(UidSet::parse("3000:3002"))).await);
        req!(true, res!(lines_of(&seen)).iter().any(|l| l.ends_with("UID EXPUNGE 3000:3002")));

        let (addr, seen) = res!(scripted(GREET, vec![SELECT_OK]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        match c.uid_expunge(&UidSet::from([7])).await {
            Ok(())  => return Err(err!("An expunge went to a server without UIDPLUS."; Test, Invalid)),
            Err(e)  => req!(true, e.tags().contains(&ErrTag::Unimplemented), "{:?}", e.tags()),
        }
        req!(false, res!(lines_of(&seen)).iter().any(|l| l.contains("EXPUNGE")),
            "an EXPUNGE was sent");
        Ok(())
    }

    /// RFC 4315's example: `APPENDUID` names the new message.
    #[tokio::test]
    async fn test_append_returns_appenduid_00() -> Outcome<()> {
        let (addr, _) = res!(scripted(GREET_PLUS, vec![
            "+ Ready for literal data\r\n", "%T% OK [APPENDUID 38505 3955] APPEND completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let got = res!(c.append("Sent", &["\\Seen"], b"Subject: x\r\n\r\ny\r\n").await);
        req!(Some(AppendUid { uid_validity: 38505, uid: 3955 }), got);
        Ok(())
    }


    // ── ENABLE, CONDSTORE and QRESYNC (F1 part 2) ───────────────────

    const ENABLED_QRESYNC: &str = "* ENABLED QRESYNC\r\n%T% OK Enabled\r\n";

    /// RFC 5161's example: the server names what it turned on, which may be less than was asked.
    #[tokio::test]
    async fn test_enable_reads_what_was_enabled_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            "* ENABLED X-GOOD-IDEA\r\n%T% OK foo\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let got = res!(c.enable(&["CONDSTORE", "X-GOOD-IDEA"]).await);
        req!(vec![fmt!("X-GOOD-IDEA")], got);
        req!(true, res!(lines_of(&seen)).iter().any(|l| l.ends_with("ENABLE CONDSTORE X-GOOD-IDEA")));
        req!(true, c.is_enabled("x-good-idea"));
        req!(false, c.is_enabled("CONDSTORE"), "an extension the server did not enable is held on");
        Ok(())
    }

    /// No `ENABLE` capability, no `ENABLE` command.
    #[tokio::test]
    async fn test_enable_is_not_sent_without_the_capability_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec![]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        match c.enable(&["QRESYNC"]).await {
            Ok(v)  => return Err(err!("ENABLE went to a server without it: {:?}", v; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Unimplemented), "{:?}", e.tags()),
        }
        req!(0, res!(lines_of(&seen)).len(), "a command was sent");
        Ok(())
    }

    /// RFC 7162 §3.2.5.1's example: a QRESYNC `SELECT` returns the mailbox's HIGHESTMODSEQ, the
    /// known UIDs that vanished, and the messages whose flags changed. QRESYNC is enabled first,
    /// because a server must refuse the parameter otherwise.
    #[tokio::test]
    async fn test_a_qresync_select_reads_vanished_and_changed_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            ENABLED_QRESYNC,
            "* 314 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 67890007] UIDVALIDITY\r\n\
             * OK [UIDNEXT 567] Predicted next UID\r\n\
             * FLAGS (\\Answered \\Flagged \\Draft \\Deleted \\Seen)\r\n\
             * OK [PERMANENTFLAGS (\\Answered \\Flagged \\Draft \\Deleted \\Seen \\*)] Permanent flags\r\n\
             * OK [HIGHESTMODSEQ 20060128194045007] Highest mailbox mod-sequence\r\n\
             * VANISHED (EARLIER) 41,43:116,118,120:211,214:540\r\n\
             * 49 FETCH (UID 117 FLAGS (\\Seen \\Answered) MODSEQ (20060115194045000))\r\n\
             * 50 FETCH (UID 119 FLAGS (\\Draft $MDNSent) MODSEQ (20060115194045308))\r\n\
             * 51 FETCH (UID 541 FLAGS (\\Seen $Forwarded) MODSEQ (20060115194045001))\r\n\
             %T% OK [READ-WRITE] mailbox selected\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let with = SelectWith::QResync {
            uid_validity:   67890007,
            modseq:         20050715194045000,
            known:          Some(res!(UidSet::parse("41,43:211,214:541"))),
        };
        let st = res!(c.select_with("INBOX", &with).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines[0].ends_with("ENABLE QRESYNC"), "QRESYNC was not enabled first: {:?}", lines);
        req!(true, lines[1].ends_with(
            "SELECT \"INBOX\" (QRESYNC (67890007 20050715194045000 41,43:211,214:541))"),
            "the SELECT was not what was expected: {:?}", lines);
        req!(Some(20060128194045007u64), st.highest_modseq);
        req!(res!(UidSet::parse("41,43:116,118,120:211,214:540")), st.vanished);
        req!(vec![117u32, 119, 541], st.changed.iter().map(|m| m.uid).collect::<Vec<_>>());
        req!(Some(20060115194045000u64), st.changed[0].modseq);
        req!(vec![fmt!("\\Draft"), fmt!("$MDNSent")], st.changed[1].flags.clone());
        req!(314, st.exists);
        Ok(())
    }

    /// A server without QRESYNC is not sent a QRESYNC select, nor an `ENABLE` for one.
    #[tokio::test]
    async fn test_a_qresync_select_is_refused_where_not_offered_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec![]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let with = SelectWith::QResync { uid_validity: 1, modseq: 2, known: None };
        match c.select_with("INBOX", &with).await {
            Ok(_)  => return Err(err!("A QRESYNC select went to a server without it."; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Unimplemented), "{:?}", e.tags()),
        }
        match c.select_with("INBOX", &SelectWith::CondStore).await {
            Ok(_)  => return Err(err!("A CONDSTORE select went to a server without it."; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Unimplemented), "{:?}", e.tags()),
        }
        req!(0, res!(lines_of(&seen)).len(), "a command was sent");
        Ok(())
    }

    /// RFC 7162 §3.1.2's example: `(CONDSTORE)` on the select, and `HIGHESTMODSEQ` in the reply.
    /// Once the mailbox has mod-sequences, a fetch asks for `MODSEQ`; a `NOMODSEQ` one does not.
    #[tokio::test]
    async fn test_a_condstore_select_reads_highestmodseq_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            "* 172 EXISTS\r\n* OK [UIDVALIDITY 3857529045] UIDVALIDITY\r\n\
             * OK [HIGHESTMODSEQ 715194045007]\r\n%T% OK [READ-WRITE] SELECT completed\r\n",
            "* 1 FETCH (UID 4 MODSEQ (65402) FLAGS (\\Seen))\r\n%T% OK done\r\n",
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 9] x\r\n* OK [NOMODSEQ] Sorry\r\n%T% OK [READ-ONLY] done\r\n",
            "* 1 FETCH (UID 4 FLAGS (\\Seen))\r\n%T% OK done\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let st = res!(c.select_with("INBOX", &SelectWith::CondStore).await);
        req!(Some(715194045007u64), st.highest_modseq);
        let got = res!(c.uid_fetch(&UidSet::from([4]), FetchWhat::Flags).await);
        req!(Some(65402u64), got[0].modseq);
        let st = res!(c.examine_with("Shared", &SelectWith::CondStore).await);
        req!(None::<u64>, st.highest_modseq, "NOMODSEQ read as a mod-sequence");
        res!(c.uid_fetch(&UidSet::from([4]), FetchWhat::Flags).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines[0].ends_with("SELECT \"INBOX\" (CONDSTORE)"), "{:?}", lines);
        req!(true, lines[1].ends_with("UID FETCH 4 (UID FLAGS MODSEQ)"), "{:?}", lines);
        req!(true, lines[2].ends_with("EXAMINE \"Shared\" (CONDSTORE)"), "{:?}", lines);
        req!(true, lines[3].ends_with("UID FETCH 4 (UID FLAGS)"), "{:?}", lines);
        Ok(())
    }

    /// RFC 7162 §3.2.6's example: with QRESYNC on, `CHANGEDSINCE` asks for `VANISHED` too, and
    /// the `VANISHED (EARLIER)` reply names the UIDs expunged among those asked about.
    #[tokio::test]
    async fn test_changedsince_reads_vanished_earlier_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            ENABLED_QRESYNC,
            "* 10 EXISTS\r\n* OK [UIDVALIDITY 5] x\r\n* OK [HIGHESTMODSEQ 90000] x\r\n%T% OK done\r\n",
            "* VANISHED (EARLIER) 300:310,405,411\r\n\
             * 1 FETCH (UID 404 MODSEQ (65402) FLAGS (\\Seen))\r\n\
             * 2 FETCH (UID 406 MODSEQ (75403) FLAGS (\\Deleted))\r\n\
             * 4 FETCH (UID 408 MODSEQ (29738) FLAGS ($NoJunk $AutoJunk $MDNSent))\r\n\
             %T% OK Conditional UID FETCH completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.enable(&["QRESYNC"]).await);
        res!(c.select("INBOX").await);
        let got = res!(c.uid_fetch_changed(&UidSet::from_range(300, 500), FetchWhat::Flags, 12345).await);
        let lines = res!(lines_of(&seen));
        req!(true, lines[2].ends_with("UID FETCH 300:500 (UID FLAGS MODSEQ) (CHANGEDSINCE 12345 VANISHED)"),
            "{:?}", lines);
        req!(res!(UidSet::parse("300:310,405,411")), got.vanished);
        req!(vec![404u32, 406, 408], got.messages.iter().map(|m| m.uid).collect::<Vec<_>>());
        req!(Some(29738u64), got.messages[2].modseq);
        Ok(())
    }

    /// RFC 7162 §3.1.4.1: CONDSTORE without QRESYNC asks no `VANISHED`, which such a server would
    /// refuse; a server with neither is sent no `CHANGEDSINCE`.
    #[tokio::test]
    async fn test_changedsince_without_qresync_asks_no_vanished_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted("* OK [CAPABILITY IMAP4rev1 CONDSTORE] ready\r\n", vec![
            SELECT_OK,
            "* 1 FETCH (UID 4 MODSEQ (65402) FLAGS (\\Seen))\r\n\
             * 2 FETCH (UID 6 MODSEQ (75403) FLAGS (\\Deleted))\r\n%T% OK done\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let got = res!(c.uid_fetch_changed(&UidSet::from_open(1), FetchWhat::Flags, 12345).await);
        req!(true, res!(lines_of(&seen)).iter()
            .any(|l| l.ends_with("UID FETCH 1:* (UID FLAGS MODSEQ) (CHANGEDSINCE 12345)")));
        req!(2, got.messages.len());
        req!(true, got.vanished.is_empty());

        let (addr, seen) = res!(scripted(GREET, vec![SELECT_OK]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        match c.uid_fetch_changed(&UidSet::from_open(1), FetchWhat::Flags, 1).await {
            Ok(_)  => return Err(err!("CHANGEDSINCE went to a server without CONDSTORE."; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Unimplemented), "{:?}", e.tags()),
        }
        req!(1, res!(lines_of(&seen)).len(), "a command was sent");
        Ok(())
    }

    /// A drop under a read on a QRESYNC mailbox: the fresh connection logs in, enables QRESYNC
    /// again, and repeats the select with its parameters before asking again.
    #[tokio::test]
    async fn test_a_reconnect_enables_again_and_repeats_the_select_00() -> Outcome<()> {
        const SEL: &str = "* 2 EXISTS\r\n* OK [UIDVALIDITY 7] x\r\n* OK [HIGHESTMODSEQ 99] x\r\n\
                           %T% OK [READ-ONLY] EXAMINE completed\r\n";
        let (addr, seen) = res!(scripted_seq(vec![
            (GREET_PLUS, vec![LOGIN_OK, ENABLED_QRESYNC, SEL]),
            (GREET_PLUS, vec![LOGIN_OK, ENABLED_QRESYNC, SEL, SEARCH_3_4]),
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.login(USER, PASS).await);
        let with = SelectWith::QResync { uid_validity: 7, modseq: 50, known: None };
        res!(c.examine_with("INBOX", &with).await);
        req!(vec![3u32, 4], res!(c.uid_search("ALL").await));
        let lines = res!(lines_of(&seen));
        req!(2, lines.iter().filter(|l| l.ends_with("ENABLE QRESYNC")).count(), "{:?}", lines);
        req!(2, lines.iter().filter(|l| l.ends_with("EXAMINE \"INBOX\" (QRESYNC (7 50))")).count(),
            "{:?}", lines);
        req!(true, c.is_enabled("QRESYNC"));
        Ok(())
    }

    // ── STATUS, ID, SUBSCRIBE and LSUB (F1 part 2) ──────────────────

    /// RFC 7162 §3.1.3's example: `HIGHESTMODSEQ` is asked for only where CONDSTORE is offered.
    #[tokio::test]
    async fn test_status_reads_highestmodseq_where_offered_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            "* STATUS blurdybloop (MESSAGES 231 UIDNEXT 7500 UIDVALIDITY 3 UNSEEN 4 \
             HIGHESTMODSEQ 7011231777)\r\n%T% OK STATUS completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let st = res!(c.status("blurdybloop").await);
        req!(true, res!(lines_of(&seen))[0].ends_with(
            "STATUS \"blurdybloop\" (MESSAGES UIDNEXT UIDVALIDITY UNSEEN HIGHESTMODSEQ)"));
        req!(231, st.messages);
        req!(7500, st.uid_next);
        req!(3, st.uid_validity);
        req!(4, st.unseen);
        req!(Some(7011231777u64), st.highest_modseq);

        let (addr, seen) = res!(scripted(GREET, vec![
            "* STATUS {5}\r\nINBOX (MESSAGES 2 UIDNEXT 9 UIDVALIDITY 1 UNSEEN 0)\r\n%T% OK done\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let st = res!(c.status("INBOX").await);
        req!(false, res!(lines_of(&seen))[0].contains("HIGHESTMODSEQ"));
        req!(2, st.messages);
        req!(None::<u64>, st.highest_modseq);
        Ok(())
    }

    /// RFC 2971's example: `ID` sends the client's fields and reads the server's.
    #[tokio::test]
    async fn test_id_exchanges_fields_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET_PLUS, vec![
            "* ID (\"name\" \"Cyrus\" \"version\" \"1.5\" \"os\" \"sunos\" \"os-version\" \"5.5\" \
             \"support-url\" \"mailto:cyrus-bugs+@andrew.cmu.edu\" \"vendor\" NIL)\r\n\
             %T% OK ID completed\r\n",
            "* ID NIL\r\n%T% OK ID completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        let got = res!(c.id(&[("name", "Daimond"), ("version", "0.5")]).await);
        req!(6, got.len());
        req!((fmt!("name"), Some(fmt!("Cyrus"))), got[0].clone());
        req!((fmt!("vendor"), None::<String>), got[5].clone());
        req!(0, res!(c.id(&[]).await).len());
        let lines = res!(lines_of(&seen));
        req!(true, lines[0].ends_with("ID (\"name\" \"Daimond\" \"version\" \"0.5\")"), "{:?}", lines);
        req!(true, lines[1].ends_with("ID NIL"), "{:?}", lines);

        let (addr, seen) = res!(scripted(GREET, vec![]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        match c.id(&[]).await {
            Ok(_)  => return Err(err!("ID went to a server without it."; Test, Invalid)),
            Err(e) => req!(true, e.tags().contains(&ErrTag::Unimplemented), "{:?}", e.tags()),
        }
        req!(0, res!(lines_of(&seen)).len());
        Ok(())
    }

    /// `SUBSCRIBE`, `UNSUBSCRIBE` and `LSUB` on the wire.
    #[tokio::test]
    async fn test_subscribe_and_lsub_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec![
            "%T% OK SUBSCRIBE completed\r\n",
            "%T% OK UNSUBSCRIBE completed\r\n",
            "* LSUB () \".\" #news.comp.mail.misc\r\n* LSUB (\\Sent) \".\" \"Sent Items\"\r\n\
             %T% OK LSUB completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.subscribe("Sent Items").await);
        res!(c.unsubscribe("Old").await);
        let got = res!(c.lsub("", "*").await);
        let lines = res!(lines_of(&seen));
        req!(true, lines[0].ends_with("SUBSCRIBE \"Sent Items\""), "{:?}", lines);
        req!(true, lines[1].ends_with("UNSUBSCRIBE \"Old\""), "{:?}", lines);
        req!(true, lines[2].ends_with("LSUB \"\" \"*\""), "{:?}", lines);
        req!(vec![fmt!("#news.comp.mail.misc"), fmt!("Sent Items")],
            got.iter().map(|m| m.name.clone()).collect::<Vec<_>>());
        req!(Some(SpecialUse::Sent), got[1].special_use());
        Ok(())
    }

    // ── The confirming fetch and Gmail's items (F1 part 2) ──────────

    /// A loose server's `SEARCH` names three UIDs; the header `FETCH` confirms one, and only it is
    /// kept, with its size, for a caller re-resolving by Message-ID and size.
    #[tokio::test]
    async fn test_a_message_id_search_keeps_only_confirmed_matches_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted(GREET, vec![
            SELECT_OK,
            "* SEARCH 4 7 9\r\n%T% OK SEARCH completed\r\n",
            "* 1 FETCH (UID 4 RFC822.SIZE 900 BODY[HEADER.FIELDS (MESSAGE-ID)] {32}\r\n\
             Message-ID: <xabc@example.org>\r\n)\r\n\
             * 2 FETCH (UID 7 RFC822.SIZE 1200 BODY[HEADER.FIELDS (MESSAGE-ID)] {35}\r\n\
             Message-Id:\r\n <abc@example.org>\r\n\r\n)\r\n\
             * 3 FETCH (UID 9 RFC822.SIZE 50 BODY[HEADER.FIELDS (MESSAGE-ID)] {2}\r\n\r\n)\r\n\
             %T% OK FETCH completed\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let got = res!(c.uid_search_message_id("<abc@example.org>").await);
        let lines = res!(lines_of(&seen));
        req!(true, lines[1].ends_with("UID SEARCH HEADER Message-ID \"abc@example.org\""), "{:?}", lines);
        req!(true, lines[2].ends_with(
            "UID FETCH 4,7,9 (UID RFC822.SIZE BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])"), "{:?}", lines);
        req!(vec![(7u32, 1200u32)], got.iter().map(|m| (m.uid, m.size)).collect::<Vec<_>>());
        Ok(())
    }

    /// Gmail's `X-GM-MSGID` and `X-GM-LABELS` are asked for only where `X-GM-EXT-1` is offered,
    /// and read back.
    #[tokio::test]
    async fn test_gmail_items_only_where_offered_00() -> Outcome<()> {
        let (addr, seen) = res!(scripted("* OK [CAPABILITY IMAP4rev1 X-GM-EXT-1] Gimap ready\r\n", vec![
            SELECT_OK,
            "* 1 FETCH (X-GM-MSGID 1278455344230334865 X-GM-LABELS (\\Inbox \\Sent Important \
             \"Muy Importante\") UID 3 FLAGS (\\Seen))\r\n%T% OK Success\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let got = res!(c.uid_fetch(&UidSet::from([3]), FetchWhat::Flags).await);
        req!(true, res!(lines_of(&seen))[1].ends_with("UID FETCH 3 (UID FLAGS X-GM-MSGID X-GM-LABELS)"));
        req!(Some(1278455344230334865u64), got[0].gm_msgid);
        req!(vec![fmt!("\\Inbox"), fmt!("\\Sent"), fmt!("Important"), fmt!("Muy Importante")],
            got[0].gm_labels.clone());

        let (addr, seen) = res!(scripted(GREET, vec![
            SELECT_OK, "* 1 FETCH (UID 3 FLAGS (\\Seen))\r\n%T% OK done\r\n",
        ]).await);
        let mut c = res!(ImapClient::connect(&cfg(addr)).await);
        res!(c.select("INBOX").await);
        let got = res!(c.uid_fetch(&UidSet::from([3]), FetchWhat::Flags).await);
        req!(false, res!(lines_of(&seen))[1].contains("X-GM-"));
        req!(None::<u64>, got[0].gm_msgid);
        Ok(())
    }

}
