// U4 owns this file. The chain is `Arc`-linked rather than borrowed (`&Styles` links, as the design first
// sketched) so a realised element can carry its chain into flow, the fixpoint cache and the memo without a
// lifetime.
//
// Folding follows Typst's `Fold` implementations, keyed on the schema's `Fold`: `Add` is `text.size`'s
// em-relative accumulation (and concatenation for arrays, key overlay for dictionaries); `Merge` is the
// part-wise fold of strokes, sides and corners. A schema cannot yet say which parts a `Merge` field has, so
// `radius` folds as corners, `first-line-indent` as an `amount` dictionary and every other `Merge` field as
// sides. A fold that meets values it cannot combine lets the inner value win, as `Replace` would.

use crate::diag::Diagnostic;
use crate::eval::args::Args;
use crate::eval::content::{
	Content,
	ElemKind,
	FieldId,
	FieldSpec,
	FieldType,
	Fold,
};
use crate::eval::func::Func;
use crate::eval::lib::foundations;
use crate::eval::ops;
use crate::eval::select::Selector;
use crate::eval::value::{
	Dash,
	DashItem,
	Dict,
	FromValue,
	Length,
	LineCap,
	LineJoin,
	Paint,
	Stroke,
	SymVariants,
	Symbol,
	Type,
	Value,
};
use crate::eval::{
	Context,
	Engine,
};
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

pub const DEFAULT_FONT_SIZE_PT: f64 = 11.0;	// Typst's `text.size` default

#[derive(Clone, Debug)]
pub struct Property {
	pub elem:	ElemKind,
	pub field:	FieldId,
	pub value:	Value,
	pub span:	Span,
}

impl Property {
	pub fn new(elem: ElemKind, field: FieldId, value: Value, span: Span) -> Self {
		Self { elem, field, value, span }
	}
}

/// What a show rule does to a matched element.
#[derive(Clone, Debug)]
pub enum Transformation {
	Content(Content),	// `show sel: [replacement]`
	Func(Func),			// `show sel: it => ..`, and `show: f`
	Style(Styles),		// `show sel: set ..`
}

/// The identity of a recipe for the guard set: its position counted from the root of the chain (the
/// outermost recipe is 1), so it stays put while the chain grows inwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecipeIndex(pub usize);

#[derive(Clone, Debug)]
pub struct Recipe {
	pub selector:	Option<Selector>,	// None: `show: f`, wrapping the rest of the scope
	pub transform:	Transformation,
	pub span:		Span,
}

impl Recipe {
	/// Is this a show-set rule, whose styles are collected rather than applied as a step?
	pub fn is_show_set(&self) -> bool { matches!(self.transform, Transformation::Style(_)) }

	/// Does the recipe's selector match the target? A selector-less recipe matches nothing here: it
	/// transforms the rest of its scope when it is applied, not individual elements.
	pub fn applicable(&self, target: &Content, styles: &StyleChain) -> Outcome<bool> {
		match &self.selector {
			Some(sel)	=> sel.matches(target, Some(styles)),
			None		=> Ok(false),
		}
	}
}

/// One entry of a `Styles` list. A `Revocation` switches the recipe with that index off for everything
/// styled beneath it; realisation uses it so a text rule's output is not matched by the same rule.
#[derive(Clone, Debug)]
pub enum Style {
	Property(Property),
	Recipe(Recipe),
	Revocation(RecipeIndex),
}

impl Style {
	/// The element a style concerns: a property's element, or a recipe's selected element. Grouping is
	/// interrupted by styles on the grouped elements.
	pub fn element(&self) -> Option<ElemKind> {
		match self {
			Style::Property(p)	=> Some(p.elem),
			Style::Recipe(r)	=> match &r.selector {
				Some(Selector::Elem(k, _))	=> Some(*k),
				_							=> None,
			},
			Style::Revocation(_) => None,
		}
	}

	pub fn span(&self) -> Span {
		match self {
			Style::Property(p)		=> p.span,
			Style::Recipe(r)		=> r.span,
			Style::Revocation(_)	=> Span::detached(),
		}
	}
}

/// A list of styles from one `set` or `show`, or several merged. Later entries are inner: they win.
#[derive(Clone, Debug, Default)]
pub struct Styles(pub Arc<Vec<Style>>);

impl Styles {
	pub fn new() -> Self { Self::default() }

	pub fn from_style(s: Style) -> Self { Styles(Arc::new(vec![s])) }

	pub fn from_vec(v: Vec<Style>) -> Self { Styles(Arc::new(v)) }

	pub fn is_empty(&self) -> bool { self.0.is_empty() }

	pub fn len(&self) -> usize { self.0.len() }

	pub fn push(&mut self, s: Style) { Arc::make_mut(&mut self.0).push(s); }

	pub fn iter(&self) -> impl Iterator<Item = &Style> { self.0.iter() }

	pub fn as_slice(&self) -> &[Style] { &self.0 }

	/// Appends `other`'s styles after this one's, as consecutive `set` rules accumulate.
	pub fn extend(&mut self, other: &Styles) {
		Arc::make_mut(&mut self.0).extend(other.0.iter().cloned());
	}

	/// Puts `outer` before these styles, so these keep precedence: Typst's `Styles::apply`.
	pub fn apply_outer(&mut self, outer: &Styles) {
		if outer.is_empty() {
			return;
		}
		let mut v = (*outer.0).clone();
		v.extend(self.0.iter().cloned());
		self.0 = Arc::new(v);
	}

	/// Does any style set a property of `kind`?
	pub fn has_property_of(&self, kind: ElemKind) -> bool {
		self.iter().any(|s| matches!(s, Style::Property(p) if p.elem == kind))
	}
}

#[derive(Debug)]
pub struct ChainLink {
	pub styles:	Styles,
	pub parent:	Option<Arc<ChainLink>>,
}

/// The styles in force at a point, innermost first. Cloning is a reference-count bump.
#[derive(Clone, Debug, Default)]
pub struct StyleChain {
	pub head:	Option<Arc<ChainLink>>,
}

impl StyleChain {
	pub fn root() -> Self { Self::default() }

	/// This chain with `styles` pushed innermost.
	pub fn chain(&self, styles: &Styles) -> StyleChain {
		if styles.is_empty() {
			return self.clone();
		}
		StyleChain { head: Some(Arc::new(ChainLink { styles: styles.clone(), parent: self.head.clone() })) }
	}

	/// The links, innermost first.
	pub fn links(&self) -> Vec<&Arc<ChainLink>> {
		let mut links = Vec::new();
		let mut cur = self.head.as_ref();
		while let Some(l) = cur {
			links.push(l);
			cur = l.parent.as_ref();
		}
		links
	}

	pub fn depth(&self) -> usize { self.links().len() }

	/// Are both the same chain, link for link? Two chains built separately from equal styles are not.
	pub fn ptr_eq(&self, other: &StyleChain) -> bool {
		match (&self.head, &other.head) {
			(None, None)		=> true,
			(Some(a), Some(b))	=> Arc::ptr_eq(a, b),
			_					=> false,
		}
	}

	/// Every style, innermost first.
	pub fn walk(&self) -> impl Iterator<Item = &Style> {
		self.links().into_iter().flat_map(|l| l.styles.0.iter().rev())
	}

	/// Every recipe with its index, innermost first, skipping those a revocation further in switched off.
	pub fn recipes(&self) -> Vec<(RecipeIndex, &Recipe)> {
		let all: Vec<&Style> = self.walk().collect();
		let total = all.iter().filter(|s| matches!(s, Style::Recipe(_))).count();
		let mut revoked = Vec::new();
		let mut out = Vec::new();
		let mut r = 0;
		for s in all {
			match s {
				Style::Recipe(recipe) => {
					let index = RecipeIndex(total - r);
					r += 1;
					if !revoked.contains(&index) {
						out.push((index, recipe));
					}
				}
				Style::Revocation(i)	=> revoked.push(*i),
				Style::Property(_)		=> (),
			}
		}
		out
	}

	/// The chain of the longest link sequence every chain shares, counted from the root: Typst's trunk,
	/// under which realisation places a group built from differently styled members.
	pub fn trunk<'a, I: IntoIterator<Item = &'a StyleChain>>(chains: I) -> StyleChain {
		let mut trunk: Option<Vec<Arc<ChainLink>>> = None;
		for c in chains {
			let mut outer_first: Vec<Arc<ChainLink>> = c.links().into_iter().cloned().collect();
			outer_first.reverse();
			trunk = Some(match trunk {
				None	=> outer_first,
				Some(t)	=> {
					let n = t.iter().zip(outer_first.iter()).take_while(|(a, b)| Arc::ptr_eq(a, b)).count();
					t.into_iter().take(n).collect()
				}
			});
		}
		StyleChain { head: trunk.and_then(|t| t.last().cloned()) }
	}

	/// The styles of the links above the innermost `depth`-deep trunk, merged outer to inner: what must
	/// be reapplied locally when content moves under the trunk.
	pub fn suffix(&self, depth: usize) -> Styles {
		let links = self.links();
		let keep = links.len().saturating_sub(depth);
		let mut out = Styles::new();
		for l in links.into_iter().take(keep).rev() {
			out.extend(&l.styles);
		}
		out
	}

	/// Every value set for `elem.field`, innermost first, for a unit that folds a `Fold::Custom` field
	/// itself.
	pub fn values(&self, elem: ElemKind, field: FieldId) -> Vec<&Value> {
		self.walk().filter_map(|s| match s {
			Style::Property(p) if p.elem == elem && p.field == field	=> Some(&p.value),
			_															=> None,
		}).collect()
	}

	/// The value of `elem.field` in force: the set values folded by the field's schema rule onto the
	/// schema default, or the innermost set value for `Replace` and `Custom`.
	pub fn get(&self, elem: ElemKind, field: FieldId) -> Option<Value> {
		let spec = elem.field_spec(field);
		let values = self.values(elem, field);
		match spec {
			Some(s)	=> fold_all(s, &values),
			None	=> values.first().map(|v| (*v).clone()),
		}
	}

	pub fn get_as<T: FromValue>(&self, elem: ElemKind, field: FieldId) -> Outcome<Option<T>> {
		match self.get(elem, field) {
			Some(v)	=> Ok(Some(res!(T::from_value(v)))),
			None	=> Ok(None),
		}
	}

	/// The value for an element in hand: its own field if set -- folded onto the chain's value for a
	/// folding field, as `rect(stroke: 2pt)` under `set rect(stroke: red)` is a red 2pt stroke -- else the
	/// chain's.
	pub fn resolve(&self, elem: &Content, field: FieldId) -> Option<Value> {
		let kind = elem.kind();
		match elem.get(field) {
			Some(own) => {
				let spec = kind.and_then(|k| k.field_spec(field));
				match spec {
					Some(s) if matches!(s.fold, Fold::Add | Fold::Merge) => {
						match kind.and_then(|k| self.get(k, field)) {
							Some(outer) => {
								let is_def = kind.map(|k| self.values(k, field).is_empty()).unwrap_or(false);
								Some(fold(s, own, &outer, is_def))
							}
							None => Some(own.clone()),
						}
					}
					_ => Some(own.clone()),
				}
			}
			None => kind.and_then(|k| self.get(k, field)),
		}
	}

	/// The font size in force, in points: `text.size` folded, Typst's 11pt when the schema is silent.
	pub fn font_size(&self) -> f64 {
		let size = ElemKind::Text.field_id("size").and_then(|id| self.get(ElemKind::Text, id));
		match size {
			Some(Value::Length(l))	=> l.abs + l.em * DEFAULT_FONT_SIZE_PT,
			_						=> DEFAULT_FONT_SIZE_PT,
		}
	}

	/// A length in points, its em part resolved against the font size in force.
	pub fn resolve_length(&self, l: Length) -> f64 {
		if l.em == 0.0 {
			return l.abs;
		}
		l.resolve(self.font_size())
	}
}

// Folding

/// The value of a field set to `values` (innermost first) on top of its schema default: folded by the
/// field's rule, or the innermost value for `Replace` and `Custom`.
pub fn fold_all(spec: &FieldSpec, values: &[&Value]) -> Option<Value> {
	let default = spec.default.to_value();
	if !matches!(spec.fold, Fold::Add | Fold::Merge) {
		return match values.first() {
			Some(v)	=> Some((*v).clone()),
			None	=> default,
		};
	}
	let mut acc: Option<(Value, bool)> = default.map(|d| (d, true));
	for v in values.iter().rev() {
		acc = Some(match acc {
			None				=> ((*v).clone(), false),
			Some((o, is_def))	=> (fold(spec, v, &o, is_def), false),
		});
	}
	acc.map(|(v, _)| v)
}

/// Folds an inner value onto an outer one by the field's rule. `outer_default` says the outer value is
/// the schema default rather than a set value: a `none` default contributes no parts to a sides fold,
/// as Typst's `Sides<Option<T>>` default has none.
pub fn fold(spec: &FieldSpec, inner: &Value, outer: &Value, outer_default: bool) -> Value {
	match spec.fold {
		Fold::Replace | Fold::Custom	=> inner.clone(),
		Fold::Add						=> fold_add(inner, outer),
		Fold::Merge						=> fold_merge(Shape::of(spec), inner, outer, outer_default),
	}
}

fn fold_add(inner: &Value, outer: &Value) -> Value {
	match (inner, outer) {
		// Typst's `TextSize` fold: the inner em part is resolved against the outer size.
		(Value::Length(i), Value::Length(o)) => Value::Length(Length {
			abs:	i.abs + i.em * o.abs,
			em:		i.em * o.em,
		}),
		(Value::Array(i), Value::Array(o)) => {
			let mut v = (**o).clone();
			v.extend(i.iter().cloned());
			Value::array(v)
		}
		(Value::Dict(i), Value::Dict(o))	=> Value::dict(overlay(o, i)),
		_									=> inner.clone(),
	}
}

/// The parts a `Merge` field folds by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
	Sides,
	Corners,
	Keyed(&'static str),	// a dictionary whose bare-value form is this key (`first-line-indent`'s amount)
}

impl Shape {
	fn of(spec: &FieldSpec) -> Shape {
		match spec.name {
			"radius"			=> Shape::Corners,
			"first-line-indent"	=> Shape::Keyed("amount"),
			_					=> Shape::Sides,
		}
	}
}

const SIDE_KEYS:	&[&str] = &["left", "top", "right", "bottom", "x", "y", "rest", "inside", "outside"];
const CORNER_KEYS:	&[&str] = &["top-left", "top-right", "bottom-right", "bottom-left",
	"left", "top", "right", "bottom", "rest"];
const STROKE_KEYS:	&[&str] = &["paint", "thickness", "cap", "join", "dash", "miter-limit"];

fn fold_merge(shape: Shape, inner: &Value, outer: &Value, outer_default: bool) -> Value {
	if matches!(inner, Value::None | Value::Auto) {
		return inner.clone();
	}
	match shape {
		Shape::Keyed(key) => match (inner, outer) {
			(Value::Dict(i), Value::Dict(o))	=> Value::dict(overlay(o, i)),
			(Value::Dict(i), o) if !matches!(o, Value::None | Value::Auto) => {
				let mut base = Dict::new();
				base.insert(key, o.clone());
				Value::dict(overlay(&base, i))
			}
			(i, Value::Dict(o)) => {
				let mut top = Dict::new();
				top.insert(key, i.clone());
				Value::dict(overlay(o, &top))
			}
			_ => inner.clone(),
		},
		Shape::Sides | Shape::Corners => {
			let ip = parts(shape, inner);
			let op = parts(shape, outer);
			if ip.is_none() && op.is_none() {
				return fold_part(inner, outer);
			}
			let named = ip.as_ref().map(|p| p.two_sided).unwrap_or(false)
				|| op.as_ref().map(|p| p.two_sided).unwrap_or(false);
			let ip = match ip {
				Some(p)	=> p.parts,
				None	=> uniform(shape, Some(inner)),
			};
			let op = match op {
				Some(p)								=> p.parts,
				None if outer_default && outer.is_none()	=> uniform(shape, None),
				None								=> uniform(shape, Some(outer)),
			};
			let merged: Vec<Option<Value>> = ip.into_iter().zip(op).map(|(i, o)| match (i, o) {
				(Some(i), Some(o))	=> Some(fold_part(&i, &o)),
				(Some(i), None)		=> Some(i),
				(None, o)			=> o,
			}).collect();
			collapse(shape, merged, named)
		}
	}
}

/// Folds one part: strokes part-wise, anything else inner-wins.
fn fold_part(inner: &Value, outer: &Value) -> Value {
	if is_stroke_like(inner) && is_stroke_like(outer)
		&& !(matches!(inner, Value::Length(_)) && matches!(outer, Value::Length(_)))
	{
		if let (Some(i), Some(o)) = (to_stroke(inner), to_stroke(outer)) {
			return Value::Stroke(Arc::new(merge_stroke(i, o)));
		}
	}
	inner.clone()
}

struct Parts {
	parts:		Vec<Option<Value>>,	// sides: left, top, right, bottom; corners: tl, tr, br, bl
	two_sided:	bool,				// written with `inside`/`outside`
}

fn is_parts_dict(shape: Shape, d: &Dict) -> bool {
	let keys = match shape {
		Shape::Corners	=> CORNER_KEYS,
		_				=> SIDE_KEYS,
	};
	d.keys().all(|k| keys.contains(&k))
}

fn parts(shape: Shape, v: &Value) -> Option<Parts> {
	let d = match v {
		Value::Dict(d) if is_parts_dict(shape, d)	=> d,
		_											=> return None,
	};
	let get = |k: &str| d.get(k).cloned();
	match shape {
		Shape::Corners => {
			let rest = get("rest");
			let left = get("left").or_else(|| rest.clone());
			let top = get("top").or_else(|| rest.clone());
			let right = get("right").or_else(|| rest.clone());
			let bottom = get("bottom").or_else(|| rest.clone());
			Some(Parts { parts: vec![
				get("top-left").or_else(|| top.clone()).or_else(|| left.clone()),
				get("top-right").or_else(|| top.clone()).or_else(|| right.clone()),
				get("bottom-right").or_else(|| bottom.clone()).or_else(|| right.clone()),
				get("bottom-left").or_else(|| bottom.clone()).or_else(|| left.clone()),
			], two_sided: false })
		}
		_ => {
			let rest = get("rest");
			let x = get("x").or_else(|| rest.clone());
			let y = get("y").or_else(|| rest.clone());
			let two_sided = d.contains("inside") || d.contains("outside");
			let left = get("left").or_else(|| get("inside")).or_else(|| x.clone());
			let right = get("right").or_else(|| get("outside")).or_else(|| x.clone());
			Some(Parts { parts: vec![
				left,
				get("top").or_else(|| y.clone()),
				right,
				get("bottom").or_else(|| y.clone()),
			], two_sided })
		}
	}
}

fn uniform(_shape: Shape, v: Option<&Value>) -> Vec<Option<Value>> {
	vec![v.cloned(), v.cloned(), v.cloned(), v.cloned()]
}

/// Back to a value: one value when every part is present and equal, else a dictionary of the parts
/// present, in Typst's order.
fn collapse(shape: Shape, parts: Vec<Option<Value>>, two_sided: bool) -> Value {
	let all_same = match parts.first() {
		Some(Some(first)) => parts.iter().all(|p| match p {
			Some(v)	=> same(v, first),
			None	=> false,
		}),
		_ => false,
	};
	if all_same && !two_sided {
		if let Some(Some(v)) = parts.into_iter().next() {
			return v;
		}
		return Value::None;
	}
	let names: [&str; 4] = match shape {
		Shape::Corners				=> ["top-left", "top-right", "bottom-right", "bottom-left"],
		_ if two_sided				=> ["inside", "top", "outside", "bottom"],
		_							=> ["left", "top", "right", "bottom"],
	};
	let mut d = Dict::new();
	for (name, p) in names.iter().zip(parts) {
		if let Some(v) = p {
			d.insert(name, v);
		}
	}
	Value::dict(d)
}

fn overlay(outer: &Dict, inner: &Dict) -> Dict {
	let mut d = outer.clone();
	for (k, v) in inner.iter() {
		d.insert(k, v.clone());
	}
	d
}

fn same(a: &Value, b: &Value) -> bool {
	match (a, b) {
		(Value::Stroke(x), Value::Stroke(y))	=> stroke_eq(x, y),
		(Value::Color(x), Value::Color(y))		=> x == y,
		_										=> ops::equal(a, b),
	}
}

fn stroke_eq(a: &Stroke, b: &Stroke) -> bool {
	let paint_eq = match (&a.paint, &b.paint) {
		(None, None)										=> true,
		(Some(Paint::Color(x)), Some(Paint::Color(y)))		=> x == y,
		(Some(Paint::Gradient(x)), Some(Paint::Gradient(y)))	=> x == y,
		(Some(Paint::Tiling(x)), Some(Paint::Tiling(y)))	=> Arc::ptr_eq(x, y),
		_													=> false,
	};
	paint_eq && a.thickness == b.thickness && a.cap == b.cap && a.join == b.join
		&& a.dash == b.dash && a.miter_limit == b.miter_limit
}

fn is_stroke_like(v: &Value) -> bool {
	match v {
		Value::Stroke(_) | Value::Length(_) | Value::Color(_) | Value::Gradient(_) | Value::Tiling(_) => true,
		Value::Dict(d) => !d.is_empty() && d.keys().all(|k| STROKE_KEYS.contains(&k)),
		_ => false,
	}
}

/// A stroke from any value Typst accepts as one: a stroke, a length (thickness), a paint, or a
/// dictionary of stroke parts. `None` when the value is none of these or a part is malformed.
pub fn to_stroke(v: &Value) -> Option<Stroke> {
	match v {
		Value::Stroke(s)	=> Some((**s).clone()),
		Value::Length(l)	=> Some(Stroke { thickness: Some(*l), ..Stroke::default() }),
		Value::Color(c)		=> Some(Stroke { paint: Some(Paint::Color(*c)), ..Stroke::default() }),
		Value::Gradient(g)	=> Some(Stroke { paint: Some(Paint::Gradient(g.clone())), ..Stroke::default() }),
		Value::Tiling(t)	=> Some(Stroke { paint: Some(Paint::Tiling(t.clone())), ..Stroke::default() }),
		Value::Dict(d) => {
			let mut s = Stroke::default();
			for (k, v) in d.iter() {
				match k {
					"paint" => s.paint = Some(match v {
						Value::Color(c)		=> Paint::Color(*c),
						Value::Gradient(g)	=> Paint::Gradient(g.clone()),
						Value::Tiling(t)	=> Paint::Tiling(t.clone()),
						_					=> return None,
					}),
					"thickness" => s.thickness = Some(match v {
						Value::Length(l)	=> *l,
						_					=> return None,
					}),
					"cap" => s.cap = Some(match v {
						Value::Str(x) if x.as_str() == "butt"	=> LineCap::Butt,
						Value::Str(x) if x.as_str() == "round"	=> LineCap::Round,
						Value::Str(x) if x.as_str() == "square"	=> LineCap::Square,
						_										=> return None,
					}),
					"join" => s.join = Some(match v {
						Value::Str(x) if x.as_str() == "miter"	=> LineJoin::Miter,
						Value::Str(x) if x.as_str() == "round"	=> LineJoin::Round,
						Value::Str(x) if x.as_str() == "bevel"	=> LineJoin::Bevel,
						_										=> return None,
					}),
					"dash" => s.dash = Some(match to_dash(v) {
						Some(d)	=> d,
						None	=> return None,
					}),
					"miter-limit" => s.miter_limit = Some(match v {
						Value::Int(i)	=> *i as f64,
						Value::Float(f)	=> *f,
						_				=> return None,
					}),
					_ => return None,
				}
			}
			Some(s)
		}
		_ => None,
	}
}

/// A dash from `none`, a named pattern, an array of lengths and `"dot"`, or `(array:, phase:)`.
/// `Some(None)` is an explicit solid line.
fn to_dash(v: &Value) -> Option<Option<Dash>> {
	let pt = |x: f64| DashItem::Len(Length::pt(x));
	let pattern = |items: Vec<DashItem>| Some(Some(Dash { array: items, phase: Length::zero() }));
	match v {
		Value::None => Some(None),
		Value::Str(s) => match s.as_str() {
			"solid"					=> Some(None),
			"dotted"				=> pattern(vec![DashItem::Dot, pt(2.0)]),
			"densely-dotted"		=> pattern(vec![DashItem::Dot, pt(1.0)]),
			"loosely-dotted"		=> pattern(vec![DashItem::Dot, pt(4.0)]),
			"dashed"				=> pattern(vec![pt(3.0), pt(3.0)]),
			"densely-dashed"		=> pattern(vec![pt(3.0), pt(2.0)]),
			"loosely-dashed"		=> pattern(vec![pt(3.0), pt(6.0)]),
			"dash-dotted"			=> pattern(vec![pt(3.0), pt(2.0), DashItem::Dot, pt(2.0)]),
			"densely-dash-dotted"	=> pattern(vec![pt(3.0), pt(1.0), DashItem::Dot, pt(1.0)]),
			"loosely-dash-dotted"	=> pattern(vec![pt(3.0), pt(4.0), DashItem::Dot, pt(4.0)]),
			_						=> None,
		},
		Value::Array(a) => {
			let mut items = Vec::new();
			for x in a.iter() {
				items.push(match x {
					Value::Length(l)							=> DashItem::Len(*l),
					Value::Str(s) if s.as_str() == "dot"		=> DashItem::Dot,
					_											=> return None,
				});
			}
			pattern(items)
		}
		Value::Dict(d) => {
			let array = match d.get("array").map(to_dash) {
				Some(Some(Some(dash)))	=> dash.array,
				_						=> return None,
			};
			let phase = match d.get("phase") {
				Some(Value::Length(l))	=> *l,
				None					=> Length::zero(),
				_						=> return None,
			};
			Some(Some(Dash { array, phase }))
		}
		_ => None,
	}
}

fn merge_stroke(inner: Stroke, outer: Stroke) -> Stroke {
	Stroke {
		paint:			inner.paint.or(outer.paint),
		thickness:		inner.thickness.or(outer.thickness),
		cap:			inner.cap.or(outer.cap),
		join:			inner.join.or(outer.join),
		dash:			inner.dash.or(outer.dash),
		miter_limit:	inner.miter_limit.or(outer.miter_limit),
	}
}

// Set rules

/// The name Typst's cast errors use for a type: `integer`, `string`, not the `repr` names.
pub fn long_name(t: Type) -> &'static str {
	match t {
		Type::Int		=> "integer",
		Type::Str		=> "string",
		Type::Bool		=> "boolean",
		Type::Relative	=> "relative length",
		other			=> other.name(),
	}
}

/// "A, B, or C", as Typst lists the types a cast accepts.
fn list_types(ts: &[&str]) -> String {
	match ts.len() {
		0	=> String::new(),
		1	=> ts[0].to_string(),
		2	=> fmt!("{} or {}", ts[0], ts[1]),
		n	=> fmt!("{}, or {}", ts[..n - 1].join(", "), ts[n - 1]),
	}
}

/// The message for a value a field does not accept.
pub fn expected_message(ty: FieldType, found: &Value) -> String {
	let expected = match ty {
		FieldType::Any			=> "any value".to_string(),
		FieldType::Of(t)		=> long_name(t).to_string(),
		FieldType::OneOf(ts)	=> list_types(&ts.iter().map(|t| long_name(*t)).collect::<Vec<_>>()),
		FieldType::Content		=> "content".to_string(),
	};
	fmt!("expected {}, found {}", expected, long_name(found.ty()))
}

/// `set elem(args)`: the settable fields given, as styles, in schema order. A positional settable field
/// takes the first positional argument its type accepts; a named one the last argument of its name.
/// Anything left over -- a required field, a synthesised one, an unknown name -- is "unexpected argument".
pub fn set_rule(engine: &mut Engine, kind: ElemKind, mut args: Args) -> Outcome<Styles> {
	let mut styles = Styles::new();
	for (i, spec) in kind.fields().iter().enumerate() {
		if !spec.settable || spec.synthesised || spec.required {
			continue;
		}
		let id = FieldId(i as u8);
		let taken = if spec.positional {
			match args.items.iter().position(|a| a.name.is_none() && spec.ty.accepts(&a.value)) {
				Some(p)	=> Some(args.items.remove(p)),
				None	=> None,
			}
		} else {
			let mut found = None;
			let mut j = 0;
			while j < args.items.len() {
				if args.items[j].name.as_deref() == Some(spec.name) {
					found = Some(args.items.remove(j));
				} else {
					j += 1;
				}
			}
			found
		};
		if let Some(arg) = taken {
			if !spec.ty.accepts(&arg.value) {
				let msg = expected_message(spec.ty, &arg.value);
				return Err(engine.error(arg.span, msg));
			}
			styles.push(Style::Property(Property::new(kind, id, arg.value, arg.span)));
		}
	}
	if let Some(a) = args.items.first() {
		let span = if a.span.is_detached() { args.span } else { a.span };
		let msg = match &a.name {
			Some(n)	=> fmt!("unexpected argument: {}", n),
			None	=> "unexpected argument".to_string(),
		};
		return Err(engine.error(span, msg));
	}
	Ok(styles)
}

// Applying recipes

/// `content` under a recipe, as a `show` statement leaves the rest of its scope: a selector-less recipe
/// (`show: f`) is applied at once, as Typst does, and any other is attached as a style for realisation.
pub fn styled_with_recipe(engine: &mut Engine, content: Content, recipe: Recipe) -> Outcome<Content> {
	if recipe.selector.is_none() {
		let chain = match &engine.context.styles {
			Some(s)	=> s.clone(),
			None	=> StyleChain::root(),
		};
		return apply_recipe(engine, &recipe, content, &chain);
	}
	Ok(content.styled(Styles::from_style(Style::Recipe(recipe))))
}

/// Applies one recipe to its target. A function recipe runs with the target's location and `chain` as
/// its context and receives the target as its one argument; its result is displayed as content.
pub fn apply_recipe(engine: &mut Engine, recipe: &Recipe, target: Content, chain: &StyleChain) -> Outcome<Content> {
	match &recipe.transform {
		Transformation::Content(c)	=> Ok(c.clone()),
		Transformation::Style(s)	=> Ok(target.styled(s.clone())),
		Transformation::Func(f) => {
			let span = target.span();
			let name = match target.kind() {
				Some(k)	=> k.name(),
				None	=> "content",
			};
			let mut args = Args::new(recipe.span);
			args.push(span, Value::Content(target.clone()));
			let saved = std::mem::replace(&mut engine.context, Context {
				location:	target.location(),
				styles:		Some(chain.clone()),
			});
			let before = engine.diags.len();
			let result = engine.call_func(f, args);
			engine.context = saved;
			match result {
				Ok(v)	=> Ok(display(v)),
				Err(e)	=> {
					if recipe.selector.is_some() {
						let note = fmt!("error occurred while applying show rule to this {}", name);
						for d in engine.diags[before..].iter_mut().filter(|d| d.is_error()) {
							d.trace.push((span, note.clone()));
						}
					}
					Err(e)
				}
			}
		}
	}
}

/// A value as content, Typst's `Value::display`: content as is, `none` empty, a string or symbol as
/// text, a number as its `repr`, anything else as its `repr` in text.
pub fn display(v: Value) -> Content {
	match v {
		Value::Content(c)	=> c,
		Value::None			=> Content::empty(),
		Value::Str(s)		=> Content::text(&s),
		Value::Symbol(s)	=> Content::text(&symbol_text(&s)),
		other				=> Content::text(&foundations::repr(&other)),
	}
}

/// The text of a symbol under its applied modifiers: of the variants carrying every applied modifier,
/// the one with fewest modifiers, the first on a tie.
pub fn symbol_text(s: &Symbol) -> String {
	let applied: Vec<&str> = s.modifiers.split('.').filter(|m| !m.is_empty()).collect();
	let found = match &s.variants {
		SymVariants::Single(t)		=> Some(t.to_string()),
		SymVariants::Static(vs)		=> pick_variant(vs.iter().map(|(m, t)| (*m, *t)), &applied),
		SymVariants::Runtime(vs)	=> pick_variant(vs.iter().map(|(m, t)| (m.as_str(), t.as_str())), &applied),
	};
	found.unwrap_or_default()
}

fn pick_variant<'a, I: Iterator<Item = (&'a str, &'a str)>>(variants: I, applied: &[&str]) -> Option<String> {
	let mut best: Option<(usize, &'a str)> = None;
	for (mods, text) in variants {
		let ms: Vec<&str> = mods.split('.').filter(|m| !m.is_empty()).collect();
		if !applied.iter().all(|a| ms.contains(a)) {
			continue;
		}
		if best.map(|(n, _)| ms.len() < n).unwrap_or(true) {
			best = Some((ms.len(), text));
		}
	}
	best.map(|(_, t)| t.to_string())
}

/// Records an error with several hints and returns it.
pub(crate) fn error_hints(engine: &mut Engine, span: Span, message: &str, hints: &[&str]) -> Error<ErrTag> {
	let mut d = Diagnostic::error(span, message);
	for h in hints {
		d = d.with_hint(*h);
	}
	engine.diags.push(d);
	err!("{}", message; Input, Invalid)
}
