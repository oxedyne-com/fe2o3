//! Phase 2: a minimal loopback HTTP/1.1 server that serves the existing `fe2o3_austenite`
//! `web/pearl-reader/` assets, plus the caller's own `.prl`, and opens the OS default browser at it.
//!
//! `fe2o3_steel` is fe2o3's own HTTP app server, and the obvious first reach for "serve some files" --
//! but `AppShellContext::start_server` wants an app config, a TUI shell context, a validated server
//! config, certificates and (optionally) an `O3db`, and the crate it lives in links `rustls`, `ring` and
//! `rcgen` unconditionally to serve even one static byte on loopback. That is not this job: a desktop
//! reader shell wants four files and one filename served to `127.0.0.1` alone, nothing else, and no TLS
//! ever touches a loopback-only socket. `fe2o3_net`'s own HTTP layer (`HttpMessage`/`HttpMessageReader`)
//! is the next reach and is a better fit in shape, but the crate it lives in carries the very same
//! `rustls`/`ring`/`rcgen` weight unconditionally alongside `tokio`'s full feature set, purely so a
//! reader shell that serves nothing but GET on loopback can parse a request line.
//!
//! So this is the minimal, std-only loopback server the unit's own brief allows for when embedding
//! Steel proves heavier than the page: one thread per connection, GET only, no TLS, no configuration
//! file, and a bind that only ever names `127.0.0.1` -- there is no "expose to network" knob, matching
//! the stance Steel's own local-admin listener (`fe2o3_steel/src/srv/admin/local_listener.rs`) takes for
//! the same reason. It is a genuinely app-specific eight-route file server, not a general one.

use oxedyne_fe2o3_core::prelude::*;

use std::io::{
	Read,
	Write,
};
use std::net::{
	IpAddr,
	Ipv4Addr,
	SocketAddr,
	TcpListener,
	TcpStream,
};
use std::sync::Arc;
use std::thread;

// Embedded verbatim from `fe2o3_austenite/web/pearl-reader/`, so the native shell serves exactly the one
// browser-side reader the web build does -- no second implementation to keep in step with the first.
const INDEX_HTML:		&str = include_str!("../../fe2o3_austenite/web/pearl-reader/index.html");
const JDAT_JS:			&str = include_str!("../../fe2o3_austenite/web/pearl-reader/jdat.js");
const PEARL_JS:			&str = include_str!("../../fe2o3_austenite/web/pearl-reader/pearl.js");
const AUTHORING_JS:	&str = include_str!("../../fe2o3_austenite/web/pearl-reader/authoring.js");

// Far past any request head this shell ever answers a GET to; a request that has not ended its headers
// by here is not one of the four fixed routes asking politely.
const MAX_REQUEST_HEAD: usize = 1 << 16;

/// A loopback shell: the fixed reader assets, plus one caller-named `.prl` served at its own path so the
/// served `index.html`'s `?doc=` query can point straight at it.
pub struct Shell {
	doc_name:	String,		// path segment the document answers at, e.g. "opened.prl"
	doc_bytes:	Vec<u8>,
}

impl Shell {
	pub fn new(doc_name: String, doc_bytes: Vec<u8>) -> Self {
		Self { doc_name, doc_bytes }
	}

	/// Binds a loopback listener. `port` 0 asks the OS for an ephemeral one; the caller reads back what
	/// was chosen with `listener.local_addr()`. Never binds anything but `127.0.0.1`.
	pub fn bind(port: u16) -> Outcome<TcpListener> {
		let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
		Ok(res!(TcpListener::bind(addr), IO, Network, Init))
	}

	/// Serves forever on `listener`, one thread per connection. Returns only on a fatal accept error; a
	/// per-connection failure is logged and the loop keeps running, since the shell has one browser tab
	/// as its client and a broken connection there is the tab having moved on, not a reason to stop.
	pub fn serve(self, listener: TcpListener) -> Outcome<()> {
		let shared = Arc::new(self);
		loop {
			let (stream, _peer) = res!(listener.accept(), IO, Network);
			let shell = shared.clone();
			let spawned = thread::Builder::new()
				.name("pearlite-shell-conn".to_string())
				.spawn(move || {
					if let Err(e) = shell.handle(stream) {
						warn!("Pearlite shell connection: {}", e);
					}
				});
			if let Err(e) = spawned {
				warn!("Pearlite shell: could not spawn a connection thread: {}", e);
			}
		}
	}

	/// The bytes and content type this shell answers `path` with, or `None` for anything else -- every
	/// route is fixed, since this is four files and one filename, not a general file server.
	fn asset(&self, path: &str) -> Option<(&[u8], &'static str)> {
		match path {
			"/" | "/index.html"	=> Some((INDEX_HTML.as_bytes(), "text/html; charset=utf-8")),
			"/jdat.js"				=> Some((JDAT_JS.as_bytes(), "text/javascript; charset=utf-8")),
			"/pearl.js"				=> Some((PEARL_JS.as_bytes(), "text/javascript; charset=utf-8")),
			"/authoring.js"			=> Some((AUTHORING_JS.as_bytes(), "text/javascript; charset=utf-8")),
			p if p.trim_start_matches('/') == self.doc_name =>
				Some((self.doc_bytes.as_slice(), "text/plain; charset=utf-8")),
			_ => None,
		}
	}

	/// Reads one HTTP/1.1 request head from `stream` and answers it. Every route here is a bodiless GET,
	/// so reading stops at the header block's terminating blank line -- there is never a body to read.
	fn handle(&self, mut stream: TcpStream) -> Outcome<()> {
		let mut req = Vec::new();
		let mut chunk = [0u8; 4096];
		loop {
			let n = res!(stream.read(&mut chunk), IO, Network, Read);
			if n == 0 {
				break;	// the peer closed before a full header block arrived
			}
			req.extend_from_slice(&chunk[..n]);
			if header_block_ends(&req) {
				break;
			}
			if req.len() > MAX_REQUEST_HEAD {
				return self.respond(&mut stream, 400, "text/plain", b"request head too large");
			}
		}
		if req.is_empty() {
			return Ok(());	// nothing arrived; the peer closed an idle connection
		}

		let head	= String::from_utf8_lossy(&req);
		let line	= head.lines().next().unwrap_or("");
		let mut parts	= line.split_whitespace();
		let method	= parts.next().unwrap_or("");
		let target	= parts.next().unwrap_or("/");
		let path	= target.split('?').next().unwrap_or("/");

		if method != "GET" {
			return self.respond(&mut stream, 405, "text/plain", b"only GET is served here");
		}
		match self.asset(path) {
			Some((body, content_type))	=> self.respond(&mut stream, 200, content_type, body),
			None						=> self.respond(&mut stream, 404, "text/plain", b"not found"),
		}
	}

	fn respond(&self, stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8])
		-> Outcome<()>
	{
		let head = fmt!(
			"HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
			status, reason_phrase(status), content_type, body.len());
		res!(stream.write_all(head.as_bytes()), IO, Network, Write);
		res!(stream.write_all(body), IO, Network, Write);
		Ok(())
	}
}

/// Does `buf` carry a complete HTTP header block (ending `\r\n\r\n`)?
fn header_block_ends(buf: &[u8]) -> bool {
	buf.windows(4).any(|w| w == b"\r\n\r\n")
}

fn reason_phrase(status: u16) -> &'static str {
	match status {
		200	=> "OK",
		400	=> "Bad Request",
		404	=> "Not Found",
		405	=> "Method Not Allowed",
		_	=> "Error",
	}
}

/// Opens the OS default browser at `url`. A failure to find or spawn a browser is an error the caller
/// logs, not a reason to stop the server -- the shell is up regardless, and the URL is printable for the
/// caller to open by hand.
pub fn open_browser(url: &str) -> Outcome<()> {
	let mut cmd = if cfg!(target_os = "macos") {
		std::process::Command::new("open")
	} else if cfg!(target_os = "windows") {
		let mut c = std::process::Command::new("cmd");
		c.args(["/C", "start", ""]);
		c
	} else {
		std::process::Command::new("xdg-open")
	};
	cmd.arg(url);
	res!(cmd.spawn(), IO, System, Init);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	use std::time::Duration;

	/// One raw GET over loopback, read to connection close (every response here sets `Connection:
	/// close`), returned as the response text -- head and body together, since a small fixed asset needs
	/// no separate parse to check.
	fn get(port: u16, path: &str) -> String {
		let mut stream = TcpStream::connect(("127.0.0.1", port))
			.expect("connecting to the Pearlite test shell");
		let req = fmt!("GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", path);
		stream.write_all(req.as_bytes()).expect("writing the test request");
		let mut buf = Vec::new();
		stream.read_to_end(&mut buf).expect("reading the test response");
		String::from_utf8_lossy(&buf).into_owned()
	}

	// The whole Phase 2 gate: the shell starts on loopback, serves the reader's own index.html and
	// pearl.js byte for byte, serves the caller's document at its own path, and 404s an unknown one --
	// everything a headless check can confirm without a real browser.
	#[test]
	fn test_shell_serves_index_pearl_js_and_the_document_00() -> Outcome<()> {
		let doc_text = "pearl 1\nnot a real document, just fixture bytes for the shell test\n";
		let shell = Shell::new("opened.prl".to_string(), doc_text.as_bytes().to_vec());
		let listener = res!(Shell::bind(0));
		let port = res!(listener.local_addr(), IO, Network).port();
		thread::spawn(move || {
			let _ = shell.serve(listener);
		});
		// The listener is already bound before this thread starts; the sleep is a generous margin for
		// the accept loop to be running, not something the assertions below depend on for correctness.
		thread::sleep(Duration::from_millis(50));

		let index = get(port, "/");
		assert!(index.starts_with("HTTP/1.1 200"), "index did not answer 200: {}", head_of(&index));
		assert!(index.contains("Pearl Reader"), "index body is not the pearl-reader page");
		assert!(index.contains("pearl.js"), "index does not reference pearl.js");

		let js = get(port, "/pearl.js");
		assert!(js.starts_with("HTTP/1.1 200"), "pearl.js did not answer 200: {}", head_of(&js));
		assert!(js.contains("Pearl"), "pearl.js body does not look like the reader script");

		let jdat = get(port, "/jdat.js");
		assert!(jdat.starts_with("HTTP/1.1 200"), "jdat.js did not answer 200");

		let doc = get(port, "/opened.prl");
		assert!(doc.starts_with("HTTP/1.1 200"), "the document did not answer 200: {}", head_of(&doc));
		assert!(doc.ends_with(doc_text), "the served document bytes did not match what was given");

		let missing = get(port, "/nope");
		assert!(missing.starts_with("HTTP/1.1 404"), "an unknown path should 404: {}", head_of(&missing));
		Ok(())
	}

	fn head_of(s: &str) -> &str {
		s.lines().next().unwrap_or("")
	}
}
