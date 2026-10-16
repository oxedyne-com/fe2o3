// SPDX-License-Identifier: Apache-2.0
// Portions Copyright (c) The Typst Project Developers, from typst-syntax 0.15.1's `src/parser.rs`; see
// fe2o3_austenite/licences/typst-LICENSE.txt for the full Apache-2.0 text.
// Modified for Austenite.
//! Ported from typst-syntax 0.15.1 (`src/parser.rs`, Apache-2.0, (c) the Typst authors): the same
//! recursive descent over the three modes, the same newline modes, backtracking memo and depth limit,
//! so the tree has typst-syntax's kinds, child order and error nodes. The tree is built with detached
//! spans and numbered once at the end, which keeps every node's span its exact byte range.

use crate::syntax::{
	FileId,
	SyntaxKind,
	SyntaxNode,
};
use crate::syntax::ast;
use crate::syntax::kind::{
	self,
	SyntaxSet,
	syntax_set,
};
use crate::syntax::lexer::{
	self,
	LexMode,
	Lexer,
};

use std::collections::{
	HashMap,
	HashSet,
};
use std::ops::Range;

const MAX_DEPTH: u32 = 256;			// Typst's nesting limit

/// Parses a file in markup mode into a `Markup` root.
pub fn parse(text: &str, file: FileId) -> SyntaxNode {
	let mut p = Parser::new(text, LexMode::Markup);
	markup_exprs(&mut p, true, syntax_set!(End));
	p.finish_into(SyntaxKind::Markup, file)
}

/// Parses code, as `eval(mode: "code")` and a `typst eval` probe need, into a `Code` root.
pub fn parse_code(text: &str, file: FileId) -> SyntaxNode {
	let mut p = Parser::new(text, LexMode::Code);
	code_exprs(&mut p, syntax_set!(End));
	p.finish_into(SyntaxKind::Code, file)
}

/// Parses maths, as `eval(mode: "math")` needs, into a `Math` root.
pub fn parse_math(text: &str, file: FileId) -> SyntaxNode {
	let mut p = Parser::new(text, LexMode::Math);
	math_exprs(&mut p, syntax_set!(End));
	p.finish_into(SyntaxKind::Math, file)
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Markup                                                                                    │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

fn markup(p: &mut Parser, at_start: bool, wrap_trivia: bool, stop: SyntaxSet) {
	let m = if wrap_trivia { p.before_trivia() } else { p.marker() };
	markup_exprs(p, at_start, stop);
	if wrap_trivia {
		p.flush_trivia();
	}
	p.wrap(m, SyntaxKind::Markup);
}

fn markup_exprs(p: &mut Parser, mut at_start: bool, stop: SyntaxSet) {
	if !p.check_depth_until(stop) {
		return;
	}
	at_start |= p.had_newline();
	let mut nesting: usize = 0;
	// A nested right bracket is text, whatever the stop set says.
	while !p.at_set(stop) || (nesting > 0 && p.at(SyntaxKind::RightBracket)) {
		markup_expr(p, at_start, &mut nesting);
		at_start = p.had_newline();
	}
}

fn markup_expr(p: &mut Parser, at_start: bool, nesting: &mut usize) {
	if !p.enter() {
		return;
	}
	match p.current() {
		SyntaxKind::LeftBracket						=> {
			*nesting += 1;
			p.convert_and_eat(SyntaxKind::Text);
		},
		SyntaxKind::RightBracket if *nesting > 0	=> {
			*nesting -= 1;
			p.convert_and_eat(SyntaxKind::Text);
		},
		SyntaxKind::RightBracket					=> {
			p.unexpected();
			p.hint("try using a backslash escape: \\]");
		},

		SyntaxKind::Shebang
		| SyntaxKind::Text
		| SyntaxKind::Linebreak
		| SyntaxKind::Escape
		| SyntaxKind::Shorthand
		| SyntaxKind::SmartQuote
		| SyntaxKind::Link
		| SyntaxKind::Label
		| SyntaxKind::Raw							=> p.eat(),

		SyntaxKind::Hash							=> embedded_code_expr(p),
		SyntaxKind::Star							=> strong(p),
		SyntaxKind::Underscore						=> emph(p),
		SyntaxKind::HeadingMarker if at_start		=> heading(p),
		SyntaxKind::ListMarker if at_start			=> list_item(p),
		SyntaxKind::EnumMarker if at_start			=> enum_item(p),
		SyntaxKind::TermMarker if at_start			=> term_item(p),
		SyntaxKind::RefMarker						=> reference(p),
		SyntaxKind::Dollar							=> equation(p),

		SyntaxKind::HeadingMarker
		| SyntaxKind::ListMarker
		| SyntaxKind::EnumMarker
		| SyntaxKind::TermMarker
		| SyntaxKind::Colon							=> p.convert_and_eat(SyntaxKind::Text),

		_											=> p.unexpected(),
	}
	p.leave();
}

fn strong(p: &mut Parser) {
	delimited_markup(p, SyntaxKind::Star, SyntaxKind::Strong, "no text within stars",
		"using multiple consecutive stars (e.g. **) has no additional effect");
}

fn emph(p: &mut Parser) {
	delimited_markup(p, SyntaxKind::Underscore, SyntaxKind::Emph, "no text within underscores",
		"using multiple consecutive underscores (e.g. __) has no additional effect");
}

// `*strong*` and `_emph_`: stop at a paragraph break, warn when empty.
fn delimited_markup(p: &mut Parser, delim: SyntaxKind, wrapper: SyntaxKind, warning: &str, hint: &str) {
	p.with_nl_mode(AtNewline::StopParBreak, |p| {
		let m = p.marker();
		p.assert(delim);
		markup(p, false, true, syntax_set!(RightBracket, End).add(delim));
		let closed = p.expect_closing_delimiter(m, delim);
		p.wrap(m, wrapper);
		if closed {
			if let Some(n) = p.node_mut(m) {
				if n.len() == 2 {
					n.warn(warning);
					n.hint(hint);
				}
			}
		}
	});
}

fn heading(p: &mut Parser) {
	p.with_nl_mode(AtNewline::Stop, |p| {
		let m = p.marker();
		p.assert(SyntaxKind::HeadingMarker);
		markup(p, false, false, syntax_set!(Label, RightBracket, End));
		p.wrap(m, SyntaxKind::Heading);
	});
}

fn list_item(p: &mut Parser) {
	let col = p.current_column();
	p.with_nl_mode(AtNewline::RequireColumn(col), |p| {
		let m = p.marker();
		p.assert(SyntaxKind::ListMarker);
		markup(p, true, false, syntax_set!(RightBracket, End));
		p.wrap(m, SyntaxKind::ListItem);
	});
}

fn enum_item(p: &mut Parser) {
	let col = p.current_column();
	p.with_nl_mode(AtNewline::RequireColumn(col), |p| {
		let m = p.marker();
		p.assert(SyntaxKind::EnumMarker);
		markup(p, true, false, syntax_set!(RightBracket, End));
		p.wrap(m, SyntaxKind::EnumItem);
	});
}

fn term_item(p: &mut Parser) {
	let col = p.current_column();
	p.with_nl_mode(AtNewline::RequireColumn(col), |p| {
		let m = p.marker();
		p.with_nl_mode(AtNewline::Stop, |p| {
			p.assert(SyntaxKind::TermMarker);
			markup(p, false, false, syntax_set!(Colon, RightBracket, End));
		});
		p.expect(SyntaxKind::Colon);
		markup(p, true, false, syntax_set!(RightBracket, End));
		p.wrap(m, SyntaxKind::TermItem);
	});
}

fn reference(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::RefMarker);
	if p.directly_at(SyntaxKind::LeftBracket) {
		content_block(p);
	}
	p.wrap(m, SyntaxKind::Ref);
}

fn equation(p: &mut Parser) {
	let m = p.marker();
	p.enter_modes(LexMode::Math, AtNewline::Continue, |p| {
		p.assert(SyntaxKind::Dollar);
		math(p, syntax_set!(Dollar, End));
		p.expect_closing_delimiter(m, SyntaxKind::Dollar);
	});
	p.wrap(m, SyntaxKind::Equation);
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Maths                                                                                     │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

fn math(p: &mut Parser, stop: SyntaxSet) {
	let m = p.marker();
	math_exprs(p, stop);
	p.wrap(m, SyntaxKind::Math);
}

// Returns the number of expressions parsed, errors included.
fn math_exprs(p: &mut Parser, stop: SyntaxSet) -> usize {
	if !p.check_depth_until(stop) {
		return 1;
	}
	let mut count = 0;
	while !p.at_set(stop) {
		if p.at_set(kind::MATH_EXPR) {
			math_expr(p);
		} else {
			p.unexpected();
		}
		count += 1;
	}
	count
}

fn math_expr(p: &mut Parser) {
	math_expr_prec(p, 0, syntax_set!());
}

const MATH_FUNC_PREC: u8 = 2;
const MATH_ROOT_PREC: u8 = 2;

fn math_expr_prec(p: &mut Parser, min_prec: u8, stop: SyntaxSet) {
	if !p.enter() {
		return;
	}
	math_expr_prec_inner(p, min_prec, stop);
	p.leave();
}

fn math_expr_prec_inner(p: &mut Parser, min_prec: u8, stop: SyntaxSet) {
	let m = p.marker();
	let mut continuable = false;
	match p.current() {
		SyntaxKind::Hash													=> embedded_code_expr(p),

		// The lexer builds whole `MathFieldAccess` chains.
		SyntaxKind::MathIdent | SyntaxKind::MathFieldAccess					=> {
			continuable = true;
			p.eat();
			if MATH_FUNC_PREC >= min_prec && p.directly_at(SyntaxKind::LeftParen) {
				math_args(p);
				p.wrap(m, SyntaxKind::MathCall);
				continuable = false;
			}
		},

		SyntaxKind::LeftBrace | SyntaxKind::LeftParen						=> math_delimited(p),
		SyntaxKind::RightBrace if p.current_text() == "|]"					=>
			p.convert_and_eat(SyntaxKind::MathShorthand),
		SyntaxKind::Dot
		| SyntaxKind::Bang
		| SyntaxKind::Comma
		| SyntaxKind::Semicolon
		| SyntaxKind::RightBrace
		| SyntaxKind::RightParen											=> p.convert_and_eat(SyntaxKind::MathText),

		SyntaxKind::MathText												=> {
			continuable = is_math_alphabetic(p.current_text());
			p.eat();
		},

		SyntaxKind::Linebreak
		| SyntaxKind::MathAlignPoint
		| SyntaxKind::MathShorthand											=> p.eat(),

		SyntaxKind::MathPrimes | SyntaxKind::Escape | SyntaxKind::Str		=> {
			continuable = true;
			p.eat();
		},

		SyntaxKind::Root													=> {
			p.eat();
			let m2 = p.marker();
			math_expr_prec(p, MATH_ROOT_PREC, syntax_set!());
			math_unparen(p, m2);
			p.wrap(m, SyntaxKind::MathRoot);
		},

		_																	=> p.expected("expression"),
	}

	// An implicit call: a continuable atom directly followed by delimiters groups with them, so `a(b)/c`
	// is `(a(b))/c`.
	if continuable
		&& MATH_FUNC_PREC >= min_prec
		&& !p.had_trivia()
		&& p.at_set(syntax_set!(LeftBrace, LeftParen))
	{
		math_delimited(p);
		p.wrap(m, SyntaxKind::Math);
	}

	// Infix and postfix operators, `MathAttach[ MathText("x"), Hat("^"), MathText("2") ]`.
	loop {
		if p.at_set(stop) {
			break;
		}
		let op_kind = p.current();
		let (wrapper, assoc, prec) = match math_op(op_kind, p.had_trivia()) {
			Some(op) if op.2 >= min_prec	=> op,
			_								=> break,
		};

		// `^` chains with `_` and back; a prime chains with either.
		let mut chain = if wrapper == SyntaxKind::MathAttach {
			syntax_set!(Hat, Underscore).remove(op_kind)
		} else {
			syntax_set!()
		};

		if op_kind == SyntaxKind::Bang {
			p.convert_and_eat(SyntaxKind::MathText);
		} else {
			p.eat();
		}

		// Only the slash strips its left operand's parentheses.
		if wrapper == SyntaxKind::MathFrac {
			math_unparen(p, m);
		}

		if let Some(assoc) = assoc {
			let prec = match assoc {
				ast::Assoc::Left	=> prec + 1,
				ast::Assoc::Right	=> prec,
			};
			let m_rhs = p.marker();
			math_expr_prec(p, prec, chain);
			math_unparen(p, m_rhs);
		}

		// A prime at the end of a chain does not interrupt it: `a^b'_c^d` is `(a^(b')_c)^d`.
		if !(op_kind == SyntaxKind::MathPrimes && p.at_set(stop)) {
			while p.at_set(chain) {
				chain = chain.remove(p.current());
				p.eat();
				let m_rhs = p.marker();
				math_expr_prec(p, prec, chain);
				math_unparen(p, m_rhs);
			}
		}

		p.wrap(m, wrapper);
	}
}

fn math_op(kind: SyntaxKind, had_trivia: bool) -> Option<(SyntaxKind, Option<ast::Assoc>, u8)> {
	match kind {
		SyntaxKind::Slash					=> Some((SyntaxKind::MathFrac, Some(ast::Assoc::Left), 1)),
		SyntaxKind::Underscore				=> Some((SyntaxKind::MathAttach, Some(ast::Assoc::Right), 2)),
		SyntaxKind::Hat						=> Some((SyntaxKind::MathAttach, Some(ast::Assoc::Right), 2)),
		SyntaxKind::MathPrimes if !had_trivia	=> Some((SyntaxKind::MathAttach, None, 2)),
		SyntaxKind::Bang if !had_trivia		=> Some((SyntaxKind::Math, None, 3)),
		_									=> None,
	}
}

/// Does the text count as alphabetic in maths, grouping with a following parenthesis as an implicit call?
fn is_math_alphabetic(text: &str) -> bool {
	let mut chars = text.chars();
	match (chars.next(), chars.next()) {
		(Some(c), None)	=> c.is_alphabetic() || lexer::is_math_alphabetic_class(c),
		_				=> text.chars().all(char::is_alphabetic),
	}
}

// Matched delimiters, `[x + y]`. The lexer's brace and paren kinds turn back into text here.
fn math_delimited(p: &mut Parser) {
	let m = p.marker();
	if p.current_text() == "[|" {
		p.convert_and_eat(SyntaxKind::MathShorthand);
	} else {
		p.convert_and_eat(SyntaxKind::MathText);
	}
	let m_body = p.marker();
	math_exprs(p, syntax_set!(Dollar, End, RightBrace, RightParen));
	if p.at_set(syntax_set!(RightBrace, RightParen)) {
		p.wrap(m_body, SyntaxKind::Math);
		if p.current_text() == "|]" {
			p.convert_and_eat(SyntaxKind::MathShorthand);
		} else {
			p.convert_and_eat(SyntaxKind::MathText);
		}
		p.wrap(m, SyntaxKind::MathDelimited);
	} else {
		// Unclosed: a plain sequence.
		p.wrap(m, SyntaxKind::Math);
	}
}

// Strips one pair of round parentheses from the expression at `m`, by re-kinding them.
fn math_unparen(p: &mut Parser, m: Marker) {
	let node = match p.nodes.get_mut(m.0) {
		Some(n)	=> n,
		None	=> return,
	};
	if node.kind() != SyntaxKind::MathDelimited {
		return;
	}
	let parens = {
		let c = node.children();
		match (c.first(), c.last()) {
			(Some(f), Some(l))	=> c.len() >= 2 && f.text() == "(" && l.text() == ")",
			_					=> false,
		}
	};
	if parens {
		let children = node.children_mut();
		let n = children.len();
		if let Some(f) = children.first_mut() {
			f.convert_to_kind(SyntaxKind::LeftParen);
		}
		if let Some(l) = children.get_mut(n - 1) {
			l.convert_to_kind(SyntaxKind::RightParen);
		}
		node.convert_to_kind(SyntaxKind::Math);
	}
}

// A maths argument list, `(a, b; c, d; size: #50%)`.
fn math_args(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::LeftParen);
	let mut seen = HashSet::new();
	while !p.at_set(syntax_set!(End, Dollar, RightParen)) {
		math_arg(p, &mut seen);
		match p.current() {
			SyntaxKind::End | SyntaxKind::Dollar | SyntaxKind::RightParen	=> (),
			SyntaxKind::Semicolon | SyntaxKind::Comma						=> p.eat(),
			_																=> p.expected("comma or semicolon"),
		}
	}
	p.expect_closing_delimiter(m, SyntaxKind::RightParen);
	p.wrap(m, SyntaxKind::MathArgs);
}

fn math_arg<'s>(p: &mut Parser<'s>, seen: &mut HashSet<&'s str>) {
	let m = p.marker();
	let start = p.current_start();
	let mut arg_kind = None;

	if let Some(spread) = p.lexer.maybe_math_spread_arg(start) {
		arg_kind = Some(SyntaxKind::Spread);
		p.token.node = spread;
		p.eat();
	} else if let Some(named) = p.lexer.maybe_math_named_arg(start) {
		arg_kind = Some(SyntaxKind::Named);
		p.token.node = named;
		let text = p.current_text();
		p.eat();
		p.convert_and_eat(SyntaxKind::Colon);
		if !seen.insert(text) {
			if let Some(n) = p.node_mut(m) {
				n.convert_to_error(&format!("duplicate argument: {}", text));
			}
		}
	}

	let m_arg = p.marker();
	let count = math_exprs(p, syntax_set!(End, Dollar, Comma, Semicolon, RightParen));
	if count == 0 && arg_kind == Some(SyntaxKind::Named) {
		p.expected("expression");
	}
	// One expression stays as it is (`func(#12pt)` keeps a length); none or several become maths.
	if count != 1 {
		p.wrap(m_arg, SyntaxKind::Math);
	}
	if let Some(k) = arg_kind {
		p.wrap(m, k);
	}
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Code                                                                                      │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

fn code(p: &mut Parser, stop: SyntaxSet) {
	let m = p.marker();
	code_exprs(p, stop);
	p.wrap(m, SyntaxKind::Code);
}

fn code_exprs(p: &mut Parser, stop: SyntaxSet) {
	if !p.check_depth_until(stop) {
		return;
	}
	while !p.at_set(stop) {
		p.with_nl_mode(AtNewline::ContextualContinue, |p| {
			if !p.at_set(kind::CODE_EXPR) {
				p.unexpected();
				return;
			}
			code_expr(p);
			if !p.at_set(stop) && !p.eat_if(SyntaxKind::Semicolon) {
				p.expected("semicolon or line break");
				if p.at(SyntaxKind::Label) {
					p.hint("labels can only be applied in markup mode");
					p.hint("try wrapping your code in a markup block (`[ ]`)");
				}
			}
		});
	}
}

// An atomic code expression after a `#` in markup or maths.
fn embedded_code_expr(p: &mut Parser) {
	p.enter_modes(LexMode::Code, AtNewline::Stop, |p| {
		p.assert(SyntaxKind::Hash);
		if p.had_trivia() || p.end() {
			p.expected("expression");
			return;
		}
		let stmt = p.at_set(kind::STMT);
		code_expr_prec(p, true, 0);
		// Two-dimensional maths arguments rely on the `directly_at` here.
		let semi = (stmt || p.directly_at(SyntaxKind::Semicolon)) && p.eat_if(SyntaxKind::Semicolon);
		if stmt && !semi && !p.end() && !p.at(SyntaxKind::RightBracket) {
			p.expected("semicolon or line break");
		}
	});
}

fn code_expr(p: &mut Parser) {
	code_expr_prec(p, false, 0);
}

fn code_expr_prec(p: &mut Parser, atomic: bool, min_prec: u8) {
	if !p.enter() {
		return;
	}
	code_expr_prec_inner(p, atomic, min_prec);
	p.leave();
}

fn code_expr_prec_inner(p: &mut Parser, atomic: bool, min_prec: u8) {
	let m = p.marker();
	if p.at_set(kind::UNARY_OP) {
		if !atomic {
			let op = match ast::UnOp::from_kind(p.current()) {
				Some(op)	=> op,
				None		=> ast::UnOp::Pos,
			};
			p.eat();
			code_expr_prec(p, atomic, op.precedence());
			p.wrap(m, SyntaxKind::Unary);
		} else {
			p.unexpected();
			p.hint("to use a unary operator here, wrap the entire expression in parentheses");
		}
	} else {
		code_primary(p, atomic);
	}

	loop {
		if p.directly_at(SyntaxKind::LeftParen) || p.directly_at(SyntaxKind::LeftBracket) {
			args(p);
			p.wrap(m, SyntaxKind::FuncCall);
			continue;
		}

		let at_field_or_method = p.directly_at(SyntaxKind::Dot)
			&& p.lexer.clone().next().0 == SyntaxKind::Ident;
		if atomic && !at_field_or_method {
			break;
		}

		if p.eat_if(SyntaxKind::Dot) {
			p.expect(SyntaxKind::Ident);
			p.wrap(m, SyntaxKind::FieldAccess);
			continue;
		}

		let binop = if p.at_set(kind::BINARY_OP) {
			ast::BinOp::from_kind(p.current())
		} else if min_prec <= ast::BinOp::NotIn.precedence() && p.eat_if(SyntaxKind::Not) {
			if p.at(SyntaxKind::In) {
				Some(ast::BinOp::NotIn)
			} else {
				p.expected("keyword `in`");
				break;
			}
		} else {
			None
		};

		if let Some(op) = binop {
			let mut prec = op.precedence();
			if prec < min_prec {
				break;
			}
			if op.assoc() == ast::Assoc::Left {
				prec += 1;
			}
			p.eat();
			code_expr_prec(p, false, prec);
			p.wrap(m, SyntaxKind::Binary);
			continue;
		}

		break;
	}
}

// The atoms unary and binary operations, calls and field accesses are built from.
fn code_primary(p: &mut Parser, atomic: bool) {
	let m = p.marker();
	match p.current() {
		SyntaxKind::Ident					=> {
			p.eat();
			if !atomic && p.at(SyntaxKind::Arrow) {
				p.wrap(m, SyntaxKind::Params);
				p.assert(SyntaxKind::Arrow);
				code_expr(p);
				p.wrap(m, SyntaxKind::Closure);
			}
		},
		SyntaxKind::Underscore if !atomic	=> {
			p.eat();
			if p.at(SyntaxKind::Arrow) {
				p.wrap(m, SyntaxKind::Params);
				p.eat();
				code_expr(p);
				p.wrap(m, SyntaxKind::Closure);
			} else if p.eat_if(SyntaxKind::Eq) {
				code_expr(p);
				p.wrap(m, SyntaxKind::DestructAssignment);
			} else if let Some(n) = p.node_mut(m) {
				n.expected("expression");
			}
		},

		SyntaxKind::LeftBrace				=> code_block(p),
		SyntaxKind::LeftBracket				=> content_block(p),
		SyntaxKind::LeftParen				=> expr_with_paren(p, atomic),
		SyntaxKind::Dollar					=> equation(p),
		SyntaxKind::Let						=> let_binding(p),
		SyntaxKind::Set						=> set_rule(p),
		SyntaxKind::Show					=> show_rule(p),
		SyntaxKind::Context					=> contextual(p, atomic),
		SyntaxKind::If						=> conditional(p),
		SyntaxKind::While					=> while_loop(p),
		SyntaxKind::For						=> for_loop(p),
		SyntaxKind::Import					=> module_import(p),
		SyntaxKind::Include					=> module_include(p),
		SyntaxKind::Break					=> break_stmt(p),
		SyntaxKind::Continue				=> continue_stmt(p),
		SyntaxKind::Return					=> return_stmt(p),

		SyntaxKind::Raw
		| SyntaxKind::None
		| SyntaxKind::Auto
		| SyntaxKind::Int
		| SyntaxKind::Float
		| SyntaxKind::Bool
		| SyntaxKind::Numeric
		| SyntaxKind::Str
		| SyntaxKind::Label					=> p.eat(),

		// `#12p`, `#]`, `#"abc\"`: consume the bad token.
		_ if atomic							=> p.unexpected(),

		_									=> p.expected("expression"),
	}
}

fn block(p: &mut Parser) {
	match p.current() {
		SyntaxKind::LeftBracket	=> content_block(p),
		SyntaxKind::LeftBrace	=> code_block(p),
		_						=> p.expected("block"),
	}
}

fn code_block(p: &mut Parser) {
	let m = p.marker();
	p.enter_modes(LexMode::Code, AtNewline::Continue, |p| {
		p.assert(SyntaxKind::LeftBrace);
		code(p, syntax_set!(RightBrace, RightBracket, RightParen, End));
		p.expect_closing_delimiter(m, SyntaxKind::RightBrace);
	});
	p.wrap(m, SyntaxKind::CodeBlock);
}

fn content_block(p: &mut Parser) {
	let m = p.marker();
	p.enter_modes(LexMode::Markup, AtNewline::Continue, |p| {
		p.assert(SyntaxKind::LeftBracket);
		markup(p, true, true, syntax_set!(RightBracket, End));
		p.expect_closing_delimiter(m, SyntaxKind::RightBracket);
	});
	p.wrap(m, SyntaxKind::ContentBlock);
}

fn let_binding(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Let);

	let m2 = p.marker();
	let mut closure = false;
	let mut other = false;
	if p.eat_if(SyntaxKind::Ident) {
		if p.directly_at(SyntaxKind::LeftParen) {
			params(p);
			closure = true;
		}
	} else {
		pattern(p, false, &mut HashSet::new(), None);
		other = true;
	}

	let has_init = if closure || other { p.expect(SyntaxKind::Eq) } else { p.eat_if(SyntaxKind::Eq) };
	if has_init {
		code_expr(p);
	}
	if closure {
		p.wrap(m2, SyntaxKind::Closure);
	}
	p.wrap(m, SyntaxKind::LetBinding);
}

fn set_rule(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Set);
	let m2 = p.marker();
	p.expect(SyntaxKind::Ident);
	while p.eat_if(SyntaxKind::Dot) {
		p.expect(SyntaxKind::Ident);
		p.wrap(m2, SyntaxKind::FieldAccess);
	}
	args(p);
	if p.eat_if(SyntaxKind::If) {
		code_expr(p);
	}
	p.wrap(m, SyntaxKind::SetRule);
}

fn show_rule(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Show);
	let m2 = p.before_trivia();
	if !p.at(SyntaxKind::Colon) {
		code_expr(p);
	}
	if p.eat_if(SyntaxKind::Colon) {
		code_expr(p);
	} else {
		p.expected_at(m2, "colon");
	}
	p.wrap(m, SyntaxKind::ShowRule);
}

fn contextual(p: &mut Parser, atomic: bool) {
	let m = p.marker();
	p.assert(SyntaxKind::Context);
	code_expr_prec(p, atomic, 0);
	p.wrap(m, SyntaxKind::Contextual);
}

fn conditional(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::If);
	code_expr(p);
	block(p);
	if p.eat_if(SyntaxKind::Else) {
		if p.at(SyntaxKind::If) {
			conditional(p);
		} else {
			block(p);
		}
	}
	p.wrap(m, SyntaxKind::Conditional);
}

fn while_loop(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::While);
	code_expr(p);
	block(p);
	p.wrap(m, SyntaxKind::WhileLoop);
}

fn for_loop(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::For);
	let mut seen = HashSet::new();
	pattern(p, false, &mut seen, None);
	if p.at(SyntaxKind::Comma) {
		let n = p.eat_and_get();
		n.unexpected();
		n.hint("destructuring patterns must be wrapped in parentheses");
		if p.at_set(kind::PATTERN) {
			pattern(p, false, &mut seen, None);
		}
	}
	p.expect(SyntaxKind::In);
	code_expr(p);
	block(p);
	p.wrap(m, SyntaxKind::ForLoop);
}

fn module_import(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Import);
	code_expr(p);
	// Renames the whole module; with items, both are imported.
	if p.eat_if(SyntaxKind::As) {
		p.expect(SyntaxKind::Ident);
	}
	if p.eat_if(SyntaxKind::Colon) {
		if p.at(SyntaxKind::LeftParen) {
			p.with_nl_mode(AtNewline::Continue, |p| {
				let m2 = p.marker();
				p.assert(SyntaxKind::LeftParen);
				import_items(p);
				p.expect_closing_delimiter(m2, SyntaxKind::RightParen);
			});
		} else if !p.eat_if(SyntaxKind::Star) {
			import_items(p);
		}
	}
	p.wrap(m, SyntaxKind::ModuleImport);
}

fn import_items(p: &mut Parser) {
	let m = p.marker();
	while !p.current().is_terminator() {
		let item = p.marker();
		if !p.eat_if(SyntaxKind::Ident) {
			p.unexpected();
		}
		// A nested path, `a.b.c`.
		while p.eat_if(SyntaxKind::Dot) {
			p.expect(SyntaxKind::Ident);
		}
		p.wrap(item, SyntaxKind::ImportItemPath);
		if p.eat_if(SyntaxKind::As) {
			p.expect(SyntaxKind::Ident);
			p.wrap(item, SyntaxKind::RenamedImportItem);
		}
		if !p.current().is_terminator() {
			p.expect(SyntaxKind::Comma);
		}
	}
	p.wrap(m, SyntaxKind::ImportItems);
}

fn module_include(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Include);
	code_expr(p);
	p.wrap(m, SyntaxKind::ModuleInclude);
}

fn break_stmt(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Break);
	p.wrap(m, SyntaxKind::LoopBreak);
}

fn continue_stmt(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Continue);
	p.wrap(m, SyntaxKind::LoopContinue);
}

fn return_stmt(p: &mut Parser) {
	let m = p.marker();
	p.assert(SyntaxKind::Return);
	if p.at_set(kind::CODE_EXPR) {
		code_expr(p);
	}
	p.wrap(m, SyntaxKind::FuncReturn);
}

// A `(` may open a group, an array, a dictionary, closure parameters or a destructuring. The first
// three are tried in one pass; when `=>` or `=` follows, the parser backtracks and reparses, memoising the
// correct parse so that no parenthesised expression is parsed more than twice (`(x: (x: (x) => y) => y)
// => y` would otherwise take time exponential in its depth).
fn expr_with_paren(p: &mut Parser, atomic: bool) {
	if atomic {
		parenthesized_or_array_or_dict(p);
		return;
	}

	let (key, checkpoint) = match p.restore_memo_or_checkpoint() {
		Some(kc)	=> kc,
		None		=> return,
	};
	let prev_len = checkpoint.node_len;

	let kind = parenthesized_or_array_or_dict(p);

	if p.at(SyntaxKind::Arrow) {
		p.restore(checkpoint);
		let m = p.marker();
		params(p);
		if !p.expect(SyntaxKind::Arrow) {
			return;
		}
		code_expr(p);
		p.wrap(m, SyntaxKind::Closure);
	} else if p.at(SyntaxKind::Eq) && kind != SyntaxKind::Parenthesized {
		p.restore(checkpoint);
		let m = p.marker();
		destructuring_or_parenthesized(p, true, &mut HashSet::new());
		if !p.expect(SyntaxKind::Eq) {
			return;
		}
		code_expr(p);
		p.wrap(m, SyntaxKind::DestructAssignment);
	} else {
		return;
	}

	p.memoize_parsed_nodes(key, prev_len);
}

struct GroupState {
	count:				usize,
	maybe_just_parens:	bool,					// `(a)`, until a comma or a pair says otherwise
	kind:				Option<SyntaxKind>,		// `Array` or `Dict`, once known
	seen:				HashSet<String>,		// keys, for duplicate detection
}

// `(1 + 2)`, `(1, "hi", 12cm)` or `(thickness: 3pt)`. A leading colon forces a dictionary, so
// `(: ..a, ..b)` joins two dictionaries where `(..a, ..b)` joins arrays.
fn parenthesized_or_array_or_dict(p: &mut Parser) -> SyntaxKind {
	let mut state = GroupState { count: 0, maybe_just_parens: true, kind: None, seen: HashSet::new() };
	let m = p.marker();
	p.with_nl_mode(AtNewline::Continue, |p| {
		p.assert(SyntaxKind::LeftParen);
		if p.eat_if(SyntaxKind::Colon) {
			state.kind = Some(SyntaxKind::Dict);
		}
		while !p.current().is_terminator() {
			if !p.at_set(kind::ARRAY_OR_DICT_ITEM) {
				p.unexpected();
				continue;
			}
			array_or_dict_item(p, &mut state);
			state.count += 1;
			if !p.current().is_terminator() && p.expect(SyntaxKind::Comma) {
				state.maybe_just_parens = false;
			}
		}
		p.expect_closing_delimiter(m, SyntaxKind::RightParen);
	});

	let kind = if state.maybe_just_parens && state.count == 1 {
		SyntaxKind::Parenthesized
	} else {
		match state.kind {
			Some(k)	=> k,
			None	=> SyntaxKind::Array,
		}
	};
	p.wrap(m, kind);
	kind
}

fn array_or_dict_item(p: &mut Parser, state: &mut GroupState) {
	let m = p.marker();

	if p.eat_if(SyntaxKind::Dots) {
		code_expr(p);
		p.wrap(m, SyntaxKind::Spread);
		state.maybe_just_parens = false;
		return;
	}

	code_expr(p);

	if p.eat_if(SyntaxKind::Colon) {
		code_expr(p);
		if let Some(node) = p.node_mut(m) {
			let key = match node.kind() {
				SyntaxKind::Ident	=> Some(node.text().to_string()),
				SyntaxKind::Str		=> Some(ast::unescape_str(node.text())),
				_					=> None,
			};
			if let Some(key) = key {
				if !state.seen.insert(key.clone()) {
					node.convert_to_error(&format!("duplicate key: {}", key));
				}
			}
		}
		let pair_kind = match p.node_mut(m) {
			Some(n) if n.kind() == SyntaxKind::Ident	=> SyntaxKind::Named,
			_											=> SyntaxKind::Keyed,
		};
		p.wrap(m, pair_kind);
		state.maybe_just_parens = false;
		if state.kind == Some(SyntaxKind::Array) {
			if let Some(n) = p.node_mut(m) {
				n.expected("expression");
			}
		} else {
			state.kind = Some(SyntaxKind::Dict);
		}
	} else if state.kind == Some(SyntaxKind::Dict) {
		if let Some(n) = p.node_mut(m) {
			n.expected("named or keyed pair");
		}
	} else {
		state.kind = Some(SyntaxKind::Array);
	}
}

// A call's argument list, `(12pt, y)`, with any trailing content blocks.
fn args(p: &mut Parser) {
	if !p.directly_at(SyntaxKind::LeftParen) && !p.directly_at(SyntaxKind::LeftBracket) {
		p.expected("argument list");
		if p.at(SyntaxKind::LeftParen) || p.at(SyntaxKind::LeftBracket) {
			p.hint("there may not be any spaces before the argument list");
		}
	}

	let m = p.marker();
	if p.at(SyntaxKind::LeftParen) {
		let m2 = p.marker();
		p.with_nl_mode(AtNewline::Continue, |p| {
			p.assert(SyntaxKind::LeftParen);
			let mut seen = HashSet::new();
			while !p.current().is_terminator() {
				if !p.at_set(kind::ARG) {
					p.unexpected();
					continue;
				}
				arg(p, &mut seen);
				if !p.current().is_terminator() {
					p.expect(SyntaxKind::Comma);
				}
			}
			p.expect_closing_delimiter(m2, SyntaxKind::RightParen);
		});
	}
	while p.directly_at(SyntaxKind::LeftBracket) {
		content_block(p);
	}
	p.wrap(m, SyntaxKind::Args);
}

fn arg<'s>(p: &mut Parser<'s>, seen: &mut HashSet<&'s str>) {
	let m = p.marker();
	if p.eat_if(SyntaxKind::Dots) {
		code_expr(p);
		p.wrap(m, SyntaxKind::Spread);
		return;
	}

	let was_at_expr = p.at_set(kind::CODE_EXPR);
	let text = p.current_text();
	code_expr(p);

	if p.eat_if(SyntaxKind::Colon) {
		if was_at_expr {
			if let Some(n) = p.node_mut(m) {
				if n.kind() != SyntaxKind::Ident {
					n.expected("identifier");
				} else if !seen.insert(text) {
					n.convert_to_error(&format!("duplicate argument: {}", text));
				}
			}
		}
		code_expr(p);
		p.wrap(m, SyntaxKind::Named);
	}
}

fn params(p: &mut Parser) {
	let m = p.marker();
	p.with_nl_mode(AtNewline::Continue, |p| {
		p.assert(SyntaxKind::LeftParen);
		let mut seen = HashSet::new();
		let mut sink = false;
		while !p.current().is_terminator() {
			if !p.at_set(kind::PARAM) {
				p.unexpected();
				continue;
			}
			param(p, &mut seen, &mut sink);
			if !p.current().is_terminator() {
				p.expect(SyntaxKind::Comma);
			}
		}
		p.expect_closing_delimiter(m, SyntaxKind::RightParen);
	});
	p.wrap(m, SyntaxKind::Params);
}

fn param<'s>(p: &mut Parser<'s>, seen: &mut HashSet<&'s str>, sink: &mut bool) {
	let m = p.marker();
	if p.eat_if(SyntaxKind::Dots) {
		if p.at_set(kind::PATTERN_LEAF) {
			pattern_leaf(p, false, seen, Some("parameter"));
		}
		p.wrap(m, SyntaxKind::Spread);
		let had = std::mem::replace(sink, true);
		if had {
			if let Some(n) = p.node_mut(m) {
				n.convert_to_error("only one argument sink is allowed");
			}
		}
		return;
	}

	let was_at_pat = p.at_set(kind::PATTERN);
	pattern(p, false, seen, Some("parameter"));

	if p.eat_if(SyntaxKind::Colon) {
		if was_at_pat {
			if let Some(n) = p.node_mut(m) {
				if n.kind() != SyntaxKind::Ident {
					n.expected("identifier");
				}
			}
		}
		code_expr(p);
		p.wrap(m, SyntaxKind::Named);
	}
}

fn pattern<'s>(p: &mut Parser<'s>, reassignment: bool, seen: &mut HashSet<&'s str>, dupe: Option<&'s str>) {
	if !p.enter() {
		return;
	}
	match p.current() {
		SyntaxKind::Underscore	=> p.eat(),
		SyntaxKind::LeftParen	=> destructuring_or_parenthesized(p, reassignment, seen),
		_						=> pattern_leaf(p, reassignment, seen, dupe),
	}
	p.leave();
}

fn destructuring_or_parenthesized<'s>(p: &mut Parser<'s>, reassignment: bool, seen: &mut HashSet<&'s str>) {
	let mut sink = false;
	let mut count = 0;
	let mut maybe_just_parens = true;

	let m = p.marker();
	p.with_nl_mode(AtNewline::Continue, |p| {
		p.assert(SyntaxKind::LeftParen);
		while !p.current().is_terminator() {
			if !p.at_set(kind::DESTRUCTURING_ITEM) {
				p.unexpected();
				continue;
			}
			destructuring_item(p, reassignment, seen, &mut maybe_just_parens, &mut sink);
			count += 1;
			if !p.current().is_terminator() && p.expect(SyntaxKind::Comma) {
				maybe_just_parens = false;
			}
		}
		p.expect_closing_delimiter(m, SyntaxKind::RightParen);
	});

	if maybe_just_parens && count == 1 && !sink {
		p.wrap(m, SyntaxKind::Parenthesized);
	} else {
		p.wrap(m, SyntaxKind::Destructuring);
	}
}

fn destructuring_item<'s>(
	p:					&mut Parser<'s>,
	reassignment:		bool,
	seen:				&mut HashSet<&'s str>,
	maybe_just_parens:	&mut bool,
	sink:				&mut bool,
) {
	let m = p.marker();

	if p.eat_if(SyntaxKind::Dots) {
		if p.at_set(kind::PATTERN_LEAF) {
			pattern_leaf(p, reassignment, seen, None);
		}
		p.wrap(m, SyntaxKind::Spread);
		let had = std::mem::replace(sink, true);
		if had {
			if let Some(n) = p.node_mut(m) {
				n.convert_to_error("only one destructuring sink is allowed");
			}
		}
		return;
	}

	let was_at_pat = p.at_set(kind::PATTERN);

	// A full checkpoint, since trivia may sit between the identifier and the colon.
	let checkpoint = p.checkpoint();
	if !(p.eat_if(SyntaxKind::Ident) && p.at(SyntaxKind::Colon)) {
		p.restore(checkpoint);
		pattern(p, reassignment, seen, None);
	}

	if p.eat_if(SyntaxKind::Colon) {
		if was_at_pat {
			if let Some(n) = p.node_mut(m) {
				if n.kind() != SyntaxKind::Ident {
					n.expected("identifier");
				}
			}
		}
		pattern(p, reassignment, seen, None);
		p.wrap(m, SyntaxKind::Named);
		*maybe_just_parens = false;
	}
}

// An identifier when binding, any atomic expression when reassigning; the whole expression is parsed
// either way so that a bad one is a single error.
fn pattern_leaf<'s>(p: &mut Parser<'s>, reassignment: bool, seen: &mut HashSet<&'s str>, dupe: Option<&'s str>) {
	if p.current().is_keyword() {
		p.eat_and_get().expected("pattern");
		return;
	} else if !p.at_set(kind::PATTERN_LEAF) {
		p.expected("pattern");
		return;
	}

	let m = p.marker();
	let text = p.current_text();
	code_expr_prec(p, true, 0);

	if !reassignment {
		if let Some(n) = p.node_mut(m) {
			if n.kind() == SyntaxKind::Ident {
				if !seen.insert(text) {
					let what = match dupe {
						Some(d)	=> d,
						None	=> "binding",
					};
					n.convert_to_error(&format!("duplicate {}: {}", what, text));
				}
			} else {
				n.expected("pattern");
			}
		}
	}
}

// ┌───────────────────────────────────────────────────────────────────────────────────────────┐
// │ Parser                                                                                    │
// └───────────────────────────────────────────────────────────────────────────────────────────┘

// The parser holds one token of lookahead. Trivia before it are already in `nodes`; `wrap` leaves them
// outside the node it builds. A newline may turn the token into a fake `End` according to the newline
// mode, which is how a heading stops at the end of its line and a code expression at a line break.
struct Parser<'s> {
	text:		&'s str,
	lexer:		Lexer<'s>,
	nl_mode:	AtNewline,
	token:		Token,
	nodes:		Vec<SyntaxNode>,
	memo:		MemoArena,
	depth:		u32,
}

#[derive(Clone, Debug)]
struct Token {
	kind:		SyntaxKind,		// possibly a fake `End`
	node:		SyntaxNode,
	n_trivia:	usize,
	newline:	Option<Newline>,
	start:		usize,
	prev_end:	usize,
}

#[derive(Clone, Copy, Debug)]
struct Newline {
	column:		Option<usize>,	// in markup only
	parbreak:	bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtNewline {
	Continue,
	Stop,
	ContextualContinue,			// continue only before `else` or `.`
	StopParBreak,
	RequireColumn(usize),		// stop unless the next line is indented past the column
}

impl AtNewline {
	fn stop_at(self, nl: Newline, kind: SyntaxKind) -> bool {
		match self {
			AtNewline::Continue				=> false,
			AtNewline::Stop					=> true,
			AtNewline::ContextualContinue	=> !matches!(kind, SyntaxKind::Else | SyntaxKind::Dot),
			AtNewline::StopParBreak			=> nl.parbreak,
			AtNewline::RequireColumn(min)	=> match nl.column {
				Some(c)	=> c <= min,
				None	=> false,
			},
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Marker(usize);

#[derive(Default)]
struct MemoArena {
	arena:		Vec<SyntaxNode>,
	memo_map:	HashMap<usize, (Range<usize>, PartialState)>,
}

struct Checkpoint {
	node_len:	usize,
	state:		PartialState,
}

#[derive(Clone)]
struct PartialState {
	cursor:		usize,
	lex_mode:	LexMode,
	token:		Token,
}

impl<'s> Parser<'s> {
	fn new(text: &'s str, mode: LexMode) -> Self {
		let mut lexer = Lexer::new(text, mode);
		let nl_mode = AtNewline::Continue;
		let mut nodes = Vec::new();
		let token = Self::lex(&mut nodes, &mut lexer, nl_mode);
		Self { text, lexer, nl_mode, token, nodes, memo: MemoArena::default(), depth: 0 }
	}

	fn finish_into(mut self, kind: SyntaxKind, file: FileId) -> SyntaxNode {
		// A top-level parse always runs to the true end; anything left is swallowed as an error.
		while !self.lexer_done() {
			self.unexpected();
		}
		let mut root = SyntaxNode::inner(kind, self.nodes);
		root.numberise(file, 0);
		root
	}

	fn lexer_done(&self) -> bool {
		self.token.node.kind() == SyntaxKind::End && self.token.start >= self.text.len()
	}

	fn current(&self) -> SyntaxKind { self.token.kind }
	fn at(&self, kind: SyntaxKind) -> bool { self.token.kind == kind }
	fn at_set(&self, set: SyntaxSet) -> bool { set.contains(self.token.kind) }
	fn end(&self) -> bool { self.at(SyntaxKind::End) }
	fn directly_at(&self, kind: SyntaxKind) -> bool { self.token.kind == kind && !self.had_trivia() }
	fn had_trivia(&self) -> bool { self.token.n_trivia > 0 }
	fn had_newline(&self) -> bool { self.token.newline.is_some() }

	fn current_column(&self) -> usize {
		match self.token.newline.and_then(|nl| nl.column) {
			Some(c)	=> c,
			None	=> self.lexer.column(self.token.start),
		}
	}

	fn current_text(&self) -> &'s str {
		match self.text.get(self.token.start..self.current_end()) {
			Some(t)	=> t,
			None	=> "",
		}
	}

	fn current_start(&self) -> usize { self.token.start }
	fn current_end(&self) -> usize { self.lexer.cursor() }

	fn marker(&self) -> Marker { Marker(self.nodes.len()) }

	fn before_trivia(&self) -> Marker { Marker(self.nodes.len() - self.token.n_trivia) }

	fn node_mut(&mut self, m: Marker) -> Option<&mut SyntaxNode> { self.nodes.get_mut(m.0) }

	fn eat_and_get(&mut self) -> &mut SyntaxNode {
		let at = self.nodes.len();
		self.eat();
		&mut self.nodes[at]
	}

	fn eat_if(&mut self, kind: SyntaxKind) -> bool {
		let at = self.at(kind);
		if at {
			self.eat();
		}
		at
	}

	// The callers guarantee the token; eating whatever is there keeps the parse moving regardless.
	fn assert(&mut self, _kind: SyntaxKind) {
		self.eat();
	}

	fn convert_and_eat(&mut self, kind: SyntaxKind) {
		self.token.node.convert_to_kind(kind);
		self.eat();
	}

	fn eat(&mut self) {
		let node = std::mem::replace(&mut self.token.node, SyntaxNode::placeholder(SyntaxKind::End));
		self.nodes.push(node);
		self.token = Self::lex(&mut self.nodes, &mut self.lexer, self.nl_mode);
	}

	// Detaches the trivia before the token so that the next wrap includes them.
	fn flush_trivia(&mut self) {
		self.token.n_trivia = 0;
		self.token.prev_end = self.token.start;
	}

	fn wrap(&mut self, from: Marker, kind: SyntaxKind) {
		let to = self.before_trivia().0;
		let from = from.0.min(to);
		let children: Vec<SyntaxNode> = self.nodes.drain(from..to).collect();
		self.nodes.insert(from, SyntaxNode::inner(kind, children));
	}

	fn wrap_error(&mut self, from: Marker, message: &str) {
		let to = self.before_trivia().0;
		let from = from.0.min(to);
		let len: usize = self.nodes.drain(from..to).map(|n| n.len()).sum();
		let end = self.token.prev_end;
		let text = match self.text.get(end.saturating_sub(len)..end) {
			Some(t)	=> t,
			None	=> "",
		};
		self.nodes.insert(from, SyntaxNode::error(text, message, crate::syntax::Span::detached()));
	}

	// Parses `f` in another lexical mode; the token after it is re-lexed in the outer mode.
	fn enter_modes<F: FnOnce(&mut Parser<'s>)>(&mut self, mode: LexMode, stop: AtNewline, f: F) {
		let previous = self.lexer.mode();
		self.lexer.set_mode(mode);
		self.with_nl_mode(stop, f);
		if mode != previous {
			self.lexer.set_mode(previous);
			self.lexer.jump(self.token.prev_end);
			let keep = self.nodes.len() - self.token.n_trivia;
			self.nodes.truncate(keep);
			self.token = Self::lex(&mut self.nodes, &mut self.lexer, self.nl_mode);
		}
	}

	fn with_nl_mode<F: FnOnce(&mut Parser<'s>)>(&mut self, mode: AtNewline, f: F) {
		let previous = self.nl_mode;
		self.nl_mode = mode;
		f(self);
		self.nl_mode = previous;
		if let Some(nl) = self.token.newline {
			if mode != previous {
				// Restore the real kind, or put a fake end back.
				let actual = self.token.node.kind();
				self.token.kind = if self.nl_mode.stop_at(nl, actual) { SyntaxKind::End } else { actual };
			}
		}
	}

	fn lex(nodes: &mut Vec<SyntaxNode>, lexer: &mut Lexer, nl_mode: AtNewline) -> Token {
		let prev_end = lexer.cursor();
		let mut start = prev_end;
		let (mut kind, mut node) = lexer.next();
		let mut n_trivia = 0;
		let mut had_newline = false;
		let mut parbreak = false;

		while kind.is_trivia() {
			had_newline |= lexer.newline();
			parbreak |= kind == SyntaxKind::Parbreak;
			n_trivia += 1;
			nodes.push(node);
			start = lexer.cursor();
			let (k, n) = lexer.next();
			kind = k;
			node = n;
		}

		let newline = if had_newline {
			let column = if lexer.mode() == LexMode::Markup { Some(lexer.column(start)) } else { None };
			let nl = Newline { column, parbreak };
			if nl_mode.stop_at(nl, kind) {
				kind = SyntaxKind::End;
			}
			Some(nl)
		} else {
			None
		};

		Token { kind, node, n_trivia, newline, start, prev_end }
	}

	// Depth.

	fn enter(&mut self) -> bool {
		if self.depth < MAX_DEPTH {
			self.depth += 1;
			true
		} else {
			self.depth_check_error(None);
			false
		}
	}

	fn leave(&mut self) {
		self.depth = self.depth.saturating_sub(1);
	}

	fn check_depth_until(&mut self, stop: SyntaxSet) -> bool {
		if self.depth < MAX_DEPTH {
			true
		} else {
			self.depth_check_error(Some(stop));
			false
		}
	}

	// Eats at least one token, balancing brackets, and wraps what it ate as one error.
	fn depth_check_error(&mut self, stop: Option<SyntaxSet>) {
		let m = self.marker();
		let mut balance: usize = 0;
		self.with_nl_mode(AtNewline::Continue, |p| {
			loop {
				if p.at_set(syntax_set!(LeftBracket, LeftBrace, LeftParen)) {
					balance = balance.saturating_add(1);
				} else if p.at_set(syntax_set!(RightBracket, RightBrace, RightParen)) {
					balance = balance.saturating_sub(1);
				}
				p.eat();
				let at_stop = match stop {
					Some(s)	=> p.at_set(s),
					None	=> true,
				};
				if (balance == 0 && at_stop) || p.end() {
					break;
				}
			}
		});
		self.wrap_error(m, "maximum parsing depth exceeded");
	}

	// Memo.

	fn memoize_parsed_nodes(&mut self, key: usize, prev_len: usize) {
		let Checkpoint { state, node_len } = self.checkpoint();
		let start = self.memo.arena.len();
		if let Some(parsed) = self.nodes.get(prev_len..node_len) {
			self.memo.arena.extend_from_slice(parsed);
		}
		let range = start..self.memo.arena.len();
		self.memo.memo_map.insert(key, (range, state));
	}

	fn restore_memo_or_checkpoint(&mut self) -> Option<(usize, Checkpoint)> {
		let key = self.current_start();
		match self.memo.memo_map.get(&key).cloned() {
			Some((range, state)) => {
				if let Some(parsed) = self.memo.arena.get(range) {
					self.nodes.extend_from_slice(parsed);
				}
				self.restore_partial(state);
				None
			},
			None => Some((key, self.checkpoint())),
		}
	}

	fn restore(&mut self, checkpoint: Checkpoint) {
		self.nodes.truncate(checkpoint.node_len);
		self.restore_partial(checkpoint.state);
	}

	fn restore_partial(&mut self, state: PartialState) {
		self.lexer.jump(state.cursor);
		self.lexer.set_mode(state.lex_mode);
		self.token = state.token;
	}

	fn checkpoint(&self) -> Checkpoint {
		Checkpoint {
			node_len:	self.nodes.len(),
			state:		PartialState {
				cursor:		self.lexer.cursor(),
				lex_mode:	self.lexer.mode(),
				token:		self.token.clone(),
			},
		}
	}

	// Errors.

	fn expect(&mut self, kind: SyntaxKind) -> bool {
		let at = self.at(kind);
		if at {
			self.eat();
		} else if kind == SyntaxKind::Ident && self.token.kind.is_keyword() {
			self.trim_errors();
			self.eat_and_get().expected(kind.name());
		} else {
			self.expected(kind.name());
		}
		at
	}

	// Turns the opening delimiter at `open` into an error when the closing one is missing.
	fn expect_closing_delimiter(&mut self, open: Marker, kind: SyntaxKind) -> bool {
		let at = self.eat_if(kind);
		if !at {
			if let Some(n) = self.nodes.get_mut(open.0) {
				n.convert_to_error("unclosed delimiter");
			}
		}
		at
	}

	fn expected(&mut self, thing: &str) {
		if self.token.kind.is_error() {
			// Consume an erroneous token so that it keeps the mode it was lexed in.
			self.trim_errors();
			self.eat();
		} else if !self.after_error() {
			let m = self.before_trivia();
			self.expected_at(m, thing);
		}
	}

	fn after_error(&self) -> bool {
		let m = self.before_trivia().0;
		match m.checked_sub(1).and_then(|i| self.nodes.get(i)) {
			Some(n)	=> n.kind().is_error(),
			None	=> false,
		}
	}

	fn expected_at(&mut self, m: Marker, thing: &str) {
		let error = SyntaxNode::error("", &format!("expected {}", thing), crate::syntax::Span::detached());
		let at = m.0.min(self.nodes.len());
		self.nodes.insert(at, error);
	}

	fn hint(&mut self, hint: &str) {
		let m = self.before_trivia().0;
		if let Some(n) = m.checked_sub(1).and_then(|i| self.nodes.get_mut(i)) {
			n.hint(hint);
		}
	}

	fn unexpected(&mut self) {
		self.trim_errors();
		self.eat_and_get().unexpected();
	}

	// Drops trailing zero-length errors before the token.
	fn trim_errors(&mut self) {
		let end = self.before_trivia().0;
		let mut start = end;
		while start > 0 {
			match self.nodes.get(start - 1) {
				Some(n) if n.kind().is_error() && n.is_empty()	=> start -= 1,
				_												=> break,
			}
		}
		self.nodes.drain(start..end);
	}
}
