//! U5: content fingerprints stand for what the content is, never for where it was written. The same text at a
//! shifted offset fingerprints alike; a change to any field, label, kind, guard, style, recipe or closure
//! gives a different one; and a fingerprint read before a change is never the one read after it.

use oxedyne_fe2o3_austenite::eval::content::{
	build,
	synthesise,
	Content,
	Elem,
	ElemKind,
	FieldId,
	Sequence,
	Styled,
};
use oxedyne_fe2o3_austenite::eval::fp::{
	elem_fp,
	Shared,
};
use oxedyne_fe2o3_austenite::eval::locate::{
	Location,
	Place,
};
use oxedyne_fe2o3_austenite::eval::ops::content_add;
use oxedyne_fe2o3_austenite::eval::styles::{
	Property,
	RecipeIndex,
	StyleChain,
	Style,
	Styles,
};
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
	Elem::new(ElemKind::Metadata, vec![(FieldId(0), v)], span)
}

fn elem(e: Elem) -> Content { Content::from_elem(e) }

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

// Mutation. A fingerprint read before a change is never the one read after it. Each case reads the
// fingerprint of a node held by one handle only, so the change is made in place, where a stale cell
// would show; changes it; and compares with a node built afresh to the same shape, whose cell has never
// been read.

// The fingerprint after a change is a new one, and the one a cold node of the same shape has.
fn fresh(what: &str, warm: Fingerprint, now: &Content, cold: &Content) {
	assert_ne!(warm, now.fingerprint(), "{}: the old fingerprint survived the change", what);
	assert_eq!(now.fingerprint(), cold.fingerprint(), "{}: the fingerprint is not a cold one", what);
}

fn t(s: &str) -> Content { Content::text(s) }

fn seq(v: &[&str]) -> Content { Content::sequence(v.iter().map(|s| t(s)).collect()) }

fn prop(n: i64) -> Style {
	Style::Property(Property::new(ElemKind::Text, FieldId(1), Value::Int(n), Span::detached()))
}

fn styles(n: &[i64]) -> Styles { Styles::from_vec(n.iter().map(|n| prop(*n)).collect()) }

#[test]
fn setting_a_field_of_a_warm_element_gives_a_fresh_fingerprint() {
	let mut c = elem(meta(Value::Int(1), Span::detached()));
	let warm = c.fingerprint();
	c.set(FieldId(0), Value::Int(2));
	fresh("an existing field", warm, &c, &elem(meta(Value::Int(2), Span::detached())));
	let warm = c.fingerprint();
	c.set(FieldId(1), Value::Int(5));
	let cold = Elem::new(ElemKind::Metadata, vec![(FieldId(0), Value::Int(2)), (FieldId(1), Value::Int(5))], Span::detached());
	fresh("a new field", warm, &c, &elem(cold));
	if let Content::Elem(e) = &c {
		assert_eq!(c.fingerprint(), elem_fp(e), "the cell and a pure recomputation differ");
	}
	let warm = c.fingerprint();
	c.set(FieldId(1), Value::Int(5));
	assert_eq!(warm, c.fingerprint(), "setting a field to the value it has changed the fingerprint");
}

#[test]
fn labelling_warm_content_gives_a_fresh_fingerprint() {
	let label = || Label::new("x");
	// An element.
	let c = elem(meta(Value::Int(7), Span::detached()));
	let warm = c.fingerprint();
	let c = c.labelled(label());
	let mut cold = meta(Value::Int(7), Span::detached());
	cold.label = Some(label());
	fresh("an element", warm, &c, &elem(cold));
	// A sequence.
	let c = seq(&["a", "b"]);
	let warm = c.fingerprint();
	let c = c.labelled(label());
	let mut cold = Sequence::new(vec![t("a"), t("b")]);
	cold.label = Some(label());
	fresh("a sequence", warm, &c, &Content::Sequence(Shared::new(cold)));
	// Styled content, which labels its child.
	let c = t("a").styled(styles(&[1]));
	let warm = c.fingerprint();
	let c = c.labelled(label());
	let mut child = Elem::new(ElemKind::Text, vec![(FieldId(0), Value::str("a"))], Span::detached());
	child.label = Some(label());
	fresh("styled content", warm, &c, &Content::Styled(Shared::new(Styled::new(elem(child), styles(&[1])))));
}

#[test]
fn a_place_and_a_span_change_no_fingerprint() {
	let mut c = elem(meta(Value::Int(7), Span::new(FileId(1), 4, 9)));
	let warm = c.fingerprint();
	c.set_place(Place(3));
	assert_eq!(warm, c.fingerprint(), "a place changed the fingerprint");
	let c = c.with_span(Span::new(FileId(2), 40, 90));
	assert_eq!(warm, c.fingerprint(), "a span changed the fingerprint");
	let c = seq(&["a", "b"]);
	let warm = c.fingerprint();
	let c = c.with_span(Span::new(FileId(2), 40, 90));
	assert_eq!(warm, c.fingerprint(), "a sequence's span changed the fingerprint");
}

#[test]
fn adding_to_a_warm_sequence_gives_a_fresh_fingerprint() {
	let x = seq(&["a", "b"]);
	let warm = x.fingerprint();
	fresh("sequence and sequence", warm, &content_add(x, seq(&["c", "d"])), &seq(&["a", "b", "c", "d"]));
	let x = seq(&["a", "b"]);
	let warm = x.fingerprint();
	fresh("sequence and content", warm, &content_add(x, t("c")), &seq(&["a", "b", "c"]));
	let y = seq(&["b", "c"]);
	let warm = y.fingerprint();
	fresh("content and sequence", warm, &content_add(t("a"), y), &seq(&["a", "b", "c"]));
	let a = t("a");
	let warm = a.fingerprint();
	fresh("content and content", warm, &content_add(a, t("b")), &seq(&["a", "b"]));
}

#[test]
fn a_child_changed_through_its_parent_gives_the_parent_a_fresh_fingerprint() {
	let mut c = seq(&["a", "b"]);
	let warm = c.fingerprint();
	if let Content::Sequence(s) = &mut c {
		s.edit().children[0].set(FieldId(0), Value::str("z"));
	}
	fresh("a sequence's child", warm, &c, &seq(&["z", "b"]));
	let mut c = t("a").styled(styles(&[1]));
	let warm = c.fingerprint();
	if let Content::Styled(s) = &mut c {
		s.edit().child.set(FieldId(0), Value::str("z"));
	}
	fresh("styled content's child", warm, &c, &t("z").styled(styles(&[1])));
}

#[test]
fn growing_a_warm_style_list_gives_a_fresh_fingerprint() {
	let mut s = styles(&[1]);
	let warm = s.fingerprint();
	s.push(prop(2));
	assert_ne!(warm, s.fingerprint(), "push: the old fingerprint survived");
	assert_eq!(s.fingerprint(), styles(&[1, 2]).fingerprint(), "push: not a cold fingerprint");
	let warm = s.fingerprint();
	s.extend(&styles(&[3, 4]));
	assert_ne!(warm, s.fingerprint(), "extend: the old fingerprint survived");
	assert_eq!(s.fingerprint(), styles(&[1, 2, 3, 4]).fingerprint(), "extend: not a cold fingerprint");
	let warm = s.fingerprint();
	s.apply_outer(&styles(&[0]));
	assert_ne!(warm, s.fingerprint(), "apply_outer: the old fingerprint survived");
	assert_eq!(s.fingerprint(), styles(&[0, 1, 2, 3, 4]).fingerprint(), "apply_outer: not a cold fingerprint");
	assert_ne!(styles(&[1, 2]).fingerprint(), styles(&[2, 1]).fingerprint(), "the order of styles was not seen");
	// The list inside styled content changes its fingerprint too, though the styled node is a level up.
	let mut c = t("a").styled(styles(&[1]));
	let warm = c.fingerprint();
	if let Content::Styled(h) = &mut c {
		h.edit().styles.push(prop(2));
	}
	fresh("styled content's styles", warm, &c, &t("a").styled(styles(&[1, 2])));
}

#[test]
fn a_clone_that_is_changed_leaves_the_original_fingerprint_alone() {
	let a = elem(meta(Value::Int(1), Span::detached()));
	let warm = a.fingerprint();
	let mut b = a.clone();
	assert_eq!(b.fingerprint(), warm, "a clone does not fingerprint as its original");
	b.set(FieldId(0), Value::Int(2));
	assert_eq!(a.fingerprint(), warm, "the original's fingerprint moved");
	assert_ne!(b.fingerprint(), warm, "the clone's fingerprint did not move");
	let a = seq(&["a", "b"]);
	let warm = a.fingerprint();
	let b = content_add(a.clone(), t("c"));
	assert_eq!(a.fingerprint(), warm, "the original sequence's fingerprint moved");
	assert_eq!(a.fingerprint(), seq(&["a", "b"]).fingerprint(), "the original sequence changed");
	assert_ne!(b.fingerprint(), warm, "the extended clone's fingerprint did not move");
	let a = styles(&[1]);
	let warm = a.fingerprint();
	let mut b = a.clone();
	assert!(a.ptr_eq(&b), "a clone of a style list does not share it");
	b.push(prop(2));
	assert!(!a.ptr_eq(&b), "a changed clone still shares the list");
	assert_eq!(a.fingerprint(), warm, "the original style list's fingerprint moved");
	assert_ne!(b.fingerprint(), warm, "the changed clone's fingerprint did not move");
}

#[test]
fn synthesising_a_warm_heading_gives_a_fresh_fingerprint() -> Outcome<()> {
	let mut engine = Engine::new(World::new(PathBuf::from("/")));
	let mut h = res!(build(&mut engine, ElemKind::Heading, vec![("body", Value::Content(t("T")))], Span::detached()));
	let warm = h.fingerprint();
	res!(synthesise(&mut engine, &mut h, &StyleChain::root()));
	assert_ne!(warm, h.fingerprint(), "synthesis left the old fingerprint");
	if let Content::Elem(e) = &h {
		assert_eq!(h.fingerprint(), elem_fp(e), "the cell and a pure recomputation differ");
	}
	Ok(())
}

// A chain of functions in which each calls the one before it twice, through two names, so that the closure
// tree a naive walk unfolds doubles at every level while the closures themselves stay few.
fn chain(depth: usize, base: &str) -> String {
	let mut s = format!("#let f0(x) = {}\n", base);
	for i in 1..=depth {
		s.push_str(&fmt!("#let a{i} = f{p}\n#let b{i} = f{p}\n#let f{i}(x) = a{i}(b{i}(x))\n", i = i, p = i - 1));
	}
	s.push_str(&fmt!("#metadata(f{})\n", depth));
	s
}

// Evaluates and fingerprints on a thread of its own, so that a walk that never ends fails the test rather than
// holding the run for ever.
fn fp_within(src: String, secs: u64) -> Fingerprint {
	let (tx, rx) = std::sync::mpsc::channel();
	std::thread::spawn(move || {
		let _ = tx.send(fp(&src).map_err(|e| fmt!("{}", e)));
	});
	match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
		Ok(Ok(f))	=> f,
		Ok(Err(e))	=> panic!("the chain did not evaluate: {}", e),
		Err(_)		=> panic!("fingerprinting a chain of shared closures took over {} s: a closure reached by many paths is walked once per path", secs),
	}
}

#[test]
fn closures_reached_by_many_paths_fingerprint_in_time_proportional_to_the_closures() {
	let a = fp_within(chain(48, "x"), 20);
	let b = fp_within(chain(48, "x + 1"), 20);
	assert_ne!(a, b, "a change at the foot of the chain did not reach its head");
	assert_eq!(a, fp_within(chain(48, "x"), 20), "two evaluations of one chain differ");
}
