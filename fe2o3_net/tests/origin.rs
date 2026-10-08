//! Which posts a browser sent from another site, and which callers want JSON.
//!
//! A post that changes state on a public endpoint is a form anyone can aim a visitor's browser at.
//! `origin::cross_site` reads what the browser itself declares about where the post came from
//! (`Sec-Fetch-Site`, else `Origin`) and says whether to refuse it. A client that is not a browser
//! declares neither, and passes.
//!
//! The table tests build the fields directly. The wire test sends the same repeated lines through
//! the real parser, since a caller chooses how many lines it sends and what case it writes the name in.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_net::http::{
    fields::{
        HeaderFieldValue,
        HeaderFields,
        HeaderName,
    },
    origin::cross_site,
};

const SITE: &str = "https://example.com";

/// Fields as the wire parser would hold them: the name read from its lower-case text.
fn fields(lines: &[(&str, &str)]) -> Outcome<HeaderFields> {
    let mut f = HeaderFields::default();
    for (name, value) in lines {
        let nam = HeaderName::from(*name);
        let val = res!(HeaderFieldValue::new(&nam, value));
        f.insert(nam, val, None);
    }
    Ok(f)
}

fn refused(lines: &[(&str, &str)]) -> Outcome<bool> {
    Ok(cross_site(&res!(fields(lines)), SITE))
}

#[test]
fn test_the_fetch_metadata_names_are_known_00() -> Outcome<()> {
    for (text, name) in [
        ("sec-fetch-site", HeaderName::SecFetchSite),
        ("sec-fetch-mode", HeaderName::SecFetchMode),
        ("sec-fetch-dest", HeaderName::SecFetchDest),
        ("sec-fetch-user", HeaderName::SecFetchUser),
    ] {
        assert_eq!(HeaderName::from(text), name, "{} did not parse to its own name", text);
        assert_eq!(fmt!("{}", name), text, "the name did not print as {}", text);
    }
    Ok(())
}

#[test]
fn test_a_post_with_neither_header_passes_01() -> Outcome<()> {
    assert!(!res!(refused(&[])), "a client declaring nothing is not a browser, and passes");
    assert!(!res!(refused(&[("host", "example.com"), ("accept", "text/html")])));
    Ok(())
}

#[test]
fn test_fetch_metadata_passes_only_same_origin_02() -> Outcome<()> {
    assert!(!res!(refused(&[("sec-fetch-site", "same-origin")])));
    assert!(!res!(refused(&[("sec-fetch-site", "Same-Origin")])), "the token is not case-sensitive");
    for other in ["cross-site", "same-site", "none", "", "same-origin, cross-site", "nonsense"] {
        assert!(res!(refused(&[("sec-fetch-site", other)])),
            "Sec-Fetch-Site: {:?} must be refused", other);
    }
    Ok(())
}

#[test]
fn test_fetch_metadata_outranks_origin_03() -> Outcome<()> {
    // Where the browser sends both, the fetch metadata decides: it cannot be set from script.
    assert!(res!(refused(&[("sec-fetch-site", "cross-site"), ("origin", SITE)])));
    assert!(!res!(refused(&[("sec-fetch-site", "same-origin"), ("origin", "https://evil.example")])));
    Ok(())
}

#[test]
fn test_an_origin_alone_must_be_ours_04() -> Outcome<()> {
    assert!(!res!(refused(&[("origin", SITE)])));
    assert!(!res!(refused(&[("origin", "HTTPS://Example.COM")])), "scheme and host have no case");
    assert!(!res!(refused(&[("origin", "https://example.com:443")])), "an explicit default port is ours");
    for other in [
        "https://evil.example",
        "null",
        "",
        "http://example.com",               // another scheme
        "https://example.com:8443",         // another port
        "https://www.example.com",          // another host
        "https://example.com.evil.example", // our name as a prefix
        "https://example.com@evil.example", // our name as userinfo
        "https://evil.example/https://example.com",
        "https://example.com/post",         // an origin has no path
        "file://",
    ] {
        assert!(res!(refused(&[("origin", other)])), "Origin: {:?} must be refused", other);
    }
    Ok(())
}

#[test]
fn test_an_expected_origin_is_read_as_a_browser_writes_it_05() -> Outcome<()> {
    let f = res!(fields(&[("origin", "https://example.com")]));
    assert!(!cross_site(&f, "https://example.com/"), "a trailing slash on the configured origin");
    assert!(!cross_site(&f, "https://EXAMPLE.com"), "case on the configured origin");
    let g = res!(fields(&[("origin", "http://localhost:8080")]));
    assert!(!cross_site(&g, "http://localhost:8080"));
    assert!(cross_site(&g, "http://localhost:9090"));
    Ok(())
}

#[test]
fn test_every_value_of_a_repeated_header_is_read_06() -> Outcome<()> {
    // Neither header is a singleton, so a caller may send several lines; one bad line refuses.
    assert!(res!(refused(&[("origin", SITE), ("origin", "https://evil.example")])));
    assert!(res!(refused(&[("origin", "https://evil.example"), ("origin", SITE)])));
    assert!(!res!(refused(&[("origin", SITE), ("origin", SITE)])));
    assert!(res!(refused(&[("sec-fetch-site", "same-origin"), ("sec-fetch-site", "cross-site")])));
    assert!(res!(refused(&[("sec-fetch-site", "cross-site"), ("sec-fetch-site", "same-origin")])));
    assert!(!res!(refused(&[("sec-fetch-site", "same-origin"), ("sec-fetch-site", "same-origin")])));
    Ok(())
}

#[test]
fn test_a_site_with_no_origin_of_its_own_is_checked_against_host_07() -> Outcome<()> {
    // No configured origin: the host the visitor addressed is the only account of who we are.
    for expected in ["", "not a url"] {
        let ours = res!(fields(&[("host", "Example.com"), ("origin", "https://example.com")]));
        assert!(!cross_site(&ours, expected), "expected {:?}", expected);
        let theirs = res!(fields(&[("host", "example.com"), ("origin", "https://evil.example")]));
        assert!(cross_site(&theirs, expected), "expected {:?}", expected);
        let blind = res!(fields(&[("origin", "https://example.com")]));
        assert!(cross_site(&blind, expected), "an Origin that cannot be checked is not trusted");
        let port = res!(fields(&[("host", "example.com:8443"), ("origin", "https://example.com:8443")]));
        assert!(!cross_site(&port, expected));
        let wrong = res!(fields(&[("host", "example.com"), ("origin", "https://example.com:8443")]));
        assert!(cross_site(&wrong, expected));
        // No Origin and no fetch metadata still passes, whatever we know of ourselves.
        assert!(!cross_site(&res!(fields(&[("host", "example.com")])), expected));
    }
    Ok(())
}

#[test]
fn test_wants_json_reads_every_accept_line_08() -> Outcome<()> {
    assert!(res!(fields(&[("accept", "application/json")])).wants_json());
    assert!(res!(fields(&[("accept", "text/html, application/json;q=0.9")])).wants_json());
    assert!(res!(fields(&[("accept", "Application/JSON")])).wants_json());
    assert!(res!(fields(&[("accept", "text/html"), ("accept", "application/json")])).wants_json());
    assert!(!res!(fields(&[("accept", "text/html,application/xhtml+xml,*/*;q=0.8")])).wants_json());
    assert!(!res!(fields(&[("accept", "*/*")])).wants_json());
    assert!(!res!(fields(&[])).wants_json());
    Ok(())
}

#[cfg(feature = "async")]
mod wire {
    use super::*;

    use oxedyne_fe2o3_net::http::msg::HttpMessage;

    use std::pin::Pin;

    use tokio::io::AsyncWriteExt;

    async fn parse_request(raw: &str) -> Outcome<HttpMessage> {
        let (mut near, mut far) = tokio::io::duplex(8192);
        let bytes = raw.as_bytes().to_vec();
        tokio::spawn(async move {
            let _ = far.write_all(&bytes).await;
            let _ = far.flush().await;
        });
        let read = HttpMessage::read::<1024, 1024, _>(
            Pin::new(&mut near),
            &Vec::new(),
            Some(true),
            None,
        ).await;
        match res!(read) {
            (Some(msg), _) => Ok(msg),
            (None, _) => Err(err!("The test request did not parse."; Test, Invalid, Input)),
        }
    }

    #[tokio::test]
    async fn test_a_parsed_post_is_judged_by_what_the_browser_sent_09() -> Outcome<()> {
        let head = |extra: &str| fmt!("POST /news/subscribe HTTP/1.1\r\nHost: example.com\r\n\
            Content-Length: 0\r\n{}\r\n", extra);

        let cross = res!(parse_request(&head("Sec-Fetch-Site: cross-site\r\n")).await);
        assert!(cross_site(&cross.header.fields, SITE));

        // The name's case and the number of lines are the sender's choice.
        let twice = res!(parse_request(&head(
            "ORIGIN: https://example.com\r\norigin: https://evil.example\r\n")).await);
        assert!(cross_site(&twice.header.fields, SITE));

        let same = res!(parse_request(&head("sec-fetch-site: same-origin\r\n")).await);
        assert!(!cross_site(&same.header.fields, SITE));

        let none = res!(parse_request(&head("")).await);
        assert!(!cross_site(&none.header.fields, SITE));

        let json = res!(parse_request(&head("Accept: application/json\r\n")).await);
        assert!(json.header.fields.wants_json());
        Ok(())
    }
}

// F2 B-1: a second hostname of the site, from a browser that sends no `Sec-Fetch-Site`, is the site.
#[test]
fn test_an_alias_host_without_fetch_metadata_is_let_on_09() -> Outcome<()> {
    assert!(!res!(refused(&[("host", "www.example.com"), ("origin", "https://www.example.com")])),
        "the site's own origin on its second hostname was refused");
    // Only the request's own origin is let on: a foreign page, and the other scheme, still are not.
    assert!(res!(refused(&[("host", "www.example.com"), ("origin", "https://evil.test")])));
    assert!(res!(refused(&[("host", "www.example.com"), ("origin", "http://www.example.com")])));
    assert!(res!(refused(&[("host", "example.com"), ("origin", "https://www.example.com")])));
    Ok(())
}
