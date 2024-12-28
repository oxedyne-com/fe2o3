use crate::{
    charset::Charset,
    constant::SESSION_ID_KEY_LABEL,
    media::{
        ContentTypeValue,
        MediaType,
        Multipart,
    },
};

use oxedize_fe2o3_core::prelude::*;

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fmt,
    time::Duration,
};

use strum::{
    Display,
};


#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum HeaderName {
    AIM,
    Accept,
    AcceptAdditions,
    AcceptCH,
    AcceptCharset,
    AcceptDatetime,
    AcceptEncoding,
    AcceptFeatures,
    AcceptLanguage,
    AcceptPatch,
    AcceptPost,
    AcceptRanges,
    AcceptSignature,
    AccessControl,
    AccessControlAllowCredentials,
    AccessControlAllowHeaders,
    AccessControlAllowMethods,
    AccessControlAllowOrigin,
    AccessControlExposeHeaders,
    AccessControlMaxAge,
    AccessControlRequestHeaders,
    AccessControlRequestMethod,
    Age,
    Allow,
    ALPN,
    AltSvc,
    AltUsed,
    Alternates,
    AMPCacheTransform,
    ApplyToRedirectRef,
    AuthenticationControl,
    AuthenticationInfo,
    Authorization,
    CExt,
    CMan,
    COpt,
    CPEP,
    CPEPInfo,
    CacheControl,
    CacheStatus,
    CalManagedID,
    CalDAVTimezones,
    CapsuleProtocol,
    CDNCacheControl,
    CDNLoop,
    CertNotAfter,
    CertNotBefore,
    ClearSiteData,
    ClientCert,
    ClientCertChain,
    Close,
    ConfigurationContext,
    Connection,
    ContentBase,
    ContentDigest,
    ContentDisposition,
    ContentEncoding,
    ContentID,
    ContentLanguage,
    ContentLength,
    ContentLocation,
    ContentMD5,
    ContentRange,
    ContentScriptType,
    ContentSecurityPolicy,
    ContentSecurityPolicyReportOnly,
    ContentStyleType,
    ContentType,
    ContentVersion,
    Cookie,
    Cookie2,
    CrossOriginEmbedderPolicy,
    CrossOriginEmbedderPolicyReportOnly,
    CrossOriginOpenerPolicy,
    CrossOriginOpenerPolicyReportOnly,
    CrossOriginResourcePolicy,
    DASL,
    Date,
    DAV,
    DefaultStyle,
    DeltaBase,
    Depth,
    DerivedFrom,
    Destination,
    DifferentialID,
    Digest,
    DPoP,
    DPoPNonce,
    EarlyData,
    EDIINTFeatures,
    ETag,
    Expect,
    ExpectCT,
    Expires,
    Ext,
    Forwarded,
    From,
    GetProfile,
    Hobareg,
    Host,
    HTTP2Settings,
    If,
    IfMatch,
    IfModifiedSince,
    IfNoneMatch,
    IfRange,
    IfScheduleTagMatch,
    IfUnmodifiedSince,
    IM,
    IncludeReferredTokenBindingID,
    Isolation,
    KeepAlive,
    Label,
    LastEventID,
    LastModified,
    Link,
    Location,
    LockToken,
    Man,
    MaxForwards,
    MementoDatetime,
    Meter,
    MethodCheck,
    MethodCheckExpires,
    MIMEVersion,
    Negotiate,
    NEL,
    ODataEntityId,
    ODataIsolation,
    ODataMaxVersion,
    ODataVersion,
    Opt,
    OptionalWWWAuthenticate,
    OrderingType,
    Origin,
    OriginAgentCluster,
    OSCORE,
    OSLCCoreVersion,
    Overwrite,
    P3P,
    PEP,
    PEPInfo,
    PermissionsPolicy,
    PICSLabel,
    PingFrom,
    PingTo,
    Position,
    Pragma,
    Prefer,
    PreferenceApplied,
    Priority,
    ProfileObject,
    Protocol,
    ProtocolInfo,
    ProtocolQuery,
    ProtocolRequest,
    ProxyAuthenticate,
    ProxyAuthenticationInfo,
    ProxyAuthorization,
    ProxyFeatures,
    ProxyInstruction,
    ProxyStatus,
    Public,
    PublicKeyPins,
    PublicKeyPinsReportOnly,
    Range,
    RedirectRef,
    Referer,
    RefererRoot,
    Refresh,
    RepeatabilityClientID,
    RepeatabilityFirstSent,
    RepeatabilityRequestID,
    RepeatabilityResult,
    ReplayNonce,
    ReportingEndpoints,
    ReprDigest,
    RetryAfter,
    Safe,
    ScheduleReply,
    ScheduleTag,
    SecGPC,
    SecPurpose,
    SecTokenBinding,
    SecWebSocketAccept,
    SecWebSocketExtensions,
    SecWebSocketKey,
    SecWebSocketProtocol,
    SecWebSocketVersion,
    SecurityScheme,
    Server,
    ServerTiming,
    SetCookie,
    SetCookie2,
    SetProfile,
    Signature,
    SignatureInput,
    SLUG,
    SoapAction,
    StatusURI,
    StrictTransportSecurity,
    Sunset,
    SurrogateCapability,
    SurrogateControl,
    TCN,
    TE,
    Timeout,
    TimingAllowOrigin,
    Topic,
    Traceparent,
    Tracestate,
    Trailer,
    TransferEncoding,
    TTL,
    Upgrade,
    Urgency,
    URI,
    UserAgent,
    VariantVary,
    Vary,
    Via,
    WantContentDigest,
    WantDigest,
    WantReprDigest,
    Warning,
    WWWAuthenticate,
    XContentTypeOptions,
    XFrameOptions,
    NonStandard(String),
}

impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AIM                       => write!(f, "a-im"),
            Self::Accept                    => write!(f, "accept"),
            Self::AcceptAdditions           => write!(f, "accept-additions"),
            Self::AcceptCH                  => write!(f, "accept-ch"),
            Self::AcceptCharset             => write!(f, "accept-charset"),
            Self::AcceptDatetime            => write!(f, "accept-datetime"),
            Self::AcceptEncoding            => write!(f, "accept-encoding"),
            Self::AcceptFeatures            => write!(f, "accept-features"),
            Self::AcceptLanguage            => write!(f, "accept-language"),
            Self::AcceptPatch               => write!(f, "accept-patch"),
            Self::AcceptPost                => write!(f, "accept-post"),
            Self::AcceptRanges              => write!(f, "accept-ranges"),
            Self::AcceptSignature           => write!(f, "accept-signature"),
            Self::AccessControl             => write!(f, "access-control"),
            Self::AccessControlAllowCredentials => write!(f, "access-control-allow-credentials"),
            Self::AccessControlAllowHeaders     => write!(f, "access-control-allow-headers"),
            Self::AccessControlAllowMethods     => write!(f, "access-control-allow-methods"),
            Self::AccessControlAllowOrigin      => write!(f, "access-control-allow-origin"),
            Self::AccessControlExposeHeaders    => write!(f, "access-control-expose-headers"),
            Self::AccessControlMaxAge           => write!(f, "access-control-max-age"),
            Self::AccessControlRequestHeaders   => write!(f, "access-control-request-headers"),
            Self::AccessControlRequestMethod    => write!(f, "access-control-request-method"),
            Self::Age                       => write!(f, "age"),
            Self::Allow                     => write!(f, "allow"),
            Self::ALPN                      => write!(f, "alpn"),
            Self::AltSvc                    => write!(f, "alt-svc"),
            Self::AltUsed                   => write!(f, "alt-used"),
            Self::Alternates                => write!(f, "alternates"),
            Self::AMPCacheTransform         => write!(f, "amp-cache-transform"),
            Self::ApplyToRedirectRef        => write!(f, "apply-to-redirect-ref"),
            Self::AuthenticationControl     => write!(f, "authentication-control"),
            Self::AuthenticationInfo        => write!(f, "authentication-info"),
            Self::Authorization             => write!(f, "authorization"),
            Self::CExt                      => write!(f, "c-ext"),
            Self::CMan                      => write!(f, "c-man"),
            Self::COpt                      => write!(f, "c-opt"),
            Self::CPEP                      => write!(f, "c-pep"),
            Self::CPEPInfo                  => write!(f, "c-pep-info"),
            Self::CacheControl              => write!(f, "cache-control"),
            Self::CacheStatus               => write!(f, "cache-status"),
            Self::CalManagedID              => write!(f, "cal-managed-id"),
            Self::CalDAVTimezones           => write!(f, "caldav-timezones"),
            Self::CapsuleProtocol           => write!(f, "capsule-protocol"),
            Self::CDNCacheControl           => write!(f, "cdn-cache-control"),
            Self::CDNLoop                   => write!(f, "cdn-loop"),
            Self::CertNotAfter              => write!(f, "cert-not-after"),
            Self::CertNotBefore             => write!(f, "cert-not-before"),
            Self::ClearSiteData             => write!(f, "clear-site-data"),
            Self::ClientCert                => write!(f, "client-cert"),
            Self::ClientCertChain           => write!(f, "client-cert-chain"),
            Self::Close                     => write!(f, "close"),
            Self::ConfigurationContext      => write!(f, "configuration-context"),
            Self::Connection                => write!(f, "connection"),
            Self::ContentBase               => write!(f, "content-base"),
            Self::ContentDigest             => write!(f, "content-digest"),
            Self::ContentDisposition        => write!(f, "content-disposition"),
            Self::ContentEncoding           => write!(f, "content-encoding"),
            Self::ContentID                 => write!(f, "content-id"),
            Self::ContentLanguage           => write!(f, "content-language"),
            Self::ContentLength             => write!(f, "content-length"),
            Self::ContentLocation           => write!(f, "content-location"),
            Self::ContentMD5                => write!(f, "content-md5"),
            Self::ContentRange              => write!(f, "content-range"),
            Self::ContentScriptType         => write!(f, "content-script-type"),
            Self::ContentSecurityPolicy     => write!(f, "content-security-policy"),
            Self::ContentSecurityPolicyReportOnly => write!(f, "content-security-policy-report-only"),
            Self::ContentStyleType          => write!(f, "content-style-type"),
            Self::ContentType               => write!(f, "content-type"),
            Self::ContentVersion            => write!(f, "content-version"),
            Self::Cookie                    => write!(f, "cookie"),
            Self::Cookie2                   => write!(f, "cookie2"),
            Self::CrossOriginEmbedderPolicy => write!(f, "cross-origin-embedder-policy"),
            Self::CrossOriginEmbedderPolicyReportOnly   => write!(f, "cross-origin-embedder-policy-report-only"),
            Self::CrossOriginOpenerPolicy               => write!(f, "cross-origin-opener-policy"),
            Self::CrossOriginOpenerPolicyReportOnly     => write!(f, "cross-origin-opener-policy-report-only"),
            Self::CrossOriginResourcePolicy => write!(f, "cross-origin-resource-policy"),
            Self::DASL                      => write!(f, "dasl"),
            Self::Date                      => write!(f, "date"),
            Self::DAV                       => write!(f, "dav"),
            Self::DefaultStyle              => write!(f, "default-style"),
            Self::DeltaBase                 => write!(f, "delta-base"),
            Self::Depth                     => write!(f, "depth"),
            Self::DerivedFrom               => write!(f, "derived-from"),
            Self::Destination               => write!(f, "destination"),
            Self::DifferentialID            => write!(f, "differential-id"),
            Self::Digest                    => write!(f, "digest"),
            Self::DPoP                      => write!(f, "dpop"),
            Self::DPoPNonce                 => write!(f, "dpop-nonce"),
            Self::EarlyData                 => write!(f, "early-data"),
            Self::EDIINTFeatures            => write!(f, "ediint-features"),
            Self::ETag                      => write!(f, "etag"),
            Self::Expect                    => write!(f, "expect"),
            Self::ExpectCT                  => write!(f, "expect-ct"),
            Self::Expires                   => write!(f, "expires"),
            Self::Ext                       => write!(f, "ext"),
            Self::Forwarded                 => write!(f, "forwarded"),
            Self::From                      => write!(f, "from"),
            Self::GetProfile                => write!(f, "getprofile"),
            Self::Hobareg                   => write!(f, "hobareg"),
            Self::Host                      => write!(f, "host"),
            Self::HTTP2Settings             => write!(f, "http2-settings"),
            Self::If                        => write!(f, "if"),
            Self::IfMatch                   => write!(f, "if-match"),
            Self::IfModifiedSince           => write!(f, "if-modified-since"),
            Self::IfNoneMatch               => write!(f, "if-none-match"),
            Self::IfRange                   => write!(f, "if-range"),
            Self::IfScheduleTagMatch        => write!(f, "if-schedule-tag-match"),
            Self::IfUnmodifiedSince         => write!(f, "if-unmodified-since"),
            Self::IM                        => write!(f, "im"),
            Self::IncludeReferredTokenBindingID => write!(f, "include-referred-token-binding-id"),
            Self::Isolation                 => write!(f, "isolation"),
            Self::KeepAlive                 => write!(f, "keep-alive"),
            Self::Label                     => write!(f, "label"),
            Self::LastEventID               => write!(f, "last-event-id"),
            Self::LastModified              => write!(f, "last-modified"),
            Self::Link                      => write!(f, "link"),
            Self::Location                  => write!(f, "location"),
            Self::LockToken                 => write!(f, "lock-token"),
            Self::Man                       => write!(f, "man"),
            Self::MaxForwards               => write!(f, "max-forwards"),
            Self::MementoDatetime           => write!(f, "memento-datetime"),
            Self::Meter                     => write!(f, "meter"),
            Self::MethodCheck               => write!(f, "method-check"),
            Self::MethodCheckExpires        => write!(f, "method-check-expires"),
            Self::MIMEVersion               => write!(f, "mime-version"),
            Self::Negotiate                 => write!(f, "negotiate"),
            Self::NEL                       => write!(f, "nel"),
            Self::ODataEntityId             => write!(f, "odata-entityid"),
            Self::ODataIsolation            => write!(f, "odata-isolation"),
            Self::ODataMaxVersion           => write!(f, "odata-maxversion"),
            Self::ODataVersion              => write!(f, "odata-version"),
            Self::Opt                       => write!(f, "opt"),
            Self::OptionalWWWAuthenticate   => write!(f, "optional-www-authenticate"),
            Self::OrderingType              => write!(f, "ordering-type"),
            Self::Origin                    => write!(f, "origin"),
            Self::OriginAgentCluster        => write!(f, "origin-agent-cluster"),
            Self::OSCORE                    => write!(f, "oscore"),
            Self::OSLCCoreVersion           => write!(f, "oslc-core-version"),
            Self::Overwrite                 => write!(f, "overwrite"),
            Self::P3P                       => write!(f, "p3p"),
            Self::PEP                       => write!(f, "pep"),
            Self::PEPInfo                   => write!(f, "pep-info"),
            Self::PermissionsPolicy         => write!(f, "permissions-policy"),
            Self::PICSLabel                 => write!(f, "pics-label"),
            Self::PingFrom                  => write!(f, "ping-from"),
            Self::PingTo                    => write!(f, "ping-to"),
            Self::Position                  => write!(f, "position"),
            Self::Pragma                    => write!(f, "pragma"),
            Self::Prefer                    => write!(f, "prefer"),
            Self::PreferenceApplied         => write!(f, "preference-applied"),
            Self::Priority                  => write!(f, "priority"),
            Self::ProfileObject             => write!(f, "profileobject"),
            Self::Protocol                  => write!(f, "protocol"),
            Self::ProtocolInfo              => write!(f, "protocol-info"),
            Self::ProtocolQuery             => write!(f, "protocol-query"),
            Self::ProtocolRequest           => write!(f, "protocol-request"),
            Self::ProxyAuthenticate         => write!(f, "proxy-authenticate"),
            Self::ProxyAuthenticationInfo   => write!(f, "proxy-authentication-info"),
            Self::ProxyAuthorization        => write!(f, "proxy-authorization"),
            Self::ProxyFeatures             => write!(f, "proxy-features"),
            Self::ProxyInstruction          => write!(f, "proxy-instruction"),
            Self::ProxyStatus               => write!(f, "proxy-status"),
            Self::Public                    => write!(f, "public"),
            Self::PublicKeyPins             => write!(f, "public-key-pins"),
            Self::PublicKeyPinsReportOnly   => write!(f, "public-key-pins-report-only"),
            Self::Range                     => write!(f, "range"),
            Self::RedirectRef               => write!(f, "redirect-ref"),
            Self::Referer                   => write!(f, "referer"),
            Self::RefererRoot               => write!(f, "referer-root"),
            Self::Refresh                   => write!(f, "refresh"),
            Self::RepeatabilityClientID     => write!(f, "repeatability-client-id"),
            Self::RepeatabilityFirstSent    => write!(f, "repeatability-first-sent"),
            Self::RepeatabilityRequestID    => write!(f, "repeatability-request-id"),
            Self::RepeatabilityResult       => write!(f, "repeatability-result"),
            Self::ReplayNonce               => write!(f, "replay-nonce"),
            Self::ReportingEndpoints        => write!(f, "reporting-endpoints"),
            Self::ReprDigest                => write!(f, "repr-digest"),
            Self::RetryAfter                => write!(f, "retry-after"),
            Self::Safe                      => write!(f, "safe"),
            Self::ScheduleReply             => write!(f, "schedule-reply"),
            Self::ScheduleTag               => write!(f, "schedule-tag"),
            Self::SecGPC                    => write!(f, "sec-gpc"),
            Self::SecPurpose                => write!(f, "sec-purpose"),
            Self::SecTokenBinding           => write!(f, "sec-token-binding"),
            Self::SecWebSocketAccept        => write!(f, "sec-websocket-accept"),
            Self::SecWebSocketExtensions    => write!(f, "sec-websocket-extensions"),
            Self::SecWebSocketKey           => write!(f, "sec-websocket-key"),
            Self::SecWebSocketProtocol      => write!(f, "sec-websocket-protocol"),
            Self::SecWebSocketVersion       => write!(f, "sec-websocket-version"),
            Self::SecurityScheme            => write!(f, "security-scheme"),
            Self::Server                    => write!(f, "server"),
            Self::ServerTiming              => write!(f, "server-timing"),
            Self::SetCookie                 => write!(f, "set-cookie"),
            Self::SetCookie2                => write!(f, "set-cookie2"),
            Self::SetProfile                => write!(f, "setprofile"),
            Self::Signature                 => write!(f, "signature"),
            Self::SignatureInput            => write!(f, "signature-input"),
            Self::SLUG                      => write!(f, "slug"),
            Self::SoapAction                => write!(f, "soapaction"),
            Self::StatusURI                 => write!(f, "status-uri"),
            Self::StrictTransportSecurity   => write!(f, "strict-transport-security"),
            Self::Sunset                    => write!(f, "sunset"),
            Self::SurrogateCapability       => write!(f, "surrogate-capability"),
            Self::SurrogateControl          => write!(f, "surrogate-control"),
            Self::TCN                       => write!(f, "tcn"),
            Self::TE                        => write!(f, "te"),
            Self::Timeout                   => write!(f, "timeout"),
            Self::TimingAllowOrigin         => write!(f, "timing-allow-origin"),
            Self::Topic                     => write!(f, "topic"),
            Self::Traceparent               => write!(f, "traceparent"),
            Self::Tracestate                => write!(f, "tracestate"),
            Self::Trailer                   => write!(f, "trailer"),
            Self::TransferEncoding          => write!(f, "transfer-encoding"),
            Self::TTL                       => write!(f, "ttl"),
            Self::Upgrade                   => write!(f, "upgrade"),
            Self::Urgency                   => write!(f, "urgency"),
            Self::URI                       => write!(f, "uri"),
            Self::UserAgent                 => write!(f, "user-agent"),
            Self::VariantVary               => write!(f, "variant-vary"),
            Self::Vary                      => write!(f, "vary"),
            Self::Via                       => write!(f, "via"),
            Self::WantContentDigest         => write!(f, "want-content-digest"),
            Self::WantDigest                => write!(f, "want-digest"),
            Self::WantReprDigest            => write!(f, "want-repr-digest"),
            Self::Warning                   => write!(f, "warning"),
            Self::WWWAuthenticate           => write!(f, "www-authenticate"),
            Self::XContentTypeOptions       => write!(f, "x-content-type-options"),
            Self::XFrameOptions             => write!(f, "x-frame-options"),
            Self::NonStandard(s)            => write!(f, "{}", s),
        }
    }
}

impl From<&str> for HeaderName {
    fn from(s: &str) -> Self {
        match s {
            "a-im"				            => Self::AIM,
            "accept"			            => Self::Accept,
            "accept-additions"	            => Self::AcceptAdditions,
            "accept-ch"			            => Self::AcceptCH,
            "accept-charset"	            => Self::AcceptCharset,
            "accept-datetime"	            => Self::AcceptDatetime,
            "accept-encoding"	            => Self::AcceptEncoding,
            "accept-features"	            => Self::AcceptFeatures,
            "accept-language"	            => Self::AcceptLanguage,
            "accept-patch"		            => Self::AcceptPatch,
            "accept-post"		            => Self::AcceptPost,
            "accept-ranges"		            => Self::AcceptRanges,
            "accept-signature"	            => Self::AcceptSignature,
            "access-control"	            => Self::AccessControl,
            "access-control-allow-credentials"  => Self::AccessControlAllowCredentials,
            "access-control-allow-headers"		=> Self::AccessControlAllowHeaders,
            "access-control-allow-methods"		=> Self::AccessControlAllowMethods,
            "access-control-allow-origin"		=> Self::AccessControlAllowOrigin,
            "access-control-expose-headers"		=> Self::AccessControlExposeHeaders,
            "access-control-max-age"			=> Self::AccessControlMaxAge,
            "access-control-request-headers"	=> Self::AccessControlRequestHeaders,
            "access-control-request-method"		=> Self::AccessControlRequestMethod,
            "age"				            => Self::Age,
            "allow"					        => Self::Allow,
            "alpn"					        => Self::ALPN,
            "alt-svc"					    => Self::AltSvc,
            "alt-used"					    => Self::AltUsed,
            "alternates"				    => Self::Alternates,
            "amp-cache-transform"		    => Self::AMPCacheTransform,
            "apply-to-redirect-ref"		    => Self::ApplyToRedirectRef,
            "authentication-control"	    => Self::AuthenticationControl,
            "authentication-info"		    => Self::AuthenticationInfo,
            "authorization"				    => Self::Authorization,
            "c-ext"					        => Self::CExt,
            "c-man"					        => Self::CMan,
            "c-opt"					        => Self::COpt,
            "c-pep"					        => Self::CPEP,
            "c-pep-info"				    => Self::CPEPInfo,
            "cache-control"				    => Self::CacheControl,
            "cache-status"				    => Self::CacheStatus,
            "cal-managed-id"			    => Self::CalManagedID,
            "caldav-timezones"			    => Self::CalDAVTimezones,
            "capsule-protocol"			    => Self::CapsuleProtocol,
            "cdn-cache-control"			    => Self::CDNCacheControl,
            "cdn-loop"					    => Self::CDNLoop,
            "cert-not-after"			    => Self::CertNotAfter,
            "cert-not-before"			    => Self::CertNotBefore,
            "clear-site-data"			    => Self::ClearSiteData,
            "client-cert"				    => Self::ClientCert,
            "client-cert-chain"			    => Self::ClientCertChain,
            "close"					        => Self::Close,
            "configuration-context"		    => Self::ConfigurationContext,
            "connection"				    => Self::Connection,
            "content-base"				    => Self::ContentBase,
            "content-digest"			    => Self::ContentDigest,
            "content-disposition"		    => Self::ContentDisposition,
            "content-encoding"			    => Self::ContentEncoding,
            "content-id"				    => Self::ContentID,
            "content-language"			    => Self::ContentLanguage,
            "content-length"			    => Self::ContentLength,
            "content-location"			    => Self::ContentLocation,
            "content-md5"				    => Self::ContentMD5,
            "content-range"				    => Self::ContentRange,
            "content-script-type"		    => Self::ContentScriptType,
            "content-security-policy"	    => Self::ContentSecurityPolicy,
            "content-security-policy-report-only" => Self::ContentSecurityPolicyReportOnly,
            "content-style-type"			=> Self::ContentStyleType,
            "content-type"					=> Self::ContentType,
            "content-version"				=> Self::ContentVersion,
            "cookie"					    => Self::Cookie,
            "cookie2"					    => Self::Cookie2,
            "cross-origin-embedder-policy"				=> Self::CrossOriginEmbedderPolicy,
            "cross-origin-embedder-policy-report-only"	=> Self::CrossOriginEmbedderPolicyReportOnly,
            "cross-origin-opener-policy"				=> Self::CrossOriginOpenerPolicy,
            "cross-origin-opener-policy-report-only"	=> Self::CrossOriginOpenerPolicyReportOnly,
            "cross-origin-resource-policy"				=> Self::CrossOriginResourcePolicy,
            "dasl"					        => Self::DASL,
            "date"					        => Self::Date,
            "dav"					        => Self::DAV,
            "default-style"			        => Self::DefaultStyle,
            "delta-base"			        => Self::DeltaBase,
            "depth"					        => Self::Depth,
            "derived-from"			        => Self::DerivedFrom,
            "destination"			        => Self::Destination,
            "differential-id"		        => Self::DifferentialID,
            "digest"				        => Self::Digest,
            "dpop"					        => Self::DPoP,
            "dpop-nonce"			        => Self::DPoPNonce,
            "early-data"			        => Self::EarlyData,
            "ediint-features"		        => Self::EDIINTFeatures,
            "etag"					        => Self::ETag,
            "expect"				        => Self::Expect,
            "expect-ct"				        => Self::ExpectCT,
            "expires"				        => Self::Expires,
            "ext"					        => Self::Ext,
            "forwarded"				        => Self::Forwarded,
            "from"					        => Self::From,
            "getprofile"			        => Self::GetProfile,
            "hobareg"				        => Self::Hobareg,
            "host"					        => Self::Host,
            "http2-settings"		        => Self::HTTP2Settings,
            "if"					        => Self::If,
            "if-match"				        => Self::IfMatch,
            "if-modified-since"		        => Self::IfModifiedSince,
            "if-none-match"			        => Self::IfNoneMatch,
            "if-range"				        => Self::IfRange,
            "if-schedule-tag-match"	        => Self::IfScheduleTagMatch,
            "if-unmodified-since"	        => Self::IfUnmodifiedSince,
            "im"					        => Self::IM,
            "include-referred-token-binding-id"	=> Self::IncludeReferredTokenBindingID,
            "isolation"				        => Self::Isolation,
            "keep-alive"			        => Self::KeepAlive,
            "label"					        => Self::Label,
            "last-event-id"			        => Self::LastEventID,
            "last-modified"			        => Self::LastModified,
            "link"					        => Self::Link,
            "location"				        => Self::Location,
            "lock-token"			        => Self::LockToken,
            "man"					        => Self::Man,
            "max-forwards"			        => Self::MaxForwards,
            "memento-datetime"		        => Self::MementoDatetime,
            "meter"					        => Self::Meter,
            "method-check"			        => Self::MethodCheck,
            "method-check-expires"	        => Self::MethodCheckExpires,
            "mime-version"			        => Self::MIMEVersion,
            "negotiate"				        => Self::Negotiate,
            "nel"					        => Self::NEL,
            "odata-entityid"		        => Self::ODataEntityId,
            "odata-isolation"		        => Self::ODataIsolation,
            "odata-maxversion"		        => Self::ODataMaxVersion,
            "odata-version"			        => Self::ODataVersion,
            "opt"					        => Self::Opt,
            "optional-www-authenticate"	    => Self::OptionalWWWAuthenticate,
            "ordering-type"				    => Self::OrderingType,
            "origin"				        => Self::Origin,
            "origin-agent-cluster"	        => Self::OriginAgentCluster,
            "oscore"				        => Self::OSCORE,
            "oslc-core-version"		        => Self::OSLCCoreVersion,
            "overwrite"				        => Self::Overwrite,
            "p3p"					        => Self::P3P,
            "pep"					        => Self::PEP,
            "pep-info"				        => Self::PEPInfo,
            "permissions-policy"	        => Self::PermissionsPolicy,
            "pics-label"			        => Self::PICSLabel,
            "ping-from"				        => Self::PingFrom,
            "ping-to"				        => Self::PingTo,
            "position"				        => Self::Position,
            "pragma"				        => Self::Pragma,
            "prefer"				        => Self::Prefer,
            "preference-applied"	        => Self::PreferenceApplied,
            "priority"				        => Self::Priority,
            "profileobject"			        => Self::ProfileObject,
            "protocol"				        => Self::Protocol,
            "protocol-info"			        => Self::ProtocolInfo,
            "protocol-query"		        => Self::ProtocolQuery,
            "protocol-request"		        => Self::ProtocolRequest,
            "proxy-authenticate"	        => Self::ProxyAuthenticate,
            "proxy-authentication-info"	    => Self::ProxyAuthenticationInfo,
            "proxy-authorization"		    => Self::ProxyAuthorization,
            "proxy-features"			    => Self::ProxyFeatures,
            "proxy-instruction"			    => Self::ProxyInstruction,
            "proxy-status"				    => Self::ProxyStatus,
            "public"					    => Self::Public,
            "public-key-pins"			    => Self::PublicKeyPins,
            "public-key-pins-report-only"   => Self::PublicKeyPinsReportOnly,
            "range"					        => Self::Range,
            "redirect-ref"				    => Self::RedirectRef,
            "referer"					    => Self::Referer,
            "referer-root"				    => Self::RefererRoot,
            "refresh"					    => Self::Refresh,
            "repeatability-client-id"	    => Self::RepeatabilityClientID,
            "repeatability-first-sent"	    => Self::RepeatabilityFirstSent,
            "repeatability-request-id"	    => Self::RepeatabilityRequestID,
            "repeatability-result"		    => Self::RepeatabilityResult,
            "replay-nonce"				    => Self::ReplayNonce,
            "reporting-endpoints"		    => Self::ReportingEndpoints,
            "repr-digest"				    => Self::ReprDigest,
            "retry-after"				    => Self::RetryAfter,
            "safe"					        => Self::Safe,
            "schedule-reply"			    => Self::ScheduleReply,
            "schedule-tag"				    => Self::ScheduleTag,
            "sec-gpc"					    => Self::SecGPC,
            "sec-purpose"				    => Self::SecPurpose,
            "sec-token-binding"			    => Self::SecTokenBinding,
            "sec-websocket-accept"		    => Self::SecWebSocketAccept,
            "sec-websocket-extensions"	    => Self::SecWebSocketExtensions,
            "sec-websocket-key"			    => Self::SecWebSocketKey,
            "sec-websocket-protocol"	    => Self::SecWebSocketProtocol,
            "sec-websocket-version"		    => Self::SecWebSocketVersion,
            "security-scheme"			    => Self::SecurityScheme,
            "server"					    => Self::Server,
            "server-timing"				    => Self::ServerTiming,
            "set-cookie"				    => Self::SetCookie,
            "set-cookie2"				    => Self::SetCookie2,
            "setprofile"				    => Self::SetProfile,
            "signature"					    => Self::Signature,
            "signature-input"			    => Self::SignatureInput,
            "slug"					        => Self::SLUG,
            "soapaction"				    => Self::SoapAction,
            "status-uri"				    => Self::StatusURI,
            "strict-transport-security"	    => Self::StrictTransportSecurity,
            "sunset"					    => Self::Sunset,
            "surrogate-capability"		    => Self::SurrogateCapability,
            "surrogate-control"			    => Self::SurrogateControl,
            "tcn"					        => Self::TCN,
            "te"					        => Self::TE,
            "timeout"				        => Self::Timeout,
            "timing-allow-origin"	        => Self::TimingAllowOrigin,
            "topic"					        => Self::Topic,
            "traceparent"			        => Self::Traceparent,
            "tracestate"			        => Self::Tracestate,
            "trailer"				        => Self::Trailer,
            "transfer-encoding"		        => Self::TransferEncoding,
            "ttl"					        => Self::TTL,
            "upgrade"				        => Self::Upgrade,
            "urgency"				        => Self::Urgency,
            "uri"					        => Self::URI,
            "user-agent"			        => Self::UserAgent,
            "variant-vary"			        => Self::VariantVary,
            "vary"					        => Self::Vary,
            "via"					        => Self::Via,
            "want-content-digest"		    => Self::WantContentDigest,
            "want-digest"				    => Self::WantDigest,
            "want-repr-digest"			    => Self::WantReprDigest,
            "warning"					    => Self::Warning,
            "www-authenticate"			    => Self::WWWAuthenticate,
            "x-content-type-options"	    => Self::XContentTypeOptions,
            "x-frame-options"			    => Self::XFrameOptions,
            _ => Self::NonStandard(s.to_string()),
        }
    }
}
impl From<&String> for HeaderName {
    fn from(s: &String) -> Self {
        Self::from(s.as_str())
    }
}

impl From<String> for HeaderName {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

impl HeaderName {
    /// Categorise `HeaderNames`s mainly for ordering.
    pub fn category(&self) -> HeaderFieldCategory {
        match self {
            Self::CacheControl      |
            Self::Connection        |
            Self::Date              |
            Self::Pragma            |
            Self::Trailer           |
            Self::TransferEncoding  |
            Self::Upgrade           |
            Self::Via               |
            Self::Warning           => HeaderFieldCategory::General,
            Self::Accept                |
            Self::AcceptCharset         |
            Self::AcceptEncoding        |
            Self::AcceptLanguage        |
            Self::Authorization         |
            Self::Expect                |
            Self::From                  |
            Self::Host                  |
            Self::IfMatch               |
            Self::IfModifiedSince       |
            Self::IfNoneMatch           |
            Self::IfRange               |
            Self::IfUnmodifiedSince     |
            Self::MaxForwards           |
            Self::ProxyAuthorization    |
            Self::Range                 |
            Self::Referer               |
            Self::TE                    |
            Self::UserAgent             => HeaderFieldCategory::Request,
            Self::AcceptRanges          |
            Self::Age                   |
            Self::ETag                  |
            Self::Location              |
            Self::ProxyAuthenticate     |
            Self::RetryAfter            |
            Self::Server                |
            Self::Vary                  |
            Self::WWWAuthenticate       => HeaderFieldCategory::Response,
            Self::Allow             |
            Self::ContentEncoding   |
            Self::ContentLanguage   |
            Self::ContentLength     |
            Self::ContentLocation   |
            Self::ContentMD5        |
            Self::ContentRange      |
            Self::ContentType       |
            Self::Expires           |
            Self::LastModified      => HeaderFieldCategory::Entity,
            _ => HeaderFieldCategory::Other,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cookie {
    pub key:    String,
    pub val:    String,
    pub attrs:  Option<BTreeSet<SetCookieAttributes>>,
}

#[derive(Clone, Debug, Display, Eq, Ord, PartialEq, PartialOrd)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SetCookieAttributes {
    Domain(String),
    Expires((Duration, String)),
    HttpOnly,
    MaxAge(u32),
    Partitioned,
    Path(String),
    SameSite(SameSite),
    Secure,
}

impl fmt::Display for SetCookieAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(s)         => write!(f, "Domain={}", s),
            Self::Expires((_dt, s)) => write!(f, "Expires={}", s),
            Self::HttpOnly          => write!(f, "HttpOnly"),
            Self::MaxAge(n)         => write!(f, "Max-Age={}", n),
            Self::Partitioned       => write!(f, "Partitioned"),
            Self::Path(s)           => write!(f, "Path={}", s),
            Self::SameSite(same_site) => match same_site {
                SameSite::None => write!(f, "SameSite={}; Secure", same_site),
                _ => write!(f, "SameSite={}", same_site),
            },
            Self::Secure            => write!(f, "Secure"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionType {
    Close,
    KeepAlive,
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Close     => write!(f, "close"),
            Self::KeepAlive => write!(f, "keep-alive"),
        }
    }
}

impl FromStr for ConnectionType {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "close"         => Self::Close,
            "keep-alive"    => Self::KeepAlive,
            _ => return Err(err!(
                "Unrecognised connection type '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

impl ConnectionType {
    pub fn new(close: bool) -> Self {
        match close {
            true => Self::Close,
            _ => Self::KeepAlive,
        }
    }
}

#[derive(Clone, Debug)]
pub enum HeaderFieldValue {
    Generic(String),
    // Encapsulated:
    Connection(Option<ConnectionType>, Vec<String>),
    Cookie(Vec<Cookie>),
    ContentLength(usize),
    ContentType(ContentTypeValue),
    SecWebSocketKey(String),
    SetCookie(Cookie),
    Upgrade(Vec<String>),
}

impl fmt::Display for HeaderFieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(ct_opt, list) => if list.len() > 0 {
                write!(f, "{:?}, {}", ct_opt, list.join(", "))
            } else {
                write!(f, "{:?}", ct_opt)
            }
            Self::Cookie(list) => {
                let s = list.iter()
                    .map(|cookie| fmt!("{}={}", cookie.key, cookie.val))
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "{}", s)
            }
            Self::ContentLength(n) => write!(f, "{}", n),
            Self::ContentType(ctv) => write!(f, "{}", ctv),
            Self::Generic(s) => write!(f, "{}", s),
            Self::SecWebSocketKey(s) => write!(f, "{}", s),
            Self::SetCookie(Cookie { key, val, attrs }) => {
                if let Some(attrs_set) = attrs { 
                    if attrs_set.len() > 0 {
                        let s = attrs_set.iter()
                            .map(|attr| fmt!("{}", attr))
                            .collect::<Vec<_>>()
                            .join("; ");
                        write!(f, "{}={}; {}", key, val, s)
                    } else {
                        write!(f, "{}={}", key, val)
                    }
                } else {
                    write!(f, "{}={}", key, val)
                }
            }
            Self::Upgrade(list) => write!(f, "{}", list.join(", ")),
        }
    }
}

impl HeaderFieldValue {
    pub fn new(name: &HeaderName, value: &str) -> Outcome<Self> {
        Ok(match name {
            HeaderName::Connection => {
                let parts = value.split(',').map(str::trim).map(|w| w.to_lowercase());
                let mut contyp_opt = None;
                let mut list = Vec::new();
                for part in parts {
                    match ConnectionType::from_str(&part) {
                        Ok(ct) => if contyp_opt.is_some() {
                            return Err(err!(
                                "Connection type '{:?}' already defined, '{}' is redundant.",
                                contyp_opt, ct;
                            Invalid, Input, Conflict));
                        } else {
                            contyp_opt = Some(ct);
                        },
                        Err(_) => list.push(part),
                    }
                }
                Self::Connection(contyp_opt, list)
            },
            HeaderName::Cookie => {
                let mut list = Vec::new();
                for pair in value.split(';').map(str::trim) {
                    let mut parts = pair.split('=').map(str::trim);
                    let k = parts.next();
                    let v = parts.next();
                    match (k, v) {
                        (Some(k), Some(v)) => {
                            list.push(Cookie {
                                key: k.to_string(),
                                val: v.to_string(),
                                attrs: None,
                            });
                        },
                        (Some(k), None) => return Err(err!(
                            "Missing value for key '{}' in header field line '{}", k, value;
                        Invalid, Input, Missing)),
                        (None, Some(v)) => return Err(err!(
                            "Missing key for value '{}' in header field line '{}", v, value;
                        Invalid, Input, Missing)),
                        _ => (),
                    }
                }
                Self::Cookie(list)
            },
            HeaderName::ContentLength => match value.parse() { 
                Ok(n) => Self::ContentLength(n),
                Err(e) => return Err(err!(e,
                    "Could not parse the content length '{}'.", value;
                Invalid, Input, String, Decode)),
            },
            HeaderName::ContentType => { // A; B=C
                let mut parts = value.split(';').map(str::trim).map(|w| w.to_lowercase());
                match parts.next() { // A
                    Some(first) => {
                        let media_type = res!(MediaType::from_str(&first));
                        let is_multipart = match media_type {
                            MediaType::Multipart(Multipart::FormData) => true,
                            _ => false,
                        };
                        match parts.next() { // B=C
                            Some(second) => {
                                let mut parts2 = second.split('=')
                                    .map(str::trim)
                                    .map(|w| w.to_lowercase());
                                match parts2.next() { // B
                                    Some(left) => match is_multipart {
                                        true => if left != "boundary" {
                                            return Err(err!(
                                                "Expected 'boundary' found '{}'.", left;
                                            Invalid, Input, String, Decode));
                                        },
                                        false => if left != "charset" {
                                            return Err(err!(
                                                "Expected 'charset' found '{}'.", left;
                                            Invalid, Input, String, Decode));
                                        },
                                    },
                                    None => (),
                                }
                                match parts2.next() { // C
                                    Some(right) => match is_multipart {
                                        true => Self::ContentType(ContentTypeValue::Multipart((
                                            Multipart::FormData,
                                            right.to_string(),
                                        ))),
                                        false => Self::ContentType(ContentTypeValue::MediaType((
                                            media_type,
                                            Some(res!(Charset::from_str(&right))),
                                        ))),
                                    },
                                    None => return Err(err!("Missing {} value in '{}'.",
                                        if is_multipart { "boundary" } else { "charset" }, value;
                                    Invalid, Input, String, Decode, Missing)),
                                }
                            },
                            None => match is_multipart {
                                true => return Err(err!(
                                    "A 'boundary=string' is required but is missing in {}'.", value;
                                Invalid, Input, String, Decode, Missing)),
                                false => Self::ContentType(ContentTypeValue::MediaType((
                                    media_type,
                                    None,
                                ))),
                            },
                        }
                    },
                    None => return Err(err!(
                        "Missing header field value for Content-Type.";
                    Invalid, Input, String, Decode, Missing)),
                }
            },
            HeaderName::SecWebSocketKey     |
            HeaderName::SecWebSocketAccept  => Self::SecWebSocketKey(value.to_string()),
            HeaderName::Upgrade => {
                let list: Vec<_> = value
                    .split(',').map(str::trim).map(|w| w.to_lowercase()).collect();
                Self::Upgrade(list)
            },
            // TODO encapsulate more header field types
            _ => {
                trace!("Header field '{}' not encapsulated.", name);
                Self::Generic(value.to_string())
            }
        })
    }
}

#[derive(Clone, Debug)]
pub struct HeaderField {
    pub name:   HeaderName,
    pub value:  HeaderFieldValue,
}

impl HeaderField {

    pub fn new(line: &str, line_num: Option<u16>) -> Outcome<Self> {
        let mut parts = line.splitn(2, ':').map(str::trim);
        let line_msg = match line_num {
            Some(n) => fmt!(" at HTTP message line number {}.", n),
            None => ".".to_string(),
        };
        let name = match parts.next() {
            Some(name) => HeaderName::from(&name.to_lowercase()),
            None => return Err(err!(
                "A name was not found for the HTTP header field in the header line '{}'{}",
                line, line_msg;
            Invalid, Input, Missing)),
        };
        let value = match parts.next() {
            Some(value) => res!(HeaderFieldValue::new(&name, value)),
            None => return Err(err!(
                "A value must be present for header field name '{}' in the header line '{}'{}",
                name, line, line_msg;
            Invalid, Input, Missing)),
        };

        Ok(Self {
            name,
            value,
        })
    }

}

#[derive(Clone, Copy, Debug)]
#[repr(u16)]
pub enum HeaderFieldCategory {
    General     = 0,
    Request     = 1_000,
    Response    = 2_000,
    Entity      = 3_000,
    Other       = 5_000,
}

impl HeaderFieldCategory {
    pub fn order(&self) -> u16 {
        *self as u16
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OrderedHeaderName {
    ord: u16,
    name: HeaderName,
}

impl OrderedHeaderName {
    fn new(ord: u16, name: HeaderName) -> Self {
        Self {
            ord,
            name,
        }
    }
}

/// In HTTP, each header field is just a pair of strings (k, v), but multiple fields can exist with
/// the same key, while order is somewhat important in that we want the most general fields
/// appearing first.  Returns whether the header field name was present.
#[derive(Clone, Debug, Default)]
pub struct HeaderFields {
    fields: BTreeMap<HeaderName, (Vec<HeaderFieldValue>, u16)>,
    order: BTreeMap<OrderedHeaderName, HeaderName>,
}

impl HeaderFields {

    pub fn insert(
        &mut self,
        nam: HeaderName, 
        val: HeaderFieldValue,
        ord: Option<u16>,
    )
        -> bool
    {
        let ohn = OrderedHeaderName::new(
            match ord {
                Some(ord) => ord,
                None => nam.category().order(),
            },
            nam.clone(),
        );
        match self.fields.get_mut(&nam) {
            Some((prev_val_list, prev_ord)) => {
                prev_val_list.push(val);
                *prev_ord = ohn.ord;
                self.order.insert(ohn, nam);
                true
            },
            None => {
                self.fields.insert(nam.clone(), (vec![val], ohn.ord));
                self.order.insert(ohn, nam);
                false
            },
        }
    }

    pub fn get_all(
        &self,
        nam: &HeaderName, 
    ) 
        -> Option<&(Vec<HeaderFieldValue>, u16)>
    {
        self.fields.get(nam)
    }

    pub fn get_list(
        &self,
        nam: &HeaderName, 
    ) 
        -> Option<&Vec<HeaderFieldValue>>
    {
        match self.fields.get(nam) {
            Some((list, _)) => Some(list),
            None => None,
        }
    }

    pub fn get_one(
        &self,
        nam: &HeaderName, 
    ) 
        -> Option<&HeaderFieldValue>
    {
        match self.fields.get(nam) {
            Some((list, _)) => {
                if list.len() > 0 {
                    Some(&list[0])
                } else {
                    None
                }
            },
            _ => None,
        }
    }

    /// Get the value for the given name.  The value list must exist, and must have a length of one.
    pub fn get_the_one(
        &self,
        nam: &HeaderName, 
    ) 
        -> Outcome<&HeaderFieldValue>
    {
        match self.fields.get(nam) {
            Some((list, _)) => {
                if list.len() == 1 {
                    Ok(&list[0])
                } else {
                    Err(err!(
                        "Number of values for header field '{}', {} is not one.",
                        nam, list.len();
                    IO, Network, Invalid, Input))
                }
            },
            None => Err(err!(
                "Value list for header field '{}' does not exist.", nam;
            IO, Network, Invalid, Input)),
        }
    }

    pub fn get_session_id(&self) -> Option<String> {
        if let Some(HeaderFieldValue::Cookie(cookies)) = self.get_one(&HeaderName::Cookie) {
            for cookie in cookies {
                if cookie.key == SESSION_ID_KEY_LABEL {
                    return Some(cookie.val.clone());
                }
            }
        }
        None
    }

    /// Iterate over the header fields according to the `order` map.
    pub fn iter(&self) -> impl Iterator<Item = (HeaderName, Vec<HeaderFieldValue>)> + '_ {
        self.order.iter().filter_map(move |(_, key)| {
            self.fields.get(key).map(|(value, _)| (key.clone(), value.clone()))
        })
    }

}
