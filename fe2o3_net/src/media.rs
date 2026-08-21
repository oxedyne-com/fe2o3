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

use oxedyne_fe2o3_core::prelude::*;

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
            // `mt` here is the SUBTYPE, whose Display is "form-data", so the top
            // level has to be written. Without it a parsed multipart header was
            // re-emitted as `form-data; boundary=...`, which the next hop cannot
            // parse -- see the note on `Multipart`'s own Display.
            Self::Multipart((mt, b)) => write!(f, "multipart/{}; boundary={}", mt, b),
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
    Video(Video),
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
            Self::Video(inner)          => fmt!("video/{}", inner),
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
                "video"         => Self::Video(res!(Video::from_str(right))),
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
            Self::Application(Application::JsonLd)          |
            Self::Application(Application::ManifestJson)    |
            Self::Application(Application::FormUrlEncoded)  |
            Self::Application(Application::Xml)             => true,
            // Structured syntax suffixes per RFC 6838 §4.2.8: anything
            // that looks like `foo+json` or `foo+xml` is text-shaped, so
            // body dumps like `application/problem+json` (RFC 7807) and
            // `application/jose+json` (RFC 7515) can be logged as text
            // rather than binary.
            Self::Application(Application::Other(s)) => {
                s.ends_with("+json") || s.ends_with("+xml")
            },
            // TODO complete list
            _ => false,
        }
    }

    /// Is a body of this type worth compressing on the way out?
    ///
    /// Text of every shape is, and so is WebAssembly, whose binary format is
    /// mostly indices and opcodes and halves under DEFLATE. Everything already
    /// compressed is not: a second pass over a PNG, a JPEG, a WebP, an AVIF, a
    /// WOFF2, an Opus stream or a Zip spends the processor and *adds* bytes,
    /// since a compressor that finds no redundancy still pays for its own
    /// framing. RFC 9110 §8.4.1 names content coding as an aid to transfer, not
    /// a second encoding of an already-encoded representation.
    ///
    /// The default is `false`, so a type this crate does not model is sent as it
    /// is. That is the safe way round: a missed saving costs bandwidth, whereas
    /// compressing something already compressed costs the processor and gains
    /// nothing.
    pub fn is_compressible(&self) -> bool {
        match self {
            // Every text subtype, plus the text-shaped application subtypes and
            // SVG, which `is_text` already recognises.
            _ if self.is_text() => true,
            // Binary, but highly redundant: a module is mostly LEB128 indices.
            Self::Application(Application::Wasm)            => true,
            // Structured documents that travel uncompressed.
            Self::Application(Application::Sql)             |
            Self::Application(Application::OpenDocument)    => true,
            // The uncompressed font formats. WOFF and WOFF2 carry their own
            // compression and are deliberately absent.
            Self::Font(Font::Collection)                    |
            Self::Font(Font::Otf)                           |
            Self::Font(Font::Sfnt)                          |
            Self::Font(Font::Ttf)                           => true,
            // An uncompressed raster format, unlike every other image type.
            Self::Image(Image::Tiff)                        => true,
            Self::Model(Model::Obj)                         => true,
            _ => false,
        }
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Application     │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
///
/// The `Other(String)` variant accepts any subtype we do not have a named
/// variant for -- e.g. `application/problem+json` (RFC 7807, used by ACME
/// CAs for error responses), `application/jose+json` (RFC 7515), or any
/// future IANA subtype. This matters for clients like the ACME driver
/// in [`crate::acme::client`] which must be able to receive and parse
/// responses with arbitrary Content-Type values; a strict enum would
/// refuse the response at parse time and the caller would never see the
/// body. Keeping known subtypes as dedicated variants preserves the
/// ergonomics of pattern matching for code that cares.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Application {
    Json,
    JsonLd,
    /// `application/manifest+json`, registered with IANA by the W3C Web
    /// Application Manifest specification.
    ManifestJson,
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
    /// `application/wasm`, registered with IANA by the WebAssembly Core
    /// specification. A browser compiles a module as it arrives only when the
    /// server says this; under any other type `compileStreaming` refuses the
    /// response and the whole module must be buffered first.
    Wasm,
    Xml,
    Zip,
    Zstd,
    /// Subtype not explicitly modelled by this crate; the contained
    /// string is the raw subtype text after the `application/` prefix.
    Other(String),
}

impl Display for Application {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json                      => write!(f, "json"),
            Self::JsonLd                    => write!(f, "ld+json"),
            Self::ManifestJson              => write!(f, "manifest+json"),
            Self::Pdf                       => write!(f, "pdf"),
            Self::Sql                       => write!(f, "sql"),
            Self::MicrosoftDocument         => write!(f, "msword"),
            Self::MicrosoftPresentation     => write!(f, "vnd.ms-powerpoint"),
            Self::MicrosoftSpreadsheet      => write!(f, "vnd.ms-excel"),
            Self::OpenDocument              => write!(f, "vnd.oasis.opendocument.text"),
            Self::OpenXmlDocument           => write!(f, "vnd.openxmlformats-officedocument.wordprocessingml.document"),
            Self::OpenXmlPresentation       => write!(f, "vnd.openxmlformats-officedocument.presentationml.presentation"),
            Self::OpenXmlSpreadsheet        => write!(f, "vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            Self::FormUrlEncoded            => write!(f, "x-www-form-urlencoded"),
            Self::Wasm                      => write!(f, "wasm"),
            Self::Xml                       => write!(f, "xml"),
            Self::Zip                       => write!(f, "zip"),
            Self::Zstd                      => write!(f, "zstd"),
            Self::Other(s)                  => write!(f, "{}", s),
        }
    }
}

impl FromStr for Application {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "json"                                                          => Self::Json,
            "ld+json"                                                       => Self::JsonLd,
            "manifest+json"                                                 => Self::ManifestJson,
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
            "wasm"                                                          => Self::Wasm,
            "xml"                                                           => Self::Xml,
            "zip"                                                           => Self::Zip,
            "zstd"                                                          => Self::Zstd,
            // Any other IANA subtype: stored verbatim so callers that do
            // care about it can still read the raw string, and the HTTP
            // message parser can construct a complete HttpMessage instead
            // of failing the whole response. Structured JSON-ish subtypes
            // like `problem+json` and `jose+json` arrive here.
            other                                                           => Self::Other(other.to_string()),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Audio           │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Audio {
    Aac,
    Flac,
    Mp4,
    Mpeg,
    Ogg,
    Wav,
    Webm,
}

/// The subtype alone, because [`MediaType`] writes the `audio/` before it.
/// Writing it here as well once put `audio/audio/mpeg` on the wire, which no
/// player recognises as anything.
impl Display for Audio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Aac       => "aac",
            Self::Flac      => "flac",
            Self::Mp4       => "mp4",
            Self::Mpeg      => "mpeg",
            Self::Ogg       => "ogg",
            Self::Wav       => "wav",
            Self::Webm      => "webm",
        })
    }
}

impl FromStr for Audio {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "aac"           => Self::Aac,
            "flac"          => Self::Flac,
            "mp4"           => Self::Mp4,
            "mpeg"          => Self::Mpeg,
            "ogg"           => Self::Ogg,
            "wav"           => Self::Wav,
            // WAVE has never had one name. IANA registers `audio/vnd.wave`
            // (RFC 2361) and lists `audio/wav`, `audio/wave` and `audio/x-wav`
            // as the names in use; the WHATWG mime sniffing standard emits
            // `audio/wave`. All three are read, and `audio/wav` is written.
            "wave" | "x-wav" | "vnd.wave"
                            => Self::Wav,
            "webm"          => Self::Webm,
            _ => return Err(err!(
                "Unrecognised Audio Media subtype '{}'.", s;
            IO, Network, Unknown, Input)),
        })
    }
}

/// ╭────────────────────────────────────────────╮
/// │ IANA Top Level Media Type: Video           │
/// │ Subtypes                                   │
/// ╰────────────────────────────────────────────╯
///
/// A recording served under the wrong type is a recording no browser will play,
/// however well the bytes are delivered, so the seekable media this crate exists
/// to serve needs its types named.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Video {
    Mp4,
    Mpeg,
    Ogg,
    Quicktime,
    Webm,
    /// Matroska, which IANA has not registered but every player expects.
    XMatroska,
    /// AVI, likewise unregistered and likewise expected.
    XMsVideo,
}

impl Display for Video {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Mp4           => "mp4",
            Self::Mpeg          => "mpeg",
            Self::Ogg           => "ogg",
            Self::Quicktime     => "quicktime",
            Self::Webm          => "webm",
            Self::XMatroska     => "x-matroska",
            Self::XMsVideo      => "x-msvideo",
        })
    }
}

impl FromStr for Video {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "mp4"           => Self::Mp4,
            "mpeg"          => Self::Mpeg,
            "ogg"           => Self::Ogg,
            "quicktime"     => Self::Quicktime,
            "webm"          => Self::Webm,
            "x-matroska"    => Self::XMatroska,
            "x-msvideo"     => Self::XMsVideo,
            _ => return Err(err!(
                "Unrecognised Video Media subtype '{}'.", s;
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
    /// `image/vnd.microsoft.icon`, the IANA registration for the favicon
    /// format; `image/x-icon` is the unregistered name browsers also accept.
    Icon,
    Jpeg,
    Png,
    SvgXml,
    Tiff,
    /// `image/webp`, registered with IANA by RFC 9649.
    Webp,
}

impl Display for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::Avif      => "avif",
            Self::Gif       => "gif",
            Self::Icon      => "vnd.microsoft.icon",
            Self::Jpeg      => "jpeg",
            Self::Png       => "png",
            Self::SvgXml    => "svg+xml",
            Self::Tiff      => "tiff",
            Self::Webp      => "webp",
        })
    }
}

impl FromStr for Image {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "avif"                  => Self::Avif,
            "gif"                   => Self::Gif,
            "vnd.microsoft.icon"    => Self::Icon,
            // The unregistered name browsers have always sent and accepted, so
            // a proxy forwarding one upstream's favicon type does not fail.
            "x-icon"                => Self::Icon,
            "jpeg"                  => Self::Jpeg,
            "png"                   => Self::Png,
            "svg+xml"               => Self::SvgXml,
            "tiff"                  => Self::Tiff,
            "webp"                  => Self::Webp,
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


#[cfg(test)]
mod tests {
    use super::*;

    /// `MediaType` writes the top-level type, so a subtype that writes it again
    /// puts `audio/audio/mpeg` on the wire, which no player recognises.
    #[test]
    fn test_a_media_type_names_its_top_level_once() {
        assert_eq!(fmt!("{}", MediaType::Audio(Audio::Mpeg)), "audio/mpeg");
        assert_eq!(fmt!("{}", MediaType::Video(Video::Mp4)), "video/mp4");
        assert_eq!(fmt!("{}", MediaType::Font(Font::Woff2)), "font/woff2");
        assert_eq!(fmt!("{}", MediaType::Text(Text::Html)), "text/html");
    }

    /// Every type this crate writes it can also read back, which is what a proxy
    /// forwarding a `Content-Type` needs of it.
    #[test]
    fn test_every_recording_type_survives_a_round_trip() -> Outcome<()> {
        let types = [
            MediaType::Video(Video::Mp4),
            MediaType::Video(Video::Mpeg),
            MediaType::Video(Video::Ogg),
            MediaType::Video(Video::Quicktime),
            MediaType::Video(Video::Webm),
            MediaType::Video(Video::XMatroska),
            MediaType::Video(Video::XMsVideo),
            MediaType::Audio(Audio::Aac),
            MediaType::Audio(Audio::Flac),
            MediaType::Audio(Audio::Mp4),
            MediaType::Audio(Audio::Mpeg),
            MediaType::Audio(Audio::Ogg),
            MediaType::Audio(Audio::Wav),
            MediaType::Audio(Audio::Webm),
        ];
        for mt in types {
            let written = fmt!("{}", mt);
            let read = res!(MediaType::from_str(&written));
            assert_eq!(read, mt, "{:?} did not survive being written as {:?}", mt, written);
        }
        Ok(())
    }

    /// The exact strings in the IANA media types registry. A type spelled a
    /// little differently is a type the receiver does not recognise, and the
    /// registry is the only authority on the spelling.
    #[test]
    fn test_a_registered_type_is_spelled_as_the_registry_spells_it() {
        for (mt, registered) in [
            // WebAssembly Core, IANA registered.
            (MediaType::Application(Application::Wasm),          "application/wasm"),
            // W3C Web Application Manifest, IANA registered.
            (MediaType::Application(Application::ManifestJson),  "application/manifest+json"),
            // RFC 9649.
            (MediaType::Image(Image::Webp),                      "image/webp"),
            // IANA registered; `image/x-icon` is the unregistered name.
            (MediaType::Image(Image::Icon),                      "image/vnd.microsoft.icon"),
            // AVIF: IANA registered by the AV1 Image File Format specification.
            (MediaType::Image(Image::Avif),                      "image/avif"),
            // TIFF: RFC 3302.
            (MediaType::Image(Image::Tiff),                      "image/tiff"),
        ] {
            assert_eq!(fmt!("{}", mt), registered);
            match MediaType::from_str(registered) {
                Ok(read) => assert_eq!(read, mt, "{} was read back as {:?}", registered, read),
                Err(e) => panic!("{} was refused: {}", registered, e),
            }
        }
    }

    /// A format that carries its own compression gains nothing from a second
    /// pass and pays for the framing, so the default is to leave a type alone.
    #[test]
    fn test_only_a_type_that_gains_by_it_is_compressible() {
        for mt in [
            MediaType::Text(Text::Html),
            MediaType::Text(Text::Css),
            MediaType::Text(Text::Javascript),
            MediaType::Application(Application::Json),
            MediaType::Application(Application::Wasm),
            MediaType::Image(Image::SvgXml),
            MediaType::Font(Font::Ttf),
            MediaType::Image(Image::Tiff),
        ] {
            assert!(mt.is_compressible(), "{} gains by being compressed", mt);
        }
        for mt in [
            MediaType::Image(Image::Png),
            MediaType::Image(Image::Jpeg),
            MediaType::Image(Image::Webp),
            MediaType::Image(Image::Avif),
            MediaType::Font(Font::Woff2),
            MediaType::Audio(Audio::Ogg),
            MediaType::Video(Video::Mp4),
            MediaType::Application(Application::Zip),
            MediaType::Application(Application::Zstd),
            MediaType::Application(Application::Pdf),
        ] {
            assert!(!mt.is_compressible(), "{} is already compressed", mt);
        }
    }

    /// One format, several registered names. A proxy that refuses the name an
    /// upstream chose forwards nothing, so all of them are read even though only
    /// one is written.
    #[test]
    fn test_the_alternative_names_for_one_format_are_all_read() -> Outcome<()> {
        for name in ["audio/wav", "audio/wave", "audio/x-wav", "audio/vnd.wave"] {
            let read = res!(MediaType::from_str(name));
            assert_eq!(read, MediaType::Audio(Audio::Wav), "reading {}", name);
        }
        assert_eq!(res!(MediaType::from_str("image/x-icon")), MediaType::Image(Image::Icon));
        Ok(())
    }
}
