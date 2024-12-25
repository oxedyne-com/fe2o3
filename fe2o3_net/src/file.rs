//! File request path handling and validation.
//! 
//! This module provides functionality for handling and validating web request paths,
//! including path normalisation, security checks, and content type detection.
use crate::{
    charset::Charset,
    http::fields::HeaderFieldValue,
    media::{
        ContentTypeValue,
        Font,
        Image,
        MediaType,
        MEDIA_PLAIN_TEXT,
        Text,
    },
};

use oxedize_fe2o3_core::{
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
/// use crate::RequestPath;
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
            return Err(err!(errmsg!("Path must not end with  '/'"), IO, Network, Invalid, Input));
        }

        if path.len() == 0 {
            path = default_root_file.to_string();
        }
        let path = Path::new(&path);

        for component in path.components() {
            match component {
                Component::CurDir | Component::ParentDir => {
                    return Err(err!(errmsg!(
                        "Path must not contain relative components '.' or '..'",
                    ), IO, Network, Invalid, Input));
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
            _ => MEDIA_PLAIN_TEXT,
        })
    }

}
