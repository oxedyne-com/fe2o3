//! Common imports for users of this crate.

pub use crate::{
	canon,
	doc::Doc,
	envelope::{
		Envelope,
		Header,
	},
	key::KeyPair,
	kinds::{
		NodeKind,
		ReservedKind,
		Schema,
	},
	limit,
	text,
	MAGIC,
	SCHEMA_APP,
	SCHEMA_CHROME,
	SCHEMA_DOC,
	VERSION_MAJOR,
};
