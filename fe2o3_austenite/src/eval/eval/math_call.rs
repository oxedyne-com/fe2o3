// SPDX-License-Identifier: Apache-2.0
// Original work copyright (c) the Typst project authors (typst-eval `call.rs` and `math.rs`, version 0.15.1).
// Modified for Austenite: ported to Hematite's types, syntax tree and error handling.
// U7 owns this file. Typst 0.15's parser gives a maths function call and a maths field access their own
// syntax kinds (`MathCall`, `MathArgs`, `MathFieldAccess`), so `sqrt(x)`, `vec(a, b)`, `arrow.r` and
// `math.vec(..)` reach the evaluator through here rather than as code-mode calls.

use super::Vm;

use crate::diag::DiagnosticKind;
use crate::eval::args::{
	Arg,
	Args,
};
use crate::eval::content::{
	Content,
	ElemKind,
};
use crate::eval::func::Func;
use crate::eval::lib::{
	self,
	foundations,
};
use crate::eval::methods;
use crate::eval::ops;
use crate::eval::value::Value;
use crate::syntax::ast::{
	self,
	AstNode,
};
use crate::syntax::{
	Span,
	SyntaxNode,
};

use oxedyne_fe2o3_core::prelude::*;

impl Vm<'_> {
	/// A maths field access: a symbol's variant (`arrow.r`), a module's member (`math.vec`) or a
	/// field of a value.
	pub(super) fn eval_math_field_access(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let access = match ast::MathFieldAccess::from_untyped(node) {
			Some(a)	=> a,
			None	=> return Err(self.error(DiagnosticKind::Syntax, node.span(), "math field access without a target")),
		};
		let target = res!(self.eval_math_access(access.target()));
		let field = access.field();
		self.field(target, field.get(), field.span())
	}

	fn eval_math_access(&mut self, access: ast::MathAccess) -> Outcome<Value> {
		match access {
			ast::MathAccess::MathIdent(id)			=> self.eval_math_ident(id.to_untyped()),
			ast::MathAccess::MathFieldAccess(fa)	=> self.eval_math_field_access(fa.to_untyped()),
		}
	}

	/// A maths call. A function is called with its arguments, which two-dimensional syntax (`;`) groups
	/// into rows. A callee that is not a function shows as itself followed by its arguments in
	/// parentheses, so `f(x)` and `alpha(x)` read as text, not as errors.
	pub(super) fn eval_math_call(&mut self, node: &SyntaxNode) -> Outcome<Value> {
		let span = node.span();
		let call = match ast::MathCall::from_untyped(node) {
			Some(c)	=> c,
			None	=> return Err(self.error(DiagnosticKind::Syntax, span, "math call without a callee")),
		};
		let callee = call.callee();
		let args = call.args();
		let mut receiver: Option<(Span, Value)> = None;
		let callee_value = match callee {
			ast::MathAccess::MathIdent(id)	=> res!(self.eval_math_ident(id.to_untyped())),
			ast::MathAccess::MathFieldAccess(fa) => {
				let target_node = fa.target();
				let target = res!(self.eval_math_access(target_node));
				let field = fa.field();
				let name = field.get().to_string();
				if methods::is_mutating(&name) && matches!(target, Value::Array(_) | Value::Dict(_)) {
					return Err(self.error_hint(DiagnosticKind::Type, span, "cannot call mutating methods in math", fmt!(
						"try using code mode to call the method: `#{}`", node.full_text())));
				}
				if let Some(m) = methods::type_method(target.ty(), &name) {
					// A method takes its target as the first argument.
					receiver = Some((target_node.span(), target));
					Value::Func(Func::Native(m))
				} else {
					match target {
						Value::Symbol(_) | Value::Func(_) | Value::Type(_) | Value::Module(_)
							=> res!(self.field(target, &name, field.span())),
						Value::Dict(ref d) if matches!(d.get(&name), Some(Value::Func(_))) => return Err(self.error_hint(
							DiagnosticKind::Type, fa.span(), "cannot directly call dictionary keys as functions", fmt!(
							"to call the stored function, use code mode and wrap the field access in parentheses: `#({})(..)`",
							fa.to_untyped().full_text()))),
						other => return Err(self.error_hint(DiagnosticKind::Type, fa.span(), fmt!(
							"type {} has no method `{}`", other.ty().long_name(), name),
							"try adding a space before the parentheses")),
					}
				}
			}
		};
		let func = match &callee_value {
			Value::Func(f)	=> Some(f.clone()),
			Value::Type(t)	=> foundations::constructor(*t).map(Func::Native),
			// A symbol is callable where it is an accent or a delimiter: `hat(x)`, `floor(x)`.
			Value::Symbol(s) => lib::math::symbol_func(&ops::symbol_text(s)),
			_				=> None,
		};
		match func {
			Some(f) => {
				let mut a = res!(self.eval_math_args(args, span));
				if let Some((sp, v)) = receiver {
					a.prepend(sp, v);
				}
				self.engine.call_func(&f, a)
			}
			None => {
				let parens = res!(self.unparse_math_args(args, callee));
				let head = res!(self.display(callee_value, callee.span()));
				Ok(Value::Content(ops::content_add(head.with_span(callee.span()), parens)))
			}
		}
	}

	// The arguments of a maths call. Named and positional arguments are kept apart so a named one may sit
	// anywhere among the rows of `mat(a, delim: "[", b; c, d)`; a semicolon closes the row of positional
	// arguments since the last one, which becomes one array argument.
	fn eval_math_args(&mut self, node: ast::MathArgs, call_span: Span) -> Outcome<Args> {
		let mut named: Vec<Arg> = Vec::new();
		let mut pos: Vec<Arg> = Vec::new();
		let mut two_dim_start: Option<usize> = None;
		for item in node.arg_items() {
			let arg = item.arg;
			let span = arg.span();
			match arg {
				ast::Arg::Pos(expr) => {
					let v = res!(self.eval(expr.to_untyped()));
					pos.push(Arg { span, value_span: span, name: None, value: v });
				}
				ast::Arg::Named(n) => {
					let v = res!(self.eval(n.expr().to_untyped()));
					named.push(Arg { span, value_span: n.expr().span(), name: Some(n.name().get().to_string()), value: v });
				}
				ast::Arg::Spread(sp) => {
					match res!(self.eval(sp.expr().to_untyped())) {
						Value::None		=> (),
						Value::Array(a)	=> for v in a.iter() {
							pos.push(Arg { span, value_span: span, name: None, value: v.clone() });
						},
						Value::Dict(d)	=> for (k, v) in d.iter() {
							named.push(Arg { span, value_span: span, name: Some(k.to_string()), value: v.clone() });
						},
						Value::Args(a)	=> for x in a.items.iter() {
							if x.name.is_none() {
								pos.push(x.clone());
							} else {
								named.push(x.clone());
							}
						},
						other => return Err(self.error(DiagnosticKind::Type, sp.span(), fmt!(
							"cannot spread {}", other.ty().long_name()))),
					}
				}
			}
			if item.ends_in_semicolon {
				let start = two_dim_start.unwrap_or(0);
				Self::close_row(&mut pos, start, node.span());
				two_dim_start = Some(pos.len());
			}
		}
		if let Some(start) = two_dim_start {
			if start != pos.len() {
				Self::close_row(&mut pos, start, node.span());
			}
		}
		let mut items = named;
		items.extend(pos);
		Ok(Args { span: call_span, items })
	}

	// Replaces the positional arguments from `start` on by one array of their values.
	fn close_row(pos: &mut Vec<Arg>, start: usize, span: Span) {
		let row: Vec<Value> = pos.drain(start..).map(|a| a.value).collect();
		pos.push(Arg { span, value_span: span, name: None, value: Value::array(row) });
	}

	// For a callee that is not a function: the arguments and the punctuation between them as content,
	// wrapped as delimited maths.
	fn unparse_math_args(&mut self, args: ast::MathArgs, callee: ast::MathAccess) -> Outcome<Content> {
		let mut body: Vec<Content> = Vec::new();
		let mut failed: Option<Error<ErrTag>> = None;
		for item in args.content_items() {
			match item {
				ast::MathArgItem::Space(space) => {
					let v = res!(self.eval(space.to_untyped()));
					body.push(res!(self.display(v, space.span())).with_span(space.span()));
				}
				ast::MathArgItem::Comma(c, node)
				| ast::MathArgItem::Semicolon(c, node)
				| ast::MathArgItem::LeftParen(c, node)
				| ast::MathArgItem::RightParen(c, node) => {
					body.push(Content::symbol(&c.to_string()).with_span(node.span()));
				}
				ast::MathArgItem::Arg(ast::Arg::Pos(expr)) => {
					// `display`, not a content cast, so `sin(#1)` is no more an error than `#1` is.
					let v = res!(self.eval(expr.to_untyped()));
					body.push(res!(self.display(v, expr.span())).with_span(expr.span()));
				}
				ast::MathArgItem::Arg(ast::Arg::Named(n)) => {
					let name = callee.to_untyped().full_text();
					let fixed = n.to_untyped().full_text().replacen(':', "\\:", 1);
					let e = self.error_hint(DiagnosticKind::Syntax, n.span(),
						"named-argument syntax can only be used with functions", fmt!(
						"`{}` is not a function; to render the colon as text, escape it: `{}`", name, fixed));
					failed.get_or_insert(e);
				}
				ast::MathArgItem::Arg(ast::Arg::Spread(sp)) => {
					let name = callee.to_untyped().full_text();
					let fixed = sp.to_untyped().full_text().replacen("..", ".. ", 1);
					let e = self.error_hint(DiagnosticKind::Syntax, sp.span(),
						"spread-argument syntax can only be used with functions", fmt!(
						"`{}` is not a function; to render the dots as text, add a space: `{}`", name, fixed));
					failed.get_or_insert(e);
				}
			}
		}
		if let Some(e) = failed {
			return Err(e);
		}
		let seq = Content::sequence(body);
		Ok(res!(self.elem(ElemKind::MathLr, vec![("body", Value::Content(seq))], args.span())))
	}
}
