//! `mathgallery` -- the maths-layout comparison harness.
//!
//! Sets each exemplar expression as a single centred display equation on its own page and writes it as
//! a one-page PDF, so each can be turned into a tightly-cropped PNG and set beside the Typst oracle that
//! uses the identical font. This is the living comparison harness for tuning the maths layout against
//! Latin Modern Math: every difference between our render and the oracle is a layout error to close.
//!
//! Usage: `mathgallery [OUTPUT_DIR]` (default `mathgallery-out`). Each expression `N` is written to
//! `eqN.pdf`; convert to PNG in the shell with `pdftoppm -png -r 300 eqN.pdf eqN` then trim.

use oxedyne_fe2o3_austenite::{
	doc::{
		self,
		Block,
		Style,
	},
	driver::{
		self,
		Config,
	},
	font::FontMetrics,
	math::{
		Atom,
		Class,
	},
	page::PageGeometry,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	set::FontSet,
	shape::Dir,
};

use std::sync::Arc;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	let out_dir = match args.get(1) {
		Some(s)	=> s.clone(),
		None	=> "mathgallery-out".to_string(),
	};

	let fonts	= Arc::new(res!(oxedyne_fe2o3_austenite::fonts::libertinus()));
	let geom	= PageGeometry::a4();
	let style	= Style::default();

	res!(std::fs::create_dir_all(&out_dir));

	for (i, expr) in exemplars().into_iter().enumerate() {
		let n = i + 1;
		res!(render_one(fonts.clone(), geom, style, &expr, &out_dir, n));
	}

	println!("mathgallery: wrote {} equation PDF(s) to {}/", exemplars().len(), out_dir);
	Ok(())
}

/// Authors a one-equation document, runs the driver to a fixed point, and writes the first page as a
/// PDF. One display equation is the whole body, so the page holds only the maths, ready to crop.
fn render_one(
	fonts:		Arc<FontSet>,
	geom:		PageGeometry,
	style:		Style,
	expr:		&Atom,
	out_dir:	&str,
	n:			usize,
)
	-> Outcome<()>
{
	let blocks = vec![Block::equation(expr.clone(), false)];
	let (document, _heads) = res!(doc::author(fonts.clone(), geom, style, None, &blocks, None));

	let metrics	= FontMetrics::new(fonts.clone(), Role::Body, Dir::Ltr, style.body_size);
	let out		= res!(driver::run(&document, &metrics, Config::default()));

	let pdf = res!(oxedyne_fe2o3_austenite::emit::pdf::render_document(&out.pages));
	res!(std::fs::write(fmt!("{}/eq{}.pdf", out_dir, n), pdf));
	Ok(())
}

/// The exemplar set, each built as an [`Atom`] tree. The Typst oracle for each is the reference.
fn exemplars() -> Vec<Atom> {
	vec![
		// 1. E = m c^2
		Atom::row(vec![
			Atom::var("E"),
			Atom::rel("="),
			Atom::var("m"),
			Atom::sup(Atom::var("c"), Atom::num("2")),
		]),
		// 2. a/b -- a display fraction.
		Atom::frac(Atom::var("a"), Atom::var("b")),
		// 3. the quadratic formula.
		Atom::row(vec![
			Atom::var("x"),
			Atom::rel("="),
			Atom::frac(
				Atom::row(vec![
					Atom::bin("\u{2212}"),	// a unary minus, the minus glyph
					Atom::var("b"),
					Atom::bin("\u{00B1}"),	// plus-or-minus
					Atom::sqrt(Atom::row(vec![
						Atom::sup(Atom::var("b"), Atom::num("2")),
						Atom::bin("\u{2212}"),
						Atom::num("4"),
						Atom::var("a"),
						Atom::var("c"),
					])),
				]),
				Atom::row(vec![
					Atom::num("2"),
					Atom::var("a"),
				]),
			),
		]),
		// 4. (a/(b+c))^2 -- a squared fenced fraction.
		Atom::sup(
			Atom::fence(
				'(',
				Atom::frac(
					Atom::var("a"),
					Atom::row(vec![Atom::var("b"), Atom::bin("+"), Atom::var("c")]),
				),
				')',
			),
			Atom::num("2"),
		),
		// 5. x^2 + x_i - x_i^2 -- scripts: sup, sub, subsup.
		Atom::row(vec![
			Atom::sup(Atom::var("x"), Atom::num("2")),
			Atom::bin("+"),
			Atom::sub(Atom::var("x"), Atom::Sym("i".to_string(), Class::Ord)),
			Atom::bin("\u{2212}"),
			Atom::subsup(Atom::var("x"), Atom::Sym("i".to_string(), Class::Ord), Atom::num("2")),
		]),
		// 6. (x+1)/(y-1) inside grown parens.
		Atom::fence(
			'(',
			Atom::frac(
				Atom::row(vec![Atom::var("x"), Atom::bin("+"), Atom::num("1")]),
				Atom::row(vec![Atom::var("y"), Atom::bin("\u{2212}"), Atom::num("1")]),
			),
			')',
		),
	]
}
