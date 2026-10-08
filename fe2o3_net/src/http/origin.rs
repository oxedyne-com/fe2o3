//! Which posts a browser sent from another site.
//!
//! A state-changing post on a public endpoint is a form that any page can aim a visitor's browser
//! at. A browser declares where the post came from, in `Sec-Fetch-Site` and in `Origin`, and a page
//! cannot forge either. A client that is not a browser sends neither, and is left to whatever limit
//! applies to its own address.

use crate::http::{
    fields::{
        HeaderFields,
        HeaderName,
    },
    loc::Url,
};

use oxedyne_fe2o3_core::prelude::*;

/// Did a browser send this post from another site?
///
/// `Sec-Fetch-Site`, where present, decides: the post passes only if every line says `same-origin`.
/// Otherwise an `Origin` must name `expected_origin` on every line (scheme and host without case, a
/// default port as good as none), and `null` does not. Neither header present passes. Neither field
/// is a singleton, so a caller can send several lines, and one foreign line refuses the post.
///
/// Where `expected_origin` is empty or is no URL, the `Host` the visitor addressed stands in for it
/// and the scheme goes unchecked. An `Origin` that then cannot be checked at all is refused.
pub fn cross_site(headers: &HeaderFields, expected_origin: &str) -> bool {
    if let Some(list) = headers.get_list(&HeaderName::SecFetchSite) {
        if !list.is_empty() {
            return !list.iter().all(|v| fmt!("{}", v).trim().eq_ignore_ascii_case("same-origin"));
        }
    }
    let origins = match headers.get_list(&HeaderName::Origin) {
        Some(list) if !list.is_empty()  => list,
        _                               => return false,
    };
    let want = match Url::parse(expected_origin) {
        Ok(u)   => Some(u.origin()),
        Err(_)  => None,
    };
    let host = headers.get_one(&HeaderName::Host).map(|v| fmt!("{}", v).trim().to_lowercase());
    origins.iter().any(|v| !origin_is_ours(fmt!("{}", v).trim(), want.as_deref(), host.as_deref()))
}

fn origin_is_ours(raw: &str, want: Option<&str>, host: Option<&str>) -> bool {
    // Userinfo is no part of an origin, and `Url::parse` would discard it.
    if raw.contains('@') {
        return false;
    }
    let url = match Url::parse(raw) {
        Ok(u)   => u,
        Err(_)  => return false, // `null`, and anything else that is no origin
    };
    if url.target != "/" {
        return false; // an origin has no path
    }
    match (want, host) {
        (Some(w), _)        => url.origin() == w,
        (None, Some(h))     => {
            let bare = if url.port == url.scheme.default_port() {
                url.host.clone()
            } else {
                fmt!("{}:{}", url.host, url.port)
            };
            h == bare || h == fmt!("{}:{}", url.host, url.port)
        },
        (None, None)        => false,
    }
}
