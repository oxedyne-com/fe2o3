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
                ok!(write!(f, "{} {} {}\r\n", method, loc, self.version));
            },
            HttpHeadline::Response { status } => {
                ok!(write!(f, "{} {} {}\r\n", self.version, status, status.desc()));
            },
        }
        for (k, header_field_values) in self.fields.iter() {
            for header_field_value in header_field_values {
                // A field that renders to an empty value tells the peer nothing,
                // so it is left off the wire rather than sent as a bare name.
                if header_field_value.is_wire_empty() {
                    continue;
                }
                ok!(write!(f, "{}: {}\r\n", k, header_field_value));
            }
        }
        write!(f, "\r\n")
    }
}

impl HttpHeader {
    pub fn as_vec(&self) -> Vec<u8> {
        fmt!("{}", self).into_bytes()
    }

    /// Find the `CRLF CRLF` that ends the header block, scanning from `from`.
    ///
    /// The search runs over the accumulated header bytes rather than the latest
    /// read, and the caller resumes it three bytes back from the join, so a
    /// terminator split across two reads in any of its three places is still
    /// found.
    fn find_terminator(bytes: &[u8], from: usize) -> Option<usize> {
        bytes[from..]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|pos| from + pos)
    }

    pub async fn read<
        'a,
        const CHUNK_SIZE: usize,
        R: AsyncRead + Unpin,
    >(
        mut stream: Pin<&mut R>,
        remnant:    &Vec<u8>,
        is_request: Option<bool>,
        limits:     Option<&crate::http::msg::ReadLimits>,
    )
        -> Outcome<Option<(Self, Vec<u8>, usize)>>
    {
        //trace!("Entered HttpHeader::read");
        let mut header_bytes = Vec::new();
        let mut buf = [0u8; CHUNK_SIZE];

        let max_header_bytes = limits.and_then(|l| l.max_header_bytes);
        let header_timeout = limits.and_then(|l| l.header_read_timeout);
        let started = std::time::Instant::now();

        // The remnant of the previous message may already hold a complete header.
        header_bytes.extend_from_slice(&remnant);
        if let Some(pos) = Self::find_terminator(&header_bytes, 0) {
            let rest = header_bytes[pos + 4..].to_vec();
            header_bytes.truncate(pos);
            let (header_str, content_length) = res!(Self::parse_header_str(&header_bytes));
            return Ok(Some((
                res!(Self::parse(header_str, is_request)),
                rest,
                content_length,
            )));
        }

        if let Some(lim) = max_header_bytes {
            if header_bytes.len() > lim {
                return Err(err!(
                    "HTTP header carry-over from previous request ({} \
                    bytes) already exceeds the configured header limit \
                    of {} bytes.", header_bytes.len(), lim;
                    IO, Network, Input, TooBig));
            }
        }

        // Read from the stream until the header is complete.
        loop {
            // Slowloris guard: bound the total wall-clock time the
            // reader will spend accumulating header bytes. The
            // elapsed check runs on every loop iteration so a client
            // that manages to ship one byte before the deadline
            // still gets evicted on the next iteration.
            let bytes_read = match header_timeout {
                Some(budget) => {
                    let elapsed = started.elapsed();
                    if elapsed >= budget {
                        return Err(err!(
                            "HTTP header read timed out after {:?} \
                            (limit {:?}).", elapsed, budget;
                            IO, Network, Input, Timeout));
                    }
                    let remaining = budget - elapsed;
                    let result = tokio::time::timeout(
                        remaining,
                        stream.as_mut().read(&mut buf),
                    ).await;
                    match result {
                        Ok(Ok(bytes_read)) => bytes_read,
                        Ok(Err(e)) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                            warn!("UnexpectedEof treated as connection closure.");
                            return Ok(None);
                        },
                        Ok(Err(e)) => return Err(e.into()),
                        Err(_) => return Err(err!(
                            "HTTP header read timed out after {:?} with \
                            {} bytes accumulated so far.",
                            budget, header_bytes.len();
                            IO, Network, Input, Timeout)),
                    }
                },
                // No deadline configured.
                None => match stream.as_mut().read(&mut buf).await {
                    Ok(bytes_read) => bytes_read,
                    Err(e) if e.kind() == tokio::io::ErrorKind::UnexpectedEof => {
                        warn!("UnexpectedEof treated as connection closure.");
                        return Ok(None);
                    },
                    Err(e) => return Err(e.into()),
                },
            };

            // The peer closed before completing its header block.
            if bytes_read == 0 {
                return Ok(None);
            }
            trace!("Successfully read {} bytes into buf of size {}", bytes_read, CHUNK_SIZE);

            if let Some(lim) = max_header_bytes {
                if header_bytes.len().saturating_add(bytes_read) > lim {
                    return Err(err!(
                        "HTTP header exceeds configured limit of \
                        {} bytes.", lim;
                        IO, Network, Input, TooBig));
                }
            }

            // A header block larger than one chunk arrives over several reads,
            // and its terminator can straddle the join between any two of them.
            // The search therefore runs over the accumulated bytes, resuming
            // three back from the join so a `CRLF CRLF` split 1/3, 2/2 or 3/1 is
            // still seen.  Searching only the latest read, as this once did, lost
            // every such message in silence.
            let scan_from = header_bytes.len().saturating_sub(3);
            header_bytes.extend_from_slice(&buf[..bytes_read]);

            if let Some(pos) = Self::find_terminator(&header_bytes, scan_from) {
                trace!("FOUND at pos = {}", pos);
                let rest = header_bytes[pos + 4..].to_vec();
                header_bytes.truncate(pos);
                let (header_str, content_length) = res!(Self::parse_header_str(&header_bytes));
                return Ok(Some((
                    res!(Self::parse(header_str, is_request)),
                    rest,
                    content_length,
                )));
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


#[cfg(test)]
mod reader_tests {
    use super::*;

    use std::{
        collections::VecDeque,
        io,
        task::{
            Context,
            Poll,
        },
    };

    use tokio::io::ReadBuf;

    /// A reader that hands back the chunks it was given, one per read, so a test
    /// can split the wire bytes wherever it likes.
    ///
    /// A `Cursor` returns everything it holds in a single read, and so cannot
    /// present a header that arrives in pieces.  That is exactly how the
    /// terminator bug survived: every test fed the reader a stream that could not
    /// exhibit it.
    struct Scripted {
        chunks: VecDeque<Vec<u8>>,
    }

    impl Scripted {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self { chunks: chunks.into() }
        }

        /// Dribble the bytes out `n` at a time.
        fn dribble(bytes: &[u8], n: usize) -> Self {
            Self::new(bytes.chunks(n).map(|c| c.to_vec()).collect())
        }
    }

    impl AsyncRead for Scripted {
        fn poll_read(
            mut self:   Pin<&mut Self>,
            _cx:        &mut Context<'_>,
            buf:        &mut ReadBuf<'_>,
        )
            -> Poll<io::Result<()>>
        {
            match self.chunks.pop_front() {
                Some(chunk) => {
                    let n = std::cmp::min(chunk.len(), buf.remaining());
                    buf.put_slice(&chunk[..n]);
                    if n < chunk.len() {
                        self.chunks.push_front(chunk[n..].to_vec());
                    }
                    Poll::Ready(Ok(()))
                },
                // Nothing left: a read of zero bytes, as from a closed peer.
                None => Poll::Ready(Ok(())),
            }
        }
    }

    fn read_header(
        mut stream: Scripted,
        is_request: Option<bool>,
    )
        -> Outcome<Option<(HttpHeader, Vec<u8>, usize)>>
    {
        let rt = res!(tokio::runtime::Runtime::new());
        rt.block_on(HttpHeader::read::<{ constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE }, _>(
            Pin::new(&mut stream),
            &Vec::new(),
            is_request,
            None,
        ))
    }

    /// The `CRLF CRLF` that ends a header block can straddle the join between two
    /// reads.  Searching only the latest read never sees it, and the message is
    /// then lost in silence.
    #[test]
    fn test_a_terminator_split_between_two_reads_is_still_found() -> Outcome<()> {
        let wire: &[u8] = b"GET /a HTTP/1.1\r\nhost: example.test\r\n\r\n";
        let term = wire.len() - 4; // Where the terminator begins.
        for split in 1..=3 { // Split 1/3, then 2/2, then 3/1.
            let at = term + split;
            let stream = Scripted::new(vec![
                wire[..at].to_vec(),
                wire[at..].to_vec(),
            ]);
            let (header, remnant, content_length) = match res!(read_header(stream, Some(true))) {
                Some(triple) => triple,
                None => return Err(err!(
                    "A header whose terminator was split {}/{} across two reads \
                    was read as no message at all.", split, 4 - split;
                    Test, Missing)),
            };
            assert_eq!(header.as_vec(), wire.to_vec());
            assert!(remnant.is_empty());
            assert_eq!(content_length, 0);
        }
        Ok(())
    }

    /// A header block bigger than one read has to be assembled across several.
    /// `HTTP_DEFAULT_HEADER_CHUNK_SIZE` is 1500, so this one cannot arrive whole
    /// however the stream behaves.
    #[test]
    fn test_a_header_larger_than_one_read_is_assembled() -> Outcome<()> {
        let pad = "p".repeat(2_000);
        let wire = fmt!(
            "GET /a HTTP/1.1\r\nhost: example.test\r\nx-pad: {}\r\n\r\nBODY", pad);
        let expect = fmt!(
            "GET /a HTTP/1.1\r\nhost: example.test\r\nx-pad: {}\r\n\r\n", pad);
        for n in 1..=3 { // One, two and three bytes at a time.
            let stream = Scripted::dribble(wire.as_bytes(), n);
            let (header, remnant, _) = match res!(read_header(stream, Some(true))) {
                Some(triple) => triple,
                None => return Err(err!(
                    "A {} byte header arriving {} bytes at a time was read as no \
                    message at all.", wire.len(), n;
                    Test, Missing)),
            };
            assert_eq!(header.as_vec(), expect.clone().into_bytes());
            // Only the body bytes that shared the terminator's read are in hand.
            assert!(b"BODY".starts_with(&remnant[..]));
        }
        Ok(())
    }
}
