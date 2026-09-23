//! Map tiles from a local archive, served so that nothing about the viewer is kept.
//!
//! A tile request names a place at street resolution, so a record of tile requests is a record of
//! where each viewer looked. The route is built so that no such record can be made by it:
//!
//! - It is answered in `srv::https` before the request is logged, before the session cookie is
//!   read and before the traffic recorder sees it, and the vhost carrying it must set
//!   `access_log: false`, which silences the connection lines as well.
//! - [`TileRequest`] holds the method, path, `Origin` and `Accept-Encoding` and nothing else, so a
//!   cookie or an identifier in the request cannot reach the code that answers it.
//! - No response sets a cookie, and a failed read logs the build, never the tile.
//!
//! The archive is reached through [`TileArchive`] alone: the directory index and the byte-range
//! read belong to the PMTiles reader in `fe2o3_geom::tile::pmtiles`, and this module asks it only
//! for the bytes stored for one tile.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::srv::cfg::TileConfig;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::tile::pmtiles::{
    Archive,
    Compression,
    FileSource,
    TileType,
};
use oxedyne_fe2o3_jdat::string::enc::escape_json_string;
use oxedyne_fe2o3_net::http::{
    encoding::{
        self,
        ContentCoding,
    },
    fields::{
        HeaderFieldValue,
        HeaderName,
    },
    header::{
        HttpHeadline,
        HttpMethod,
    },
    msg::HttpMessage,
    status::HttpStatus,
};

use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
};


// Cache lifetimes
pub const TILE_MAX_AGE_SECS:    u64 = 31_536_000;   // a year: the URL names the build
pub const INDEX_MAX_AGE_SECS:   u64 = 300;          // how soon a new build is picked up
pub const INDEX_NAME:           &str = "tiles.json";

/// What a tile archive holds, as its header states it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileInfo {
    pub kind:       TileKind,
    pub coding:     ContentCoding,  // how each stored tile is compressed
    pub min_zoom:   u8,
    pub max_zoom:   u8,
    pub bounds_e7:  [i32; 4],       // min lon, min lat, max lon, max lat, in degrees × 10⁷
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileKind {
    Mvt,
    Png,
    Jpeg,
    Webp,
    Avif,
}

impl TileKind {
    /// The extension a tile URL ends in.
    pub fn ext(&self) -> &'static str {
        match self {
            Self::Mvt   => "mvt",
            Self::Png   => "png",
            Self::Jpeg  => "jpg",
            Self::Webp  => "webp",
            Self::Avif  => "avif",
        }
    }

    pub fn media_type(&self) -> &'static str {
        match self {
            Self::Mvt   => "application/vnd.mapbox-vector-tile",
            Self::Png   => "image/png",
            Self::Jpeg  => "image/jpeg",
            Self::Webp  => "image/webp",
            Self::Avif  => "image/avif",
        }
    }
}

/// The narrow interface between this route and a tile archive reader.
///
/// `tile` returns the bytes stored for one tile, still in the archive's coding, or `None` where
/// the archive holds nothing for it. It blocks on file I/O, so the route calls it off the async
/// workers.
pub trait TileArchive: Send + Sync + 'static {
    fn info(&self) -> TileInfo;
    fn tile(&self, z: u8, x: u32, y: u32) -> Outcome<Option<Vec<u8>>>;
}

/// The archive readers this build of Steel can open.
pub enum TileSource {
    Pmtiles {
        archive:    Archive<FileSource>,
        info:       TileInfo,
    },
}

impl std::fmt::Debug for TileSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pmtiles { info, .. } => write!(f, "TileSource::Pmtiles({:?})", info),
        }
    }
}

impl TileSource {
    /// Opens a build's PMTiles archive, refusing one this route cannot serve as it stands: a
    /// tile kind with no media type here, or a compression other than none and gzip.  The
    /// errors name the build and the file, never a tile.
    pub fn open(build: &str, path: &Path) -> Outcome<Self> {
        let archive = match FileSource::open(path).and_then(Archive::open) {
            Ok(a) => a,
            Err(e) => return Err(err!(e,
                "Tiles: build '{}' at {:?} cannot be opened as a PMTiles archive.", build, path;
                Configuration, File)),
        };
        let h = archive.header();
        let kind = match h.tile_type {
            TileType::Mvt   => TileKind::Mvt,
            TileType::Png   => TileKind::Png,
            TileType::Jpeg  => TileKind::Jpeg,
            TileType::Webp  => TileKind::Webp,
            TileType::Avif  => TileKind::Avif,
            other => return Err(err!(
                "Tiles: build '{}' at {:?} holds {:?} tiles, which this route does not serve.",
                build, path, other; Unimplemented, Configuration)),
        };
        let coding = match h.tile_compression {
            Compression::None   => ContentCoding::Identity,
            Compression::Gzip   => ContentCoding::Gzip,
            other => return Err(err!(
                "Tiles: build '{}' at {:?} stores its tiles with {:?} compression; only none and \
                gzip are served.", build, path, other; Unimplemented, Configuration)),
        };
        let info = TileInfo {
            kind,
            coding,
            min_zoom:   h.min_zoom,
            max_zoom:   h.max_zoom,
            bounds_e7:  h.bounds_e7(),
        };
        Ok(Self::Pmtiles { archive, info })
    }
}

impl TileArchive for TileSource {
    fn info(&self) -> TileInfo {
        match self {
            Self::Pmtiles { info, .. } => *info,
        }
    }
    fn tile(&self, z: u8, x: u32, y: u32) -> Outcome<Option<Vec<u8>>> {
        match self {
            Self::Pmtiles { archive, .. } => archive.tile(z, x, y),
        }
    }
}

/// Everything the route reads from a request. A cookie, an `Authorization` field or the peer's
/// address is not here, so nothing that answers a tile request can read one.
#[derive(Clone, Debug)]
pub struct TileRequest {
    pub method:             HttpMethod,
    pub path:               String,
    pub origin:             Option<String>,
    pub accept_encoding:    Option<String>,
}

impl TileRequest {
    /// Takes the four parts the route reads, or `None` for a message that is not a request.
    pub fn of(msg: &HttpMessage) -> Option<Self> {
        match &msg.header.headline {
            HttpHeadline::Request { method, loc } => Some(Self {
                method:             method.clone(),
                path:               loc.path.as_string().to_string(),
                origin:             msg.header.fields.get_one(&HeaderName::Origin)
                                        .map(|v| fmt!("{}", v)),
                accept_encoding:    encoding::accept_encoding(&msg.header.fields),
            }),
            HttpHeadline::Response { .. } => None,
        }
    }
}

/// One vhost's tile route: a prefix, the builds served under it, and who may fetch them.
#[derive(Debug)]
pub struct TileService<A: TileArchive> {
    prefix:     String,
    current:    String,
    builds:     BTreeMap<String, Arc<A>>,
    origins:    Vec<String>,
    attribution: String,
    base_url:   String,     // `https://<primary hostname>`, for the index
    hsts_secs:  u64,
}

// The parts of a path under the prefix.
#[derive(Debug, Eq, PartialEq)]
enum Target<'a> {
    Index,
    Tile { build: &'a str, z: u8, x: u32, y: u32, ext: &'a str },
    Unknown,
}

impl<A: TileArchive> TileService<A> {

    /// Builds the route from its config, opening each build with `open`.
    pub fn new<F>(
        cfg:        &TileConfig,
        hostname:   &str,
        hsts_secs:  u64,
        mut open:   F,
    )
        -> Outcome<Self>
    where
        F: FnMut(&str, &Path) -> Outcome<A>,
    {
        let mut builds = BTreeMap::new();
        for (build, path) in &cfg.builds {
            let archive = res!(open(build, path));
            builds.insert(build.clone(), Arc::new(archive));
        }
        if !builds.contains_key(&cfg.current) {
            return Err(err!(
                "Tiles: the current build '{}' is not among the configured builds {:?}.",
                cfg.current, builds.keys().collect::<Vec<_>>();
                Invalid, Configuration, Missing));
        }
        Ok(Self {
            prefix:         cfg.prefix.clone(),
            current:        cfg.current.clone(),
            builds,
            origins:        cfg.allow_origins.clone(),
            attribution:    cfg.attribution.clone(),
            base_url:       fmt!("https://{}", hostname),
            hsts_secs,
        })
    }

    /// Does the path fall under this route's prefix? Everything under it is answered here, so
    /// nothing under it reaches the logging dispatch that follows.
    pub fn owns(&self, path: &str) -> bool {
        match path.strip_prefix(self.prefix.as_str()) {
            Some(rest) => rest.is_empty() || rest.starts_with('/'),
            None => false,
        }
    }

    /// The answer to a request under the prefix, or `None` when the path is not this route's.
    pub async fn respond(&self, req: TileRequest) -> Option<HttpMessage> {
        if !self.owns(&req.path) {
            return None;
        }
        // An origin that is present and not listed is refused outright, so a page elsewhere
        // cannot draw these tiles even where it would not be allowed to read them.
        let origin = match &req.origin {
            Some(o) if self.origins.iter().any(|a| a == o) => Some(o.clone()),
            Some(_) => return Some(self.finish(
                HttpMessage::respond_with_text(HttpStatus::Forbidden, "Origin not permitted."),
                None, false, None)),
            None => None,
        };
        let head_only = match req.method {
            HttpMethod::GET     => false,
            HttpMethod::HEAD    => true,
            HttpMethod::OPTIONS => return Some(self.preflight(origin)),
            _ => {
                let msg = HttpMessage::respond_with_text(
                    HttpStatus::MethodNotAllowed, "Method not allowed.")
                    .with_field(HeaderName::Allow,
                        HeaderFieldValue::Generic("GET, HEAD, OPTIONS".to_string()));
                return Some(self.finish(msg, origin, false, None));
            }
        };
        let msg = match self.target(&req.path) {
            Target::Index => self.index(origin),
            Target::Tile { build, z, x, y, ext } =>
                self.tile(build, z, x, y, ext, origin, req.accept_encoding.as_deref()).await,
            Target::Unknown => self.not_found(origin),
        };
        Some(if head_only { msg.head_only() } else { msg })
    }

    fn target<'a>(&self, path: &'a str) -> Target<'a> {
        let rest = match path.strip_prefix(self.prefix.as_str())
            .and_then(|r| r.strip_prefix('/'))
        {
            Some(r) => r,
            None => return Target::Unknown,
        };
        if rest == INDEX_NAME {
            return Target::Index;
        }
        let mut parts = rest.split('/');
        let (build, z, x, file) = match (
            parts.next(), parts.next(), parts.next(), parts.next(), parts.next(),
        ) {
            (Some(b), Some(z), Some(x), Some(f), None) => (b, z, x, f),
            _ => return Target::Unknown,
        };
        let (y, ext) = match file.split_once('.') {
            Some(ye) => ye,
            None => return Target::Unknown,
        };
        match (digits::<u8>(z), digits::<u32>(x), digits::<u32>(y)) {
            (Some(z), Some(x), Some(y)) => Target::Tile { build, z, x, y, ext },
            _ => Target::Unknown,
        }
    }

    async fn tile(
        &self,
        build:  &str,
        z:      u8,
        x:      u32,
        y:      u32,
        ext:    &str,
        origin: Option<String>,
        accept: Option<&str>,
    )
        -> HttpMessage
    {
        let archive = match self.builds.get(build) {
            Some(a) => a.clone(),
            None => return self.not_found(origin),
        };
        let info = archive.info();
        // A zoom outside the archive, a column or row off the edge of the world, or an
        // extension that is not the archive's kind names no tile.
        if z < info.min_zoom || z > info.max_zoom || z > 31
            || (x as u64) >> z != 0 || (y as u64) >> z != 0
            || ext != info.kind.ext()
        {
            return self.not_found(origin);
        }
        let read = tokio::task::spawn_blocking(move || archive.tile(z, x, y)).await;
        let stored = match read {
            Ok(Ok(stored)) => stored,
            // The tile is not named, since the log must not hold where anyone looked.
            Ok(Err(_)) | Err(_) => {
                error!(err!("Tiles: build '{}' could not be read; the archive may be \
                    damaged.", build; IO, Read));
                return self.finish(HttpMessage::respond_with_text(
                    HttpStatus::InternalServerError, "Tile read failed."), origin, false, None);
            }
        };
        let bytes = match stored {
            Some(b) if !b.is_empty() => b,
            // Nothing stored is an empty tile, which is as permanent as a full one.
            _ => return self.finish(
                HttpMessage::new_response(HttpStatus::NoContent), origin, true, None),
        };
        let mut msg = HttpMessage::new_response(HttpStatus::OK)
            .with_field(HeaderName::ContentType,
                HeaderFieldValue::Generic(info.kind.media_type().to_string()));
        // The stored gzip is passed through to a client that takes it, which is every browser,
        // and decoded for one that does not.
        msg = match info.coding {
            ContentCoding::Identity => msg.with_body(bytes),
            ContentCoding::Gzip => match encoding::negotiate(accept) {
                ContentCoding::Gzip => msg
                    .with_field(HeaderName::ContentEncoding,
                        HeaderFieldValue::Generic("gzip".to_string()))
                    .with_body(bytes),
                ContentCoding::Identity => match encoding::gunzip(&bytes) {
                    Ok(plain) => msg.with_body(plain),
                    Err(_) => {
                        error!(err!("Tiles: build '{}' holds a tile that is not valid gzip.",
                            build; Decode));
                        return self.finish(HttpMessage::respond_with_text(
                            HttpStatus::InternalServerError, "Tile read failed."),
                            origin, false, None);
                    }
                },
            },
        };
        let vary = match info.coding {
            ContentCoding::Gzip     => Some("Accept-Encoding"),
            ContentCoding::Identity => None,
        };
        self.finish(msg, origin, true, vary)
    }

    fn index(&self, origin: Option<String>) -> HttpMessage {
        let archive = match self.builds.get(&self.current) {
            Some(a) => a,
            None => return self.not_found(origin),
        };
        let info = archive.info();
        let b = info.bounds_e7;
        let deg = |v: i32| fmt!("{}", v as f64 / 1e7);
        let body = fmt!(
            "{{\"tilejson\":\"3.0.0\",\"build\":\"{}\",\
            \"tiles\":[\"{}{}/{}/{{z}}/{{x}}/{{y}}.{}\"],\
            \"minzoom\":{},\"maxzoom\":{},\"bounds\":[{},{},{},{}],\"attribution\":\"{}\"}}",
            escape_json_string(&self.current),
            escape_json_string(&self.base_url),
            escape_json_string(&self.prefix),
            escape_json_string(&self.current),
            info.kind.ext(),
            info.min_zoom, info.max_zoom,
            deg(b[0]), deg(b[1]), deg(b[2]), deg(b[3]),
            escape_json_string(&self.attribution),
        );
        let msg = HttpMessage::new_response(HttpStatus::OK)
            .with_field(HeaderName::ContentType,
                HeaderFieldValue::Generic("application/json".to_string()))
            .with_field(HeaderName::CacheControl,
                HeaderFieldValue::Generic(fmt!("public, max-age={}", INDEX_MAX_AGE_SECS)))
            .with_body(body.into_bytes());
        self.finish(msg, origin, false, None)
    }

    fn preflight(&self, origin: Option<String>) -> HttpMessage {
        let msg = HttpMessage::new_response(HttpStatus::NoContent)
            .with_field(HeaderName::AccessControlAllowMethods,
                HeaderFieldValue::Generic("GET, HEAD, OPTIONS".to_string()))
            .with_field(HeaderName::AccessControlMaxAge,
                HeaderFieldValue::Generic("86400".to_string()));
        self.finish(msg, origin, false, None)
    }

    fn not_found(&self, origin: Option<String>) -> HttpMessage {
        self.finish(HttpMessage::respond_with_text(HttpStatus::NotFound, "No such tile."),
            origin, false, None)
    }

    /// The fields every answer carries. `immutable` marks a tile, whose URL names its build and
    /// so never changes; anything else that has not set its own lifetime is not stored.
    fn finish(
        &self,
        mut msg:    HttpMessage,
        origin:     Option<String>,
        immutable:  bool,
        vary:       Option<&str>,
    )
        -> HttpMessage
    {
        let fields = &mut msg.header.fields;
        if immutable {
            fields.insert(HeaderName::CacheControl, HeaderFieldValue::Generic(
                fmt!("public, max-age={}, immutable", TILE_MAX_AGE_SECS)), None);
        } else if fields.get_one(&HeaderName::CacheControl).is_none() {
            fields.insert(HeaderName::CacheControl,
                HeaderFieldValue::Generic("no-store".to_string()), None);
        }
        // The answer depends on the Origin, so a shared cache must key on it.
        let vary = match vary {
            Some(v) => fmt!("Origin, {}", v),
            None    => "Origin".to_string(),
        };
        fields.insert(HeaderName::Vary, HeaderFieldValue::Generic(vary), None);
        if let Some(o) = origin {
            fields.insert(HeaderName::AccessControlAllowOrigin,
                HeaderFieldValue::Generic(o), None);
        }
        fields.insert(HeaderName::CrossOriginResourcePolicy,
            HeaderFieldValue::Generic("same-site".to_string()), None);
        fields.insert(HeaderName::XContentTypeOptions,
            HeaderFieldValue::Generic("nosniff".to_string()), None);
        fields.insert(HeaderName::ReferrerPolicy,
            HeaderFieldValue::Generic("no-referrer".to_string()), None);
        fields.insert(HeaderName::ContentSecurityPolicy,
            HeaderFieldValue::Generic("default-src 'none'; frame-ancestors 'none'".to_string()),
            None);
        if self.hsts_secs > 0 {
            fields.insert(HeaderName::StrictTransportSecurity, HeaderFieldValue::Generic(
                fmt!("max-age={}; includeSubDomains", self.hsts_secs)), None);
        }
        msg
    }

    pub fn current(&self) -> &str { &self.current }
    pub fn prefix(&self) -> &str { &self.prefix }
}

// ASCII digits only, with no sign and no leading zero, so one tile has one URL.
fn digits<N: std::str::FromStr>(s: &str) -> Option<N> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return None;
    }
    s.parse::<N>().ok()
}
