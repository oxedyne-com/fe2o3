//! UPnP: the part that begins once SSDP has said where to look.
//!
//! [`crate::ssdp`] carries a `LOCATION` to a control point. This module is what
//! sits at the other end of it, and what the control point says next:
//!
//! - a **device description** document, XML, listing the device and its services
//!   ([`device`]),
//! - a **service description** (SCPD) per service, listing its actions
//!   ([`device::content_directory_scpd`] and friends),
//! - **SOAP** requests and responses against the control URLs ([`soap`]),
//! - and, for a MediaServer, **DIDL-Lite** as the payload a `Browse` answers
//!   with ([`didl`]).
//!
//! # Pure primitives
//!
//! Nothing here opens a socket, reads a file or knows what a library is. The
//! caller owns the transport: it routes its own HTTP, hands the request body to
//! [`soap::Action::parse`], builds the answer out of [`didl`] types and writes it
//! back. That keeps this usable from a synchronous server, an async one, or a
//! test with no server at all.
//!
//! # The two names of everything
//!
//! A UPnP service is named by a *type* (`urn:schemas-upnp-org:service:...`) and,
//! separately, by an *identifier* (`urn:upnp-org:serviceId:...`). They look alike
//! and are not interchangeable: the type says what the service is, the identifier
//! says which one it is on this device. A description that swaps them is accepted
//! by some control points and silently ignored by others, which is the failure
//! that eats an afternoon.

pub mod device;
pub mod didl;
pub mod soap;

use oxedyne_fe2o3_core::prelude::*;

/// The namespace of a device description document (UPnP DA 2.0 §2.3).
pub const NS_DEVICE: &str = "urn:schemas-upnp-org:device-1-0";

/// The namespace of a service description document (UPnP DA 2.0 §2.5).
pub const NS_SERVICE: &str = "urn:schemas-upnp-org:service-1-0";

/// The DLNA device namespace, carried on `<dlna:X_DLNADOC>`.
pub const NS_DLNA_DEVICE: &str = "urn:schemas-dlna-org:device-1-0";

/// The DLNA metadata namespace, carried on DIDL-Lite documents.
pub const NS_DLNA_METADATA: &str = "urn:schemas-dlna-org:metadata-1-0/";

/// The DIDL-Lite namespace (ContentDirectory:1 §2.8).
pub const NS_DIDL: &str = "urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/";

/// Dublin Core, which is where a DIDL-Lite title and date come from.
pub const NS_DC: &str = "http://purl.org/dc/elements/1.1/";

/// The UPnP metadata namespace, which is where an object class comes from.
pub const NS_UPNP: &str = "urn:schemas-upnp-org:metadata-1-0/upnp/";

/// The SOAP 1.1 envelope namespace, which is the one UPnP uses.
pub const NS_SOAP_ENVELOPE: &str = "http://schemas.xmlsoap.org/soap/envelope/";

/// The SOAP 1.1 encoding style, required verbatim on the envelope.
pub const SOAP_ENCODING: &str = "http://schemas.xmlsoap.org/soap/encoding/";

/// The UPnP control error namespace, carried inside a SOAP fault.
pub const NS_UPNP_CONTROL: &str = "urn:schemas-upnp-org:control-1-0";

/// `urn:schemas-upnp-org:device:MediaServer:1`.
pub const DEVICE_MEDIA_SERVER: &str = "urn:schemas-upnp-org:device:MediaServer:1";

/// `urn:schemas-upnp-org:service:ContentDirectory:1`.
pub const SERVICE_CONTENT_DIRECTORY: &str =
    "urn:schemas-upnp-org:service:ContentDirectory:1";

/// `urn:schemas-upnp-org:service:ConnectionManager:1`.
pub const SERVICE_CONNECTION_MANAGER: &str =
    "urn:schemas-upnp-org:service:ConnectionManager:1";

/// The service identifier that goes with [`SERVICE_CONTENT_DIRECTORY`].
pub const ID_CONTENT_DIRECTORY: &str = "urn:upnp-org:serviceId:ContentDirectory";

/// The service identifier that goes with [`SERVICE_CONNECTION_MANAGER`].
pub const ID_CONNECTION_MANAGER: &str = "urn:upnp-org:serviceId:ConnectionManager";

/// The `<dlna:X_DLNADOC>` value a media server declares, saying it speaks DLNA
/// version 1.50 as a Digital Media Server.
pub const DLNA_DOC_DMS: &str = "DMS-1.50";

/// The content type every UPnP document goes out under.
pub const XML_CONTENT_TYPE: &str = "text/xml; charset=\"utf-8\"";


/// Escape text for an XML element body or an attribute value.
///
/// All five predefined entities are written, including the two that only matter
/// inside an attribute, because the same title goes into both places and a
/// separate attribute escaper is one more thing to forget to call.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            '&'	=> out.push_str("&amp;"),
            '<'	=> out.push_str("&lt;"),
            '>'	=> out.push_str("&gt;"),
            '"'	=> out.push_str("&quot;"),
            '\''	=> out.push_str("&apos;"),
            // XML 1.0 admits tab, newline and carriage return and no other
            // control character. A title carrying one would make the whole
            // document unparseable, so it is dropped rather than written.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {},
            c	=> out.push(c),
        }
    }
    out
}

/// Undo [`escape`], including the numeric character references a control point
/// may have written instead of the named ones.
///
/// Anything that is not a whole reference is taken literally, which is what a
/// forgiving parser does and what keeps a stray ampersand in a file name from
/// turning into an error.
pub fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            // Push the whole run up to the next ampersand, so that multi-byte
            // characters are copied without being taken apart.
            let start = i;
            while i < bytes.len() && bytes[i] != b'&' {
                i += 1;
            }
            out.push_str(&text[start..i]);
            continue;
        }
        match text[i..].find(';') {
            Some(rel) if rel <= 10 => {
                let entity = &text[i + 1..i + rel];
                match named_entity(entity) {
                    Some(c) => {
                        out.push(c);
                        i += rel + 1;
                    },
                    None => {
                        out.push('&');
                        i += 1;
                    },
                }
            },
            _ => {
                out.push('&');
                i += 1;
            },
        }
    }
    out
}

/// One entity body, without its ampersand and semicolon.
fn named_entity(entity: &str) -> Option<char> {
    match entity {
        "amp"	=> return Some('&'),
        "lt"	=> return Some('<'),
        "gt"	=> return Some('>'),
        "quot"	=> return Some('"'),
        "apos"	=> return Some('\''),
        _	=> {},
    }
    let digits = match entity.strip_prefix('#') {
        Some(d) => d,
        None    => return None,
    };
    let code = match digits.strip_prefix('x').or_else(|| digits.strip_prefix('X')) {
        Some(hex)   => u32::from_str_radix(hex, 16).ok(),
        None        => digits.parse::<u32>().ok(),
    };
    match code {
        Some(n) => char::from_u32(n),
        None    => None,
    }
}

/// The targets and matching `USN` values a device announces over SSDP.
///
/// A UPnP root device is not one announcement but several: `upnp:rootdevice`,
/// its own UUID, its device type, and one per service it carries. Every one of
/// them pairs a target with a `USN` built from the UUID, and the two must agree
/// or the device is discovered and then cannot be reached ([`crate::ssdp`]).
/// Building the pairs in one place is what keeps them agreeing.
///
/// `uuid` is the bare identifier, without the `uuid:` prefix.
pub fn announcements(
    uuid:           &str,
    device_type:    &str,
    services:       &[&str],
)
    -> Vec<(crate::ssdp::Target, String)>
{
    use crate::ssdp::Target;
    let mut out = Vec::with_capacity(services.len() + 3);
    // The root device announcement, whose USN is the UUID and the target.
    out.push((
        Target::RootDevice,
        fmt!("uuid:{}::upnp:rootdevice", uuid),
    ));
    // The device itself, whose USN is the UUID alone.
    out.push((
        Target::Uuid(uuid.to_string()),
        fmt!("uuid:{}", uuid),
    ));
    out.push((
        res_target(device_type),
        fmt!("uuid:{}::{}", uuid, device_type),
    ));
    for service in services {
        out.push((
            res_target(service),
            fmt!("uuid:{}::{}", uuid, service),
        ));
    }
    out
}

/// A `urn:` string as a target, without going through a fallible parse: every
/// caller of [`announcements`] passes a constant from this module.
fn res_target(urn: &str) -> crate::ssdp::Target {
    match urn.strip_prefix("urn:") {
        Some(rest) => crate::ssdp::Target::Urn(rest.to_string()),
        None       => crate::ssdp::Target::Other(urn.to_string()),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_five_entities_go_out_and_come_back() {
        let awkward = "Rosie & Chloe <\"2016\"> 'x'";
        let there = escape(awkward);
        assert!(!there.contains('<'), "an angle bracket survived: {}", there);
        assert_eq!(unescape(&there), awkward);
    }

    /// A control point may write `&#38;` where this crate writes `&amp;`, and a
    /// reader that only knows the named entities silently mangles a title.
    #[test]
    fn test_a_numeric_reference_is_read() {
        assert_eq!(unescape("a&#38;b"), "a&b");
        assert_eq!(unescape("a&#x26;b"), "a&b");
        assert_eq!(unescape("caf&#233;"), "café");
    }

    /// A bare ampersand is not an entity and must not eat what follows it.
    #[test]
    fn test_what_is_not_an_entity_is_left_alone() {
        assert_eq!(unescape("100% & rising"), "100% & rising");
        assert_eq!(unescape("&notanentity;"), "&notanentity;");
        assert_eq!(unescape("&"), "&");
    }

    /// A character XML 1.0 does not admit at all is dropped rather than written,
    /// because one of them makes the whole document unparseable.
    #[test]
    fn test_a_control_character_does_not_reach_the_document() {
        assert_eq!(escape("a\u{0}b\u{7}c"), "abc");
        assert_eq!(escape("a\tb\nc"), "a\tb\nc");
    }

    /// Every announcement pairs a target with a USN that names the same thing.
    #[test]
    fn test_an_announcement_names_one_thing_twice_and_agrees_with_itself() {
        let pairs = announcements(
            "4d696e69-444c-164e-9d41-0011328c0e2f",
            DEVICE_MEDIA_SERVER,
            &[SERVICE_CONTENT_DIRECTORY, SERVICE_CONNECTION_MANAGER],
        );
        assert_eq!(pairs.len(), 5);
        for (target, usn) in &pairs {
            let named = fmt!("{}", target);
            if named.starts_with("uuid:") {
                // The device's own announcement: the USN is the UUID alone.
                assert_eq!(usn, &named);
            } else {
                assert!(usn.ends_with(&fmt!("::{}", named)),
                    "{} does not end with the target {}", usn, named);
            }
            assert!(usn.starts_with("uuid:4d696e69-"), "{} names no device", usn);
        }
    }
}
