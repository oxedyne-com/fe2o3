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
//!   `UID STORE`, `APPEND`, `LOGOUT`.
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
//! # use oxedyne_fe2o3_net::imap::client::{FetchWhat, ImapClient, ImapConfig, Security};
//! # async fn f() -> Outcome<()> {
//! let cfg = ImapConfig::new("imap.example.com", 993, Security::ImplicitTls);
//! let mut c = res!(ImapClient::connect(&cfg).await);
//! res!(c.login("alice@example.com", "app-password").await);
//! let inbox = res!(c.select("INBOX").await);
//! let uids  = res!(c.uid_search(&fmt!("UID {}:*", inbox.uid_next.saturating_sub(10))).await);
//! for msg in res!(c.uid_fetch(&uids, FetchWhat::Full).await) {
//!     println!("{} bytes, flags {:?}", msg.body.len(), msg.flags);
//! }
//! res!(c.logout().await);
//! # Ok(())
//! # }
//! ```
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::tls::{
    self,
    ClientStream,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
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


// Generous, because a large `UID FETCH` against a slow mailbox is a legitimate
// multi-second read.
pub const IMAP_CLIENT_TIMEOUT: Duration = Duration::from_secs(60);

// A server announcing a 4 GB literal is either broken or hostile, and either way
// the client should not try to allocate for it.
pub const MAX_LITERAL_BYTES: usize = 64 * 1024 * 1024;


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
        }
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


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RESULT TYPES                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// One mailbox as reported by `LIST`.
#[derive(Clone, Debug)]
pub struct MailboxInfo {
    pub name:       String,         // as the server spells it
    pub delimiter:  Option<char>,   // `None` for a flat namespace
    pub attrs:      Vec<String>,    // e.g. `\HasChildren`, `\Noselect`, `\Sent`
}

impl MailboxInfo {
    /// A mailbox that exists only to hold children cannot itself be selected.
    pub fn selectable(&self) -> bool {
        !self.attrs.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect"))
    }

    /// The RFC 6154 special-use role, where the server declared one. This is how a client
    /// recognises the Sent folder on a mailbox whose
    /// names are localised -- Gmail spells it "[Gmail]/Gesendet" for a German
    /// account, but the attribute is `\Sent` everywhere.
    pub fn special_use(&self) -> Option<SpecialUse> {
        for a in &self.attrs {
            let role = match a.to_uppercase().as_str() {
                "\\SENT"     => Some(SpecialUse::Sent),
                "\\DRAFTS"   => Some(SpecialUse::Drafts),
                "\\TRASH"    => Some(SpecialUse::Trash),
                "\\JUNK"     => Some(SpecialUse::Junk),
                "\\ARCHIVE"  => Some(SpecialUse::Archive),
                "\\ALL"      => Some(SpecialUse::All),
                "\\FLAGGED"  => Some(SpecialUse::Flagged),
                _            => None,
            };
            if role.is_some() {
                return role;
            }
        }
        None
    }
}

/// The special-use roles of RFC 6154, one per mailbox at most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialUse {
    Sent,
    Drafts,
    Trash,      // deleted messages await expunge
    Junk,       // spam
    Archive,    // filed out of the inbox
    All,        // every message however filed, Gmail's "All Mail"
    Flagged,    // starred or flagged
}

/// The state of a mailbox after `SELECT` or `EXAMINE`.
///
/// `uid_validity` is the one field a synchronising caller must persist:
/// if it changes, every UID it has cached is meaningless and the mailbox
/// must be re-read from scratch.
#[derive(Clone, Debug, Default)]
pub struct MailboxStatus {
    pub name:           String,
    pub exists:         u32,            // messages present
    pub recent:         u32,            // messages flagged `\Recent`
    pub uid_validity:   u32,            // UID namespace generation
    pub uid_next:       u32,            // the UID the next arrival will be given
    pub flags:          Vec<String>,    // defined in this mailbox
    pub read_only:      bool,           // `EXAMINE`, or a server that downgraded the `SELECT`
}

/// How much of each message to pull down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchWhat {
    Meta,       // UID, flags, internal date, size; no body at all
    Headers,    // and the RFC 5322 header block
    Full,       // and the whole message
}

impl FetchWhat {
    /// `BODY.PEEK` rather than `BODY`, so reading a message does not silently mark it `\Seen` --
    /// a sync should be invisible to whoever is reading the mailbox elsewhere.
    fn items(&self) -> &'static str {
        match self {
            Self::Meta    => "(UID FLAGS INTERNALDATE RFC822.SIZE)",
            Self::Headers => "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])",
            Self::Full    => "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[])",
        }
    }
}

/// One message returned by `UID FETCH`.
#[derive(Clone, Debug, Default)]
pub struct FetchedMessage {
    pub seq:            u32,            // sequence number in the current mailbox view
    pub uid:            u32,            // stable within the mailbox's current `uid_validity`
    pub flags:          Vec<String>,    // e.g. `\Seen`, `\Answered`
    // `INTERNALDATE` exactly as the server gave it, e.g. `01-Jan-2026 09:15:00 +0000`. Left as
    // the server's string, because only the caller knows what calendar it wants it in.
    pub internal_date:  String,
    pub size:           u32,            // `RFC822.SIZE`, which may exceed `body.len()`
    pub body:           Vec<u8>,        // whole message, header block, or empty, per FetchWhat
}

/// What a `UID STORE` does to the named flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlagOp {
    Add,        // leaving the others alone
    Remove,     // leaving the others alone
    Set,        // replacing the flag set entirely
}

impl FlagOp {
    /// `.SILENT` throughout: the client already knows what it asked for, and the untagged FETCH
    /// the server would otherwise send back is a round trip spent on nothing.
    fn item(&self) -> &'static str {
        match self {
            Self::Add    => "+FLAGS.SILENT",
            Self::Remove => "-FLAGS.SILENT",
            Self::Set    => "FLAGS.SILENT",
        }
    }
}

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
}

impl ImapClient {

    /// Protects the connection as configured and reads the greeting. Does not authenticate.
    pub async fn connect(cfg: &ImapConfig) -> Outcome<Self> {
        let tls_cfg = Arc::new(res!(tls::default_client_config()));
        Self::connect_with(cfg, tls_cfg).await
    }

    /// Connect using a caller-supplied rustls config, for a private CA or
    /// a pinned root.
    pub async fn connect_with(
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
            Ok(Err(e)) => return Err(err!(e,
                "Connecting to IMAP server {}.", addr;
                IO, Network)),
            Err(_)     => return Err(err!(
                "Timeout connecting to IMAP server {}.", addr;
                IO, Network, Timeout)),
        };

        let stream = match cfg.security {
            Security::ImplicitTls =>
                res!(tls::upgrade(plain, &cfg.host, tls_cfg.clone()).await),
            Security::StartTls | Security::Plain =>
                ClientStream::Plain(plain),
        };

        let mut client = Self {
            stream:  Some(BufReader::new(stream)),
            tag:     0,
            caps:    Vec::new(),
            timeout: cfg.timeout,
            host:    cfg.host.clone(),
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
            res!(client.capability().await);
        }
        Ok(client)
    }

    /// Upgrade a plain connection in place. The credentials have not been
    /// sent yet, and if the upgrade fails they never will be.
    async fn starttls(&mut self, tls_cfg: Arc<ClientConfig>) -> Outcome<()> {
        if self.caps.is_empty() {
            res!(self.capability().await);
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
        let upgraded = res!(tls::upgrade(plain, &self.host, tls_cfg).await);
        self.stream = Some(BufReader::new(upgraded));

        // Capabilities before and after TLS are allowed to differ, and the
        // pre-TLS set must not be trusted: re-ask.
        self.caps.clear();
        res!(self.capability().await);
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
        let resp = res!(self.command("CAPABILITY").await);
        let mut caps: Vec<String> = Vec::new();
        for line in &resp.untagged {
            caps.extend(parse_capabilities(&line.text));
        }
        caps.sort();
        caps.dedup();
        self.caps = caps;
        Ok(&self.caps)
    }

    /// Does the server advertise this capability? Compared case-insensitively.
    pub fn has_cap(&self, cap: &str) -> bool {
        let want = cap.to_uppercase();
        self.caps.iter().any(|c| *c == want)
    }

    /// For a consumer mailbox the password is an app password, not the account password.
    pub async fn login(&mut self, user: &str, pass: &str) -> Outcome<()> {
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
            res!(self.capability().await);
        }
        Ok(())
    }

    /// SASL `XOAUTH2`, the mechanism the large providers require of a registered application.
    pub async fn authenticate_xoauth2(&mut self, user: &str, token: &str) -> Outcome<()> {
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
            res!(self.capability().await);
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
        let resp = res!(self.command(&cmd).await);
        let mut out = Vec::new();
        for line in &resp.untagged {
            if let Some(mb) = res!(parse_list_line(line)) {
                out.push(mb);
            }
        }
        Ok(out)
    }

    pub async fn select(&mut self, mailbox: &str) -> Outcome<MailboxStatus> {
        self.select_impl(mailbox, false).await
    }

    /// Read-only, so nothing the client does can change a flag. The safe choice for a sync that
    /// must not disturb the mailbox.
    pub async fn examine(&mut self, mailbox: &str) -> Outcome<MailboxStatus> {
        self.select_impl(mailbox, true).await
    }

    async fn select_impl(&mut self, mailbox: &str, read_only: bool) -> Outcome<MailboxStatus> {
        let verb = if read_only { "EXAMINE" } else { "SELECT" };
        let cmd  = fmt!("{} {}", verb, quoted(mailbox));
        let resp = res!(self.command(&cmd).await);

        let mut st = MailboxStatus {
            name:      mailbox.to_string(),
            read_only,
            ..Default::default()
        };
        for line in &resp.untagged {
            res!(absorb_select_line(&mut st, &line.text));
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
        let resp = res!(self.command(&cmd).await);
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

    /// An empty `uids` is a no-op rather than a command, because `UID FETCH ` with no set is a
    /// syntax error and a caller passing an empty search result is not doing anything wrong.
    pub async fn uid_fetch(
        &mut self,
        uids:   &[u32],
        what:   FetchWhat,
    )
        -> Outcome<Vec<FetchedMessage>>
    {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let cmd  = fmt!("UID FETCH {} {}", uid_set(uids), what.items());
        let resp = res!(self.command(&cmd).await);
        let mut out: Vec<FetchedMessage> = Vec::new();
        for line in &resp.untagged {
            if let Some(msg) = res!(parse_fetch_line(line)) {
                out.push(msg);
            }
        }
        Ok(out)
    }

    /// `None` where the server does not return it: expunged between the search and the fetch,
    /// which is a race the caller cannot prevent and should not treat as an error.
    pub async fn uid_fetch_one(&mut self, uid: u32) -> Outcome<Option<FetchedMessage>> {
        let mut msgs = res!(self.uid_fetch(&[uid], FetchWhat::Full).await);
        Ok(if msgs.is_empty() { None } else { Some(msgs.remove(0)) })
    }

    pub async fn uid_store_flags(
        &mut self,
        uids:   &[u32],
        op:     FlagOp,
        flags:  &[&str],
    )
        -> Outcome<()>
    {
        if uids.is_empty() {
            return Ok(());
        }
        let cmd = fmt!("UID STORE {} {} ({})",
            uid_set(uids), op.item(), flags.join(" "));
        res!(self.command(&cmd).await);
        Ok(())
    }

    /// `body` is a raw RFC 5322 message: this is how a sent message is filed in `Sent` after
    /// SMTP has delivered it.
    pub async fn append(
        &mut self,
        mailbox:    &str,
        flags:      &[&str],
        body:       &[u8],
    )
        -> Outcome<()>
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
        let _ = resp;
        Ok(())
    }

    pub async fn logout(&mut self) -> Outcome<()> {
        res!(self.command("LOGOUT").await);
        if let Some(s) = self.stream.as_mut() {
            let _ = s.get_mut().shutdown().await;
        }
        Ok(())
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
                Status::No => Err(err!(
                    "IMAP server refused {}: {}", label, text;
                    IO, Network, Invalid)),
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
            Ok(Err(e))  => return Err(err!(e,
                "Writing to IMAP server {}.", host;
                IO, Network, Write)),
            Err(_)      => return Err(err!(
                "Timeout writing to IMAP server {}.", host;
                IO, Network, Timeout)),
        }
        match timeout(deadline, w.flush()).await {
            Ok(Ok(()))  => Ok(()),
            Ok(Err(e))  => Err(err!(e,
                "Flushing to IMAP server {}.", host;
                IO, Network, Write)),
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
                Ok(Err(e)) => return Err(err!(e,
                    "Reading a {}-byte literal from IMAP server {}.", n, host;
                    IO, Network, Read)),
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
            Ok(Err(e)) => return Err(err!(e,
                "Reading from IMAP server {}.", host;
                IO, Network, Read)),
            Err(_)     => return Err(err!(
                "Timeout reading from IMAP server {}.", host;
                IO, Network, Timeout)),
        };
        if n == 0 {
            return Err(err!(
                "IMAP server {} closed the connection.", host;
                IO, Network, Read));
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

/// Render a UID list as a compact IMAP sequence set, collapsing runs into
/// ranges: `[1,2,3,7,9,10]` becomes `1:3,7,9:10`. A mailbox synced after a
/// week away yields a set of thousands of consecutive UIDs, and a server
/// is entitled to reject a command line that long.
fn uid_set(uids: &[u32]) -> String {
    let mut sorted: Vec<u32> = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut out   = String::new();
    let mut i     = 0usize;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        if !out.is_empty() { out.push(','); }
        if start == end {
            out.push_str(&fmt!("{}", start));
        } else {
            out.push_str(&fmt!("{}:{}", start, end));
        }
        i += 1;
    }
    out
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
        let listener = res!(TcpListener::bind("127.0.0.1:0").await
            .map_err(|e| err!(e, "Binding the scripted IMAP server."; IO, Network)));
        let addr = res!(listener.local_addr()
            .map_err(|e| err!(e, "Reading its address."; IO, Network)));

        let seen: Transcript = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();

        tokio::spawn(async move {
            let (sock, _) = match listener.accept().await {
                Ok(x)  => x,
                Err(_) => return,
            };
            let (r, mut w) = sock.into_split();
            let mut rd = BufReader::new(r);

            let _ = w.write_all(greeting.as_bytes()).await;

            let mut script = script.into_iter();
            while let Some(reply) = script.next() {
                let mut buf: Vec<u8> = Vec::with_capacity(256);
                match rd.read_until(b'\n', &mut buf).await {
                    Ok(0) | Err(_) => return,
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
                            return;
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
                            None => return,
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

    fn lines_of(t: &Transcript) -> Outcome<Vec<String>> {
        match t.lock() {
            Ok(g)  => Ok(g.clone()),
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
        let msgs = res!(c.uid_fetch(&[9], FetchWhat::Full).await);
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
        let msg = match c.uid_fetch(&[9], FetchWhat::Full).await {
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
        let msg = match c.select("INBOX").await {
            Err(e) => fmt!("{}", e),
            Ok(_)  => return Err(err!(
                "A closed connection was reported as a SELECT."; Test, Invalid)),
        };
        req!(true, msg.contains("closed the connection"),
            "the error did not say the server hung up: {}", msg);
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
        res!(c.uid_fetch(&[1, 2, 3, 7, 9, 10], FetchWhat::Meta).await);
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
        req!(true, res!(c.uid_fetch(&[], FetchWhat::Full).await).is_empty());
        res!(c.uid_store_flags(&[], FlagOp::Add, &["\\Seen"]).await);
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
        res!(c.uid_store_flags(&[4, 5], FlagOp::Add, &["\\Seen"]).await);
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
            Ok(())  => return Err(err!("A refused APPEND was a success."; Test, Invalid)),
        };
        req!(true, msg.contains("OVERQUOTA"), "the server's words were dropped: {}", msg);
        req!(false, res!(lines_of(&seen)).iter().any(|l| l.starts_with("<literal ")),
            "the body was sent to a mailbox that had refused it");
        Ok(())
    }

    #[test]
    fn test_uid_set_collapses_runs() {
        assert_eq!(uid_set(&[1, 2, 3, 7, 9, 10]), "1:3,7,9:10");
        assert_eq!(uid_set(&[5]),                 "5");
        assert_eq!(uid_set(&[3, 1, 2]),           "1:3");
        assert_eq!(uid_set(&[]),                  "");
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
}
