//! Span-free structural fingerprints of values, styles and selectors.
//!
//! A fingerprint stands for what a value is, never for where it was written: no span, no byte offset
//! and no source position enters one, so the same text at a shifted offset hashes alike. A closure is
//! the exception that proves the rule: it hashes by its parameters, its body's syntax text, its
//! captured values and the file it was written in, because a relative path in its body resolves
//! against that file. There is no debug-form fallback for a value that can hold content; every
//! [`Value`] variant is matched, so a variant added later fails to compile until it is hashed.
//!
//! Content is hashed here one node at a time ([`elem_fp`], [`seq_fp`], [`styled_fp`]), each writing
//! its children's fingerprints, which [`Content::fingerprint`] reads from a cell beside the content,
//! so a shared subtree is hashed once. A content fingerprint leaves out the span, the location and
//! the place; a reader that sees locations adds them (`intro::hash_content`).
//!
//! The cells are kept honest by type. [`Shared`] is the only handle content, an element or a style
//! list is held by, and it hands out `&mut` only through [`Shared::edit`] and [`Shared::take`], which
//! forget the fingerprint. `Arc::make_mut` on a unique `Arc` does not clone, so a clone's empty cell
//! alone would not do: an edit that skipped the reset would leave the old fingerprint on changed
//! content, and the compiler now refuses any path that could.

use crate::eval::args::Args;
use crate::eval::content::{
	Elem,
	FieldId,
	Sequence,
	Styled,
};
use crate::eval::func::{
	Closure,
	Func,
	Param,
};
use crate::eval::intro::{
	CounterKey,
};
use crate::eval::scope::Scope;
use crate::eval::select::Selector;
use crate::eval::styles::{
	Property,
	Recipe,
	RecipeIndex,
	Style,
	Styles,
	Transformation,
};
use crate::eval::value::{
	Alignment,
	Angle,
	Color,
	Dash,
	DashItem,
	Datetime,
	Direction,
	Gradient,
	GradientKind,
	Label,
	Length,
	Paint,
	Ratio,
	Stroke,
	SymVariants,
	Symbol,
	Tiling,
	Value,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_hash::fingerprint::Fingerprint;
use oxedyne_fe2o3_hash::fingerprint::Fingerprinter;
use oxedyne_fe2o3_hash::fingerprint::LazyFingerprint;

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

// Hashes a presence flag, then the value if there is one.
fn hash_opt<T, F: FnOnce(&mut Fingerprinter, &T)>(h: &mut Fingerprinter, v: &Option<T>, f: F) {
	match v {
		Some(x)	=> { h.write_u8(1); f(h, x); }
		None	=> h.write_u8(0),
	}
}

fn hash_length(h: &mut Fingerprinter, l: &Length) {
	h.write_f64(l.abs);
	h.write_f64(l.em);
}

fn hash_ratios(h: &mut Fingerprinter, r: &(Ratio, Ratio)) {
	h.write_f64(r.0.0);
	h.write_f64(r.1.0);
}

fn hash_color(h: &mut Fingerprinter, c: &Color) {
	h.write_u8(c.space as u8);
	for x in c.c {
		h.write_f32(x);
	}
	h.write_f32(c.alpha);
}

fn hash_angle(h: &mut Fingerprinter, a: &Angle) { h.write_f64(a.0); }

fn hash_gradient(h: &mut Fingerprinter, g: &Gradient) {
	match &g.kind {
		GradientKind::Linear { angle } => {
			h.write_u8(0);
			hash_angle(h, angle);
		}
		GradientKind::Radial { center, radius, focal_center, focal_radius } => {
			h.write_u8(1);
			hash_ratios(h, center);
			h.write_f64(radius.0);
			hash_ratios(h, focal_center);
			h.write_f64(focal_radius.0);
		}
		GradientKind::Conic { center, angle } => {
			h.write_u8(2);
			hash_ratios(h, center);
			hash_angle(h, angle);
		}
	}
	h.write_usize(g.stops.len());
	for (c, r) in &g.stops {
		hash_color(h, c);
		h.write_f64(r.0);
	}
	h.write_u8(g.space as u8);
	h.write_u8(g.relative as u8);
}

fn hash_tiling(h: &mut Fingerprinter, t: &Tiling) {
	h.write_fingerprint(t.body.fingerprint());
	hash_opt(h, &t.size, |h, s| {
		hash_length(h, &s.0);
		hash_length(h, &s.1);
	});
	hash_length(h, &t.spacing.0);
	hash_length(h, &t.spacing.1);
	h.write_u8(t.relative as u8);
}

fn hash_paint(h: &mut Fingerprinter, p: &Paint) {
	match p {
		Paint::Color(c)		=> { h.write_u8(0); hash_color(h, c); }
		Paint::Gradient(g)	=> { h.write_u8(1); hash_gradient(h, g); }
		Paint::Tiling(t)	=> { h.write_u8(2); hash_tiling(h, t); }
	}
}

fn hash_dash(h: &mut Fingerprinter, d: &Dash) {
	h.write_usize(d.array.len());
	for item in &d.array {
		match item {
			DashItem::Len(l)	=> { h.write_u8(0); hash_length(h, l); }
			DashItem::Dot		=> h.write_u8(1),
		}
	}
	hash_length(h, &d.phase);
}

fn hash_stroke(h: &mut Fingerprinter, s: &Stroke) {
	hash_opt(h, &s.paint, hash_paint);
	hash_opt(h, &s.thickness, hash_length);
	hash_opt(h, &s.cap, |h, c| h.write_u8(*c as u8));
	hash_opt(h, &s.join, |h, j| h.write_u8(*j as u8));
	hash_opt(h, &s.dash, |h, d| hash_opt(h, d, hash_dash));
	hash_opt(h, &s.miter_limit, |h, m| h.write_f64(*m));
}

fn hash_alignment(h: &mut Fingerprinter, a: &Alignment) {
	hash_opt(h, &a.x, |h, x| h.write_u8(*x as u8));
	hash_opt(h, &a.y, |h, y| h.write_u8(*y as u8));
}

fn hash_symbol(h: &mut Fingerprinter, s: &Symbol) {
	match &s.variants {
		SymVariants::Single(t)		=> {
			h.write_u8(0);
			h.write_str(t);
		}
		SymVariants::Static(rows)	=> {
			h.write_u8(1);
			h.write_usize(rows.len());
			for (m, t) in rows.iter() {
				h.write_str(m);
				h.write_str(t);
			}
		}
		SymVariants::Runtime(rows)	=> {
			h.write_u8(2);
			h.write_usize(rows.len());
			for (m, t) in rows.iter() {
				h.write_str(m);
				h.write_str(t);
			}
		}
	}
	h.write_str(&s.modifiers);
}

fn hash_datetime(h: &mut Fingerprinter, d: &Datetime) {
	hash_opt(h, &d.year, |h, v| h.write_i32(*v));
	hash_opt(h, &d.month, |h, v| h.write_u8(*v));
	hash_opt(h, &d.day, |h, v| h.write_u8(*v));
	hash_opt(h, &d.hour, |h, v| h.write_u8(*v));
	hash_opt(h, &d.minute, |h, v| h.write_u8(*v));
	hash_opt(h, &d.second, |h, v| h.write_u8(*v));
}

fn hash_direction(h: &mut Fingerprinter, d: &Direction) { h.write_u8(*d as u8); }

/// Hashes a scope's own frame by name, and any intermediate frame above it; the parentless root is the
/// standard library, which every evaluation shares, and is left out.
fn hash_scope(h: &mut Fingerprinter, s: &Scope) {
	let mut frame = Some(s);
	let mut first = true;
	while let Some(f) = frame {
		if !first && f.parent.is_none() {
			break;
		}
		first = false;
		let mut names: Vec<&String> = f.map.keys().collect();
		names.sort();
		h.write_usize(names.len());
		for n in names {
			h.write_str(n);
			if let Some(b) = f.map.get(n) {
				hash_value(h, &b.value);
			}
		}
		frame = f.parent.as_deref();
	}
}

/// The fingerprint of a closure: its name, parameters, body text, file and everything it captured.
pub fn closure_fp(c: &Closure) -> Fingerprint {
	let mut h = Fingerprinter::new();
	hash_opt(&mut h, &c.name(), |h, n| h.write_str(n));
	h.write_usize(c.params().len());
	for p in c.params() {
		match p {
			Param::Pos(node)				=> { h.write_u8(0); h.write_str(&node.full_text()); }
			Param::Named { name, default }	=> { h.write_u8(1); h.write_str(name); hash_value(&mut h, default); }
			Param::Sink(name)				=> { h.write_u8(2); hash_opt(&mut h, name, |h, n| h.write_str(n)); }
		}
	}
	h.write_str(&c.body().full_text());
	h.write_u16(c.span().file.0);
	hash_scope(&mut h, c.captured());
	h.finish()
}

pub fn hash_func(h: &mut Fingerprinter, f: &Func) {
	match f {
		// A native function is a fieldless enum, so its debug form is its identity.
		Func::Native(n)		=> { h.write_u8(0); h.write_str(&fmt!("{:?}", n)); }
		Func::Element(k)	=> { h.write_u8(1); h.write_u64(*k as u64); }
		Func::Closure(c)	=> { h.write_u8(2); h.write_fingerprint(c.fingerprint()); }
		Func::With(w)		=> { h.write_u8(3); hash_func(h, &w.0); hash_args(h, &w.1); }
	}
}

pub fn hash_args(h: &mut Fingerprinter, a: &Args) {
	h.write_usize(a.items.len());
	for arg in a.items.iter() {
		hash_opt(h, &arg.name, |h, n| h.write_str(n));
		hash_value(h, &arg.value);
	}
}

pub fn hash_selector(h: &mut Fingerprinter, s: &Selector) {
	match s {
		Selector::Elem(k, fields) => {
			h.write_u8(0);
			h.write_u64(*k as u64);
			hash_opt(h, fields, |h, fs| {
				h.write_usize(fs.len());
				for (id, v) in fs {
					h.write_u8(id.0);
					hash_value(h, v);
				}
			});
		}
		Selector::Label(l)		=> { h.write_u8(1); h.write_str(l.as_str()); }
		Selector::Text(t)		=> { h.write_u8(2); h.write_str(t); }
		Selector::Regex(r)		=> { h.write_u8(3); h.write_str(&r.pattern); }
		Selector::Location(l)	=> { h.write_u8(4); h.write_u64(l.0); }
		Selector::Or(ss)		=> {
			h.write_u8(5);
			h.write_usize(ss.len());
			for x in ss {
				hash_selector(h, x);
			}
		}
		Selector::And(ss)		=> {
			h.write_u8(6);
			h.write_usize(ss.len());
			for x in ss {
				hash_selector(h, x);
			}
		}
		Selector::Before { selector, end, inclusive } => {
			h.write_u8(7);
			hash_selector(h, selector);
			hash_selector(h, end);
			h.write_bool(*inclusive);
		}
		Selector::After { selector, start, inclusive } => {
			h.write_u8(8);
			hash_selector(h, selector);
			hash_selector(h, start);
			h.write_bool(*inclusive);
		}
	}
}

fn hash_property(h: &mut Fingerprinter, p: &Property) {
	h.write_u64(p.elem as u64);
	h.write_u8(p.field.0);
	hash_value(h, &p.value);
	h.write_bool(p.liftable);
	h.write_bool(p.outside);
}

fn hash_recipe(h: &mut Fingerprinter, r: &Recipe) {
	hash_opt(h, &r.selector, hash_selector);
	match &r.transform {
		Transformation::Content(c)	=> { h.write_u8(0); h.write_fingerprint(c.fingerprint()); }
		Transformation::Func(f)		=> { h.write_u8(1); hash_func(h, f); }
		Transformation::Style(s)	=> { h.write_u8(2); h.write_fingerprint(s.fingerprint()); }
	}
	h.write_bool(r.outside);
}

pub fn hash_style(h: &mut Fingerprinter, s: &Style) {
	match s {
		Style::Property(p)		=> { h.write_u8(0); hash_property(h, p); }
		Style::Recipe(r)		=> { h.write_u8(1); hash_recipe(h, r); }
		Style::Revocation(i)	=> { h.write_u8(2); h.write_usize(i.0); }
	}
}

pub fn hash_styles(h: &mut Fingerprinter, s: &Styles) {
	h.write_usize(s.len());
	for style in s.iter() {
		hash_style(h, style);
	}
}

/// Hashes a value by its structure, span-free.
pub fn hash_value(h: &mut Fingerprinter, v: &Value) {
	match v {
		Value::None				=> h.write_u8(0),
		Value::Auto				=> h.write_u8(1),
		Value::Bool(b)			=> { h.write_u8(2); h.write_bool(*b); }
		Value::Int(i)			=> { h.write_u8(3); h.write_i64(*i); }
		Value::Float(f)			=> { h.write_u8(4); h.write_f64(*f); }
		// A decimal holds three plain numbers, so its debug form is its value and its scale.
		Value::Decimal(d)		=> { h.write_u8(5); h.write_str(&fmt!("{:?}", d)); }
		Value::Length(l)		=> { h.write_u8(6); hash_length(h, l); }
		Value::Angle(a)			=> { h.write_u8(7); hash_angle(h, a); }
		Value::Ratio(r)			=> { h.write_u8(8); h.write_f64(r.0); }
		Value::Relative(r)		=> { h.write_u8(9); h.write_f64(r.rel.0); hash_length(h, &r.abs); }
		Value::Fraction(f)		=> { h.write_u8(10); h.write_f64(f.0); }
		Value::Color(c)			=> { h.write_u8(11); hash_color(h, c); }
		Value::Gradient(g)		=> { h.write_u8(12); hash_gradient(h, g); }
		Value::Tiling(t)		=> { h.write_u8(13); hash_tiling(h, t); }
		Value::Stroke(s)		=> { h.write_u8(14); hash_stroke(h, s); }
		Value::Alignment(a)		=> { h.write_u8(15); hash_alignment(h, a); }
		Value::Direction(d)		=> { h.write_u8(16); hash_direction(h, d); }
		Value::Symbol(s)		=> { h.write_u8(17); hash_symbol(h, s); }
		Value::Str(s)			=> { h.write_u8(18); h.write_str(s); }
		Value::Bytes(b)			=> { h.write_u8(19); h.write_u64(b.len() as u64); h.write(b); }
		Value::Label(l)			=> { h.write_u8(20); h.write_str(l.as_str()); }
		Value::Datetime(d)		=> { h.write_u8(21); hash_datetime(h, d); }
		Value::Duration(d)		=> { h.write_u8(22); h.write_f64(d.secs); }
		Value::Version(parts)	=> {
			h.write_u8(23);
			h.write_usize(parts.len());
			for p in parts.iter() {
				h.write_u32(*p);
			}
		}
		Value::Regex(r)			=> { h.write_u8(24); h.write_str(&r.pattern); }
		Value::Content(c)		=> { h.write_u8(25); h.write_fingerprint(c.fingerprint()); }
		Value::Array(a)			=> {
			h.write_u8(26);
			h.write_usize(a.len());
			for x in a.iter() {
				hash_value(h, x);
			}
		}
		// A dictionary is hashed in insertion order, which is the order Typst iterates it in.
		Value::Dict(d)			=> {
			h.write_u8(27);
			h.write_usize(d.len());
			for (k, x) in d.iter() {
				h.write_str(k);
				hash_value(h, x);
			}
		}
		Value::Func(f)			=> { h.write_u8(28); hash_func(h, f); }
		Value::Args(a)			=> { h.write_u8(29); hash_args(h, a); }
		// A module's scope is its own frame; the standard library behind it is shared and left out.
		Value::Module(m)		=> {
			h.write_u8(30);
			h.write_str(&m.name);
			hash_scope(h, &m.scope);
			h.write_fingerprint(m.content.fingerprint());
		}
		Value::Type(t)			=> { h.write_u8(31); h.write_u64(*t as u64); }
		Value::Styles(s)		=> { h.write_u8(32); h.write_fingerprint(s.fingerprint()); }
		Value::Selector(s)		=> { h.write_u8(33); hash_selector(h, s); }
		Value::Counter(c)		=> {
			h.write_u8(34);
			match &c.key {
				CounterKey::Page		=> h.write_u8(0),
				CounterKey::Selector(s)	=> { h.write_u8(1); hash_selector(h, s); }
				CounterKey::Str(s)		=> { h.write_u8(2); h.write_str(s); }
			}
		}
		Value::State(s)			=> { h.write_u8(35); h.write_str(&s.key); hash_value(h, &s.init); }
		Value::Location(l)		=> { h.write_u8(36); h.write_u64(l.0); }
	}
}

// Guards are a set, so they are written sorted.
fn hash_guards(h: &mut Fingerprinter, guards: &[RecipeIndex]) {
	let mut g: Vec<usize> = guards.iter().map(|i| i.0).collect();
	g.sort_unstable();
	h.write_usize(g.len());
	for i in g {
		h.write_usize(i);
	}
}

fn hash_label(h: &mut Fingerprinter, l: &Option<Label>) {
	hash_opt(h, l, |h, l| h.write_str(l.as_str()));
}

/// The fingerprint of an element: its kind, label, fields by id, guards and prepared flag.
pub fn elem_fp(e: &Elem) -> Fingerprint {
	let mut h = Fingerprinter::new();
	h.write_u8(0);
	h.write_u64(e.kind as u64);
	hash_label(&mut h, &e.label);
	let mut fields: Vec<&(FieldId, Value)> = e.fields.iter().collect();
	fields.sort_by_key(|(id, _)| id.0);
	h.write_usize(fields.len());
	for (id, v) in fields {
		h.write_u8(id.0);
		hash_value(&mut h, v);
	}
	hash_guards(&mut h, &e.guards);
	h.write_bool(e.prepared);
	h.finish()
}

// Does the value hold content, or code that makes it? Such a field is what an edit changes.
fn holds_content(v: &Value) -> bool {
	match v {
		Value::Content(_) | Value::Func(_) | Value::Args(_) | Value::Module(_)	=> true,
		Value::Array(a)															=> a.iter().any(holds_content),
		Value::Dict(d)															=> d.iter().any(|(_, x)| holds_content(x)),
		_																		=> false,
	}
}

/// The shell of an element: its kind, label and the fields that hold no content, by id. Typing into the
/// content it holds leaves its shell as it was, so a place keyed by the shell stays where it is.
pub fn elem_shell_fp(e: &Elem) -> Fingerprint {
	let mut h = Fingerprinter::new();
	h.write_u8(3);
	h.write_u64(e.kind as u64);
	hash_label(&mut h, &e.label);
	let mut fields: Vec<&(FieldId, Value)> = e.fields.iter().filter(|(_, v)| !holds_content(v)).collect();
	fields.sort_by_key(|(id, _)| id.0);
	h.write_usize(fields.len());
	for (id, v) in fields {
		h.write_u8(id.0);
		hash_value(&mut h, v);
	}
	h.finish()
}

/// The shell of a sequence: its label alone, since it holds nothing but children.
pub fn seq_shell_fp(s: &Sequence) -> Fingerprint {
	let mut h = Fingerprinter::new();
	h.write_u8(4);
	hash_label(&mut h, &s.label);
	h.finish()
}

/// The shell of styled content: nothing, since it holds a child and styles over it.
pub fn styled_shell_fp() -> Fingerprint {
	let mut h = Fingerprinter::new();
	h.write_u8(5);
	h.finish()
}

/// The fingerprint of a sequence: its label, guards and children in order.
pub fn seq_fp(s: &Sequence) -> Fingerprint {
	let mut h = Fingerprinter::new();
	h.write_u8(1);
	hash_label(&mut h, &s.label);
	hash_guards(&mut h, &s.guards);
	h.write_usize(s.children.len());
	for c in &s.children {
		h.write_fingerprint(c.fingerprint());
	}
	h.finish()
}

/// The fingerprint of styled content: its child and its styles.
pub fn styled_fp(s: &Styled) -> Fingerprint {
	let mut h = Fingerprinter::new();
	h.write_u8(2);
	h.write_fingerprint(s.child.fingerprint());
	h.write_fingerprint(s.styles.fingerprint());
	h.finish()
}

/// The fingerprint of a style list.
pub fn styles_fp(s: &Styles) -> Fingerprint {
	let mut h = Fingerprinter::new();
	hash_styles(&mut h, s);
	h.finish()
}

/// A value that keeps its fingerprint in a cell of its own.
pub trait Kept: Clone {
	fn cell(&mut self) -> &mut LazyFingerprint;
}

/// A shared handle on a value and its fingerprint cell. Handles clone as a reference-count bump and
/// share the cell, so a subtree is hashed once however many parents hold it. The value is readable
/// through [`Deref`] and changeable only through [`Shared::edit`] or [`Shared::take`].
pub struct Shared<T: Kept>(Arc<T>);

impl<T: Kept> Shared<T> {
	pub fn new(v: T) -> Self { Shared(Arc::new(v)) }

	/// The value for change, copied first if another handle shares it, its fingerprint forgotten.
	pub fn edit(&mut self) -> &mut T {
		let v = Arc::make_mut(&mut self.0);
		v.cell().clear();
		v
	}

	/// The value itself, copied if another handle shares it, its fingerprint forgotten.
	pub fn take(self) -> T {
		let mut v = Arc::try_unwrap(self.0).unwrap_or_else(|a| (*a).clone());
		v.cell().clear();
		v
	}

	/// Does another handle hold the value, so that a change would copy it first?
	pub fn is_shared(&self) -> bool { Arc::strong_count(&self.0) > 1 }

	/// Do both handles share one value?
	pub fn ptr_eq(a: &Self, b: &Self) -> bool { Arc::ptr_eq(&a.0, &b.0) }
}

impl<T: Kept> Deref for Shared<T> {
	type Target = T;
	fn deref(&self) -> &T { &self.0 }
}

impl<T: Kept> Clone for Shared<T> {
	fn clone(&self) -> Self { Shared(Arc::clone(&self.0)) }
}

impl<T: Kept + Default> Default for Shared<T> {
	fn default() -> Self { Shared::new(T::default()) }
}

// Transparent, so a handle prints as the value it holds.
impl<T: Kept + fmt::Debug> fmt::Debug for Shared<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Debug::fmt(&*self.0, f) }
}
