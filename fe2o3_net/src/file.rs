//! File request path handling and validation.
//! 
//! This module provides functionality for handling and validating web request paths,
//! including path normalisation, security checks, and content type detection.
use crate::{
    charset::Charset,
    http::fields::HeaderFieldValue,
    media::{
        Audio,
        ContentTypeValue,
        Font,
        Image,
        MediaType,
        MEDIA_PLAIN_TEXT,
        Text,
        Video,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
};

use std::{
    ffi::OsStr,
    fmt,
    path::{
        Component,
        Path,
        PathBuf,
    },
};


/// A validated request path for web server routes.
///
/// `RequestPath` wraps a string path and provides validation and normalisation
/// functionality to ensure paths are safe and well-formed for serving web content.
///
/// # Examples
/// ```
/// use oxedyne_fe2o3_net::file::RequestPath;
///
/// let path = RequestPath::new("/index.html");
/// assert_eq!(path.as_str(), "/index.html");
/// ```
#[derive(Clone, Debug, Default)]
pub struct RequestPath {
    path: String,
}

impl fmt::Display for RequestPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.path)
    }
}

impl RequestPath {

    pub fn new<S: Into<String>>(path: S) -> Self {
        Self {
            path: path.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        self.path.as_str()
    }

    pub fn as_string(&self) -> &String {
        &self.path
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.path)
    }

    pub fn validate(
        &self,
        public_root_path:    &String,
        default_root_file:   &String,
    )
        -> Outcome<PathBuf>
    {
        let mut path = self.path.clone();

        if path.starts_with('/') {
            path.remove(0); // Remove leading '/'
        }

        if path.len() > 0 && self.path.ends_with('/') {
            return Err(err!("Path must not end with  '/'"; IO, Network, Invalid, Input));
        }

        if path.len() == 0 {
            path = default_root_file.to_string();
        }
        let path = Path::new(&path);

        for component in path.components() {
            match component {
                Component::CurDir | Component::ParentDir => {
                    return Err(err!(
                        "Path must not contain relative components '.' or '..'";
                    IO, Network, Invalid, Input));
                }
                _ => ()
            }
        }
        let mut pathbuf = PathBuf::from(public_root_path);
        pathbuf.push(path);
        Ok(pathbuf)
    }

    pub fn content_type(path: &Path) -> HeaderFieldValue {
        HeaderFieldValue::ContentType(match path.extension().and_then(OsStr::to_str) {
            Some("css") => ContentTypeValue::MediaType((
                MediaType::Text(Text::Css),
                Some(Charset::Utf_8),
            )),
            Some("gif") => ContentTypeValue::MediaType((
                MediaType::Image(Image::Gif),
                None,
            )),
            Some("html") => ContentTypeValue::MediaType((
                MediaType::Text(Text::Html),
                Some(Charset::Utf_8),
            )),
            Some("jpg") | Some("jpeg") => ContentTypeValue::MediaType((
                MediaType::Image(Image::Jpeg),
                None,
            )),
            Some("js") => ContentTypeValue::MediaType((
                MediaType::Text(Text::Javascript),
                Some(Charset::Utf_8),
            )),
            Some("otf") => ContentTypeValue::MediaType((
                MediaType::Font(Font::Otf),
                None,
            )),
            Some("png") => ContentTypeValue::MediaType((
                MediaType::Image(Image::Png),
                None,
            )),
            Some("svg") => ContentTypeValue::MediaType((
                MediaType::Image(Image::SvgXml),
                None,
            )),
            Some("ttf") => ContentTypeValue::MediaType((
                MediaType::Font(Font::Ttf),
                None,
            )),
            Some("woff") => ContentTypeValue::MediaType((
                MediaType::Font(Font::Woff),
                None,
            )),
            Some("woff2") => ContentTypeValue::MediaType((
                MediaType::Font(Font::Woff2),
                None,
            )),
            // Recordings. A browser plays what the server says a thing is, not
            // what its name suggests, so a video served as text is a video that
            // downloads instead of playing -- and one that never gets a scrubber,
            // however well the server answers a `Range`.
            Some("mp4") | Some("m4v") => ContentTypeValue::MediaType((
                MediaType::Video(Video::Mp4),
                None,
            )),
            Some("webm") => ContentTypeValue::MediaType((
                MediaType::Video(Video::Webm),
                None,
            )),
            Some("ogv") => ContentTypeValue::MediaType((
                MediaType::Video(Video::Ogg),
                None,
            )),
            Some("mov") => ContentTypeValue::MediaType((
                MediaType::Video(Video::Quicktime),
                None,
            )),
            Some("mkv") => ContentTypeValue::MediaType((
                MediaType::Video(Video::XMatroska),
                None,
            )),
            Some("avi") => ContentTypeValue::MediaType((
                MediaType::Video(Video::XMsVideo),
                None,
            )),
            Some("mpeg") | Some("mpg") => ContentTypeValue::MediaType((
                MediaType::Video(Video::Mpeg),
                None,
            )),
            Some("mp3") => ContentTypeValue::MediaType((
                MediaType::Audio(Audio::Mpeg),
                None,
            )),
            Some("m4a") => ContentTypeValue::MediaType((
                MediaType::Audio(Audio::Mp4),
                None,
            )),
            Some("oga") | Some("ogg") => ContentTypeValue::MediaType((
                MediaType::Audio(Audio::Ogg),
                None,
            )),
            Some("wav") => ContentTypeValue::MediaType((
                MediaType::Audio(Audio::Wav),
                None,
            )),
            Some("flac") => ContentTypeValue::MediaType((
                MediaType::Audio(Audio::Flac),
                None,
            )),
            Some("aac") => ContentTypeValue::MediaType((
                MediaType::Audio(Audio::Aac),
                None,
            )),
            _ => MEDIA_PLAIN_TEXT,
        })
    }

}


#[cfg(test)]
mod tests {
    use super::*;

    /// A browser plays what the server says a thing is, not what its name
    /// suggests. A recording served as `text/plain` downloads rather than plays,
    /// and never gets a scrubber however well the server answers a `Range`.
    #[test]
    fn test_a_recording_is_served_as_a_recording() {
        for (name, expected) in [
            ("clip.mp4",    "video/mp4"),
            ("clip.m4v",    "video/mp4"),
            ("clip.webm",   "video/webm"),
            ("clip.ogv",    "video/ogg"),
            ("clip.mov",    "video/quicktime"),
            ("clip.mkv",    "video/x-matroska"),
            ("clip.avi",    "video/x-msvideo"),
            ("clip.mpg",    "video/mpeg"),
            ("track.mp3",   "audio/mpeg"),
            ("track.m4a",   "audio/mp4"),
            ("track.ogg",   "audio/ogg"),
            ("track.wav",   "audio/wav"),
            ("track.flac",  "audio/flac"),
            ("track.aac",   "audio/aac"),
        ] {
            let got = fmt!("{}", RequestPath::content_type(Path::new(name)));
            assert_eq!(got, expected, "{} was served as {}", name, got);
        }
    }

    /// What the map does not know is still plain text, as it always was.
    #[test]
    fn test_an_unknown_extension_is_still_plain_text() {
        assert_eq!(
            fmt!("{}", RequestPath::content_type(Path::new("thing.xyzzy"))),
            "text/plain; charset=utf-8",
        );
    }
}
