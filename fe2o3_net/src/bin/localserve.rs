//! `localserve` -- a std-only static file server for a directory, from the command line.
//!
//! The tokio-free [`LocalServer`](oxedyne_fe2o3_net::http::local::LocalServer) as a standalone tool: the
//! drop-in replacement for `python3 -m http.server` a dev-preview loop reaches for. It has no
//! required-features, so it builds under `--no-default-features` -- the whole point being a static server
//! that pulls in neither an async runtime nor a TLS stack.
//!
//! ```text
//! localserve <DIR> [--port N] [--lan]
//! ```
//!
//! Loopback (`127.0.0.1`) by default; `--lan` binds `0.0.0.0` so a phone on the same network can reach
//! the preview. Port `0` (the default) asks the OS for an ephemeral one and prints what it chose.

use oxedyne_fe2o3_net::http::local::{
	Listen,
	LocalServer,
};

use oxedyne_fe2o3_core::prelude::*;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let dir = match args.first().filter(|a| !a.starts_with("--")) {
		Some(d)	=> d.clone(),
		None	=> {
			println!("usage: localserve <DIR> [--port N] [--lan]");
			return Ok(());
		},
	};
	let port = match flag_value(&args, "--port") {
		Some(s) => match s.parse::<u16>() {
			Ok(v)	=> v,
			Err(e)	=> return Err(err!("'{}' is not a valid --port value: {}.", s, e; Input, Invalid)),
		},
		None => 0,
	};
	let lan		= has_flag(&args, "--lan");
	let listen	= if lan { Listen::Any(port) } else { Listen::Loopback(port) };

	let listener	= res!(LocalServer::bind(listen));
	let bound		= res!(listener.local_addr(), IO, Network);
	let host		= if lan { "0.0.0.0" } else { "127.0.0.1" };
	println!("localserve: serving {} at http://{}:{}/", dir, host, bound.port());

	LocalServer::dir(dir).serve(listener)
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
	args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
	args.iter().any(|a| a == name)
}
