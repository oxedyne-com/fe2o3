//! Phase 2: a loopback browser-shell that serves the existing `fe2o3_austenite` `web/pearl-reader/`
//! assets, plus the caller's own `.prl`, and opens the OS default browser at it.
//!
//! The server itself is `fe2o3_net`'s std-only [`LocalServer`](oxedyne_fe2o3_net::http::local::LocalServer)
//! -- one thread per connection, GET only, no TLS, no async runtime, bound to `127.0.0.1` alone. This
//! module is now just the reader's fixed routes: the four embedded assets and the one document, wired up
//! and handed to that server. The std-only server was born here (embedding tokio and rustls to serve four
//! files on loopback made no sense); it has since moved into `fe2o3_net` so the dev-preview script and any
//! other caller can share it rather than each growing its own.

use oxedyne_fe2o3_net::http::local::{
	Listen,
	LocalServer,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	borrow::Cow,
	net::TcpListener,
};

// Embedded verbatim from `fe2o3_austenite/web/pearl-reader/`, so the native shell serves exactly the one
// browser-side reader the web build does -- no second implementation to keep in step with the first.
const INDEX_HTML:		&str = include_str!("../../fe2o3_austenite/web/pearl-reader/index.html");
const JDAT_JS:			&str = include_str!("../../fe2o3_austenite/web/pearl-reader/jdat.js");
const PEARL_JS:			&str = include_str!("../../fe2o3_austenite/web/pearl-reader/pearl.js");
const AUTHORING_JS:	&str = include_str!("../../fe2o3_austenite/web/pearl-reader/authoring.js");

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
		LocalServer::bind(Listen::Loopback(port))
	}

	/// Serves forever on `listener`, delegating to the shared std-only server. The reader's four assets
	/// and the one document are the fixed routes; there is no directory root, so nothing else is served.
	pub fn serve(self, listener: TcpListener) -> Outcome<()> {
		LocalServer::new()
			.route("/",				Cow::Borrowed(INDEX_HTML.as_bytes()),	"text/html; charset=utf-8")
			.route("/index.html",	Cow::Borrowed(INDEX_HTML.as_bytes()),	"text/html; charset=utf-8")
			.route("/jdat.js",		Cow::Borrowed(JDAT_JS.as_bytes()),		"text/javascript; charset=utf-8")
			.route("/pearl.js",		Cow::Borrowed(PEARL_JS.as_bytes()),		"text/javascript; charset=utf-8")
			.route("/authoring.js",	Cow::Borrowed(AUTHORING_JS.as_bytes()),	"text/javascript; charset=utf-8")
			.route(
				&fmt!("/{}", self.doc_name),
				Cow::Owned(self.doc_bytes),
				"text/plain; charset=utf-8",
			)
			.serve(listener)
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

	use std::{
		io::{
			Read,
			Write,
		},
		net::TcpStream,
		thread,
		time::Duration,
	};

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
