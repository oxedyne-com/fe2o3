//! IMAP4rev1 session loop.
//!
//! Drives one TLS-wrapped TCP connection through the IMAP command set
//! defined in [`crate::imap`]. The session is parameterised over a
//! `MailStore` and `UserStore` so the same loop serves any Hematite
//! mailbox backend.

use crate::mail::{
    store::{
        FolderName,
        MailStore,
        MailUser,
        MessageFlags,
        MessageMeta,
    },
    user::UserStore,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        AsyncWrite,
        AsyncWriteExt,
    },
};


/// How often an idling session looks at the mailbox.
///
/// A Maildir folder status is a directory listing, so this is cheap; the cost
/// is one stat per idle connection per interval, against a client that would
/// otherwise poll every ten minutes and be told nothing.
pub const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How long the server idles before asking the client to renew.
///
/// RFC 2177 tells clients to re-issue `IDLE` at least every 29 minutes.
/// Ending it a little earlier means a connection a NAT was about to drop
/// silently is refreshed deliberately instead.
pub const IDLE_MAX_DURATION: Duration = Duration::from_secs(28 * 60);

/// Longest partial line the server will buffer from an idling client. Only
/// `DONE` is legal, so anything approaching this is not a client to indulge.
pub const IDLE_MAX_LINE: usize = 1024;

/// Maximum line length the server will accept for a command. IMAP
/// itself imposes no formal limit, but capping protects us from a
/// runaway client.
pub const IMAP_MAX_LINE: usize = 8 * 1024;

/// Maximum literal size accepted on `APPEND` (mirrors the SMTP
/// message size limit).
pub const IMAP_MAX_LITERAL: usize = 20_480_000;


/// One IMAP listener configuration.
///
/// Cheaply cloneable -- inner state (handler, user store, hostname,
/// folder list) is wrapped in `Arc`s so a single listener fan-outs
/// across every accept loop without contention.
///
/// The server is generic over its transport: it is handed an established
/// stream and speaks IMAP over it, whether that is a TLS stream from the
/// listener, a plain socket on loopback, or an in-memory pipe in a test.
#[derive(Clone)]
pub struct ImapServer<M: MailStore, U: UserStore> {
    /// Mailbox storage backend.
    pub store:      M,
    /// User authentication backend.
    pub users:      U,
    /// Hostname to advertise in the greeting and BYE messages.
    pub hostname:   Arc<String>,
}

/// Per-connection IMAP session state.
struct ImapSession {
    /// Authenticated user, populated on successful LOGIN.
    user:           Option<MailUser>,
    /// Currently SELECTed folder, if any.
    selected:       Option<FolderName>,
    /// `true` if the current selection is read-only (EXAMINE).
    read_only:      bool,
    /// Cached message metadata for the selected folder, in UID order.
    /// Refreshed on SELECT / EXAMINE / NOOP and after STORE / EXPUNGE.
    messages:       Vec<MessageMeta>,
}

impl ImapSession {
    fn new() -> Self {
        Self {
            user:       None,
            selected:   None,
            read_only:  false,
            messages:   Vec::new(),
        }
    }
}


impl<M: MailStore, U: UserStore> ImapServer<M, U> {

    /// Drive one IMAP session to completion over an established TLS
    /// stream. Returns when the client logs out or the connection
    /// drops.
    pub async fn run<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        mut stream: S,
        peer:       SocketAddr,
    )
        -> Outcome<()>
    {
        let mut session = ImapSession::new();

        // Greeting.
        let greet = fmt!(
            "* OK [CAPABILITY {}] {} Hematite Steel IMAP ready\r\n",
            capability_list(),
            self.hostname,
        );
        if let Err(e) = stream.write_all(greet.as_bytes()).await {
            return Err(err!(e, "Writing IMAP greeting."; IO, Network, Write));
        }
        if let Err(e) = stream.flush().await {
            return Err(err!(e, "Flushing IMAP greeting."; IO, Network, Write));
        }

        loop {
            let line = match res!(read_line(&mut stream).await) {
                Some(l) => l,
                None => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed = match parse_command(&line) {
                Some(p) => p,
                None => {
                    let bad = fmt!("* BAD Cannot parse command\r\n");
                    let _ = stream.write_all(bad.as_bytes()).await;
                    continue;
                }
            };
            match self.dispatch(&mut stream, &mut session, parsed, peer).await {
                Ok(true) => break,
                Ok(false) => continue,
                Err(e) => {
                    error!(err!(e,
                        "IMAP dispatch failure for {:?}.", peer;
                        IO));
                    break;
                }
            }
        }

        let _ = stream.shutdown().await;
        Ok(())
    }

    /// Dispatch one parsed command. Returns `Ok(true)` to terminate
    /// the session (LOGOUT or fatal protocol error).
    async fn dispatch<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        session:    &mut ImapSession,
        parsed:     ParsedCommand,
        peer:       SocketAddr,
    )
        -> Outcome<bool>
    {
        let tag = parsed.tag.clone();
        let cmd = parsed.command.to_uppercase();
        let args = parsed.args.clone();

        // Trace every command so we can debug client behaviour.
        // LOGIN argument is suppressed so the password never reaches
        // the journal in cleartext -- the second arg of LOGIN is the
        // password, the first is the username.
        let log_args = if cmd == "LOGIN" {
            "<redacted>".to_string()
        } else {
            args.clone()
        };
        info!("IMAP {} {} {} {}", peer, tag, cmd, log_args);

        match cmd.as_str() {
            "CAPABILITY" => {
                let line = fmt!("* CAPABILITY {}\r\n", capability_list());
                res!(write_all(stream, line.as_bytes()).await);
                res!(write_ok(stream, &tag, "CAPABILITY completed").await);
                Ok(false)
            }
            "NOOP" => {
                if session.user.is_some() && session.selected.is_some() {
                    res!(self.refresh_selected(session, false).await);
                    res!(self.write_select_status(stream, session).await);
                }
                res!(write_ok(stream, &tag, "NOOP completed").await);
                Ok(false)
            }
            "IDLE" => {
                if session.user.is_none() || session.selected.is_none() {
                    res!(write_bad(stream, &tag,
                        "IDLE requires an authenticated session with a \
                        selected mailbox").await);
                    return Ok(false);
                }
                self.idle(stream, session, &tag).await
            }
            "LOGOUT" => {
                res!(write_all(stream, b"* BYE Logging out\r\n").await);
                res!(write_ok(stream, &tag, "LOGOUT completed").await);
                Ok(true)
            }
            "LOGIN" => {
                let mut it = ArgIter::new(&args);
                let user = match it.next_string() {
                    Some(s) => s,
                    None => {
                        res!(write_bad(stream, &tag, "LOGIN requires user").await);
                        return Ok(false);
                    }
                };
                let pass = match it.next_string() {
                    Some(s) => s,
                    None => {
                        res!(write_bad(stream, &tag, "LOGIN requires password").await);
                        return Ok(false);
                    }
                };
                let result = self.users.authenticate(&user, &pass);
                let mu = match result {
                    Ok(Some(u)) => u,
                    Ok(None) => {
                        res!(write_no(stream, &tag, "LOGIN failed").await);
                        return Ok(false);
                    }
                    Err(e) => {
                        warn!("LOGIN backend error: {}", e);
                        res!(write_no(stream, &tag, "LOGIN failed").await);
                        return Ok(false);
                    }
                };
                if let Err(e) = self.store.ensure_user(&mu) {
                    warn!("ensure_user error for {}: {}", mu.address(), e);
                }
                session.user = Some(mu);
                res!(write_ok(stream, &tag, "LOGIN completed").await);
                Ok(false)
            }
            "AUTHENTICATE" => {
                // Not implemented in MVP -- LOGIN-only.
                res!(write_no(stream, &tag, "AUTHENTICATE not supported, use LOGIN").await);
                Ok(false)
            }
            "SELECT" | "EXAMINE" => {
                if session.user.is_none() {
                    res!(write_no(stream, &tag, "Authenticate first").await);
                    return Ok(false);
                }
                let mut it = ArgIter::new(&args);
                let folder = match it.next_string() {
                    Some(s) => FolderName::new(s),
                    None => {
                        res!(write_bad(stream, &tag, "SELECT requires folder").await);
                        return Ok(false);
                    }
                };
                session.selected = Some(folder);
                session.read_only = cmd == "EXAMINE";
                res!(self.refresh_selected(session, true).await);
                res!(self.write_select_status(stream, session).await);
                let suffix = if session.read_only { "[READ-ONLY]" } else { "[READ-WRITE]" };
                res!(write_ok(stream, &tag, &fmt!("{} {} completed", suffix, cmd)).await);
                Ok(false)
            }
            "CLOSE" => {
                // RFC 3501 §6.4.2: if not read-only, expunge first.
                if let (Some(user), Some(folder)) = (session.user.clone(), session.selected.clone()) {
                    if !session.read_only {
                        let _ = self.store.expunge(&user, &folder);
                    }
                }
                session.selected = None;
                session.read_only = false;
                session.messages.clear();
                res!(write_ok(stream, &tag, "CLOSE completed").await);
                Ok(false)
            }
            "LIST" | "LSUB" => {
                let user = match &session.user {
                    Some(u) => u.clone(),
                    None => {
                        res!(write_no(stream, &tag, "Authenticate first").await);
                        return Ok(false);
                    }
                };
                let mut it = ArgIter::new(&args);
                let _refname = it.next_string().unwrap_or_default();
                let pattern = it.next_string().unwrap_or_default();
                let folders = if cmd == "LSUB" {
                    res!(self.store.list_subscribed(&user))
                } else {
                    res!(self.store.list_folders(&user))
                };
                for f in &folders {
                    if !match_imap_pattern(&pattern, f.as_str()) {
                        continue;
                    }
                    let attrs = special_use_attrs(f.as_str());
                    let line = fmt!("* {} ({}) \"/\" \"{}\"\r\n",
                        cmd, attrs, escape_quoted(f.as_str()));
                    res!(write_all(stream, line.as_bytes()).await);
                }
                res!(write_ok(stream, &tag, &fmt!("{} completed", cmd)).await);
                Ok(false)
            }
            "SUBSCRIBE" => {
                let user = match &session.user {
                    Some(u) => u.clone(),
                    None => {
                        res!(write_no(stream, &tag, "Authenticate first").await);
                        return Ok(false);
                    }
                };
                let mut it = ArgIter::new(&args);
                let folder = match it.next_string() {
                    Some(s) => FolderName::new(s),
                    None => {
                        res!(write_bad(stream, &tag, "SUBSCRIBE requires folder").await);
                        return Ok(false);
                    }
                };
                let _ = self.store.subscribe(&user, &folder);
                res!(write_ok(stream, &tag, "SUBSCRIBE completed").await);
                Ok(false)
            }
            "UNSUBSCRIBE" => {
                // No-op: the MVP store keeps the subscription set
                // forever; Thunderbird does not rely on UNSUBSCRIBE.
                res!(write_ok(stream, &tag, "UNSUBSCRIBE completed").await);
                Ok(false)
            }
            "CREATE" => {
                let user = match &session.user {
                    Some(u) => u.clone(),
                    None => {
                        res!(write_no(stream, &tag, "Authenticate first").await);
                        return Ok(false);
                    }
                };
                let mut it = ArgIter::new(&args);
                let folder = match it.next_string() {
                    Some(s) => FolderName::new(s),
                    None => {
                        res!(write_bad(stream, &tag, "CREATE requires folder").await);
                        return Ok(false);
                    }
                };
                let _ = self.store.create_folder(&user, &folder);
                let _ = self.store.subscribe(&user, &folder);
                res!(write_ok(stream, &tag, "CREATE completed").await);
                Ok(false)
            }
            "DELETE" => {
                // Not supported on MVP -- folder deletion is a manual
                // operation on disk.
                res!(write_no(stream, &tag, "DELETE not supported").await);
                Ok(false)
            }
            "STATUS" => {
                let user = match &session.user {
                    Some(u) => u.clone(),
                    None => {
                        res!(write_no(stream, &tag, "Authenticate first").await);
                        return Ok(false);
                    }
                };
                let mut it = ArgIter::new(&args);
                let folder = match it.next_string() {
                    Some(s) => FolderName::new(s),
                    None => {
                        res!(write_bad(stream, &tag, "STATUS requires folder").await);
                        return Ok(false);
                    }
                };
                let items = it.next_paren_list().unwrap_or_default();
                let status = res!(self.store.folder_status(&user, &folder));
                let mut parts: Vec<String> = Vec::new();
                for item in items.split_whitespace() {
                    match item.to_uppercase().as_str() {
                        "MESSAGES"      => parts.push(fmt!("MESSAGES {}", status.exists)),
                        "RECENT"        => parts.push(fmt!("RECENT {}", status.recent)),
                        "UIDNEXT"       => parts.push(fmt!("UIDNEXT {}", status.uid_next)),
                        "UIDVALIDITY"   => parts.push(fmt!("UIDVALIDITY {}", status.uid_validity)),
                        "UNSEEN"        => parts.push(fmt!("UNSEEN {}", status.unseen)),
                        _ => (),
                    }
                }
                let line = fmt!("* STATUS \"{}\" ({})\r\n",
                    escape_quoted(folder.as_str()),
                    parts.join(" "),
                );
                res!(write_all(stream, line.as_bytes()).await);
                res!(write_ok(stream, &tag, "STATUS completed").await);
                Ok(false)
            }
            "FETCH" | "UID" | "STORE" | "SEARCH" | "EXPUNGE" | "APPEND" => {
                // Group the remaining commands together so the
                // borrow checker keeps `session` mutably available.
                self.dispatch_data_command(stream, session, &tag, &cmd, &args).await
            }
            _ => {
                res!(write_bad(stream, &tag, "Unknown command").await);
                Ok(false)
            }
        }
    }

    async fn dispatch_data_command<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        session:    &mut ImapSession,
        tag:        &str,
        cmd:        &str,
        args:       &str,
    )
        -> Outcome<bool>
    {
        if session.user.is_none() {
            res!(write_no(stream, tag, "Authenticate first").await);
            return Ok(false);
        }
        match cmd {
            "FETCH" => {
                if session.selected.is_none() {
                    res!(write_no(stream, tag, "No mailbox selected").await);
                    return Ok(false);
                }
                let mut it = ArgIter::new(args);
                let seq_set = match it.next_atom() {
                    Some(s) => s,
                    None => {
                        res!(write_bad(stream, tag, "FETCH requires seq set").await);
                        return Ok(false);
                    }
                };
                let items = it.rest().to_string();
                res!(self.do_fetch(stream, session, &seq_set, &items, false).await);
                res!(write_ok(stream, tag, "FETCH completed").await);
                Ok(false)
            }
            "STORE" => {
                if session.selected.is_none() {
                    res!(write_no(stream, tag, "No mailbox selected").await);
                    return Ok(false);
                }
                let mut it = ArgIter::new(args);
                let seq_set = match it.next_atom() {
                    Some(s) => s,
                    None => {
                        res!(write_bad(stream, tag, "STORE requires seq set").await);
                        return Ok(false);
                    }
                };
                let op = match it.next_atom() {
                    Some(s) => s,
                    None => {
                        res!(write_bad(stream, tag, "STORE requires op").await);
                        return Ok(false);
                    }
                };
                let flags = it.next_paren_list().unwrap_or_default();
                res!(self.do_store(stream, session, &seq_set, &op, &flags, false).await);
                res!(write_ok(stream, tag, "STORE completed").await);
                Ok(false)
            }
            "SEARCH" => {
                if session.selected.is_none() {
                    res!(write_no(stream, tag, "No mailbox selected").await);
                    return Ok(false);
                }
                res!(self.do_search(stream, session, args, false).await);
                res!(write_ok(stream, tag, "SEARCH completed").await);
                Ok(false)
            }
            "EXPUNGE" => {
                let user = match &session.user {
                    Some(u) => u.clone(),
                    None => {
                        res!(write_no(stream, tag, "Authenticate first").await);
                        return Ok(false);
                    }
                };
                let folder = match &session.selected {
                    Some(f) => f.clone(),
                    None => {
                        res!(write_no(stream, tag, "No mailbox selected").await);
                        return Ok(false);
                    }
                };
                let removed = res!(self.store.expunge(&user, &folder));
                // For each expunged UID we send an untagged EXPUNGE
                // with the *sequence number* (1-based index in the
                // pre-expunge list). We use the cached message list.
                for uid in &removed {
                    if let Some(idx) = session.messages.iter().position(|m| m.uid == *uid) {
                        let line = fmt!("* {} EXPUNGE\r\n", idx + 1);
                        res!(write_all(stream, line.as_bytes()).await);
                        session.messages.remove(idx);
                    }
                }
                res!(write_ok(stream, tag, "EXPUNGE completed").await);
                Ok(false)
            }
            "APPEND" => {
                let user = match &session.user {
                    Some(u) => u.clone(),
                    None => {
                        res!(write_no(stream, tag, "Authenticate first").await);
                        return Ok(false);
                    }
                };
                res!(self.do_append(stream, &user, tag, args).await);
                Ok(false)
            }
            "UID" => {
                // UID FETCH / UID STORE / UID SEARCH / UID COPY.
                let mut it = ArgIter::new(args);
                let sub = match it.next_atom() {
                    Some(s) => s.to_uppercase(),
                    None => {
                        res!(write_bad(stream, tag, "UID requires subcommand").await);
                        return Ok(false);
                    }
                };
                match sub.as_str() {
                    "FETCH" => {
                        let seq_set = match it.next_atom() {
                            Some(s) => s,
                            None => {
                                res!(write_bad(stream, tag, "UID FETCH requires seq set").await);
                                return Ok(false);
                            }
                        };
                        let items = it.rest().to_string();
                        res!(self.do_fetch(stream, session, &seq_set, &items, true).await);
                        res!(write_ok(stream, tag, "UID FETCH completed").await);
                    }
                    "STORE" => {
                        let seq_set = match it.next_atom() {
                            Some(s) => s,
                            None => {
                                res!(write_bad(stream, tag, "UID STORE requires seq set").await);
                                return Ok(false);
                            }
                        };
                        let op = match it.next_atom() {
                            Some(s) => s,
                            None => {
                                res!(write_bad(stream, tag, "UID STORE requires op").await);
                                return Ok(false);
                            }
                        };
                        let flags = it.next_paren_list().unwrap_or_default();
                        res!(self.do_store(stream, session, &seq_set, &op, &flags, true).await);
                        res!(write_ok(stream, tag, "UID STORE completed").await);
                    }
                    "SEARCH" => {
                        res!(self.do_search(stream, session, it.rest(), true).await);
                        res!(write_ok(stream, tag, "UID SEARCH completed").await);
                    }
                    _ => {
                        res!(write_bad(stream, tag, "UID subcommand not supported").await);
                    }
                }
                Ok(false)
            }
            _ => {
                res!(write_bad(stream, tag, "Unknown command").await);
                Ok(false)
            }
        }
    }

    /// Refresh the cached message list from the backing store and
    /// optionally clear the `\Recent` flag (SELECT vs EXAMINE).
    /// RFC 2177 `IDLE`: hold the connection open and tell the client the
    /// moment the mailbox changes, instead of making it ask.
    ///
    /// Without this a client polls -- Thunderbird every ten minutes by
    /// default -- so mail that has already arrived sits unannounced for up to
    /// ten minutes, and the client wakes the server up all day to be told
    /// nothing has happened. IDLE inverts both halves of that bargain.
    ///
    /// The client sends `IDLE`, we answer `+ idling`, and from then until it
    /// sends `DONE` the only thing it may send is `DONE`. We meanwhile watch
    /// the mailbox and push an untagged `EXISTS` when the count moves.
    ///
    /// # Why a timeout around the read, and not `select!`
    ///
    /// The obvious shape -- `select!` between the socket and a ticker -- wants
    /// a mutable borrow of the stream in one branch and another in the handler
    /// of the other, which does not compile, and inviting people to work around
    /// that by reading a byte at a time invites losing bytes: a future dropped
    /// mid-line has already taken those bytes off the socket. `AsyncReadExt::read`
    /// is cancel-safe -- if it is dropped, nothing was consumed -- so a timeout
    /// around it is both correct and simple, and the partial line lives in a
    /// buffer that outlives each attempt.
    async fn idle<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        session:    &mut ImapSession,
        tag:        &str,
    )
        -> Outcome<bool>
    {
        let user = match session.user.clone() {
            Some(u) => u,
            None => return Ok(false),
        };
        let folder = match session.selected.clone() {
            Some(f) => f,
            None => return Ok(false),
        };

        res!(write_all(stream, b"+ idling\r\n").await);

        let mut last_exists = res!(self.store.folder_status(&user, &folder)).exists;
        let mut pending: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 256];
        let mut waited = Duration::from_secs(0);

        loop {
            let read = tokio::time::timeout(
                IDLE_POLL_INTERVAL,
                stream.read(&mut chunk),
            ).await;

            match read {
                // The client hung up. Not an error: closing an idle
                // connection is how clients end a session all the time.
                Ok(Ok(0)) => return Ok(true),
                Ok(Ok(n)) => {
                    pending.extend_from_slice(&chunk[..n]);
                    // Only `DONE` is legal here, so a line that is not `DONE`
                    // ends the IDLE rather than being silently swallowed --
                    // a client that thinks it issued a command and got no
                    // answer will hang for ever.
                    if let Some(i) = pending.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=i).collect();
                        let text = String::from_utf8_lossy(&line)
                            .trim()
                            .to_uppercase();
                        if text == "DONE" {
                            res!(write_ok(stream, tag, "IDLE terminated").await);
                        } else {
                            res!(write_bad(stream, tag,
                                "Only DONE is valid while idling").await);
                        }
                        return Ok(false);
                    }
                    // A line this long is not a client we want to keep
                    // buffering for.
                    if pending.len() > IDLE_MAX_LINE {
                        res!(write_bad(stream, tag,
                            "Line too long while idling").await);
                        return Ok(false);
                    }
                }
                Ok(Err(e)) => return Err(err!(e,
                    "Reading from an idling IMAP client.";
                    IO, Network, Read)),
                // Nothing from the client: look at the mailbox.
                Err(_elapsed) => {
                    waited = waited.saturating_add(IDLE_POLL_INTERVAL);

                    let status = res!(self.store.folder_status(&user, &folder));
                    if status.exists != last_exists {
                        last_exists = status.exists;
                        res!(self.refresh_selected(session, false).await);
                        res!(write_all(stream,
                            fmt!("* {} EXISTS\r\n", status.exists).as_bytes()).await);
                        res!(write_all(stream,
                            fmt!("* {} RECENT\r\n", status.recent).as_bytes()).await);
                    }

                    // RFC 2177 tells clients to re-issue IDLE at least every
                    // 29 minutes, and warns servers to expect stale ones. End
                    // it ourselves a little before that: a well-behaved client
                    // simply idles again, and a NAT that would have dropped
                    // the connection silently never gets the chance.
                    if waited >= IDLE_MAX_DURATION {
                        res!(write_all(stream,
                            b"* OK Idle time expired, please re-issue IDLE\r\n").await);
                        res!(write_ok(stream, tag, "IDLE terminated").await);
                        return Ok(false);
                    }
                }
            }
        }
    }

    async fn refresh_selected(
        &self,
        session:    &mut ImapSession,
        clear_rec:  bool,
    )
        -> Outcome<()>
    {
        let user = match session.user.clone() {
            Some(u) => u,
            None => return Ok(()),
        };
        let folder = match session.selected.clone() {
            Some(f) => f,
            None => return Ok(()),
        };
        let read_only = !clear_rec || session.read_only;
        let messages = res!(self.store.list_messages(&user, &folder, read_only));
        session.messages = messages;
        Ok(())
    }

    /// Send the untagged status responses required after SELECT or
    /// EXAMINE: EXISTS, RECENT, UIDVALIDITY, UIDNEXT, FLAGS,
    /// PERMANENTFLAGS, [UNSEEN N].
    async fn write_select_status<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        session:    &mut ImapSession,
    )
        -> Outcome<()>
    {
        let user = match &session.user {
            Some(u) => u.clone(),
            None => return Ok(()),
        };
        let folder = match &session.selected {
            Some(f) => f.clone(),
            None => return Ok(()),
        };
        let status = res!(self.store.folder_status(&user, &folder));
        let lines = [
            fmt!("* {} EXISTS\r\n", status.exists),
            fmt!("* {} RECENT\r\n", status.recent),
            fmt!("* OK [UIDVALIDITY {}] UIDs valid\r\n", status.uid_validity),
            fmt!("* OK [UIDNEXT {}] Next UID\r\n", status.uid_next),
            fmt!("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n"),
            fmt!("* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)] OK\r\n"),
        ];
        for line in &lines {
            res!(write_all(stream, line.as_bytes()).await);
        }
        // UNSEEN: position of the first unseen message, if any.
        if let Some(idx) = session.messages.iter().position(|m| !m.flags.seen) {
            let line = fmt!("* OK [UNSEEN {}] First unseen\r\n", idx + 1);
            res!(write_all(stream, line.as_bytes()).await);
        }
        Ok(())
    }

    async fn do_fetch<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        session:    &mut ImapSession,
        seq_set:    &str,
        items_raw:  &str,
        by_uid:     bool,
    )
        -> Outcome<()>
    {
        let user = match &session.user {
            Some(u) => u.clone(),
            None => return Ok(()),
        };
        let folder = match &session.selected {
            Some(f) => f.clone(),
            None => return Ok(()),
        };
        res!(self.refresh_selected(session, false).await);

        let items = parse_fetch_items(items_raw);
        let indices = resolve_set(seq_set, &session.messages, by_uid);

        for &idx in &indices {
            let meta = match session.messages.get(idx) {
                Some(m) => m.clone(),
                None => continue,
            };
            let seq_no = idx + 1;

            // Read raw bytes once if any item needs them.
            let mut needs_bytes = false;
            for it in &items {
                match it {
                    FetchItem::Body { .. }      |
                    FetchItem::Rfc822           |
                    FetchItem::Rfc822Header     |
                    FetchItem::Rfc822Text       |
                    FetchItem::Envelope         => needs_bytes = true,
                    _ => (),
                }
            }
            let raw: Option<Vec<u8>> = if needs_bytes {
                Some(res!(self.store.fetch_bytes(&user, &folder, meta.uid)))
            } else {
                None
            };

            let mut implicit_seen = false;

            // Encode each item as a sequence of segments. Text segments
            // may be joined with a separating space; literal segments
            // contain raw byte payloads that must be written
            // unmodified. We collect segments first and decide whether
            // to also append a synthetic FLAGS segment after evaluating
            // implicit \Seen.
            let mut segments: Vec<FetchSegment> = Vec::new();
            for it in &items {
                match it {
                    FetchItem::Uid => {
                        segments.push(FetchSegment::Text(fmt!("UID {}", meta.uid.0)));
                    }
                    FetchItem::Flags => {
                        segments.push(FetchSegment::Text(fmt!(
                            "FLAGS ({})", meta.flags.to_imap_list())));
                    }
                    FetchItem::Rfc822Size => {
                        segments.push(FetchSegment::Text(fmt!(
                            "RFC822.SIZE {}", meta.size)));
                    }
                    FetchItem::InternalDate => {
                        segments.push(FetchSegment::Text(fmt!(
                            "INTERNALDATE \"{}\"",
                            format_internal_date(meta.internal),
                        )));
                    }
                    FetchItem::Envelope => {
                        if let Some(ref bytes) = raw {
                            segments.push(FetchSegment::Text(fmt!(
                                "ENVELOPE {}", build_envelope(bytes))));
                        }
                    }
                    FetchItem::Body { peek, section } => {
                        if !peek { implicit_seen = true; }
                        let bytes = match raw {
                            Some(ref b) => extract_section(b, section),
                            None => Vec::new(),
                        };
                        let label = section_label(section);
                        segments.push(FetchSegment::Literal {
                            prefix: fmt!("BODY[{}] ", label),
                            payload: bytes,
                        });
                    }
                    FetchItem::Rfc822 => {
                        implicit_seen = true;
                        let bytes = match raw {
                            Some(ref b) => b.clone(),
                            None => Vec::new(),
                        };
                        segments.push(FetchSegment::Literal {
                            prefix: "RFC822 ".to_string(),
                            payload: bytes,
                        });
                    }
                    FetchItem::Rfc822Header => {
                        let bytes = match raw {
                            Some(ref b) => extract_section(b, &Section::Header),
                            None => Vec::new(),
                        };
                        segments.push(FetchSegment::Literal {
                            prefix: "RFC822.HEADER ".to_string(),
                            payload: bytes,
                        });
                    }
                    FetchItem::Rfc822Text => {
                        implicit_seen = true;
                        let bytes = match raw {
                            Some(ref b) => extract_section(b, &Section::Text),
                            None => Vec::new(),
                        };
                        segments.push(FetchSegment::Literal {
                            prefix: "RFC822.TEXT ".to_string(),
                            payload: bytes,
                        });
                    }
                }
            }

            // RFC 3501 §6.4.8: every FETCH response triggered by a
            // UID command must include the UID, even when the client
            // did not request it. Inject one if it's missing.
            if by_uid {
                let already_uid = segments.iter().any(|s| {
                    matches!(s, FetchSegment::Text(t) if t.starts_with("UID "))
                });
                if !already_uid {
                    segments.insert(0, FetchSegment::Text(fmt!("UID {}", meta.uid.0)));
                }
            }

            // Implicit \Seen.
            if implicit_seen && !session.read_only && !meta.flags.seen {
                let mut new_flags = meta.flags;
                new_flags.seen = true;
                let _ = self.store.set_flags(&user, &folder, meta.uid, new_flags);
                if let Some(m) = session.messages.iter_mut().find(|m| m.uid == meta.uid) {
                    m.flags.seen = true;
                }
                let already = segments.iter().any(|s| {
                    matches!(s, FetchSegment::Text(t) if t.starts_with("FLAGS"))
                });
                if !already {
                    segments.push(FetchSegment::Text(fmt!(
                        "FLAGS ({})", new_flags.to_imap_list())));
                }
            }

            // Emit `* SEQ FETCH ( ... )\r\n` with a SP between segments.
            let opener = fmt!("* {} FETCH (", seq_no);
            res!(write_all(stream, opener.as_bytes()).await);
            for (i, seg) in segments.iter().enumerate() {
                if i > 0 {
                    res!(write_all(stream, b" ").await);
                }
                match seg {
                    FetchSegment::Text(t) => {
                        res!(write_all(stream, t.as_bytes()).await);
                    }
                    FetchSegment::Literal { prefix, payload } => {
                        let head = fmt!("{}{{{}}}\r\n", prefix, payload.len());
                        res!(write_all(stream, head.as_bytes()).await);
                        res!(write_all(stream, payload).await);
                    }
                }
            }
            res!(write_all(stream, b")\r\n").await);
        }
        Ok(())
    }

    async fn do_store<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        session:    &mut ImapSession,
        seq_set:    &str,
        op:         &str,
        flags_raw:  &str,
        by_uid:     bool,
    )
        -> Outcome<()>
    {
        let user = match &session.user {
            Some(u) => u.clone(),
            None => return Ok(()),
        };
        let folder = match &session.selected {
            Some(f) => f.clone(),
            None => return Ok(()),
        };
        res!(self.refresh_selected(session, false).await);
        let indices = resolve_set(seq_set, &session.messages, by_uid);
        let silent = op.to_uppercase().ends_with(".SILENT");
        let base = op.trim_end_matches(".SILENT").trim_end_matches(".silent");
        let new_flag_names: Vec<&str> = flags_raw
            .split_whitespace()
            .collect();

        for &idx in &indices {
            let meta = match session.messages.get(idx) {
                Some(m) => m.clone(),
                None => continue,
            };
            let mut flags = meta.flags;
            match base {
                "+FLAGS" | "+flags" => {
                    for f in &new_flag_names { flags.set(f, true); }
                }
                "-FLAGS" | "-flags" => {
                    for f in &new_flag_names { flags.set(f, false); }
                }
                "FLAGS"  | "flags"  => {
                    flags = MessageFlags::default();
                    for f in &new_flag_names { flags.set(f, true); }
                }
                _ => {
                    return Err(err!(
                        "STORE op '{}' not understood.", op;
                        Invalid, Input));
                }
            }
            let final_flags = res!(self.store.set_flags(&user, &folder, meta.uid, flags));
            if let Some(m) = session.messages.iter_mut().find(|m| m.uid == meta.uid) {
                m.flags = final_flags;
            }
            if !silent {
                let mut parts = vec![fmt!("FLAGS ({})", final_flags.to_imap_list())];
                if by_uid {
                    parts.push(fmt!("UID {}", meta.uid.0));
                }
                let line = fmt!("* {} FETCH ({})\r\n", idx + 1, parts.join(" "));
                res!(write_all(stream, line.as_bytes()).await);
            }
        }
        Ok(())
    }

    async fn do_search<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        session:    &mut ImapSession,
        args:       &str,
        by_uid:     bool,
    )
        -> Outcome<()>
    {
        // Minimal SEARCH: ALL, UID <set>, SEEN/UNSEEN, ANSWERED,
        // UNANSWERED, FLAGGED, DELETED. Anything else returns the
        // entire mailbox.
        res!(self.refresh_selected(session, false).await);
        let trimmed = args.trim();
        let upper = trimmed.to_uppercase();
        let mut matched: Vec<&MessageMeta> = Vec::new();
        if upper == "ALL" || upper.is_empty() {
            for m in &session.messages { matched.push(m); }
        } else if upper == "UNSEEN" {
            for m in &session.messages { if !m.flags.seen { matched.push(m); } }
        } else if upper == "SEEN" {
            for m in &session.messages { if m.flags.seen { matched.push(m); } }
        } else if upper == "FLAGGED" {
            for m in &session.messages { if m.flags.flagged { matched.push(m); } }
        } else if upper == "DELETED" {
            for m in &session.messages { if m.flags.deleted { matched.push(m); } }
        } else if upper.starts_with("UID ") {
            let set = &trimmed[4..];
            let indices = resolve_set(set, &session.messages, true);
            for &i in &indices {
                if let Some(m) = session.messages.get(i) { matched.push(m); }
            }
        } else {
            // Fall back to ALL.
            for m in &session.messages { matched.push(m); }
        }
        let mut parts: Vec<String> = Vec::new();
        for m in &matched {
            if by_uid {
                parts.push(fmt!("{}", m.uid.0));
            } else {
                if let Some(idx) = session.messages.iter().position(|x| x.uid == m.uid) {
                    parts.push(fmt!("{}", idx + 1));
                }
            }
        }
        let line = fmt!("* SEARCH {}\r\n", parts.join(" "));
        res!(write_all(stream, line.as_bytes()).await);
        Ok(())
    }

    async fn do_append<S: AsyncRead + AsyncWrite + Unpin + Send>(
        &self,
        stream:     &mut S,
        user:       &MailUser,
        tag:        &str,
        args:       &str,
    )
        -> Outcome<bool>
    {
        // Parse: <folder> [(flags)] [date-time] {literal_size}
        let mut it = ArgIter::new(args);
        let folder = match it.next_string() {
            Some(s) => FolderName::new(s),
            None => {
                res!(write_bad(stream, tag, "APPEND requires folder").await);
                return Ok(false);
            }
        };
        let mut flags = MessageFlags::default();
        let mut internal: Option<SystemTime> = None;

        loop {
            it.skip_whitespace();
            if it.peek() == Some('(') {
                let raw_flags = it.next_paren_list().unwrap_or_default();
                for f in raw_flags.split_whitespace() {
                    flags.set(f, true);
                }
            } else if it.peek() == Some('"') {
                let dt = it.next_string().unwrap_or_default();
                internal = parse_internal_date(&dt);
            } else if it.peek() == Some('{') {
                break;
            } else {
                break;
            }
        }

        // Literal size.
        let lit = match it.next_literal_marker() {
            Some(n) => n,
            None => {
                res!(write_bad(stream, tag, "APPEND requires literal").await);
                return Ok(false);
            }
        };
        if lit > IMAP_MAX_LITERAL {
            res!(write_no(stream, tag, "Literal too large").await);
            return Ok(false);
        }
        // Synchronising literal: send continuation if not LITERAL+.
        if !it.literal_is_non_sync {
            res!(write_all(stream, b"+ Ready for literal data\r\n").await);
        }
        // Read exactly lit bytes.
        let mut buf = vec![0u8; lit];
        if let Err(e) = stream.read_exact(&mut buf).await {
            return Err(err!(e, "Reading APPEND literal."; IO, Network, Read));
        }
        // Read trailing CRLF after the literal.
        let mut tail = [0u8; 2];
        let _ = stream.read_exact(&mut tail).await;

        let uid = res!(self.store.append(user, &folder, &buf, flags, internal));
        let _ = uid;
        res!(write_ok(stream, tag, "APPEND completed").await);
        Ok(false)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COMMAND PARSING                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

#[derive(Clone, Debug)]
struct ParsedCommand {
    tag:        String,
    command:    String,
    args:       String,
}

fn parse_command(line: &str) -> Option<ParsedCommand> {
    let trimmed = line.trim_end_matches(|c: char| c == '\r' || c == '\n');
    let mut parts = trimmed.splitn(3, ' ');
    let tag = parts.next()?.to_string();
    let cmd = parts.next()?.to_string();
    let args = parts.next().unwrap_or("").to_string();
    if tag.is_empty() || cmd.is_empty() {
        return None;
    }
    Some(ParsedCommand { tag, command: cmd, args })
}

struct ArgIter<'a> {
    s:      &'a str,
    pos:    usize,
    /// Set after `next_literal_marker` if the marker had a `+` suffix
    /// (LITERAL+ extension).
    literal_is_non_sync: bool,
}

impl<'a> ArgIter<'a> {
    fn new(s: &'a str) -> Self { Self { s, pos: 0, literal_is_non_sync: false } }

    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' { self.pos += c.len_utf8(); }
            else { break; }
        }
    }

    fn rest(&self) -> &str { &self.s[self.pos..] }

    fn next_atom(&mut self) -> Option<String> {
        self.skip_whitespace();
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' { break; }
            self.pos += c.len_utf8();
        }
        if start == self.pos { None } else { Some(self.s[start..self.pos].to_string()) }
    }

    /// Read either a quoted string (`"..."`) or an atom; intended for
    /// folder names and other simple string arguments.
    fn next_string(&mut self) -> Option<String> {
        self.skip_whitespace();
        if self.peek() == Some('"') {
            self.pos += 1;
            let start = self.pos;
            let mut escaped = false;
            let mut out = String::new();
            while let Some(c) = self.peek() {
                self.pos += c.len_utf8();
                if escaped {
                    out.push(c);
                    escaped = false;
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    continue;
                }
                if c == '"' {
                    return Some(out);
                }
                out.push(c);
            }
            // Unterminated.
            Some(self.s[start..].to_string())
        } else {
            self.next_atom()
        }
    }

    /// Read a parenthesised flag list, returning the inner text.
    fn next_paren_list(&mut self) -> Option<String> {
        self.skip_whitespace();
        if self.peek() != Some('(') { return None; }
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            self.pos += c.len_utf8();
            if c == '(' { depth += 1; }
            if c == ')' {
                depth -= 1;
                if depth == 0 {
                    return Some(self.s[start..self.pos - 1].to_string());
                }
            }
        }
        Some(self.s[start..self.pos].to_string())
    }

    /// Read a literal marker `{N}` or `{N+}`. Sets
    /// `literal_is_non_sync` for the LITERAL+ form.
    fn next_literal_marker(&mut self) -> Option<usize> {
        self.skip_whitespace();
        if self.peek() != Some('{') { return None; }
        self.pos += 1;
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '}' { break; }
            self.pos += c.len_utf8();
        }
        let inner = &self.s[start..self.pos];
        if self.peek() == Some('}') { self.pos += 1; }
        let (digits, plus) = if let Some(d) = inner.strip_suffix('+') {
            (d, true)
        } else {
            (inner, false)
        };
        let n: usize = digits.parse().ok()?;
        self.literal_is_non_sync = plus;
        Some(n)
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FETCH ITEM PARSING                                                        │
// └───────────────────────────────────────────────────────────────────────────┘

#[derive(Clone, Debug)]
enum FetchItem {
    Uid,
    Flags,
    Rfc822Size,
    InternalDate,
    Envelope,
    Body { peek: bool, section: Section },
    Rfc822,
    Rfc822Header,
    Rfc822Text,
}

#[derive(Clone, Debug)]
enum Section {
    Whole,
    Header,
    Text,
    HeaderFields(Vec<String>),
    HeaderFieldsNot(Vec<String>),
}

fn parse_fetch_items(raw: &str) -> Vec<FetchItem> {
    // Strip enclosing parens if present. A single item may also appear
    // without parens.
    let inner = raw.trim();
    let inner = if inner.starts_with('(') && inner.ends_with(')') {
        &inner[1..inner.len() - 1]
    } else {
        inner
    };
    // Walk tokens. We need to handle BODY[...] and BODY.PEEK[...]
    // including their internal whitespace.
    let mut out = Vec::new();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
        if i >= bytes.len() { break; }
        // Read up to the next whitespace, or until a `[` (BODY[]).
        let start = i;
        while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'\t' && bytes[i] != b'[' {
            i += 1;
        }
        let mut name = inner[start..i].to_string();
        let mut section_text = String::new();
        if i < bytes.len() && bytes[i] == b'[' {
            // Read up to matching `]`.
            i += 1;
            let s = i;
            let mut depth = 1usize;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'[' { depth += 1; }
                if bytes[i] == b']' { depth -= 1; if depth == 0 { break; } }
                i += 1;
            }
            section_text = inner[s..i].to_string();
            if i < bytes.len() && bytes[i] == b']' { i += 1; }
            name.push_str(&fmt!("[{}]", section_text));
        }
        let token_upper = name.to_uppercase();
        match token_upper.as_str() {
            "UID"           => out.push(FetchItem::Uid),
            "FLAGS"         => out.push(FetchItem::Flags),
            "RFC822.SIZE"   => out.push(FetchItem::Rfc822Size),
            "INTERNALDATE"  => out.push(FetchItem::InternalDate),
            "ENVELOPE"      => out.push(FetchItem::Envelope),
            "RFC822"        => out.push(FetchItem::Rfc822),
            "RFC822.HEADER" => out.push(FetchItem::Rfc822Header),
            "RFC822.TEXT"   => out.push(FetchItem::Rfc822Text),
            _ => {
                if token_upper.starts_with("BODY[")
                    || token_upper.starts_with("BODY.PEEK[")
                {
                    let peek = token_upper.starts_with("BODY.PEEK[");
                    let section = parse_section(&section_text);
                    out.push(FetchItem::Body { peek, section });
                } else if token_upper == "BODY" {
                    out.push(FetchItem::Body { peek: false, section: Section::Whole });
                }
            }
        }
    }
    out
}

fn parse_section(text: &str) -> Section {
    let upper = text.to_uppercase();
    let trimmed = upper.trim();
    if trimmed.is_empty() {
        return Section::Whole;
    }
    if trimmed == "HEADER" {
        return Section::Header;
    }
    if trimmed == "TEXT" {
        return Section::Text;
    }
    if trimmed.starts_with("HEADER.FIELDS.NOT") {
        let rest = &text[text.find('(').map(|i| i + 1).unwrap_or(0)..];
        let rest = rest.trim_end_matches(')');
        let names = rest.split_whitespace().map(|s| s.to_string()).collect();
        return Section::HeaderFieldsNot(names);
    }
    if trimmed.starts_with("HEADER.FIELDS") {
        let rest = &text[text.find('(').map(|i| i + 1).unwrap_or(0)..];
        let rest = rest.trim_end_matches(')');
        let names = rest.split_whitespace().map(|s| s.to_string()).collect();
        return Section::HeaderFields(names);
    }
    Section::Whole
}

fn section_label(section: &Section) -> String {
    match section {
        Section::Whole              => String::new(),
        Section::Header             => "HEADER".to_string(),
        Section::Text               => "TEXT".to_string(),
        Section::HeaderFields(names) => fmt!(
            "HEADER.FIELDS ({})",
            names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "),
        ),
        Section::HeaderFieldsNot(names) => fmt!(
            "HEADER.FIELDS.NOT ({})",
            names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "),
        ),
    }
}

fn extract_section(bytes: &[u8], section: &Section) -> Vec<u8> {
    let (head, body) = split_msg(bytes);
    match section {
        Section::Whole  => bytes.to_vec(),
        Section::Header => {
            let mut h = head.to_vec();
            h.extend_from_slice(b"\r\n");
            h
        }
        Section::Text => body.to_vec(),
        Section::HeaderFields(names) => filter_headers(head, names, false),
        Section::HeaderFieldsNot(names) => filter_headers(head, names, true),
    }
}

fn split_msg(bytes: &[u8]) -> (&[u8], &[u8]) {
    if let Some(i) = find_subseq(bytes, b"\r\n\r\n") {
        return (&bytes[..i], &bytes[i + 4..]);
    }
    if let Some(i) = find_subseq(bytes, b"\n\n") {
        return (&bytes[..i], &bytes[i + 2..]);
    }
    (bytes, &[])
}

fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() { return None; }
    for i in 0..=hay.len() - needle.len() {
        if &hay[i..i + needle.len()] == needle { return Some(i); }
    }
    None
}

fn filter_headers(head: &[u8], names: &[String], invert: bool) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let upper_names: Vec<String> = names.iter().map(|s| s.to_uppercase()).collect();
    let mut out = String::new();
    let mut keep_current = false;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation; if we are keeping the previous header,
            // keep the continuation too.
            if keep_current {
                out.push_str(line);
                out.push_str("\r\n");
            }
            continue;
        }
        if let Some(i) = line.find(':') {
            let name = line[..i].trim().to_uppercase();
            let in_list = upper_names.iter().any(|n| n == &name);
            keep_current = if invert { !in_list } else { in_list };
            if keep_current {
                out.push_str(line);
                out.push_str("\r\n");
            }
        } else {
            keep_current = false;
        }
    }
    out.push_str("\r\n");
    out.into_bytes()
}

fn build_envelope(bytes: &[u8]) -> String {
    let (head, _) = split_msg(bytes);
    let headers = parse_headers(head);
    let date = nstring(headers.get("date"));
    let subj = nstring(headers.get("subject"));
    let from = address_list(headers.get("from"));
    let sender = headers.get("sender")
        .map(|s| address_list(Some(s)))
        .unwrap_or_else(|| from.clone());
    let reply_to = headers.get("reply-to")
        .map(|s| address_list(Some(s)))
        .unwrap_or_else(|| from.clone());
    let to = address_list(headers.get("to"));
    let cc = address_list(headers.get("cc"));
    let bcc = address_list(headers.get("bcc"));
    let in_reply_to = nstring(headers.get("in-reply-to"));
    let message_id = nstring(headers.get("message-id"));
    fmt!(
        "({} {} {} {} {} {} {} {} {} {})",
        date, subj, from, sender, reply_to, to, cc, bcc, in_reply_to, message_id,
    )
}

fn parse_headers(head: &[u8]) -> std::collections::HashMap<String, String> {
    let text = String::from_utf8_lossy(head);
    let mut out = std::collections::HashMap::new();
    let mut name: Option<String> = None;
    let mut value = String::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() { continue; }
        if line.starts_with(' ') || line.starts_with('\t') {
            value.push(' ');
            value.push_str(line.trim_start());
            continue;
        }
        if let Some(n) = name.take() {
            out.insert(n.to_lowercase(), value.trim().to_string());
            value.clear();
        }
        if let Some(i) = line.find(':') {
            name = Some(line[..i].trim().to_string());
            value = line[i + 1..].trim().to_string();
        }
    }
    if let Some(n) = name {
        out.insert(n.to_lowercase(), value.trim().to_string());
    }
    out
}

fn nstring(s: Option<&String>) -> String {
    match s {
        Some(s) => fmt!("\"{}\"", escape_quoted(s)),
        None => "NIL".to_string(),
    }
}

/// Render an address list as a parenthesised list of address triples,
/// `((name nil mailbox host) ...)`. The MVP parser only handles the
/// simple `local@domain` and `Name <local@domain>` forms.
fn address_list(s: Option<&String>) -> String {
    let s = match s { Some(s) => s.as_str(), None => return "NIL".to_string() };
    let mut entries: Vec<String> = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() { continue; }
        let (name, addr) = if let Some(start) = part.find('<') {
            let end = part.find('>').unwrap_or(part.len());
            let name = part[..start].trim().trim_matches('"').to_string();
            (name, part[start + 1..end].to_string())
        } else {
            (String::new(), part.to_string())
        };
        let (local, host) = match addr.rfind('@') {
            Some(i) => (addr[..i].to_string(), addr[i + 1..].to_string()),
            None    => (addr, String::new()),
        };
        let name_field = if name.is_empty() {
            "NIL".to_string()
        } else {
            fmt!("\"{}\"", escape_quoted(&name))
        };
        entries.push(fmt!(
            "({} NIL \"{}\" \"{}\")",
            name_field,
            escape_quoted(&local),
            escape_quoted(&host),
        ));
    }
    if entries.is_empty() {
        "NIL".to_string()
    } else {
        fmt!("({})", entries.join(""))
    }
}

fn escape_quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ SET RESOLUTION                                                            │
// └───────────────────────────────────────────────────────────────────────────┘

/// Resolve an IMAP message set (`1`, `1:5`, `*`, `1,3,5:7`) to a list
/// of indices into the cached message list. When `by_uid` is true the
/// set is interpreted as a UID range, otherwise as a sequence-number
/// range.
fn resolve_set(set: &str, msgs: &[MessageMeta], by_uid: bool) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if msgs.is_empty() { return out; }
    let max_seq = msgs.len() as u32;
    let max_uid = msgs.last().map(|m| m.uid.0).unwrap_or(0);
    for piece in set.split(',') {
        let piece = piece.trim();
        if piece.is_empty() { continue; }
        let (lo_str, hi_str) = match piece.find(':') {
            Some(i) => (&piece[..i], &piece[i + 1..]),
            None    => (piece, piece),
        };
        let lo = parse_set_number(lo_str, by_uid, max_seq, max_uid);
        let hi = parse_set_number(hi_str, by_uid, max_seq, max_uid);
        let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
        for i in 0..msgs.len() {
            let key = if by_uid { msgs[i].uid.0 } else { (i + 1) as u32 };
            if key >= lo && key <= hi {
                out.push(i);
            }
        }
    }
    out
}

fn parse_set_number(s: &str, by_uid: bool, max_seq: u32, max_uid: u32) -> u32 {
    if s == "*" { if by_uid { max_uid } else { max_seq } }
    else { s.parse().unwrap_or(0) }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RESPONSE WRITERS                                                          │
// └───────────────────────────────────────────────────────────────────────────┘

async fn write_all<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> Outcome<()> {
    if let Err(e) = stream.write_all(bytes).await {
        return Err(err!(e, "Writing IMAP bytes."; IO, Network, Write));
    }
    if let Err(e) = stream.flush().await {
        return Err(err!(e, "Flushing IMAP bytes."; IO, Network, Write));
    }
    Ok(())
}

async fn write_ok<S: AsyncWrite + Unpin>(stream: &mut S, tag: &str, text: &str) -> Outcome<()> {
    let line = fmt!("{} OK {}\r\n", tag, text);
    write_all(stream, line.as_bytes()).await
}

async fn write_no<S: AsyncWrite + Unpin>(stream: &mut S, tag: &str, text: &str) -> Outcome<()> {
    let line = fmt!("{} NO {}\r\n", tag, text);
    write_all(stream, line.as_bytes()).await
}

async fn write_bad<S: AsyncWrite + Unpin>(stream: &mut S, tag: &str, text: &str) -> Outcome<()> {
    let line = fmt!("{} BAD {}\r\n", tag, text);
    write_all(stream, line.as_bytes()).await
}

async fn read_line<S: AsyncRead + Unpin>(stream: &mut S) -> Outcome<Option<String>> {
    let mut buf = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    loop {
        let n = match stream.read(&mut byte).await {
            Ok(n) => n,
            Err(e) => return Err(err!(e, "Reading IMAP line."; IO, Network, Read)),
        };
        if n == 0 {
            if buf.is_empty() { return Ok(None); }
            break;
        }
        buf.push(byte[0]);
        if byte[0] == b'\n' { break; }
        if buf.len() >= IMAP_MAX_LINE {
            return Err(err!(
                "IMAP line exceeded {} bytes.", IMAP_MAX_LINE;
                Invalid, Input, Excessive));
        }
    }
    while buf.last() == Some(&b'\n') || buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MISC HELPERS                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

fn capability_list() -> &'static str {
    "IMAP4rev1 LITERAL+ AUTH=PLAIN AUTH=LOGIN SPECIAL-USE IDLE"
}

/// Return the RFC 6154 SPECIAL-USE attribute (with leading backslash
/// and no trailing space) for a well-known folder name, plus the
/// `\HasNoChildren` hint that most clients rely on. Returns just
/// `\HasNoChildren` for ordinary folders.
fn special_use_attrs(name: &str) -> String {
    let su = match name {
        "Sent"      => Some("\\Sent"),
        "Drafts"    => Some("\\Drafts"),
        "Trash"     => Some("\\Trash"),
        "Junk"      => Some("\\Junk"),
        "Archive"   => Some("\\Archive"),
        _ => None,
    };
    match su {
        Some(tag) => fmt!("{} \\HasNoChildren", tag),
        None      => "\\HasNoChildren".to_string(),
    }
}

/// Match an IMAP `LIST` mailbox pattern against a folder name. `*`
/// matches any number of characters (including the hierarchy
/// separator), `%` matches any number of characters except `/`. An
/// empty pattern matches everything.
fn match_imap_pattern(pattern: &str, name: &str) -> bool {
    if pattern.is_empty() { return true; }
    pattern_recurse(pattern.as_bytes(), name.as_bytes())
}

fn pattern_recurse(p: &[u8], n: &[u8]) -> bool {
    if p.is_empty() { return n.is_empty(); }
    match p[0] {
        b'*' => {
            for i in 0..=n.len() {
                if pattern_recurse(&p[1..], &n[i..]) {
                    return true;
                }
            }
            false
        }
        b'%' => {
            for i in 0..=n.len() {
                // % does not match the separator '/'.
                if i > 0 && n[i - 1] == b'/' { break; }
                if pattern_recurse(&p[1..], &n[i..]) {
                    return true;
                }
            }
            false
        }
        c => {
            if !n.is_empty() && n[0].eq_ignore_ascii_case(&c) {
                pattern_recurse(&p[1..], &n[1..])
            } else {
                false
            }
        }
    }
}

/// Format a SystemTime as an IMAP INTERNALDATE string, e.g.
/// `13-Apr-2026 10:00:00 +0000`. Fixed UTC offset.
fn format_internal_date(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, mo, d, h, mi, s) = unix_to_civil(secs);
    let months = [
        "Jan","Feb","Mar","Apr","May","Jun",
        "Jul","Aug","Sep","Oct","Nov","Dec",
    ];
    let mon = months[(mo as usize - 1).min(11)];
    fmt!("{:02}-{}-{:04} {:02}:{:02}:{:02} +0000", d, mon, y, h, mi, s)
}

/// Parse an IMAP date-time string back into a SystemTime. Tolerant.
fn parse_internal_date(s: &str) -> Option<SystemTime> {
    // Format: "13-Apr-2026 10:00:00 +0000"
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 { return None; }
    let date_parts: Vec<&str> = parts[0].split('-').collect();
    if date_parts.len() != 3 { return None; }
    let day: u32 = date_parts[0].parse().ok()?;
    let mon: u32 = match date_parts[1] {
        "Jan"=>1,"Feb"=>2,"Mar"=>3,"Apr"=>4,"May"=>5,"Jun"=>6,
        "Jul"=>7,"Aug"=>8,"Sep"=>9,"Oct"=>10,"Nov"=>11,"Dec"=>12,
        _ => return None,
    };
    let year: i32 = date_parts[2].parse().ok()?;
    let time_parts: Vec<&str> = parts[1].split(':').collect();
    if time_parts.len() != 3 { return None; }
    let h: u32 = time_parts[0].parse().ok()?;
    let mi: u32 = time_parts[1].parse().ok()?;
    let se: u32 = time_parts[2].parse().ok()?;
    let secs = civil_to_unix(year, mon, day, h, mi, se);
    Some(UNIX_EPOCH + std::time::Duration::from_secs(secs))
}

/// Convert Unix seconds to (year, month, day, hour, minute, second)
/// using Howard Hinnant's date algorithm (proleptic Gregorian, UTC).
fn unix_to_civil(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let h = rem / 3_600;
    let mi = (rem / 60) % 60;
    let s = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d, h, mi, s)
}

/// Inverse of `unix_to_civil`.
fn civil_to_unix(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * if m > 2 { m - 3 } else { m + 9 } + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146_097 + doe as i64 - 719_468;
    let secs = (days as u64).wrapping_mul(86_400)
        + h as u64 * 3_600
        + mi as u64 * 60
        + s as u64;
    secs
}

/// One emitted FETCH response component. Text segments are joined
/// with a SP separator inside the parenthesised FETCH response;
/// literal segments contain raw bytes that must be written byte-for-
/// byte after a `prefix{N}\r\n` header.
enum FetchSegment {
    Text(String),
    Literal { prefix: String, payload: Vec<u8> },
}
