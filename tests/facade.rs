//! Proves the facade's `all` feature exposes every member crate.
//!
//! The `all` feature once listed `dep:` entries, which link the member crates
//! but suppress the implicit features the re-exports in `lib.rs` are gated on,
//! so `--features all` compiled everything and exposed nothing. These `use`
//! statements are compile-time proof against that regression: if a re-export
//! goes dark, this test target fails to build.
#![cfg(feature = "all")]

#[allow(unused_imports)]
use oxedyne_fe2o3::{
	bot,
	core,
	crypto,
	data,
	datime,
	file,
	geom,
	graphics,
	hash,
	iop_crypto,
	iop_db,
	iop_hash,
	jdat,
	mail,
	namex,
	net,
	num,
	o3db,
	ore,
	shield,
	stds,
	steel,
	syntax,
	sys,
	test,
	text,
	tui,
	units,
};

/// The imports above are the test; this arms the target.
#[test]
fn the_facade_exposes_what_all_promises() {
	// Touch one item through the facade so the re-export chain is exercised
	// beyond name resolution.
	let id = ore::id::OpId::new(ore::id::ReplicaId::new(1), 1);
	assert_eq!(id.replica.inner(), 1);
}
