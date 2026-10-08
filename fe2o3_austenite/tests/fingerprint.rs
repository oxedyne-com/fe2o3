//! U5: content fingerprints stand for what the content is, never for where it was written. The same text at a
//! shifted offset fingerprints alike; a change to any field, label, kind, guard, style, recipe or closure
//! gives a different one; and a fingerprint read before a change is never the one read after it.

use oxedyne_fe2o3_austenite::eval::content::{
	Content,
	Elem,
	ElemKind,
	FieldId,
};
use oxedyne_fe2o3_austenite::eval::locate::{
	Location,
	Place,
};
use oxedyne_fe2o3_austenite::eval::styles::RecipeIndex;
use oxedyne_fe2o3_austenite::eval::value::{
	Label,
	Value,
};
use oxedyne_fe2o3_austenite::eval::{
	eval_source,
	Engine,
	World,
};
use oxedyne_fe2o3_austenite::syntax::{
	FileId,
	Span,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;

use std::path::PathBuf;
use std::sync::Arc;

// What a source evaluates to, fingerprinted.
fn fp(src: &str) -> Outcome<Fingerprint> {
	let root = PathBuf::from("/");
	let mut world = World::new(root.clone());
	let id = res!(world.add_source(root.join("__fp.typ"), src.to_string()));
	let mut engine = Engine::new(world);
	let module = res!(eval_source(&mut engine, id));
	Ok(module.content.fingerprint())
}

// A metadata element, which holds one value in field 0.
fn meta(v: Value, span: Span) -> Elem {
	Elem {
		kind:		ElemKind::Metadata,
		fields:		vec![(FieldId(0), v)],
		label:		None,
		location:	None,
		span,
		guards:		Vec::new(),
		prepared:	false,
		place:		None,
	}
}

fn elem(e: Elem) -> Content { Content::Elem(Arc::new(e)) }

// Two sources of the same shape, the second written at other offsets.
const SHIFTED: &[(&str, &str, &str)] = &[
	("a closure numbering",
		"#let a = 1\n#heading(numbering: n => [Part #n])[T]",
		"#let aaaaaaaaaaaaaaaa = 12345\n#heading(numbering: n => [Part #n])[T]"),
	("a closure that captures",
		"#let k = 5\n#let a = 1\n#heading(numbering: n => [#k #n])[T]",
		"#let k = 5\n#let aaaaaaaaaaaaaaaa = 12345\n#heading(numbering: n => [#k #n])[T]"),
	("a show rule with a closure",
		"#let a = 1\n#show heading: it => [X #it.body]\n#heading[T]",
		"#let aaaaaaaaaaaaaaaa = 12345\n#show heading: it => [X #it.body]\n#heading[T]"),
	("a set rule",
		"#let a = 1\n#set text(size: 10pt)\nHello",
		"#let aaaaaaaaaaaaaaaa = 12345\n#set text(size: 10pt)\nHello"),
	("a label",
		"#let a = 1\n#heading[T] <lab>",
		"#let aaaaaaaaaaaaaaaa = 12345\n#heading[T] <lab>"),
	("a selector with fields",
		"#let a = 1\n#show heading.where(level: 1): it => it\n#heading[T]",
		"#let aaaaaaaaaaaaaaaa = 12345\n#show heading.where(level: 1): it => it\n#heading[T]"),
];

// Two sources that differ in one thing, which the fingerprint must see.
const DIFFERENT: &[(&str, &str, &str)] = &[
	("a field value",					"#heading[A]",									"#heading[B]"),
	("a level",							"#heading(level: 1)[A]",						"#heading(level: 2)[A]"),
	("an extra field",					"#heading[A]",									"#heading(outlined: false)[A]"),
	("a field id",						"#heading(outlined: false)[A]",					"#heading(bookmarked: false)[A]"),
	("a label",							"#heading[A] <a>",								"#heading[A] <b>"),
	("a missing label",					"#heading[A]",									"#heading[A] <a>"),
	("a kind",							"#heading[A]",									"#strong[A]"),
	("a style property value",			"#set text(size: 10pt)\nA",						"#set text(size: 11pt)\nA"),
	("a style property",				"#set text(size: 10pt)\nA",						"#set text(weight: 700)\nA"),
	("a recipe selector",				"#show heading: it => it\nA",					"#show strong: it => it\nA"),
	("a recipe selector field",			"#show heading.where(level: 1): it => it\nA",	"#show heading.where(level: 2): it => it\nA"),
	("a recipe transform function",		"#show heading: it => [X]\nA",					"#show heading: it => [Y]\nA"),
	("a recipe transform content",		"#show heading: [X]\nA",						"#show heading: [Y]\nA"),
	("a closure body",					"#heading(numbering: n => [A #n])[T]",			"#heading(numbering: n => [B #n])[T]"),
	("a closure capture",				"#let c = 1\n#heading(numbering: n => [#c #n])[T]",
										"#let c = 2\n#heading(numbering: n => [#c #n])[T]"),
	("a closure parameter default",		"#heading(numbering: (n, d: 1) => [#n #d])[T]",	"#heading(numbering: (n, d: 2) => [#n #d])[T]"),
	("a sequence order",				"#heading[A] #strong[B]",						"#strong[B] #heading[A]"),
	("a sequence length",				"A B",											"A B C"),
	("a nested field value",			"#strong[#emph[A]]",							"#strong[#emph[B]]"),
	("an array value in a field",		"#metadata((1, 2, 3))",							"#metadata((1, 2, 4))"),
	("a dictionary key in a field",		"#metadata((a: 1))",							"#metadata((b: 1))"),
	("a colour in a field",				"#text(fill: red)[A]",							"#text(fill: blue)[A]"),
	("a length in a field",				"#h(10pt)",										"#h(12pt)"),
];

#[test]
fn the_same_text_at_shifted_offsets_fingerprints_alike() -> Outcome<()> {
	for (name, a, b) in SHIFTED {
		assert_ne!(a, b, "{}: the two sources are the same text", name);
		let fa = res!(fp(a));
		let fb = res!(fp(b));
		assert_eq!(fa, fb, "{}: a shifted offset changed the fingerprint", name);
		assert_eq!(fa, res!(fp(a)), "{}: two evaluations of one source differ", name);
	}
	assert_eq!(SHIFTED.len(), 6, "a case was dropped");
	Ok(())
}

#[test]
fn a_change_to_any_part_of_the_content_changes_the_fingerprint() -> Outcome<()> {
	for (name, a, b) in DIFFERENT {
		let fa = res!(fp(a));
		let fb = res!(fp(b));
		assert_ne!(fa, fb, "{}: the fingerprint did not see the change", name);
		assert_eq!(fa, res!(fp(a)), "{}: two evaluations of one source differ", name);
	}
	assert_eq!(DIFFERENT.len(), 23, "a case was dropped");
	Ok(())
}

#[test]
fn spans_locations_and_places_are_not_in_a_content_fingerprint() {
	let a = elem(meta(Value::Int(7), Span::new(FileId(1), 4, 9)));
	let b = elem(meta(Value::Int(7), Span::new(FileId(3), 400, 900)));
	assert_eq!(a.fingerprint(), b.fingerprint(), "a span entered the fingerprint");
	let mut placed = meta(Value::Int(7), Span::detached());
	placed.location = Some(Location(11));
	placed.place = Some(Place(12));
	assert_eq!(a.fingerprint(), elem(placed).fingerprint(), "a location or place entered the fingerprint");
}

#[test]
fn a_guard_a_label_and_the_prepared_flag_are_in_a_content_fingerprint() {
	let plain = elem(meta(Value::Int(7), Span::detached())).fingerprint();
	let mut guarded = meta(Value::Int(7), Span::detached());
	guarded.guards.push(RecipeIndex(2));
	assert_ne!(plain, elem(guarded.clone()).fingerprint(), "a guard was not seen");
	let mut other = meta(Value::Int(7), Span::detached());
	other.guards.push(RecipeIndex(3));
	assert_ne!(elem(guarded.clone()).fingerprint(), elem(other).fingerprint(), "a guard's index was not seen");
	// Guards are a set: the order they were added in is not a difference.
	let mut two = meta(Value::Int(7), Span::detached());
	two.guards = vec![RecipeIndex(1), RecipeIndex(2)];
	let mut flip = meta(Value::Int(7), Span::detached());
	flip.guards = vec![RecipeIndex(2), RecipeIndex(1)];
	assert_eq!(elem(two).fingerprint(), elem(flip).fingerprint(), "the order of guards was seen");
	let mut labelled = meta(Value::Int(7), Span::detached());
	labelled.label = Some(Label::new("x"));
	assert_ne!(plain, elem(labelled).fingerprint(), "a label was not seen");
	let mut prepared = meta(Value::Int(7), Span::detached());
	prepared.prepared = true;
	assert_ne!(plain, elem(prepared).fingerprint(), "the prepared flag was not seen");
}

#[test]
fn the_order_fields_were_set_in_is_not_a_difference() {
	let mut a = meta(Value::Int(1), Span::detached());
	a.fields.push((FieldId(1), Value::Int(2)));
	let mut b = meta(Value::Int(2), Span::detached());
	b.fields = vec![(FieldId(1), Value::Int(2)), (FieldId(0), Value::Int(1))];
	assert_eq!(elem(a).fingerprint(), elem(b).fingerprint(), "the order of fields was seen");
}
