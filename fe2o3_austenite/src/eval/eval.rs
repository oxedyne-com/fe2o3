// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-eval, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, IR and error handling.
// U2 owns this file: the tree-walking evaluator. Its public surface is `eval_module`, `eval_string` and
// `Engine::call_func`; the walker's own state (a VM with the scope chain and flow control) is private.
//
// The walker reads the concrete syntax tree directly, by kind and child order as typst-syntax 0.14/0.15
// lays them out, and follows typst-eval's semantics and wording. Three behaviours a reader could not
// guess from the types:
//
// * `set` and `show` style the *rest* of their markup or code block. The rest is gathered iteratively
//   and wrapped from the innermost rule outward, so a file with hundreds of rules does not recurse.
// * `show: f` with no selector is applied at once to the rest of the block, as Typst does; it never
//   reaches the style chain.
// * A closure captures by value the free names it mentions, at definition. Over-capturing a name that
//   a parameter later shadows is harmless, so the capture pass is a plain identifier scan.

use crate::diag::{
	Diagnostic,
	DiagnosticKind,
};
use crate::eval::args::{
	Arg,
	Args,
};
use crate::eval::content::{
	self,
	Content,
	ElemKind,
};
use crate::eval::func::{
	Closure,
	Func,
	NativeFunc,
	Param,
};
use crate::eval::import;
use crate::eval::lib;
use crate::eval::lib::foundations;
use crate::eval::methods;
use crate::eval::native_kind;
use crate::eval::ops;
use crate::eval::scope::{
	Binding,
	Scope,
};
use crate::eval::select::{
	self,
	Selector,
};
use crate::eval::styles::{
	self,
	Recipe,
	Style,
	Styles,
	Transformation,
};
use crate::eval::value::{
	Angle,
	Dict,
	Fraction,
	Label,
	Length,
	Module,
	Ratio,
	Value,
};
use crate::eval::Engine;
use crate::syntax::ast::{
	self,
	AstNode,
	BareImportError,
};
use crate::syntax::{
	parser,
	FileId,
	Span,
	SyntaxKind,
	SyntaxNode,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::unicode::segment;

use std::collections::{
	HashMap,
	HashSet,
};
use std::sync::Arc;

mod math_call;

const MAX_WHILE_ITERATIONS: usize = 10_000;	// Typst's per-loop limit for `while`

/// Which mode `eval(..)` parses its string in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalMode {
	Markup,
	Code,
	Math,
}

thread_local! {
	static LIBRARY: Arc<Scope> = Arc::new(lib::library());
}

/// The standard library scope, built once per thread and shared by every module and closure.
pub fn library() -> Arc<Scope> {
	LIBRARY.with(|l| l.clone())
}

/// Evaluates a loaded source: its top-level bindings become the module's scope, its markup (with every
/// `set`/`show` applied to the rest of its scope) the module's content.
pub fn eval_module(engine: &mut Engine, id: FileId) -> Outcome<Module> {
	let (root, path) = match engine.world.source(id) {
		Some(s)	=> (s.root.clone(), s.path.clone()),
		None	=> return Err(err!("No source with id {} is loaded.", id.0; Missing, Input)),
	};
	if report_syntax_errors(engine, &root, None) {
		return Err(err!("{} does not parse.", path.display(); Input, Invalid));
	}
	let lib = library();
	let (content, frame) = {
		let mut vm = Vm::new(engine, &lib, lib.clone(), id, false, Span::detached());
		let content = res!(vm.eval_markup(root.children()));
		res!(vm.forbid_flow());
		(content, vm.frames.pop().unwrap_or_default())
	};
	let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
	let mut module = Module::new(name, Scope { map: frame, parent: None });
	module.content = content;
	Ok(module)
}

/// `eval(text, mode:, scope:)`: evaluates a string against the standard library plus `scope`.
pub fn eval_string(
	engine:	&mut Engine,
	text:	&str,
	mode:	EvalMode,
	scope:	Scope,
	span:	Span,
)
	-> Outcome<Value>
{
	let root = match mode {
		EvalMode::Markup	=> parser::parse(text, FileId::DETACHED),
		EvalMode::Code		=> parser::parse_code(text, FileId::DETACHED),
		EvalMode::Math		=> parser::parse_math(text, FileId::DETACHED),
	};
	if report_syntax_errors(engine, &root, Some(span)) {
		return Err(err!("The string passed to eval does not parse."; Input, Invalid));
	}
	let lib = library();
	let base = Scope { map: scope.map, parent: Some(lib.clone()) };
	// The string's own nodes have no file; a path in it resolves from the file that called `eval`.
	let mut vm = Vm::new(engine, &base, lib.clone(), span.file, false, span);
	let out = match mode {
		EvalMode::Markup	=> Value::Content(res!(vm.eval_markup(root.children()))),
		EvalMode::Code		=> res!(vm.eval_code(root.children())),
		EvalMode::Math		=> {
			let body = res!(vm.eval_math_seq(root.children()));
			Value::Content(res!(vm.elem(ElemKind::Equation, vec![
				("block",	Value::Bool(false)),
				("body",	Value::Content(body)),
			], span)))
		}
	};
	res!(vm.forbid_flow());
	Ok(out)
}

// Every syntax error becomes a diagnostic; `at` replaces a detached position (a string given to `eval`).
fn report_syntax_errors(engine: &mut Engine, root: &SyntaxNode, at: Option<Span>) -> bool {
	let errors = root.errors();
	for (span, e) in &errors {
		let span = match at {
			Some(s) if span.is_detached()	=> s,
			_								=> *span,
		};
		let mut d = Diagnostic::error(DiagnosticKind::Syntax, span, e.message.clone());
		for h in &e.hints {
			d = d.with_hint(h.clone());
		}
		engine.diags.push(d);
	}
	!errors.is_empty()
}

impl Engine {
	/// Calls any function value: native, element constructor, closure or `.with`. The one call path
	/// realisation (show-rule closures), the library (`array.map`) and flow (`layout`) all use.
	pub fn call_func(&mut self, func: &Func, args: Args) -> Outcome<Value> {
		let span = args.span;
		let mark = self.diags.len();
		let out = match func {
			Func::Native(n)		=> n.call(self, args),
			Func::Element(k)	=> {
				let mut args = args;
				content::construct(self, *k, &mut args).map(Value::Content)
			}
			Func::Closure(c)	=> {
				let out = call_closure(self, c, func, args);
				if out.is_err() {
					let name = c.name.clone().unwrap_or_else(|| "closure".to_string());
					for d in self.diags[mark..].iter_mut() {
						if d.is_error() {
							d.trace.push((span, fmt!("while calling `{}`", name)));
						}
					}
				}
				out
			}
			Func::With(w)		=> {
				let mut all = w.1.clone();
				all.span = span;
				all.items.extend(args.items);
				self.call_func(&w.0, all)
			}
		};
		match out {
			Ok(v)	=> Ok(v),
			Err(e)	=> Err(self.adopt(mark, span, e)),
		}
	}

	/// An error raised without a diagnostic (a native function's `err!`) gets one at `span`; an error
	/// that already recorded its own is passed on untouched.
	pub fn adopt(&mut self, mark: usize, span: Span, e: Error<ErrTag>) -> Error<ErrTag> {
		let recorded = self.diags.get(mark..).map(|ds| ds.iter().any(|d| d.is_error())).unwrap_or(false);
		if !recorded {
			self.diags.push(Diagnostic::error(native_kind(&e), span, e.plain()));
		}
		e
	}
}

fn call_closure(engine: &mut Engine, c: &Arc<Closure>, func: &Func, args: Args) -> Outcome<Value> {
	res!(engine.enter_call(args.span));
	let lib = library();
	let out = {
		let mut vm = Vm::new(engine, &c.captured, lib, c.span.file, true, Span::detached());
		if let Some(name) = &c.name {
			vm.define(name, Value::Func(func.clone()), c.span);
		}
		match vm.bind_params(&c.params, args) {
			Ok(())	=> vm.eval_closure_body(&c.body),
			Err(e)	=> Err(e),
		}
	};
	engine.exit_call();
	out
}

// The evaluator

#[derive(Clone, Debug)]
enum Flow {
	Break(Span),
	Continue(Span),
	Return(Span, Option<Value>, bool),	// the value, and whether an `if` or a loop lies between
}

// A style that applies to the rest of a block once the rest is known.
enum Pending {
	Styles(Styles),
	Recipe(Recipe),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bind {
	Define,
	Assign,
}

enum Step {
	Key(String, Span),
	At(Value, Span),
}

type Frame = HashMap<String, Binding>;

struct Vm<'a> {
	engine:		&'a mut Engine,
	base:		&'a Scope,		// the library, a closure's captures, or `eval`'s scope over the library
	lib:		Arc<Scope>,		// the parent of every capture scope
	frames:		Vec<Frame>,		// block scopes, innermost last
	flow:		Option<Flow>,
	file:		FileId,
	in_func:	bool,			// running a closure body, so `base` holds captures
	fallback:	Span,			// where a detached span reports (the `eval` call)
}

fn is_expr(k: SyntaxKind) -> bool {
	use SyntaxKind as K;
	matches!(k,
		K::Text | K::Space | K::Linebreak | K::Parbreak | K::Escape | K::Shorthand | K::SmartQuote
		| K::Strong | K::Emph | K::Raw | K::Link | K::Label | K::Ref | K::Heading | K::ListItem
		| K::EnumItem | K::TermItem | K::Equation | K::Math | K::MathText | K::MathIdent
		| K::MathShorthand | K::MathAlignPoint | K::MathDelimited | K::MathAttach | K::MathPrimes
		| K::MathFrac | K::MathRoot | K::MathCall | K::MathFieldAccess | K::Ident | K::None | K::Auto | K::Bool | K::Int | K::Float
		| K::Numeric | K::Str | K::CodeBlock | K::ContentBlock | K::Parenthesized | K::Array | K::Dict
		| K::Unary | K::Binary | K::FieldAccess | K::FuncCall | K::Closure | K::LetBinding | K::SetRule
		| K::ShowRule | K::Contextual | K::Conditional | K::WhileLoop | K::ForLoop | K::ModuleImport
		| K::ModuleInclude | K::LoopBreak | K::LoopContinue | K::FuncReturn | K::DestructAssignment)
}

// A code expression: trivia, punctuation and keywords are not.
fn is_code_expr(k: SyntaxKind) -> bool {
	is_expr(k) && !matches!(k, SyntaxKind::Space | SyntaxKind::Parbreak)
}

// What a `let`, `for` or destructuring assignment binds to: an expression, `_`, or a pattern.
fn is_pattern(k: SyntaxKind) -> bool {
	is_code_expr(k) || matches!(k, SyntaxKind::Destructuring | SyntaxKind::Underscore)
}

fn pattern_after(n: &SyntaxNode, k: SyntaxKind) -> Option<&SyntaxNode> {
	n.children().iter().skip_while(|c| c.kind() != k).skip(1).find(|c| is_pattern(c.kind()))
}

fn code_exprs(n: &SyntaxNode) -> Vec<&SyntaxNode> {
	n.children().iter().filter(|c| is_code_expr(c.kind())).collect()
}

// The expression following the first child of kind `k`.
fn expr_after(n: &SyntaxNode, k: SyntaxKind) -> Option<&SyntaxNode> {
	n.children().iter().skip_while(|c| c.kind() != k).skip(1).find(|c| is_code_expr(c.kind()))
}

fn last_expr(n: &SyntaxNode) -> Option<&SyntaxNode> {
	n.children().iter().rev().find(|c| is_code_expr(c.kind()))
}

fn first_expr(n: &SyntaxNode) -> Option<&SyntaxNode> {
	n.children().iter().find(|c| is_code_expr(c.kind()))
}

fn ident_text(n: &SyntaxNode) -> Option<&str> {
	n.children().iter().rev().find(|c| c.kind() == SyntaxKind::Ident).map(|c| c.text())
}

// Is the callee a name that math mode resolved, so a non-function callee displays with its arguments?
fn in_math(n: &SyntaxNode) -> bool {
	match n.kind() {
		SyntaxKind::MathIdent	=> true,
		SyntaxKind::FieldAccess	=> first_expr(n).map(in_math).unwrap_or(false),
		_						=> false,
	}
}

fn collect_idents(n: &SyntaxNode, out: &mut HashSet<String>) {
	match n.kind() {
		SyntaxKind::Ident | SyntaxKind::MathIdent	=> { out.insert(n.text().to_string()); }
		_ => for c in n.children() {
			collect_idents(c, out);
		},
	}
}

fn is_invariant(n: &SyntaxNode) -> bool {
	match n.kind() {
		SyntaxKind::Ident | SyntaxKind::MathIdent	=> false,
		SyntaxKind::FieldAccess	=> first_expr(n).map(is_invariant).unwrap_or(true),
		SyntaxKind::FuncCall	=> n.children().iter().all(is_invariant),
		_						=> n.children().iter().all(is_invariant),
	}
}

fn can_diverge(n: &SyntaxNode) -> bool {
	matches!(n.kind(), SyntaxKind::Break | SyntaxKind::Return) || n.children().iter().any(can_diverge)
}

fn unescape_markup(esc: &str) -> String {
	match esc.strip_prefix("\\u{").and_then(|s| s.strip_suffix('}')) {
		Some(hex)	=> u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
			.map(|c| c.to_string()).unwrap_or_else(|| esc.to_string()),
		None		=> esc.chars().nth(1).map(|c| c.to_string()).unwrap_or_default(),
	}
}

fn markup_shorthand(s: &str) -> &str {
	match s {
		"~"		=> "\u{a0}",
		"-?"	=> "\u{ad}",
		"--"	=> "\u{2013}",
		"---"	=> "\u{2014}",
		"..."	=> "\u{2026}",
		"-"		=> "\u{2212}",
		other	=> other,
	}
}

fn math_shorthand(s: &str) -> &str {
	match s {
		"..."	=> "\u{2026}",
		"-"		=> "\u{2212}",
		"*"		=> "\u{2217}",
		"~"		=> "\u{223c}",
		"!="	=> "\u{2260}",
		":="	=> "\u{2254}",
		"::="	=> "\u{2a74}",
		"=:"	=> "\u{2255}",
		"<<"	=> "\u{226a}",
		"<<<"	=> "\u{22d8}",
		">>"	=> "\u{226b}",
		">>>"	=> "\u{22d9}",
		"<="	=> "\u{2264}",
		">="	=> "\u{2265}",
		"->"	=> "\u{2192}",
		"-->"	=> "\u{27f6}",
		"|->"	=> "\u{21a6}",
		">->"	=> "\u{21a3}",
		"->>"	=> "\u{21a0}",
		"<-"	=> "\u{2190}",
		"<--"	=> "\u{27f5}",
		"<-<"	=> "\u{21a2}",
		"<<-"	=> "\u{219e}",
		"<->"	=> "\u{2194}",
		"<-->"	=> "\u{27f7}",
		"~>"	=> "\u{21dd}",
		"~~>"	=> "\u{27ff}",
		"<~"	=> "\u{21dc}",
		"<~~"	=> "\u{2b33}",
		"=>"	=> "\u{21d2}",
		"|=>"	=> "\u{2907}",
		"==>"	=> "\u{27f9}",
		"<=="	=> "\u{27f8}",
		"<=>"	=> "\u{21d4}",
		"<==>"	=> "\u{27fa}",
		"[|"	=> "\u{27e6}",
		"|]"	=> "\u{27e7}",
		"||"	=> "\u{2016}",
		other	=> other,
	}
}

/// A name a function value carries as a scope member: `list.item`, `assert.eq`, `str.from-unicode`.
pub fn func_field(f: &Func, name: &str) -> Option<Func> {
	match f {
		Func::Element(k)	=> match k.scoped(name) {
			Some(e)	=> Some(Func::Element(e)),
			None	=> lib::visual::scoped(*k, name).map(|v| Func::Native(NativeFunc::Visual(v))),
		},
		Func::Native(n)		=> match foundations::func_scope(f, name) {
			Some(Value::Func(g))	=> Some(g),
			_						=> foundations::constructed_type(*n)
				.and_then(|t| methods::type_method(t, name)).map(Func::Native),
		},
		Func::With(w)		=> func_field(&w.0, name),
		Func::Closure(_)	=> None,
	}
}

// Walks a place to the value it names. The error carries a span, message and optional hint.
fn slot<'b>(
	frames:		&'b mut [Frame],
	base:		&Scope,
	in_func:	bool,
	root:		&str,
	root_span:	Span,
	steps:		&[Step],
	create_last:	bool,
)
	-> std::result::Result<&'b mut Value, (Span, String, Option<String>)>
{
	let mut cur = match frames.iter_mut().rev().find_map(|f| f.get_mut(root)) {
		Some(b)	=> &mut b.value,
		None	=> {
			let msg = if in_func && base.map.contains_key(root) {
				"variables from outside the function are read-only and cannot be modified".to_string()
			} else if base.get(root).is_some() {
				fmt!("cannot mutate a constant: {}", root)
			} else {
				fmt!("unknown variable: {}", root)
			};
			return Err((root_span, msg, None));
		}
	};
	let n = steps.len();
	for (i, step) in steps.iter().enumerate() {
		match step {
			Step::Key(k, sp) => {
				let ty = cur.ty();
				match cur {
					Value::Dict(d) => {
						let d = Arc::make_mut(d);
						if create_last && i + 1 == n && !d.contains(k) {
							d.insert(k, Value::None);
						}
						cur = match d.get_mut(k) {
							Some(v)	=> v,
							None	=> return Err((*sp, fmt!("dictionary does not contain key \"{}\"", k), None)),
						};
					}
					Value::Symbol(_) | Value::Content(_) | Value::Module(_) | Value::Func(_) => return Err((*sp,
						fmt!("cannot mutate fields on {}", ty.long_name()), None)),
					Value::Length(_) | Value::Relative(_) | Value::Alignment(_) | Value::Stroke(_)
						| Value::Version(_) => return Err((*sp,
						fmt!("fields on {} are not yet mutable", ty.long_name()),
						Some(fmt!("try creating a new {} with the updated field value instead", ty.long_name())))),
					_ => return Err((*sp, fmt!("{} does not have accessible fields", ty.long_name()), None)),
				}
			}
			Step::At(key, sp) => {
				let ty = cur.ty();
				match (cur, key) {
					(Value::Array(a), Value::Int(i)) => {
						let a = Arc::make_mut(a);
						let len = a.len();
						let idx = if *i < 0 { len as i64 + i } else { *i };
						if idx < 0 || idx >= len as i64 {
							return Err((*sp, fmt!("array index out of bounds (index: {}, len: {})", i, len), None));
						}
						cur = match a.get_mut(idx as usize) {
							Some(v)	=> v,
							None	=> return Err((*sp, "array index out of bounds".to_string(), None)),
						};
					}
					(Value::Dict(d), Value::Str(k)) => {
						let d = Arc::make_mut(d);
						cur = match d.get_mut(k) {
							Some(v)	=> v,
							None	=> return Err((*sp, fmt!("dictionary does not contain key \"{}\"", k),
								Some("use `insert` to add or update values".to_string()))),
						};
					}
					(Value::Array(_), other) => return Err((*sp,
						fmt!("expected integer, found {}", other.ty().long_name()), None)),
					(Value::Dict(_), other) => return Err((*sp,
						fmt!("expected string, found {}", other.ty().long_name()), None)),
					_ => return Err((*sp, fmt!("cannot mutate a temporary value of type {}", ty.long_name()), None)),
				}
			}
		}
	}
	Ok(cur)
}

impl<'a> Vm<'a> {
	fn new(
		engine:		&'a mut Engine,
		base:		&'a Scope,
		lib:		Arc<Scope>,
		file:		FileId,
		in_func:	bool,
		fallback:	Span,
	)
		-> Self
	{
		Self { engine, base, lib, frames: vec![Frame::new()], flow: None, file, in_func, fallback }
	}

	fn fix(&self, span: Span) -> Span {
		if span.is_detached() { self.fallback } else { span }
	}

	fn error<S: Into<String>>(&mut self, kind: DiagnosticKind, span: Span, msg: S) -> Error<ErrTag> {
		let span = self.fix(span);
		self.engine.error(kind, span, msg)
	}

	fn error_hint<S: Into<String>, H: Into<String>>(
		&mut self,
		kind:	DiagnosticKind,
		span:	Span,
		msg:	S,
		hint:	H,
	)
		-> Error<ErrTag>
	{
		let span = self.fix(span);
		self.engine.error_hint(kind, span, msg, hint)
	}

	fn warn<S: Into<String>>(&mut self, kind: DiagnosticKind, span: Span, msg: S) {
		let span = self.fix(span);
		self.engine.warn(kind, span, msg);
	}

	// A library error (an operator, a cast) reported at the expression that caused it.
	fn fail(&mut self, span: Span, e: Error<ErrTag>) -> Error<ErrTag> {
		let msg = e.plain();
		self.error(native_kind(&e), span, msg)
	}

	fn define(&mut self, name: &str, value: Value, span: Span) {
		if let Some(f) = self.frames.last_mut() {
			f.insert(name.to_string(), Binding { value, span });
		}
	}

	fn lookup(&self, name: &str) -> Option<&Value> {
		for f in self.frames.iter().rev() {
			if let Some(b) = f.get(name) {
				return Some(&b.value);
			}
		}
		self.base.get(name)
	}

	// A name bound above the library: a local, a capture, or `eval`'s scope.
	fn lookup_local(&self, name: &str) -> Option<&Value> {
		for f in self.frames.iter().rev() {
			if let Some(b) = f.get(name) {
				return Some(&b.value);
			}
		}
		let mut s = self.base;
		while let Some(parent) = &s.parent {
			if let Some(b) = s.map.get(name) {
				return Some(&b.value);
			}
			s = parent;
		}
		None
	}

	// Maths resolves free names in the `math` module rather than the global library.
	fn lookup_math(&self, name: &str) -> Option<Value> {
		if let Some(v) = self.lookup_local(name) {
			return Some(v.clone());
		}
		match self.lib.get("math") {
			Some(Value::Module(m))	=> m.scope.get(name).cloned(),
			_						=> None,
		}
	}

	fn capture(&self, node: &SyntaxNode) -> Scope {
		let mut names = HashSet::new();
		collect_idents(node, &mut names);
		let mut map = HashMap::new();
		for n in names {
			if let Some(v) = self.lookup_local(&n) {
				map.insert(n, Binding { value: v.clone(), span: Span::detached() });
			}
		}
		Scope { map, parent: Some(self.lib.clone()) }
	}

	fn forbid_flow(&mut self) -> Outcome<()> {
		match self.flow.take() {
			None						=> Ok(()),
			Some(Flow::Break(s))		=> Err(self.error(DiagnosticKind::Syntax, s, "cannot break outside of loop")),
			Some(Flow::Continue(s))		=> Err(self.error(DiagnosticKind::Syntax, s, "cannot continue outside of loop")),
			Some(Flow::Return(s, _, _))	=> Err(self.error(DiagnosticKind::Syntax, s, "cannot return outside of function")),
		}
	}

	fn elem(&mut self, kind: ElemKind, fields: Vec<(&str, Value)>, span: Span) -> Outcome<Content> {
		content::build(self.engine, kind, fields, span)
	}

	fn display(&mut self, v: Value, span: Span) -> Outcome<Content> {
		content::display(self.engine, v, span)
	}

	fn expect_bool(&mut self, node: &SyntaxNode) -> Outcome<bool> {
		match res!(self.eval(node)) {
			Value::Bool(b)	=> Ok(b),
			other			=> Err(self.error(DiagnosticKind::Type, node.span(), fmt!("expected boolean, found {}", other.ty().long_name()))),
		}
	}

	// Blocks

	fn eval_markup(&mut self, nodes: &[SyntaxNode]) -> Outcome<Content> {
		let outer = self.flow.take();
		let mut stack: Vec<(Vec<Content>, Pending)> = Vec::new();
		let mut seq: Vec<Content> = Vec::new();
		for node in nodes {
			match node.kind() {
				SyntaxKind::SetRule => {
					let styles = res!(self.eval_set(node));
					if self.flow.is_some() {
						break;
					}
					stack.push((std::mem::take(&mut seq), Pending::Styles(styles)));
				}
				SyntaxKind::ShowRule => {
					let recipe = res!(self.eval_show(node));
					if self.flow.is_some() {
						break;
					}
					stack.push((std::mem::take(&mut seq), Pending::Recipe(recipe)));
				}
				k if is_expr(k) => match res!(self.eval(node)) {
					Value::Label(label)	=> self.attach_label(&mut seq, label, node.span()),
					v					=> {
						let c = res!(self.display(v, node.span()));
						seq.push(c);
					}
				},
				_ => (),
			}
			if self.flow.is_some() {
				break;
			}
		}
		let mut content = Content::sequence(seq);
		while let Some((mut prev, pending)) = stack.pop() {
			let tail = res!(self.apply_pending(content, pending));
			prev.push(tail);
			content = Content::sequence(prev);
		}
		if outer.is_some() {
			self.flow = outer;
		}
		Ok(content)
	}

	fn attach_label(&mut self, seq: &mut [Content], label: Label, span: Span) {
		match seq.iter_mut().rev().find(|c| !ops::unlabellable(c)) {
			Some(c) => {
				if c.label().is_some() {
					self.warn(DiagnosticKind::Lint, c.span(), "content labelled multiple times");
				}
				*c = std::mem::take(c).labelled(label);
			}
			None => self.warn(DiagnosticKind::Lint, span, fmt!("label `<{}>` is not attached to anything", label.as_str())),
		}
	}

	fn eval_code(&mut self, nodes: &[SyntaxNode]) -> Outcome<Value> {
		let outer = self.flow.take();
		let mut stack: Vec<(Value, Span, Pending)> = Vec::new();
		let mut output = Value::None;
		for node in nodes {
			let span = node.span();
			match node.kind() {
				SyntaxKind::SetRule => {
					let styles = res!(self.eval_set(node));
					if self.flow.is_some() {
						break;
					}
					stack.push((std::mem::take(&mut output), span, Pending::Styles(styles)));
				}
				SyntaxKind::ShowRule => {
					let recipe = res!(self.eval_show(node));
					if self.flow.is_some() {
						break;
					}
					stack.push((std::mem::take(&mut output), span, Pending::Recipe(recipe)));
				}
				k if is_code_expr(k) => {
					let v = res!(self.eval(node));
					output = match ops::join(output, v) {
						Ok(v)	=> v,
						Err(e)	=> return Err(self.fail(span, e)),
					};
				}
				_ => (),
			}
			if self.flow.is_some() {
				self.warn_discarded(&output);
				break;
			}
		}
		while let Some((prev, span, pending)) = stack.pop() {
			let tail = res!(self.display(output, span));
			let styled = res!(self.apply_pending(tail, pending));
			output = match ops::join(prev, Value::Content(styled)) {
				Ok(v)	=> v,
				Err(e)	=> return Err(self.fail(span, e)),
			};
		}
		if outer.is_some() {
			self.flow = outer;
		}
		Ok(output)
	}

	fn apply_pending(&mut self, content: Content, pending: Pending) -> Outcome<Content> {
		match pending {
			Pending::Styles(s)	=> Ok(content.styled(s)),
			Pending::Recipe(r)	=> styles::styled_with_recipe(self.engine, content, r),
		}
	}

	fn scoped<T>(&mut self, f: impl FnOnce(&mut Self) -> Outcome<T>) -> Outcome<T> {
		self.frames.push(Frame::new());
		let out = f(self);
		self.frames.pop();
		out
	}

	// Expressions

	fn eval(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		use SyntaxKind as K;
		let span = node.span();
		match node.kind() {
			// Markup
			K::Text			=> Ok(Value::Content(Content::text(node.text()).with_span(span))),
			K::Space		=> Ok(Value::Content(Content::marker(ElemKind::Space, span))),
			K::Linebreak	=> Ok(Value::Content(Content::marker(ElemKind::Linebreak, span))),
			K::Parbreak		=> Ok(Value::Content(Content::marker(ElemKind::Parbreak, span))),
			K::Escape		=> Ok(Value::Content(Content::symbol(&unescape_markup(node.text())).with_span(span))),
			K::Shorthand	=> Ok(Value::Content(Content::symbol(markup_shorthand(node.text())).with_span(span))),
			K::SmartQuote	=> self.elem(ElemKind::SmartQuote, vec![
				("double", Value::Bool(node.text() == "\"")),
			], span).map(Value::Content),
			K::Strong		=> self.eval_body_elem(node, ElemKind::Strong),
			K::Emph			=> self.eval_body_elem(node, ElemKind::Emph),
			K::Raw			=> self.eval_raw(node),
			K::Link			=> self.eval_link(node),
			K::Label		=> {
				let t = node.text();
				let name = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(t);
				Ok(Value::Label(Label::new(name)))
			}
			K::Ref			=> self.eval_ref(node),
			K::Heading		=> self.eval_heading(node),
			K::ListItem		=> self.eval_body_elem(node, ElemKind::ListItem),
			K::EnumItem		=> self.eval_enum_item(node),
			K::TermItem		=> self.eval_term_item(node),
			K::Equation		=> self.eval_equation(node),
			K::Markup		=> self.eval_markup(node.children()).map(Value::Content),
			// Maths
			K::Math			=> self.eval_math_seq(node.children()).map(Value::Content),
			// One character is a symbol, a number is text, as Typst's `MathText` evaluates.
			K::MathText		=> Ok(Value::Content(match node.text().chars().next() {
				Some(c) if c.is_numeric()	=> Content::text(node.text()),
				_							=> Content::symbol(node.text()),
			}.with_span(span))),
			K::MathIdent	=> self.eval_math_ident(node),
			K::MathShorthand	=> Ok(Value::Content(Content::symbol(math_shorthand(node.text())).with_span(span))),
			K::MathAlignPoint	=> Ok(Value::Content(Content::marker(ElemKind::MathAlignPoint, span))),
			K::MathDelimited	=> self.eval_math_delimited(node),
			K::MathAttach	=> self.eval_math_attach(node),
			K::MathPrimes	=> {
				// One leaf holds every prime of a run, so its length is the count.
				let n = ast::MathPrimes::from_untyped(node).map(|p| p.count()).unwrap_or(1);
				self.elem(ElemKind::MathPrimes, vec![("count", Value::Int(n as i64))], span).map(Value::Content)
			}
			K::MathFrac		=> self.eval_math_frac(node),
			K::MathRoot		=> self.eval_math_root(node),
			K::MathCall		=> self.eval_math_call(node),
			K::MathFieldAccess	=> self.eval_math_field_access(node),
			// Literals
			K::Ident		=> match self.lookup(node.text()) {
				Some(v)	=> Ok(v.clone()),
				None	=> Err(self.error(DiagnosticKind::UnknownVariable, span, fmt!("unknown variable: {}", node.text()))),
			},
			K::None			=> Ok(Value::None),
			K::Auto			=> Ok(Value::Auto),
			K::Bool			=> Ok(Value::Bool(node.text() == "true")),
			K::Int			=> self.eval_int(node),
			K::Float		=> match node.text().parse::<f64>() {
				Ok(f)	=> Ok(Value::Float(f)),
				Err(_)	=> Err(self.error(DiagnosticKind::Syntax, span, fmt!("invalid floating point number: {}", node.text()))),
			},
			K::Numeric		=> self.eval_numeric(node),
			K::Str			=> Ok(Value::str(ast::unescape_str(node.text()))),
			// Code
			K::Code			=> self.eval_code(node.children()),
			K::CodeBlock	=> match node.child(K::Code) {
				Some(code)	=> {
					let code = code.clone();
					self.scoped(|vm| vm.eval_code(code.children()))
				}
				None		=> {
					let kids: Vec<SyntaxNode> = node.children().to_vec();
					self.scoped(|vm| vm.eval_code(&kids))
				}
			},
			K::ContentBlock	=> {
				let kids: Vec<SyntaxNode> = match node.child(K::Markup) {
					Some(m)	=> m.children().to_vec(),
					None	=> Vec::new(),
				};
				self.scoped(|vm| vm.eval_markup(&kids)).map(Value::Content)
			}
			K::Parenthesized	=> match first_expr(node) {
				Some(e)	=> self.eval(e),
				None	=> Ok(Value::None),
			},
			K::Array		=> self.eval_array(node),
			K::Dict			=> self.eval_dict(node),
			K::Unary		=> self.eval_unary(node),
			K::Binary		=> self.eval_binary(node),
			K::FieldAccess	=> {
				let target = match first_expr(node) {
					Some(t)	=> t,
					None	=> return Err(self.error(DiagnosticKind::Syntax, span, "field access without a target")),
				};
				let tv = res!(self.eval(target));
				let name = ident_text(node).unwrap_or("");
				self.field(tv, name, span)
			}
			K::FuncCall		=> self.eval_call(node),
			K::Closure		=> self.eval_closure(node, None),
			K::LetBinding	=> self.eval_let(node),
			K::SetRule		=> self.eval_set(node).map(Value::Styles),
			K::ShowRule		=> {
				// A show rule outside a block's statement list: its styles, for `show: set ..` nesting.
				let r = res!(self.eval_show(node));
				Ok(Value::Styles(Styles::from_style(Style::Recipe(r))))
			}
			K::Contextual	=> self.eval_context(node),
			K::Conditional	=> self.eval_if(node),
			K::WhileLoop	=> self.eval_while(node),
			K::ForLoop		=> self.eval_for(node),
			K::ModuleImport	=> self.eval_import(node),
			K::ModuleInclude	=> self.eval_include(node),
			K::LoopBreak	=> {
				if self.flow.is_none() {
					self.flow = Some(Flow::Break(span));
				}
				Ok(Value::None)
			}
			K::LoopContinue	=> {
				if self.flow.is_none() {
					self.flow = Some(Flow::Continue(span));
				}
				Ok(Value::None)
			}
			K::FuncReturn	=> {
				let v = match first_expr(node) {
					Some(e)	=> Some(res!(self.eval(e))),
					None	=> None,
				};
				if self.flow.is_none() {
					self.flow = Some(Flow::Return(span, v, false));
				}
				Ok(Value::None)
			}
			K::DestructAssignment	=> {
				let pat = node.children().iter().find(|c| is_pattern(c.kind())).cloned();
				let val = last_expr(node).cloned();
				match (pat, val) {
					(Some(p), Some(v)) => {
						let value = res!(self.eval(&v));
						res!(self.destructure(&p, value, Bind::Assign));
						Ok(Value::None)
					}
					_ => Err(self.error(DiagnosticKind::Syntax, span, "incomplete destructuring assignment")),
				}
			}
			K::Error		=> Err(self.error(DiagnosticKind::Syntax, span, node.error_info()
				.map(|e| e.message.clone()).unwrap_or_else(|| "syntax error".to_string()))),
			other			=> Err(self.error(DiagnosticKind::Syntax, span, fmt!("expected expression, found {}", other.name()))),
		}
	}

	fn eval_int(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let t = node.text();
		let (digits, radix) = if let Some(h) = t.strip_prefix("0x") {
			(h, 16)
		} else if let Some(o) = t.strip_prefix("0o") {
			(o, 8)
		} else if let Some(b) = t.strip_prefix("0b") {
			(b, 2)
		} else {
			(t, 10)
		};
		match i64::from_str_radix(digits, radix) {
			Ok(i)	=> Ok(Value::Int(i)),
			Err(_) if radix == 10 => match t.parse::<f64>() {
				Ok(f)	=> Ok(Value::Float(f)),
				Err(_)	=> Err(self.error(DiagnosticKind::Syntax, node.span(), fmt!("invalid integer: {}", t))),
			},
			Err(_)	=> Err(self.error(DiagnosticKind::Syntax, node.span(), "integer number is too large")),
		}
	}

	fn eval_numeric(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let t = node.text();
		let split = t.char_indices().rev()
			.take_while(|(_, c)| c.is_ascii_alphabetic() || *c == '%')
			.last().map(|(i, _)| i).unwrap_or(t.len());
		let (n, unit) = t.split_at(split);
		let v = match n.parse::<f64>() {
			Ok(v)	=> v,
			Err(_)	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), fmt!("invalid number: {}", t))),
		};
		Ok(match unit {
			"pt"	=> Value::Length(Length::pt(v)),
			"mm"	=> Value::Length(Length::pt(v * 72.0 / 25.4)),
			"cm"	=> Value::Length(Length::pt(v * 72.0 / 2.54)),
			"in"	=> Value::Length(Length::pt(v * 72.0)),
			"em"	=> Value::Length(Length::em(v)),
			"deg"	=> Value::Angle(Angle(v.to_radians())),
			"rad"	=> Value::Angle(Angle(v)),
			"fr"	=> Value::Fraction(Fraction(v)),
			"%"		=> Value::Ratio(Ratio(v / 100.0)),
			other	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), fmt!("invalid unit: {}", other))),
		})
	}

	fn eval_body_elem(&mut self, node: &SyntaxNode, kind: ElemKind) -> Outcome<Value> {
		let body = match node.child(SyntaxKind::Markup) {
			Some(m)	=> res!(self.eval_markup(m.children())),
			None	=> Content::empty(),
		};
		self.elem(kind, vec![("body", Value::Content(body))], node.span()).map(Value::Content)
	}

	fn eval_raw(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let lines: Vec<&str> = node.children().iter()
			.filter(|c| c.kind() == SyntaxKind::Text).map(|c| c.text()).collect();
		let delim = node.child(SyntaxKind::RawDelim).map(|d| d.text().len()).unwrap_or(1);
		let block = delim >= 3 && node.children().iter()
			.any(|c| c.kind() == SyntaxKind::RawTrimmed && c.text().contains(['\n', '\r']));
		let mut fields = vec![
			("text",	Value::str(lines.join("\n"))),
			("block",	Value::Bool(block)),
		];
		if let Some(lang) = node.child(SyntaxKind::RawLang) {
			fields.push(("lang", Value::str(lang.text())));
		}
		self.elem(ElemKind::Raw, fields, node.span()).map(Value::Content)
	}

	fn eval_link(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let url = node.text();
		let shown = url.strip_prefix("mailto:").or_else(|| url.strip_prefix("tel:")).unwrap_or(url);
		let body = Content::text(shown).with_span(node.span());
		self.elem(ElemKind::Link, vec![
			("dest",	Value::str(url)),
			("body",	Value::Content(body)),
		], node.span()).map(Value::Content)
	}

	fn eval_ref(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let marker = node.child(SyntaxKind::RefMarker).map(|m| m.text()).unwrap_or("");
		let target = marker.strip_prefix('@').unwrap_or(marker);
		let mut fields = vec![("target", Value::Label(Label::new(target)))];
		if let Some(block) = node.child(SyntaxKind::ContentBlock) {
			let block = block.clone();
			fields.push(("supplement", res!(self.eval(&block))));
		}
		self.elem(ElemKind::Ref, fields, node.span()).map(Value::Content)
	}

	fn eval_heading(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let depth = node.child(SyntaxKind::HeadingMarker).map(|m| m.text().len()).unwrap_or(1).max(1);
		let body = match node.child(SyntaxKind::Markup) {
			Some(m)	=> res!(self.eval_markup(m.children())),
			None	=> Content::empty(),
		};
		self.elem(ElemKind::Heading, vec![
			("depth",	Value::Int(depth as i64)),
			("body",	Value::Content(body)),
		], node.span()).map(Value::Content)
	}

	fn eval_enum_item(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let number = node.child(SyntaxKind::EnumMarker)
			.and_then(|m| m.text().trim_end_matches('.').parse::<i64>().ok());
		let body = match node.child(SyntaxKind::Markup) {
			Some(m)	=> res!(self.eval_markup(m.children())),
			None	=> Content::empty(),
		};
		let mut fields = Vec::new();
		if let Some(n) = number {
			fields.push(("number", Value::Int(n)));
		}
		fields.push(("body", Value::Content(body)));
		self.elem(ElemKind::EnumItem, fields, node.span()).map(Value::Content)
	}

	fn eval_term_item(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let parts: Vec<SyntaxNode> = node.children().iter()
			.filter(|c| c.kind() == SyntaxKind::Markup).cloned().collect();
		let term = match parts.first() {
			Some(m)	=> res!(self.eval_markup(m.children())),
			None	=> Content::empty(),
		};
		let desc = match parts.get(1) {
			Some(m)	=> res!(self.eval_markup(m.children())),
			None	=> Content::empty(),
		};
		self.elem(ElemKind::TermItem, vec![
			("term",		Value::Content(term)),
			("description",	Value::Content(desc)),
		], node.span()).map(Value::Content)
	}

	// Maths

	fn eval_equation(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let kids = node.children();
		let spaced = |n: Option<&SyntaxNode>| n.map(|c| c.kind() == SyntaxKind::Space).unwrap_or(false);
		let block = kids.len() >= 2 && spaced(kids.get(1)) && spaced(kids.get(kids.len() - 2));
		let body = match node.child(SyntaxKind::Math) {
			Some(m)	=> res!(self.eval_math_seq(m.children())),
			None	=> Content::empty(),
		};
		self.elem(ElemKind::Equation, vec![
			("block",	Value::Bool(block)),
			("body",	Value::Content(body)),
		], node.span()).map(Value::Content)
	}

	fn eval_math_seq(&mut self, nodes: &[SyntaxNode]) -> Outcome<Content> {
		let mut seq = Vec::new();
		for n in nodes {
			if n.kind() == SyntaxKind::Hash || !is_expr(n.kind()) {
				continue;
			}
			let v = res!(self.eval(n));
			seq.push(res!(self.display(v, n.span())));
		}
		Ok(Content::sequence(seq))
	}

	fn eval_math_ident(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let name = node.text();
		match self.lookup_math(name) {
			Some(v)	=> Ok(v),
			None if name.chars().count() > 1 && name.chars().all(char::is_alphabetic) => {
				let spaced: Vec<String> = name.chars().map(|c| c.to_string()).collect();
				Err(self.error_hint(DiagnosticKind::UnknownVariable, node.span(), fmt!("unknown variable: {}", name), fmt!(
					"if you meant to display multiple letters as is, try adding spaces between each letter: `{}`",
					spaced.join(" "))))
			}
			None	=> Err(self.error(DiagnosticKind::UnknownVariable, node.span(), fmt!("unknown variable: {}", name))),
		}
	}

	// A script or fraction operand drops the parentheses that only group it: `x^(a+b)`.
	fn eval_math_operand(&mut self, node: &SyntaxNode) -> Outcome<Content> {
		if node.kind() == SyntaxKind::MathDelimited {
			let kids = node.children();
			let open = kids.first().map(|c| c.text());
			let close = kids.last().map(|c| c.text());
			if open == Some("(") && close == Some(")") {
				return match node.child(SyntaxKind::Math) {
					Some(m)	=> self.eval_math_seq(m.children()),
					None	=> Ok(Content::empty()),
				};
			}
		}
		let v = res!(self.eval(node));
		self.display(v, node.span())
	}

	fn eval_math_delimited(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let kids = node.children();
		let mut body = Content::empty();
		if let Some(open) = kids.first() {
			if open.kind() != SyntaxKind::Math {
				let v = res!(self.eval(open));
				body = ops::content_add(body, res!(self.display(v, open.span())));
			}
		}
		if let Some(m) = node.child(SyntaxKind::Math) {
			let inner = res!(self.eval_math_seq(m.children()));
			body = ops::content_add(body, inner);
		}
		if kids.len() > 1 {
			if let Some(close) = kids.last() {
				if close.kind() != SyntaxKind::Math {
					let v = res!(self.eval(close));
					body = ops::content_add(body, res!(self.display(v, close.span())));
				}
			}
		}
		self.elem(ElemKind::MathLr, vec![("body", Value::Content(body))], node.span()).map(Value::Content)
	}

	fn eval_math_attach(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let kids = node.children();
		let base = match kids.first() {
			Some(b)	=> {
				let v = res!(self.eval(b));
				res!(self.display(v, b.span()))
			}
			None	=> Content::empty(),
		};
		let mut fields = vec![("base", Value::Content(base))];
		let mut i = 1;
		while i < kids.len() {
			match kids[i].kind() {
				SyntaxKind::Hat | SyntaxKind::Underscore => {
					let name = if kids[i].kind() == SyntaxKind::Hat { "t" } else { "b" };
					if let Some(operand) = kids[i + 1..].iter().find(|c| is_expr(c.kind()) && c.kind() != SyntaxKind::Space) {
						let c = res!(self.eval_math_operand(operand));
						fields.push((name, Value::Content(c)));
					}
					i += 2;
				}
				SyntaxKind::MathPrimes => {
					let p = kids[i].clone();
					fields.push(("tr", res!(self.eval(&p))));
					i += 1;
				}
				_ => i += 1,
			}
		}
		self.elem(ElemKind::MathAttach, fields, node.span()).map(Value::Content)
	}

	fn eval_math_frac(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let parts: Vec<SyntaxNode> = node.children().iter()
			.filter(|c| is_expr(c.kind()) && c.kind() != SyntaxKind::Space).cloned().collect();
		let (num, denom) = match (parts.first(), parts.last()) {
			(Some(n), Some(d)) if parts.len() >= 2	=> (n.clone(), d.clone()),
			_ => return Err(self.error(DiagnosticKind::Syntax, node.span(), "incomplete fraction")),
		};
		let num = res!(self.eval_math_operand(&num));
		let denom = res!(self.eval_math_operand(&denom));
		self.elem(ElemKind::MathFrac, vec![
			("num",		Value::Content(num)),
			("denom",	Value::Content(denom)),
		], node.span()).map(Value::Content)
	}

	fn eval_math_root(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let index = match node.child(SyntaxKind::Root).map(|r| r.text()) {
			Some("\u{221b}")	=> Some("3"),
			Some("\u{221c}")	=> Some("4"),
			_					=> None,
		};
		let radicand = match node.children().iter().rev().find(|c| is_expr(c.kind()) && c.kind() != SyntaxKind::Space) {
			Some(r)	=> {
				let r = r.clone();
				res!(self.eval_math_operand(&r))
			}
			None	=> Content::empty(),
		};
		let mut fields = Vec::new();
		if let Some(i) = index {
			fields.push(("index", Value::Content(Content::text(i))));
		}
		fields.push(("radicand", Value::Content(radicand)));
		self.elem(ElemKind::MathRoot, fields, node.span()).map(Value::Content)
	}

	// Collections and operators

	fn eval_array(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let mut out = Vec::new();
		for c in node.children() {
			match c.kind() {
				SyntaxKind::Spread => {
					let e = match last_expr(c) {
						Some(e)	=> e.clone(),
						None	=> continue,
					};
					match res!(self.eval(&e)) {
						Value::None			=> (),
						Value::Array(a)		=> out.extend(a.iter().cloned()),
						Value::Dict(_)		=> return Err(self.error_hint(DiagnosticKind::Type, c.span(),
							"cannot spread dictionary into array",
							fmt!("add a colon to create a dictionary instead: `(: {},)`", c.full_text().trim()))),
						other				=> return Err(self.error(DiagnosticKind::Type, c.span(),
							fmt!("cannot spread {} into array", other.ty().long_name()))),
					}
				}
				k if is_code_expr(k) => out.push(res!(self.eval(c))),
				_ => (),
			}
		}
		Ok(Value::array(out))
	}

	fn eval_dict(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let mut d = Dict::new();
		for c in node.children() {
			match c.kind() {
				SyntaxKind::Named => {
					let name = c.children().iter().find(|x| x.kind() == SyntaxKind::Ident)
						.map(|x| x.text().to_string()).unwrap_or_default();
					let e = match last_expr(c) {
						Some(e)	=> e.clone(),
						None	=> continue,
					};
					let v = res!(self.eval(&e));
					d.insert(&name, v);
				}
				SyntaxKind::Keyed => {
					let (k, e) = match (first_expr(c), last_expr(c)) {
						(Some(k), Some(e))	=> (k.clone(), e.clone()),
						_					=> continue,
					};
					let key = match res!(self.eval(&k)) {
						Value::Str(s)	=> s,
						other			=> return Err(self.error(DiagnosticKind::Type, k.span(),
							fmt!("expected string, found {}", other.ty().long_name()))),
					};
					let v = res!(self.eval(&e));
					d.insert(&key, v);
				}
				SyntaxKind::Spread => {
					let e = match last_expr(c) {
						Some(e)	=> e.clone(),
						None	=> continue,
					};
					match res!(self.eval(&e)) {
						Value::None		=> (),
						Value::Dict(x)	=> for (k, v) in x.iter() {
							d.insert(k, v.clone());
						},
						other			=> return Err(self.error(DiagnosticKind::Type, c.span(),
							fmt!("cannot spread {} into dictionary", other.ty().long_name()))),
					}
				}
				_ => (),
			}
		}
		Ok(Value::dict(d))
	}

	fn eval_unary(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let op = node.children().iter().find(|c| !c.kind().is_trivia()).map(|c| c.kind());
		let operand = match last_expr(node) {
			Some(e)	=> e,
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "missing operand")),
		};
		let v = res!(self.eval(operand));
		let out = match op {
			Some(SyntaxKind::Plus)	=> ops::pos(v),
			Some(SyntaxKind::Minus)	=> ops::neg(v),
			_						=> ops::not(v),
		};
		match out {
			Ok(v)	=> Ok(v),
			Err(e)	=> Err(self.fail(node.span(), e)),
		}
	}

	fn eval_binary(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		use SyntaxKind as K;
		let sig: Vec<&SyntaxNode> = node.significant().collect();
		let (lhs, rhs) = match (sig.first(), sig.last()) {
			(Some(l), Some(r)) if sig.len() >= 3	=> (*l, *r),
			_ => return Err(self.error(DiagnosticKind::Syntax, node.span(), "incomplete binary expression")),
		};
		let op = match (sig.get(1).map(|n| n.kind()), sig.get(2).map(|n| n.kind())) {
			(Some(K::Not), Some(K::In))	=> None,
			(Some(k), _)				=> Some(k),
			_							=> None,
		};
		let span = node.span();
		match op {
			Some(K::Eq)		=> return self.assign_op(lhs, rhs, None),
			Some(K::PlusEq)	=> return self.assign_op(lhs, rhs, Some(ops::add)),
			Some(K::HyphEq)	=> return self.assign_op(lhs, rhs, Some(ops::sub)),
			Some(K::StarEq)	=> return self.assign_op(lhs, rhs, Some(ops::mul)),
			Some(K::SlashEq)	=> return self.assign_op(lhs, rhs, Some(ops::div)),
			Some(K::And)	=> {
				let a = res!(self.eval(lhs));
				if matches!(a, Value::Bool(false)) {
					return Ok(a);
				}
				let b = res!(self.eval(rhs));
				return match ops::and(a, b) {
					Ok(v)	=> Ok(v),
					Err(e)	=> Err(self.fail(span, e)),
				};
			}
			Some(K::Or)		=> {
				let a = res!(self.eval(lhs));
				if matches!(a, Value::Bool(true)) {
					return Ok(a);
				}
				let b = res!(self.eval(rhs));
				return match ops::or(a, b) {
					Ok(v)	=> Ok(v),
					Err(e)	=> Err(self.fail(span, e)),
				};
			}
			_				=> (),
		}
		let a = res!(self.eval(lhs));
		let b = res!(self.eval(rhs));
		let out = match op {
			Some(K::Plus)	=> ops::add(a, b),
			Some(K::Minus)	=> ops::sub(a, b),
			Some(K::Star)	=> ops::mul(a, b),
			Some(K::Slash)	=> ops::div(a, b),
			Some(K::EqEq)	=> Ok(Value::Bool(ops::equal(&a, &b))),
			Some(K::ExclEq)	=> Ok(Value::Bool(!ops::equal(&a, &b))),
			Some(K::Lt)		=> ops::compare(&a, &b).map(|o| Value::Bool(o.is_lt())),
			Some(K::LtEq)	=> ops::compare(&a, &b).map(|o| Value::Bool(o.is_le())),
			Some(K::Gt)		=> ops::compare(&a, &b).map(|o| Value::Bool(o.is_gt())),
			Some(K::GtEq)	=> ops::compare(&a, &b).map(|o| Value::Bool(o.is_ge())),
			Some(K::In)		=> ops::contains(&a, &b).map(Value::Bool),
			None			=> ops::contains(&a, &b).map(|c| Value::Bool(!c)),
			Some(other)		=> return Err(self.error(DiagnosticKind::Syntax, span, fmt!("unknown operator {}", other.name()))),
		};
		match out {
			Ok(v)	=> Ok(v),
			Err(e)	=> Err(self.fail(span, e)),
		}
	}

	// Places and assignment

	fn place(&mut self, node: &SyntaxNode) -> Outcome<(String, Span, Vec<Step>)> {
		match node.kind() {
			SyntaxKind::Ident			=> Ok((node.text().to_string(), node.span(), Vec::new())),
			SyntaxKind::Parenthesized	=> match first_expr(node) {
				Some(e)	=> {
					let e = e.clone();
					self.place(&e)
				}
				None	=> Err(self.error(DiagnosticKind::Syntax, node.span(), "cannot mutate a temporary value")),
			},
			SyntaxKind::FieldAccess => {
				let target = match first_expr(node) {
					Some(t)	=> t.clone(),
					None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "cannot mutate a temporary value")),
				};
				let name = ident_text(node).unwrap_or("").to_string();
				let (root, rs, mut steps) = res!(self.place(&target));
				steps.push(Step::Key(name, node.span()));
				Ok((root, rs, steps))
			}
			SyntaxKind::FuncCall => {
				let callee = first_expr(node).cloned();
				if let Some(c) = callee {
					if c.kind() == SyntaxKind::FieldAccess && ident_text(&c) == Some("at") {
						let mut args = res!(self.eval_args(node.child(SyntaxKind::Args), node.span()));
						let key = match args.items.iter().position(|a| a.name.is_none()) {
							Some(i)	=> args.items.remove(i).value,
							None	=> return Err(self.error(DiagnosticKind::Type, node.span(), "missing argument: index")),
						};
						let target = match first_expr(&c) {
							Some(t)	=> t.clone(),
							None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "cannot mutate a temporary value")),
						};
						let (root, rs, mut steps) = res!(self.place(&target));
						steps.push(Step::At(key, node.span()));
						return Ok((root, rs, steps));
					}
				}
				res!(self.eval(node));
				Err(self.error(DiagnosticKind::Syntax, node.span(), "cannot mutate a temporary value"))
			}
			_ => {
				res!(self.eval(node));
				Err(self.error(DiagnosticKind::Syntax, node.span(), "cannot mutate a temporary value"))
			}
		}
	}

	fn place_error(&mut self, e: (Span, String, Option<String>)) -> Error<ErrTag> {
		match e.2 {
			Some(h)	=> self.error_hint(DiagnosticKind::Type, e.0, e.1, h),
			None	=> self.error(DiagnosticKind::Type, e.0, e.1),
		}
	}

	// Reads the value at a place.
	fn read_place(&mut self, root: &str, rs: Span, steps: &[Step]) -> Outcome<Value> {
		let got = match slot(&mut self.frames, self.base, self.in_func, root, rs, steps, false) {
			Ok(v)	=> Ok(v.clone()),
			Err(e)	=> Err(e),
		};
		match got {
			Ok(v)	=> Ok(v),
			Err(e)	=> Err(self.place_error(e)),
		}
	}

	fn write_place(&mut self, root: &str, rs: Span, steps: &[Step], v: Value, create: bool) -> Outcome<()> {
		let got = match slot(&mut self.frames, self.base, self.in_func, root, rs, steps, create) {
			Ok(s)	=> {
				*s = v;
				Ok(())
			}
			Err(e)	=> Err(e),
		};
		match got {
			Ok(())	=> Ok(()),
			Err(e)	=> Err(self.place_error(e)),
		}
	}

	fn assign_op(
		&mut self,
		lhs:	&SyntaxNode,
		rhs:	&SyntaxNode,
		op:		Option<fn(Value, Value) -> Outcome<Value>>,
	)
		-> Outcome<Value>
	{
		let r = res!(self.eval(rhs));
		let (root, rs, steps) = res!(self.place(lhs));
		match op {
			None	=> {
				// `d.key = v` may create the key; every other place must exist.
				let create = matches!(steps.last(), Some(Step::Key(..)));
				res!(self.write_place(&root, rs, &steps, r, create));
			}
			Some(f)	=> {
				let old = res!(self.read_place(&root, rs, &steps));
				let span = lhs.span().join(rhs.span());
				let new = match f(old, r) {
					Ok(v)	=> v,
					Err(e)	=> return Err(self.fail(span, e)),
				};
				res!(self.write_place(&root, rs, &steps, new, false));
			}
		}
		Ok(Value::None)
	}

	// Bindings and patterns

	fn eval_let(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let pattern = match pattern_after(node, SyntaxKind::Let) {
			Some(p)	=> p.clone(),
			None	=> return Ok(Value::None),
		};
		if pattern.kind() == SyntaxKind::Closure {
			if let Some(name) = pattern.children().iter().find(|c| c.kind() == SyntaxKind::Ident) {
				let name = name.text().to_string();
				let f = res!(self.eval_closure(&pattern, Some(name.clone())));
				self.define(&name, f, pattern.span());
				return Ok(Value::None);
			}
		}
		let init = match expr_after(node, SyntaxKind::Eq) {
			Some(e)	=> {
				let e = e.clone();
				res!(self.eval(&e))
			}
			None	=> Value::None,
		};
		if self.flow.is_some() {
			return Ok(Value::None);
		}
		res!(self.destructure(&pattern, init, Bind::Define));
		Ok(Value::None)
	}

	fn destructure(&mut self, pat: &SyntaxNode, v: Value, mode: Bind) -> Outcome<()> {
		match pat.kind() {
			SyntaxKind::Underscore		=> Ok(()),
			SyntaxKind::Parenthesized	=> match first_expr(pat) {
				Some(inner)	=> {
					let inner = inner.clone();
					self.destructure(&inner, v, mode)
				}
				None		=> Ok(()),
			},
			SyntaxKind::Destructuring	=> match v {
				Value::Array(a)	=> self.destructure_array(pat, &a, mode),
				Value::Dict(d)	=> self.destructure_dict(pat, &d, mode),
				other			=> Err(self.error(DiagnosticKind::Type, pat.span(), fmt!("cannot destructure {}", other.ty().long_name()))),
			},
			SyntaxKind::Ident if mode == Bind::Define => {
				self.define(pat.text(), v, pat.span());
				Ok(())
			}
			_ if mode == Bind::Assign	=> {
				let (root, rs, steps) = res!(self.place(pat));
				let create = matches!(steps.last(), Some(Step::Key(..)));
				self.write_place(&root, rs, &steps, v, create)
			}
			_ => Err(self.error(DiagnosticKind::Syntax, pat.span(), "cannot assign to this expression")),
		}
	}

	fn pattern_items(pat: &SyntaxNode) -> Vec<SyntaxNode> {
		pat.children().iter().filter(|c| {
			matches!(c.kind(), SyntaxKind::Named | SyntaxKind::Spread) || is_pattern(c.kind())
		}).cloned().collect()
	}

	fn wrong_count(&mut self, pat: &SyntaxNode, items: &[SyntaxNode], len: usize) -> Error<ErrTag> {
		let count = items.iter().filter(|i| !matches!(i.kind(), SyntaxKind::Spread | SyntaxKind::Named)).count();
		let spread = items.iter().any(|i| i.kind() == SyntaxKind::Spread);
		let quantifier = if len > count { "too many" } else { "not enough" };
		let expected = match (spread, count) {
			(true, 1)	=> "at least 1 element".to_string(),
			(true, c)	=> fmt!("at least {} elements", c),
			(false, 0)	=> "an empty array".to_string(),
			(false, 1)	=> "a single element".to_string(),
			(false, c)	=> fmt!("{} elements", c),
		};
		self.error_hint(DiagnosticKind::Type, pat.span(), fmt!("{} elements to destructure", quantifier), fmt!(
			"the provided array has a length of {}, but the pattern expects {}", len, expected))
	}

	fn destructure_array(&mut self, pat: &SyntaxNode, a: &[Value], mode: Bind) -> Outcome<()> {
		let items = Self::pattern_items(pat);
		let len = a.len();
		let mut i = 0;
		for item in &items {
			match item.kind() {
				SyntaxKind::Spread => {
					let size = (1 + len).checked_sub(items.len());
					let sink = size.and_then(|s| a.get(i..i + s));
					let (size, sink) = match (size, sink) {
						(Some(s), Some(k))	=> (s, k.to_vec()),
						_					=> return Err(self.wrong_count(pat, &items, len)),
					};
					if let Some(target) = item.children().iter().rev().find(|c| is_pattern(c.kind())) {
						let target = target.clone();
						res!(self.destructure(&target, Value::array(sink), mode));
					}
					i += size;
				}
				SyntaxKind::Named => {
					return Err(self.error(DiagnosticKind::Type, item.span(), "cannot destructure named pattern from an array"));
				}
				_ => {
					let v = match a.get(i) {
						Some(v)	=> v.clone(),
						None	=> return Err(self.wrong_count(pat, &items, len)),
					};
					res!(self.destructure(item, v, mode));
					i += 1;
				}
			}
		}
		if i < len {
			return Err(self.wrong_count(pat, &items, len));
		}
		Ok(())
	}

	fn destructure_dict(&mut self, pat: &SyntaxNode, d: &Dict, mode: Bind) -> Outcome<()> {
		let items = Self::pattern_items(pat);
		let mut used: HashSet<String> = HashSet::new();
		let mut sink: Option<Option<SyntaxNode>> = None;
		for item in &items {
			match item.kind() {
				SyntaxKind::Ident => {
					let name = item.text();
					let v = match d.get(name) {
						Some(v)	=> v.clone(),
						None	=> return Err(self.error(DiagnosticKind::Type, item.span(),
							fmt!("dictionary does not contain key \"{}\"", name))),
					};
					res!(self.destructure(item, v, mode));
					used.insert(name.to_string());
				}
				SyntaxKind::Named => {
					let key = match item.children().iter().find(|c| c.kind() == SyntaxKind::Ident) {
						Some(k)	=> k.clone(),
						None	=> continue,
					};
					let v = match d.get(key.text()) {
						Some(v)	=> v.clone(),
						None	=> return Err(self.error(DiagnosticKind::Type, key.span(),
							fmt!("dictionary does not contain key \"{}\"", key.text()))),
					};
					let sub = item.children().iter().rev().find(|c| is_pattern(c.kind())).cloned();
					if let Some(sub) = sub {
						res!(self.destructure(&sub, v, mode));
					}
					used.insert(key.text().to_string());
				}
				SyntaxKind::Spread	=> sink = Some(item.children().iter().rev()
					.find(|c| is_pattern(c.kind())).cloned()),
				_ => return Err(self.error(DiagnosticKind::Type, item.span(), "cannot destructure unnamed pattern from dictionary")),
			}
		}
		if let Some(Some(target)) = sink {
			let mut rest = Dict::new();
			for (k, v) in d.iter() {
				if !used.contains(k) {
					rest.insert(k, v.clone());
				}
			}
			res!(self.destructure(&target, Value::dict(rest), mode));
		}
		Ok(())
	}

	// Functions

	fn eval_closure(&mut self, node: &SyntaxNode, name: Option<String>) -> Outcome<Value> {
		let mut params = Vec::new();
		if let Some(ps) = node.child(SyntaxKind::Params) {
			let ps = ps.clone();
			for p in ps.children() {
				match p.kind() {
					SyntaxKind::Ident | SyntaxKind::Underscore | SyntaxKind::Destructuring
						| SyntaxKind::Parenthesized => params.push(Param::Pos(p.clone())),
					SyntaxKind::Named => {
						let pname = p.children().iter().find(|c| c.kind() == SyntaxKind::Ident)
							.map(|c| c.text().to_string()).unwrap_or_default();
						let default = match last_expr(p) {
							Some(e)	=> res!(self.eval(e)),
							None	=> Value::None,
						};
						params.push(Param::Named { name: pname, default });
					}
					SyntaxKind::Spread => params.push(Param::Sink(
						p.children().iter().find(|c| c.kind() == SyntaxKind::Ident).map(|c| c.text().to_string()))),
					_ => (),
				}
			}
		}
		let body = match node.children().iter().skip_while(|c| !matches!(c.kind(), SyntaxKind::Arrow | SyntaxKind::Eq))
			.skip(1).find(|c| is_code_expr(c.kind()))
		{
			Some(b)	=> b.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "closure has no body")),
		};
		let captured = self.capture(node);
		Ok(Value::Func(Func::Closure(Arc::new(Closure { name, params, body, captured, span: node.span() }))))
	}

	fn bind_params(&mut self, params: &[Param], mut args: Args) -> Outcome<()> {
		let span = args.span;
		let num_pos = params.iter().filter(|p| matches!(p, Param::Pos(_))).count();
		let sink_size = args.pos_count().checked_sub(num_pos);
		let mut sink: Option<(Option<String>, Vec<Arg>)> = None;
		for p in params {
			match p {
				Param::Pos(pat) => {
					let v = match args.items.iter().position(|a| a.name.is_none()) {
						Some(i)	=> args.items.remove(i).value,
						None	=> {
							let what = if pat.kind() == SyntaxKind::Ident { pat.text() } else { "pattern parameter" };
							return Err(self.error(DiagnosticKind::Type, span, fmt!("missing argument: {}", what)));
						}
					};
					res!(self.destructure(pat, v, Bind::Define));
				}
				Param::Named { name, default } => {
					let v = match res!(args.named::<Value>(name)) {
						Some(v)	=> v,
						None	=> default.clone(),
					};
					self.define(name, v, span);
				}
				Param::Sink(n) => {
					let mut taken = Vec::new();
					if let Some(k) = sink_size {
						for _ in 0..k {
							match args.items.iter().position(|a| a.name.is_none()) {
								Some(i)	=> taken.push(args.items.remove(i)),
								None	=> break,
							}
						}
					}
					sink = Some((n.clone(), taken));
				}
			}
		}
		if let Some((name, pos)) = sink {
			let mut rest = args.take();
			rest.items.extend(pos);
			if let Some(n) = name {
				self.define(&n, Value::Args(Arc::new(rest)), span);
			}
		}
		match args.items.first() {
			None								=> Ok(()),
			Some(Arg { name: Some(n), .. })		=> {
				let msg = fmt!("unexpected argument: {}", n);
				Err(self.error(DiagnosticKind::Type, span, msg))
			}
			Some(_)								=> Err(self.error(DiagnosticKind::Type, span, "unexpected argument")),
		}
	}

	fn eval_closure_body(&mut self, body: &SyntaxNode) -> Outcome<Value> {
		let out = res!(self.eval(body));
		match self.flow.take() {
			None							=> Ok(out),
			Some(Flow::Return(_, Some(v), _))	=> Ok(v),
			Some(Flow::Return(_, None, _))		=> Ok(out),
			Some(Flow::Break(s))			=> Err(self.error(DiagnosticKind::Syntax, s, "cannot break outside of loop")),
			Some(Flow::Continue(s))			=> Err(self.error(DiagnosticKind::Syntax, s, "cannot continue outside of loop")),
		}
	}

	fn eval_args(&mut self, node: Option<&SyntaxNode>, span: Span) -> Outcome<Args> {
		// A call in a string given to `eval` has no span of its own; it takes the call site's, whose file
		// a path in the call resolves against, as in Typst.
		let mut args = Args::new(self.fix(span));
		let node = match node {
			Some(n)	=> n.clone(),
			None	=> return Ok(args),
		};
		for c in node.children() {
			match c.kind() {
				SyntaxKind::Named => {
					let name = c.children().iter().find(|x| x.kind() == SyntaxKind::Ident)
						.map(|x| x.text().to_string()).unwrap_or_default();
					let (v, at) = match last_expr(c) {
						Some(e)	=> (res!(self.eval(e)), e.span()),
						None	=> (Value::None, c.span()),
					};
					args.push_named_at(self.fix(c.span()), self.fix(at), name, v);
				}
				SyntaxKind::Spread => {
					let v = match last_expr(c) {
						Some(e)	=> res!(self.eval(e)),
						None	=> Value::None,
					};
					match v {
						Value::None		=> (),
						Value::Array(a)	=> for x in a.iter() {
							args.push(self.fix(c.span()), x.clone());
						},
						Value::Dict(d)	=> for (k, x) in d.iter() {
							args.push_named(self.fix(c.span()), k, x.clone());
						},
						Value::Args(a)	=> args.items.extend(a.items.iter().cloned()),
						other			=> return Err(self.error(DiagnosticKind::Type, c.span(),
							fmt!("cannot spread {}", other.ty().long_name()))),
					}
				}
				k if is_code_expr(k) => {
					let v = res!(self.eval(c));
					args.push(self.fix(c.span()), v);
				}
				_ => (),
			}
		}
		Ok(args)
	}

	fn eval_call(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let span = node.span();
		let callee = match first_expr(node) {
			Some(c)	=> c.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, span, "call without a callee")),
		};
		let args_node = node.child(SyntaxKind::Args).cloned();
		if callee.kind() == SyntaxKind::FieldAccess {
			let target = match first_expr(&callee) {
				Some(t)	=> t.clone(),
				None	=> return Err(self.error(DiagnosticKind::Syntax, span, "field access without a target")),
			};
			let field = ident_text(&callee).unwrap_or("").to_string();
			if methods::is_mutating(&field) && matches!(target.kind(),
				SyntaxKind::Ident | SyntaxKind::FieldAccess | SyntaxKind::FuncCall | SyntaxKind::Parenthesized)
			{
				let args = res!(self.eval_args(args_node.as_ref(), span));
				let (root, rs, steps) = res!(self.place(&target));
				let got = match slot(&mut self.frames, self.base, self.in_func, &root, rs, &steps, false) {
					Ok(place)	=> Ok(methods::call_method_mut(self.engine, place, &field, args, span)),
					Err(e)		=> Err(e),
				};
				return match got {
					Ok(r)	=> r,
					Err(e)	=> Err(self.place_error(e)),
				};
			}
			let tv = res!(self.eval(&target));
			let mut args = res!(self.eval_args(args_node.as_ref(), span));
			if methods::is_mutating(&field) && matches!(tv, Value::Array(_) | Value::Dict(_)) {
				return Err(self.error(DiagnosticKind::Syntax, span, "cannot mutate a temporary value"));
			}
			if let Some(f) = methods::type_method(tv.ty(), &field) {
				args.prepend(target.span(), tv);
				return self.engine.call_func(&Func::Native(f), args);
			}
			return match tv {
				Value::Symbol(_) | Value::Func(_) | Value::Type(_) | Value::Module(_) => {
					let fv = res!(self.field(tv, &field, callee.span()));
					self.call_value(fv, args, &callee, args_node.as_ref(), span)
				}
				Value::Dict(ref d) if matches!(d.get(&field), Some(Value::Func(_))) => Err(self.error_hint(DiagnosticKind::Type, span,
					"cannot directly call dictionary keys as functions", fmt!(
					"to call the stored function, wrap the field access in parentheses: `({})(..)`",
					callee.full_text().trim()))),
				other => Err(self.error(DiagnosticKind::Type, callee.span(),
					fmt!("type {} has no method `{}`", other.ty().long_name(), field))),
			};
		}
		let cv = res!(self.eval(&callee));
		let args = res!(self.eval_args(args_node.as_ref(), span));
		self.call_value(cv, args, &callee, args_node.as_ref(), span)
	}

	fn call_value(
		&mut self,
		callee:		Value,
		mut args:	Args,
		callee_node:	&SyntaxNode,
		args_node:	Option<&SyntaxNode>,
		span:		Span,
	)
		-> Outcome<Value>
	{
		match callee {
			Value::Func(f)	=> self.engine.call_func(&f, args),
			Value::Type(t)	=> match foundations::constructor(t) {
				Some(f)	=> self.engine.call_func(&Func::Native(f), args),
				None	=> Err(self.error(DiagnosticKind::Type, callee_node.span(), fmt!("type {} is not callable", t.name()))),
			},
			other if in_math(callee_node) => {
				// `$f(x)$` with a non-function `f`: the callee, then its arguments in parentheses.
				let head = res!(self.display(other, callee_node.span()));
				let mut body = Content::text("(");
				let pos = res!(args.all::<Value>());
				let n = pos.len();
				for (i, v) in pos.into_iter().enumerate() {
					if i > 0 {
						body = ops::content_add(body, Content::text(","));
					}
					let c = res!(self.display(v, span));
					body = ops::content_add(body, c);
				}
				let trailing = args_node.map(|a| {
					a.children().iter().rev().find(|c| !c.kind().is_trivia() && c.kind() != SyntaxKind::RightParen)
						.map(|c| c.kind() == SyntaxKind::Comma).unwrap_or(false)
				}).unwrap_or(false);
				if trailing && n > 0 {
					body = ops::content_add(body, Content::text(","));
				}
				body = ops::content_add(body, Content::text(")"));
				let lr = res!(self.elem(ElemKind::MathLr, vec![("body", Value::Content(body))], span));
				Ok(Value::Content(Content::sequence(vec![head, lr])))
			}
			other => Err(self.error(DiagnosticKind::Type, callee_node.span(),
				fmt!("expected function, found {}", other.ty().long_name()))),
		}
	}

	/// `value.name`: a symbol modifier, a dictionary key, a content field, a module, function or type
	/// member, or one of the few fields plain values carry.
	fn field(&mut self, v: Value, name: &str, span: Span) -> Outcome<Value> {
		let ty = v.ty();
		let found = match &v {
			Value::Symbol(s)	=> match ops::symbol_modified(s, name) {
				Some(m)	=> Some(Value::Symbol(m)),
				None	=> return Err(self.error(DiagnosticKind::Type, span, "unknown symbol modifier")),
			},
			Value::Version(_)	=> match foundations::field(&v, name) {
				Some(x)	=> Some(x),
				None	=> return Err(self.error(DiagnosticKind::Type, span, "unknown version component")),
			},
			Value::Dict(d)		=> match d.get(name) {
				Some(v)	=> Some(v.clone()),
				None	=> return Err(self.error(DiagnosticKind::Type, span, fmt!("dictionary does not contain key \"{}\"", name))),
			},
			Value::Content(c)	=> match methods::content_field(c, name) {
				Some(v)	=> Some(v),
				None	=> return Err(self.error(DiagnosticKind::Type, span, fmt!(
					"{} does not have field \"{}\"", methods::content_name(c), name))),
			},
			Value::Type(t)		=> match foundations::type_scope(*t, name) {
				Some(x)	=> Some(x),
				None	=> return Err(self.error(DiagnosticKind::Type, span, fmt!("type {} does not contain field `{}`", t.long_name(), name))),
			},
			Value::Func(f)		=> match func_field(f, name) {
				Some(g)	=> Some(Value::Func(g)),
				None	=> return Err(self.error(DiagnosticKind::Type, span, match f.name() {
					Some(n)	=> fmt!("function `{}` does not contain field `{}`", n, name),
					None	=> fmt!("function does not contain field `{}`", name),
				})),
			},
			Value::Module(m)	=> match m.scope.get(name) {
				Some(v)	=> Some(v.clone()),
				None	=> return Err(self.error(DiagnosticKind::UnknownVariable, span, fmt!("module `{}` does not contain `{}`", m.name, name))),
			},
			Value::Length(_) | Value::Relative(_) | Value::Alignment(_) | Value::Stroke(_)
								=> foundations::field(&v, name),
			_ => return Err(self.error(DiagnosticKind::Type, span, fmt!("cannot access fields on type {}", ty.long_name()))),
		};
		match found {
			Some(v)	=> Ok(v),
			None	=> Err(self.error(DiagnosticKind::Type, span, fmt!("{} does not contain field \"{}\"", ty.long_name(), name))),
		}
	}

	// Rules

	fn eval_set(&mut self, node: &SyntaxNode) -> Outcome<Styles> {
		if let Some(cond) = expr_after(node, SyntaxKind::If) {
			let cond = cond.clone();
			if !res!(self.expect_bool(&cond)) {
				return Ok(Styles::new());
			}
		}
		let target = match expr_after(node, SyntaxKind::Set) {
			Some(t)	=> t.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "set rule without a target")),
		};
		let tv = res!(self.eval(&target));
		let kind = match &tv {
			Value::Func(f)	=> match f.element() {
				Some(k)	=> k,
				None	=> return Err(self.error(DiagnosticKind::Type, target.span(), "only element functions can be used in set rules")),
			},
			other	=> return Err(self.error(DiagnosticKind::Type, target.span(),
				fmt!("expected function, found {}", other.ty().long_name()))),
		};
		let args = res!(self.eval_args(node.child(SyntaxKind::Args), node.span()));
		let mark = self.engine.diags.len();
		match styles::set_rule(self.engine, kind, args) {
			Ok(s)	=> Ok(s),
			Err(e)	=> {
				let span = self.fix(node.span());
				Err(self.engine.adopt(mark, span, e))
			}
		}
	}

	fn eval_show(&mut self, node: &SyntaxNode) -> Outcome<Recipe> {
		let span = node.span();
		let before_colon: Vec<SyntaxNode> = node.children().iter()
			.take_while(|c| c.kind() != SyntaxKind::Colon)
			.filter(|c| is_code_expr(c.kind())).cloned().collect();
		let selector = match before_colon.first() {
			Some(s)	=> Some(res!(self.eval_selector(s))),
			None	=> None,
		};
		let transform_node = match node.children().iter().skip_while(|c| c.kind() != SyntaxKind::Colon)
			.skip(1).find(|c| is_code_expr(c.kind()))
		{
			Some(t)	=> t.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, span, "show rule without a transformation")),
		};
		let transform = if transform_node.kind() == SyntaxKind::SetRule {
			Transformation::Style(res!(self.eval_set(&transform_node)))
		} else {
			match res!(self.eval(&transform_node)) {
				Value::Content(c)	=> Transformation::Content(c),
				Value::Func(f)		=> Transformation::Func(f),
				Value::Styles(s)	=> Transformation::Style(s),
				v @ (Value::None | Value::Str(_) | Value::Symbol(_))
									=> Transformation::Content(res!(self.display(v, transform_node.span()))),
				other				=> return Err(self.error(DiagnosticKind::Type, transform_node.span(),
					fmt!("expected content or function, found {}", other.ty().long_name()))),
			}
		};
		Ok(Recipe { selector, transform, span })
	}

	fn eval_selector(&mut self, node: &SyntaxNode) -> Outcome<Selector> {
		let v = res!(self.eval(node));
		select::cast_showable(self.engine, node.span(), v)
	}

	fn eval_context(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let body = match last_expr(node) {
			Some(b)	=> b.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "context without a body")),
		};
		let captured = self.capture(&body);
		let closure = Closure { name: None, params: Vec::new(), body: body.clone(), captured, span: body.span() };
		let func = Value::Func(Func::Closure(Arc::new(closure)));
		self.elem(ElemKind::Context, vec![("func", func)], node.span()).map(Value::Content)
	}

	// Control flow

	fn eval_if(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let exprs = code_exprs(node);
		let (cond, then) = match (exprs.first(), exprs.get(1)) {
			(Some(c), Some(t))	=> ((*c).clone(), (*t).clone()),
			_					=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "incomplete conditional")),
		};
		let otherwise = expr_after(node, SyntaxKind::Else).cloned();
		let out = if res!(self.expect_bool(&cond)) {
			res!(self.eval(&then))
		} else {
			match otherwise {
				Some(e)	=> res!(self.eval(&e)),
				None	=> Value::None,
			}
		};
		self.mark_return_conditional();
		Ok(out)
	}

	/// A `return` inside an `if` or a loop may not have run, so it discards nothing for certain.
	fn mark_return_conditional(&mut self) {
		if let Some(Flow::Return(_, _, conditional)) = &mut self.flow {
			*conditional = true;
		}
	}

	/// Typst's warning for a `return` that throws away content the block had already produced.
	fn warn_discarded(&mut self, output: &Value) {
		let span = match &self.flow {
			Some(Flow::Return(span, Some(_), false))	=> *span,
			_											=> return,
		};
		let tree = match output {
			Value::Content(c)	=> c,
			_					=> return,
		};
		let mut d = Diagnostic::warning(DiagnosticKind::Lint, self.fix(span), "this return unconditionally discards the content before it")
			.with_hint("try omitting the `return` to automatically join all values");
		if contains_update(tree, 0) {
			d = d.with_hint("state/counter updates are content that must end up in the document to have an effect");
		}
		self.engine.diags.push(d);
	}

	fn eval_while(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let exprs = code_exprs(node);
		let (cond, body) = match (exprs.first(), exprs.get(1)) {
			(Some(c), Some(b))	=> ((*c).clone(), (*b).clone()),
			_					=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "incomplete while loop")),
		};
		let outer = self.flow.take();
		let mut output = Value::None;
		let mut i = 0;
		while res!(self.expect_bool(&cond)) {
			if i == 0 && is_invariant(&cond) && !can_diverge(&body) {
				return Err(self.error(DiagnosticKind::Limit, cond.span(), "condition is always true"));
			}
			if i >= MAX_WHILE_ITERATIONS {
				return Err(self.error(DiagnosticKind::Limit, node.span(), "loop seems to be infinite"));
			}
			res!(self.burn(node.span()));
			let v = res!(self.eval(&body));
			output = match ops::join(output, v) {
				Ok(v)	=> v,
				Err(e)	=> return Err(self.fail(body.span(), e)),
			};
			match self.flow {
				Some(Flow::Break(_))	=> {
					self.flow = None;
					break;
				}
				Some(Flow::Continue(_))	=> self.flow = None,
				Some(Flow::Return(..))	=> {
					self.mark_return_conditional();
					break;
				}
				None					=> (),
			}
			i += 1;
		}
		if outer.is_some() {
			self.flow = outer;
		}
		Ok(output)
	}

	fn burn(&mut self, span: Span) -> Outcome<()> {
		let span = self.fix(span);
		self.engine.burn(span)
	}

	fn eval_for(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let pattern = match pattern_after(node, SyntaxKind::For) {
			Some(p)	=> p.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "for loop without a pattern")),
		};
		let iterable = match expr_after(node, SyntaxKind::In) {
			Some(e)	=> e.clone(),
			None	=> return Err(self.error(DiagnosticKind::Type, node.span(), "for loop without an iterable")),
		};
		let body = match last_expr(node) {
			Some(b)	=> b.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "for loop without a body")),
		};
		let it = res!(self.eval(&iterable));
		let destructuring = pattern.kind() == SyntaxKind::Destructuring;
		let items: Vec<Value> = match it {
			Value::Array(a)	=> (*a).clone(),
			Value::Dict(d)	=> d.iter().map(|(k, v)| Value::array(vec![Value::str(k), v.clone()])).collect(),
			Value::Str(s) if !destructuring	=> segment::graphemes(&s).into_iter().map(Value::str).collect(),
			Value::Bytes(b) if !destructuring	=> b.iter().map(|x| Value::Int(*x as i64)).collect(),
			v @ (Value::Str(_) | Value::Bytes(_))	=> return Err(self.error(DiagnosticKind::Type, pattern.span(),
				fmt!("cannot destructure values of {}", v.ty().long_name()))),
			other => return Err(self.error(DiagnosticKind::Type, iterable.span(), fmt!("cannot loop over {}", other.ty().long_name()))),
		};
		let outer = self.flow.take();
		let mut output = Value::None;
		self.frames.push(Frame::new());
		let mut result = Ok(());
		for v in items {
			if let Err(e) = self.burn(node.span()) {
				result = Err(e);
				break;
			}
			if let Err(e) = self.destructure(&pattern, v, Bind::Define) {
				result = Err(e);
				break;
			}
			let v = match self.eval(&body) {
				Ok(v)	=> v,
				Err(e)	=> {
					result = Err(e);
					break;
				}
			};
			output = match ops::join(std::mem::take(&mut output), v) {
				Ok(v)	=> v,
				Err(e)	=> {
					result = Err(self.fail(body.span(), e));
					break;
				}
			};
			match self.flow {
				Some(Flow::Break(_))	=> {
					self.flow = None;
					break;
				}
				Some(Flow::Continue(_))	=> self.flow = None,
				Some(Flow::Return(..))	=> {
					self.mark_return_conditional();
					break;
				}
				None					=> (),
			}
		}
		self.frames.pop();
		res!(result);
		if outer.is_some() {
			self.flow = outer;
		}
		Ok(output)
	}

	// Modules

	fn eval_import(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let src_node = match expr_after(node, SyntaxKind::Import) {
			Some(s)	=> s.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "import without a source")),
		};
		let src_span = src_node.span();
		let source = res!(self.eval(&src_node));
		let source = match source {
			Value::Str(spec) => {
				let mark = self.engine.diags.len();
				match import::import_module(self.engine, &spec, self.file, self.fix(src_span)) {
					Ok(m)	=> Value::Module(m),
					Err(e)	=> {
						let span = self.fix(src_span);
						return Err(self.engine.adopt(mark, span, e));
					}
				}
			}
			Value::Func(Func::Closure(_)) => return Err(self.error(DiagnosticKind::Type, src_span, "cannot import from user-defined functions")),
			v @ (Value::Module(_) | Value::Func(_) | Value::Type(_)) => v,
			other => return Err(self.error(DiagnosticKind::Type, src_span, fmt!(
				"expected path, module, function, or type, found {}", other.ty().long_name()))),
		};
		let new_name = node.children().iter().skip_while(|c| c.kind() != SyntaxKind::As)
			.skip(1).find(|c| c.kind() == SyntaxKind::Ident).cloned();
		// `as` directly after the source renames the module; an `as` inside the item list does not.
		let colon_seen_before_as = node.children().iter()
			.position(|c| c.kind() == SyntaxKind::Colon)
			.zip(node.children().iter().position(|c| c.kind() == SyntaxKind::As))
			.map(|(c, a)| c < a).unwrap_or(false);
		let new_name = if colon_seen_before_as { None } else { new_name };
		if let Some(n) = &new_name {
			if src_node.kind() == SyntaxKind::Ident && src_node.text() == n.text() {
				self.engine.warn(DiagnosticKind::Lint, self.fix(n.span()), "unnecessary import rename to same name");
			}
			self.define(n.text(), source.clone(), n.span());
		}
		let imports = node.children().iter().skip_while(|c| c.kind() != SyntaxKind::Colon).skip(1)
			.find(|c| matches!(c.kind(), SyntaxKind::Star | SyntaxKind::ImportItems | SyntaxKind::Ident)).cloned();
		match imports {
			None => {
				if new_name.is_none() {
					let bare = match ast::ModuleImport::from_untyped(node) {
						Some(m)	=> m.bare_name(),
						None	=> Err(BareImportError::Dynamic),
					};
					match bare {
						// `import calc` binds `calc` to itself.
						Ok(_) if src_node.kind() == SyntaxKind::Ident	=> {
							self.engine.warn(DiagnosticKind::Lint, self.fix(src_span), "this import has no effect");
						}
						Ok(b)	=> self.define(&b, source, src_span),
						Err(BareImportError::Dynamic)	=> return Err(self.error_hint(DiagnosticKind::Syntax, src_span,
							"dynamic import requires an explicit name", "you can name the import with `as`")),
						Err(_)	=> return Err(self.error_hint(DiagnosticKind::Syntax, src_span,
							"module name would not be a valid identifier", "you can rename the import with `as`")),
					}
				}
			}
			Some(star) if star.kind() == SyntaxKind::Star => {
				let entries: Vec<(String, Value)> = match &source {
					Value::Module(m)	=> m.scope.iter().map(|(k, b)| (k.clone(), b.value.clone())).collect(),
					Value::Func(Func::Element(k)) => ElemKind::ALL.iter()
						.filter(|c| c.path().strip_prefix(k.path()).map(|r| r.starts_with('.') && !r[1..].contains('.')).unwrap_or(false))
						.map(|c| (c.name().to_string(), Value::Func(Func::Element(*c)))).collect(),
					_ => Vec::new(),
				};
				for (k, v) in entries {
					self.define(&k, v, star.span());
				}
			}
			Some(items) => {
				let list: Vec<SyntaxNode> = if items.kind() == SyntaxKind::Ident {
					vec![items.clone()]
				} else {
					items.children().iter().filter(|c| matches!(c.kind(),
						SyntaxKind::ImportItemPath | SyntaxKind::RenamedImportItem | SyntaxKind::Ident)).cloned().collect()
				};
				let mut failed = None;
				for item in &list {
					let (path, bound): (Vec<SyntaxNode>, SyntaxNode) = match item.kind() {
						SyntaxKind::Ident => (vec![item.clone()], item.clone()),
						SyntaxKind::ImportItemPath => {
							let p: Vec<SyntaxNode> = item.children().iter()
								.filter(|c| c.kind() == SyntaxKind::Ident).cloned().collect();
							match p.last() {
								Some(l)	=> (p.clone(), l.clone()),
								None	=> continue,
							}
						}
						_ => {
							let path_node = item.child(SyntaxKind::ImportItemPath);
							let p: Vec<SyntaxNode> = match path_node {
								Some(pn)	=> pn.children().iter().filter(|c| c.kind() == SyntaxKind::Ident).cloned().collect(),
								None		=> item.children().iter().take_while(|c| c.kind() != SyntaxKind::As)
									.filter(|c| c.kind() == SyntaxKind::Ident).cloned().collect(),
							};
							let b = item.children().iter().skip_while(|c| c.kind() != SyntaxKind::As).skip(1)
								.find(|c| c.kind() == SyntaxKind::Ident).cloned();
							match b {
								Some(b)	=> {
									if p.last().map(|l| l.text() == b.text()).unwrap_or(false) {
										self.engine.warn(DiagnosticKind::Lint, self.fix(b.span()), "unnecessary import rename to same name");
									}
									(p, b)
								}
								None	=> continue,
							}
						}
					};
					let mut cur = source.clone();
					let mut ok = true;
					for (i, comp) in path.iter().enumerate() {
						let next = scope_member(&cur, comp.text());
						match next {
							Some(v) => {
								if i + 1 < path.len() && !matches!(v, Value::Module(_) | Value::Func(_) | Value::Type(_)) {
									failed = Some(self.error(DiagnosticKind::Type, comp.span(), fmt!(
										"expected module, function, or type, found {}", v.ty().long_name())));
									ok = false;
									break;
								}
								cur = v;
							}
							None => {
								failed = Some(self.error(DiagnosticKind::UnknownVariable, comp.span(), "unresolved import"));
								ok = false;
								break;
							}
						}
					}
					if ok {
						self.define(bound.text(), cur, bound.span());
					}
				}
				if let Some(e) = failed {
					return Err(e);
				}
			}
		}
		Ok(Value::None)
	}

	fn eval_include(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let src_node = match expr_after(node, SyntaxKind::Include) {
			Some(s)	=> s.clone(),
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "include without a source")),
		};
		let span = self.fix(src_node.span());
		match res!(self.eval(&src_node)) {
			Value::Str(spec) => {
				let mark = self.engine.diags.len();
				match import::include(self.engine, &spec, self.file, span) {
					Ok(c)	=> Ok(Value::Content(c)),
					Err(e)	=> Err(self.engine.adopt(mark, span, e)),
				}
			}
			Value::Module(m)	=> Ok(Value::Content(m.content.clone())),
			other				=> Err(self.error(DiagnosticKind::Type, span, fmt!("expected path or module, found {}", other.ty().long_name()))),
		}
	}
}

// A name `import`ed from a module, function or type.
fn scope_member(v: &Value, name: &str) -> Option<Value> {
	match v {
		Value::Module(m)	=> m.scope.get(name).cloned(),
		Value::Func(f)		=> func_field(f, name).map(Value::Func),
		Value::Type(t)		=> methods::type_method(*t, name).map(|f| Value::Func(Func::Native(f))),
		_					=> None,
	}
}

// The name a bare `import "path"` binds: the package name or the file stem.

// Does the content hold a counter or state update, anywhere in its tree?
fn contains_update(c: &Content, depth: usize) -> bool {
	if depth > 256 {
		return false;
	}
	match c {
		Content::Sequence(seq)	=> seq.children.iter().any(|k| contains_update(k, depth + 1)),
		Content::Styled(st)		=> contains_update(&st.child, depth + 1),
		Content::Elem(e)		=> matches!(e.kind, ElemKind::CounterUpdate | ElemKind::StateUpdate)
			|| e.fields.iter().any(|(_, v)| match v {
				Value::Content(k)	=> contains_update(k, depth + 1),
				_					=> false,
			}),
	}
}
