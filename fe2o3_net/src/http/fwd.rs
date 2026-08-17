//! The forwarding headers a proxy hop owns, and who is allowed to speak them.
//!
//! A reverse proxy sits in front of upstream applications and tells each one where the request came
//! from. It does that with `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Host` and the RFC
//! 7239 `Forwarded` field. Those four are assertions about the hop, and an assertion about the hop
//! is only worth anything if the hop is the one making it.
//!
//! Nothing stops a caller sending its own. If a caller's copy is forwarded and the hop's is appended
//! after it, the upstream receives two values -- the caller's first, the hop's second -- and the
//! obvious way to read a header returns the first. [`HeaderFields::get_one`] returns `list[0]`, so an
//! upstream doing the obvious thing reads whatever the caller invented.
//! [`HeaderFields::get_last`](crate::http::fields::HeaderFields::get_last) is the reader that
//! belongs with these headers, and it exists because of this module.
//!
//! What that costs is not a weaker limit but no limit at all. An address guard keyed on the first
//! `X-Forwarded-For` counts a fresh allowance for every fresh invented address, while looking
//! configured. An upstream reading the first `X-Forwarded-Proto` believes a TLS request arrived in
//! plaintext, and one that redirects plaintext to HTTPS on that basis loops.
//!
//! So this module strips all four from the caller before the hop appends its own -- unless the
//! immediate peer is a configured trusted proxy, in which case the caller's chain is preserved and
//! the hop's value appended to it, which is what makes a CDN work. See [`ForwardedPolicy`].
//!
//! The invariant every reader may rely on: **this hop's own value is last, under either policy.**
//! With an untrusted peer there is exactly one value and it is the hop's; with a trusted peer the
//! caller's chain is kept and the hop's is appended after it. "Read the last value" is therefore a
//! correct instruction for an upstream whatever the policy says.
//!
//! [`HeaderFields::get_one`]: crate::http::fields::HeaderFields::get_one
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::http::msg::HttpMessage;

use oxedyne_fe2o3_core::prelude::*;

use std::net::{
    IpAddr,
    SocketAddr,
};


// The forwarding headers a hop owns, lowercased for comparison.  A caller's copy
// of any of them is dropped unless the peer is trusted, because each is a claim
// about the hop the request took and only the hop can make it honestly.
pub const FORWARDED_HEADERS: [&str; 4] = [
    "x-forwarded-for",
    "x-forwarded-proto",
    "x-forwarded-host",
    "forwarded",
];

// The headers a hop rewrites for itself, lowercased for comparison.  `Host` names
// the upstream rather than the caller's original; `Connection` and
// `Transfer-Encoding` are hop-by-hop; `Content-Length` is recomputed from the body
// actually sent.  Passing a caller's `Transfer-Encoding` across a hop that does not
// re-chunk is a request smuggling primitive, which is why it is on this list
// rather than left to each call site.
pub const MANAGED_HEADERS: [&str; 4] = [
    "host",
    "connection",
    "content-length",
    "transfer-encoding",
];

/// Is this one of the forwarding headers a hop owns?
///
/// The comparison is ASCII case-insensitive. Header names are one name whatever their case, and a
/// case-sensitive test here is how a caller gets a forged `X-FORWARDED-FOR` past the strip.
pub fn is_forwarded_header(name: &str) -> bool {
    FORWARDED_HEADERS.iter().any(|held| name.eq_ignore_ascii_case(held))
}

/// Is this one of the headers a hop rewrites for itself?
///
/// Case-insensitive, for the same reason as [`is_forwarded_header`].
pub fn is_managed_header(name: &str) -> bool {
    MANAGED_HEADERS.iter().any(|held| name.eq_ignore_ascii_case(held))
}

/// A peer whose forwarding headers are believed.
///
/// Written in configuration either as a bare address, `198.51.100.7`, or as a prefix,
/// `198.51.100.0/24`. A content delivery network publishes its egress as prefixes, so a form that
/// only accepts single addresses would need hundreds of lines to express one CDN.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrustedPeer {
    Addr(IpAddr),    // one exact address
    Prefix {         // every address sharing the leading `bits` of `base`
        base: IpAddr,
        bits: u8,
    },
}

impl TrustedPeer {
    /// One configuration entry, written either `addr` or `addr/bits`.
    pub fn parse(entry: &str) -> Outcome<Self> {
        let entry = entry.trim();
        match entry.rsplit_once('/') {
            Some((addr, bits)) => {
                let base = res!(addr.parse::<IpAddr>().map_err(|e| err!(e,
                    "Trusted proxy: '{}' is not an IP address.", addr;
                    Invalid, Input, Decode)));
                let bits = res!(bits.parse::<u8>().map_err(|e| err!(e,
                    "Trusted proxy: '{}' is not a prefix length.", bits;
                    Invalid, Input, Decode)));
                let max = match base {
                    IpAddr::V4(_) => 32,
                    IpAddr::V6(_) => 128,
                };
                if bits > max {
                    return Err(err!(
                        "Trusted proxy: prefix length {} exceeds the {} bits of '{}'.",
                        bits, max, base;
                        Invalid, Input, TooBig));
                }
                Ok(Self::Prefix { base, bits })
            }
            None => {
                let addr = res!(entry.parse::<IpAddr>().map_err(|e| err!(e,
                    "Trusted proxy: '{}' is neither an IP address nor a prefix.", entry;
                    Invalid, Input, Decode)));
                Ok(Self::Addr(addr))
            }
        }
    }

    /// Does this entry cover the given address?
    pub fn covers(&self, addr: &IpAddr) -> bool {
        match self {
            Self::Addr(held)            => held == addr,
            Self::Prefix { base, bits } => prefix_covers(base, *bits, addr),
        }
    }
}

/// Do `base` and `addr` share their leading `bits`?
///
/// A v4 prefix never covers a v6 address and vice versa: an operator who writes both means both,
/// and silently widening one family into the other is how a prefix ends up covering more than it
/// says.
fn prefix_covers(
    base:   &IpAddr,
    bits:   u8,
    addr:   &IpAddr,
)
    -> bool
{
    let (base_bytes, addr_bytes): (Vec<u8>, Vec<u8>) = match (base, addr) {
        (IpAddr::V4(b), IpAddr::V4(a)) => (b.octets().to_vec(), a.octets().to_vec()),
        (IpAddr::V6(b), IpAddr::V6(a)) => (b.octets().to_vec(), a.octets().to_vec()),
        _ => return false,
    };
    let whole = (bits / 8) as usize;
    let spare = bits % 8;
    if base_bytes[..whole] != addr_bytes[..whole] {
        return false;
    }
    if spare == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - spare);
    (base_bytes[whole] & mask) == (addr_bytes[whole] & mask)
}

/// Which immediate peers are believed when they speak the forwarding headers.
///
/// Empty means nobody, which means the caller's copies are always stripped. That is the default and
/// it is correct for a host that faces the public directly: nothing in front of the proxy means
/// nothing in front of the proxy is entitled to name the client.
///
/// It stops being correct the day something does sit in front. Stripping unconditionally would then
/// discard the real client address rather than preserve it, replacing every client with the CDN's
/// egress -- a security fix turned into a quieter bug. Hence the policy rather than a constant.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ForwardedPolicy {
    trusted: Vec<TrustedPeer>,    // their forwarding headers are preserved
}

impl ForwardedPolicy {
    /// A policy that trusts nobody, so every caller's forwarding headers are stripped.
    pub fn none() -> Self {
        Self { trusted: Vec::new() }
    }

    /// Build a policy from configuration entries, each a bare address or a prefix.
    ///
    /// An entry that will not parse is an error rather than an entry quietly skipped: a skipped
    /// entry leaves an allow-list that looks populated and trusts nobody, or an operator who
    /// believes their CDN is named here when it is not.
    pub fn new(entries: &[String]) -> Outcome<Self> {
        let mut trusted = Vec::with_capacity(entries.len());
        for entry in entries {
            trusted.push(res!(TrustedPeer::parse(entry)));
        }
        Ok(Self { trusted })
    }

    /// Does this policy name anybody at all?
    pub fn is_empty(&self) -> bool {
        self.trusted.is_empty()
    }

    /// Is the immediate peer one whose forwarding headers are believed?
    ///
    /// A v4 address arriving on a dual-stack listener is reported as the v4-mapped v6 address
    /// `::ffff:a.b.c.d`, which would never match a v4 entry written the obvious way, so it is
    /// unmapped before the comparison.
    pub fn trusts(&self, peer: &SocketAddr) -> bool {
        let addr = match peer.ip() {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4)    => IpAddr::V4(v4),
                None        => IpAddr::V6(v6),
            },
            other => other,
        };
        self.trusted.iter().any(|held| held.covers(&addr))
    }
}

/// Is this a `Host` value safe to repeat inside a header a hop writes?
///
/// The caller chose it, so it is quoted into `Forwarded` and repeated in `X-Forwarded-Host`.
/// Anything outside the host grammar is dropped rather than escaped: a value that cannot be a host
/// is not one worth passing on, and dropping it is the only outcome with no way to be wrong.
fn is_safe_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.chars().all(|c|
            c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '[' | ']' | '_'))
}

/// Copy the caller's headers into `req`, then append this hop's own forwarding headers.
///
/// The caller's `Host`, `Connection`, `Content-Length` and `Transfer-Encoding` are never copied --
/// this hop writes its own. The four forwarding headers are copied only when `policy` trusts
/// `peer`; otherwise they are dropped, so the values this function appends are the only ones the
/// upstream sees.
///
/// This hop's own values go last in every case, which is what makes "read the last value" a correct
/// instruction for an upstream whatever the policy says. The reader that does it is
/// [`HeaderFields::get_last`](crate::http::fields::HeaderFields::get_last).
///
/// The scheme written is `https`: these builders serve a TLS listener, which is the only listener
/// entitled to say so.
pub fn write_forwarded_headers(
    req:        &mut String,
    request:    &HttpMessage,
    peer:       &SocketAddr,
    policy:     &ForwardedPolicy,
) {
    let trusted = policy.trusts(peer);
    let mut caller_host: Option<String> = None;

    for (name, values) in request.header.fields.iter() {
        let name_str = fmt!("{}", name);
        if is_managed_header(&name_str) {
            if name_str.eq_ignore_ascii_case("host") {
                caller_host = values.first().map(|value| fmt!("{}", value));
            }
            continue;
        }
        if !trusted && is_forwarded_header(&name_str) {
            continue;
        }
        for value in values {
            req.push_str(&fmt!("{}: {}\r\n", name_str, value));
        }
    }

    // This hop's own account of the request, appended after anything preserved above.
    req.push_str(&fmt!("X-Forwarded-For: {}\r\n", peer));
    req.push_str("X-Forwarded-Proto: https\r\n");
    let host = match caller_host {
        Some(ref value) if is_safe_host(value) => Some(value.clone()),
        _ => None,
    };
    if let Some(ref value) = host {
        req.push_str(&fmt!("X-Forwarded-Host: {}\r\n", value));
    }
    // RFC 7239 §4. The `for` node identifier carries a port and so must be a quoted string, and a
    // v6 address must be bracketed inside it -- which is exactly how `SocketAddr` prints.
    match host {
        Some(ref value) => req.push_str(&fmt!(
            "Forwarded: for=\"{}\";proto=https;host=\"{}\"\r\n", peer, value)),
        None => req.push_str(&fmt!(
            "Forwarded: for=\"{}\";proto=https\r\n", peer)),
    }
}

/// Build the whole request head a hop sends to an HTTP proxy upstream.
///
/// The returned string is the bytes written to the upstream socket, headers and terminating blank
/// line included. `Connection: close` makes the response termination unambiguous, and
/// `Content-Length` describes the body this hop is about to write rather than the one the caller
/// claimed.
pub fn build_proxy_request_head(
    method:         &str,
    upstream_path:  &str,
    upstream_host:  &str,
    request:        &HttpMessage,
    peer:           &SocketAddr,
    policy:         &ForwardedPolicy,
    body_len:       usize,
)
    -> String
{
    let mut req = String::with_capacity(512 + body_len);
    req.push_str(&fmt!("{} {} HTTP/1.1\r\n", method, upstream_path));
    req.push_str(&fmt!("Host: {}\r\n", upstream_host));
    write_forwarded_headers(&mut req, request, peer, policy);
    req.push_str("Connection: close\r\n");
    req.push_str(&fmt!("Content-Length: {}\r\n", body_len));
    req.push_str("\r\n");
    req
}

/// Build the whole upgrade request head a hop sends to a WebSocket upstream.
///
/// There is no body, so no `Content-Length`; `Connection: Upgrade` replaces the caller's, which is
/// hop-by-hop and never copied.
pub fn build_upgrade_request_head(
    upstream_path:  &str,
    upstream_host:  &str,
    request:        &HttpMessage,
    peer:           &SocketAddr,
    policy:         &ForwardedPolicy,
)
    -> String
{
    let mut req = String::with_capacity(512);
    req.push_str(&fmt!("GET {} HTTP/1.1\r\n", upstream_path));
    req.push_str(&fmt!("Host: {}\r\n", upstream_host));
    write_forwarded_headers(&mut req, request, peer, policy);
    req.push_str("Connection: Upgrade\r\n");
    req.push_str("\r\n");
    req
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Header names are one name whatever their case.
    ///
    /// The wire parser lowercases every name it reads, so a case-sensitive comparison against the
    /// lowercase forms would pass every test driven from bytes and still be wrong for a message
    /// built in code. This checks the predicate itself, which is the only place the property lives.
    #[test]
    fn test_forwarded_header_names_are_case_insensitive_00() {
        for name in [
            "x-forwarded-for",
            "X-Forwarded-For",
            "X-FORWARDED-FOR",
            "x-forwarded-proto",
            "X-Forwarded-Proto",
            "X-FORWARDED-PROTO",
            "x-forwarded-host",
            "X-Forwarded-Host",
            "forwarded",
            "Forwarded",
            "FORWARDED",
        ] {
            assert!(is_forwarded_header(name),
                "'{}' is a forwarding header whatever its case", name);
        }
        assert!(!is_forwarded_header("x-forwarded-for-real"),
            "the match is the whole name, not a prefix");
        assert!(!is_forwarded_header("cookie"));

        for name in ["host", "Host", "HOST", "connection", "Connection",
                     "content-length", "Content-Length", "transfer-encoding", "Transfer-Encoding"] {
            assert!(is_managed_header(name),
                "'{}' is a managed header whatever its case", name);
        }
        assert!(!is_managed_header("x-forwarded-for"));
    }

    /// A bare address covers itself and nothing else.
    #[test]
    fn test_trusted_peer_bare_address_00() -> Outcome<()> {
        let peer = res!(TrustedPeer::parse("198.51.100.7"));
        assert!(peer.covers(&res!("198.51.100.7".parse::<IpAddr>(), Test)));
        assert!(!peer.covers(&res!("198.51.100.8".parse::<IpAddr>(), Test)));
        Ok(())
    }

    /// A prefix covers its range, stops at its edges, and does not cross address families.
    #[test]
    fn test_trusted_peer_prefix_00() -> Outcome<()> {
        let peer = res!(TrustedPeer::parse("198.51.100.0/24"));
        assert!(peer.covers(&res!("198.51.100.0".parse::<IpAddr>(), Test)));
        assert!(peer.covers(&res!("198.51.100.255".parse::<IpAddr>(), Test)));
        assert!(!peer.covers(&res!("198.51.101.0".parse::<IpAddr>(), Test)));

        // A prefix that does not end on a byte boundary, which is where an implementation that
        // only compares whole bytes goes wrong.
        let peer = res!(TrustedPeer::parse("10.1.0.0/20"));
        assert!(peer.covers(&res!("10.1.0.1".parse::<IpAddr>(), Test)));
        assert!(peer.covers(&res!("10.1.15.255".parse::<IpAddr>(), Test)));
        assert!(!peer.covers(&res!("10.1.16.0".parse::<IpAddr>(), Test)),
            "10.1.16.0 is outside a /20 based at 10.1.0.0");

        // Families do not mix.
        let peer = res!(TrustedPeer::parse("2001:db8::/32"));
        assert!(peer.covers(&res!("2001:db8::1".parse::<IpAddr>(), Test)));
        assert!(!peer.covers(&res!("2001:db9::1".parse::<IpAddr>(), Test)));
        assert!(!peer.covers(&res!("10.0.0.1".parse::<IpAddr>(), Test)));

        // Nonsense is a configuration error, not a peer that quietly trusts nobody.
        assert!(TrustedPeer::parse("not-an-address").is_err());
        assert!(TrustedPeer::parse("10.0.0.0/33").is_err());
        assert!(TrustedPeer::parse("10.0.0.0/x").is_err());
        Ok(())
    }

    /// The default policy trusts nobody, and a v4 peer arriving v4-mapped still matches.
    #[test]
    fn test_forwarded_policy_trusts_00() -> Outcome<()> {
        let none = ForwardedPolicy::none();
        assert!(none.is_empty());
        assert!(!none.trusts(&res!("203.0.113.7:4000".parse::<SocketAddr>(), Test)));

        let policy = res!(ForwardedPolicy::new(&[fmt!("198.51.100.0/24")]));
        assert!(policy.trusts(&res!("198.51.100.9:4000".parse::<SocketAddr>(), Test)));
        assert!(!policy.trusts(&res!("203.0.113.7:4000".parse::<SocketAddr>(), Test)));
        assert!(policy.trusts(&res!("[::ffff:198.51.100.9]:4000".parse::<SocketAddr>(), Test)),
            "a dual-stack listener reports a v4 peer as v4-mapped v6");
        Ok(())
    }

    /// An entry that will not parse is refused, not skipped.
    ///
    /// A policy built from a list with one bad entry and the rest good would otherwise trust the
    /// rest, which is an allow-list that reads as populated while missing whatever the typo was.
    #[test]
    fn test_forwarded_policy_refuses_a_bad_entry_00() -> Outcome<()> {
        assert!(ForwardedPolicy::new(&[fmt!("198.51.100.0/24"), fmt!("nonsense")]).is_err());
        let policy = res!(ForwardedPolicy::new(&[fmt!("198.51.100.0/24")]));
        assert!(policy.trusts(&res!("198.51.100.1:80".parse::<SocketAddr>(), Test)));
        Ok(())
    }
}
