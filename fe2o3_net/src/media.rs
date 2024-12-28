//! The specification for Media Types is:
//!     https://www.iana.org/assignments/media-types/media-types.xhtml
//! RFC 2046 Section 1:
//! "In general, the top-level media type is used to declare the general
//! type of data, while the subtype specifies a specific format for that
//! type of data.  Thus, a media type of "image/xyz" is enough to tell a
//! user agent that the data is an image, even if the user agent has no
//! knowledge of the specific image format "xyz".  Such information can
//! be used, for example, to decide whether or not to show a user the raw
//! data from an unrecognized subtype -- such an action might be
//! reasonable for unrecognized subtypes of "text", but not for
//! unrecognized subtypes of "image" or "audio".  For this reason,
//! registered subtypes of "text", "image", "audio", and "video" should
//! not contain embedded information that is really of a different type.
//! Such compound formats should be represented using the "multipart" or
//! "application" types."
//!
//! TODO Complete types.
use crate::charset::Charset;

use oxedize_fe2o3_core::prelude::*;

use std::{
    fmt::{
        self,
        Display,
    },
    str::FromStr,
};


pub const MEDIA_PLAIN_TEXT: ContentTypeValue =
    ContentTypeValue::MediaType((
        MediaType::Text(Text::Plain),
        Some(Charset::Utf_8),
    ));

/// Encapsulator for "Content-Type" header.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ContentTypeValue {
    MediaType((MediaType, Option<Charset>)),
    Multipart((Multipart, String)),
}

impl Display for ContentTypeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MediaType((mt, cs_opt)) => match cs_opt {
                Some(cs) => write!(f, "{}; charset={}", mt, cs),
                None => write!(f, "{}", mt),
            },
            Self::Multipart((mt, b)) => write!(f, "{}; boundary={}", mt, b),
        }
    }
}

/// ╭────────────────────────────╮
/// │ IANA Top Level Media Types │
/// ╰────────────────────────────╯
/// 
///   RFC 2046 Section 2
///   https://www.rfc-editor.org/rfc/rfc2046.html#section-2
/// 
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MediaType {
    // Discrete
    Application(Application),
    Audio(Audio),
    Font(Font),
    Image(Image),
    Model(Model),
    Text(Text),
    //Video(Video),
    // Composite
    //Message(Message),
    Multipart(Multipart),
}

impl Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Application(inner)    => fmt!("application/{}", inner),
            Self::Audio(inner)          => fmt!("audio/{}", inner),
            Self::Font(inner)           => fmt!("font/{}", inner),
            Self::Image(inner)          => fmt!("image/{}", inner),
            Self::Model(inner)          => fmt!("model/{}", inner),
            Self::Text(inner)           => fmt!("text/{}", inner),
            Self::Multipart(inner)      => fmt!("multipart/{}", inner),
        })
    }
}

impl FromStr for MediaType {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.split_once('/') {
            Some((left, right)) => match left {
                "application"   => Self::Application(res!(Application::from_str(right))),
                "audio"         => Self::Audio(res!(Audio::from_str(right))),
                "font"          => Self::Font(res!(Font::from_str(right))),
                "image"         => Self::Image(res!(Image::from_str(right))),
                "model"         => Self::Model(res!(Model::from_str(right))),
                "text"          => Self::Text(res!(Text::from_str(right))),
                "multipart"     => Self::Multipart(res!(Multipart::from_str(right))),
                _ => return Err(err!(
                    "Unrecognised Media type '{}' in '{}'.", left, s;
                IO, Network, Unknown, Input)),
            },
            _ => return Err(err!(
                "Invalid Media type '{}', '/' character not found.", s;
            IO, Network, Invalid, Input)),
        })
    }
}

impl MediaType {
    pub fn is_text(&self) -> bool {
        match self {
            Self::Text(_)                                   |
            Self::Image(Image::SvgXml)                      |
            Self::Application(Application::Json)            |
            Self::Application(Application::FormUrlEncoded)  |
            Self::Application(Application::Xml)             => true,
            // TODO complete list
            _ => false,
        }
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Application     │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Application {
    Json,
    JsonLd,
    Pdf,
    Sql,
    MicrosoftDocument,
    MicrosoftPresentation,
    MicrosoftSpreadsheet,
    OpenDocument,
    OpenXmlDocument,
    OpenXmlPresentation,
    OpenXmlSpreadsheet,
    FormUrlEncoded,
    Xml,
    Zip,
    Zstd,
}

impl Display for Application {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Json                      => "json",
            Self::JsonLd                    => "ld+json",
            Self::Pdf                       => "pdf",
            Self::Sql                       => "sql",
            Self::MicrosoftDocument         => "msword",
            Self::MicrosoftPresentation     => "vnd.ms-powerpoint",
            Self::MicrosoftSpreadsheet      => "vnd.ms-excel",
            Self::OpenDocument              => "vnd.oasis.opendocument.text",
            Self::OpenXmlDocument           => "vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::OpenXmlPresentation       => "vnd.openxmlformats-officedocument.presentationml.presentation",
            Self::OpenXmlSpreadsheet        => "vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::FormUrlEncoded            => "x-www-form-urlencoded",
            Self::Xml                       => "xml",
            Self::Zip                       => "zip",
            Self::Zstd                      => "zstd",
        })
    }
}

impl FromStr for Application {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "json"                                                          => Self::Json,                  
            "ld+json"                                                       => Self::JsonLd,               
            "pdf"                                                           => Self::Pdf,                   
            "sql"                                                           => Self::Sql,                   
            "msword"                                                        => Self::MicrosoftDocument,     
            "vnd.ms-powerpoint"                                             => Self::MicrosoftPresentation, 
            "vnd.ms-excel"                                                  => Self::MicrosoftSpreadsheet,  
            "vnd.oasis.opendocument.text"                                   => Self::OpenDocument,          
            "vnd.openxmlformats-officedocument.wordprocessingml.document"   => Self::OpenXmlDocument,       
            "vnd.openxmlformats-officedocument.presentationml.presentation" => Self::OpenXmlPresentation,   
            "vnd.openxmlformats-officedocument.spreadsheetml.sheet"         => Self::OpenXmlSpreadsheet,    
            "x-www-form-urlencoded"                                         => Self::FormUrlEncoded,        
            "xml"                                                           => Self::Xml,                   
            "zip"                                                           => Self::Zip,                   
            "zstd"                                                          => Self::Zstd,                  
            _ => return Err(err!(
                "Unrecognised Application Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Audio           │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Audio {
    Mpeg,
    Ogg,
}

impl Display for Audio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "audio/{}", match self {
            Self::Mpeg      => "mpeg",
            Self::Ogg       => "ogg",
        })
    }
}

impl FromStr for Audio {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "mpeg"          => Self::Mpeg,                  
            "ogg"           => Self::Ogg,               
            _ => return Err(err!(
                "Unrecognised Audio Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Font            │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Font {
    Collection,
    Otf,
    Sfnt,
    Ttf,
    Woff,
    Woff2,
}

impl Display for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Collection    => "collection",
            Self::Otf           => "otf",
            Self::Sfnt          => "sfnt",
            Self::Ttf           => "ttf",
            Self::Woff          => "woff",
            Self::Woff2         => "woff2",
        })
    }
}

impl FromStr for Font {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "collection"        => Self::Collection,
            "otf"               => Self::Otf,
            "sfnt"              => Self::Sfnt,
            "ttf"               => Self::Ttf,
            "woff"              => Self::Woff,
            "woff2"             => Self::Woff2,
            _ => return Err(err!(
                "Unrecognised Font Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Image           │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Image {
    Avif,
    Gif,
    Jpeg,
    Png,
    SvgXml,
    Tiff,
}

impl Display for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Avif      => "avif",
            Self::Gif       => "gif",
            Self::Jpeg      => "jpeg",
            Self::Png       => "png",
            Self::SvgXml    => "svg+xml",   
            Self::Tiff      => "tiff",
        })
    }
}

impl FromStr for Image {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "avif"          => Self::Avif,   
            "gif"           => Self::Gif,   
            "jpeg"          => Self::Jpeg,   
            "png"           => Self::Png,    
            "svg+xml"       => Self::SvgXml,
            "tiff"          => Self::Tiff,   
            _ => return Err(err!(
                "Unrecognised Image Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Model           │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Model {
    Obj,
}

impl Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Obj       => "obj",
        })
    }
}

impl FromStr for Model {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "obj"           => Self::Obj,               
            _ => return Err(err!(
                "Unrecognised Model Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Multipart       │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Multipart {
    FormData,
}

impl Display for Multipart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::FormData      => "form-data",
        })
    }
}

impl FromStr for Multipart {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "form-data"         => Self::FormData,               
            _ => return Err(err!(
                "Unrecognised Multipart Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Text            │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Text {
    Plain,
    Css,
    Csv,
    Html,
    Javascript,
    Xml,
}

impl Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Plain         => "plain",
            Self::Css           => "css",
            Self::Csv           => "csv",
            Self::Html          => "html",
            Self::Javascript    => "javascript",
            Self::Xml           => "xml",
        })
    }
}

impl FromStr for Text {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "plain"             => Self::Plain,
            "css"               => Self::Css,
            "csv"               => Self::Csv,
            "html"              => Self::Html,
            "javascript"        => Self::Javascript,
            "xml"               => Self::Xml,
            _ => return Err(err!(
                "Unrecognised Text Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}
