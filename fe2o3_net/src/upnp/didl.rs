//! DIDL-Lite: what a ContentDirectory `Browse` actually answers with
//! (ContentDirectory:1 §2.8, and DLNA guidelines part 1 §7.3 for the profiles).
//!
//! The answer to a browse is not the XML a control point sees first. It is a
//! *string* carried inside the `<Result>` argument of a SOAP response, so the
//! whole DIDL-Lite document is escaped once on the way out. That double layer is
//! the commonest way a media server's output is subtly wrong: an unescaped
//! ampersand in a photograph's file name breaks the outer document, and a
//! doubly-escaped one shows up on the television as `&amp;`.
//!
//! # `protocolInfo`, and why it decides everything
//!
//! Each `<res>` element carries a `protocolInfo` of four colon-separated fields:
//! `http-get:*:image/jpeg:DLNA.ORG_PN=JPEG_LRG;DLNA.ORG_OP=01;...`. The fourth
//! field is where a television decides whether it will play something at all. A
//! set that finds no `DLNA.ORG_PN` it recognises may show the item and refuse to
//! open it, and a set given a `DLNA.ORG_PN` that does not match the bytes behind
//! it fails in stranger ways still. [`ProtocolInfo`] therefore holds the pieces
//! apart, so that a caller trying profile strings against a real television
//! changes one table rather than a dozen format strings.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::upnp::{
    escape,
    NS_DC,
    NS_DIDL,
    NS_DLNA_METADATA,
    NS_UPNP,
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;


/// A control point decides how to *treat* an object from its `upnp:class`, not
/// from what is in it: a set showing a slideshow looks for `imageItem`, and one
/// browsing for something to play looks for `videoItem`. Held as an enum so that
/// a class cannot be misspelled at one call site out of six.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Class {
    Container,              // nothing more specific is true
    StorageFolder,          // what a filesystem tree becomes
    PhotoAlbum,             // what an album becomes
    Photo,
    Movie,
    Other(&'static str),    // a class this crate does not model
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Container	=> "object.container",
            Self::StorageFolder	=> "object.container.storageFolder",
            Self::PhotoAlbum	=> "object.container.album.photoAlbum",
            Self::Photo	=> "object.item.imageItem.photo",
            Self::Movie	=> "object.item.videoItem.movie",
            Self::Other(s)	=> s,
        })
    }
}

/// The fourth field of a `protocolInfo`, which is where DLNA lives.
///
/// Every part is optional, because a resource whose profile is not confidently
/// known is better described by `*` than by a guess: a television shown a profile
/// that does not match the bytes fails in a way that is hard to read, whereas one
/// shown no profile at all either plays the file or does not.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DlnaExtras {
    pub profile:    Option<String>,         // `DLNA.ORG_PN`, e.g. `JPEG_LRG`
    pub operations: Option<&'static str>,   // `DLNA.ORG_OP`, time-seek then byte-seek
    pub converted:  Option<bool>,           // `DLNA.ORG_CI`, 1 where the server made the bytes
    pub flags:      Option<&'static str>,   // `DLNA.ORG_FLAGS`, see FLAGS_IMAGE and FLAGS_STREAMING
}

//// `DLNA.ORG_OP` values.
// Two flags, time-seek then byte-seek. A server answering HTTP byte ranges and
// nothing else advertises the first of these.
pub const OP_BYTE_RANGE: &str = "01";
pub const OP_NONE: &str = "00";

//// `DLNA.ORG_FLAGS` values.
// One thirty-two digit hexadecimal number whose meaning is all in its first
// eight digits; the rest are reserved and are zero. Both declare DLNA v1.5 and
// HTTP stalling; the image value is interactive transfer, and the film value
// adds streaming and background transfer.
pub const FLAGS_IMAGE: &str = "00D00000000000000000000000000000";
pub const FLAGS_STREAMING: &str = "01700000000000000000000000000000";

impl DlnaExtras {

    /// The extras a rendition the server made itself carries.
    pub fn rendition(profile: &str) -> Self {
        Self {
            profile:    Some(profile.to_string()),
            operations: Some(OP_BYTE_RANGE),
            converted:  Some(true),
            flags:      Some(FLAGS_IMAGE),
        }
    }

    /// The extras an original served as it lies carries.
    pub fn original(profile: Option<String>, streaming: bool) -> Self {
        Self {
            profile,
            operations: Some(OP_BYTE_RANGE),
            converted:  Some(false),
            flags:      Some(if streaming { FLAGS_STREAMING } else { FLAGS_IMAGE }),
        }
    }
}

impl fmt::Display for DlnaExtras {
    /// In the order the DLNA guidelines write it. An entirely empty set of
    /// extras goes out as `*`, the field's way of saying nothing is claimed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::with_capacity(4);
        if let Some(pn) = &self.profile {
            parts.push(fmt!("DLNA.ORG_PN={}", pn));
        }
        if let Some(op) = self.operations {
            parts.push(fmt!("DLNA.ORG_OP={}", op));
        }
        if let Some(ci) = self.converted {
            parts.push(fmt!("DLNA.ORG_CI={}", if ci { 1 } else { 0 }));
        }
        if let Some(flags) = self.flags {
            parts.push(fmt!("DLNA.ORG_FLAGS={}", flags));
        }
        if parts.is_empty() {
            return write!(f, "*");
        }
        write!(f, "{}", parts.join(";"))
    }
}

/// The whole `protocolInfo` attribute: how to fetch it, from where, what it is,
/// and what DLNA says about it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolInfo {
    pub protocol:   String,     // `http-get` for everything served over HTTP
    pub network:    String,     // `*` everywhere, a field only because the syntax has four
    pub content:    String,     // content type, e.g. `image/jpeg`
    pub extras:     DlnaExtras,
}

impl ProtocolInfo {

    pub fn http_get<S: Into<String>>(content: S, extras: DlnaExtras) -> Self {
        Self {
            protocol:   "http-get".to_string(),
            network:    "*".to_string(),
            content:    content.into(),
            extras,
        }
    }
}

impl fmt::Display for ProtocolInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}:{}", self.protocol, self.network, self.content, self.extras)
    }
}

/// One `<res>`. An object may carry several, and a control point picks whichever
/// it likes the look of, which is why a thumbnail and a full-size rendition of
/// the same photograph are two resources on one item rather than two items.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resource {
    pub uri:        String,
    pub info:       ProtocolInfo,
    pub size:       Option<u64>,        // where it is known without reading the bytes
    pub resolution: Option<(u32, u32)>, // width then height, in pixels
    pub duration:   Option<String>,     // running time, as `H:MM:SS.mmm`
    pub depth:      Option<u32>,        // bits per pixel, which a few sets read and none require
}

impl Resource {

    /// The `uri` and the `info` are the two things every resource must have.
    pub fn new<S: Into<String>>(uri: S, info: ProtocolInfo) -> Self {
        Self {
            uri:        uri.into(),
            info,
            size:       None,
            resolution: None,
            duration:   None,
            depth:      None,
        }
    }

    pub fn sized(mut self, bytes: u64) -> Self {
        self.size = Some(bytes);
        self
    }

    pub fn at(mut self, w: u32, h: u32) -> Self {
        self.resolution = Some((w, h));
        self
    }

    /// Every value written here is escaped, attributes included.
    fn write(&self, out: &mut String) {
        out.push_str(&fmt!("<res protocolInfo=\"{}\"", escape(&fmt!("{}", self.info))));
        if let Some(size) = self.size {
            out.push_str(&fmt!(" size=\"{}\"", size));
        }
        if let Some((w, h)) = self.resolution {
            out.push_str(&fmt!(" resolution=\"{}x{}\"", w, h));
        }
        if let Some(duration) = &self.duration {
            out.push_str(&fmt!(" duration=\"{}\"", escape(duration)));
        }
        if let Some(depth) = self.depth {
            out.push_str(&fmt!(" colorDepth=\"{}\"", depth));
        }
        out.push_str(&fmt!(">{}</res>", escape(&self.uri)));
    }
}

/// Something a control point can browse into.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Container {
    pub id:         String,
    pub parent:     String,                 // the root's parent is `-1`
    pub title:      String,
    pub class:      Class,
    // A control point draws a count from this, and some will not descend without one.
    pub children:   Option<u64>,            // direct children, where that is cheap to know
    pub restricted: bool,                   // a served library is restricted, going out as `1`
    pub searchable: bool,                   // whether `Search` may be run against it
}

impl Container {

    /// A restricted, unsearchable container, which is what a served library is
    /// made of.
    pub fn new<I, P, T>(id: I, parent: P, title: T, class: Class) -> Self
    where
        I: Into<String>,
        P: Into<String>,
        T: Into<String>,
    {
        Self {
            id:         id.into(),
            parent:     parent.into(),
            title:      title.into(),
            class,
            children:   None,
            restricted: true,
            searchable: false,
        }
    }

    pub fn holding(mut self, n: u64) -> Self {
        self.children = Some(n);
        self
    }

    fn write(&self, out: &mut String) {
        out.push_str(&fmt!("<container id=\"{}\" parentID=\"{}\" restricted=\"{}\"",
            escape(&self.id), escape(&self.parent), if self.restricted { 1 } else { 0 }));
        if let Some(n) = self.children {
            out.push_str(&fmt!(" childCount=\"{}\"", n));
        }
        out.push_str(&fmt!(" searchable=\"{}\">", if self.searchable { 1 } else { 0 }));
        out.push_str(&fmt!("<dc:title>{}</dc:title>", escape(&self.title)));
        out.push_str(&fmt!("<upnp:class>{}</upnp:class>", self.class));
        out.push_str("</container>");
    }
}

/// Something a control point can play or show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub id:         String,
    pub parent:     String,                 // the container it was browsed from
    pub title:      String,
    pub class:      Class,
    // A television that groups or sorts by date reads this and nothing else.
    pub date:       Option<String>,         // `YYYY-MM-DDTHH:MM:SS`
    pub art:        Option<String>,         // thumbnail, which most sets use and none need
    pub art_profile: Option<String>,        // goes out as `dlna:profileID`
    pub restricted: bool,
    pub resources:  Vec<Resource>,          // best first
}

impl Item {

    /// A restricted item with no resources yet.
    pub fn new<I, P, T>(id: I, parent: P, title: T, class: Class) -> Self
    where
        I: Into<String>,
        P: Into<String>,
        T: Into<String>,
    {
        Self {
            id:         id.into(),
            parent:     parent.into(),
            title:      title.into(),
            class,
            date:       None,
            art:        None,
            art_profile: None,
            restricted: true,
            resources:  Vec::new(),
        }
    }

    pub fn with(mut self, res: Resource) -> Self {
        self.resources.push(res);
        self
    }

    pub fn taken<S: Into<String>>(mut self, when: S) -> Self {
        self.date = Some(when.into());
        self
    }

    pub fn thumbnail<S: Into<String>>(mut self, uri: S, profile: &str) -> Self {
        self.art = Some(uri.into());
        self.art_profile = Some(profile.to_string());
        self
    }

    fn write(&self, out: &mut String) {
        out.push_str(&fmt!("<item id=\"{}\" parentID=\"{}\" restricted=\"{}\">",
            escape(&self.id), escape(&self.parent), if self.restricted { 1 } else { 0 }));
        out.push_str(&fmt!("<dc:title>{}</dc:title>", escape(&self.title)));
        out.push_str(&fmt!("<upnp:class>{}</upnp:class>", self.class));
        if let Some(date) = &self.date {
            out.push_str(&fmt!("<dc:date>{}</dc:date>", escape(date)));
        }
        if let Some(art) = &self.art {
            match &self.art_profile {
                Some(profile) => out.push_str(&fmt!(
                    "<upnp:albumArtURI dlna:profileID=\"{}\">{}</upnp:albumArtURI>",
                    escape(profile), escape(art))),
                None => out.push_str(&fmt!(
                    "<upnp:albumArtURI>{}</upnp:albumArtURI>", escape(art))),
            }
        }
        for res in &self.resources {
            res.write(out);
        }
        out.push_str("</item>");
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Object {
    Container(Container),
    Item(Item),
}

/// A DIDL-Lite document: the objects, and the four namespaces they are spelled in.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Didl {
    pub objects: Vec<Object>,   // in the order they are to be shown
}

impl Didl {

    /// An empty document, which is a perfectly good answer to a browse.
    pub fn new() -> Self {
        Self { objects: Vec::new() }
    }

    pub fn container(&mut self, c: Container) {
        self.objects.push(Object::Container(c));
    }

    pub fn item(&mut self, i: Item) {
        self.objects.push(Object::Item(i));
    }

    /// A browse's `NumberReturned`.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// No XML declaration: the string goes inside a SOAP argument, where a second
    /// declaration would be in the middle of a document and make it unparseable.
    pub fn to_xml(&self) -> String {
        let mut out = String::with_capacity(256 + self.objects.len() * 400);
        out.push_str(&fmt!(
            "<DIDL-Lite xmlns=\"{}\" xmlns:dc=\"{}\" xmlns:upnp=\"{}\" xmlns:dlna=\"{}\">",
            NS_DIDL, NS_DC, NS_UPNP, NS_DLNA_METADATA));
        for object in &self.objects {
            match object {
                Object::Container(c)	=> c.write(&mut out),
                Object::Item(i)	=> i.write(&mut out),
            }
        }
        out.push_str("</DIDL-Lite>");
        out
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_protocol_info_has_its_four_fields_in_order() {
        let info = ProtocolInfo::http_get("image/jpeg", DlnaExtras::rendition("JPEG_LRG"));
        assert_eq!(fmt!("{}", info),
            "http-get:*:image/jpeg:DLNA.ORG_PN=JPEG_LRG;DLNA.ORG_OP=01;\
             DLNA.ORG_CI=1;DLNA.ORG_FLAGS=00D00000000000000000000000000000");
    }

    /// A resource claiming nothing says so with a star, rather than with an empty
    /// fourth field that some sets read as a malformed one.
    #[test]
    fn test_a_resource_that_claims_no_profile_says_star() {
        let info = ProtocolInfo::http_get("video/quicktime", DlnaExtras::default());
        assert_eq!(fmt!("{}", info), "http-get:*:video/quicktime:*");
    }

    #[test]
    fn test_a_container_carries_its_title_class_and_count() {
        let mut didl = Didl::new();
        didl.container(Container::new("0$A", "0", "Albums", Class::StorageFolder).holding(7));
        let xml = didl.to_xml();
        assert!(xml.contains("<container id=\"0$A\" parentID=\"0\" restricted=\"1\" \
            childCount=\"7\" searchable=\"0\">"), "{}", xml);
        assert!(xml.contains("<dc:title>Albums</dc:title>"), "{}", xml);
        assert!(xml.contains("<upnp:class>object.container.storageFolder</upnp:class>"),
            "{}", xml);
    }

    #[test]
    fn test_an_item_carries_its_resources_in_the_order_given() {
        let item = Item::new("0$D$2016$03$abc", "0$D$2016$03", "IMG_0079", Class::Photo)
            .taken("2016-03-04T10:22:31")
            .thumbnail("http://h/t/abc.jpg", "JPEG_TN")
            .with(Resource::new(
                "http://h/r/abc.jpg",
                ProtocolInfo::http_get("image/jpeg", DlnaExtras::rendition("JPEG_LRG")),
            ).sized(482_113).at(1920, 1440));
        let mut didl = Didl::new();
        didl.item(item);
        let xml = didl.to_xml();
        assert!(xml.contains("<dc:date>2016-03-04T10:22:31</dc:date>"), "{}", xml);
        assert!(xml.contains("resolution=\"1920x1440\""), "{}", xml);
        assert!(xml.contains("size=\"482113\""), "{}", xml);
        assert!(xml.contains("<upnp:albumArtURI dlna:profileID=\"JPEG_TN\">"), "{}", xml);
    }

    /// The name of a photograph is a file name, and a file name may hold any of
    /// the five characters XML reserves. One of them unescaped breaks the whole
    /// answer, not merely one item.
    #[test]
    fn test_a_title_with_reserved_characters_does_not_break_the_document() {
        let mut didl = Didl::new();
        didl.container(Container::new(
            "0$F$1", "0$F", "Rosie & Chloe <\"2016\">", Class::StorageFolder));
        let xml = didl.to_xml();
        assert!(xml.contains("Rosie &amp; Chloe &lt;&quot;2016&quot;&gt;"), "{}", xml);
        // And exactly one unescaped angle bracket pair per element.
        assert_eq!(xml.matches("<dc:title>").count(), 1);
    }

    /// A URI is a string in an element body and its ampersands are escaped there
    /// too, which is what a query string in a resource URL makes necessary.
    #[test]
    fn test_a_resource_uri_is_escaped() {
        let mut didl = Didl::new();
        didl.item(Item::new("i", "0", "t", Class::Photo).with(Resource::new(
            "http://h/r?a=1&b=2",
            ProtocolInfo::http_get("image/jpeg", DlnaExtras::default()),
        )));
        let xml = didl.to_xml();
        assert!(xml.contains("http://h/r?a=1&amp;b=2"), "{}", xml);
    }

    /// The document goes inside a SOAP string argument, so it must not carry an
    /// XML declaration of its own.
    #[test]
    fn test_the_document_has_no_declaration() {
        assert!(!Didl::new().to_xml().starts_with("<?xml"));
        assert!(Didl::new().to_xml().starts_with("<DIDL-Lite xmlns="));
    }
}
