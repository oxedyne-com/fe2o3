// U0 owns this file. The `ElemKind` list is closed: an element Typst has and this list lacks is added
// here (one line in the table below) by agreement, never worked around. Each element's field schema,
// custom constructor and native show live with the unit that lays it out, reached through `Family`.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::lib;
use crate::eval::locate::Location;
use crate::eval::styles::{
	RecipeIndex,
	StyleChain,
	Styles,
};
use crate::eval::value::{
	FromValue,
	Label,
	Value,
	Type,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

/// Which unit's files hold an element's schema, constructor and native show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
	Text,	// lib/text.rs, U6a
	Model,	// lib/model/, U5
	Layout,	// lib/layout.rs, U6b
	Grid,	// lib/grid.rs, U6c
	Visual,	// lib/visual.rs, U6d
	Math,	// lib/math.rs, U7
	Intro,	// lib/intro.rs, U8
	Realise,	// realise.rs, U4: what realisation itself makes (tags) and the kinds of plain content
}

macro_rules! elem_kinds {
	($($v:ident => ($path:literal, $fam:ident, $loc:literal),)*) => {
		#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
		pub enum ElemKind {
			$($v,)*
		}

		impl ElemKind {
			pub const ALL: &'static [ElemKind] = &[$(ElemKind::$v,)*];

			/// The dotted path Typst code reaches the element function by: `heading`, `list.item`,
			/// `math.frac`.
			pub fn path(self) -> &'static str {
				match self {
					$(ElemKind::$v => $path,)*
				}
			}

			pub fn family(self) -> Family {
				match self {
					$(ElemKind::$v => Family::$fam,)*
				}
			}

			/// Does realisation give every instance a location, labelled or not?
			pub fn locatable(self) -> bool {
				match self {
					$(ElemKind::$v => $loc,)*
				}
			}
		}
	};
}

elem_kinds! {
	// Text (U6a)
	Text			=> ("text",					Text,	false),
	Space			=> ("space",				Text,	false),
	Linebreak		=> ("linebreak",			Text,	false),
	SmartQuote		=> ("smartquote",			Text,	false),
	Smallcaps		=> ("smallcaps",			Text,	false),
	Underline		=> ("underline",			Text,	true),
	Overline		=> ("overline",				Text,	true),
	Strike			=> ("strike",				Text,	true),
	Highlight		=> ("highlight",			Text,	true),
	Super			=> ("super",				Text,	false),
	Sub				=> ("sub",					Text,	false),
	Symbol			=> ("symbol",				Text,	false),
	// Model (U5)
	Par				=> ("par",					Model,	true),
	ParLine			=> ("par.line",				Model,	true),
	Parbreak		=> ("parbreak",				Model,	false),
	Strong			=> ("strong",				Model,	true),
	Emph			=> ("emph",					Model,	true),
	Raw				=> ("raw",					Model,	true),
	RawLine			=> ("raw.line",				Model,	false),
	Heading			=> ("heading",				Model,	true),
	Title			=> ("title",				Model,	true),
	List			=> ("list",					Model,	true),
	ListItem		=> ("list.item",			Model,	false),
	Enum			=> ("enum",					Model,	true),
	EnumItem		=> ("enum.item",			Model,	false),
	Terms			=> ("terms",				Model,	true),
	TermItem		=> ("terms.item",			Model,	false),
	Link			=> ("link",					Model,	true),
	Ref				=> ("ref",					Model,	true),
	Cite			=> ("cite",					Model,	true),
	Footnote		=> ("footnote",				Model,	true),
	FootnoteEntry	=> ("footnote.entry",		Model,	true),
	Figure			=> ("figure",				Model,	true),
	FigureCaption	=> ("figure.caption",		Model,	true),
	Outline			=> ("outline",				Model,	true),
	OutlineEntry	=> ("outline.entry",		Model,	true),
	Quote			=> ("quote",				Model,	true),
	Bibliography	=> ("bibliography",			Model,	true),
	CiteGroup		=> ("cite-group",			Model,	true),
	Document		=> ("document",				Model,	true),
	Divider			=> ("divider",				Model,	false),
	// Layout (U6b)
	Box				=> ("box",					Layout,	false),
	Block			=> ("block",				Layout,	false),
	Align			=> ("align",				Layout,	false),
	Pad				=> ("pad",					Layout,	false),
	Stack			=> ("stack",				Layout,	false),
	H				=> ("h",					Layout,	false),
	V				=> ("v",					Layout,	false),
	Place			=> ("place",				Layout,	true),
	PlaceFlush		=> ("place.flush",			Layout,	false),
	Columns			=> ("columns",				Layout,	false),
	Colbreak		=> ("colbreak",				Layout,	false),
	Pagebreak		=> ("pagebreak",			Layout,	false),
	Page			=> ("page",					Layout,	false),
	// Grid and table (U6c)
	Grid			=> ("grid",					Grid,	false),
	GridCell		=> ("grid.cell",			Grid,	false),
	GridHeader		=> ("grid.header",			Grid,	false),
	GridFooter		=> ("grid.footer",			Grid,	false),
	GridHLine		=> ("grid.hline",			Grid,	false),
	GridVLine		=> ("grid.vline",			Grid,	false),
	Table			=> ("table",				Grid,	true),
	TableCell		=> ("table.cell",			Grid,	false),
	TableHeader		=> ("table.header",			Grid,	false),
	TableFooter		=> ("table.footer",			Grid,	false),
	TableHLine		=> ("table.hline",			Grid,	false),
	TableVLine		=> ("table.vline",			Grid,	false),
	// Visual (U6d)
	Image			=> ("image",				Visual,	true),
	Line			=> ("line",					Visual,	false),
	Rect			=> ("rect",					Visual,	false),
	Square			=> ("square",				Visual,	false),
	Circle			=> ("circle",				Visual,	false),
	Ellipse			=> ("ellipse",				Visual,	false),
	Polygon			=> ("polygon",				Visual,	false),
	Curve			=> ("curve",				Visual,	false),
	CurveMove		=> ("curve.move",			Visual,	false),
	CurveLine		=> ("curve.line",			Visual,	false),
	CurveQuad		=> ("curve.quad",			Visual,	false),
	CurveCubic		=> ("curve.cubic",			Visual,	false),
	CurveClose		=> ("curve.close",			Visual,	false),
	Path			=> ("path",					Visual,	false),
	Move			=> ("move",					Visual,	false),
	Scale			=> ("scale",				Visual,	false),
	Rotate			=> ("rotate",				Visual,	false),
	Skew			=> ("skew",					Visual,	false),
	Hide			=> ("hide",					Visual,	false),
	Repeat			=> ("repeat",				Visual,	false),
	// Maths (U7)
	Equation		=> ("math.equation",		Math,	true),
	MathAlignPoint	=> ("math.align-point",		Math,	false),
	MathAttach		=> ("math.attach",			Math,	false),
	MathScripts		=> ("math.scripts",			Math,	false),
	MathLimits		=> ("math.limits",			Math,	false),
	MathPrimes		=> ("math.primes",			Math,	false),
	MathFrac		=> ("math.frac",			Math,	false),
	MathBinom		=> ("math.binom",			Math,	false),
	MathLr			=> ("math.lr",				Math,	false),
	MathMid			=> ("math.mid",				Math,	false),
	MathMat			=> ("math.mat",				Math,	false),
	MathVec			=> ("math.vec",				Math,	false),
	MathCases		=> ("math.cases",			Math,	false),
	MathRoot		=> ("math.root",			Math,	false),
	MathAccent		=> ("math.accent",			Math,	false),
	MathOp			=> ("math.op",				Math,	false),
	MathClass		=> ("math.class",			Math,	false),
	MathCancel		=> ("math.cancel",			Math,	false),
	MathStretch		=> ("math.stretch",			Math,	false),
	MathUnderline	=> ("math.underline",		Math,	false),
	MathOverline	=> ("math.overline",		Math,	false),
	MathUnderbrace	=> ("math.underbrace",		Math,	false),
	MathOverbrace	=> ("math.overbrace",		Math,	false),
	MathUnderbracket	=> ("math.underbracket",	Math,	false),
	MathOverbracket	=> ("math.overbracket",		Math,	false),
	MathUnderparen	=> ("math.underparen",		Math,	false),
	MathOverparen	=> ("math.overparen",		Math,	false),
	MathUndershell	=> ("math.undershell",		Math,	false),
	MathOvershell	=> ("math.overshell",		Math,	false),
	// Introspection (U8)
	Metadata		=> ("metadata",				Intro,	true),
	CounterUpdate	=> ("counter.update",		Intro,	true),
	StateUpdate		=> ("state.update",			Intro,	true),
	Context			=> ("context",				Intro,	true),
	Layout			=> ("layout",				Intro,	true),
	// Realisation (U4)
	Sequence		=> ("sequence",				Realise,	false),
	Styled			=> ("styled",				Realise,	false),
	Tag				=> ("tag",					Realise,	false),
}

impl ElemKind {
	/// The last path segment, what `repr` of the element function prints (`item` for `list.item`).
	pub fn name(self) -> &'static str {
		let p = self.path();
		match p.rfind('.') {
			Some(i)	=> &p[i + 1..],
			None	=> p,
		}
	}

	/// The element reached as a top-level global (`heading`), not through a parent (`list.item`) or the
	/// `math` module (`math.frac`). Typst binds no global to `space`, to its internal cite groups, tags,
	/// sequences and styled content, keeps `context` as a keyword, and gives the names `symbol` and `path`
	/// to types, so those elements are reached only as values.
	pub fn is_global(self) -> bool {
		!self.path().contains('.') && !matches!(self,
			ElemKind::Space | ElemKind::Symbol | ElemKind::Path | ElemKind::Context | ElemKind::CiteGroup
				| ElemKind::Sequence | ElemKind::Styled | ElemKind::Tag)
	}

	/// Can a user's `query`, `locate` or selector name the element? Typst locates `place` for the flow's
	/// float bookkeeping, yet refuses it in a query: "place is not locatable".
	pub fn queryable(self) -> bool {
		self.locatable() && !matches!(self, ElemKind::Place)
	}

	/// The element scoped under this one's function: `ElemKind::List.scoped("item")` is `ListItem`.
	pub fn scoped(self, name: &str) -> Option<ElemKind> {
		let parent = self.path();
		ElemKind::ALL.iter().copied().find(|k| {
			let p = k.path();
			p.len() == parent.len() + 1 + name.len()
				&& p.starts_with(parent)
				&& p.as_bytes().get(parent.len()) == Some(&b'.')
				&& p.ends_with(name)
		})
	}

	/// The element's field schema, from the owning unit.
	pub fn fields(self) -> &'static [FieldSpec] {
		match self.family() {
			Family::Text	=> lib::text::fields(self),
			Family::Model	=> lib::model::fields(self),
			Family::Layout	=> lib::layout::fields(self),
			Family::Grid	=> lib::grid::fields(self),
			Family::Visual	=> lib::visual::fields(self),
			Family::Math	=> lib::math::fields(self),
			Family::Intro	=> lib::intro::fields(self),
			Family::Realise	=> crate::eval::realise::fields(self),
		}
	}

	pub fn field_id(self, name: &str) -> Option<FieldId> {
		self.fields().iter().position(|f| f.name == name).map(|i| FieldId(i as u8))
	}

	pub fn field_spec(self, id: FieldId) -> Option<&'static FieldSpec> {
		self.fields().get(id.0 as usize)
	}
}

// Field schema

/// A field's index into its element's schema slice. Stable for a build, never serialised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldId(pub u8);

/// What a field accepts. `Any` defers checking to the owning unit's typed accessor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
	Any,
	Of(Type),
	OneOf(&'static [Type]),
	Content,	// content, str, symbol or number, displayed
}

impl FieldType {
	pub fn accepts(self, v: &Value) -> bool {
		match self {
			FieldType::Any			=> true,
			FieldType::Of(t)		=> v.ty() == t,
			FieldType::OneOf(ts)	=> ts.contains(&v.ty()),
			FieldType::Content		=> matches!(v.ty(),
				Type::Content | Type::Str | Type::Symbol | Type::Int | Type::Float | Type::None),
		}
	}
}

/// A default a `static` schema can hold. `Computed` means the owning unit resolves it in code (Typst's
/// heading `numbering` depends on nothing static, `text.lang`'s region on the language, and so on).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldDefault {
	None,
	Auto,
	Bool(bool),
	Int(i64),
	Float(f64),
	Pt(f64),
	Em(f64),
	Ratio(f64),
	Str(&'static str),
	EmptyContent,
	EmptyArray,
	Computed,
	Required,	// no default: construction fails without it
}

/// How successive `set` values of a field combine along the style chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
	Replace,				// innermost wins
	Add,					// accumulates, e.g. `text.size` em-relative
	Sides,					// side by side, each side's stroke part-wise (`inset`, `margin`, `rect.stroke`)
	Corners,				// corner by corner (`radius`)
	Stroke,					// one stroke, part-wise (`line.stroke`)
	Keyed(&'static str),	// a dictionary whose bare-value form is this key (`first-line-indent`'s amount)
	Custom,					// the owning unit folds it in code
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldSpec {
	pub name:			&'static str,
	pub ty:				FieldType,
	pub default:		FieldDefault,
	pub fold:			Fold,
	pub positional:		bool,
	pub required:		bool,
	pub variadic:		bool,	// takes all remaining positional arguments (`list(..children)`)
	pub settable:		bool,
	pub synthesised:	bool,	// filled during realisation, never passed (`heading.level` when auto, `counter`)
	pub internal:		bool,	// Typst-internal: hidden from `fields()`, field access, `repr`, `set` and construction
}

impl FieldSpec {
	/// A settable, named, optional field: the common case.
	pub const fn named(name: &'static str, ty: FieldType, default: FieldDefault) -> Self {
		Self { name, ty, default, fold: Fold::Replace, positional: false, required: false,
			variadic: false, settable: true, synthesised: false, internal: false }
	}

	/// A required positional field, not settable: a body, a text, a destination.
	pub const fn required(name: &'static str, ty: FieldType) -> Self {
		Self { name, ty, default: FieldDefault::Required, fold: Fold::Replace, positional: true,
			required: true, variadic: false, settable: false, synthesised: false, internal: false }
	}

	pub const fn fold(mut self, fold: Fold) -> Self { self.fold = fold; self }
	pub const fn positional(mut self) -> Self { self.positional = true; self }
	pub const fn variadic(mut self) -> Self { self.variadic = true; self.positional = true; self }
	pub const fn unsettable(mut self) -> Self { self.settable = false; self }
	pub const fn synthesised(mut self) -> Self { self.synthesised = true; self.settable = false; self }
	pub const fn internal(mut self) -> Self { self.internal = true; self.settable = false; self }
}

impl FieldDefault {
	/// The default as a value, `None` for one only code can compute or a required field.
	pub fn to_value(self) -> Option<Value> {
		use crate::eval::value::{
			Length,
			Ratio,
		};
		match self {
			FieldDefault::None			=> Some(Value::None),
			FieldDefault::Auto			=> Some(Value::Auto),
			FieldDefault::Bool(b)		=> Some(Value::Bool(b)),
			FieldDefault::Int(i)		=> Some(Value::Int(i)),
			FieldDefault::Float(f)		=> Some(Value::Float(f)),
			FieldDefault::Pt(p)			=> Some(Value::Length(Length::pt(p))),
			FieldDefault::Em(e)			=> Some(Value::Length(Length::em(e))),
			FieldDefault::Ratio(r)		=> Some(Value::Ratio(Ratio(r))),
			FieldDefault::Str(s)		=> Some(Value::str(s)),
			FieldDefault::EmptyContent	=> Some(Value::Content(Content::empty())),
			FieldDefault::EmptyArray	=> Some(Value::array(Vec::new())),
			FieldDefault::Computed		=> None,
			FieldDefault::Required		=> None,
		}
	}
}

// Content

/// One element instance. `guards` are the recipes already applied to it, Typst's defence against a
/// show rule re-matching its own output.
#[derive(Clone, Debug)]
pub struct Elem {
	pub kind:		ElemKind,
	pub fields:		Vec<(FieldId, Value)>,
	pub label:		Option<Label>,
	pub location:	Option<Location>,
	pub span:		Span,
	pub guards:		Vec<RecipeIndex>,
	pub prepared:	bool,	// synthesised fields filled and location assigned
}

/// Content in order. A labelled sequence is located and guarded as an element is, so show rules on
/// its label apply once and it can be found by its location.
#[derive(Clone, Debug)]
pub struct Sequence {
	pub children:	Vec<Content>,
	pub label:		Option<Label>,
	pub span:		Span,
	pub location:	Option<Location>,
	pub guards:		Vec<RecipeIndex>,
}

impl Sequence {
	pub fn new(children: Vec<Content>) -> Self {
		Self { children, label: None, span: Span::detached(), location: None, guards: Vec::new() }
	}
}

#[derive(Clone, Debug)]
pub struct Styled {
	pub child:	Content,
	pub styles:	Styles,
}

/// Typst's `content` value: an element, a sequence of content, or content under local styles.
#[derive(Clone, Debug)]
pub enum Content {
	Elem(Arc<Elem>),
	Sequence(Arc<Sequence>),
	Styled(Arc<Styled>),
}

impl Default for Content {
	fn default() -> Self { Content::empty() }
}

impl Content {
	pub fn empty() -> Self {
		Content::Sequence(Arc::new(Sequence::new(Vec::new())))
	}

	pub fn new(kind: ElemKind, fields: Vec<(FieldId, Value)>, span: Span) -> Self {
		Content::Elem(Arc::new(Elem {
			kind, fields, label: None, location: None, span, guards: Vec::new(), prepared: false,
		}))
	}

	/// A text element. By contract `text`'s field 0 is its string (`lib/text.rs`).
	pub fn text(s: &str) -> Self {
		Content::new(ElemKind::Text, vec![(FieldId(0), Value::str(s))], Span::detached())
	}

	/// A symbol element, what Typst makes of a symbol value, an escape, a shorthand or one maths
	/// character. By contract `symbol`'s field 0 is its text (`lib/text.rs`).
	pub fn symbol(s: &str) -> Self {
		Content::new(ElemKind::Symbol, vec![(FieldId(0), Value::str(s))], Span::detached())
	}

	/// An element with no fields set, for markers such as `parbreak()` and `linebreak()`.
	pub fn marker(kind: ElemKind, span: Span) -> Self { Content::new(kind, Vec::new(), span) }

	pub fn sequence(children: Vec<Content>) -> Self {
		if children.len() == 1 {
			if let Some(c) = children.into_iter().next() {
				return c;
			}
			return Content::empty();
		}
		Content::Sequence(Arc::new(Sequence::new(children)))
	}

	pub fn styled(self, styles: Styles) -> Self {
		if styles.is_empty() {
			return self;
		}
		Content::Styled(Arc::new(Styled { child: self, styles }))
	}

	pub fn kind(&self) -> Option<ElemKind> {
		match self {
			Content::Elem(e)	=> Some(e.kind),
			_					=> None,
		}
	}

	pub fn is(&self, kind: ElemKind) -> bool { self.kind() == Some(kind) }

	pub fn elem(&self) -> Option<&Elem> {
		match self {
			Content::Elem(e)	=> Some(e),
			_					=> None,
		}
	}

	pub fn is_empty(&self) -> bool {
		match self {
			Content::Sequence(s)	=> s.children.iter().all(|c| c.is_empty()),
			Content::Styled(s)		=> s.child.is_empty(),
			Content::Elem(_)		=> false,
		}
	}

	pub fn span(&self) -> Span {
		match self {
			Content::Elem(e)		=> e.span,
			Content::Sequence(s)	=> s.span,
			Content::Styled(s)		=> s.child.span(),
		}
	}

	pub fn with_span(mut self, span: Span) -> Self {
		match &mut self {
			Content::Elem(e)		=> Arc::make_mut(e).span = span,
			Content::Sequence(s)	=> Arc::make_mut(s).span = span,
			Content::Styled(_)		=> (),
		}
		self
	}

	pub fn label(&self) -> Option<&Label> {
		match self {
			Content::Elem(e)		=> e.label.as_ref(),
			Content::Sequence(s)	=> s.label.as_ref(),
			Content::Styled(s)		=> s.child.label(),
		}
	}

	pub fn labelled(mut self, label: Label) -> Self {
		match &mut self {
			Content::Elem(e)		=> Arc::make_mut(e).label = Some(label),
			Content::Sequence(s)	=> Arc::make_mut(s).label = Some(label),
			Content::Styled(s)		=> {
				let st = Arc::make_mut(s);
				st.child = st.child.clone().labelled(label);
			}
		}
		self
	}

	pub fn location(&self) -> Option<Location> {
		match self {
			Content::Elem(e)		=> e.location,
			Content::Sequence(s)	=> s.location,
			Content::Styled(_)		=> None,
		}
	}

	/// The kind `content.func()` reports: the element's, or `sequence` and `styled` for plain content.
	pub fn func_kind(&self) -> ElemKind {
		match self {
			Content::Elem(e)		=> e.kind,
			Content::Sequence(_)	=> ElemKind::Sequence,
			Content::Styled(_)		=> ElemKind::Styled,
		}
	}

	/// A field's stored value, `None` when unset (not the default; styles and schema supply that).
	pub fn get(&self, id: FieldId) -> Option<&Value> {
		match self {
			Content::Elem(e)	=> e.fields.iter().find(|(f, _)| *f == id).map(|(_, v)| v),
			_					=> None,
		}
	}

	pub fn get_as<T: FromValue>(&self, id: FieldId) -> Outcome<Option<T>> {
		match self.get(id) {
			Some(v)	=> Ok(Some(res!(T::from_value(v.clone())))),
			None	=> Ok(None),
		}
	}

	/// A field by name, as `it.body` reads it.
	pub fn field(&self, name: &str) -> Option<&Value> {
		match self {
			Content::Elem(e)	=> e.kind.field_id(name).and_then(|id| self.get(id)),
			_					=> None,
		}
	}

	/// Sets a field in place (copy on write); a no-op on a sequence or styled content.
	pub fn set(&mut self, id: FieldId, value: Value) {
		if let Content::Elem(e) = self {
			let e = Arc::make_mut(e);
			match e.fields.iter_mut().find(|(f, _)| *f == id) {
				Some(slot)	=> slot.1 = value,
				None		=> e.fields.push((id, value)),
			}
		}
	}

	/// Direct children of a sequence or styled content; an element's body is a field, not a child.
	pub fn children(&self) -> &[Content] {
		match self {
			Content::Sequence(s)	=> &s.children,
			Content::Styled(s)		=> std::slice::from_ref(&s.child),
			Content::Elem(_)		=> &[],
		}
	}

	/// The text a reader would see, ignoring styles, as Typst's `plain-text` does for text, spaces and
	/// breaks. Other elements contribute their `body` or `text` field when they have one.
	pub fn plain_text(&self) -> String {
		let mut s = String::new();
		self.push_plain(&mut s);
		s
	}

	fn push_plain(&self, s: &mut String) {
		match self {
			Content::Sequence(seq)	=> for c in &seq.children { c.push_plain(s); },
			Content::Styled(st)		=> st.child.push_plain(s),
			Content::Elem(e) => match e.kind {
				ElemKind::Text | ElemKind::Symbol => if let Some(Value::Str(t)) = self.get(FieldId(0)) {
					s.push_str(t);
				},
				ElemKind::Space | ElemKind::Linebreak	=> s.push(' '),
				ElemKind::Parbreak						=> s.push_str("\n\n"),
				_ => match self.field("body").or_else(|| self.field("text")) {
					Some(Value::Content(c))	=> c.push_plain(s),
					Some(Value::Str(t))		=> s.push_str(t),
					_						=> (),
				},
			},
		}
	}
}

/// Folds a `Fold::Custom` field's inner value onto its outer one, by the family that declared it. A
/// family declaring a `Custom` field gains an arm here with its fold.
pub fn fold_custom(kind: ElemKind, field: &str, inner: Value, outer: Value) -> Outcome<Value> {
	match kind.family() {
		Family::Visual	=> lib::visual::fold(kind, field, inner, outer),
		Family::Model	=> lib::model::fold(kind, field, inner, outer),
		_				=> Err(err!(
			"`{}.{}` is declared `Fold::Custom`, but its family has no fold in `content::fold_custom`",
			kind.path(), field; Unimplemented)),
	}
}

/// Typst's `Value::display`, how a value shows when placed in markup: numbers and versions as their
/// `repr` in text, a string as text, a symbol as a symbol element, a module as its content, and
/// anything else as its `repr` in inline `typc` raw.
pub fn display(engine: &mut Engine, v: Value, span: Span) -> Outcome<Content> {
	Ok(match v {
		Value::None			=> Content::empty(),
		// Numbers show with a typographic minus and a decimal as its digits, not as their `repr`.
		Value::Int(_) | Value::Float(_) | Value::Decimal(_) | Value::Version(_)
							=> Content::text(&lib::foundations::display(&v).unwrap_or_default()).with_span(span),
		Value::Str(s)		=> Content::text(&s).with_span(span),
		Value::Symbol(s)	=> Content::symbol(&crate::eval::ops::symbol_text(&s)).with_span(span),
		Value::Content(c)	=> if c.span().is_detached() { c.with_span(span) } else { c },
		Value::Module(m)	=> m.content.clone(),
		other				=> {
			let text = lib::foundations::repr(&other);
			res!(build(engine, ElemKind::Raw, vec![
				("text",	Value::str(text)),
				("block",	Value::Bool(false)),
				("lang",	Value::str("typc")),
			], span))
		}
	})
}

/// Builds an element with its fields named as Typst names them. A name the owning unit's schema lacks
/// is a contract fault, reported rather than dropped.
pub fn build(engine: &mut Engine, kind: ElemKind, fields: Vec<(&str, Value)>, span: Span) -> Outcome<Content> {
	let mut fv = Vec::with_capacity(fields.len());
	for (name, v) in fields {
		match kind.field_id(name) {
			Some(id)	=> fv.push((id, v)),
			None		=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
				"element `{}` has no field `{}` in its schema", kind.path(), name))),
		}
	}
	Ok(Content::new(kind, fv, span))
}

// Family dispatch

/// Builds an element from call arguments: the owning unit's custom constructor if it has one, else the
/// generic schema walk (required positionals, optional positionals, variadics, then named fields).
pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Content> {
	let custom = match kind.family() {
		Family::Text	=> res!(lib::text::construct(engine, kind, args)),
		Family::Model	=> res!(lib::model::construct(engine, kind, args)),
		Family::Layout	=> res!(lib::layout::construct(engine, kind, args)),
		Family::Grid	=> res!(lib::grid::construct(engine, kind, args)),
		Family::Visual	=> res!(lib::visual::construct(engine, kind, args)),
		Family::Math	=> res!(lib::math::construct(engine, kind, args)),
		Family::Intro	=> res!(lib::intro::construct(engine, kind, args)),
		Family::Realise	=> res!(crate::eval::realise::construct(engine, kind, args)),
	};
	if let Some(c) = custom {
		return Ok(c);
	}
	let span = args.span;
	let mut fields = Vec::new();
	for (i, spec) in kind.fields().iter().enumerate() {
		let id = FieldId(i as u8);
		if spec.synthesised || spec.internal {
			continue;
		}
		let value = if spec.variadic {
			Some(Value::array(res!(args.all::<Value>())))
		} else if spec.positional && spec.required {
			Some(res!(args.expect::<Value>(spec.name)))
		} else if spec.positional {
			res!(args.find::<Value>(|v| spec.ty.accepts(v)))
		} else {
			res!(args.named::<Value>(spec.name))
		};
		if let Some(v) = value {
			if !spec.ty.accepts(&v) {
				return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"{}: field `{}` does not accept {}", kind.path(), spec.name, v.ty().name())));
			}
			fields.push((id, res!(cast_field(kind, spec.name, v))));
		}
	}
	res!(std::mem::take(args).finish());
	Ok(Content::new(kind, fields, span))
}

/// The element's native show: its default realisation into other elements, or `None` for a primitive
/// that flow lays out itself.
pub fn native_show(engine: &mut Engine, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(None),
	};
	match kind.family() {
		Family::Text	=> lib::text::show(engine, elem, styles),
		Family::Model	=> lib::model::show(engine, elem, styles),
		Family::Layout	=> lib::layout::show(engine, elem, styles),
		Family::Grid	=> lib::grid::show(engine, elem, styles),
		Family::Visual	=> lib::visual::show(engine, elem, styles),
		Family::Math	=> lib::math::show(engine, elem, styles),
		Family::Intro	=> lib::intro::show(engine, elem, styles),
		Family::Realise	=> Ok(None),
	}
}

/// Casts a value into an element's field as its family casts it. Construction and `set` rules both
/// cast through here, so a value one accepts the other accepts. A family whose fields need no more
/// than their schema type takes the value as it is.
pub fn cast_field(kind: ElemKind, name: &str, v: Value) -> Outcome<Value> {
	match kind.family() {
		Family::Visual	=> lib::visual::cast_field(kind, name, v),
		Family::Model	=> lib::model::cast_field(kind, name, v),
		_				=> Ok(v),
	}
}

/// An element's built-in show-set styles, Typst's `ShowSet`: applied outside the user's own show-set
/// rules when the element is prepared, so a heading's weight is visible to `show heading: it => ..`
/// and a user rule still overrides it. The styles may depend on the element's own fields (a heading's
/// level), so the element is passed. A family whose elements have them gains an arm here.
pub fn show_set(elem: &Content, styles: &StyleChain) -> Outcome<Styles> {
	let kind = match elem.kind() {
		Some(k)	=> k,
		None	=> return Ok(Styles::new()),
	};
	match kind.family() {
		Family::Model	=> lib::model::show_set(elem, styles),
		Family::Text | Family::Layout | Family::Grid | Family::Visual | Family::Math
			| Family::Intro | Family::Realise	=> Ok(Styles::new()),
	}
}

/// Fills an element's synthesised fields once its styles are known, Typst's `Synthesize`: a heading's
/// resolved numbering, a figure's kind and supplement. A family with such fields gains an arm here.
pub fn synthesise(engine: &mut Engine, elem: &mut Content, styles: &StyleChain) -> Outcome<()> {
	let family = match elem.kind() {
		Some(k)	=> k.family(),
		None	=> return Ok(()),
	};
	match family {
		Family::Model	=> lib::model::synthesise(engine, elem, styles),
		Family::Text | Family::Layout | Family::Grid | Family::Visual | Family::Math
			| Family::Intro | Family::Realise	=> Ok(()),
	}
}
