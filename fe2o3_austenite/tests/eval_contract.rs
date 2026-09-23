//! U0's contract checks: the shared tables the parallel units build on are self-consistent.

use oxedyne_fe2o3_austenite::eval::args::Args;
use oxedyne_fe2o3_austenite::eval::content::{
	construct,
	Content,
	ElemKind,
	FieldId,
};
use oxedyne_fe2o3_austenite::eval::lib::library;
use oxedyne_fe2o3_austenite::eval::locate::Locator;
use oxedyne_fe2o3_austenite::eval::value::{
	Dict,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::ledger::{
	AnchorId,
	AnchorKind,
};
use oxedyne_fe2o3_austenite::syntax::{
	FileId,
	Span,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::HashSet;
use std::path::PathBuf;

#[test]
fn element_paths_are_unique_and_scoped() {
	let mut seen = HashSet::new();
	for k in ElemKind::ALL {
		assert!(seen.insert(k.path()), "duplicate element path {}", k.path());
	}
	assert_eq!(ElemKind::List.scoped("item"), Some(ElemKind::ListItem));
	assert_eq!(ElemKind::Table.scoped("cell"), Some(ElemKind::TableCell));
	assert_eq!(ElemKind::Heading.scoped("item"), None);
	assert_eq!(ElemKind::ListItem.name(), "item");
}

// What `#context query(<elem>)` does under typst 0.15.1 for the elements whose flag changed from U0's
// first table: true where it answers, false where it says "<elem> is not locatable".
const QUERYABLE_IN_TYPST: [(ElemKind, bool); 16] = [
	(ElemKind::Par,				true),
	(ElemKind::ParLine,			true),
	(ElemKind::Strong,			true),
	(ElemKind::Raw,				true),
	(ElemKind::List,			true),
	(ElemKind::ListItem,		false),
	(ElemKind::Link,			true),
	(ElemKind::FigureCaption,	true),
	(ElemKind::Underline,		true),
	(ElemKind::Table,			true),
	(ElemKind::TableCell,		false),
	(ElemKind::Image,			true),
	(ElemKind::Place,			false),
	(ElemKind::Divider,			false),
	(ElemKind::Text,			false),
	(ElemKind::Block,			false),
];

#[test]
fn queryable_elements_match_typst_and_are_located() {
	for (kind, want) in QUERYABLE_IN_TYPST {
		assert_eq!(kind.queryable(), want, "query({}) under typst 0.15.1", kind.path());
	}
	for k in ElemKind::ALL {
		assert!(!k.queryable() || k.locatable(), "{} is queryable but never located", k.path());
	}
	// Located for float bookkeeping, as Typst's `PlaceElem` is, though no query may name it.
	assert!(ElemKind::Place.locatable());
}

#[test]
fn library_defines_global_elements_only() {
	let lib = library();
	assert!(matches!(lib.get("heading"), Some(Value::Func(_))));
	assert!(lib.get("item").is_none());
	assert!(lib.get("frac").is_none());
}

#[test]
fn text_constructs_through_the_generic_schema_walk() -> Outcome<()> {
	let mut engine = Engine::new(World::new(PathBuf::from("/")));
	let mut args = Args::new(Span::detached());
	args.push(Span::detached(), Value::str("hello"));
	let c = res!(construct(&mut engine, ElemKind::Text, &mut args));
	assert_eq!(c.plain_text(), "hello");
	assert!(matches!(c.get(FieldId(0)), Some(Value::Str(_))));
	let seq = Content::sequence(vec![Content::text("a"), Content::marker(ElemKind::Space, Span::detached()), c]);
	assert_eq!(seq.plain_text(), "a hello");
	Ok(())
}

#[test]
fn dict_keeps_insertion_order() {
	let mut d = Dict::new();
	d.insert("b", Value::Int(1));
	d.insert("a", Value::Int(2));
	d.insert("b", Value::Int(3));
	assert_eq!(d.keys().collect::<Vec<_>>(), vec!["b", "a"]);
	assert!(matches!(d.remove("b"), Some(Value::Int(3))));
	assert!(matches!(d.get("a"), Some(Value::Int(2))));
}

#[test]
fn locations_are_stable_and_distinct() {
	let span = Span::new(FileId(0), 10, 20);
	let mut a = Locator::default();
	let mut b = Locator::default();
	let a1 = a.locate(ElemKind::Heading, span);
	let a2 = a.locate(ElemKind::Heading, span);
	assert_ne!(a1, a2);
	assert_eq!(a1, b.locate(ElemKind::Heading, span));
}

#[test]
fn location_anchor_round_trips_the_ledger_encoding() -> Outcome<()> {
	let id = AnchorId::new(AnchorKind::Location, "00000000deadbeef");
	let back = res!(AnchorId::from_dat(res!(id.to_dat())));
	assert_eq!(back, id);
	assert_eq!(back.kind.name(), "location");
	Ok(())
}
