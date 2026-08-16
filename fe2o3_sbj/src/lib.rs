//! SBJ, Signed Binary JDAT: the file format of the oxeweb.
//!
//! An SBJ file is a signed envelope wrapping a tree of typed nodes encoded in BDAT, JDAT's binary
//! form. The hash of the tree region is the document's permanent address, and the envelope's
//! signature binds that address to its author.
//!
//! The normative description is `SPEC.md` beside this crate. Where the two disagree, this code is
//! wrong.
//!
//! The container is not specific to documents. An envelope declares the schema of its payload, so
//! an oxeweb document (`oxeweb/doc/0`) and a signed administrative command are the same artefact
//! with different payloads and different validators.

pub mod canon;
pub mod doc;
pub mod envelope;
pub mod import;
pub mod index;
pub mod key;
pub mod kinds;
pub mod prelude;
pub mod text;
pub mod validate;

/// File magic, `SBJ\0`.
pub const MAGIC: [u8; 4] = [0x53, 0x42, 0x4A, 0x00];

/// Format major version implemented here.
pub const VERSION_MAJOR: u16 = 0;

/// Fixed header length in bytes: magic, major version, envelope length.
pub const HEADER_LEN: usize = 8;

/// Schema identifier for an oxeweb document payload, which admits the kinds 1 to 13 and no others.
pub const SCHEMA_DOC: &'static str = "oxeweb/doc/0";

/// Schema identifier for the browser's own chrome, which admits the document kinds and the `edit`
/// node (§4.2).
pub const SCHEMA_CHROME: &'static str = "oxeweb/chrome/0";

/// Schema identifier for an application's tree, which admits the document kinds, the `edit` node and
/// the `surface` node (§4.2).
pub const SCHEMA_APP: &'static str = "oxeweb/app/0";

/// Limits enforced before a document is trusted. See `SPEC.md` §5.
pub mod limit {
	/// Maximum size of the tree region, in bytes.
	pub const TREE_BYTES:	usize = 4 * 1024 * 1024;
	/// Maximum size of the envelope region, in bytes.
	pub const ENVELOPE_BYTES:	usize = 4 * 1024;
	/// Maximum number of nodes in a tree.
	pub const NODES:	usize = 100_000;
	/// Maximum number of `surface` nodes in one tree (§4.2).
	///
	/// A surface is the one place in the format where something other than the author's data reaches
	/// the screen, and every one of them is a live application instance the host must lay out, budget
	/// and present. The number is revisable on evidence, like every other limit here; the commitment
	/// made now is that there is one, since a tree that may open unboundedly many instances is a tree
	/// that may exhaust the host by being opened.
	pub const SURFACES:	usize = 8;
	/// Maximum nesting depth, enforced by the decoder before verification.
	///
	/// Set so that a document at the ceiling decodes within a standard 2 MiB worker-thread stack,
	/// since a recursive decoder spends a frame per level and a stack overflow aborts the process
	/// rather than returning an error. Sixty-four levels of document nesting is already far beyond
	/// anything real, where a deeply structured document reaches perhaps twenty, and it matches the
	/// default depth limit of the BDAT decoder in `fe2o3_jdat`.
	pub const DEPTH:	usize = 64;
}
