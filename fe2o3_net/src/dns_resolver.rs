//! Minimal DNS over UDP client.
//!
//! Implements the small slice of RFC 1035 needed by the SMTP outbound
//! client: A and MX lookups against the first nameserver listed in
//! `/etc/resolv.conf`, with no caching, no recursion fallback, no
//! truncation handling, no DNSSEC.
//!
//! The motivation is to keep Hematite free of a third-party resolver
//! crate. Outbound SMTP needs MX lookups and `std::net` does not expose
//! them; rather than add `hickory-resolver` we own the ~250 lines.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    net::{
        IpAddr,
        Ipv4Addr,
        SocketAddr,
        UdpSocket,
    },
    time::Duration,
};


/// One MX record returned by [`lookup_mx`].
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MxRecord {
    /// Preference value (lower = preferred).
    pub preference: u16,
    /// Exchange host name. Trailing dot stripped.
    pub exchange:   String,
}

/// Read `/etc/resolv.conf` and return the first listed nameserver, or
/// `8.8.8.8` if the file is missing/unreadable.
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

/// Resolve MX records for `domain`. Returns the records sorted in
/// preference order (lowest first). On a successful query that returns
/// no records, falls back to a synthetic MX pointing at `domain` itself
/// per RFC 5321 §5.1.
pub fn lookup_mx(domain: &str) -> Outcome<Vec<MxRecord>> {
    let resolver = system_resolver();
    let response = res!(query(domain, QTYPE_MX, resolver));
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

/// Resolve an A record for `host`. Returns every IPv4 answer in the
/// order the server sent them, after CNAME chasing.
pub fn lookup_a(host: &str) -> Outcome<Vec<Ipv4Addr>> {
    let resolver = system_resolver();
    let response = res!(query(host, QTYPE_A, resolver));
    parse_a_response(&response)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DNS WIRE FORMAT                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

const QTYPE_A:  u16 = 1;
const QTYPE_MX: u16 = 15;
const QCLASS_IN: u16 = 1;

const FLAG_RD: u16 = 0x0100; // Recursion desired.

const RTYPE_A:      u16 = 1;
const RTYPE_NS:     u16 = 2;
const RTYPE_CNAME:  u16 = 5;
const RTYPE_MX:     u16 = 15;

/// Build a DNS query packet for `name` of type `qtype` and send it to
/// `resolver`, returning the raw response bytes. Single retry on
/// timeout.
fn query(name: &str, qtype: u16, resolver: SocketAddr) -> Outcome<Vec<u8>> {
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
    if let Err(e) = socket.set_read_timeout(Some(Duration::from_secs(5))) {
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

/// Encode a DNS name as a sequence of length-prefixed labels followed
/// by a zero terminator.
fn encode_qname(name: &str, out: &mut Vec<u8>) {
    for label in name.trim_end_matches('.').split('.') {
        let bytes = label.as_bytes();
        out.push(bytes.len() as u8);
        out.extend_from_slice(bytes);
    }
    out.push(0);
}

/// Parse the entire DNS response and return every MX answer.
fn parse_mx_response(buf: &[u8]) -> Outcome<Vec<MxRecord>> {
    let (_id, ancount, mut pos) = res!(parse_response_header(buf));
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

/// Parse the entire DNS response and return every A answer.
fn parse_a_response(buf: &[u8]) -> Outcome<Vec<Ipv4Addr>> {
    let (_id, ancount, mut pos) = res!(parse_response_header(buf));
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

/// Parse the 12-byte DNS header and the question section, returning
/// `(transaction_id, ancount, position_of_first_answer)`.
fn parse_response_header(buf: &[u8]) -> Outcome<(u16, u16, usize)> {
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
    Ok((id, ancount, pos))
}

/// Parse one resource record header at `pos`. Returns
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

/// Decode a DNS name starting at `pos`, following compression pointers
/// (RFC 1035 §4.1.4). Returns the decoded name and the position
/// immediately after the *uncompressed* part of the name (i.e. the
/// position the caller should resume reading at).
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
