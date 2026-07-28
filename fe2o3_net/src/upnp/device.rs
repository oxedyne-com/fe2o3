//! The two documents a control point fetches before it asks anything: the device
//! description a `LOCATION` points at, and the service description behind each
//! `SCPDURL` in it (UPnP DA 2.0 §2.3 and §2.5).
//!
//! Both are static for a given device, so both are built once and served from
//! memory. What is not static is the URLs inside them: a description reached at
//! one address must name its services at that same address, or a control point on
//! another machine follows a relative path from the wrong base. The URL fields
//! here are therefore whatever the caller puts in them, absolute or relative, and
//! the caller is the one that knows.
//!
//! # A stub is not optional
//!
//! A MediaServer must carry ConnectionManager as well as ContentDirectory, and a
//! television checks that it is there before it will browse. Everything it is
//! asked is answerable with a constant, which is why
//! [`connection_manager_scpd`] exists and why a server that skips it is refused
//! by sets that would otherwise have worked.

use crate::upnp::{
    escape,
    DLNA_DOC_DMS,
    NS_DEVICE,
    NS_DLNA_DEVICE,
    NS_SERVICE,
};

use oxedyne_fe2o3_core::prelude::*;


/// One service on a device, as the description lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Service {
    /// The service *type*, e.g. `urn:schemas-upnp-org:service:ContentDirectory:1`.
    pub service_type:   String,
    /// The service *identifier*, e.g. `urn:upnp-org:serviceId:ContentDirectory`.
    /// Not the same thing as the type, and not interchangeable with it.
    pub service_id:     String,
    /// Where the service description document is.
    pub scpd_url:       String,
    /// Where SOAP actions are posted.
    pub control_url:    String,
    /// Where a control point subscribes for events. Required in the document even
    /// by a device that never sends any.
    pub event_url:      String,
}

impl Service {

    /// A service whose three URLs sit under one prefix, which is the ordinary
    /// arrangement and the one that cannot get them out of step.
    pub fn under(service_type: &str, service_id: &str, base: &str) -> Self {
        Self {
            service_type:   service_type.to_string(),
            service_id:     service_id.to_string(),
            scpd_url:       fmt!("{}/scpd.xml", base),
            control_url:    fmt!("{}/control", base),
            event_url:      fmt!("{}/event", base),
        }
    }

    fn write(&self, out: &mut String) {
        out.push_str("<service>");
        out.push_str(&fmt!("<serviceType>{}</serviceType>", escape(&self.service_type)));
        out.push_str(&fmt!("<serviceId>{}</serviceId>", escape(&self.service_id)));
        out.push_str(&fmt!("<SCPDURL>{}</SCPDURL>", escape(&self.scpd_url)));
        out.push_str(&fmt!("<controlURL>{}</controlURL>", escape(&self.control_url)));
        out.push_str(&fmt!("<eventSubURL>{}</eventSubURL>", escape(&self.event_url)));
        out.push_str("</service>");
    }
}

/// A picture of the device, which a control point shows beside its name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Icon {
    /// The content type, e.g. `image/png`.
    pub mimetype:   String,
    /// Its width in pixels.
    pub width:      u32,
    /// Its height in pixels.
    pub height:     u32,
    /// Bits per pixel.
    pub depth:      u32,
    /// Where to fetch it.
    pub url:        String,
}

impl Icon {

    fn write(&self, out: &mut String) {
        out.push_str("<icon>");
        out.push_str(&fmt!("<mimetype>{}</mimetype>", escape(&self.mimetype)));
        out.push_str(&fmt!("<width>{}</width>", self.width));
        out.push_str(&fmt!("<height>{}</height>", self.height));
        out.push_str(&fmt!("<depth>{}</depth>", self.depth));
        out.push_str(&fmt!("<url>{}</url>", escape(&self.url)));
        out.push_str("</icon>");
    }
}

/// A root device, and everything its description document says about it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Device {
    /// The device type, e.g. [`crate::upnp::DEVICE_MEDIA_SERVER`].
    pub device_type:    String,
    /// The name a television puts on the screen. This is the one field a person
    /// ever sees, so it is the one worth choosing.
    pub friendly_name:  String,
    /// Who made it.
    pub manufacturer:   String,
    /// Their address on the web.
    pub manufacturer_url: Option<String>,
    /// A sentence about it.
    pub model_description: Option<String>,
    /// What it is called.
    pub model_name:     String,
    /// Which version of it.
    pub model_number:   Option<String>,
    /// Its address on the web.
    pub model_url:      Option<String>,
    /// A serial number, where there is one.
    pub serial_number:  Option<String>,
    /// The unique device name, `uuid:...`, which must be the same string SSDP
    /// announces and must not change between restarts.
    pub udn:            String,
    /// A page a person could open in a browser.
    pub presentation_url: Option<String>,
    /// The `<dlna:X_DLNADOC>` values, saying which DLNA device classes are
    /// claimed. [`DLNA_DOC_DMS`] is the one a media server declares.
    pub dlna_docs:      Vec<String>,
    /// Pictures of it.
    pub icons:          Vec<Icon>,
    /// What it can do.
    pub services:       Vec<Service>,
}

impl Device {

    /// A media server with the two services one must carry.
    ///
    /// `base` is the path prefix the service URLs sit under, e.g. `/dlna`; `udn`
    /// is the full `uuid:...` string.
    pub fn media_server(friendly_name: &str, udn: &str, base: &str) -> Self {
        Self {
            device_type:    super::DEVICE_MEDIA_SERVER.to_string(),
            friendly_name:  friendly_name.to_string(),
            manufacturer:   String::new(),
            manufacturer_url: None,
            model_description: None,
            model_name:     String::new(),
            model_number:   None,
            model_url:      None,
            serial_number:  None,
            udn:            udn.to_string(),
            presentation_url: None,
            dlna_docs:      vec![DLNA_DOC_DMS.to_string()],
            icons:          Vec::new(),
            services:       vec![
                Service::under(
                    super::SERVICE_CONTENT_DIRECTORY,
                    super::ID_CONTENT_DIRECTORY,
                    &fmt!("{}/cds", base),
                ),
                Service::under(
                    super::SERVICE_CONNECTION_MANAGER,
                    super::ID_CONNECTION_MANAGER,
                    &fmt!("{}/cms", base),
                ),
            ],
        }
    }

    /// The description document.
    ///
    /// `config_id` is what SSDP announces as `CONFIGID.UPNP.ORG`, and must change
    /// whenever this document does; a control point that has cached the old one
    /// otherwise never fetches the new.
    pub fn description(&self, config_id: u32) -> String {
        let mut out = String::with_capacity(1536);
        out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n");
        out.push_str(&fmt!("<root xmlns=\"{}\" xmlns:dlna=\"{}\" configId=\"{}\">",
            NS_DEVICE, NS_DLNA_DEVICE, config_id));
        // The specification version is the UPnP architecture's, not the device's,
        // and is 1.0 for every device a television speaks to.
        out.push_str("<specVersion><major>1</major><minor>0</minor></specVersion>");
        out.push_str("<device>");
        out.push_str(&fmt!("<deviceType>{}</deviceType>", escape(&self.device_type)));
        out.push_str(&fmt!("<friendlyName>{}</friendlyName>", escape(&self.friendly_name)));
        out.push_str(&fmt!("<manufacturer>{}</manufacturer>", escape(&self.manufacturer)));
        if let Some(url) = &self.manufacturer_url {
            out.push_str(&fmt!("<manufacturerURL>{}</manufacturerURL>", escape(url)));
        }
        if let Some(text) = &self.model_description {
            out.push_str(&fmt!("<modelDescription>{}</modelDescription>", escape(text)));
        }
        out.push_str(&fmt!("<modelName>{}</modelName>", escape(&self.model_name)));
        if let Some(number) = &self.model_number {
            out.push_str(&fmt!("<modelNumber>{}</modelNumber>", escape(number)));
        }
        if let Some(url) = &self.model_url {
            out.push_str(&fmt!("<modelURL>{}</modelURL>", escape(url)));
        }
        if let Some(serial) = &self.serial_number {
            out.push_str(&fmt!("<serialNumber>{}</serialNumber>", escape(serial)));
        }
        out.push_str(&fmt!("<UDN>{}</UDN>", escape(&self.udn)));
        for doc in &self.dlna_docs {
            out.push_str(&fmt!("<dlna:X_DLNADOC xmlns:dlna=\"{}\">{}</dlna:X_DLNADOC>",
                NS_DLNA_DEVICE, escape(doc)));
        }
        if !self.icons.is_empty() {
            out.push_str("<iconList>");
            for icon in &self.icons {
                icon.write(&mut out);
            }
            out.push_str("</iconList>");
        }
        out.push_str("<serviceList>");
        for service in &self.services {
            service.write(&mut out);
        }
        out.push_str("</serviceList>");
        if let Some(url) = &self.presentation_url {
            out.push_str(&fmt!("<presentationURL>{}</presentationURL>", escape(url)));
        }
        out.push_str("</device></root>");
        out
    }
}

/// The service description for ContentDirectory:1, listing the four actions a
/// browsable server implements.
///
/// `Search` is deliberately absent: a service that lists an action must implement
/// it, and a control point that finds `Search` here and gets a 401 back has been
/// lied to. `SearchCapabilities` answering with an empty string is the correct way
/// to say a server cannot search.
pub fn content_directory_scpd() -> String {
    fmt!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
<scpd xmlns=\"{}\">\
<specVersion><major>1</major><minor>0</minor></specVersion>\
<actionList>\
<action><name>GetSearchCapabilities</name><argumentList>\
<argument><name>SearchCaps</name><direction>out</direction>\
<relatedStateVariable>SearchCapabilities</relatedStateVariable></argument>\
</argumentList></action>\
<action><name>GetSortCapabilities</name><argumentList>\
<argument><name>SortCaps</name><direction>out</direction>\
<relatedStateVariable>SortCapabilities</relatedStateVariable></argument>\
</argumentList></action>\
<action><name>GetSystemUpdateID</name><argumentList>\
<argument><name>Id</name><direction>out</direction>\
<relatedStateVariable>SystemUpdateID</relatedStateVariable></argument>\
</argumentList></action>\
<action><name>Browse</name><argumentList>\
<argument><name>ObjectID</name><direction>in</direction>\
<relatedStateVariable>A_ARG_TYPE_ObjectID</relatedStateVariable></argument>\
<argument><name>BrowseFlag</name><direction>in</direction>\
<relatedStateVariable>A_ARG_TYPE_BrowseFlag</relatedStateVariable></argument>\
<argument><name>Filter</name><direction>in</direction>\
<relatedStateVariable>A_ARG_TYPE_Filter</relatedStateVariable></argument>\
<argument><name>StartingIndex</name><direction>in</direction>\
<relatedStateVariable>A_ARG_TYPE_Index</relatedStateVariable></argument>\
<argument><name>RequestedCount</name><direction>in</direction>\
<relatedStateVariable>A_ARG_TYPE_Count</relatedStateVariable></argument>\
<argument><name>SortCriteria</name><direction>in</direction>\
<relatedStateVariable>A_ARG_TYPE_SortCriteria</relatedStateVariable></argument>\
<argument><name>Result</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_Result</relatedStateVariable></argument>\
<argument><name>NumberReturned</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_Count</relatedStateVariable></argument>\
<argument><name>TotalMatches</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_Count</relatedStateVariable></argument>\
<argument><name>UpdateID</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_UpdateID</relatedStateVariable></argument>\
</argumentList></action>\
</actionList>\
<serviceStateTable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_ObjectID</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_Result</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_BrowseFlag</name>\
<dataType>string</dataType><allowedValueList>\
<allowedValue>BrowseMetadata</allowedValue>\
<allowedValue>BrowseDirectChildren</allowedValue>\
</allowedValueList></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_Filter</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_SortCriteria</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_Index</name>\
<dataType>ui4</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_Count</name>\
<dataType>ui4</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_UpdateID</name>\
<dataType>ui4</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>SearchCapabilities</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>SortCapabilities</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"yes\"><name>SystemUpdateID</name>\
<dataType>ui4</dataType></stateVariable>\
</serviceStateTable>\
</scpd>", NS_SERVICE)
}

/// The service description for ConnectionManager:1.
///
/// Every action here is answerable with a constant on a server that streams over
/// HTTP and holds no connections, which is why the service is a stub. It is not
/// optional: a MediaServer without it is refused by sets that check.
pub fn connection_manager_scpd() -> String {
    fmt!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
<scpd xmlns=\"{}\">\
<specVersion><major>1</major><minor>0</minor></specVersion>\
<actionList>\
<action><name>GetProtocolInfo</name><argumentList>\
<argument><name>Source</name><direction>out</direction>\
<relatedStateVariable>SourceProtocolInfo</relatedStateVariable></argument>\
<argument><name>Sink</name><direction>out</direction>\
<relatedStateVariable>SinkProtocolInfo</relatedStateVariable></argument>\
</argumentList></action>\
<action><name>GetCurrentConnectionIDs</name><argumentList>\
<argument><name>ConnectionIDs</name><direction>out</direction>\
<relatedStateVariable>CurrentConnectionIDs</relatedStateVariable></argument>\
</argumentList></action>\
<action><name>GetCurrentConnectionInfo</name><argumentList>\
<argument><name>ConnectionID</name><direction>in</direction>\
<relatedStateVariable>A_ARG_TYPE_ConnectionID</relatedStateVariable></argument>\
<argument><name>RcsID</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_RcsID</relatedStateVariable></argument>\
<argument><name>AVTransportID</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_AVTransportID</relatedStateVariable></argument>\
<argument><name>ProtocolInfo</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_ProtocolInfo</relatedStateVariable></argument>\
<argument><name>PeerConnectionManager</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_ConnectionManager</relatedStateVariable></argument>\
<argument><name>PeerConnectionID</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_ConnectionID</relatedStateVariable></argument>\
<argument><name>Direction</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_Direction</relatedStateVariable></argument>\
<argument><name>Status</name><direction>out</direction>\
<relatedStateVariable>A_ARG_TYPE_ConnectionStatus</relatedStateVariable></argument>\
</argumentList></action>\
</actionList>\
<serviceStateTable>\
<stateVariable sendEvents=\"yes\"><name>SourceProtocolInfo</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"yes\"><name>SinkProtocolInfo</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"yes\"><name>CurrentConnectionIDs</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_ConnectionStatus</name>\
<dataType>string</dataType><allowedValueList>\
<allowedValue>OK</allowedValue>\
<allowedValue>ContentFormatMismatch</allowedValue>\
<allowedValue>InsufficientBandwidth</allowedValue>\
<allowedValue>UnreliableChannel</allowedValue>\
<allowedValue>Unknown</allowedValue>\
</allowedValueList></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_ConnectionManager</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_Direction</name>\
<dataType>string</dataType><allowedValueList>\
<allowedValue>Input</allowedValue>\
<allowedValue>Output</allowedValue>\
</allowedValueList></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_ProtocolInfo</name>\
<dataType>string</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_ConnectionID</name>\
<dataType>i4</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_AVTransportID</name>\
<dataType>i4</dataType></stateVariable>\
<stateVariable sendEvents=\"no\"><name>A_ARG_TYPE_RcsID</name>\
<dataType>i4</dataType></stateVariable>\
</serviceStateTable>\
</scpd>", NS_SERVICE)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_media_server_carries_both_services_with_their_own_urls() -> Outcome<()> {
        let device = Device::media_server("Ochre", "uuid:1-2-3", "/dlna");
        req!(device.services.len(), 2usize);
        let xml = device.description(1);
        assert!(xml.contains("<deviceType>urn:schemas-upnp-org:device:MediaServer:1\
            </deviceType>"), "{}", xml);
        assert!(xml.contains("<UDN>uuid:1-2-3</UDN>"), "{}", xml);
        assert!(xml.contains("<controlURL>/dlna/cds/control</controlURL>"), "{}", xml);
        assert!(xml.contains("<controlURL>/dlna/cms/control</controlURL>"), "{}", xml);
        assert!(xml.contains("<SCPDURL>/dlna/cds/scpd.xml</SCPDURL>"), "{}", xml);
        assert!(xml.contains("<X_DLNADOC") || xml.contains("dlna:X_DLNADOC"), "{}", xml);
        Ok(())
    }

    /// The type and the identifier are two different strings, and swapping them is
    /// accepted by some control points and silently ignored by others.
    #[test]
    fn test_the_service_type_and_its_identifier_are_not_the_same_string() {
        let device = Device::media_server("Ochre", "uuid:1-2-3", "/dlna");
        let xml = device.description(1);
        assert!(xml.contains("<serviceType>urn:schemas-upnp-org:service:\
            ContentDirectory:1</serviceType>"), "{}", xml);
        assert!(xml.contains("<serviceId>urn:upnp-org:serviceId:ContentDirectory\
            </serviceId>"), "{}", xml);
    }

    /// A name a person chose may hold an ampersand, and one unescaped makes the
    /// description unparseable and the device undiscoverable.
    #[test]
    fn test_a_friendly_name_is_escaped() {
        let device = Device::media_server("Ben & Jerry's", "uuid:1", "/dlna");
        let xml = device.description(1);
        assert!(xml.contains("<friendlyName>Ben &amp; Jerry&apos;s</friendlyName>"),
            "{}", xml);
    }

    /// Every action a service description lists must be one the service answers,
    /// so `Search` is absent from a server that cannot search.
    #[test]
    fn test_the_content_directory_lists_only_what_it_implements() {
        let scpd = content_directory_scpd();
        for action in ["Browse", "GetSearchCapabilities", "GetSortCapabilities",
            "GetSystemUpdateID"] {
            assert!(scpd.contains(&fmt!("<name>{}</name>", action)),
                "{} is missing from the description", action);
        }
        assert!(!scpd.contains("<name>Search</name>"),
            "Search is listed by a service that does not implement it");
    }

    #[test]
    fn test_the_connection_manager_lists_its_three_actions() {
        let scpd = connection_manager_scpd();
        for action in ["GetProtocolInfo", "GetCurrentConnectionIDs",
            "GetCurrentConnectionInfo"] {
            assert!(scpd.contains(&fmt!("<name>{}</name>", action)),
                "{} is missing from the description", action);
        }
    }
}
