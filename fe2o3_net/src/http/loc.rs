/// HTTP URL locator parsing and manipulation.
///
/// This module handles parsing and manipulating HTTP URL locators, which consist of:
/// - A path component
/// - Optional query parameters
/// - Optional fragment identifier
///
/// The locator follows the standard URL format:
/// `path?param1=value1&param2=value2#fragment`
///
/// # Examples
/// ```
/// use oxedyne_fe2o3_core::prelude::*;
/// use oxedyne_fe2o3_jdat::prelude::*;
/// use oxedyne_fe2o3_net::http::loc::HttpLocator;
///
/// # fn main() -> Outcome<()> {
/// let loc = res!(HttpLocator::new("/path?key=value#section"));
/// assert_eq!(loc.path.as_str(), "/path");
/// assert!(loc.data.contains_key(&dat!("key")));
/// assert_eq!(loc.frag, "section");
/// # Ok(())
/// # }
/// ```
use crate::file::RequestPath;
use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::fmt;


/// Represents a parsed HTTP URL locator with path, query params, and fragment.
///
/// # Fields
/// * `path` - The validated request path component.
/// * `data` - Map of parsed query parameters.
/// * `query` - The raw, verbatim query substring (no leading `?`), kept so a
///   proxy can forward it byte-for-byte. The parse into `data` is convenient
///   for handlers but lossy (encoding, ordering, repeated keys), so it must not
///   be used to reconstruct a request target.
/// * `frag` - Fragment identifier (part after #).
#[derive(Clone, Debug, Default)]
pub struct HttpLocator {
    pub path:  RequestPath,
    pub data:  DaticleMap,
    pub query: String,
    pub frag:  String,
}

impl fmt::Display for HttpLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Write the path
        write!(f, "{}", self.path.as_str())?;

        if !self.data.is_empty() {
            write!(f, "?")?;
            for (i, (k, v)) in self.data.iter().enumerate() {
                write!(f, "{}={}", k, v)?;
                if i < self.data.len() - 1 {
                    write!(f, "&")?;
                }
            }
        }

        if !self.frag.is_empty() {
            write!(f, "#{}", self.frag)?;
        }

        Ok(())
    }
}

impl HttpLocator {

    pub fn new<S: Into<String>>(loc: S) -> Outcome<Self> {
        let (path, data, query, frag) = res!(Self::parse_locator_string(&loc.into()));
        Ok(Self {
            path,
            data,
            query,
            frag,
        })
    }

    fn parse_locator_string(path: &str) -> Outcome<(RequestPath, DaticleMap, String, String)> {
        let mut split = path.split('?');
        let path = RequestPath::new(split.next().unwrap_or_default().to_string());
        let rest = split.next().unwrap_or_default();
        let mut split_rest = rest.split('#');
        let query_string = split_rest.next().unwrap_or_default();
        let fragment = split_rest.next().unwrap_or_default();
        let map = res!(Self::parse_query_string(query_string));

        // Retain the raw query verbatim as well as the parsed map: a proxy must
        // forward it byte-for-byte, and the parsed map cannot be relied on to
        // reconstruct it faithfully.
        Ok((path, map, query_string.to_string(), fragment.to_string()))
    }

    fn parse_query_string(query: &str) -> Outcome<DaticleMap> {
        let mut result = DaticleMap::new(); 
        for pair in query.split('&') {
            let mut split = pair.split('=');
            if let (Some(k), Some(v)) = (split.next(), split.next()) {
                let k = res!(Dat::from_str(k));
                let v = res!(Dat::from_str(v));
                result.insert(k, v);
            }
        }
        Ok(result)
        //Ok(query.split('&')
        //    .filter_map(|pair| {
        //        let mut split = pair.split('=');
        //        if let (Some(k), Some(v)) = (split.next(), split.next()) {
        //            let k = res!(Dat::from_str(k));
        //            let v = res!(Dat::from_str(v));
        //            Some((k, v))
        //        } else {
        //            None
        //        }
        //    })
        //    .collect()
        //)
    }

}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ ABSOLUTE URL                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

/// The scheme of an absolute HTTP URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UrlScheme {
    /// Plain HTTP, port 80 unless the URL says otherwise.
    Http,
    /// HTTP over TLS, port 443 unless the URL says otherwise.
    Https,
}

impl fmt::Display for UrlScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http	=> write!(f, "http"),
            Self::Https	=> write!(f, "https"),
        }
    }
}

impl UrlScheme {
    /// The port the scheme implies when the URL names none.
    pub fn default_port(&self) -> u16 {
        match self {
            Self::Http	=> 80,
            Self::Https	=> 443,
        }
    }

    /// Whether the connection this scheme names is TLS-wrapped.
    pub fn is_tls(&self) -> bool {
        matches!(self, Self::Https)
    }
}

/// An absolute HTTP URL, split into the four things a client needs to act on it.
///
/// [`HttpLocator`] is the *server's* view of a URL -- the request target and
/// nothing else, because the host arrived separately in the `Host` header. This
/// is the *client's* view: the whole of it, including whom to connect to.
///
/// The parse follows a browser's, in the one place where a naive parse is a
/// security hole. In `https://example.com@127.0.0.1/`, the host is
/// `127.0.0.1` and `example.com` is discarded userinfo, so a server that reads
/// the host as everything before the first `/` reaches the loopback address it
/// meant to refuse. Here, the host is what follows the last `@`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Url {
    /// What to speak.
    pub scheme: UrlScheme,
    /// Whom to speak to: a host name or an IP literal, lowercased.
    pub host:   String,
    /// The port, explicit or implied by the scheme.
    pub port:   u16,
    /// What to ask for: the path and any query string, as written on the
    /// request line. Never empty, and never carries the fragment, which is
    /// the browser's business and is not sent.
    pub target: String,
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.port == self.scheme.default_port() {
            write!(f, "{}://{}{}", self.scheme, self.host, self.target)
        } else {
            write!(f, "{}://{}:{}{}", self.scheme, self.host, self.port, self.target)
        }
    }
}

impl Url {

    /// Parse an absolute `http` or `https` URL.
    ///
    /// Any other scheme is refused here rather than downstream, so that a
    /// `file:`, `gopher:` or `javascript:` URL cannot reach a socket at all.
    pub fn parse(raw: &str) -> Outcome<Self> {
        let raw = raw.trim();
        let (scheme_txt, rest) = match raw.find("://") {
            Some(i) => (&raw[..i], &raw[i + 3..]),
            None => return Err(err!(
                "The URL {:?} names no scheme; an absolute http:// or https:// \
                URL is required.", raw;
                Invalid, Input, Missing)),
        };
        let scheme = match scheme_txt.to_lowercase().as_str() {
            "http"	=> UrlScheme::Http,
            "https"	=> UrlScheme::Https,
            other	=> return Err(err!(
                "The URL scheme {:?} is not one this client speaks; only http \
                and https are.", other;
                Invalid, Input, Unimplemented)),
        };

        // The authority ends at the first delimiter of what follows it.
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let tail      = &rest[end..];

        // Userinfo is discarded, and the host is what follows the last `@`:
        // `https://example.com@127.0.0.1/` names the loopback address, and a
        // parser that thinks otherwise is the request forgery.
        let hostport = match authority.rfind('@') {
            Some(i) => &authority[i + 1..],
            None    => authority,
        };
        if hostport.is_empty() {
            return Err(err!(
                "The URL {:?} names no host.", raw;
                Invalid, Input, Missing));
        }

        // An IPv6 literal wears brackets, and the colons inside them are not
        // the one that introduces the port.
        let (host, port_txt) = if let Some(close) = hostport.strip_prefix('[') {
            match close.find(']') {
                Some(i) => {
                    let host = &close[..i];
                    let after = &close[i + 1..];
                    let port = match after.strip_prefix(':') {
                        Some(p) => Some(p),
                        None if after.is_empty() => None,
                        None => return Err(err!(
                            "The URL {:?} has trailing junk after its IPv6 \
                            host.", raw;
                            Invalid, Input)),
                    };
                    (host, port)
                }
                None => return Err(err!(
                    "The URL {:?} opens an IPv6 host that it never closes.", raw;
                    Invalid, Input)),
            }
        } else {
            match hostport.rfind(':') {
                Some(i) => (&hostport[..i], Some(&hostport[i + 1..])),
                None    => (hostport, None),
            }
        };
        if host.is_empty() {
            return Err(err!(
                "The URL {:?} names no host.", raw;
                Invalid, Input, Missing));
        }
        let port = match port_txt {
            Some(p) => match p.parse::<u16>() {
                Ok(n) if n > 0 => n,
                _ => return Err(err!(
                    "The URL {:?} names {:?}, which is not a port.", raw, p;
                    Invalid, Input)),
            },
            None => scheme.default_port(),
        };

        // The fragment is the browser's, and is never sent.
        let target = match tail.find('#') {
            Some(i) => &tail[..i],
            None    => tail,
        };
        let target = if target.is_empty() { "/" } else { target };

        Ok(Self {
            scheme,
            host: host.to_lowercase(),
            port,
            target: target.to_string(),
        })
    }

    /// Resolve a `Location` header against this URL, as a redirect is followed.
    ///
    /// Absolute, protocol-relative (`//host/path`), root-relative (`/path`) and
    /// relative (`path`) forms are all handled, because servers send all four.
    pub fn join(&self, loc: &str) -> Outcome<Self> {
        let loc = loc.trim();
        if loc.is_empty() {
            return Err(err!(
                "The redirect names no location."; Invalid, Input, Missing));
        }
        // An absolute URL leads with its scheme. A relative one whose query
        // happens to carry `://` -- a `?next=https://elsewhere` -- must not be
        // mistaken for one, so the scheme is matched only at the very start and
        // only where every character before the `://` could belong to a scheme.
        if let Some(i) = loc.find("://") {
            let scheme = &loc[..i];
            if !scheme.is_empty()
                && scheme.chars().all(|c|
                    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            {
                return Self::parse(loc);
            }
        }
        if let Some(rest) = loc.strip_prefix("//") {
            return Self::parse(&fmt!("{}://{}", self.scheme, rest));
        }
        let base = if self.port == self.scheme.default_port() {
            fmt!("{}://{}", self.scheme, self.host)
        } else {
            fmt!("{}://{}:{}", self.scheme, self.host, self.port)
        };
        if loc.starts_with('/') {
            return Self::parse(&fmt!("{}{}", base, loc));
        }
        // Relative to the directory the current target sits in. The query is no
        // part of the path, and is dropped before the directory is found, or a
        // `/` inside a `?query` would be read as a directory boundary it is not.
        let path = match self.target.find('?') {
            Some(i) => &self.target[..i],
            None    => &self.target,
        };
        let dir = match path.rfind('/') {
            Some(i) => &path[..i + 1],
            None    => "/",
        };
        Self::parse(&fmt!("{}{}{}", base, dir, loc))
    }

    /// The origin as a browser writes it: `scheme://host[:port]`, with the
    /// port shown only when it is not the scheme's own.
    pub fn origin(&self) -> String {
        if self.port == self.scheme.default_port() {
            fmt!("{}://{}", self.scheme, self.host)
        } else {
            fmt!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }
}


#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn test_a_locator_keeps_its_raw_query_for_forwarding() -> Outcome<()> {
        let loc = res!(HttpLocator::new("/api/admin?view=geo&page=2"));
        assert_eq!(loc.path.as_string(), "/api/admin");
        // The raw query survives verbatim, no leading `?`, for a proxy to
        // forward byte-for-byte -- the parse into `data` is a convenience only.
        assert_eq!(loc.query, "view=geo&page=2");
        Ok(())
    }

    #[test]
    fn test_a_locator_with_no_query_has_an_empty_query() -> Outcome<()> {
        let loc = res!(HttpLocator::new("/api/health"));
        assert_eq!(loc.query, "");
        Ok(())
    }

    #[test]
    fn test_a_plain_url_parses_into_its_parts() -> Outcome<()> {
        let u = res!(Url::parse("https://example.com/a/b?x=1#frag"));
        assert_eq!(u.scheme, UrlScheme::Https);
        assert_eq!(u.host,   "example.com");
        assert_eq!(u.port,   443);
        // The fragment is never sent.
        assert_eq!(u.target, "/a/b?x=1");
        assert_eq!(fmt!("{}", u), "https://example.com/a/b?x=1");
        Ok(())
    }

    #[test]
    fn test_a_bare_host_gets_the_schemes_port_and_a_root_target() -> Outcome<()> {
        let u = res!(Url::parse("http://example.com"));
        assert_eq!(u.port,   80);
        assert_eq!(u.target, "/");
        let s = res!(Url::parse("https://example.com:8443/x"));
        assert_eq!(s.port, 8443);
        assert_eq!(fmt!("{}", s), "https://example.com:8443/x");
        Ok(())
    }

    #[test]
    fn test_userinfo_does_not_become_the_host() -> Outcome<()> {
        // The classic disguise: the host is the loopback address, and
        // `example.com` is userinfo a browser throws away.
        let u = res!(Url::parse("https://example.com@127.0.0.1/admin"));
        assert_eq!(u.host, "127.0.0.1");
        let v = res!(Url::parse("https://user:pass@evil.test:8080/"));
        assert_eq!(v.host, "evil.test");
        assert_eq!(v.port, 8080);
        Ok(())
    }

    #[test]
    fn test_an_ipv6_literal_keeps_its_colons() -> Outcome<()> {
        let u = res!(Url::parse("http://[2606:4700::1111]:8080/x"));
        assert_eq!(u.host, "2606:4700::1111");
        assert_eq!(u.port, 8080);
        let v = res!(Url::parse("http://[::1]/"));
        assert_eq!(v.host, "::1");
        assert_eq!(v.port, 80);
        Ok(())
    }

    #[test]
    fn test_a_scheme_this_client_does_not_speak_is_refused() {
        for raw in [
            "file:///etc/passwd",
            "gopher://example.com/",
            "javascript://example.com/",
            "ftp://example.com/x",
        ] {
            assert!(Url::parse(raw).is_err(), "{} should not parse", raw);
        }
        // And so is a URL with no scheme at all.
        assert!(Url::parse("example.com/x").is_err());
        assert!(Url::parse("https://").is_err());
    }

    #[test]
    fn test_a_redirect_resolves_in_all_four_forms() -> Outcome<()> {
        let base = res!(Url::parse("https://example.com/a/b?x=1"));

        let abs = res!(base.join("http://other.test/z"));
        assert_eq!(fmt!("{}", abs), "http://other.test/z");

        let proto = res!(base.join("//other.test/z"));
        assert_eq!(fmt!("{}", proto), "https://other.test/z");

        let root = res!(base.join("/z?q=2"));
        assert_eq!(fmt!("{}", root), "https://example.com/z?q=2");

        let rel = res!(base.join("c/d"));
        assert_eq!(fmt!("{}", rel), "https://example.com/a/c/d");
        Ok(())
    }

    #[test]
    fn test_a_redirect_can_change_the_host_it_points_at() -> Outcome<()> {
        // Which is exactly why a caller must vet every hop, not only the first.
        let base = res!(Url::parse("https://example.com/a"));
        let next = res!(base.join("http://127.0.0.1:80/latest/meta-data/"));
        assert_eq!(next.host, "127.0.0.1");
        assert_eq!(next.port, 80);
        Ok(())
    }

    #[test]
    fn test_a_scheme_in_a_query_is_not_an_absolute_redirect() -> Outcome<()> {
        // `://` sitting inside a query is not a scheme, so this stays a
        // same-origin, root-relative redirect rather than jumping to `evil`.
        let base = res!(Url::parse("https://example.com/a"));
        let next = res!(base.join("/login?return=https://evil.test/x"));
        assert_eq!(next.host, "example.com");
        assert_eq!(next.target, "/login?return=https://evil.test/x");
        Ok(())
    }

    #[test]
    fn test_a_relative_redirect_resolves_against_the_path_not_the_query() -> Outcome<()> {
        // The base carries a query with a `/` in it. That slash is no directory
        // boundary, so the relative target resolves against `/a/`, not `/a/x=`.
        let base = res!(Url::parse("https://example.com/a/b?next=/x/y"));
        let next = res!(base.join("c"));
        assert_eq!(next.target, "/a/c");
        Ok(())
    }
}
