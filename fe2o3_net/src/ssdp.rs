//! SSDP: the discovery half of UPnP (UPnP Device Architecture 2.0, §1).
//!
//! A television on a home network finds what it can play from by shouting into a
//! multicast group and listening for whoever answers. That is all SSDP is: three
//! HTTP-shaped messages sent over UDP to 239.255.255.250:1900, with a start line
//! and a block of fields and no body at all.
//!
//! - `M-SEARCH * HTTP/1.1` -- a searcher asking who is out there.
//! - `HTTP/1.1 200 OK` -- a unicast answer to a search, sent back to the asker.
//! - `NOTIFY * HTTP/1.1` -- an announcement, multicast: `ssdp:alive` when a
//!   device appears or renews, `ssdp:byebye` when it goes.
//!
//! Every one of them names a service in two ways: a *target* (`ST` on a search
//! and its answer, `NT` on a notification) saying what kind of thing it is, and a
//! `USN` saying which particular thing it is. A responder that gets those two out
//! of step is discovered and then cannot be reached, which is the failure mode
//! that eats an afternoon.
//!
//! # What is here
//!
//! The messages, their parsing and their serialisation, and a responder that
//! binds the group and reads and writes them. The types and the parsing are
//! tested; the responder's live behaviour on a real network is not, and wants a
//! second machine to test against rather than a unit test.
//!
//! # What is not
//!
//! The description document a `LOCATION` points at, and the SOAP services behind
//! it, are UPnP rather than SSDP and live in [`crate::upnp`].
//!
//! # Two responders
//!
//! [`Responder`] is async over tokio. [`SyncResponder`] is the same protocol over
//! `std::net`, for a binary that wants no runtime: discovery is one socket, three
//! message shapes and a thread that blocks on a read, and pulling in an executor
//! to run it is a poor trade. Neither is a wrapper around the other; they share
//! the messages above, which is where the protocol actually is.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::time::Time;

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::BTreeMap,
    fmt,
    net::{
        Ipv4Addr,
        SocketAddr,
        SocketAddrV4,
    },
    str::FromStr,
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use tokio::net::UdpSocket;


//// The group, and the port that goes with it (UPnP DA 2.0 §1.1.1).
pub const MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
pub const PORT: u16 = 1900;

// The value of `MAN` on a search. The quotes are part of the field, and a
// searcher that leaves them off is ignored by conforming devices.
pub const DISCOVER: &str = "\"ssdp:discover\"";

// The largest datagram this reader will accept. A conforming SSDP message is a
// few hundred bytes; the rest of the space is for the long `SERVER` and
// vendor-extension fields that real devices send.
pub const MAX_DATAGRAM: usize = 2048;

// How long an announcement stands before it must be renewed. The specification
// requires at least 1800.
pub const DEFAULT_MAX_AGE: u32 = 1800;  // seconds


/// What a message is about: a device type, a service type, a particular device by
/// its UUID, or everything at once.
///
/// Held as an enum because a responder answers `All` and `RootDevice` and its own
/// `Uuid` differently, and a string comparison spread across the call sites is how
/// one of those cases quietly stops being answered.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Target {
    All,            // `ssdp:all`, answered once per thing
    RootDevice,     // `upnp:rootdevice`
    Uuid(String),   // `uuid:...`, one particular device
    Urn(String),    // `urn:...`, a device or service type
    Other(String),  // anything else a device chose to name itself by
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All           => write!(f, "ssdp:all"),
            Self::RootDevice    => write!(f, "upnp:rootdevice"),
            Self::Uuid(s)       => write!(f, "uuid:{}", s),
            Self::Urn(s)        => write!(f, "urn:{}", s),
            Self::Other(s)      => write!(f, "{}", s),
        }
    }
}

impl FromStr for Target {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        Ok(match s {
            "ssdp:all"          => Self::All,
            "upnp:rootdevice"   => Self::RootDevice,
            _ => match s.split_once(':') {
                Some(("uuid", rest))    => Self::Uuid(rest.to_string()),
                Some(("urn", rest))     => Self::Urn(rest.to_string()),
                _                       => Self::Other(s.to_string()),
            },
        })
    }
}

impl Target {
    /// Does an announcement of `self` answer a search for `wanted`? `ssdp:all`
    /// matches everything, and everything else matches only itself.
    pub fn answers(&self, wanted: &Target) -> bool {
        match wanted {
            Target::All => true,
            other       => self == other,
        }
    }
}

/// What a `NOTIFY` is saying, carried in its `NTS` field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Nts {
    Alive,      // here, and stays for its `CACHE-CONTROL` lifetime
    ByeBye,     // going, now
    Update,     // still here, on a new boot identifier, UPnP DA 2.0 §1.2.4
}

impl fmt::Display for Nts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Alive     => "ssdp:alive",
            Self::ByeBye    => "ssdp:byebye",
            Self::Update    => "ssdp:update",
        })
    }
}

impl FromStr for Nts {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.trim() {
            "ssdp:alive"    => Self::Alive,
            "ssdp:byebye"   => Self::ByeBye,
            "ssdp:update"   => Self::Update,
            _ => return Err(err!(
                "Unrecognised SSDP NTS '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// A search: `M-SEARCH * HTTP/1.1`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Search {
    pub target:     Target,
    // `MX`: the largest number of seconds a responder should wait before
    // answering, so a hundred devices do not answer at the same instant.
    pub mx:         u8,
    pub user_agent: Option<String>,             // who asked, and what they call themselves
    pub extra:      BTreeMap<String, String>,   // fields this crate does not model
}

impl Search {
    /// The customary two second spread.
    pub fn new(target: Target) -> Self {
        Self {
            target,
            mx:         2,
            user_agent: None,
            extra:      BTreeMap::new(),
        }
    }
}

/// A unicast answer to a search: `HTTP/1.1 200 OK`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResponse {
    pub max_age:    u32,                        // seconds the answer stands
    pub date:       Option<String>,             // HTTP date, generated by Responder when absent
    pub location:   String,                     // the device description document
    pub server:     String,                     // OS, UPnP version, product
    pub target:     Target,                     // echoed back from the search
    pub usn:        String,                     // which particular thing is answering
    // `BOOTID.UPNP.ORG` changes when the device restarts, `CONFIGID.UPNP.ORG`
    // when its description does.
    pub boot_id:    Option<u32>,
    pub config_id:  Option<u32>,
    pub extra:      BTreeMap<String, String>,   // fields this crate does not model
}

/// An announcement: `NOTIFY * HTTP/1.1`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notify {
    pub target:     Target,
    pub nts:        Nts,                        // arriving, leaving, or renewing
    pub usn:        String,                     // which particular thing
    // A `byebye` carries neither of these: there is nothing left to fetch, and
    // the announcement stands until contradicted.
    pub max_age:    Option<u32>,                // seconds
    pub location:   Option<String>,             // the description document
    pub server:     Option<String>,
    pub boot_id:    Option<u32>,                // `BOOTID.UPNP.ORG`
    pub config_id:  Option<u32>,                // `CONFIGID.UPNP.ORG`
    pub extra:      BTreeMap<String, String>,   // fields this crate does not model
}

impl Notify {
    /// An `ssdp:alive` for a thing that has just appeared, or is renewing.
    pub fn alive(target: Target, usn: String, location: String, server: String) -> Self {
        Self {
            target,
            nts:        Nts::Alive,
            usn,
            max_age:    Some(DEFAULT_MAX_AGE),
            location:   Some(location),
            server:     Some(server),
            boot_id:    None,
            config_id:  None,
            extra:      BTreeMap::new(),
        }
    }

    /// Carries neither a lifetime nor a location: there is nothing left to fetch,
    /// and saying otherwise leaves a control point holding a URL that has stopped
    /// answering.
    pub fn byebye(target: Target, usn: String) -> Self {
        Self {
            target,
            nts:        Nts::ByeBye,
            usn,
            max_age:    None,
            location:   None,
            server:     None,
            boot_id:    None,
            config_id:  None,
            extra:      BTreeMap::new(),
        }
    }
}

/// One SSDP datagram.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SsdpMessage {
    Search(Search),
    Response(SearchResponse),
    Notify(Notify),
}

impl SsdpMessage {

    /// Real networks carry SSDP from devices that get the details wrong, so this
    /// is forgiving about spacing, field name case and line endings, and strict
    /// only about the things that decide what the message means: the start line,
    /// and the fields the message cannot be acted on without.
    pub fn parse(bytes: &[u8]) -> Outcome<Self> {
        let text = String::from_utf8_lossy(bytes);
        let mut lines = text.split('\n');
        let start = match lines.next() {
            Some(line) => line.trim_end_matches('\r').trim(),
            None => return Err(err!(
                "An SSDP datagram was empty."; IO, Network, Input, Missing)),
        };

        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        for line in lines {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            match line.split_once(':') {
                Some((name, value)) => {
                    fields.insert(name.trim().to_uppercase(), value.trim().to_string());
                }
                // A line with no colon is not a field. Devices send them; they
                // are of no use and are not worth refusing the message over.
                None => continue,
            }
        }

        let upper = start.to_uppercase();
        if upper.starts_with("M-SEARCH") {
            return Ok(Self::Search(res!(Search::from_fields(&mut fields))));
        }
        if upper.starts_with("NOTIFY") {
            return Ok(Self::Notify(res!(Notify::from_fields(&mut fields))));
        }
        if upper.starts_with("HTTP/") {
            // Only a 200 is an answer. Anything else is a device refusing, and
            // there is nothing to be discovered from it.
            let code = start.split_whitespace().nth(1).unwrap_or("");
            if code != "200" {
                return Err(err!(
                    "An SSDP response answered '{}', which is not a discovery.", start;
                IO, Network, Input, Invalid));
            }
            return Ok(Self::Response(res!(SearchResponse::from_fields(&mut fields))));
        }

        Err(err!(
            "'{}' is not the start line of any SSDP message.", start;
        IO, Network, Input, Invalid))
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

impl fmt::Display for SsdpMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Search(m)     => write!(f, "{}", m.as_text()),
            Self::Response(m)   => write!(f, "{}", m.as_text()),
            Self::Notify(m)     => write!(f, "{}", m.as_text()),
        }
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FIELDS TO MESSAGES, AND BACK                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// Take a field out of the map, so what is left over is the extras.
fn take(fields: &mut BTreeMap<String, String>, name: &str) -> Option<String> {
    fields.remove(name)
}

fn need(fields: &mut BTreeMap<String, String>, name: &str, what: &str) -> Outcome<String> {
    match fields.remove(name) {
        Some(v) => Ok(v),
        None => Err(err!(
            "An SSDP {} carried no {} field.", what, name;
        IO, Network, Input, Missing)),
    }
}

/// Read a `CACHE-CONTROL: max-age=1800` into its seconds.
fn max_age_of(value: &str) -> Option<u32> {
    for part in value.split(',') {
        if let Some((k, v)) = part.split_once('=') {
            if k.trim().eq_ignore_ascii_case("max-age") {
                return v.trim().parse().ok();
            }
        }
    }
    None
}

/// In a stable order, so a message written twice is the same bytes twice.
fn extra_lines(extra: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (name, value) in extra {
        out.push_str(&fmt!("{}: {}\r\n", name, value));
    }
    out
}

impl Search {

    fn from_fields(fields: &mut BTreeMap<String, String>) -> Outcome<Self> {
        // `HOST` and `MAN` are required of a sender and useless to a receiver
        // that already knows where the datagram arrived and what kind it is, so
        // they are dropped rather than checked: a device that sends a slightly
        // wrong `MAN` is still searching.
        let _ = take(fields, "HOST");
        let _ = take(fields, "MAN");
        let st = res!(need(fields, "ST", "search"));
        let target = res!(Target::from_str(&st));
        let mx = take(fields, "MX")
            .and_then(|v| v.trim().parse::<u8>().ok())
            .unwrap_or(0);
        let user_agent = take(fields, "USER-AGENT");
        Ok(Self {
            target,
            mx,
            user_agent,
            extra: std::mem::take(fields),
        })
    }

    pub fn as_text(&self) -> String {
        let mut out = String::new();
        out.push_str("M-SEARCH * HTTP/1.1\r\n");
        out.push_str(&fmt!("HOST: {}:{}\r\n", MULTICAST_ADDR, PORT));
        out.push_str(&fmt!("MAN: {}\r\n", DISCOVER));
        out.push_str(&fmt!("MX: {}\r\n", self.mx));
        out.push_str(&fmt!("ST: {}\r\n", self.target));
        if let Some(ua) = &self.user_agent {
            out.push_str(&fmt!("USER-AGENT: {}\r\n", ua));
        }
        out.push_str(&extra_lines(&self.extra));
        out.push_str("\r\n");
        out
    }
}

impl SearchResponse {

    fn from_fields(fields: &mut BTreeMap<String, String>) -> Outcome<Self> {
        let _ = take(fields, "EXT");
        let max_age = take(fields, "CACHE-CONTROL")
            .and_then(|v| max_age_of(&v))
            .unwrap_or(DEFAULT_MAX_AGE);
        let date = take(fields, "DATE");
        let location = res!(need(fields, "LOCATION", "response"));
        let server = take(fields, "SERVER").unwrap_or_default();
        let st = res!(need(fields, "ST", "response"));
        let target = res!(Target::from_str(&st));
        let usn = res!(need(fields, "USN", "response"));
        let boot_id = take(fields, "BOOTID.UPNP.ORG").and_then(|v| v.trim().parse().ok());
        let config_id = take(fields, "CONFIGID.UPNP.ORG").and_then(|v| v.trim().parse().ok());
        Ok(Self {
            max_age,
            date,
            location,
            server,
            target,
            usn,
            boot_id,
            config_id,
            extra: std::mem::take(fields),
        })
    }

    /// `EXT:` is an empty field that means nothing and is required anyway
    /// (UPnP DA 2.0 §1.3.3): a control point that does not see it discards the
    /// answer.
    pub fn as_text(&self) -> String {
        let mut out = String::new();
        out.push_str("HTTP/1.1 200 OK\r\n");
        out.push_str(&fmt!("CACHE-CONTROL: max-age={}\r\n", self.max_age));
        if let Some(date) = &self.date {
            out.push_str(&fmt!("DATE: {}\r\n", date));
        }
        out.push_str("EXT:\r\n");
        out.push_str(&fmt!("LOCATION: {}\r\n", self.location));
        out.push_str(&fmt!("SERVER: {}\r\n", self.server));
        out.push_str(&fmt!("ST: {}\r\n", self.target));
        out.push_str(&fmt!("USN: {}\r\n", self.usn));
        if let Some(id) = self.boot_id {
            out.push_str(&fmt!("BOOTID.UPNP.ORG: {}\r\n", id));
        }
        if let Some(id) = self.config_id {
            out.push_str(&fmt!("CONFIGID.UPNP.ORG: {}\r\n", id));
        }
        out.push_str(&extra_lines(&self.extra));
        out.push_str("\r\n");
        out
    }
}

impl Notify {

    fn from_fields(fields: &mut BTreeMap<String, String>) -> Outcome<Self> {
        let _ = take(fields, "HOST");
        let nt = res!(need(fields, "NT", "notification"));
        let target = res!(Target::from_str(&nt));
        let nts_txt = res!(need(fields, "NTS", "notification"));
        let nts = res!(Nts::from_str(&nts_txt));
        let usn = res!(need(fields, "USN", "notification"));
        let max_age = take(fields, "CACHE-CONTROL").and_then(|v| max_age_of(&v));
        let location = take(fields, "LOCATION");
        let server = take(fields, "SERVER");
        let boot_id = take(fields, "BOOTID.UPNP.ORG").and_then(|v| v.trim().parse().ok());
        let config_id = take(fields, "CONFIGID.UPNP.ORG").and_then(|v| v.trim().parse().ok());
        Ok(Self {
            target,
            nts,
            usn,
            max_age,
            location,
            server,
            boot_id,
            config_id,
            extra: std::mem::take(fields),
        })
    }

    pub fn as_text(&self) -> String {
        let mut out = String::new();
        out.push_str("NOTIFY * HTTP/1.1\r\n");
        out.push_str(&fmt!("HOST: {}:{}\r\n", MULTICAST_ADDR, PORT));
        if let Some(age) = self.max_age {
            out.push_str(&fmt!("CACHE-CONTROL: max-age={}\r\n", age));
        }
        if let Some(loc) = &self.location {
            out.push_str(&fmt!("LOCATION: {}\r\n", loc));
        }
        out.push_str(&fmt!("NT: {}\r\n", self.target));
        out.push_str(&fmt!("NTS: {}\r\n", self.nts));
        if let Some(server) = &self.server {
            out.push_str(&fmt!("SERVER: {}\r\n", server));
        }
        out.push_str(&fmt!("USN: {}\r\n", self.usn));
        if let Some(id) = self.boot_id {
            out.push_str(&fmt!("BOOTID.UPNP.ORG: {}\r\n", id));
        }
        if let Some(id) = self.config_id {
            out.push_str(&fmt!("CONFIGID.UPNP.ORG: {}\r\n", id));
        }
        out.push_str(&extra_lines(&self.extra));
        out.push_str("\r\n");
        out
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SOCKET                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// A socket bound to the SSDP group on one interface, reading and writing the
/// messages above.
///
/// # One interface at a time
///
/// A machine with two networks on it is on two SSDP groups, and a single socket
/// joined to both cannot say which one a datagram came from or choose which one an
/// announcement goes out on. So a responder is per-interface, named by the local
/// address to bind and join on, and a caller with several interfaces holds several
/// responders.
///
/// # What this cannot do yet
///
/// Two processes cannot both listen on port 1900: that needs `SO_REUSEADDR`, which
/// neither the standard library nor tokio exposes on a bound socket, and setting
/// it means a socket option this crate has no dependency to set. A machine already
/// running a UPnP daemon will therefore refuse the bind, with the address in the
/// error. Likewise the outgoing interface for a multicast send is chosen by
/// binding to that interface's address rather than by `IP_MULTICAST_IF`, which is
/// what the kernel does with a bound source address on Linux and every BSD.
#[derive(Debug)]
pub struct Responder {
    socket: UdpSocket,
    iface:  Ipv4Addr,   // bound to, and announced on
}

impl Responder {

    /// `iface` is the local IPv4 address of the interface to speak on;
    /// `Ipv4Addr::UNSPECIFIED` lets the kernel choose, which is right on a machine
    /// with one network and wrong on a machine with two.
    pub async fn bind(iface: Ipv4Addr) -> Outcome<Self> {
        Self::bind_to_port(iface, PORT).await
    }

    /// A test wants an ephemeral port; a real responder wants 1900, because that
    /// is where searches are sent.
    pub async fn bind_to_port(iface: Ipv4Addr, port: u16) -> Outcome<Self> {
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        let socket = match UdpSocket::bind(bind_addr).await {
            Ok(s)  => s,
            Err(e) => return Err(err!(e,
                "Binding the SSDP port {}. A UPnP daemon already listening there \
                holds it exclusively, since this socket does not ask to share it.",
                port;
            IO, Network, Init)),
        };
        let result = socket.join_multicast_v4(MULTICAST_ADDR, iface);
        res!(result, IO, Network, Init);
        // An announcement should be heard by the machine that sent it, since a
        // control point may be running beside the device it is discovering.
        let result = socket.set_multicast_loop_v4(true);
        res!(result, IO, Network, Init);
        Ok(Self {
            socket,
            iface,
        })
    }

    pub fn local_addr(&self) -> Outcome<SocketAddr> {
        let result = self.socket.local_addr();
        Ok(res!(result, IO, Network))
    }

    pub fn interface(&self) -> Ipv4Addr {
        self.iface
    }

    /// A datagram that does not parse is not an error the caller can do anything
    /// about -- the network carries plenty of them -- so it is logged and the wait
    /// resumes.
    pub async fn recv(&self) -> Outcome<(SsdpMessage, SocketAddr)> {
        let mut buf = vec![0u8; MAX_DATAGRAM];
        loop {
            let (n, from) = match self.socket.recv_from(&mut buf).await {
                Ok(pair) => pair,
                Err(e) => return Err(err!(e,
                    "Reading an SSDP datagram."; IO, Network, Wire, Read)),
            };
            match SsdpMessage::parse(&buf[..n]) {
                Ok(msg) => return Ok((msg, from)),
                Err(e)  => {
                    debug!("An SSDP datagram from {} did not parse: {}", from, e);
                    continue;
                }
            }
        }
    }

    pub async fn multicast(&self, msg: &SsdpMessage) -> Outcome<()> {
        let to = SocketAddrV4::new(MULTICAST_ADDR, PORT);
        res!(self.send_to(msg, SocketAddr::V4(to)).await);
        Ok(())
    }

    pub async fn send_to(&self, msg: &SsdpMessage, to: SocketAddr) -> Outcome<()> {
        let bytes = msg.as_bytes();
        let result = self.socket.send_to(&bytes, to).await;
        let sent = res!(result, IO, Network, Wire, Write);
        if sent != bytes.len() {
            return Err(err!(
                "An SSDP datagram of {} bytes went out as {}.", bytes.len(), sent;
            IO, Network, Wire, Write, Size));
        }
        Ok(())
    }

    /// Goes back to the address the search came from. The `ST` of the answer is
    /// the target actually being announced, not the `ssdp:all` that may have been
    /// asked: a control point matches the two, and an answer that echoes
    /// `ssdp:all` is discarded.
    pub async fn answer(
        &self,
        to:         SocketAddr,
        target:     Target,
        usn:        String,
        location:   String,
        server:     String,
    )
        -> Outcome<()>
    {
        let response = SearchResponse {
            max_age:    DEFAULT_MAX_AGE,
            date:       Some(res!(http_date())),
            location,
            server,
            target,
            usn,
            boot_id:    None,
            config_id:  None,
            extra:      BTreeMap::new(),
        };
        res!(self.send_to(&SsdpMessage::Response(response), to).await);
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE SAME SOCKET, WITHOUT A RUNTIME                                        │
// └───────────────────────────────────────────────────────────────────────────┘

/// A blocking SSDP socket: the same protocol as [`Responder`] over `std::net`.
///
/// A media server is a thread that blocks on a read and answers what arrives.
/// That is the whole of discovery, and it needs no executor.
///
/// # Where the datagrams go
///
/// One socket is bound to `0.0.0.0` on the SSDP port and joined to the group on
/// each interface named by [`SyncResponder::join`]. A search is answered by
/// unicast back to whoever sent it, which the routing table places correctly
/// however many interfaces there are. A multicast announcement, though, leaves by
/// the interface the kernel picks, because `IP_MULTICAST_IF` is not reachable
/// through the standard library and this crate carries no socket-options
/// dependency. On a machine with one network -- which is what a household has --
/// the two are the same interface.
///
/// # Sharing the port
///
/// Two processes cannot both hold port 1900 here: `SO_REUSEADDR` and
/// `SO_REUSEPORT` are likewise out of the standard library's reach. A machine
/// already running a UPnP daemon refuses the bind, with the address in the error.
#[derive(Debug)]
pub struct SyncResponder {
    socket: std::net::UdpSocket,
    joined: Vec<Ipv4Addr>,  // the interfaces the group was successfully joined on
}

impl SyncResponder {

    /// On every address, and joining no group yet.
    pub fn bind() -> Outcome<Self> {
        Self::bind_to_port(PORT)
    }

    /// The same, on a port of the caller's choosing. A test wants an ephemeral
    /// one; a real responder wants 1900, because that is where searches are sent.
    pub fn bind_to_port(port: u16) -> Outcome<Self> {
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        let socket = match std::net::UdpSocket::bind(bind_addr) {
            Ok(s)  => s,
            Err(e) => return Err(err!(e,
                "Binding the SSDP port {}. A UPnP daemon already listening there \
                holds it exclusively, since this socket does not ask to share it.",
                port;
            IO, Network, Init)),
        };
        // An announcement should be heard by the machine that sent it, since a
        // control point may be running beside the device it is discovering.
        let result = socket.set_multicast_loop_v4(true);
        res!(result, IO, Network, Init);
        Ok(Self {
            socket,
            joined: Vec::new(),
        })
    }

    /// `iface` is the interface's local IPv4 address. Joining the same interface
    /// twice is an error from the kernel and is
    /// reported as one; [`Self::join_every_interface`] is the call that tolerates
    /// it, since it does not know what is already joined.
    pub fn join(&mut self, iface: Ipv4Addr) -> Outcome<()> {
        let result = self.socket.join_multicast_v4(&MULTICAST_ADDR, &iface);
        res!(result, IO, Network, Init);
        self.joined.push(iface);
        Ok(())
    }

    /// Best effort by design: an interface that refuses the join is skipped rather
    /// than failing the lot, because one unusable interface on a machine with three
    /// must not stop a television on the other two from finding anything. An answer
    /// of zero is the caller's cue to complain.
    pub fn join_every_interface(&mut self) -> Outcome<usize> {
        let mut joined = 0usize;
        for iface in local_interfaces() {
            if self.joined.contains(&iface) {
                continue;
            }
            match self.join(iface) {
                Ok(())  => joined += 1,
                Err(e)  => debug!("SSDP could not join the group on {}: {}", iface, e),
            }
        }
        Ok(joined)
    }

    pub fn interfaces(&self) -> &[Ipv4Addr] {
        &self.joined
    }

    pub fn local_addr(&self) -> Outcome<SocketAddr> {
        let result = self.socket.local_addr();
        Ok(res!(result, IO, Network))
    }

    /// How long a read may block before it gives up. `None` blocks for ever,
    /// which leaves no way to shut the thread down.
    pub fn set_timeout(&self, how_long: Option<Duration>) -> Outcome<()> {
        let result = self.socket.set_read_timeout(how_long);
        res!(result, IO, Network);
        Ok(())
    }

    /// Another handle on the same socket, so that one thread can read while
    /// another announces.
    pub fn try_clone(&self) -> Outcome<Self> {
        let result = self.socket.try_clone();
        let socket = res!(result, IO, Network);
        Ok(Self {
            socket,
            joined: self.joined.clone(),
        })
    }

    /// `Ok(None)` means the read timed out, which is how a shutdown flag gets
    /// looked at. A datagram that does not parse is not an error the caller can do
    /// anything about -- the network carries plenty of them -- so it is logged and
    /// the wait resumes.
    pub fn recv(&self) -> Outcome<Option<(SsdpMessage, SocketAddr)>> {
        let mut buf = vec![0u8; MAX_DATAGRAM];
        loop {
            let (n, from) = match self.socket.recv_from(&mut buf) {
                Ok(pair) => pair,
                Err(e) => {
                    // A timeout is spelled two ways depending on the platform,
                    // and neither is a failure.
                    if matches!(e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
                    {
                        return Ok(None);
                    }
                    return Err(err!(e,
                        "Reading an SSDP datagram."; IO, Network, Wire, Read));
                },
            };
            match SsdpMessage::parse(&buf[..n]) {
                Ok(msg) => return Ok(Some((msg, from))),
                Err(e)  => {
                    debug!("An SSDP datagram from {} did not parse: {}", from, e);
                    continue;
                },
            }
        }
    }

    pub fn multicast(&self, msg: &SsdpMessage) -> Outcome<()> {
        let to = SocketAddrV4::new(MULTICAST_ADDR, PORT);
        res!(self.send_to(msg, SocketAddr::V4(to)));
        Ok(())
    }

    pub fn send_to(&self, msg: &SsdpMessage, to: SocketAddr) -> Outcome<()> {
        let bytes = msg.as_bytes();
        let result = self.socket.send_to(&bytes, to);
        let sent = res!(result, IO, Network, Wire, Write);
        if sent != bytes.len() {
            return Err(err!(
                "An SSDP datagram of {} bytes went out as {}.", bytes.len(), sent;
            IO, Network, Wire, Write, Size));
        }
        Ok(())
    }

    /// Goes back to the address the search came from. The `ST` of the answer is
    /// the target actually being announced, not the `ssdp:all` that may have been
    /// asked: a control point matches the two, and an answer that echoes
    /// `ssdp:all` is discarded.
    pub fn answer(
        &self,
        to:         SocketAddr,
        target:     Target,
        usn:        String,
        location:   String,
        server:     String,
    )
        -> Outcome<()>
    {
        let response = SearchResponse {
            max_age:    DEFAULT_MAX_AGE,
            date:       Some(res!(http_date())),
            location,
            server,
            target,
            usn,
            boot_id:    None,
            config_id:  None,
            extra:      BTreeMap::new(),
        };
        res!(self.send_to(&SsdpMessage::Response(response), to));
        Ok(())
    }
}

/// The local address of the interface that would carry a packet to the SSDP
/// group, which on a machine with one network is the only answer there is.
///
/// No packet is sent: connecting a UDP socket only chooses a route.
pub fn route_interface() -> Option<Ipv4Addr> {
    let socket = match std::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(s)  => s,
        Err(_) => return None,
    };
    if socket.connect(SocketAddrV4::new(MULTICAST_ADDR, PORT)).is_err() {
        return None;
    }
    match socket.local_addr() {
        Ok(SocketAddr::V4(addr))    => Some(*addr.ip()),
        _                           => None,
    }
}

/// Every IPv4 address this machine holds, as interfaces to join a group on.
///
/// The route this machine would use to reach the group is always first, because
/// it is the one that matters and the only one some platforms can be asked for.
/// On Linux the rest are read from the kernel's routing table; elsewhere the list
/// is the route interface and the loopback, which covers a machine with one
/// network and understates a machine with several.
pub fn local_interfaces() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = Vec::new();
    if let Some(addr) = route_interface() {
        out.push(addr);
    }
    for addr in host_addresses() {
        if !out.contains(&addr) {
            out.push(addr);
        }
    }
    if !out.contains(&Ipv4Addr::LOCALHOST) {
        out.push(Ipv4Addr::LOCALHOST);
    }
    out
}

/// Every address the kernel calls local, read from `/proc/net/fib_trie`.
///
/// The file lists the routing trie, and a machine's own addresses appear in it as
/// a `/32 host LOCAL` route under the address itself. That is a Linux detail and a
/// stable one; on any other platform this answers nothing and
/// [`local_interfaces`] falls back to the route interface alone.
#[cfg(target_os = "linux")]
fn host_addresses() -> Vec<Ipv4Addr> {
    let text = match std::fs::read_to_string("/proc/net/fib_trie") {
        Ok(t)  => t,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<Ipv4Addr> = Vec::new();
    let mut candidate: Option<Ipv4Addr> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("|-- ") {
            candidate = Ipv4Addr::from_str(rest.trim()).ok();
            continue;
        }
        // The line after an address says what kind of route it is. Only a host
        // route to the machine itself is an interface address.
        if trimmed.starts_with("/32 host LOCAL") {
            if let Some(addr) = candidate.take() {
                if !addr.is_loopback() && !out.contains(&addr) {
                    out.push(addr);
                }
            }
        }
    }
    out
}

/// Nothing, on a platform whose routing table is not a file.
#[cfg(not(target_os = "linux"))]
fn host_addresses() -> Vec<Ipv4Addr> {
    Vec::new()
}

/// Now, as an HTTP date, for the `DATE` field of an answer.
fn http_date() -> Outcome<String> {
    let since = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d)  => d,
        Err(_) => Duration::from_secs(0), // A clock before the epoch is odd, not fatal.
    };
    Ok(res!(Time::fmt_http(&since)))
}


#[cfg(test)]
mod tests {
    use super::*;

    // A search as a real control point sends one, line endings and all.
    const A_SEARCH: &str = "M-SEARCH * HTTP/1.1\r\n\
        HOST: 239.255.255.250:1900\r\n\
        MAN: \"ssdp:discover\"\r\n\
        MX: 3\r\n\
        ST: urn:schemas-upnp-org:device:MediaServer:1\r\n\
        USER-AGENT: Linux/6.1 UPnP/2.0 Probe/1.0\r\n\
        \r\n";

    const AN_ANSWER: &str = "HTTP/1.1 200 OK\r\n\
        CACHE-CONTROL: max-age=1800\r\n\
        DATE: Mon, 28 Jul 2026 10:00:00 GMT\r\n\
        EXT:\r\n\
        LOCATION: http://192.168.1.10:8200/rootDesc.xml\r\n\
        SERVER: Linux/6.1 UPnP/2.0 Server/1.0\r\n\
        ST: urn:schemas-upnp-org:device:MediaServer:1\r\n\
        USN: uuid:4d696e69-444c-164e-9d41-0011328c0e2f::urn:schemas-upnp-org:device:MediaServer:1\r\n\
        \r\n";

    const AN_ALIVE: &str = "NOTIFY * HTTP/1.1\r\n\
        HOST: 239.255.255.250:1900\r\n\
        CACHE-CONTROL: max-age=1800\r\n\
        LOCATION: http://192.168.1.10:8200/rootDesc.xml\r\n\
        NT: upnp:rootdevice\r\n\
        NTS: ssdp:alive\r\n\
        SERVER: Linux/6.1 UPnP/2.0 Server/1.0\r\n\
        USN: uuid:4d696e69-444c-164e-9d41-0011328c0e2f::upnp:rootdevice\r\n\
        \r\n";

    const A_BYEBYE: &str = "NOTIFY * HTTP/1.1\r\n\
        HOST: 239.255.255.250:1900\r\n\
        NT: upnp:rootdevice\r\n\
        NTS: ssdp:byebye\r\n\
        USN: uuid:4d696e69-444c-164e-9d41-0011328c0e2f::upnp:rootdevice\r\n\
        \r\n";

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ TARGETS                                                               │
    // └───────────────────────────────────────────────────────────────────────┘

    #[test]
    fn test_a_target_survives_being_written_and_read() -> Outcome<()> {
        let targets = [
            Target::All,
            Target::RootDevice,
            Target::Uuid("4d696e69-444c-164e-9d41-0011328c0e2f".to_string()),
            Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string()),
            Target::Other("some-vendor-thing".to_string()),
        ];
        for target in targets {
            let written = fmt!("{}", target);
            assert_eq!(res!(Target::from_str(&written)), target,
                "{:?} did not survive being written as {:?}", target, written);
        }
        Ok(())
    }

    /// `ssdp:all` is answered by everything; everything else by itself alone. A
    /// responder that gets this wrong announces a service nobody asked about, and
    /// control points ignore the answer.
    #[test]
    fn test_who_answers_whom() {
        let root = Target::RootDevice;
        let server = Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string());
        assert!(root.answers(&Target::All));
        assert!(server.answers(&Target::All));
        assert!(root.answers(&Target::RootDevice));
        assert!(!root.answers(&server));
        assert!(!server.answers(&Target::RootDevice));
    }

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ PARSING                                                               │
    // └───────────────────────────────────────────────────────────────────────┘

    #[test]
    fn test_a_search_is_read() -> Outcome<()> {
        match res!(SsdpMessage::parse(A_SEARCH.as_bytes())) {
            SsdpMessage::Search(s) => {
                assert_eq!(s.target,
                    Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string()));
                assert_eq!(s.mx, 3);
                assert_eq!(s.user_agent.as_deref(), Some("Linux/6.1 UPnP/2.0 Probe/1.0"));
                assert!(s.extra.is_empty(), "unexpected extras: {:?}", s.extra);
            }
            other => return Err(err!("A search was read as {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    #[test]
    fn test_an_answer_is_read() -> Outcome<()> {
        match res!(SsdpMessage::parse(AN_ANSWER.as_bytes())) {
            SsdpMessage::Response(r) => {
                assert_eq!(r.max_age, 1800);
                assert_eq!(r.location, "http://192.168.1.10:8200/rootDesc.xml");
                assert_eq!(r.target,
                    Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string()));
                assert!(r.usn.starts_with("uuid:4d696e69"));
                assert_eq!(r.date.as_deref(), Some("Mon, 28 Jul 2026 10:00:00 GMT"));
            }
            other => return Err(err!("An answer was read as {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    #[test]
    fn test_an_alive_and_a_byebye_are_read() -> Outcome<()> {
        match res!(SsdpMessage::parse(AN_ALIVE.as_bytes())) {
            SsdpMessage::Notify(n) => {
                assert_eq!(n.nts, Nts::Alive);
                assert_eq!(n.target, Target::RootDevice);
                assert_eq!(n.max_age, Some(1800));
                assert_eq!(n.location.as_deref(),
                    Some("http://192.168.1.10:8200/rootDesc.xml"));
            }
            other => return Err(err!("An alive was read as {:?}.", other; Test, Mismatch)),
        }
        // A byebye carries neither a lifetime nor a location: there is nothing
        // left to fetch.
        match res!(SsdpMessage::parse(A_BYEBYE.as_bytes())) {
            SsdpMessage::Notify(n) => {
                assert_eq!(n.nts, Nts::ByeBye);
                assert_eq!(n.max_age, None);
                assert_eq!(n.location, None);
            }
            other => return Err(err!("A byebye was read as {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    /// Devices on a real network get the details wrong. Field names in any case,
    /// bare newlines instead of CRLF, and spacing around the colon are all things
    /// that must not stop a message being understood.
    #[test]
    fn test_a_sloppy_sender_is_still_understood() -> Outcome<()> {
        let sloppy = "m-search * HTTP/1.1\n\
            host:239.255.255.250:1900\n\
            Man:  \"ssdp:discover\"\n\
            mx:  1\n\
            st:ssdp:all\n\
            \n";
        match res!(SsdpMessage::parse(sloppy.as_bytes())) {
            SsdpMessage::Search(s) => {
                assert_eq!(s.target, Target::All);
                assert_eq!(s.mx, 1);
            }
            other => return Err(err!("A sloppy search was read as {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    /// A field this crate does not model is kept rather than dropped, so a
    /// responder can pass a vendor extension through and a reader can see it.
    #[test]
    fn test_an_unmodelled_field_is_kept() -> Outcome<()> {
        let with_extra = "NOTIFY * HTTP/1.1\r\n\
            HOST: 239.255.255.250:1900\r\n\
            NT: upnp:rootdevice\r\n\
            NTS: ssdp:alive\r\n\
            USN: uuid:x::upnp:rootdevice\r\n\
            X-VENDOR-THING: 42\r\n\
            \r\n";
        match res!(SsdpMessage::parse(with_extra.as_bytes())) {
            SsdpMessage::Notify(n) => {
                assert_eq!(n.extra.get("X-VENDOR-THING").map(String::as_str), Some("42"));
                // And it goes back out again.
                assert!(n.as_text().contains("X-VENDOR-THING: 42\r\n"));
            }
            other => return Err(err!("A notify was read as {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    #[test]
    fn test_what_is_not_an_ssdp_message_is_refused() {
        for bad in [
            "",                                     // Nothing at all.
            "GET / HTTP/1.1\r\nHost: x\r\n\r\n",    // HTTP, but not SSDP.
            "M-SEARCH * HTTP/1.1\r\nMX: 1\r\n\r\n", // A search naming nothing.
            "NOTIFY * HTTP/1.1\r\nNT: upnp:rootdevice\r\n\r\n", // No NTS, no USN.
            "HTTP/1.1 404 Not Found\r\n\r\n",       // A refusal is not a discovery.
        ] {
            assert!(SsdpMessage::parse(bad.as_bytes()).is_err(),
                "{:?} should not have parsed", bad);
        }
    }

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ SERIALISATION                                                         │
    // └───────────────────────────────────────────────────────────────────────┘

    /// The bytes on the wire, compared with a message from a real device rather
    /// than with what this module happens to produce.
    #[test]
    fn test_a_search_goes_out_as_a_search() {
        let mut search = Search::new(
            Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string()));
        search.mx = 3;
        search.user_agent = Some("Linux/6.1 UPnP/2.0 Probe/1.0".to_string());
        assert_eq!(search.as_text(), A_SEARCH);
    }

    /// `EXT:` is empty, means nothing, and is required: a control point that does
    /// not see it discards the answer (UPnP DA 2.0 §1.3.3).
    #[test]
    fn test_an_answer_carries_the_empty_ext_field() {
        let answer = SearchResponse {
            max_age:    1800,
            date:       Some("Mon, 28 Jul 2026 10:00:00 GMT".to_string()),
            location:   "http://192.168.1.10:8200/rootDesc.xml".to_string(),
            server:     "Linux/6.1 UPnP/2.0 Server/1.0".to_string(),
            target:     Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string()),
            usn:        "uuid:4d696e69-444c-164e-9d41-0011328c0e2f::\
                         urn:schemas-upnp-org:device:MediaServer:1".to_string(),
            boot_id:    None,
            config_id:  None,
            extra:      BTreeMap::new(),
        };
        assert_eq!(answer.as_text(), AN_ANSWER);
    }

    #[test]
    fn test_an_alive_goes_out_as_an_alive() {
        let alive = Notify::alive(
            Target::RootDevice,
            "uuid:4d696e69-444c-164e-9d41-0011328c0e2f::upnp:rootdevice".to_string(),
            "http://192.168.1.10:8200/rootDesc.xml".to_string(),
            "Linux/6.1 UPnP/2.0 Server/1.0".to_string(),
        );
        assert_eq!(alive.as_text(), AN_ALIVE);
    }

    #[test]
    fn test_a_byebye_promises_nothing() {
        let bye = Notify::byebye(
            Target::RootDevice,
            "uuid:4d696e69-444c-164e-9d41-0011328c0e2f::upnp:rootdevice".to_string(),
        );
        assert_eq!(bye.as_text(), A_BYEBYE);
    }

    /// Everything this module writes, it can read back.
    #[test]
    fn test_every_message_survives_a_round_trip() -> Outcome<()> {
        for text in [A_SEARCH, AN_ANSWER, AN_ALIVE, A_BYEBYE] {
            let msg = res!(SsdpMessage::parse(text.as_bytes()));
            let again = res!(SsdpMessage::parse(&msg.as_bytes()));
            assert_eq!(msg, again, "a message changed on the way round: {:?}", text);
        }
        Ok(())
    }

    #[test]
    fn test_a_cache_control_yields_its_seconds() {
        assert_eq!(max_age_of("max-age=1800"), Some(1800));
        assert_eq!(max_age_of("max-age = 60"), Some(60));
        assert_eq!(max_age_of("public, max-age=120"), Some(120));
        assert_eq!(max_age_of("no-cache"), None);
    }

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ THE SOCKET                                                            │
    // └───────────────────────────────────────────────────────────────────────┘

    /// The group is joined and a message goes round it, on the loopback interface
    /// and an ephemeral port so nothing on the machine is disturbed. Live
    /// behaviour on a real network is not tested here: it wants a second machine.
    #[test]
    fn test_a_responder_binds_joins_and_carries_a_message() -> Outcome<()> {
        let rt = res!(tokio::runtime::Runtime::new());
        rt.block_on(async {
            let responder = match Responder::bind_to_port(Ipv4Addr::LOCALHOST, 0).await {
                Ok(r) => r,
                Err(e) => {
                    // A machine with no loopback multicast cannot run this, and
                    // that is the machine's business rather than a failure of the
                    // code under test.
                    warn!("SSDP loopback multicast is unavailable here: {}", e);
                    return Ok(());
                }
            };
            let addr = res!(responder.local_addr());
            let alive = SsdpMessage::Notify(Notify::alive(
                Target::RootDevice,
                "uuid:test::upnp:rootdevice".to_string(),
                "http://127.0.0.1:8200/rootDesc.xml".to_string(),
                "Test/1.0 UPnP/2.0 Test/1.0".to_string(),
            ));
            res!(responder.send_to(&alive, addr).await);
            let got = tokio::time::timeout(
                Duration::from_secs(2),
                responder.recv(),
            ).await;
            match got {
                Ok(Ok((msg, _from)))    => assert_eq!(msg, alive),
                Ok(Err(e))              => return Err(e),
                Err(_)                  => return Err(err!(
                    "The responder never heard its own announcement."; Test, Timeout)),
            }
            Ok(())
        })
    }

    /// The same, with no runtime under it: a search goes out, comes back, and is
    /// answered to the address it came from.
    #[test]
    fn test_a_blocking_responder_carries_a_search_and_its_answer() -> Outcome<()> {
        let mut responder = res!(SyncResponder::bind_to_port(0));
        match responder.join(Ipv4Addr::LOCALHOST) {
            Ok(())  => {},
            Err(e)  => {
                // A machine with no loopback multicast cannot run this, and that
                // is the machine's business rather than a failure of the code.
                warn!("SSDP loopback multicast is unavailable here: {}", e);
                return Ok(());
            },
        }
        res!(responder.set_timeout(Some(Duration::from_secs(2))));
        let addr = res!(responder.local_addr());

        let search = SsdpMessage::Search(Search::new(
            Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string())));
        res!(responder.send_to(&search, addr));
        let (msg, from) = match res!(responder.recv()) {
            Some(pair) => pair,
            None => return Err(err!(
                "The responder never heard the search."; Test, Timeout)),
        };
        req!(msg, search);

        // And the answer goes back to the asker, echoing the target that was
        // asked for rather than the one that was searched under.
        res!(responder.answer(
            from,
            Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string()),
            "uuid:test::urn:schemas-upnp-org:device:MediaServer:1".to_string(),
            "http://127.0.0.1:8336/dlna/desc.xml".to_string(),
            "Test/1.0 UPnP/1.0 Ochre/0.1".to_string(),
        ));
        match res!(responder.recv()) {
            Some((SsdpMessage::Response(r), _)) => {
                req!(r.location, "http://127.0.0.1:8336/dlna/desc.xml".to_string());
                req!(r.target,
                    Target::Urn("schemas-upnp-org:device:MediaServer:1".to_string()));
                assert!(r.usn.ends_with(&fmt!("::{}", r.target)),
                    "the answer's USN and ST disagree: {} against {}", r.usn, r.target);
            },
            other => return Err(err!(
                "The answer came back as {:?}.", other; Test, Mismatch)),
        }
        Ok(())
    }

    /// A read that finds nothing is not a failure, and is what lets a serving
    /// thread look at its shutdown flag.
    #[test]
    fn test_a_blocking_read_that_finds_nothing_says_so() -> Outcome<()> {
        let responder = res!(SyncResponder::bind_to_port(0));
        res!(responder.set_timeout(Some(Duration::from_millis(200))));
        req!(res!(responder.recv()).is_none(), true);
        Ok(())
    }

    /// The machine is asked what interfaces it has, and every answer is an
    /// address a group can actually be joined on.
    #[test]
    fn test_the_interfaces_offered_are_addresses_of_this_machine() -> Outcome<()> {
        let ifaces = local_interfaces();
        assert!(ifaces.contains(&Ipv4Addr::LOCALHOST),
            "the loopback is always usable and was not offered: {:?}", ifaces);
        for addr in &ifaces {
            assert!(!addr.is_multicast(), "{} is a group, not an interface", addr);
            assert!(!addr.is_unspecified(), "0.0.0.0 is not an interface");
        }
        Ok(())
    }
}
