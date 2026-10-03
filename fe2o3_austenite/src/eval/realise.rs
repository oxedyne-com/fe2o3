// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-realize, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U4 owns this file: content plus styles in, an ordered element stream out, with show rules applied,
// locations assigned and inline runs grouped. Flow calls it again for each container's body, so one call
// realises one level, not the whole tree.
//
// A port of Typst 0.15's `typst-realize`: show-rule verdicts with guards, preparation (location, built-in
// show-set styles, materialised and synthesised fields), text and regex rules over merged text runs, and
// the grouping rules (textual, par, cites, list, enum, terms) with their priorities and interruptions.
// A location tag is a `Pair` with `tag` set in the stream and a `tag` element in content, so tags inside
// a paragraph stay in its body; tags inside a list or a cite group are lifted out of it (a start before
// the group element, an end after it). This file also answers for the `Realise` family: `tag`,
// `sequence` and `styled`, the kinds of what realisation makes and of plain content.

use crate::diag::DiagnosticKind;
use crate::eval::args::Args;
use crate::eval::content::{
	self,
	native_show,
	Content,
	ElemKind,
	Family,
	FieldDefault,
	FieldId,
	FieldSpec,
	FieldType,
};
use crate::eval::locate::Location;
use crate::eval::styles::{
	apply_recipe,
	error_hints,
	Recipe,
	RecipeIndex,
	Style,
	StyleChain,
	Styles,
	Transformation,
};
use crate::eval::value::{
	Label,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

pub const MAX_SHOW_RULE_DEPTH:	usize	= 64;	// Typst's own limit
pub const MAX_GROUPING_STEPS:	usize	= 512;	// ditto, for groups that keep producing groups

/// What the caller will do with the stream, which decides grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealiseMode {
	Document,	// page elements and pagebreaks surface; everything else as in Flow
	Flow,		// block-level: inline runs grouped into `par`, list items into `list`/`enum`/`terms`
	Inline,		// a paragraph's children: text runs merged, text/regex show rules applied
	Math,		// an equation's body
}

/// Where a located element starts and ends in the stream, for the ledger and the introspector. `Start`
/// carries the element as prepared (location set, fields materialised).
#[derive(Clone, Debug)]
pub enum Tag {
	Start(Content),
	End(Location),
}

impl Tag {
	/// The tag as a `tag` element, how it sits inside content such as a paragraph's body.
	pub fn to_content(&self) -> Content {
		match self {
			Tag::Start(c)	=> Content::new(ElemKind::Tag, vec![(TAG_START, Value::Content(c.clone()))], c.span()),
			Tag::End(l)		=> Content::new(ElemKind::Tag, vec![(TAG_END, Value::Location(*l))], Span::detached()),
		}
	}

	/// The tag a `tag` element carries.
	pub fn from_content(c: &Content) -> Option<Tag> {
		if !c.is(ElemKind::Tag) {
			return None;
		}
		match (c.get(TAG_START), c.get(TAG_END)) {
			(Some(Value::Content(e)), _)	=> Some(Tag::Start(e.clone())),
			(_, Some(Value::Location(l)))	=> Some(Tag::End(*l)),
			_								=> None,
		}
	}
}

// The `Realise` family's schemas.
const TAG_START:	FieldId = FieldId(0);
const TAG_END:		FieldId = FieldId(1);

const TAG: &[FieldSpec] = &[
	FieldSpec::named("start",	FieldType::Of(Type::Content),	FieldDefault::None).unsettable(),
	FieldSpec::named("end",		FieldType::Of(Type::Location),	FieldDefault::None).unsettable(),
];
const SEQUENCE: &[FieldSpec] = &[
	FieldSpec::required("children", FieldType::Of(Type::Array)),
];
const STYLED: &[FieldSpec] = &[
	FieldSpec::required("child", FieldType::Content),
	FieldSpec::required("styles", FieldType::Of(Type::Styles)),
];

pub fn fields(kind: ElemKind) -> &'static [FieldSpec] {
	match kind {
		ElemKind::Tag		=> TAG,
		ElemKind::Sequence	=> SEQUENCE,
		ElemKind::Styled	=> STYLED,
		_					=> &[],
	}
}

/// `sequence(children)` and `styled(child, styles)` build plain content, as the functions
/// `content.func()` returns for it do in Typst; a tag is made only by realisation.
pub fn construct(engine: &mut Engine, kind: ElemKind, args: &mut Args) -> Outcome<Option<Content>> {
	let span = args.span;
	let built = match kind {
		ElemKind::Sequence => {
			let items = res!(args.expect::<Value>("children"));
			let items = match items {
				Value::Array(a)	=> a,
				other			=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"expected array, found {}", other.ty().long_name()))),
			};
			let mut children = Vec::with_capacity(items.len());
			for v in items.iter() {
				match v {
					Value::Content(c)	=> children.push(c.clone()),
					other				=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
						"expected content, found {}", other.ty().long_name()))),
				}
			}
			Content::sequence(children)
		}
		ElemKind::Styled => {
			let child = res!(args.expect::<Value>("child"));
			let styles = res!(args.expect::<Value>("styles"));
			let child = res!(content::display(engine, child, span));
			match styles {
				Value::Styles(s)	=> child.styled(s),
				other				=> return Err(engine.error(DiagnosticKind::Type, span, fmt!(
					"expected styles, found {}", other.ty().long_name()))),
			}
		}
		_ => return Err(engine.error(DiagnosticKind::Type, span, fmt!("`{}` is made by realisation, not called", kind.path()))),
	};
	res!(std::mem::take(args).finish());
	Ok(Some(built))
}

/// One realised element and the styles it is laid out under. A tag pair has empty content and `tag` set;
/// flow emits a ledger anchor for it and lays nothing out.
#[derive(Clone, Debug)]
pub struct Pair {
	pub content:	Content,
	pub styles:		StyleChain,
	pub tag:		Option<Tag>,
}

impl Pair {
	pub fn new(content: Content, styles: StyleChain) -> Self { Self { content, styles, tag: None } }

	pub fn tag(tag: Tag, styles: StyleChain) -> Self { Self { content: Content::empty(), styles, tag: Some(tag) } }

	pub fn is_tag(&self) -> bool { self.tag.is_some() }
}

/// Does a realised document set any content? Spaces, paragraph breaks, page breaks and column breaks do not,
/// and a tag only marks a place; any other element does, though it draws nothing, as a vertical space or an
/// empty box does. A `context` that gives nothing, a `set` rule and a `let` leave no element, and the furniture
/// a `set page` names is laid out beside the body and is no part of it.
pub fn sets_content(pairs: &[Pair]) -> bool {
	pairs.iter().any(|p| !p.is_tag() && !matches!(
		p.content.kind(),
		Some(ElemKind::Space | ElemKind::Parbreak | ElemKind::Pagebreak | ElemKind::Colbreak),
	))
}

/// Realises `content` under `styles`. In `Flow` mode a body that is inline only -- no paragraph break,
/// no block-level element -- comes back ungrouped, as Typst's fragment realisation leaves it: flow sets
/// it as one paragraph without a `par` element, so `show par` rules do not reach it.
pub fn realise(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	mode:		RealiseMode,
)
	-> Outcome<Vec<Pair>>
{
	Ok(res!(realise_with(engine, content, styles, mode, false)).0)
}

/// Realises a body in `Flow` mode and says whether it is one paragraph's worth of inline elements left without
/// a `par`: Typst's `FragmentKind::Inline`. Text with spaces between its runs (`A: B`) is such a body, so the
/// caller cannot re-derive the verdict from the pairs alone.
pub fn realise_fragment(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
)
	-> Outcome<(Vec<Pair>, bool)>
{
	realise_with(engine, content, styles, RealiseMode::Flow, false)
}

/// Realises as `realise` does, but leaves model elements (headings, paragraphs, lists, strong and
/// emphasised text, figures and the like) standing where no user show rule replaces them, instead of
/// expanding them by their built-in show, as Typst's HTML export keeps them. The structure a reader
/// sees can then be compared with that export.
pub fn realise_structure(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	mode:		RealiseMode,
)
	-> Outcome<Vec<Pair>>
{
	Ok(res!(realise_with(engine, content, styles, mode, true)).0)
}

fn realise_with(
	engine:		&mut Engine,
	content:	&Content,
	styles:		&StyleChain,
	mode:		RealiseMode,
	keep_model:	bool,
)
	-> Outcome<(Vec<Pair>, bool)>
{
	let mut s = State {
		engine,
		mode,
		keep_model,
		sink:			Vec::new(),
		groupings:		Vec::new(),
		outside:		mode == RealiseMode::Document,
		may_attach:		false,
		saw_parbreak:	false,
		fully_inline:	false,
		depth:			0,
	};
	res!(s.visit(content, styles));
	res!(s.finish());
	Ok((s.sink, s.fully_inline))
}

/// Is the element set inline, within a paragraph, rather than as a block of its own?
pub fn is_inline(content: &Content) -> bool {
	Rule::Par.trigger(content, false)
}

// Grouping rules

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
	Textual,
	Par,
	Cites,
	List,
	Enum,
	Terms,
}

impl Rule {
	fn priority(self) -> u8 {
		match self {
			Rule::Textual						=> 3,
			Rule::Cites | Rule::List | Rule::Enum | Rule::Terms	=> 2,
			Rule::Par							=> 1,
		}
	}

	/// Does the element open (or continue) such a group? When the model stands unexpanded (`keep`), strong
	/// text, emphasis and a link are phrasing content, as Typst's HTML export groups them, so they stay
	/// in their paragraph instead of ending it.
	fn trigger(self, c: &Content, keep: bool) -> bool {
		let k = match c.kind() {
			Some(k)	=> k,
			None	=> return false,
		};
		match self {
			Rule::Textual	=> matches!(k, ElemKind::Text | ElemKind::Linebreak | ElemKind::SmartQuote),
			Rule::Par		=> match k {
				ElemKind::Text | ElemKind::H | ElemKind::Linebreak | ElemKind::SmartQuote
					| ElemKind::Box	=> true,
				ElemKind::Strong | ElemKind::Emph | ElemKind::Link	=> keep,
				// An inline equation; a block one is `block: true`.
				ElemKind::Equation	=> !matches!(c.field("block"), Some(Value::Bool(true))),
				_					=> false,
			},
			Rule::Cites	=> k == ElemKind::Cite,
			Rule::List	=> k == ElemKind::ListItem,
			Rule::Enum	=> k == ElemKind::EnumItem,
			Rule::Terms	=> k == ElemKind::TermItem,
		}
	}

	/// May the element sit inside such a group without opening one?
	fn inner(self, c: &Content) -> bool {
		match self {
			Rule::Textual | Rule::Par | Rule::Cites	=> c.is(ElemKind::Space),
			Rule::List | Rule::Enum | Rule::Terms	=> c.is(ElemKind::Space) || c.is(ElemKind::Parbreak),
		}
	}

	/// Do styles on this element end such a group?
	fn interrupted_by(self, k: ElemKind) -> bool {
		match self {
			Rule::Textual	=> true,
			Rule::Par		=> matches!(k, ElemKind::Par | ElemKind::Align),
			Rule::Cites		=> matches!(k, ElemKind::CiteGroup | ElemKind::Cite),
			Rule::List		=> matches!(k, ElemKind::List | ElemKind::Align),
			Rule::Enum		=> matches!(k, ElemKind::Enum | ElemKind::Align),
			Rule::Terms		=> matches!(k, ElemKind::Terms | ElemKind::Align),
		}
	}
}

const LAYOUT_RULES:	&[Rule] = &[Rule::Textual, Rule::Par, Rule::Cites, Rule::List, Rule::Enum, Rule::Terms];
const PAR_RULES:	&[Rule] = &[Rule::Textual, Rule::Cites, Rule::List, Rule::Enum, Rule::Terms];
const MATH_RULES:	&[Rule] = &[Rule::Cites, Rule::List, Rule::Enum, Rule::Terms];

#[derive(Clone, Copy, Debug)]
struct Grouping {
	start:	usize,
	rule:	Rule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpaceState {
	Destructive,	// a space here would be dropped: start, after a break
	Supportive,		// after content a space may follow
	Space(usize),	// a space was kept, at this index
}

/// The show step a verdict chose.
enum Step {
	Recipe(Recipe, RecipeIndex),
	Builtin,
}

struct RegexMatch {
	offset:	usize,
	text:	String,
	index:	RecipeIndex,
	recipe:	Recipe,
	styles:	StyleChain,
}

struct State<'e> {
	engine:			&'e mut Engine,
	mode:			RealiseMode,
	keep_model:		bool,	// model elements stand unexpanded (`realise_structure`)
	sink:			Vec<Pair>,
	groupings:		Vec<Grouping>,
	outside:		bool,	// at the document's top level, not inside a container or show-rule output
	may_attach:		bool,	// the last block was a paragraph, so `v(attach: true)` survives
	saw_parbreak:	bool,
	fully_inline:	bool,	// the whole body is one paragraph's worth of inline elements (`Flow` mode)
	depth:			usize,	// nested show-rule outputs
}

impl State<'_> {
	fn rules(&self) -> &'static [Rule] {
		match self.mode {
			RealiseMode::Document | RealiseMode::Flow	=> LAYOUT_RULES,
			RealiseMode::Inline							=> PAR_RULES,
			RealiseMode::Math							=> MATH_RULES,
		}
	}

	fn visit(&mut self, content: &Content, styles: &StyleChain) -> Outcome<()> {
		// A tag carried in content (a paragraph's body) goes back into the stream where it stands.
		if let Some(tag) = Tag::from_content(content) {
			self.push_tag(tag, styles);
			return Ok(());
		}
		if res!(self.visit_kind_rules(content, styles)) {
			return Ok(());
		}
		if res!(self.visit_show_rules(content, styles)) {
			return Ok(());
		}
		match content {
			Content::Sequence(seq) => {
				for c in &seq.children {
					res!(self.visit(c, styles));
				}
				Ok(())
			}
			Content::Styled(st)	=> self.visit_styled(&st.child, &st.styles, styles, false),
			Content::Elem(_)	=> self.leaf(content, styles),
		}
	}

	/// An element no rule transforms further: grouped, filtered or pushed.
	fn leaf(&mut self, content: &Content, styles: &StyleChain) -> Outcome<()> {
		if res!(self.visit_grouping_rules(content, styles)) {
			return Ok(());
		}
		if res!(self.visit_filter_rules(content, styles)) {
			return Ok(());
		}
		self.sink.push(Pair::new(content.clone(), styles.clone()));
		Ok(())
	}

	fn push_tag(&mut self, tag: Tag, styles: &StyleChain) {
		self.sink.push(Pair::tag(tag, styles.clone()));
	}

	// Kind rules

	fn visit_kind_rules(&mut self, content: &Content, styles: &StyleChain) -> Outcome<bool> {
		let kind = match content.kind() {
			Some(k)	=> k,
			None	=> return Ok(false),
		};
		if self.mode == RealiseMode::Math {
			// An equation nested in maths is transparent: `#let x = $pi$; $ x $`.
			if kind == ElemKind::Equation {
				if let Some(Value::Content(body)) = content.field("body") {
					let body = body.clone();
					res!(self.visit(&body, styles));
					return Ok(true);
				}
			}
			// A maths character is a symbol and a number is text; rules run on each element alone here,
			// not over a textual run.
			if kind == ElemKind::Text || kind == ElemKind::Symbol {
				if let Some(Value::Str(t)) = content.get(FieldId(0)) {
					let t = t.clone();
					if let Some(m) = res!(find_regex_match_in_str(&t, styles)) {
						let pair = Pair::new(content.clone(), styles.clone());
						res!(self.visit_regex_match(vec![pair], m));
						return Ok(true);
					}
				}
			}
			return Ok(false);
		}
		// Maths content outside an equation is wrapped in one.
		if kind.family() == Family::Math && kind != ElemKind::Equation {
			if let Some(id) = ElemKind::Equation.field_id("body") {
				let eq = Content::new(ElemKind::Equation, vec![(id, Value::Content(content.clone()))], content.span());
				res!(self.visit(&eq, styles));
				return Ok(true);
			}
		}
		// A symbol outside maths is text, as Typst makes it before any show rule or grouping runs. A
		// shorthand (`--`, `...`, `~`, `-?`), an escape and a symbol value all arrive here, so they join
		// the paragraph and the textual run they stand in, and text and regex rules reach them.
		if kind == ElemKind::Symbol {
			if let Some(Value::Str(t)) = content.get(FieldId(0)) {
				let mut text = Content::text(t).with_span(content.span());
				if let Some(label) = content.label() {
					text = text.labelled(label.clone());
				}
				res!(self.visit(&text, styles));
				return Ok(true);
			}
		}
		Ok(false)
	}

	// Show rules

	fn visit_show_rules(&mut self, content: &Content, styles: &StyleChain) -> Outcome<bool> {
		match content {
			Content::Elem(_)										=> self.show_elem(content, styles),
			Content::Sequence(seq) if seq.label.is_some()			=> self.show_labelled(content, styles),
			_														=> Ok(false),
		}
	}

	fn show_elem(&mut self, target: &Content, styles: &StyleChain) -> Outcome<bool> {
		let (prepared, guards) = match target {
			Content::Elem(e)	=> (e.prepared, e.guards.clone()),
			_					=> return Ok(false),
		};
		// The verdict: the innermost unguarded matching recipe is the step; show-set rules are collected
		// (all of them, until the element is prepared, whichever side of the step they sit).
		let mut map = Styles::new();
		let mut step = None;
		for (index, recipe) in styles.recipes() {
			if !res!(recipe.applicable(target, styles)) {
				continue;
			}
			if let Transformation::Style(set) = &recipe.transform {
				if !prepared {
					map.apply_outer(set);
				}
				continue;
			}
			if step.is_some() || guards.contains(&index) {
				continue;
			}
			step = Some(Step::Recipe(recipe.clone(), index));
			if prepared {
				break;
			}
		}
		let step = step.unwrap_or(Step::Builtin);
		let mut output = target.clone();
		let mut tags = None;
		if !prepared {
			tags = res!(self.prepare(&mut output, &mut map, styles));
		}
		let chained = styles.chain(&map);
		// A show rule's error does not end the compile here: the element shows as nothing and the error
		// stands for the final pass to report (Typst's `Engine::delay`).
		let mark = self.engine.diags.len();
		let result = match step {
			Step::Recipe(recipe, index) => {
				if let Content::Elem(e) = &mut output {
					Arc::make_mut(e).guards.push(index);
				}
				let shown = apply_recipe(self.engine, &recipe, output.clone(), &chained);
				self.engine.delay(mark, target.span(), shown)
			}
			Step::Builtin => match self.builtin_show(&output, &chained) {
				Ok(Some(c)) => spanned(c, output.span()),
				Err(e) => {
					let shown = self.engine.delay(mark, target.span(), Err::<Content, _>(e));
					spanned(shown, output.span())
				},
				Ok(None) => {
					// A primitive: flow lays it out itself.
					if let Some((start, _)) = &tags {
						self.push_tag(start.clone(), styles);
					}
					res!(self.visit_styled(&output, &map, styles, true));
					if let Some((_, end)) = tags {
						self.push_tag(end, styles);
					}
					return Ok(true);
				}
			},
		};
		if let Some((start, _)) = &tags {
			self.push_tag(start.clone(), styles);
		}
		res!(self.visit_output(target, &result, &map, styles));
		if let Some((_, end)) = tags {
			self.push_tag(end, styles);
		}
		Ok(true)
	}

	// The element's built-in show, or `None` for a primitive -- and for a model element when the
	// structure is being kept.
	fn builtin_show(&mut self, elem: &Content, styles: &StyleChain) -> Outcome<Option<Content>> {
		if self.keep_model && elem.kind().map(|k| k.family() == Family::Model).unwrap_or(false) {
			return Ok(None);
		}
		native_show(self.engine, elem, styles)
	}

	/// A labelled sequence meets label rules too, and is located and guarded as an element is: its
	/// location is assigned once, and a recipe applied to it is not applied to its output again.
	fn show_labelled(&mut self, target: &Content, styles: &StyleChain) -> Outcome<bool> {
		let (prepared, guards) = match target {
			Content::Sequence(seq)	=> (seq.location.is_some(), seq.guards.clone()),
			_						=> return Ok(false),
		};
		let mut map = Styles::new();
		let mut step = None;
		for (index, recipe) in styles.recipes() {
			if !res!(recipe.applicable(target, styles)) {
				continue;
			}
			if let Transformation::Style(set) = &recipe.transform {
				if !prepared {
					map.apply_outer(set);
				}
				continue;
			}
			if step.is_some() || guards.contains(&index) {
				continue;
			}
			step = Some((recipe.clone(), index));
			if prepared {
				break;
			}
		}
		if prepared && step.is_none() {
			return Ok(false);
		}
		let mut output = target.clone();
		let mut tags = None;
		if let Content::Sequence(seq) = &mut output {
			let seq = Arc::make_mut(seq);
			if seq.location.is_none() {
				let loc = self.engine.locator.locate(ElemKind::Sequence, seq.span);
				seq.location = Some(loc);
				tags = Some(loc);
			}
			if let Some((_, index)) = &step {
				seq.guards.push(*index);
			}
		}
		if tags.is_some() {
			self.push_tag(Tag::Start(output.clone()), styles);
		}
		match step {
			Some((recipe, _)) => {
				let chained = styles.chain(&map);
				let mark = self.engine.diags.len();
				let shown = apply_recipe(self.engine, &recipe, output.clone(), &chained);
				let result = self.engine.delay(mark, target.span(), shown);
				res!(self.visit_output(target, &result, &map, styles));
			}
			// No recipe: the located sequence itself, which the next visit walks into at this level,
			// so page rules inside it still reach the page.
			None => res!(self.visit_styled(&output, &map, styles, false)),
		}
		if let Some(loc) = tags {
			self.push_tag(Tag::End(loc), styles);
		}
		Ok(true)
	}

	/// Visits a show rule's output under the collected show-set styles, one level deeper.
	fn visit_output(&mut self, target: &Content, output: &Content, map: &Styles, styles: &StyleChain) -> Outcome<()> {
		self.depth += 1;
		if self.depth > MAX_SHOW_RULE_DEPTH {
			return Err(error_hints(self.engine, DiagnosticKind::Limit, target.span(), "maximum show rule depth exceeded", &[
				"maybe a show rule matches its own output",
				"maybe there are too deeply nested elements",
			]));
		}
		let prev_outside = self.outside;
		self.outside &= target.is(ElemKind::Context);
		let r = self.visit_styled(output, map, styles, false);
		self.outside = prev_outside;
		self.depth -= 1;
		r
	}

	/// Gives the element its location (when locatable or labelled) and copies the style chain's values
	/// of its unset settable fields into it, so a show rule sees them; returns its tags when located.
	fn prepare(&mut self, target: &mut Content, map: &mut Styles, styles: &StyleChain) -> Outcome<Option<(Tag, Tag)>> {
		// Built-in show-set styles sit outside the user's, which override them.
		let builtin = res!(content::show_set(target, styles));
		let e = match target {
			Content::Elem(e)	=> Arc::make_mut(e),
			_					=> return Ok(None),
		};
		if e.location.is_none() && (e.kind.locatable() || e.label.is_some()) {
			e.location = Some(self.engine.locator.locate(e.kind, e.span));
		}
		// An element that lays a body out keeps one place for it however often it is laid out. One a layouter
		// made and gave a place already (a grid's cell) keeps that.
		if e.place.is_none() && e.kind.has_place() {
			e.place = Some(self.engine.locator.next(e.kind, e.span));
		}
		map.apply_outer(&builtin);
		let chain = styles.chain(map);
		for (i, spec) in e.kind.fields().iter().enumerate() {
			let id = FieldId(i as u8);
			if !spec.settable || e.fields.iter().any(|(f, _)| *f == id) {
				continue;
			}
			if let Some(v) = res!(chain.get(e.kind, id)) {
				e.fields.push((id, v));
			}
		}
		res!(content::synthesise(self.engine, target, &chain));
		let loc = match target {
			Content::Elem(e)	=> {
				let e = Arc::make_mut(e);
				e.prepared = true;
				e.location
			}
			_					=> None,
		};
		Ok(loc.map(|l| (Tag::Start(target.clone()), Tag::End(l))))
	}

	// Styled content

	/// Visits `child` under `local` styles pushed onto `outer`. `leaf` pushes the child without another
	/// round of rules (it is a prepared primitive).
	fn visit_styled(&mut self, child: &Content, local: &Styles, outer: &StyleChain, leaf: bool) -> Outcome<()> {
		if local.is_empty() {
			return if leaf { self.leaf(child, outer) } else { self.visit(child, outer) };
		}
		// `show: f` transforms the rest of its scope at once, the styles before it staying outside.
		if let Some(i) = local.iter().position(|s| matches!(s, Style::Recipe(r) if r.selector.is_none())) {
			let before = Styles::from_vec(local.as_slice()[..i].to_vec());
			let after = Styles::from_vec(local.as_slice()[i + 1..].to_vec());
			let recipe = match &local.as_slice()[i] {
				Style::Recipe(r)	=> r.clone(),
				_					=> return Err(err!("selector-less recipe vanished"; Bug)),
			};
			let chain = outer.chain(&before);
			let out = res!(apply_recipe(self.engine, &recipe, child.clone().styled(after), &chain));
			return self.visit_styled(&out, &before, outer, false);
		}
		let mut pagebreak = false;
		for style in local.iter() {
			let p = match style {
				Style::Property(p)	=> p,
				_					=> continue,
			};
			match p.elem {
				ElemKind::Document if self.mode != RealiseMode::Document => {
					return Err(self.engine.error(DiagnosticKind::Type, p.span, "document set rules are not allowed inside of containers"));
				}
				ElemKind::Page if self.mode != RealiseMode::Document => {
					return Err(self.engine.error(DiagnosticKind::Type, p.span, "page configuration is not allowed inside of containers"));
				}
				ElemKind::Page => {
					// Page styles break free of the show-rule cage.
					pagebreak = true;
					self.outside = true;
				}
				_ => (),
			}
		}
		if pagebreak {
			// The leading break takes the styles up to and including the last page property only.
			let last = local.as_slice().iter().rposition(|s| matches!(s, Style::Property(p) if p.elem == ElemKind::Page));
			let relevant = match last {
				Some(l)	=> Styles::from_vec(local.as_slice()[..=l].to_vec()),
				None	=> local.clone(),
			};
			res!(self.visit(&weak_pagebreak(false), &outer.chain(&relevant)));
		}
		res!(self.finish_interrupted(local));
		let chain = outer.chain(local);
		res!(if leaf { self.leaf(child, &chain) } else { self.visit(child, &chain) });
		res!(self.finish_interrupted(local));
		if pagebreak {
			res!(self.visit(&weak_pagebreak(true), outer));
		}
		Ok(())
	}

	/// Ends every group that the styled elements interrupt, innermost first, as long as one is. Finishing
	/// a textual run opens the paragraph it belongs to, which no style on a text interrupts, so the
	/// paragraph stays open across a styled run inside it.
	fn finish_interrupted(&mut self, local: &Styles) -> Outcome<()> {
		let mut last = None;
		for elem in local.iter().filter_map(|s| s.element()) {
			if last == Some(elem) {
				continue;
			}
			let mut i = 0;
			while self.groupings.iter().any(|g| g.rule.interrupted_by(elem)) {
				res!(self.finish_innermost_grouping());
				i += 1;
				if i > MAX_GROUPING_STEPS {
					return Err(self.engine.error(DiagnosticKind::Limit, Span::detached(), "maximum grouping depth exceeded"));
				}
			}
			last = Some(elem);
		}
		Ok(())
	}

	// Grouping

	fn visit_grouping_rules(&mut self, content: &Content, styles: &StyleChain) -> Outcome<bool> {
		let matching = self.rules().iter().copied().find(|r| r.trigger(content, self.keep_model));
		let mut i = 0;
		while let Some(active) = self.groupings.last().copied() {
			// A rule of higher priority nests a new group inside the active one.
			if matching.map(|r| r.priority() > active.rule.priority()).unwrap_or(false) {
				break;
			}
			if active.rule.trigger(content, self.keep_model) || active.rule.inner(content) {
				self.sink.push(Pair::new(content.clone(), styles.clone()));
				return Ok(true);
			}
			res!(self.finish_innermost_grouping());
			i += 1;
			if i > MAX_GROUPING_STEPS {
				return Err(self.engine.error(DiagnosticKind::Limit, content.span(), "maximum grouping depth exceeded"));
			}
		}
		if let Some(rule) = matching {
			self.groupings.push(Grouping { start: self.sink.len(), rule });
			self.sink.push(Pair::new(content.clone(), styles.clone()));
			return Ok(true);
		}
		Ok(false)
	}

	/// Spaces and paragraph breaks left outside every group mean nothing to block flow; attached spacing
	/// survives only straight after a paragraph.
	fn visit_filter_rules(&mut self, content: &Content, styles: &StyleChain) -> Outcome<bool> {
		if matches!(self.mode, RealiseMode::Inline | RealiseMode::Math) {
			return Ok(false);
		}
		if content.is(ElemKind::Space) {
			return Ok(true);
		}
		if content.is(ElemKind::Parbreak) {
			self.may_attach = false;
			self.saw_parbreak = true;
			return Ok(true);
		}
		if !self.may_attach && content.is(ElemKind::V) {
			let attach = match ElemKind::V.field_id("attach") {
				Some(id)	=> res!(styles.resolve(content, id)),
				None		=> None,
			};
			if matches!(attach, Some(Value::Bool(true))) {
				return Ok(true);
			}
		}
		self.may_attach = content.is(ElemKind::Par);
		Ok(false)
	}

	fn finish(&mut self) -> Outcome<()> {
		let mut i = 0;
		let mut inline = false;
		while !self.groupings.is_empty() {
			if self.is_fully_inline() {
				inline = true;
				self.fully_inline = true;
				break;
			}
			res!(self.finish_innermost_grouping());
			i += 1;
			if i > MAX_GROUPING_STEPS {
				return Err(self.engine.error(DiagnosticKind::Limit, Span::detached(), "maximum grouping depth exceeded"));
			}
		}
		if inline || matches!(self.mode, RealiseMode::Inline | RealiseMode::Math) {
			res!(collapse_spaces(&mut self.sink, 0));
		}
		Ok(())
	}

	/// A fragment whose whole content is one paragraph's worth of inline elements.
	fn is_fully_inline(&self) -> bool {
		self.mode == RealiseMode::Flow
			&& !self.saw_parbreak
			&& matches!(self.groupings.as_slice(), [g] if g.start == 0 && g.rule == Rule::Par)
	}

	fn finish_innermost_grouping(&mut self) -> Outcome<()> {
		let g = match self.groupings.pop() {
			Some(g)	=> g,
			None	=> return Ok(()),
		};
		// Trailing members that did not trigger the group are not part of it.
		let end = match self.sink[g.start..].iter().rposition(|p| !p.is_tag() && g.rule.trigger(&p.content, self.keep_model)) {
			Some(i)	=> g.start + i + 1,
			None	=> g.start,
		};
		let tail = self.sink.split_off(end);
		match g.rule {
			Rule::Textual						=> res!(self.finish_textual(g.start)),
			Rule::Par							=> res!(self.finish_par(g.start)),
			Rule::Cites							=> res!(self.finish_cites(g.start)),
			Rule::List | Rule::Enum | Rule::Terms	=> res!(self.finish_list(g.start, g.rule)),
		}
		for p in tail {
			match p.tag {
				Some(t)	=> self.sink.push(Pair::tag(t, p.styles)),
				None	=> res!(self.visit(&p.content, &p.styles)),
			}
		}
		Ok(())
	}

	fn finish_textual(&mut self, start: usize) -> Outcome<()> {
		if let Some(m) = res!(find_regex_match_in_elems(&self.sink[start..])) {
			res!(collapse_spaces(&mut self.sink, start));
			let elems = self.sink.split_off(start);
			return self.visit_regex_match(elems, m);
		}
		// No match: the run is a paragraph's to group, when this realisation groups paragraphs.
		if self.groupings.is_empty() && self.rules().contains(&Rule::Par) {
			self.groupings.push(Grouping { start, rule: Rule::Par });
		}
		Ok(())
	}

	/// Splits the run around the match and puts the recipe's output in its place, under a revocation of
	/// the recipe so it cannot match its own output.
	fn visit_regex_match(&mut self, elems: Vec<Pair>, m: RegexMatch) -> Outcome<()> {
		let (ms, me) = (m.offset, m.offset + m.text.len());
		let span = elems.iter().map(|p| p.content.span()).find(|s| !s.is_detached()).unwrap_or(Span::detached());
		let piece = Content::text(&m.text).with_span(span);
		let output = res!(apply_recipe(self.engine, &m.recipe, piece, &m.styles));
		let mut pending = Some(output);
		let revoked = m.styles.chain(&Styles::from_style(Style::Revocation(m.index)));
		let mut cursor = 0;
		for p in elems {
			if let Some(t) = p.tag {
				self.sink.push(Pair::tag(t, p.styles));
				continue;
			}
			let text = match (p.content.kind(), p.content.get(FieldId(0))) {
				(Some(ElemKind::Text | ElemKind::Symbol), Some(Value::Str(t)))	=> Some(t.clone()),
				_																=> None,
			};
			let len = text.as_ref().map(|t| t.len()).unwrap_or(1);
			let (es, ee) = (cursor, cursor + len);
			if es < ms {
				if ee <= ms {
					res!(self.visit(&p.content, &p.styles));
				} else if let Some(t) = &text {
					let piece = slice_text(&p.content, t, 0, ms - es);
					res!(self.visit(&piece, &p.styles));
				}
			}
			if ms < ee {
				if let Some(out) = pending.take() {
					res!(self.visit(&out, &revoked));
				}
			}
			if ee > me {
				if es >= me {
					res!(self.visit(&p.content, &p.styles));
				} else if let Some(t) = &text {
					let piece = slice_text(&p.content, t, me - es, t.len());
					res!(self.visit(&piece, &p.styles));
				}
			}
			cursor = ee;
		}
		if let Some(out) = pending.take() {
			res!(self.visit(&out, &revoked));
		}
		Ok(())
	}

	/// A paragraph keeps the tags that fall inside it in its body, as `tag` elements, so a located
	/// element in running text is found where it is set.
	fn finish_par(&mut self, start: usize) -> Outcome<()> {
		res!(collapse_spaces(&mut self.sink, start));
		let elems = self.sink.split_off(start);
		let span = select_span(&elems);
		let (body, trunk) = repack(&elems);
		let id = res!(field(ElemKind::Par, "body"));
		let par = Content::new(ElemKind::Par, vec![(id, Value::Content(body))], span);
		self.visit(&par, &trunk)
	}

	/// Adjacent citations, with only spaces between them, become one cite group.
	fn finish_cites(&mut self, start: usize) -> Outcome<()> {
		let elems = self.sink.split_off(start);
		let (cites, rest): (Vec<Pair>, Vec<Pair>) = elems.into_iter().partition(|p| p.content.is(ElemKind::Cite));
		let tags: Vec<Pair> = rest.into_iter().filter(|p| p.is_tag()).collect();
		let span = select_span(&cites);
		let trunk = StyleChain::trunk(cites.iter().map(|p| &p.styles));
		let children = cites.iter().map(|p| Value::Content(p.content.clone())).collect();
		let id = res!(field(ElemKind::CiteGroup, "children"));
		let group = Content::new(ElemKind::CiteGroup, vec![(id, Value::array(children))], span);
		self.around_tags(tags, |s| s.visit(&group, &trunk))
	}

	fn finish_list(&mut self, start: usize, rule: Rule) -> Outcome<()> {
		let (kind, item_kind) = match rule {
			Rule::List	=> (ElemKind::List, ElemKind::ListItem),
			Rule::Enum	=> (ElemKind::Enum, ElemKind::EnumItem),
			_			=> (ElemKind::Terms, ElemKind::TermItem),
		};
		let elems = self.sink.split_off(start);
		let tight = !elems.iter().any(|p| p.content.is(ElemKind::Parbreak));
		let (items, rest): (Vec<Pair>, Vec<Pair>) = elems.into_iter().partition(|p| p.content.is(item_kind));
		let tags: Vec<Pair> = rest.into_iter().filter(|p| p.is_tag()).collect();
		let span = select_span(&items);
		let trunk = StyleChain::trunk(items.iter().map(|p| &p.styles));
		let depth = trunk.depth();
		let bodies: &[&str] = match item_kind {
			ElemKind::TermItem	=> &["term", "description"],
			_					=> &["body"],
		};
		let mut children = Vec::new();
		for p in &items {
			let local = p.styles.suffix(depth);
			let mut item = p.content.clone();
			if !local.is_empty() {
				for name in bodies {
					if let Some(id) = item_kind.field_id(name) {
						if let Some(Value::Content(b)) = item.get(id) {
							let b = b.clone().styled(local.clone());
							item.set(id, Value::Content(b));
						}
					}
				}
			}
			children.push(Value::Content(item));
		}
		let mut fields = vec![(res!(field(kind, "children")), Value::array(children))];
		if let Some(id) = kind.field_id("tight") {
			fields.push((id, Value::Bool(tight)));
		}
		let list = Content::new(kind, fields, span);
		self.around_tags(tags, |s| s.visit(&list, &trunk))
	}

	/// Tags lifted out of a group: starts before the group's element, ends after it.
	fn around_tags<F: FnOnce(&mut Self) -> Outcome<()>>(&mut self, tags: Vec<Pair>, f: F) -> Outcome<()> {
		let (starts, ends): (Vec<Pair>, Vec<Pair>) = tags.into_iter()
			.partition(|p| matches!(p.tag, Some(Tag::Start(_))));
		self.sink.extend(starts);
		res!(f(self));
		self.sink.extend(ends);
		Ok(())
	}
}

/// A schema field the grouping must fill; its absence is the owning unit's gap, reported as such.
fn field(kind: ElemKind, name: &str) -> Outcome<FieldId> {
	match kind.field_id(name) {
		Some(id)	=> Ok(id),
		None		=> Err(err!("{} has no `{}` field in its schema, so realisation cannot group into it",
			kind.path(), name; Unimplemented)),
	}
}

/// A weak page break, as page styles in the document's flow produce around themselves; `boundary`
/// marks the one after, whose styles take precedence.
fn weak_pagebreak(boundary: bool) -> Content {
	let mut fields = Vec::new();
	if let Some(id) = ElemKind::Pagebreak.field_id("weak") {
		fields.push((id, Value::Bool(true)));
	}
	if boundary {
		if let Some(id) = ElemKind::Pagebreak.field_id("boundary") {
			fields.push((id, Value::Bool(true)));
		}
	}
	Content::new(ElemKind::Pagebreak, fields, Span::detached())
}

/// A native show's output takes the element's span where it has none of its own.
fn spanned(c: Content, span: Span) -> Content {
	if c.span().is_detached() { c.with_span(span) } else { c }
}

fn slice_text(c: &Content, t: &str, from: usize, to: usize) -> Content {
	let mut piece = c.clone();
	let s = t.get(from..to).unwrap_or(t);
	piece.set(FieldId(0), Value::str(s));
	piece
}

fn select_span(pairs: &[Pair]) -> Span {
	pairs.iter().filter(|p| !p.is_tag()).map(|p| p.content.span()).find(|s| !s.is_detached())
		.unwrap_or(Span::detached())
}

/// Rebuilds content from grouped pairs: each run of members under one chain becomes a sequence styled
/// with what its chain has beyond the group's trunk, and the trunk is returned for the group itself.
/// A tag among them becomes a `tag` element; the trunk is taken over the members alone.
fn repack(pairs: &[Pair]) -> (Content, StyleChain) {
	let members: Vec<&Pair> = pairs.iter().filter(|p| !p.is_tag()).collect();
	let trunk = if members.is_empty() {
		StyleChain::trunk(pairs.iter().map(|p| &p.styles))
	} else {
		StyleChain::trunk(members.iter().map(|p| &p.styles))
	};
	let depth = trunk.depth();
	let mut children = Vec::new();
	let mut i = 0;
	while i < pairs.len() {
		let mut j = i + 1;
		while j < pairs.len() && pairs[j].styles.ptr_eq(&pairs[i].styles) {
			j += 1;
		}
		let run: Vec<Content> = pairs[i..j].iter().map(|p| match &p.tag {
			Some(t)	=> t.to_content(),
			None	=> p.content.clone(),
		}).collect();
		children.push(Content::sequence(run).styled(pairs[i].styles.suffix(depth)));
		i = j;
	}
	(Content::sequence(children), trunk)
}

/// Drops spaces at the edges of a run and next to breaks: after nothing, before or after a line break,
/// and before a weak or fractional `h`.
fn collapse_spaces(sink: &mut Vec<Pair>, start: usize) -> Outcome<()> {
	let tail = sink.split_off(start);
	let mut state = SpaceState::Destructive;
	for p in tail {
		if !p.is_tag() {
			let c = &p.content;
			if c.is(ElemKind::Space) {
				if state != SpaceState::Supportive {
					continue;
				}
				state = SpaceState::Space(sink.len());
			} else if c.is(ElemKind::Linebreak) {
				destruct_space(sink, &mut state);
			} else if c.is(ElemKind::H) {
				if res!(is_weak_or_fractional(c, &p.styles)) {
					destruct_space(sink, &mut state);
				}
			} else {
				state = SpaceState::Supportive;
			}
		}
		sink.push(p);
	}
	destruct_space(sink, &mut state);
	Ok(())
}

fn destruct_space(sink: &mut Vec<Pair>, state: &mut SpaceState) {
	if let SpaceState::Space(i) = *state {
		if i < sink.len() {
			sink.remove(i);
		}
	}
	*state = SpaceState::Destructive;
}

fn is_weak_or_fractional(h: &Content, styles: &StyleChain) -> Outcome<bool> {
	let field = |name: &str| -> Outcome<Option<Value>> {
		match ElemKind::H.field_id(name) {
			Some(id)	=> styles.resolve(h, id),
			None		=> Ok(None),
		}
	};
	let amount = res!(field("amount"));
	let weak = res!(field("weak"));
	Ok(matches!(amount, Some(Value::Fraction(_))) || matches!(weak, Some(Value::Bool(true))))
}

// Text and regex rules

/// The leftmost match of any text or regex rule in a textual run. The run is read as Typst reads it --
/// spaces collapsed, a line break as `\n`, smart quotes as straight ones -- and searched one stretch of
/// equal styles at a time, since a rule cannot match across a style change.
fn find_regex_match_in_elems(elems: &[Pair]) -> Outcome<Option<RegexMatch>> {
	let mut buf = String::new();
	let mut base = 0;
	let mut current: Option<StyleChain> = None;
	let mut space = SpaceState::Destructive;
	for p in elems {
		if p.is_tag() {
			continue;
		}
		let c = &p.content;
		let linebreak = c.is(ElemKind::Linebreak);
		if linebreak {
			if let SpaceState::Space(_) = space {
				buf.pop();
			}
		}
		let changed = current.as_ref().map(|cur| !cur.ptr_eq(&p.styles)).unwrap_or(false);
		if changed && !buf.is_empty() {
			if let Some(cur) = &current {
				if let Some(m) = res!(find_regex_match_in_str(&buf, cur)) {
					return Ok(Some(RegexMatch { offset: base + m.offset, ..m }));
				}
			}
			base += buf.len();
			buf.clear();
		}
		current = Some(p.styles.clone());
		if c.is(ElemKind::Space) {
			if space != SpaceState::Supportive {
				continue;
			}
			buf.push(' ');
			space = SpaceState::Space(0);
		} else if linebreak {
			buf.push('\n');
			space = SpaceState::Destructive;
		} else if c.is(ElemKind::SmartQuote) {
			let double = match ElemKind::SmartQuote.field_id("double") {
				Some(id)	=> res!(p.styles.resolve(c, id)),
				None		=> None,
			};
			buf.push(if matches!(double, Some(Value::Bool(false))) { '\'' } else { '"' });
			space = SpaceState::Supportive;
		} else if let Some(Value::Str(t)) = c.get(FieldId(0)) {
			buf.push_str(t);
			space = SpaceState::Supportive;
		}
	}
	match &current {
		Some(cur) => Ok(res!(find_regex_match_in_str(&buf, cur)).map(|m| RegexMatch { offset: base + m.offset, ..m })),
		None => Ok(None),
	}
}

/// The leftmost non-empty match of the text and regex rules in force, the innermost rule winning a tie.
fn find_regex_match_in_str(text: &str, styles: &StyleChain) -> Outcome<Option<RegexMatch>> {
	let mut best: Option<(usize, usize, RecipeIndex, &Recipe)> = None;
	for (index, recipe) in styles.recipes() {
		let sel = match &recipe.selector {
			Some(s)	=> s,
			None	=> continue,
		};
		let (s, e) = match res!(sel.find_text(text)) {
			Some(m)	=> m,
			None	=> continue,
		};
		if s == e {
			continue;
		}
		if best.map(|(bs, ..)| bs <= s).unwrap_or(false) {
			continue;
		}
		best = Some((s, e, index, recipe));
	}
	Ok(best.map(|(s, e, index, recipe)| RegexMatch {
		offset:	s,
		text:	text[s..e].to_string(),
		index,
		recipe:	recipe.clone(),
		styles:	styles.clone(),
	}))
}

/// The label a realised pair's content carries, for the introspector.
pub fn label_of(pair: &Pair) -> Option<&Label> {
	match &pair.tag {
		Some(Tag::Start(c))	=> c.label(),
		_					=> pair.content.label(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn pair(kind: ElemKind) -> Pair {
		Pair::new(Content::new(kind, Vec::new(), Span::detached()), StyleChain::root())
	}

	/// Nothing, spaces, breaks and tags set nothing; any other element is content, drawn or not.
	#[test]
	fn a_realised_body_sets_content_unless_it_holds_only_spaces_breaks_and_tags() {
		assert!(!sets_content(&[]));
		assert!(!sets_content(&[
			pair(ElemKind::Space), pair(ElemKind::Parbreak), pair(ElemKind::Pagebreak), pair(ElemKind::Colbreak),
		]));
		let tag = Pair::tag(Tag::End(Location(1)), StyleChain::root());
		assert!(!sets_content(&[tag.clone(), pair(ElemKind::Space)]));
		assert!(sets_content(&[tag, pair(ElemKind::Space), pair(ElemKind::Linebreak)]));
		assert!(sets_content(&[pair(ElemKind::Parbreak), pair(ElemKind::Metadata)]));
	}
}
