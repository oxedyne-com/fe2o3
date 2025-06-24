use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt,
    str::FromStr,
};

use strum::FromRepr;

/// Ref: [IANA](https://www.iana.org/assignments/http-status-codes/http-status-codes.xhtml)
#[derive(Clone, Copy, Debug, Eq, FromRepr, PartialEq)]
#[repr(u16)]
pub enum HttpStatus {
    Continue                        = 100,
    SwitchingProtocols              = 101,
    Processing                      = 102,
    EarlyHints                      = 103,
    OK                              = 200,
    Created                         = 201,
    Accepted                        = 202,
    NonAuthoritativeInformation     = 203,
    NoContent                       = 204,
    ResetContent                    = 205,
    PartialContent                  = 206,
    MultiStatus                     = 207,
    AlreadyReported                 = 208,
    IMUsed                          = 226,
    MultipleChoices                 = 300,
    MovedPermanently                = 301,
    Found                           = 302,
    SeeOther                        = 303,
    NotModified                     = 304,
    UseProxy                        = 305,
    TemporaryRedirect               = 307,
    PermanentRedirect               = 308,
    BadRequest                      = 400,
    Unauthorized                    = 401,
    PaymentRequired                 = 402,
    Forbidden                       = 403,
    NotFound                        = 404,
    MethodNotAllowed                = 405,
    NotAcceptable                   = 406,
    ProxyAuthenticationRequired     = 407,
    RequestTimeout                  = 408,
    Conflict                        = 409,
    Gone                            = 410,
    LengthRequired                  = 411,
    PreconditionFailed              = 412,
    ContentTooLarge                 = 413,
    URITooLong                      = 414,
    UnsupportedMediaType            = 415,
    RangeNotSatisfiable             = 416,
    ExpectationFailed               = 417,
    MisdirectedRequest              = 421,
    UnprocessableContent            = 422,
    Locked                          = 423,
    FailedDependency                = 424,
    TooEarly                        = 425,
    UpgradeRequired                 = 426,
    PreconditionRequired            = 428,
    TooManyRequests                 = 429,
    RequestHeaderFieldsTooLarge     = 431,
    UnavailableForLegalReasons      = 451,
    InternalServerError             = 500,
    NotImplemented                  = 501,
    BadGateway                      = 502,
    ServiceUnavailable              = 503,
    GatewayTimeout                  = 504,
    HTTPVersionNotSupported         = 505,
    VariantAlsoNegotiates           = 506,
    InsufficientStorage             = 507,
    LoopDetected                    = 508,
    NotExtended                     = 510,
    NetworkAuthenticationRequired   = 511,
}

impl fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", (*self as u16).to_string())
    }
}

impl FromStr for HttpStatus {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.parse::<u16>() {
            Ok(n) => match Self::from_repr(n) {
                Some(status) => Ok(status),
                None => Err(err!(
                    "Unrecognised HTTP status code '{}'.", s;
                IO, Network, Unknown, Input)),
            },
            Err(e) => Err(err!(e, "Code '{}' is not a u16.", s; IO, Network, Mismatch, Input))
        }
    }
}

impl HttpStatus {

    pub fn desc(&self) -> &'static str {
        match self {
            Self::Continue                        => "Continue",
            Self::SwitchingProtocols              => "Switching Protocols",			
            Self::Processing                      => "Processing",			
            Self::EarlyHints                      => "Early Hints",			
            Self::OK                              => "OK",			
            Self::Created                         => "Created",			
            Self::Accepted                        => "Accepted",			
            Self::NonAuthoritativeInformation     => "Non Authoritative Information",			
            Self::NoContent                       => "No Content",			
            Self::ResetContent                    => "Reset Content",			
            Self::PartialContent                  => "Partial Content",			
            Self::MultiStatus                     => "MultiStatus",			
            Self::AlreadyReported                 => "Already Reported",			
            Self::IMUsed                          => "IM Used",			
            Self::MultipleChoices                 => "MultipleChoices",			
            Self::MovedPermanently                => "MovedPermanently",			
            Self::Found                           => "Found",			
            Self::SeeOther                        => "SeeOther",			
            Self::NotModified                     => "NotModified",			
            Self::UseProxy                        => "UseProxy",			
            Self::TemporaryRedirect               => "Temporary Redirect",			
            Self::PermanentRedirect               => "Permanent Redirect",			
            Self::BadRequest                      => "Bad Request",			
            Self::Unauthorized                    => "Unauthorized",			
            Self::PaymentRequired                 => "Payment Required",			
            Self::Forbidden                       => "Forbidden",			
            Self::NotFound                        => "Not Found",			
            Self::MethodNotAllowed                => "Method Not Allowed",			
            Self::NotAcceptable                   => "Not Acceptable",			
            Self::ProxyAuthenticationRequired     => "Proxy Authentication Required",			
            Self::RequestTimeout                  => "Request Timeout",			
            Self::Conflict                        => "Conflict",			
            Self::Gone                            => "Gone",			
            Self::LengthRequired                  => "Length Required",			
            Self::PreconditionFailed              => "Precondition Failed",			
            Self::ContentTooLarge                 => "Content Too Large",			
            Self::URITooLong                      => "URI Too Long",			
            Self::UnsupportedMediaType            => "Unsupported Media Type",			
            Self::RangeNotSatisfiable             => "Range Not Satisfiable",			
            Self::ExpectationFailed               => "Expectation Failed",			
            Self::MisdirectedRequest              => "Misdirected Request",			
            Self::UnprocessableContent            => "Unprocessable Content",			
            Self::Locked                          => "Locked",			
            Self::FailedDependency                => "Failed Dependency",			
            Self::TooEarly                        => "Too Early",			
            Self::UpgradeRequired                 => "Upgrade Required",			
            Self::PreconditionRequired            => "Precondition Required",			
            Self::TooManyRequests                 => "Too Many Requests",			
            Self::RequestHeaderFieldsTooLarge     => "Request Header Fields Too Large",			
            Self::UnavailableForLegalReasons      => "Unavailable For Legal Reasons",			
            Self::InternalServerError             => "Internal Server Error",			
            Self::NotImplemented                  => "Not Implemented",			
            Self::BadGateway                      => "Bad Gateway",			
            Self::ServiceUnavailable              => "Service Unavailable",			
            Self::GatewayTimeout                  => "Gateway Timeout",			
            Self::HTTPVersionNotSupported         => "HTTP Version Not Supported",			
            Self::VariantAlsoNegotiates           => "Variant Also Negotiates",			
            Self::InsufficientStorage             => "Insufficient Storage",			
            Self::LoopDetected                    => "Loop Detected",			
            Self::NotExtended                     => "Not Extended",			
            Self::NetworkAuthenticationRequired   => "Network Authentication Required",			
        }
    }
}
