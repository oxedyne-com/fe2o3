use crate::{
    constant,
    http::{
        fields::{
            HeaderField,
            HeaderFields,
            HeaderFieldValue,
            HeaderName,
        },
        loc::HttpLocator,
        status::HttpStatus,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt,
    pin::Pin,
    str::FromStr,
};

use strum::{
    Display,
    EnumString,
};
use tokio::{
    io::{
        AsyncRead,
        AsyncReadExt,
        //AsyncWriteExt,
    },
};

#[derive(Debug)]
pub enum HttpVersion {
    Http1_1,
    Http2_0,
    Http3_0,
}

impl fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http1_1 => write!(f, "HTTP/1.1"),
            Self::Http2_0 => write!(f, "HTTP/2"),
            Self::Http3_0 => write!(f, "HTTP/3"),
        }
    }
}

impl FromStr for HttpVersion {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "HTTP/1.1" => Self::Http1_1,
            "HTTP/2" => Self::Http2_0,
            "HTTP/3" => Self::Http3_0,
            _ => return Err(err!(
                "Unrecognised HTTP version {}.", s;
            IO, Network, Unknown, Input))
        })
    }
}

#[derive(Clone, Copy, Debug, Display, EnumString, PartialEq)]
pub enum HttpMethod {
    CONNECT,
    DELETE,
    GET,
    HEAD,
    OPTIONS,
    PATCH,
    POST,
    PUT,
    TRACE,
}

impl HttpMethod {
    pub fn body_required(&self) -> bool {
        match self {
            Self::POST  |
            Self::PUT   |
            Self::PATCH => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum HttpHeadline {
    Request {
        method: HttpMethod,
        loc: HttpLocator,
    },
    Response {
        status: HttpStatus,
    }
}

impl HttpHeadline {

    /// Used when the caller may not know if the message is a request or a response.
    pub fn parse(
        line:       &str,
        is_request: Option<bool>,
    )
        -> Outcome<(Self, HttpVersion)>
    {
        match is_request {
            Some(true) => Self::parse_request(line),
            Some(false) => Self::parse_response(line),
            None => {
                let mut parts = line.split_whitespace();
                if let Some(first_str) = parts.next() {
                    match HttpMethod::from_str(first_str) {
                        Ok(method) => if let Some(loc_str) = parts.next() {
                            if let Some(version_str) = parts.next() {
                                return Ok((
                                    HttpHeadline::Request {
                                        method,
                                        loc: res!(HttpLocator::new(loc_str)),
                                    },
                                    res!(HttpVersion::from_str(version_str)),
                                ));
                            }
                        },
                        Err(_) => match HttpVersion::from_str(first_str) {
                            Ok(version) => if let Some(code_str) = parts.next() {
                                return Ok((
                                    HttpHeadline::Response {
                                        status: res!(HttpStatus::from_str(code_str)),
                                    },
                                    version,
                                ));
                            },
                            Err(_) => return Err(err!(
                                "HTTP message headline '{}' begins with an unrecognised word.", line;
                            IO, Network, Invalid, Input)),
                        },
                    }
                }
                Err(err!(
                    "HTTP request headline '{}' invalid, expected at least 3 components.", line;
                IO, Network, Missing, Input))
            },
        }
    }

    /// Used when the caller knows the message is a request.
    pub fn parse_request(line: &str) -> Outcome<(Self, HttpVersion)> {
        let mut parts = line.split_whitespace();
        if let Some(method_str) = parts.next() {
            if let Some(loc_str) = parts.next() {
                if let Some(version_str) = parts.next() {
                    return Ok((
                        HttpHeadline::Request {
                            method: res!(HttpMethod::from_str(method_str)),
                            loc: res!(HttpLocator::new(loc_str)),
                        },
                        res!(HttpVersion::from_str(version_str)),
                    ));
                }
            }
        }
        Err(err!(
            "HTTP request headline '{}' invalid, expected at least 3 components.", line;
        IO, Network, Missing, Input))
    }

    /// Used when the caller knows the message is a response.
    pub fn parse_response(line: &str) -> Outcome<(Self, HttpVersion)> {
        let mut parts = line.split_whitespace();
        if let Some(version_str) = parts.next() {
            if let Some(code_str) = parts.next() {
                return Ok((
                    HttpHeadline::Response {
                        status: res!(HttpStatus::from_str(code_str)),
                    },
                    res!(HttpVersion::from_str(version_str)),
                ));
            }
        }
        Err(err!(
            "HTTP response headline '{}' invalid, expected at least 3 components.", line;
        IO, Network, Missing, Input))
    }

}

#[derive(Debug)]
pub struct HttpHeader {
    pub version:    HttpVersion,
    pub headline:   HttpHeadline,
    pub fields:     HeaderFields,
}

impl Default for HttpHeader {
    fn default() -> Self {
        Self {
            version:    HttpVersion::Http1_1,
            headline:   HttpHeadline::Request {
                method: HttpMethod::GET,
                loc:    HttpLocator::default(),
            },
            fields:     HeaderFields::default(),
        }
    }
}

impl fmt::Display for HttpHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.headline {
            HttpHeadline::Request { method, loc } => {
                write!(f, "{} {} {}\r\n", method, loc, self.version)?;
            },
            HttpHeadline::Response { status } => {
                write!(f, "{} {} {}\r\n", self.version, status, status.desc())?;
            },
        }
        for (k, header_field_values) in self.fields.iter() {
            for header_field_value in header_field_values {
                write!(f, "{}: {}\r\n", k, header_field_value)?;
            }
        }
        write!(f, "\r\n")
    }
}

impl HttpHeader {
    pub fn as_vec(&self) -> Vec<u8> {
        fmt!("{}", self).into_bytes()
    }

    pub async fn read<
        'a,
        const CHUNK_SIZE: usize,
        R: AsyncRead + Unpin,
    >(
        mut stream: Pin<&mut R>,
        remnant:    &Vec<u8>,
        is_request: Option<bool>,
    )
        -> Outcome<Option<(Self, Vec<u8>, usize)>>
    {
        //trace!("Entered HttpHeader::read");
        let mut header_bytes = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];
    
        // Check if the remnant contains a complete header
        if let Some(pos) = remnant.windows(4).position(|w| w == b"\r\n\r\n") {
            header_bytes.extend_from_slice(&remnant[..pos]);
            let remnant = remnant[pos + 4..].to_vec();
            let (header_str, content_length) = res!(Self::parse_header_str(&header_bytes));
            return Ok(Some((
                res!(Self::parse(header_str, is_request)),
                remnant,
                content_length,
            )));
        }
    
        // Append the remnant to the header bytes
        header_bytes.extend_from_slice(&remnant);
    
        // Read from the stream until the header is complete
        loop {
            //trace!("Attempting to read from stream...");
            let result = stream.as_mut().read(&mut buf).await;
            match result {
                Ok(bytes_read) => {
                    trace!("Successfully read {} bytes into buf of size {}", bytes_read, CHUNK_SIZE);
    
                    if let Some(pos) = buf[..bytes_read].windows(4).position(|w| w == b"\r\n\r\n") {
                        trace!("FOUND at pos = {}", pos);
                        header_bytes.extend_from_slice(&buf[..pos]);
                        let remnant = buf[pos + 4..bytes_read].to_vec();
                        let (header_str, content_length) = res!(Self::parse_header_str(&header_bytes));
                        return Ok(Some((
                            res!(Self::parse(header_str, is_request)),
                            remnant,
                            content_length,
                        )));
                    } else {
                        header_bytes.extend_from_slice(&buf[..bytes_read]);
                    }
    
                    if bytes_read == 0 {
                        return Ok(None);
                    }
                }
                Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                    warn!("UnexpectedEof treated as connection closure.");
                    return Ok(None);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    
    fn parse_header_str(header_bytes: &[u8]) -> Outcome<(String, usize)> {
        let header_str = match std::str::from_utf8(header_bytes) {
            Ok(s) => s.to_string(),
            Err(e) => return Err(err!(e,
                "Invalid UTF-8 sequence in header bytes.";
            IO, Network, Invalid, Input)),
        };
    
        let header = res!(Self::parse(header_str.clone(), None));
    
        let content_length = match header.fields.get_one(&HeaderName::ContentLength) {
            Some(HeaderFieldValue::ContentLength(n)) => *n,
            _ => 0,
        };
    
        Ok((header_str, content_length))
    }


    pub fn parse(
        header_str: String,
        is_request: Option<bool>,
    )
        -> Outcome<Self>
    {
        // Parse the headline.
        let mut header = Self::default();
        let mut lines = header_str.lines();
        let (headline, version) = match lines.next() {
            Some(line) => res!(HttpHeadline::parse(line, is_request)),
            None => return Err(err!(
                "HTTP request missing headline.";
            IO, Network, Missing, Input)),
        };
    
        header.version = version;
        header.headline = headline;
    
        // Parse the fields.
        let mut i: u16 = 1;
        let mut ml: u8 = 1;
        let mut current_header = String::new();
        for line in lines {

            // Accommodate multi-line headers.
            let is_a_continuation = line.starts_with(' ') || line.starts_with('\t');
            if ml == 1 || is_a_continuation {
                current_header.push_str(line.trim_start());
                ml += 1;
                if ml < constant::HTTP_HEADER_MAX_MULTILINES {
                    if is_a_continuation {
                        continue;
                    }
                } else {
                    return Err(err!(
                        "The HTTP header '{}' has stretched across {} lines, exceeding \
                        the limit for this server.", current_header,
                        constant::HTTP_HEADER_MAX_MULTILINES;
                    IO, Network, Invalid, Input));
                }
            }
    
            if !current_header.is_empty() {
                // Check for exceeding max fields
                if i > constant::HTTP_HEADER_MAX_FIELDS {
                    return Err(err!(
                        "Number of header fields exceeds limit of {}.",
                        constant::HTTP_HEADER_MAX_FIELDS;
                    IO, Network, Invalid, Input));
                }
                let hf = res!(HeaderField::new(&current_header, Some(i)));
                header.fields.insert(hf.name, hf.value, Some(i));
                i += 1;
                ml = 1;
                current_header = String::new();
            }
        }
    
        Ok(header)
    }

    pub fn get_a_field_value(&self, nam: &HeaderName) -> Option<&HeaderFieldValue> { 
        self.fields.get_one(&nam)
    }

    pub fn get_the_field_value(&self, nam: &HeaderName) -> Outcome<&HeaderFieldValue> { 
        self.fields.get_the_one(&nam)
    }
}
