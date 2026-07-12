use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_stds::regions::Country;

use std::{
    convert::TryFrom,
    fmt::{self},
    net::IpAddr,
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
            // Documentation, 2001:db8::/32.
            if seg[0] == 0x2001 && seg[1] == 0x0db8 { return false; }
            // An IPv4-mapped address is only as safe as the IPv4 inside it.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_publicly_routable(&IpAddr::V4(v4));
            }
            true
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

    let answers = res!(crate::dns_resolver::lookup_a(host));
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
    fn test_literal_loopback_is_refused_without_dns() {
        assert!(resolve_public("127.0.0.1").is_err());
        assert!(resolve_public("::1").is_err());
    }
}
