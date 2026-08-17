//! SOAP as UPnP uses it: one action, flat arguments, no schema (UPnP DA 2.0 §3).
//!
//! A control point invokes an action by POSTing an envelope to a control URL and
//! naming the action twice: once in a `SOAPACTION` header field, and again as the
//! single element inside `<s:Body>`. The arguments are that element's children,
//! each holding text and nothing else. The answer is the same shape with `Response`
//! on the end of the name.
//!
//! That is the whole protocol as it is met in practice, and it is why this module
//! is a scanner rather than an XML parser: the document is machine-written, one
//! level deep, and the alternative is a parser dependency for a body that is
//! always the same six lines.
//!
//! # What this does not do
//!
//! Namespace prefixes are compared by local name, so a control point that binds
//! the envelope namespace to `SOAP-ENV` rather than `s` is understood, and one
//! that binds `s` to something else entirely is misunderstood. No control point
//! does the second. Attributes on argument elements are ignored, and so is
//! anything outside `<s:Body>`.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::upnp::{
    escape,
    unescape,
    NS_SOAP_ENVELOPE,
    NS_UPNP_CONTROL,
    SOAP_ENCODING,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;


#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Action {
    pub service:    String,                     // service type, empty where `SOAPACTION` was absent
    pub name:       String,                     // e.g. `Browse`
    pub args:       BTreeMap<String, String>,   // by name, unescaped
}

impl Action {

    /// The header is advisory: the body is what says which action to run, and a
    /// control point whose header disagrees with its body is answered from the
    /// body. Pass `None` where the field was absent.
    pub fn parse(soap_action: Option<&str>, body: &str) -> Outcome<Self> {
        let (service, _named) = match soap_action {
            Some(v) => res!(parse_action_field(v)),
            None    => (String::new(), String::new()),
        };
        let (name, args) = res!(parse_body(body));
        Ok(Self {
            service,
            name,
            args,
        })
    }

    /// UPnP answers a missing argument with error 402, and a caller that wants
    /// that spelling wraps this in [`SoapError::InvalidArgs`].
    pub fn need(&self, arg: &str) -> Outcome<&str> {
        match self.args.get(arg) {
            Some(v) => Ok(v.as_str()),
            None => Err(err!(
                "The {} action carried no {} argument.", self.name, arg;
            Input, Missing)),
        }
    }

    /// An absent or unreadable argument reads as zero. Every numeric argument in
    /// ContentDirectory:1 is an unsigned index or count, and a control point that
    /// writes an empty `StartingIndex` means the beginning rather than an error.
    pub fn count(&self, arg: &str) -> u64 {
        match self.args.get(arg) {
            Some(v) => v.trim().parse::<u64>().unwrap_or(0),
            None    => 0,
        }
    }
}

/// The value is `"urn:schemas-upnp-org:service:ContentDirectory:1#Browse"`, with
/// the quotation marks part of the field, and it splits at the `#`.
pub fn parse_action_field(value: &str) -> Outcome<(String, String)> {
    let trimmed = value.trim().trim_matches('"');
    match trimmed.rsplit_once('#') {
        Some((service, action)) => Ok((service.to_string(), action.to_string())),
        None => Err(err!(
            "A SOAPACTION of {:?} names no action: it wants service#Action.", value;
        Input, Invalid)),
    }
}

pub fn parse_body(body: &str) -> Outcome<(String, BTreeMap<String, String>)> {
    let inner = match element_body(body, "Body") {
        Some(inner) => inner,
        None => return Err(err!(
            "A SOAP envelope carried no Body element."; Input, Missing)),
    };
    let (name, (contents, _after)) = match first_element(inner) {
        Some(pair) => pair,
        None => return Err(err!(
            "A SOAP Body carried no action element."; Input, Missing)),
    };
    let mut args = BTreeMap::new();
    let mut rest = contents;
    while let Some((arg, (value, after))) = first_element(rest) {
        // Where the same argument is sent twice the last wins, which is what a
        // map does and what no control point relies on either way.
        args.insert(arg.to_string(), unescape(value));
        rest = after;
    }
    Ok((name.to_string(), args))
}

/// Matched on local name, ignoring any prefix. `None` for an element that is not
/// there, and for one written as an empty tag, which for a SOAP body means the
/// same thing.
fn element_body<'a>(xml: &'a str, local: &str) -> Option<&'a str> {
    let mut from = 0usize;
    while let Some(open) = xml[from..].find('<') {
        let start = from + open;
        let close = match xml[start..].find('>') {
            Some(rel) => start + rel,
            None      => return None,
        };
        let tag = &xml[start + 1..close];
        if !tag.starts_with('/') && !tag.starts_with('?') && !tag.starts_with('!')
            && !tag.ends_with('/')
            && local_name(tag) == local
        {
            // The matching close tag, found by its local name so that a prefix
            // change between the two does not lose it.
            let after = close + 1;
            let mut at = after;
            while let Some(rel) = xml[at..].find("</") {
                let shut = at + rel;
                let shut_end = match xml[shut..].find('>') {
                    Some(r) => shut + r,
                    None    => return None,
                };
                if local_name(&xml[shut + 2..shut_end]) == local {
                    return Some(&xml[after..shut]);
                }
                at = shut_end + 1;
            }
            return None;
        }
        from = close + 1;
    }
    None
}

/// Its local name, its text, and what follows it. An empty element (`<Filter/>`)
/// yields an empty value, which is what a control point asking for every field
/// sends.
fn first_element(xml: &str) -> Option<(&str, (&str, &str))> {
    let open = match xml.find('<') {
        Some(at) => at,
        None     => return None,
    };
    let close = match xml[open..].find('>') {
        Some(rel) => open + rel,
        None      => return None,
    };
    let tag = &xml[open + 1..close];
    if tag.starts_with('/') || tag.starts_with('?') || tag.starts_with('!') {
        return None;
    }
    let local = local_name(tag);
    if tag.ends_with('/') {
        return Some((local, ("", &xml[close + 1..])));
    }
    let after = close + 1;
    let mut at = after;
    let mut depth = 0usize;
    loop {
        let next = match xml[at..].find('<') {
            Some(rel) => at + rel,
            None      => return None,
        };
        let shut_end = match xml[next..].find('>') {
            Some(rel) => next + rel,
            None      => return None,
        };
        let inner = &xml[next + 1..shut_end];
        if let Some(name) = inner.strip_prefix('/') {
            if local_name(name) == local && depth == 0 {
                return Some((local, (&xml[after..next], &xml[shut_end + 1..])));
            }
            depth = depth.saturating_sub(1);
        } else if !inner.ends_with('/') && !inner.starts_with('?') && !inner.starts_with('!')
            && local_name(inner) == local
        {
            // A nested element of the same name, which argument values do not
            // have and which costs nothing to survive.
            depth += 1;
        }
        at = shut_end + 1;
    }
}

/// What is left after any namespace prefix and before any attribute.
fn local_name(tag: &str) -> &str {
    let name = match tag.find(|c: char| c.is_whitespace()) {
        Some(at) => &tag[..at],
        None     => tag,
    };
    let name = name.trim_end_matches('/');
    match name.rsplit_once(':') {
        Some((_, local)) => local,
        None             => name,
    }
}

/// `service` is the service *type*, which the response element carries as its
/// namespace, and the arguments go out in the order given: ContentDirectory:1
/// specifies an order for them and some control points read them positionally.
pub fn response(service: &str, action: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n");
    out.push_str(&fmt!(
        "<s:Envelope xmlns:s=\"{}\" s:encodingStyle=\"{}\">",
        NS_SOAP_ENVELOPE, SOAP_ENCODING));
    out.push_str("<s:Body>");
    out.push_str(&fmt!("<u:{}Response xmlns:u=\"{}\">", action, service));
    for (name, value) in args {
        out.push_str(&fmt!("<{}>{}</{}>", name, escape(value), name));
    }
    out.push_str(&fmt!("</u:{}Response>", action));
    out.push_str("</s:Body></s:Envelope>");
    out
}

/// The UPnP errors a ContentDirectory server actually returns (UPnP DA 2.0 §3.3.2
/// and ContentDirectory:1 §2.7).
///
/// Held as an enum rather than as bare numbers because the code and the phrase
/// belong together: a control point shows the phrase to somebody, and a fault
/// whose two halves disagree is worse than no fault at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoapError {
    InvalidAction,      // this service has no such action
    InvalidArgs,        // an argument is missing, misnamed or unreadable
    ActionFailed,       // understood, and could not be carried out
    NoSuchObject,       // the `ObjectID` names nothing
    UnsupportedSort,    // the `SortCriteria` names a property this server cannot sort on
    CannotProcess,      // no reason of its own, so the last resort
}

impl SoapError {

    /// The `errorCode` a fault carries.
    pub fn code(&self) -> u16 {
        match self {
            Self::InvalidAction	=> 401,
            Self::InvalidArgs	=> 402,
            Self::ActionFailed	=> 501,
            Self::NoSuchObject	=> 701,
            Self::UnsupportedSort	=> 709,
            Self::CannotProcess	=> 720,
        }
    }

    /// The `errorDescription` that goes with it, in the specification's words.
    pub fn description(&self) -> &'static str {
        match self {
            Self::InvalidAction	=> "Invalid Action",
            Self::InvalidArgs	=> "Invalid Args",
            Self::ActionFailed	=> "Action Failed",
            Self::NoSuchObject	=> "No such object",
            Self::UnsupportedSort	=> "Unsupported or invalid sort criteria",
            Self::CannotProcess	=> "Cannot process the request",
        }
    }

    /// A SOAP fault goes back with HTTP status 500, which is the caller's to set:
    /// a control point that receives a fault under a 200 discards it.
    pub fn envelope(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n");
        out.push_str(&fmt!(
            "<s:Envelope xmlns:s=\"{}\" s:encodingStyle=\"{}\">",
            NS_SOAP_ENVELOPE, SOAP_ENCODING));
        out.push_str("<s:Body><s:Fault>");
        out.push_str("<faultcode>s:Client</faultcode>");
        out.push_str("<faultstring>UPnPError</faultstring>");
        out.push_str("<detail>");
        out.push_str(&fmt!("<UPnPError xmlns=\"{}\">", NS_UPNP_CONTROL));
        out.push_str(&fmt!("<errorCode>{}</errorCode>", self.code()));
        out.push_str(&fmt!("<errorDescription>{}</errorDescription>",
            escape(self.description())));
        out.push_str("</UPnPError></detail>");
        out.push_str("</s:Fault></s:Body></s:Envelope>");
        out
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    // A `Browse` as a control point sends one: prefixed envelope, unprefixed
    // arguments, and an escaped filter.
    const A_BROWSE: &str = "<?xml version=\"1.0\"?>\
        <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
        s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
        <s:Body>\
        <u:Browse xmlns:u=\"urn:schemas-upnp-org:service:ContentDirectory:1\">\
        <ObjectID>0</ObjectID>\
        <BrowseFlag>BrowseDirectChildren</BrowseFlag>\
        <Filter>*</Filter>\
        <StartingIndex>0</StartingIndex>\
        <RequestedCount>25</RequestedCount>\
        <SortCriteria></SortCriteria>\
        </u:Browse>\
        </s:Body>\
        </s:Envelope>";

    #[test]
    fn test_a_browse_is_read() -> Outcome<()> {
        let action = res!(Action::parse(
            Some("\"urn:schemas-upnp-org:service:ContentDirectory:1#Browse\""),
            A_BROWSE,
        ));
        assert_eq!(action.service, "urn:schemas-upnp-org:service:ContentDirectory:1");
        assert_eq!(action.name, "Browse");
        assert_eq!(res!(action.need("ObjectID")), "0");
        assert_eq!(res!(action.need("BrowseFlag")), "BrowseDirectChildren");
        assert_eq!(action.count("RequestedCount"), 25);
        assert_eq!(action.count("StartingIndex"), 0);
        assert_eq!(res!(action.need("SortCriteria")), "");
        Ok(())
    }

    /// A control point that uses another prefix for the envelope, writes its
    /// arguments as empty tags, and puts whitespace between them, is still
    /// understood. All three are seen on real networks.
    #[test]
    fn test_a_differently_written_envelope_is_read() -> Outcome<()> {
        let odd = "<SOAP-ENV:Envelope \
            xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\">\n\
            <SOAP-ENV:Body>\n\
            <m:Browse xmlns:m=\"urn:schemas-upnp-org:service:ContentDirectory:1\">\n\
            <ObjectID>0$D</ObjectID>\n\
            <BrowseFlag>BrowseMetadata</BrowseFlag>\n\
            <Filter/>\n\
            <StartingIndex>0</StartingIndex>\n\
            <RequestedCount>0</RequestedCount>\n\
            <SortCriteria/>\n\
            </m:Browse>\n\
            </SOAP-ENV:Body>\n\
            </SOAP-ENV:Envelope>";
        let action = res!(Action::parse(None, odd));
        assert_eq!(action.name, "Browse");
        assert_eq!(res!(action.need("ObjectID")), "0$D");
        assert_eq!(res!(action.need("Filter")), "");
        assert_eq!(action.count("RequestedCount"), 0);
        Ok(())
    }

    /// An escaped argument comes back as what it meant.
    #[test]
    fn test_an_escaped_argument_is_unescaped() -> Outcome<()> {
        let body = "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
            <s:Body><u:Search xmlns:u=\"urn:x\">\
            <SearchCriteria>dc:title contains &quot;Rosie &amp; Chloe&quot;</SearchCriteria>\
            </u:Search></s:Body></s:Envelope>";
        let action = res!(Action::parse(None, body));
        assert_eq!(action.name, "Search");
        assert_eq!(res!(action.need("SearchCriteria")),
            "dc:title contains \"Rosie & Chloe\"");
        Ok(())
    }

    #[test]
    fn test_what_is_not_an_invocation_is_refused() {
        for bad in [
            "",
            "<html><body>not soap</body></html>",
            "<s:Envelope xmlns:s=\"x\"><s:Body></s:Body></s:Envelope>",
        ] {
            assert!(Action::parse(None, bad).is_err(), "{:?} should not have parsed", bad);
        }
        assert!(parse_action_field("no-hash-here").is_err());
    }

    #[test]
    fn test_the_action_field_splits_into_service_and_action() -> Outcome<()> {
        let (service, action) = res!(parse_action_field(
            "\"urn:schemas-upnp-org:service:ConnectionManager:1#GetProtocolInfo\""));
        assert_eq!(service, "urn:schemas-upnp-org:service:ConnectionManager:1");
        assert_eq!(action, "GetProtocolInfo");
        Ok(())
    }

    /// The answer is the request's shape with `Response` on the name, and its
    /// arguments keep the order they were given.
    #[test]
    fn test_a_response_names_the_action_and_keeps_its_argument_order() -> Outcome<()> {
        let xml = response(
            "urn:schemas-upnp-org:service:ContentDirectory:1",
            "Browse",
            &[
                ("Result", "<DIDL-Lite/>".to_string()),
                ("NumberReturned", "3".to_string()),
                ("TotalMatches", "40".to_string()),
                ("UpdateID", "1".to_string()),
            ],
        );
        assert!(xml.contains("<u:BrowseResponse xmlns:u=\
            \"urn:schemas-upnp-org:service:ContentDirectory:1\">"));
        // The DIDL-Lite payload goes inside a string, so it is escaped.
        assert!(xml.contains("<Result>&lt;DIDL-Lite/&gt;</Result>"), "{}", xml);
        let returned = match xml.find("<NumberReturned>") {
            Some(at) => at,
            None => return Err(err!("No NumberReturned in {}", xml; Test, Missing)),
        };
        let total = match xml.find("<TotalMatches>") {
            Some(at) => at,
            None => return Err(err!("No TotalMatches in {}", xml; Test, Missing)),
        };
        assert!(returned < total, "the arguments came out in the wrong order");
        Ok(())
    }

    /// The envelope this module writes, it can read back.
    #[test]
    fn test_an_answer_survives_a_round_trip() -> Outcome<()> {
        let xml = response("urn:x:service:ContentDirectory:1", "Browse", &[
            ("Result", "<DIDL-Lite xmlns=\"y\"><item id=\"0$A$1\"/></DIDL-Lite>".to_string()),
            ("NumberReturned", "1".to_string()),
        ]);
        let (name, args) = res!(parse_body(&xml));
        assert_eq!(name, "BrowseResponse");
        assert_eq!(args.get("NumberReturned").map(String::as_str), Some("1"));
        assert_eq!(args.get("Result").map(String::as_str),
            Some("<DIDL-Lite xmlns=\"y\"><item id=\"0$A$1\"/></DIDL-Lite>"));
        Ok(())
    }

    #[test]
    fn test_a_fault_carries_the_code_and_the_phrase_that_belongs_to_it() {
        let xml = SoapError::NoSuchObject.envelope();
        assert!(xml.contains("<errorCode>701</errorCode>"), "{}", xml);
        assert!(xml.contains("<errorDescription>No such object</errorDescription>"), "{}", xml);
        assert!(xml.contains("<faultstring>UPnPError</faultstring>"), "{}", xml);
        assert_eq!(SoapError::InvalidArgs.code(), 402);
        assert_eq!(SoapError::ActionFailed.code(), 501);
    }
}
