//! Server-side SMTP session state machine.
//!
//! Implements RFC 5321 (SMTP), RFC 3207 (STARTTLS), RFC 4954 (AUTH PLAIN
//! and LOGIN) at the level needed to host a single Hematite mailbox: the
//! receive path (port 25, no auth, accepts mail destined to local users)
//! and the submission path (port 587, AUTH required after STARTTLS, may
//! relay anywhere).
//!
//! The session loop runs over an enum-based `MaybeTls` stream so that a
//! plain TCP connection can be transparently swapped to TLS in response
//! to `STARTTLS` without duplicating the rest of the state machine.

use crate::{
    smtp::{
        cmd::SmtpCommand,
        codes::SmtpResponseCode,
        handler::{
            HandlerOutcome,
            SmtpHandler,
            SmtpTransaction,
        },
    },
    mail::user::UserStore,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{
        Context,
        Poll,
    },
};

use base64;
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
        ReadBuf,
    },
    net::TcpStream,
};
use tokio_rustls::{
    TlsAcceptor,
    server::TlsStream,
};


/// Maximum number of bytes the server is willing to read inside the
/// `DATA` phase of a single transaction. Mirrors Postfix's
/// `message_size_limit = 20480000` default.
pub const SMTP_MESSAGE_SIZE_LIMIT: usize = 20_480_000;

/// Maximum number of recipients accepted in one transaction.
pub const SMTP_MAX_RCPT: usize = 100;

/// Per-line read budget used when reading wire commands. SMTP lines
/// must not exceed 1000 bytes including CRLF (RFC 5321 §4.5.3.1.6),
/// 512 for command lines; we add some slack for safety.
pub const SMTP_MAX_LINE: usize = 4_096;


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MAYBE TLS                                                                 │
// │                                                                           │
// │ Enum-based stream that is either a plain TcpStream or a TlsStream<TcpStream│
// │ after a successful STARTTLS upgrade. Implements AsyncRead/AsyncWrite by    │
// │ delegating, with explicit pin projection on each variant.                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Either a plain TCP stream or a TLS-wrapped TCP stream.
///
/// Used so the SMTP and IMAP session loops can be written once over a
/// single `S: AsyncRead + AsyncWrite` and dynamically swap in a TLS
/// upgrade in response to STARTTLS.
pub enum MaybeTls {
    /// Plain TCP, used before STARTTLS or when TLS is not negotiated.
    Plain(TcpStream),
    /// TLS-wrapped TCP, used after STARTTLS or on implicit-TLS ports.
    Tls(Box<TlsStream<TcpStream>>),
}

impl MaybeTls {
    /// Returns `true` when the underlying transport is TLS-wrapped.
    pub fn is_tls(&self) -> bool {
        matches!(self, MaybeTls::Tls(_))
    }

    /// Consume the wrapper and return the inner plain stream, if any.
    /// Returns `None` if the connection has already been upgraded to
    /// TLS, which is what we want for STARTTLS handling.
    pub fn into_plain(self) -> Option<TcpStream> {
        match self {
            MaybeTls::Plain(s) => Some(s),
            MaybeTls::Tls(_)   => None,
        }
    }
}

impl AsyncRead for MaybeTls {
    fn poll_read(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
        buf:    &mut ReadBuf<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        // Safe pin projection by hand: we never move the inner stream
        // ourselves, and Pin::new is sound for both variants because
        // TcpStream and TlsStream are Unpin.
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeTls::Tls(s)   => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTls {
    fn poll_write(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
        buf:    &[u8],
    )
        -> Poll<std::io::Result<usize>>
    {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeTls::Tls(s)   => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_flush(cx),
            MaybeTls::Tls(s)   => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self:   Pin<&mut Self>,
        cx:     &mut Context<'_>,
    )
        -> Poll<std::io::Result<()>>
    {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeTls::Tls(s)   => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SMTP SERVER MODE                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

/// Which port and policy this listener is serving.
///
/// Receive listens on 25 and accepts only mail bound for local users,
/// without authentication. Submission listens on 587, requires STARTTLS
/// before AUTH, and lets authenticated users relay to anywhere.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmtpMode {
    /// MX inbound (port 25). No `AUTH`, recipients must resolve locally.
    Receive,
    /// MSA submission (port 587). `AUTH` required, recipients are not
    /// constrained to local mailboxes.
    Submission,
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SESSION STATE                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// Where in the SMTP transaction we currently are.
#[derive(Clone, Debug, Eq, PartialEq)]
enum SmtpPhase {
    /// Just connected; expecting HELO or EHLO.
    Greeted,
    /// EHLO received; ready for `MAIL FROM` (or other extensions).
    Ehlo,
    /// `MAIL FROM` received; expecting `RCPT TO`.
    MailFrom,
    /// At least one `RCPT TO` received; expecting more or `DATA`.
    Rcpt,
}

/// Per-connection mutable state. Reset by `RSET` and after a successful
/// `DATA` transaction.
struct SmtpSession {
    phase:          SmtpPhase,
    helo_domain:    String,
    mail_from:      String,
    rcpt_to:        Vec<String>,
    auth_user:      Option<crate::mail::store::MailUser>,
}

impl SmtpSession {
    fn new() -> Self {
        Self {
            phase:          SmtpPhase::Greeted,
            helo_domain:    String::new(),
            mail_from:      String::new(),
            rcpt_to:        Vec::new(),
            auth_user:      None,
        }
    }

    /// Reset transaction-scoped state (envelope), keeping the HELO
    /// domain and authenticated identity.
    fn reset_transaction(&mut self) {
        if !self.helo_domain.is_empty() {
            self.phase = SmtpPhase::Ehlo;
        } else {
            self.phase = SmtpPhase::Greeted;
        }
        self.mail_from.clear();
        self.rcpt_to.clear();
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SMTP SERVER                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// One SMTP listener configuration.
///
/// Cheaply cloneable -- the inner state (handler, user store, TLS
/// acceptor, hostname) is wrapped in `Arc`s by the caller so a single
/// listener can fan out across every accept loop.
#[derive(Clone)]
pub struct SmtpServer<H: SmtpHandler, U: UserStore> {
    /// Application hook for completed transactions.
    pub handler:        H,
    /// User store used by the AUTH path.
    pub users:          U,
    /// TLS acceptor for STARTTLS upgrades. `None` disables STARTTLS;
    /// submission listeners must always have one.
    pub tls_acceptor:   Option<TlsAcceptor>,
    /// Hostname to advertise in the 220 banner and the `Received:`
    /// header. Should be the public MX hostname.
    pub hostname:       Arc<String>,
    /// Receive vs submission policy.
    pub mode:           SmtpMode,
}

impl<H: SmtpHandler, U: UserStore> SmtpServer<H, U> {

    /// Drive one accepted TCP connection through a complete SMTP
    /// session, including any in-session STARTTLS upgrade.
    pub async fn run(
        &self,
        plain:  TcpStream,
        peer:   SocketAddr,
    )
        -> Outcome<()>
    {
        let mut stream = MaybeTls::Plain(plain);
        let mut session = SmtpSession::new();

        // 220 banner.
        let banner = fmt!(
            "{} ESMTP Hematite Steel ready",
            self.hostname,
        );
        res!(write_response(&mut stream, SmtpResponseCode::ServiceReady, &banner).await);

        // Inner loop. Returns when the client quits, errors out, or
        // requests a STARTTLS upgrade. STARTTLS triggers a re-entry
        // with a TLS-wrapped stream.
        loop {
            let next = res!(self.run_loop(&mut stream, &mut session, peer).await);
            match next {
                LoopExit::Quit => break,
                LoopExit::StartTls => {
                    // Pull the plain stream back out and run the TLS
                    // handshake. The acceptor must be configured -- if
                    // it is not, run_loop refused the STARTTLS request.
                    let acceptor = match self.tls_acceptor.clone() {
                        Some(a) => a,
                        None => return Err(err!(
                            "STARTTLS requested but no TLS acceptor configured.";
                            Init, Missing)),
                    };
                    let plain = match stream.into_plain() {
                        Some(s) => s,
                        None => return Err(err!(
                            "STARTTLS requested on a stream that was already TLS.";
                            Invalid, Bug)),
                    };
                    let tls = match acceptor.accept(plain).await {
                        Ok(t) => t,
                        Err(e) => return Err(err!(e,
                            "STARTTLS handshake failed for {:?}.", peer;
                            IO, Network, Init)),
                    };
                    stream = MaybeTls::Tls(Box::new(tls));
                    // RFC 3207 §4.2: a successful STARTTLS resets the
                    // session to immediately after the greeting.
                    session = SmtpSession::new();
                    // No new banner -- the greeting was sent before TLS.
                }
            }
        }

        // Best-effort shutdown.
        let _ = stream.shutdown().await;
        Ok(())
    }

    async fn run_loop(
        &self,
        stream:     &mut MaybeTls,
        session:    &mut SmtpSession,
        peer:       SocketAddr,
    )
        -> Outcome<LoopExit>
    {
        loop {
            let line = match read_line(stream).await {
                Ok(Some(l)) => l,
                Ok(None) => return Ok(LoopExit::Quit),
                Err(e) => return Err(err!(e,
                    "Reading SMTP command line from {:?}.", peer;
                    IO, Network, Read)),
            };

            // Empty lines are tolerated.
            if line.trim().is_empty() {
                continue;
            }

            let cmd = match SmtpCommand::from_str(&line) {
                Ok(c) => c,
                Err(e) => {
                    warn!("SMTP {:?} {}: unparseable command {:?}: {}",
                        self.mode, peer, line, e);
                    res!(write_response(
                        stream,
                        SmtpResponseCode::CommandUnrecognized,
                        "Command not recognised",
                    ).await);
                    continue;
                }
            };

            match cmd {
                SmtpCommand::Helo(domain) => {
                    session.helo_domain = domain.as_str().to_string();
                    session.phase = SmtpPhase::Ehlo;
                    res!(write_response(
                        stream,
                        SmtpResponseCode::RequestedMailActionOkayCompleted,
                        &fmt!("{} Hello {}", self.hostname, domain.as_str()),
                    ).await);
                }
                SmtpCommand::Ehlo(domain) => {
                    session.helo_domain = domain.as_str().to_string();
                    session.phase = SmtpPhase::Ehlo;
                    res!(self.write_ehlo(stream, domain.as_str()).await);
                }
                SmtpCommand::StartTls => {
                    if stream.is_tls() {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::BadSequenceOfCommands,
                            "STARTTLS not allowed inside TLS",
                        ).await);
                        continue;
                    }
                    if self.tls_acceptor.is_none() {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::CommandNotImplemented,
                            "STARTTLS not available",
                        ).await);
                        continue;
                    }
                    res!(write_response(
                        stream,
                        SmtpResponseCode::ServiceReady,
                        "Ready to start TLS",
                    ).await);
                    return Ok(LoopExit::StartTls);
                }
                SmtpCommand::Auth(arg) => {
                    if self.mode == SmtpMode::Receive {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::CommandNotImplemented,
                            "AUTH not available on this port",
                        ).await);
                        continue;
                    }
                    if !stream.is_tls() {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::EncryptionRequiredForAuthentication,
                            "Must issue STARTTLS before AUTH",
                        ).await);
                        continue;
                    }
                    match self.handle_auth(stream, &arg).await {
                        Ok(Some(user)) => {
                            session.auth_user = Some(user);
                            res!(write_response(
                                stream,
                                SmtpResponseCode::AuthenticationSuccessful,
                                "Authentication successful",
                            ).await);
                        }
                        Ok(None) => {
                            res!(write_response(
                                stream,
                                SmtpResponseCode::AuthenticationCredentialsInvalid,
                                "Authentication credentials invalid",
                            ).await);
                        }
                        Err(e) => {
                            warn!("SMTP AUTH transport error from {:?}: {}", peer, e);
                            res!(write_response(
                                stream,
                                SmtpResponseCode::TransactionFailed,
                                "Authentication aborted",
                            ).await);
                        }
                    }
                }
                SmtpCommand::MailFrom(addr) => {
                    if self.mode == SmtpMode::Submission && session.auth_user.is_none() {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::AuthenticationRequired,
                            "Authentication required",
                        ).await);
                        continue;
                    }
                    if session.phase != SmtpPhase::Ehlo {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::BadSequenceOfCommands,
                            "Bad command sequence",
                        ).await);
                        continue;
                    }
                    session.mail_from = addr;
                    session.phase = SmtpPhase::MailFrom;
                    res!(write_response(
                        stream,
                        SmtpResponseCode::RequestedMailActionOkayCompleted,
                        "Sender OK",
                    ).await);
                }
                SmtpCommand::RcptTo(addr) => {
                    if !matches!(session.phase, SmtpPhase::MailFrom | SmtpPhase::Rcpt) {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::BadSequenceOfCommands,
                            "MAIL FROM required first",
                        ).await);
                        continue;
                    }
                    if session.rcpt_to.len() >= SMTP_MAX_RCPT {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::ExceededStorageAllocation,
                            "Too many recipients",
                        ).await);
                        continue;
                    }
                    if self.mode == SmtpMode::Receive
                        && !self.handler.rcpt_acceptable(&addr)
                    {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::MailboxUnavailableOrAccessDenied,
                            "No such user",
                        ).await);
                        continue;
                    }
                    session.rcpt_to.push(addr);
                    session.phase = SmtpPhase::Rcpt;
                    res!(write_response(
                        stream,
                        SmtpResponseCode::RequestedMailActionOkayCompleted,
                        "Recipient OK",
                    ).await);
                }
                SmtpCommand::Data => {
                    if session.phase != SmtpPhase::Rcpt {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::BadSequenceOfCommands,
                            "Need MAIL FROM and RCPT TO first",
                        ).await);
                        continue;
                    }
                    res!(write_response(
                        stream,
                        SmtpResponseCode::StartMailInput,
                        "End data with <CR><LF>.<CR><LF>",
                    ).await);
                    let body = match read_data(stream).await {
                        Ok(b) => b,
                        Err(e) => {
                            return Err(err!(e,
                                "Reading SMTP DATA body from {:?}.", peer;
                                IO, Network, Read));
                        }
                    };
                    let txn = SmtpTransaction {
                        mail_from:      session.mail_from.clone(),
                        rcpt_to:        session.rcpt_to.clone(),
                        helo_domain:    session.helo_domain.clone(),
                        auth_user:      session.auth_user.clone(),
                        peer,
                        tls:            stream.is_tls(),
                        raw_message:    body,
                    };
                    let outcome = match self.mode {
                        SmtpMode::Receive    => self.handler.deliver_inbound(txn),
                        SmtpMode::Submission => self.handler.submit_outbound(txn),
                    };
                    match outcome {
                        Ok(HandlerOutcome::Accepted(qid)) => {
                            res!(write_response(
                                stream,
                                SmtpResponseCode::RequestedMailActionOkayCompleted,
                                &fmt!("OK queued as {}", qid),
                            ).await);
                        }
                        Ok(HandlerOutcome::RejectPermanent(reason)) => {
                            res!(write_response(
                                stream,
                                SmtpResponseCode::TransactionFailed,
                                &reason,
                            ).await);
                        }
                        Ok(HandlerOutcome::RejectTemporary(reason)) => {
                            res!(write_response(
                                stream,
                                SmtpResponseCode::LocalErrorInProcessing,
                                &reason,
                            ).await);
                        }
                        Err(e) => {
                            error!(err!(e,
                                "SMTP handler error from {:?}.", peer;
                                IO));
                            res!(write_response(
                                stream,
                                SmtpResponseCode::LocalErrorInProcessing,
                                "Local error processing message",
                            ).await);
                        }
                    }
                    session.reset_transaction();
                }
                SmtpCommand::Rset => {
                    session.reset_transaction();
                    res!(write_response(
                        stream,
                        SmtpResponseCode::RequestedMailActionOkayCompleted,
                        "Reset OK",
                    ).await);
                }
                SmtpCommand::Noop => {
                    res!(write_response(
                        stream,
                        SmtpResponseCode::RequestedMailActionOkayCompleted,
                        "OK",
                    ).await);
                }
                SmtpCommand::Vrfy(_) => {
                    res!(write_response(
                        stream,
                        SmtpResponseCode::CannotVerifyUserButWillAttemptDelivery,
                        "Cannot VRFY user, but will attempt delivery",
                    ).await);
                }
                SmtpCommand::Expn(_) => {
                    res!(write_response(
                        stream,
                        SmtpResponseCode::CommandNotImplemented,
                        "EXPN not supported",
                    ).await);
                }
                SmtpCommand::Help(_) => {
                    res!(write_response(
                        stream,
                        SmtpResponseCode::HelpMessage,
                        "HELO EHLO MAIL RCPT DATA RSET NOOP QUIT STARTTLS AUTH",
                    ).await);
                }
                SmtpCommand::Quit => {
                    res!(write_response(
                        stream,
                        SmtpResponseCode::ServiceClosingTransmissionChannel,
                        &fmt!("{} closing connection", self.hostname),
                    ).await);
                    return Ok(LoopExit::Quit);
                }
                _ => {
                    res!(write_response(
                        stream,
                        SmtpResponseCode::CommandNotImplemented,
                        "Not implemented",
                    ).await);
                }
            }
        }
    }

    async fn write_ehlo(
        &self,
        stream: &mut MaybeTls,
        peer:   &str,
    )
        -> Outcome<()>
    {
        // Build the multi-line capability list. Lines other than the
        // last use `250-`, the last uses `250 `.
        let mut lines: Vec<String> = Vec::new();
        lines.push(fmt!("{} Hello {}", self.hostname, peer));
        lines.push(fmt!("PIPELINING"));
        lines.push(fmt!("8BITMIME"));
        lines.push(fmt!("SIZE {}", SMTP_MESSAGE_SIZE_LIMIT));
        lines.push(fmt!("ENHANCEDSTATUSCODES"));
        if !stream.is_tls() && self.tls_acceptor.is_some() {
            lines.push(fmt!("STARTTLS"));
        }
        if self.mode == SmtpMode::Submission && stream.is_tls() {
            lines.push(fmt!("AUTH PLAIN LOGIN"));
        }
        lines.push(fmt!("HELP"));

        let last = lines.len() - 1;
        for (i, line) in lines.iter().enumerate() {
            let sep = if i == last { ' ' } else { '-' };
            let frame = fmt!("250{}{}\r\n", sep, line);
            if let Err(e) = stream.write_all(frame.as_bytes()).await {
                return Err(err!(e, "Writing EHLO line."; IO, Network, Write));
            }
        }
        if let Err(e) = stream.flush().await {
            return Err(err!(e, "Flushing EHLO."; IO, Network, Write));
        }
        Ok(())
    }

    async fn handle_auth(
        &self,
        stream: &mut MaybeTls,
        arg:    &str,
    )
        -> Outcome<Option<crate::mail::store::MailUser>>
    {
        let mut parts = arg.splitn(2, char::is_whitespace);
        let mech = match parts.next() {
            Some(m) => m.to_uppercase(),
            None => return Ok(None),
        };
        let initial = parts.next().map(|s| s.trim().to_string());

        match mech.as_str() {
            "PLAIN" => {
                // RFC 4616: base64( authzid \0 authcid \0 passwd ).
                let payload = match initial {
                    Some(p) if !p.is_empty() => p,
                    _ => {
                        res!(write_response(
                            stream,
                            SmtpResponseCode::AuthInputData,
                            "",
                        ).await);
                        match res!(read_line(stream).await) {
                            Some(l) => l.trim().to_string(),
                            None => return Ok(None),
                        }
                    }
                };
                let raw = match base64::decode(payload.as_bytes()) {
                    Ok(b) => b,
                    Err(_) => return Ok(None),
                };
                let (user, pass) = match parse_plain(&raw) {
                    Some(p) => p,
                    None => return Ok(None),
                };
                self.users.authenticate(&user, &pass)
            }
            "LOGIN" => {
                // RFC ietf-sasl-login: server prompts for username then
                // password, both base64. The challenges may or may not
                // be expected by the client; safest is to send the
                // standard "Username:" / "Password:" prompts.
                res!(write_response(
                    stream,
                    SmtpResponseCode::AuthInputData,
                    &base64::encode(b"Username:"),
                ).await);
                let user_b64 = match res!(read_line(stream).await) {
                    Some(l) => l.trim().to_string(),
                    None => return Ok(None),
                };
                let user_bytes = match base64::decode(user_b64.as_bytes()) {
                    Ok(b) => b,
                    Err(_) => return Ok(None),
                };
                res!(write_response(
                    stream,
                    SmtpResponseCode::AuthInputData,
                    &base64::encode(b"Password:"),
                ).await);
                let pass_b64 = match res!(read_line(stream).await) {
                    Some(l) => l.trim().to_string(),
                    None => return Ok(None),
                };
                let pass_bytes = match base64::decode(pass_b64.as_bytes()) {
                    Ok(b) => b,
                    Err(_) => return Ok(None),
                };
                let user = String::from_utf8_lossy(&user_bytes).to_string();
                let pass = String::from_utf8_lossy(&pass_bytes).to_string();
                self.users.authenticate(&user, &pass)
            }
            _ => {
                Ok(None)
            }
        }
    }
}


/// Parsed result of the inner read loop: the client either disconnected
/// or asked us to upgrade to TLS.
#[derive(Clone, Copy, Debug)]
enum LoopExit {
    /// The client sent QUIT or the connection was closed cleanly.
    Quit,
    /// STARTTLS was issued; the caller must perform a TLS handshake on
    /// the underlying TCP stream and re-enter the read loop.
    StartTls,
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ WIRE HELPERS                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Write one single-line SMTP response as `NNN<space>text<CRLF>`.
pub async fn write_response<S: AsyncWrite + Unpin>(
    stream:     &mut S,
    code:       SmtpResponseCode,
    text:       &str,
)
    -> Outcome<()>
{
    let line = fmt!("{} {}\r\n", code, text);
    if let Err(e) = stream.write_all(line.as_bytes()).await {
        return Err(err!(e, "Writing SMTP response."; IO, Network, Write));
    }
    if let Err(e) = stream.flush().await {
        return Err(err!(e, "Flushing SMTP response."; IO, Network, Write));
    }
    Ok(())
}

/// Read one CRLF-terminated line, returning `None` on a clean EOF and
/// trimming the trailing `\r\n` (or bare `\n`). Caps the line at
/// `SMTP_MAX_LINE` bytes.
pub async fn read_line<S: AsyncRead + Unpin>(
    stream: &mut S,
)
    -> Outcome<Option<String>>
{
    let mut buf = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    loop {
        let n = match stream.read(&mut byte).await {
            Ok(n) => n,
            Err(e) => return Err(err!(e, "Reading SMTP line byte."; IO, Network, Read)),
        };
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            break;
        }
        buf.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
        if buf.len() >= SMTP_MAX_LINE {
            return Err(err!(
                "SMTP line exceeded {} bytes.", SMTP_MAX_LINE;
                Invalid, Input, Excessive));
        }
    }
    // Trim trailing CRLF / LF.
    while buf.last() == Some(&b'\n') || buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// Read the body of a `DATA` command: every byte up to and excluding
/// the terminating `<CRLF>.<CRLF>`, with dot-unstuffing applied (any
/// line whose first character is `.` has that dot stripped, RFC 5321
/// §4.5.2). CR/LF are preserved as `\r\n` so downstream consumers see
/// canonical RFC 5322 line endings.
pub async fn read_data<S: AsyncRead + Unpin>(
    stream: &mut S,
)
    -> Outcome<Vec<u8>>
{
    let mut out = Vec::with_capacity(4096);
    let mut at_line_start = true;
    let mut just_saw_cr = false;
    let mut byte = [0u8; 1];

    loop {
        let n = match stream.read(&mut byte).await {
            Ok(n) => n,
            Err(e) => return Err(err!(e,
                "Reading SMTP DATA byte."; IO, Network, Read)),
        };
        if n == 0 {
            return Err(err!(
                "Connection closed mid-DATA.";
                IO, Network, Read, Missing));
        }
        let b = byte[0];

        // Detect the dot-stuff terminator: a line consisting solely of
        // `.` ends the DATA. We track at_line_start to identify the
        // first byte after a CRLF.
        if at_line_start && b == b'.' {
            // Peek ahead: read the next byte. If it is CR (followed by
            // LF) the dot terminates the message; otherwise it is a
            // dot-stuffed body line and we strip the leading dot.
            let mut peek = [0u8; 1];
            let m = match stream.read(&mut peek).await {
                Ok(n) => n,
                Err(e) => return Err(err!(e,
                    "Reading SMTP DATA dot peek."; IO, Network, Read)),
            };
            if m == 0 {
                return Err(err!(
                    "Connection closed mid-DATA after lone dot.";
                    IO, Network, Read, Missing));
            }
            if peek[0] == b'\r' {
                // Expect LF next.
                let mut tail = [0u8; 1];
                let k = match stream.read(&mut tail).await {
                    Ok(n) => n,
                    Err(e) => return Err(err!(e,
                        "Reading SMTP DATA terminator LF."; IO, Network, Read)),
                };
                if k == 0 || tail[0] != b'\n' {
                    return Err(err!(
                        "Malformed DATA terminator (.<CR> without <LF>).";
                        Invalid, Input));
                }
                return Ok(out);
            }
            // Not the terminator -- it was a dot-stuffed line. The
            // leading dot has been consumed and discarded; the peeked
            // byte is the actual first body byte of the line.
            out.push(peek[0]);
            at_line_start = peek[0] == b'\n';
            just_saw_cr   = peek[0] == b'\r';
            if out.len() > SMTP_MESSAGE_SIZE_LIMIT {
                return Err(err!(
                    "SMTP DATA exceeded size limit of {} bytes.",
                    SMTP_MESSAGE_SIZE_LIMIT;
                    Invalid, Input, Excessive));
            }
            continue;
        }

        out.push(b);
        if out.len() > SMTP_MESSAGE_SIZE_LIMIT {
            return Err(err!(
                "SMTP DATA exceeded size limit of {} bytes.",
                SMTP_MESSAGE_SIZE_LIMIT;
                Invalid, Input, Excessive));
        }

        // Update line-start tracking. A new line starts after CRLF or
        // a bare LF.
        if just_saw_cr && b == b'\n' {
            at_line_start = true;
            just_saw_cr = false;
        } else if b == b'\n' {
            at_line_start = true;
            just_saw_cr = false;
        } else if b == b'\r' {
            just_saw_cr = true;
            at_line_start = false;
        } else {
            at_line_start = false;
            just_saw_cr = false;
        }
    }
}

/// Parse the SASL PLAIN payload (`authzid \0 authcid \0 passwd`) into
/// `(authcid, passwd)`. `authzid` is ignored.
fn parse_plain(raw: &[u8]) -> Option<(String, String)> {
    let mut nuls = raw.iter().enumerate().filter_map(|(i, b)| {
        if *b == 0u8 { Some(i) } else { None }
    });
    let first  = nuls.next()?;
    let second = nuls.next()?;
    let user = String::from_utf8(raw[first + 1..second].to_vec()).ok()?;
    let pass = String::from_utf8(raw[second + 1..].to_vec()).ok()?;
    Some((user, pass))
}
