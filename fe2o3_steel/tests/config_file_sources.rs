//! `ApiRoute::config_file_sources` and `WebhookRoute::config_file_sources` name every file a
//! resolve would read, after `{env:}` resolution, and read none of them.
//!
//! A caller that must vet where a config's secrets come from (a test network refusing to start
//! on a production key, say) has to see the paths before the read, because the read is the
//! thing it is guarding against.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_steel::srv::cfg::{
    ApiRoute,
    WebhookRoute,
};

use std::path::{
    Path,
    PathBuf,
};

fn api_route(headers: Vec<(&str, &str)>, config: Vec<(&str, &str)>) -> ApiRoute {
    ApiRoute {
        path:           fmt!("/api/x"),
        upstream_host:  None,
        upstream_port:  None,
        upstream_path:  None,
        upstream_tls:   true,
        headers:        headers.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        handler:        Some(fmt!("x")),
        config:         config.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    }
}

#[test]
fn test_config_file_sources_lists_both_forms_after_env_and_reads_none() -> Outcome<()> {
    // Names unique to this test, so a parallel test cannot see or change them.
    std::env::set_var("FE2O3_CFS_MODE", "sandbox");
    std::env::remove_var("FE2O3_CFS_UNSET");
    let root = Path::new("/nonexistent/fe2o3_cfs_root");
    let route = api_route(
        vec![("Authorization", "Bearer {file:./keys/hdr}")],
        vec![
            ("sk",     "{file:./keys/stripe/{env:FE2O3_CFS_MODE:live}/sk}"),
            ("voice",  "{file?:{env:FE2O3_CFS_UNSET:/abs/oregami_key}}"),
            ("plain",  "no placeholder here"),
            ("two",    "{file:a}-{file?:b}"),
        ],
    );

    // Every one of these paths is absent, so a resolve would fail on the first `{file:}`.
    let mut copy = route.clone();
    assert!(copy.resolve_config(root).is_err(), "the fixture must be unreadable to prove no read");

    let got = res!(route.config_file_sources(root));
    let want: Vec<(String, PathBuf)> = vec![
        (fmt!("Authorization"), root.join("./keys/hdr")),
        (fmt!("sk"),            root.join("./keys/stripe/sandbox/sk")),
        (fmt!("voice"),         PathBuf::from("/abs/oregami_key")),  // An absolute path replaces the root.
        (fmt!("two"),           root.join("a")),
        (fmt!("two"),           root.join("b")),
    ];
    assert_eq!(got, want);
    Ok(())
}

#[test]
fn test_config_file_sources_on_a_webhook_route() -> Outcome<()> {
    std::env::set_var("FE2O3_CFS_WH_MODE", "sandbox");
    let root = Path::new("/nonexistent/fe2o3_cfs_wh");
    let route = WebhookRoute {
        path:           fmt!("/webhook/x"),
        handler:        Some(fmt!("x")),
        upstream_host:  None,
        upstream_port:  None,
        upstream_path:  None,
        upstream_tls:   true,
        config:         vec![
            (fmt!("whsec"), "{file:./keys/stripe/{env:FE2O3_CFS_WH_MODE:live}/whsec}".to_string()),
            (fmt!("tol"),   fmt!("300")),
        ],
    };
    let got = res!(route.config_file_sources(root));
    assert_eq!(got, vec![(fmt!("whsec"), root.join("./keys/stripe/sandbox/whsec"))]);
    Ok(())
}

#[test]
fn test_config_file_sources_refuses_an_unclosed_placeholder() {
    let route = api_route(vec![], vec![("k", "{file:./never")]);
    let e = route.config_file_sources(Path::new("/nonexistent"));
    assert!(e.is_err(), "an unclosed placeholder is malformed config, not an empty list");
}
