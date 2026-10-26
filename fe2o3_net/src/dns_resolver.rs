//! Minimal DNS over UDP client.
//!
//! Implements the small slice of RFC 1035 needed by the SMTP outbound
//! client: A and MX lookups against the first nameserver listed in
//! `/etc/resolv.conf`, with no caching, no recursion fallback, no
//! truncation handling, no DNSSEC.
//!
//! An A lookup answers from the hosts file first, as the system resolver does under
//! `hosts: files dns`: `localhost` and any `*.localhost` are loopback (RFC 6761 6.3), a name the
//! hosts file holds is answered from it alone, and only then is DNS asked. Until 26 Oct 2026 it
//! asked DNS only, so a name that every other program on the machine reached through
//! `/etc/hosts` did not resolve here (found by the mail world, D-20261026-01). [`Resolver`] names
//! a hosts file and a nameserver of the caller's own, for a test or a network namespace.
//!
//! The motivation is to keep Hematite free of a third-party resolver
//! crate. Outbound SMTP needs MX lookups and `std::net` does not expose
//! them; rather than add `hickory-resolver` we own the ~250 lines.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::{
        IpAddr,
        Ipv4Addr,
        SocketAddr,
        UdpSocket,
    },
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};


/// One MX record returned by [`lookup_mx`].
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MxRecord {
    pub preference: u16,       // lower is preferred
    pub exchange:   String,    // host name, trailing dot stripped
}

/// The first nameserver listed in `/etc/resolv.conf`, or `8.8.8.8` where the
/// file is missing or unreadable.
pub fn system_resolver() -> SocketAddr {
    let fallback = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 53);
    let contents = match std::fs::read_to_string("/etc/resolv.conf") {
        Ok(s) => s,
        Err(_) => return fallback,
    };
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("nameserver") {
            let ip_str = rest.trim();
            if let Ok(ip) = ip_str.parse::<IpAddr>() {
                return SocketAddr::new(ip, 53);
            }
        }
    }
    fallback
}

/// Sorted into preference order, lowest first. A successful query that returns
/// no records falls back to a synthetic MX pointing at `domain` itself, per
/// RFC 5321 §5.1.
pub fn lookup_mx(domain: &str) -> Outcome<Vec<MxRecord>> {
    let resolver = system_resolver();
    let response = res!(query(domain, QTYPE_MX, resolver, DNS_TIMEOUT));
    let mut records = res!(parse_mx_response(&response));
    if records.is_empty() {
        records.push(MxRecord {
            preference: 0,
            exchange:   domain.trim_end_matches('.').to_string(),
        });
    }
    records.sort();
    Ok(records)
}

/// Every IPv4 answer, in the order the server sent them, from the hosts file where it holds the
/// name and from DNS otherwise; see [`Resolver::lookup_a`].
///
/// An empty list is a name that exists but has no A record, which is also what an IPv6-only host
/// looks like. A name that does not exist (RCODE 3, NXDOMAIN) is an error tagged `Permanent`, so a
/// caller can tell the two apart: asking again will not make the second one appear, but may well
/// change the first.
pub fn lookup_a(host: &str) -> Outcome<Vec<Ipv4Addr>> {
    Resolver::system().lookup_a(host)
}

pub const HOSTS_FILE:       &str        = "/etc/hosts"; // fixed: the system hosts file, which Resolver::system reads by definition.
pub const DNS_TIMEOUT:      Duration    = Duration::from_secs(5);

/// Where names are looked up: a hosts file, then a nameserver.
#[derive(Clone, Debug)]
pub struct Resolver {
    pub hosts:      PathBuf,        // read on every lookup, as glibc does; a missing file holds nothing
    pub nameserver: SocketAddr,
    pub timeout:    Duration,       // per DNS read, which is tried twice
}

impl Resolver {

    /// `/etc/hosts`, and the first nameserver in `/etc/resolv.conf`.
    pub fn system() -> Self {
        Self {
            hosts:      PathBuf::from(HOSTS_FILE),
            nameserver: system_resolver(),
            timeout:    DNS_TIMEOUT,
        }
    }

    pub fn with_hosts<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.hosts = path.as_ref().to_path_buf();
        self
    }

    pub fn with_nameserver(mut self, ns: SocketAddr) -> Self {
        self.nameserver = ns;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// An address literal is itself; `localhost` and `*.localhost` are loopback; a name the hosts
    /// file holds is answered from it alone, every matching IPv4 line in file order; anything else
    /// is asked of DNS. An unreadable hosts file is one that holds nothing, as it is to glibc.
    pub fn lookup_a(&self, host: &str) -> Outcome<Vec<Ipv4Addr>> {
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            return Ok(vec![ip]);
        }
        let name = host.trim_end_matches('.').to_ascii_lowercase();
        if name == "localhost" || name.ends_with(".localhost") { // fixed: loopback names, RFC 6761.
            return Ok(vec![Ipv4Addr::LOCALHOST]);
        }
        if let Ok(text) = std::fs::read_to_string(&self.hosts) {
            let found = hosts_a(&text, &name);
            if !found.is_empty() {
                return Ok(found);
            }
        }
        let response = res!(query(host, QTYPE_A, self.nameserver, self.timeout));
        parse_a_response(&response)
    }
}

/// The IPv4 addresses a hosts file gives `name` (lower case, no trailing dot), in file order.
/// Each line is an address, a canonical name and any aliases, with `#` starting a comment.
pub fn hosts_a(text: &str, name: &str) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = match line.find('#') {
            Some(i) => &line[..i],
            None    => line,
        };
        let mut words = line.split_whitespace();
        let ip = match words.next().map(|w| w.parse::<Ipv4Addr>()) {
            Some(Ok(ip)) => ip,
            _            => continue, // blank, or an IPv6 line
        };
        if words.any(|w| w.trim_end_matches('.').eq_ignore_ascii_case(name)) && !out.contains(&ip) {
            out.push(ip);
        }
    }
    out
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DNS WIRE FORMAT                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

const QTYPE_A:  u16 = 1;
const QTYPE_MX: u16 = 15;
const QCLASS_IN: u16 = 1;

const FLAG_RD: u16 = 0x0100; // Recursion desired.

const RCODE_NXDOMAIN: u8 = 3;

const RTYPE_A:      u16 = 1;
const RTYPE_NS:     u16 = 2;
const RTYPE_CNAME:  u16 = 5;
const RTYPE_MX:     u16 = 15;

/// The raw response bytes.  One retry on timeout, and no more.
fn query(name: &str, qtype: u16, resolver: SocketAddr, wait: Duration) -> Outcome<Vec<u8>> {
    let id: u16 = (std::process::id() as u16) ^ (qtype as u16);
    let mut packet = Vec::with_capacity(64);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&FLAG_RD.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    packet.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    packet.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    packet.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    encode_qname(name, &mut packet);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&QCLASS_IN.to_be_bytes());

    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => return Err(err!(e,
            "Binding UDP socket for DNS query."; IO, Network, Init)),
    };
    if let Err(e) = socket.set_read_timeout(Some(wait)) {
        return Err(err!(e,
            "Setting DNS read timeout."; IO, Network, Init));
    }
    if let Err(e) = socket.send_to(&packet, resolver) {
        return Err(err!(e,
            "Sending DNS query to {:?}.", resolver; IO, Network, Write));
    }
    let mut buf = [0u8; 4096];
    let (n, _src) = match socket.recv_from(&mut buf) {
        Ok(p) => p,
        Err(_) => {
            // One retry.
            if let Err(e) = socket.send_to(&packet, resolver) {
                return Err(err!(e,
                    "Resending DNS query to {:?}.", resolver;
                    IO, Network, Write));
            }
            match socket.recv_from(&mut buf) {
                Ok(p) => p,
                Err(e) => return Err(err!(e,
                    "Reading DNS response from {:?}.", resolver;
                    IO, Network, Read)),
            }
        }
    };
    Ok(buf[..n].to_vec())
}

/// Length-prefixed labels, then a zero terminator.
fn encode_qname(name: &str, out: &mut Vec<u8>) {
    for label in name.trim_end_matches('.').split('.') {
        let bytes = label.as_bytes();
        out.push(bytes.len() as u8);
        out.extend_from_slice(bytes);
    }
    out.push(0);
}

fn parse_mx_response(buf: &[u8]) -> Outcome<Vec<MxRecord>> {
    let (_id, _rcode, ancount, mut pos) = res!(parse_response_header(buf));
    let mut out = Vec::new();
    for _ in 0..ancount {
        let (rtype, rdlength, rdata_pos, next) = res!(parse_rr(buf, pos));
        pos = next;
        if rtype == RTYPE_MX {
            if rdlength < 3 {
                return Err(err!(
                    "MX RDATA too short ({} bytes).", rdlength;
                    Invalid, Input, Decode));
            }
            let pref = u16::from_be_bytes([
                buf[rdata_pos],
                buf[rdata_pos + 1],
            ]);
            let (name, _consumed) = res!(read_name(buf, rdata_pos + 2));
            out.push(MxRecord {
                preference: pref,
                exchange:   name,
            });
        }
    }
    Ok(out)
}

fn parse_a_response(buf: &[u8]) -> Outcome<Vec<Ipv4Addr>> {
    let (_id, rcode, ancount, mut pos) = res!(parse_response_header(buf));
    if rcode == RCODE_NXDOMAIN {
        return Err(err!(
            "The name does not exist (DNS RCODE 3, NXDOMAIN).";
            IO, Network, Missing, Permanent));
    }
    let mut out = Vec::new();
    for _ in 0..ancount {
        let (rtype, rdlength, rdata_pos, next) = res!(parse_rr(buf, pos));
        pos = next;
        if rtype == RTYPE_A {
            if rdlength != 4 {
                return Err(err!(
                    "A RDATA must be 4 bytes (got {}).", rdlength;
                    Invalid, Input, Decode));
            }
            out.push(Ipv4Addr::new(
                buf[rdata_pos],
                buf[rdata_pos + 1],
                buf[rdata_pos + 2],
                buf[rdata_pos + 3],
            ));
        }
        // CNAME and NS records are silently skipped: lookup_a returns
        // any direct A answers in the same response, which is what
        // resolvers typically include.
        let _ = RTYPE_CNAME;
        let _ = RTYPE_NS;
    }
    Ok(out)
}

/// `(transaction_id, rcode, ancount, position_of_first_answer)`, having read the
/// 12-byte header and the question section. Only RCODEs 0 and 3 are accepted.
fn parse_response_header(buf: &[u8]) -> Outcome<(u16, u8, u16, usize)> {
    if buf.len() < 12 {
        return Err(err!(
            "DNS response too short ({} bytes).", buf.len();
            Invalid, Input, Decode));
    }
    let id      = u16::from_be_bytes([buf[0], buf[1]]);
    let flags   = u16::from_be_bytes([buf[2], buf[3]]);
    let qdcount = u16::from_be_bytes([buf[4], buf[5]]);
    let ancount = u16::from_be_bytes([buf[6], buf[7]]);
    // Bottom 4 bits of the flags are RCODE.
    let rcode = (flags & 0x000f) as u8;
    if rcode != 0 && rcode != 3 {
        return Err(err!(
            "DNS response RCODE = {}.", rcode;
            IO, Network, Wire));
    }
    let mut pos = 12;
    for _ in 0..qdcount {
        let (_qname, consumed) = res!(read_name(buf, pos));
        pos = consumed + 4; // skip QTYPE + QCLASS
    }
    Ok((id, rcode, ancount, pos))
}

/// One resource record header at `pos`, as
/// `(rtype, rdlength, rdata_position, position_after_rdata)`.
fn parse_rr(buf: &[u8], pos: usize) -> Outcome<(u16, u16, usize, usize)> {
    let (_name, after_name) = res!(read_name(buf, pos));
    if after_name + 10 > buf.len() {
        return Err(err!(
            "DNS RR header truncated at offset {}.", after_name;
            Invalid, Input, Decode));
    }
    let rtype     = u16::from_be_bytes([buf[after_name],     buf[after_name + 1]]);
    let _rclass   = u16::from_be_bytes([buf[after_name + 2], buf[after_name + 3]]);
    let _ttl      = u32::from_be_bytes([
        buf[after_name + 4],
        buf[after_name + 5],
        buf[after_name + 6],
        buf[after_name + 7],
    ]);
    let rdlength = u16::from_be_bytes([buf[after_name + 8], buf[after_name + 9]]);
    let rdata_pos = after_name + 10;
    let after_rdata = rdata_pos + rdlength as usize;
    if after_rdata > buf.len() {
        return Err(err!(
            "DNS RR RDATA truncated.";
            Invalid, Input, Decode));
    }
    Ok((rtype, rdlength, rdata_pos, after_rdata))
}

/// Follows the compression pointers of RFC 1035 §4.1.4. The position returned is
/// the one after the *uncompressed* part of the name, which is where the caller
/// resumes reading.
fn read_name(buf: &[u8], start: usize) -> Outcome<(String, usize)> {
    let mut name = String::new();
    let mut pos = start;
    let mut after: Option<usize> = None;
    let mut hops = 0;
    loop {
        if hops > 20 {
            return Err(err!(
                "DNS name compression loop.";
                Invalid, Input, Decode));
        }
        if pos >= buf.len() {
            return Err(err!(
                "DNS name overruns buffer.";
                Invalid, Input, Decode));
        }
        let len = buf[pos];
        if len == 0 {
            pos += 1;
            if after.is_none() {
                after = Some(pos);
            }
            break;
        }
        if len & 0xc0 == 0xc0 {
            // Compression pointer.
            if pos + 1 >= buf.len() {
                return Err(err!(
                    "DNS pointer truncated.";
                    Invalid, Input, Decode));
            }
            let target = (((len & 0x3f) as usize) << 8) | (buf[pos + 1] as usize);
            if after.is_none() {
                after = Some(pos + 2);
            }
            pos = target;
            hops += 1;
            continue;
        }
        if len & 0xc0 != 0 {
            return Err(err!(
                "DNS label length has reserved high bits set.";
                Invalid, Input, Decode));
        }
        let label_end = pos + 1 + len as usize;
        if label_end > buf.len() {
            return Err(err!(
                "DNS label overruns buffer.";
                Invalid, Input, Decode));
        }
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(&buf[pos + 1..label_end]));
        pos = label_end;
    }
    Ok((name, after.unwrap_or(pos)))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A response to an A query for `gone.example`, as a resolver would send it. Each answer is
    /// `(rtype, rdata)`, named by a pointer back to the question.
    fn response(rcode: u8, answers: &[(u16, &[u8])]) -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(&0x1234u16.to_be_bytes());
        b.extend_from_slice(&(0x8180u16 | rcode as u16).to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&(answers.len() as u16).to_be_bytes());
        b.extend_from_slice(&[0, 0, 0, 0]);
        encode_qname("gone.example", &mut b);
        b.extend_from_slice(&QTYPE_A_TEST.to_be_bytes());
        b.extend_from_slice(&QCLASS_IN.to_be_bytes());
        for (rtype, rdata) in answers {
            b.extend_from_slice(&[0xc0, 0x0c]);
            b.extend_from_slice(&rtype.to_be_bytes());
            b.extend_from_slice(&QCLASS_IN.to_be_bytes());
            b.extend_from_slice(&60u32.to_be_bytes());
            b.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
            b.extend_from_slice(rdata);
        }
        b
    }

    const QTYPE_A_TEST:     u16 = 1;
    const RTYPE_AAAA_TEST:  u16 = 28;

    /// The name does not exist. Asking again will not make it.
    #[test]
    fn test_nxdomain_is_a_permanent_error_for_an_a_lookup_00() -> Outcome<()> {
        match parse_a_response(&response(3, &[])) {
            Ok(v) => Err(err!("NXDOMAIN read as an answer: {:?}.", v; Test, Mismatch)),
            Err(e) => {
                req!(true, e.tags().contains(&ErrTag::Permanent), "NXDOMAIN was not tagged permanent");
                Ok(())
            }
        }
    }

    /// The name exists and has no A record. Not an error, and not a verdict on the host.
    #[test]
    fn test_noerror_without_an_a_record_is_an_empty_answer_00() -> Outcome<()> {
        let v = res!(parse_a_response(&response(0, &[])));
        req!(true, v.is_empty(), "NOERROR with no answer was not empty");
        Ok(())
    }

    /// An IPv6-only host: AAAA answered, no A. It is an existing name with no IPv4 address.
    #[test]
    fn test_an_aaaa_only_answer_is_an_empty_a_answer_00() -> Outcome<()> {
        let aaaa = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let v = res!(parse_a_response(&response(0, &[(RTYPE_AAAA_TEST, &aaaa)])));
        req!(true, v.is_empty(), "an AAAA answer was read as an A address");
        Ok(())
    }

    #[test]
    fn test_an_a_answer_is_returned_00() -> Outcome<()> {
        let v = res!(parse_a_response(&response(0, &[(1, &[192, 0, 2, 7])])));
        req!(vec![Ipv4Addr::new(192, 0, 2, 7)], v, "the A address was not returned");
        Ok(())
    }

    // ── The hosts file ───────────────────────────────────────────

    /// A resolver whose nameserver is a port nothing answers on, so that a lookup reaching DNS
    /// fails, and a hosts file written for the test, never the machine's own.
    fn offline(hosts: &str, tag: &str) -> Outcome<(Resolver, std::path::PathBuf)> {
        let path = std::env::temp_dir().join(fmt!("fe2o3_hosts_{}_{}", std::process::id(), tag));
        res!(std::fs::write(&path, hosts)
            .map_err(|e| err!(e, "Writing the test hosts file."; IO, File, Write)));
        let dead = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9);
        Ok((Resolver::system().with_hosts(&path).with_nameserver(dead).with_timeout(
            Duration::from_millis(200)), path))
    }

    /// A name in the hosts file is answered from it and DNS is never asked, as every system
    /// resolver does under `hosts: files dns`.
    #[test]
    fn test_a_hosts_file_entry_wins_over_dns_00() -> Outcome<()> {
        let (r, path) = res!(offline(
            "# test\n127.0.0.1\tlocalhost\n10.9.8.7  mail.world.test  mail # alias\n\
             ::1 mail.world.test\n10.9.8.8 MAIL.world.test.\n", "wins"));
        let v = res!(r.lookup_a("mail.world.test"));
        let _ = std::fs::remove_file(&path);
        req!(vec![Ipv4Addr::new(10, 9, 8, 7), Ipv4Addr::new(10, 9, 8, 8)], v,
            "the hosts file was not answered first, in order and without case");
        Ok(())
    }

    #[test]
    fn test_a_hosts_file_alias_is_a_name_00() -> Outcome<()> {
        let (r, path) = res!(offline("10.9.8.7 mail.world.test mail\n", "alias"));
        let v = res!(r.lookup_a("mail."));
        let _ = std::fs::remove_file(&path);
        req!(vec![Ipv4Addr::new(10, 9, 8, 7)], v, "an alias was not answered");
        Ok(())
    }

    /// `localhost` is loopback (RFC 6761 6.3) with no hosts file at all, and so is an address
    /// literal, which is no name.
    #[test]
    fn test_localhost_and_a_literal_need_no_lookup_00() -> Outcome<()> {
        let (r, path) = res!(offline("", "local"));
        let _ = std::fs::remove_file(&path);
        req!(vec![Ipv4Addr::LOCALHOST], res!(r.lookup_a("localhost")));
        req!(vec![Ipv4Addr::LOCALHOST], res!(r.lookup_a("imap.localhost")));
        req!(vec![Ipv4Addr::new(192, 0, 2, 1)], res!(r.lookup_a("192.0.2.1")));
        Ok(())
    }

    /// A name the hosts file does not hold goes on to DNS, which here does not answer.
    #[test]
    fn test_a_name_not_in_the_hosts_file_asks_dns_00() -> Outcome<()> {
        let (r, path) = res!(offline("10.9.8.7 mail.world.test\n", "dns"));
        let out = r.lookup_a("other.world.test");
        let _ = std::fs::remove_file(&path);
        req!(true, out.is_err(), "a name in no hosts file was answered without DNS: {:?}", out);
        Ok(())
    }
}
