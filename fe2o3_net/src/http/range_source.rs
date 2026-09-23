//! A remote file read by byte range over HTTP or HTTPS, as a PMTiles [`RangeSource`].
//!
//! `fe2o3_geom` reads a PMTiles archive through any [`RangeSource`] and carries no network
//! code; this is the network half.  One request per range, `Connection: close`, as the rest of
//! this client does.  A ranged request must come back `206 Partial Content` with exactly the
//! bytes asked for; a server that ignores `Range` and sends the whole file is refused rather
//! than read, since the file may be a hundred gigabytes.
//!
//! [`HttpRangeSource::read_async`] is the form for a caller already in an async runtime.  The
//! synchronous [`RangeSource::read`] runs the request on a runtime of its own and refuses,
//! rather than panics, when called from inside another.

use crate::http::{
    client::{
        http_request,
        https_request,
    },
    header::{
        HttpHeadline,
        HttpMethod,
    },
    status::HttpStatus,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::tile::pmtiles::RangeSource;

use std::sync::Arc;

use tokio_rustls::rustls::ClientConfig;

/// A remote file, addressed by URL, read by `Range` requests.
pub struct HttpRangeSource {
    host:   String,
    port:   u16,
    path:   String,
    tls:    Option<Arc<ClientConfig>>,  // None for plain HTTP
}

impl HttpRangeSource {
    /// A source for an `https://` or `http://` URL.  An `https://` URL needs `tls`, for which
    /// [`crate::tls::default_client_config`] gives the host's own trust store.
    pub fn new(url: &str, tls: Option<Arc<ClientConfig>>) -> Outcome<Self> {
        let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(err!("{:?} is not an http or https URL.", url; Invalid, Input));
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None    => (rest, "/".to_string()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), res!(p.parse::<u16>(), Invalid, Input)),
            None => (authority.to_string(), if secure { 443 } else { 80 }),
        };
        if host.is_empty() {
            return Err(err!("{:?} names no host.", url; Invalid, Input, Missing));
        }
        if secure && tls.is_none() {
            return Err(err!("{:?} is https, and no TLS configuration was given.", url;
                Invalid, Input, Missing));
        }
        Ok(Self { host, port, path, tls: if secure { tls } else { None } })
    }

    /// Reads `len` bytes from `offset`.
    pub async fn read_async(&self, offset: u64, len: u64) -> Outcome<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let last = match offset.checked_add(len - 1) {
            Some(l) => l,
            None => return Err(err!("Bytes {}+{} run past the end of any file.", offset, len;
                Invalid, Input, Range)),
        };
        let range = fmt!("bytes={}-{}", offset, last);
        let headers = [("Range", range.as_str()), ("Accept-Encoding", "identity")];
        let msg = match &self.tls {
            Some(tls) => res!(https_request(&self.host, self.port, HttpMethod::GET, &self.path,
                &headers, &[], tls.clone()).await),
            None => res!(http_request(&self.host, self.port, HttpMethod::GET, &self.path,
                &headers, &[]).await),
        };
        match &msg.header.headline {
            HttpHeadline::Response { status: HttpStatus::PartialContent } => (),
            HttpHeadline::Response { status } => return Err(err!(
                "{}{} answered {:?} to {}, not 206 Partial Content.", self.host, self.path,
                status, range; Network, Unexpected)),
            _ => return Err(err!("{} sent a request in reply.", self.host; Network, Unexpected)),
        }
        if msg.body.len() as u64 != len {
            return Err(err!("{}{} sent {} bytes for {}, which is {} bytes.", self.host, self.path,
                msg.body.len(), range, len; Network, Mismatch, Size));
        }
        Ok(msg.body)
    }
}

impl RangeSource for HttpRangeSource {
    fn read(&self, offset: u64, len: u64) -> Outcome<Vec<u8>> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(err!("A blocking range read was asked for inside an async runtime; \
                use read_async there."; Invalid, Thread));
        }
        // A runtime a read, built and dropped here: one kept in the source would panic when
        // dropped by a caller that had since moved into an async context.
        let rt = res!(tokio::runtime::Builder::new_current_thread().enable_all().build(),
            Init, Thread);
        rt.block_on(self.read_async(offset, len))
    }
}
