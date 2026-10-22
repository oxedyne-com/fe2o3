use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_stds::regions::Country;

use std::{
    convert::TryFrom,
    fmt::{self},
    net::{
        IpAddr,
        Ipv4Addr,
        Ipv6Addr,
    },
};

pub struct PhoneNumbers;

impl PhoneNumbers {
    
    pub fn country_to_prefixes(c: &Country) -> Outcome<Vec<u16>> {
        match c {
            Country::Australia => Ok(vec![61]),
            _ => Err(err!(
                "No prefix defined for country {:?}.", c;
            Invalid, Input, Missing)),
        }
    }

    pub fn prefix_to_country(p: u16) -> Option<Country> {
        match p {
            61 => Some(Country::Australia),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhoneNumber {
    pub prefix: u16,
    pub num:    String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EmailAddress {
    pub loc:    String,
    pub dom:    String,
}

impl TryFrom<&str> for EmailAddress {
    type Error = Error<ErrTag>;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        if s.len() == 0 {
            return Err(err!(
                "Trying to interpret an email address from '{}': \
                length is zero.", s;
            Decode, String, Invalid, Input));
        }
        let mut at = None;
        for (i, c) in s.chars().enumerate() {
            match c {
                '@' => match at {
                    None => at = Some(i),
                    Some(j) => return Err(err!(
                        "Trying to interpret an email address from '{}': '@' \
                        character found at position {} found previously at \
                        position {}.", s, i, j;
                    Decode, String, Invalid, Input)),
                },
                ' ' => return Err(err!(
                    "Trying to interpret an email address from '{}': Space \
                    characters are invalid, space found at position {}.", s, i;
                Decode, String, Invalid, Input)),
                _ => (),
            }
        }
        match at {
            None => Err(err!(
                "Trying to interpret an email address from '{}': '@' \
                character not found.", s;
            Decode, String, Invalid, Input)),
            Some(i) => {
                let (left, right) = s.split_at(i);
                if left.len() == 0 {
                    return Err(err!(
                        "Trying to interpret an email address from '{}': \
                        local (left) part of email address has length \
                        zero.", s;
                    Decode, String, Invalid, Input));
                }
                if right.len() == 0 {
                    return Err(err!(
                        "Trying to interpret an email address from '{}': \
                        domain (right) part of email address has length \
                        zero.", s;
                    Decode, String, Invalid, Input));
                }
                let right = &right[1..];
                match right.find('@') {
                    Some(j) => return Err(err!(
                        "Trying to interpret an email address from '{}': \
                        invalid '@' character found at position {} in the \
                        domain part '{}'.", s, j, right;
                    Decode, String, Invalid, Input)),
                    None => (),
                }
                Ok(EmailAddress {
                    loc: left.to_string(),
                    dom: right.to_string(),
                })
            },
        }
    }
}

/// An address on a Hematite-native overlay identity layer.
///
/// `real` is the underlying backing identity (whatever the overlay
/// treats as the host or account behind the address) and `virt` is
/// the virtual face presented to correspondents. The wire form is
/// `overlay:<real>//<virt>`. Left as a placeholder type until the
/// overlay's address model is nailed down by downstream consumers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OverlayAddress {
    real:    String,
    virt:    String,
}

/// One of the several address shapes a contact can be reached at:
/// phone, email, or a Hematite-native overlay address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContactAddress {
    Phone(PhoneNumber),
    Email(EmailAddress),
    Overlay(OverlayAddress),
}

impl fmt::Display for ContactAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContactAddress::Phone(pn) => write!(f, "+{} {}", pn.prefix, pn.num),
            ContactAddress::Email(email) => write!(f, "{}@{}", email.loc, email.dom),
            ContactAddress::Overlay(addr) => write!(f, "overlay:{}//{}", addr.real, addr.virt),
        }
    }
}

// FINISHME
impl TryFrom<&str> for ContactAddress {
    type Error = Error<ErrTag>;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        let s = s.trim_start();
        if s.starts_with("overlay:") {
            let addr = OverlayAddress::default();
            Ok(ContactAddress::Overlay(addr))
        } else if s.contains('@') {
            let email = res!(EmailAddress::try_from(s));
            Ok(ContactAddress::Email(email))
        } else {
            let pn = PhoneNumber::default();
            Ok(ContactAddress::Phone(pn))
        }
    }

}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_address_decoding_00() -> Outcome<()> {
        let email = "test@my.domain";
        let addr = res!(EmailAddress::try_from(email));
        let expected = EmailAddress { loc: fmt!("test"), dom: fmt!("my.domain") };
        if addr != expected {
            return Err(err!(
                "Decoding of address '{}' should produce '{:?}'.", email, expected;
            Invalid, Input, Decode, String));
        }
        Ok(())
    }

    #[test]
    fn test_email_address_decoding_01() -> Outcome<()> {
        let email = "test@my@domain";
        let result = EmailAddress::try_from(email);
        if result.is_ok() {
            return Err(err!(
                "Decoding of address '{}' should produce an error.", email;
            Invalid, Input, Decode, String));
        }
        Ok(())
    }

    #[test]
    fn test_email_address_decoding_02() -> Outcome<()> {
        let email = "test @my.domain";
        let result = EmailAddress::try_from(email);
        if result.is_ok() {
            return Err(err!(
                "Decoding of address '{}' should produce an error.", email;
            Invalid, Input, Decode, String));
        }
        Ok(())
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ CLIENT KEYS                                                               │
// └───────────────────────────────────────────────────────────────────────────┘

/// The address a per-client limit is keyed on.
///
/// An IPv6 subscriber is given a whole /64 and can take a fresh address from it on every request, so
/// a limit keyed on the full 128 bits is no limit at all. IPv4 is kept as it is, and an IPv4-mapped
/// IPv6 address (`::ffff:a.b.c.d`) is the IPv4 address it carries, so one client reached over either
/// family is one key. Every other IPv6 address becomes the first address of its /64.
pub fn client_key(ip: &IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_)   => *ip,
        IpAddr::V6(v6)  => match v6.to_ipv4_mapped() {
            Some(v4)    => IpAddr::V4(v4),
            None        => {
                let mut o = v6.octets();
                for b in &mut o[8..] {
                    *b = 0;
                }
                IpAddr::V6(Ipv6Addr::from(o))
            },
        },
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ OUTBOUND ADDRESS VETTING                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Whether an address is one a server may connect to on a user's say-so.
///
/// A service that opens a connection to a host its user named -- a mail
/// bridge, a webhook sender, a link previewer -- is a request forgery
/// waiting to happen: the user names `localhost`, or `169.254.169.254`, or
/// a private address behind the firewall, and the server dutifully reaches
/// somewhere the user could never reach themselves. What makes it dangerous
/// is that the server's *position* is the privilege, not its credentials.
///
/// So: loopback, private, link-local, multicast, broadcast, unspecified and
/// the documentation and benchmark ranges are all refused. What remains is
/// what the user could have reached from their own machine anyway.
pub fn is_publicly_routable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()          // 127/8
                || v4.is_private()       // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()    // 169.254/16, and so the cloud metadata address
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()   // 0.0.0.0
                || v4.is_documentation() // 192.0.2/24, 198.51.100/24, 203.0.113/24
            {
                return false;
            }
            let o = v4.octets();
            // Shared address space (RFC 6598, carrier-grade NAT).
            if o[0] == 100 && (64..128).contains(&o[1]) { return false; }
            // "This network", 0.0.0.0/8, and the IETF protocol assignments, 192.0.0.0/24.
            if o[0] == 0 { return false; }
            if o[0] == 192 && o[1] == 0 && o[2] == 0 { return false; }
            // Benchmarking (RFC 2544).
            if o[0] == 198 && (o[1] == 18 || o[1] == 19) { return false; }
            // Reserved for future use, 240/4 upwards.
            if o[0] >= 240 { return false; }
            true
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
            {
                return false;
            }
            let seg = v6.segments();
            // Unique local, fc00::/7.
            if (seg[0] & 0xfe00) == 0xfc00 { return false; }
            // Link-local, fe80::/10.
            if (seg[0] & 0xffc0) == 0xfe80 { return false; }
            // Site-local, fec0::/10, deprecated but still routed locally by some stacks.
            if (seg[0] & 0xffc0) == 0xfec0 { return false; }
            // Documentation, 2001:db8::/32 and 3fff::/20.
            if seg[0] == 0x2001 && seg[1] == 0x0db8 { return false; }
            if seg[0] == 0x3fff && seg[1] < 0x1000 { return false; }
            // Teredo, 2001::/32, and benchmarking, 2001:2::/48.
            if seg[0] == 0x2001 && seg[1] == 0x0000 { return false; }
            if seg[0] == 0x2001 && seg[1] == 0x0002 && seg[2] == 0 { return false; }
            // The local-use NAT64 block, 64:ff9b:1::/48 (RFC 8215), is reachable only through a
            // translator the operator runs, which turns it back into a connection to the IPv4 inside.
            if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2] == 0x0001 { return false; }
            // Any other form that carries an IPv4 address is only as safe as the IPv4 inside it: the
            // mapped (::ffff:a.b.c.d), the compatible (::a.b.c.d), NAT64 (64:ff9b::/96) and 6to4
            // (2002::/16, the address in the next 32 bits). A translator or a tunnel on the path turns
            // each back into a connection to that IPv4 host.
            let v4_in = |hi: u16, lo: u16| Ipv4Addr::new(
                (hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
            let embedded = if let Some(v4) = v6.to_ipv4_mapped() {
                Some(v4)
            } else if seg[..6].iter().all(|&s| s == 0) {
                Some(v4_in(seg[6], seg[7]))
            } else if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2..6].iter().all(|&s| s == 0) {
                Some(v4_in(seg[6], seg[7]))
            } else if seg[0] == 0x2002 {
                Some(v4_in(seg[1], seg[2]))
            } else {
                None
            };
            match embedded {
                Some(v4)    => is_publicly_routable(&IpAddr::V4(v4)),
                None        => true,
            }
        }
    }
}

/// Resolve a host the user named, and return only the addresses a server
/// may actually connect to.
///
/// Fails, rather than returning an empty list, when the host resolves
/// entirely into space a server must not reach -- because that is not an
/// empty result, it is an attempted request forgery, and the caller wants
/// to say so.
///
/// The caller should connect to one of the returned addresses *directly*,
/// not re-resolve the name: resolving twice invites the answer to change in
/// between (DNS rebinding), which is the whole trick.
pub fn resolve_public(host: &str) -> Outcome<Vec<IpAddr>> {
    // A bare address needs no resolution, and must not get any: a literal
    // is exactly how the forgery is usually spelled.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_publicly_routable(&ip) {
            return Err(err!(
                "The address {} is not publicly routable, and this server \
                will not connect to it on request.", ip;
                Invalid, Input, Security));
        }
        return Ok(vec![ip]);
    }

    let answers = match crate::dns_resolver::lookup_a(host) {
        Ok(a) => a,
        // NXDOMAIN is an error from the resolver now, but to this caller it is still a host that does
        // not resolve.
        Err(e) if e.tags().contains(&ErrTag::Permanent) => return Err(err!(e,
            "The host '{}' does not resolve.", host;
            Invalid, Input, NotFound)),
        Err(e) => return Err(e),
    };
    if answers.is_empty() {
        return Err(err!(
            "The host '{}' does not resolve.", host;
            Invalid, Input, NotFound));
    }
    let public: Vec<IpAddr> = answers.into_iter()
        .map(IpAddr::V4)
        .filter(is_publicly_routable)
        .collect();
    if public.is_empty() {
        return Err(err!(
            "The host '{}' resolves only to addresses that are not publicly \
            routable, and this server will not connect to it on request.", host;
            Invalid, Input, Security));
    }
    Ok(public)
}


#[cfg(test)]
mod vetting_tests {
    use super::*;

    fn v4(s: &str) -> IpAddr { s.parse().expect("test address") }

    #[test]
    fn test_private_space_is_refused() {
        for s in [
            "127.0.0.1",        // loopback
            "10.1.2.3",         // private
            "172.16.0.1",       // private
            "192.168.1.1",      // private
            "169.254.169.254",  // the cloud metadata service
            "0.0.0.0",          // unspecified
            "100.64.0.1",       // carrier-grade NAT
            "224.0.0.1",        // multicast
            "255.255.255.255",  // broadcast
            "240.0.0.1",        // reserved
        ] {
            assert!(!is_publicly_routable(&v4(s)), "{} should be refused", s);
        }
    }

    #[test]
    fn test_public_space_is_allowed() {
        for s in ["1.1.1.1", "8.8.8.8", "142.250.70.14", "2606:4700::1111"] {
            assert!(is_publicly_routable(&v4(s)), "{} should be allowed", s);
        }
    }

    #[test]
    fn test_ipv6_private_space_is_refused() {
        for s in ["::1", "fc00::1", "fe80::1", "2001:db8::1", "::ffff:127.0.0.1"] {
            assert!(!is_publicly_routable(&v4(s)), "{} should be refused", s);
        }
    }

    #[test]
    fn test_ipv6_forms_that_carry_a_private_ipv4_are_refused() {
        for s in [
            "::7f00:1",             // IPv4-compatible 127.0.0.1
            "::a01:203",            // IPv4-compatible 10.1.2.3
            "64:ff9b::7f00:1",      // NAT64 127.0.0.1
            "64:ff9b::a01:203",     // NAT64 10.1.2.3
            "2002:7f00:1::",        // 6to4 127.0.0.1
            "2002:a01:203::1",      // 6to4 10.1.2.3
        ] {
            assert!(!is_publicly_routable(&v4(s)), "{} should be refused", s);
        }
        // The same forms around a public IPv4 stay allowed: it is the inside that is judged.
        for s in ["64:ff9b::808:808", "2002:808:808::1"] {
            assert!(is_publicly_routable(&v4(s)), "{} should be allowed", s);
        }
    }

    #[test]
    fn test_literal_loopback_is_refused_without_dns() {
        assert!(resolve_public("127.0.0.1").is_err());
        assert!(resolve_public("::1").is_err());
    }
}

#[cfg(test)]
mod client_key_tests {
    use super::*;

    fn ip(s: &str) -> IpAddr { s.parse().expect("test address") }

    #[test]
    fn test_ipv4_is_its_own_key() {
        for s in ["1.2.3.4", "10.0.0.1", "127.0.0.1", "255.255.255.255"] {
            assert_eq!(client_key(&ip(s)), ip(s), "{} changed", s);
        }
    }

    #[test]
    fn test_an_ipv4_mapped_address_keys_as_its_ipv4_form() {
        assert_eq!(client_key(&ip("::ffff:1.2.3.4")), ip("1.2.3.4"));
        assert_eq!(client_key(&ip("::ffff:127.0.0.1")), ip("127.0.0.1"));
    }

    #[test]
    fn test_ipv6_is_masked_to_its_slash_64() {
        assert_eq!(client_key(&ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd")), ip("2001:db8:1:2::"));
        assert_eq!(client_key(&ip("2001:db8:1:2::")), ip("2001:db8:1:2::"));
        assert_eq!(client_key(&ip("::1")), ip("::"));
    }

    #[test]
    fn test_two_addresses_in_one_slash_64_share_a_key_and_two_slash_64s_do_not() {
        let a = client_key(&ip("2001:db8:1:2::1"));
        let b = client_key(&ip("2001:db8:1:2:ffff:ffff:ffff:ffff"));
        let c = client_key(&ip("2001:db8:1:3::1"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_a_key_is_stable_under_a_second_application() {
        for s in ["2001:db8:1:2:aaaa:bbbb:cccc:dddd", "::ffff:1.2.3.4", "9.9.9.9"] {
            let once = client_key(&ip(s));
            assert_eq!(client_key(&once), once, "{} is not a fixed point", s);
        }
    }
}
