//! The `#[wasm_bindgen]` API surface exposed to the browser.
//!
//! Three probes prove the browser vertical end-to-end, no server
//! involved:
//!
//! 1. [`core_probe`] — the wasm module instantiates and a `fe2o3_core`
//!    call path (getrandom-backed RNG, the wasm clock shim, the error
//!    machinery) executes without panicking.
//! 2. [`write_file`] / [`read_file`] — a byte-exact OPFS round trip
//!    through the [`opfs`](crate::wasm::opfs) edge.
//! 3. [`llm_probe`] — the real wasm [`LlmClient`](crate::llm::LlmClient)
//!    transport issues a cross-origin `fetch` to a provider and returns
//!    the HTTP status.
//!
//! Async functions surface to JS as `Promise`s (via
//! `wasm-bindgen-futures`); [`Outcome`] errors are mapped to a rejected
//! `Promise` through [`to_js_err`](crate::wasm::to_js_err).

use crate::llm::LlmClient;
use crate::wasm::{opfs, to_js_err};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::rand::Rand;
use oxedyne_fe2o3_core::wasm::{console_log, now_ms};

use wasm_bindgen::prelude::*;


/// Default per-turn token cap for the probe client.  The value is
/// irrelevant to a dummy-key probe (the request never reaches
/// generation), but the field must be set.
const PROBE_MAX_TOKENS: u32 = 16;


/// Run a `fe2o3_core` call path in the browser and return a one-line
/// summary — the F2 proof that the gated core *runs*, not merely
/// compiles.
///
/// Exercises getrandom (via [`Rand::rand_u64`]), the wasm clock shim
/// ([`now_ms`]), the console shim ([`console_log`]) and the error
/// machinery ([`err`]).  Never panics.
#[wasm_bindgen]
pub fn core_probe() -> Result<String, JsValue> {
    // getrandom-backed RNG — panics on wasm if the `js` backend is not
    // wired, so a returned value is itself proof.
    let r = Rand::rand_u64();

    // Wall-clock via the JS `Date.now()` shim.
    let t = now_ms();

    // The error machinery must format cleanly under wasm.
    let sample: Error<ErrTag> = err!("probe sample error"; Test);
    let err_len = fmt!("{}", sample).len();

    let summary = fmt!(
        "core ok: rand_u64={:#018x}, now_ms={:.0}, err_fmt_len={}",
        r, t, err_len,
    );
    console_log(&summary);
    Ok(summary)
}

/// Write `content` (UTF-8) to `path` in OPFS, creating parents as
/// needed.  Rejects on a jail violation or an OPFS failure.
#[wasm_bindgen]
pub async fn write_file(path: String, content: String) -> Result<(), JsValue> {
    opfs::write_file(&path, content.as_bytes()).await.map_err(to_js_err)
}

/// Read `path` from OPFS and return its contents as a UTF-8 string.
#[wasm_bindgen]
pub async fn read_file(path: String) -> Result<String, JsValue> {
    match opfs::read_file(&path).await {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).to_string()),
        Err(e)    => Err(to_js_err(e)),
    }
}

/// Probe the LLM transport: issue a real cross-origin `fetch` to
/// `base_url` with `api_key` and `model`, returning the HTTP status.
///
/// A `401` with a dummy key is success — it proves `fetch` + CORS + the
/// wasm transport path work end-to-end without a valid key.
#[wasm_bindgen]
pub async fn llm_probe(
    base_url: String,
    api_key:  String,
    model:    String,
) -> Result<u32, JsValue> {
    match run_llm_probe(&base_url, &api_key, &model).await {
        Ok(status) => Ok(status as u32),
        Err(e)     => Err(to_js_err(e)),
    }
}

/// Inner probe returning an [`Outcome`], so the transport path uses the
/// error macros throughout; the `#[wasm_bindgen]` wrapper maps the result
/// to the JS boundary.
async fn run_llm_probe(base_url: &str, api_key: &str, model: &str) -> Outcome<u16> {
    let (host, port, path) = res!(parse_url(base_url));
    let client = LlmClient::new(&host, port, &path, api_key, model, PROBE_MAX_TOKENS);
    let status = res!(client.probe_status().await);
    Ok(status)
}

/// Split an `https://host[:port]/path` URL into `(host, port, path)`.
///
/// Only `https` is accepted (the browser rejects mixed-content and the
/// providers are TLS-only).  The port defaults to 443 when absent.
fn parse_url(url: &str) -> Outcome<(String, u16, String)> {
    let rest = match url.strip_prefix("https://") {
        Some(r) => r,
        None => return Err(err!(
            "llm_probe: URL '{}' must start with https://.", url;
            Invalid, Input)),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None    => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port = res!(p.parse::<u16>()
                .map_err(|e| err!(e, "llm_probe: bad port in '{}'.", url; Invalid, Input)));
            (h.to_string(), port)
        }
        None => (authority.to_string(), 443u16),
    };
    if host.is_empty() {
        return Err(err!("llm_probe: empty host in '{}'.", url; Invalid, Input));
    }
    Ok((host, port, path.to_string()))
}
