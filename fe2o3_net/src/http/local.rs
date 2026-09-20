//! A std-only loopback static HTTP/1.1 server: no tokio, no TLS, no configuration file.
//!
//! `fe2o3_net`'s async HTTP layer (`HttpMessage`/`HttpMessageReader`) is the crate's real server, and
//! the right reach for anything a network peer touches -- but it rides on `tokio`'s full feature set and,
//! behind the `async` feature, `rustls`/`ring`/`rcgen`. A desktop reader shell that serves four files to
//! a browser tab on `127.0.0.1`, and a `typst watch`-style dev preview that serves a rasterised document
//! to a phone on the LAN, want none of that: they parse a bodiless GET, answer it, and close. Making a
//! caller link an async runtime and a TLS stack to do so was the whole reason a downstream app grew its
//! own std-only server (`fe2o3_pearlite::shell`) and a downstream script shelled out to python's
//! `http.server`. This is that server, lifted into fe2o3 and generalised, so neither has to.
//!
//! One thread per connection, GET only, `Connection: close`, `Cache-Control: no-store` (a dev preview
//! polls its own status files, and a stale answer there is a preview showing the wrong page). Two ways to
//! give it something to serve, which compose: fixed in-memory [`LocalServer::route`]s answered first, and
//! an optional [`LocalServer::dir`] root read off the disk for everything else, with the path-traversal
//! reject and content-type map the crate already carries ([`RequestPath`]).

use crate::{
	file::RequestPath,
	http::status::HttpStatus,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	borrow::Cow,
	collections::BTreeMap,
	io::{
		Read,
		Write,
	},
	net::{
		IpAddr,
		Ipv4Addr,
		SocketAddr,
		TcpListener,
		TcpStream,
	},
	path::PathBuf,
	sync::Arc,
	thread,
};

// Far past any request head this server ever answers a GET to; a request that has not ended its headers
// by here is not one of the fixed routes, nor a file under the root, asking politely.
const MAX_REQUEST_HEAD: usize = 1 << 16;

/// Which interface a [`LocalServer`] listens on.
///
/// `Loopback` binds `127.0.0.1` alone -- there is no "expose to network" knob, which is the right stance
/// for a shell whose only client is a browser tab on the same machine. `Any` binds `0.0.0.0`, for the one
/// case that wants it: a dev preview reachable from a phone on the same network.
pub enum Listen {
	Loopback(u16),
	Any(u16),
}

// One fixed in-memory route: the bytes and the exact `Content-Type` string to answer with.
struct Route {
	body:			Cow<'static, [u8]>,
	content_type:	String,
}

/// A blocking static file server for loopback or LAN use.
///
/// Fixed routes are matched first, exactly, by request path; anything not matched is looked up under the
/// directory root if one was set, and 404s otherwise. Both may be present at once.
pub struct LocalServer {
	root:	Option<PathBuf>,			// files served off the disk, `None` for a routes-only server
	routes:	BTreeMap<String, Route>,	// fixed in-memory answers, keyed by exact request path
}

impl LocalServer {

	/// A server with no directory root: only the fixed routes added with [`LocalServer::route`] are
	/// served.
	pub fn new() -> Self {
		Self {
			root:	None,
			routes:	BTreeMap::new(),
		}
	}

	/// A server that reads files under `root`. A request path is resolved beneath it (`/` maps to
	/// `index.html`), rejecting any `.`/`..` component; fixed routes still take precedence.
	pub fn dir(root: impl Into<PathBuf>) -> Self {
		Self {
			root:	Some(root.into()),
			routes:	BTreeMap::new(),
		}
	}

	/// Add a fixed in-memory route answered at exactly `path` (e.g. `/`, `/pearl.js`). A borrowed body
	/// costs nothing to hold; an owned one (a document read into memory) is moved in.
	pub fn route(mut self, path: &str, body: Cow<'static, [u8]>, content_type: &str) -> Self {
		self.routes.insert(
			path.to_string(),
			Route {
				body,
				content_type:	content_type.to_string(),
			},
		);
		self
	}

	/// Bind a listener on the chosen interface. Port `0` asks the OS for an ephemeral one, which the
	/// caller reads back with `listener.local_addr()`.
	pub fn bind(listen: Listen) -> Outcome<TcpListener> {
		let addr = match listen {
			Listen::Loopback(port)	=> SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
			Listen::Any(port)		=> SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
		};
		Ok(res!(TcpListener::bind(addr), IO, Network, Init))
	}

	/// Serve forever on `listener`, one thread per connection. Returns only on a fatal accept error; a
	/// per-connection failure is logged and the loop keeps running, since a broken connection is a client
	/// that moved on, not a reason to stop the server.
	pub fn serve(self, listener: TcpListener) -> Outcome<()> {
		let shared = Arc::new(self);
		loop {
			let (stream, _peer) = res!(listener.accept(), IO, Network);
			let server = shared.clone();
			let spawned = thread::Builder::new()
				.name("local-http-conn".to_string())
				.spawn(move || {
					if let Err(e) = server.handle(stream) {
						warn!("Local HTTP connection: {}", e);
					}
				});
			if let Err(e) = spawned {
				warn!("Local HTTP server: could not spawn a connection thread: {}", e);
			}
		}
	}

	/// Reads one HTTP/1.1 request head from `stream` and answers it. Every route here is a bodiless GET,
	/// so reading stops at the header block's terminating blank line -- there is never a body to read.
	fn handle(&self, mut stream: TcpStream) -> Outcome<()> {
		let mut req		= Vec::new();
		let mut chunk	= [0u8; 4096];
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
				return self.respond(&mut stream, HttpStatus::BadRequest, "text/plain",
					b"request head too large");
			}
		}
		if req.is_empty() {
			return Ok(());	// nothing arrived; the peer closed an idle connection
		}

		let head		= String::from_utf8_lossy(&req);
		let line		= head.lines().next().unwrap_or("");
		let mut parts	= line.split_whitespace();
		let method		= parts.next().unwrap_or("");
		let target		= parts.next().unwrap_or("/");
		let path		= target.split('?').next().unwrap_or("/");

		if method != "GET" {
			return self.respond(&mut stream, HttpStatus::MethodNotAllowed, "text/plain",
				b"only GET is served here");
		}

		// A fixed route wins over the directory root, so a caller can shadow or supply files the root does
		// not hold.
		if let Some(route) = self.routes.get(path) {
			let content_type = route.content_type.clone();
			return self.respond(&mut stream, HttpStatus::OK, &content_type, &route.body);
		}

		match &self.root {
			Some(root) => {
				let root_str	= root.to_string_lossy().into_owned();
				let index		= "index.html".to_string();
				// A `.`/`..` component, or a path that ends in `/`, is refused rather than served: the
				// reject is the crate's own, shared with the async server.
				let resolved = match RequestPath::new(path).validate(&root_str, &index) {
					Ok(p)	=> p,
					Err(_)	=> return self.respond(&mut stream, HttpStatus::Forbidden, "text/plain",
						b"forbidden"),
				};
				match std::fs::read(&resolved) {
					Ok(body) => {
						let content_type = fmt!("{}", RequestPath::content_type(&resolved));
						self.respond(&mut stream, HttpStatus::OK, &content_type, &body)
					},
					// A missing file, or a path that named a directory, is simply not here.
					Err(_) => self.respond(&mut stream, HttpStatus::NotFound, "text/plain", b"not found"),
				}
			},
			None => self.respond(&mut stream, HttpStatus::NotFound, "text/plain", b"not found"),
		}
	}

	fn respond(
		&self,
		stream:			&mut TcpStream,
		status:			HttpStatus,
		content_type:	&str,
		body:			&[u8],
	)
		-> Outcome<()>
	{
		// `Cache-Control: no-store` because the motivating callers poll: a dev preview re-fetches its own
		// status files, and a stale answer there is a preview showing the wrong page.
		let head = fmt!(
			"HTTP/1.1 {} {}\r\n\
			Content-Type: {}\r\n\
			Content-Length: {}\r\n\
			Cache-Control: no-store\r\n\
			Connection: close\r\n\r\n",
			status, status.desc(), content_type, body.len());
		res!(stream.write_all(head.as_bytes()), IO, Network, Write);
		res!(stream.write_all(body), IO, Network, Write);
		Ok(())
	}
}

impl Default for LocalServer {
	fn default() -> Self {
		Self::new()
	}
}

/// Does `buf` carry a complete HTTP header block (ending `\r\n\r\n`)?
fn header_block_ends(buf: &[u8]) -> bool {
	buf.windows(4).any(|w| w == b"\r\n\r\n")
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
			.expect("connecting to the test server");
		let req = fmt!("GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", path);
		stream.write_all(req.as_bytes()).expect("writing the test request");
		let mut buf = Vec::new();
		stream.read_to_end(&mut buf).expect("reading the test response");
		String::from_utf8_lossy(&buf).into_owned()
	}

	/// A raw request with an arbitrary method, for the GET-only check.
	fn request(port: u16, method: &str, path: &str) -> String {
		let mut stream = TcpStream::connect(("127.0.0.1", port))
			.expect("connecting to the test server");
		let req = fmt!("{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", method, path);
		stream.write_all(req.as_bytes()).expect("writing the test request");
		let mut buf = Vec::new();
		stream.read_to_end(&mut buf).expect("reading the test response");
		String::from_utf8_lossy(&buf).into_owned()
	}

	fn head_of(s: &str) -> &str {
		s.lines().next().unwrap_or("")
	}

	// The whole gate for the fixed-route mode (the shell case ported from `fe2o3_pearlite::shell`): the
	// server starts on loopback, serves an index and a script byte for byte, serves a caller's document
	// at its own path, 404s an unknown path, refuses a non-GET, and marks every answer `no-store`.
	#[test]
	fn test_fixed_routes_are_served_and_others_refused_00() -> Outcome<()> {
		let index	= "<!doctype html><title>Fixture</title><script src=pearl.js></script>";
		let script	= "// pearl reader fixture\n";
		let doc		= "pearl 1\nnot a real document, just fixture bytes\n";
		let server = LocalServer::new()
			.route("/",				Cow::Borrowed(index.as_bytes()),	"text/html; charset=utf-8")
			.route("/index.html",	Cow::Borrowed(index.as_bytes()),	"text/html; charset=utf-8")
			.route("/pearl.js",		Cow::Borrowed(script.as_bytes()),	"text/javascript; charset=utf-8")
			.route("/opened.prl",	Cow::Owned(doc.as_bytes().to_vec()),	"text/plain; charset=utf-8");
		let listener	= res!(LocalServer::bind(Listen::Loopback(0)));
		let port		= res!(listener.local_addr(), IO, Network).port();
		thread::spawn(move || {
			let _ = server.serve(listener);
		});
		// The listener is already bound before the thread starts; the sleep is a generous margin for the
		// accept loop to be running, not something the assertions depend on for correctness.
		thread::sleep(Duration::from_millis(50));

		let got_index = get(port, "/");
		assert!(got_index.starts_with("HTTP/1.1 200"), "index did not answer 200: {}", head_of(&got_index));
		assert!(got_index.contains("Fixture"), "index body was not the fixture page");
		assert!(got_index.to_lowercase().contains("cache-control: no-store"),
			"index answer was not marked no-store: {}", got_index);

		let got_js = get(port, "/pearl.js");
		assert!(got_js.starts_with("HTTP/1.1 200"), "pearl.js did not answer 200: {}", head_of(&got_js));
		assert!(got_js.to_lowercase().contains("content-type: text/javascript"),
			"pearl.js was not served as javascript: {}", got_js);

		let got_doc = get(port, "/opened.prl");
		assert!(got_doc.starts_with("HTTP/1.1 200"), "the document did not answer 200");
		assert!(got_doc.ends_with(doc), "the served document bytes did not match what was given");

		let missing = get(port, "/nope");
		assert!(missing.starts_with("HTTP/1.1 404"), "an unknown path should 404: {}", head_of(&missing));

		let posted = request(port, "POST", "/");
		assert!(posted.starts_with("HTTP/1.1 405"), "a non-GET should 405: {}", head_of(&posted));
		Ok(())
	}

	// The directory-root mode (the dev-preview case): files under the root are read off the disk with the
	// right content type, `/` maps to `index.html`, and a `..` traversal is refused rather than served.
	#[test]
	fn test_a_directory_root_serves_files_and_refuses_traversal_01() -> Outcome<()> {
		let dir = std::env::temp_dir()
			.join(fmt!("fe2o3-local-{}-{}", std::process::id(), "dir"));
		res!(std::fs::create_dir_all(&dir), IO, File);
		res!(std::fs::write(dir.join("index.html"), b"<!doctype html><title>Root</title>"), IO, File);
		res!(std::fs::write(dir.join("pages.json"), br#"{"stamp":"x","pages":2}"#), IO, File);

		let server		= LocalServer::dir(dir.clone());
		let listener	= res!(LocalServer::bind(Listen::Loopback(0)));
		let port		= res!(listener.local_addr(), IO, Network).port();
		thread::spawn(move || {
			let _ = server.serve(listener);
		});
		thread::sleep(Duration::from_millis(50));

		let root = get(port, "/");
		assert!(root.starts_with("HTTP/1.1 200"), "`/` did not map to index.html: {}", head_of(&root));
		assert!(root.contains("Root"), "`/` did not serve index.html's bytes");

		let json = get(port, "/pages.json");
		assert!(json.starts_with("HTTP/1.1 200"), "pages.json did not answer 200");
		assert!(json.to_lowercase().contains("content-type: application/json"),
			"pages.json was not served as JSON: {}", json);

		let escape = get(port, "/../Cargo.toml");
		assert!(!escape.starts_with("HTTP/1.1 200"),
			"a `..` traversal was served: {}", head_of(&escape));

		let _ = std::fs::remove_dir_all(&dir);
		Ok(())
	}
}
