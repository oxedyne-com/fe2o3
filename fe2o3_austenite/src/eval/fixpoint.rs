// U8 owns this file: the outer fixpoint. Evaluate once; then up to `MAX_PASSES` times stream the pages
// from the top against the previous introspector, recording each page's located elements as it passes
// and handing the page to the sink, stopping when every recorded read answers the same against the
// introspector the pass built. A fifth pass still moving is a warning and its output stands, as in Typst.
//
// Each pass starts clean: the locator, the read log, the context and the loop fuel are reset, and the
// diagnostics of the previous pass are dropped, so a `context` error a first pass makes for want of a
// later label is gone once the label is known. Only the final pass's errors fail the run.
//
// Streaming (addendum 2026-09-23): a pass holds one page at a time. Nothing here keeps a page, a frame or
// a run; what outlives a page is its records in the [`Builder`], and what outlives a pass is the
// [`Introspector`] built from them and the sink's own output.

use crate::diag::{
	Diagnostic,
	DiagnosticKind,
};
use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::intro::{
	self,
	Builder,
	Introspector,
	ReadLog,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::{
	Module,
	Value,
};
use crate::eval::{
	Context,
	Engine,
};
use crate::page::Page;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

pub const MAX_PASSES: u32 = 5;

/// What a compilation laid out: the final introspector and the counts. The pages went to the sink.
#[derive(Clone, Debug)]
pub struct Laid {
	pub intro:		Arc<Introspector>,
	pub pages:		u32,
	pub passes:		u32,
	pub converged:	bool,
}

/// Where a pass's pages go, one at a time. Every pass but the last is discarded, so a sink keeps what it
/// writes provisional until [`finish`](Self::finish), which runs once, after the pass that converged
/// (or the fifth), with that pass's introspector: the outline, the page tree and the ledger come from it.
pub trait PageSink {
	/// Takes a finished page (placed and decorated) and drops it.
	fn page(&mut self, engine: &mut Engine, page: Page) -> Outcome<()>;
	/// Forgets the pass just ended: its output and its glyph sets.
	fn discard_pass(&mut self) -> Outcome<()>;
	fn finish(&mut self, engine: &mut Engine, intro: &Introspector) -> Outcome<()>;
}

/// One pass over the document's content: its pages to the sink, its located elements to the builder.
/// Returns the number of pages. [`Pages`] is the document's; a test may lay out by other means.
pub trait Layouter {
	fn lay<S: PageSink>(
		&mut self,
		engine:		&mut Engine,
		content:	&Content,
		styles:		&StyleChain,
		builder:	&mut Builder,
		sink:		&mut S,
	)
		-> Outcome<u32>;
}

/// A pass's pages, one at a time: the document's are U6b's `flow::Paginator`, each body placed by
/// `driver::place_page`, which records the page's located elements in the builder, then decorated.
pub trait PageSource {
	/// The next page, placed and decorated, with its `page.numbering`; `None` after the last page.
	fn next_page(&mut self, engine: &mut Engine, builder: &mut Builder) -> Outcome<Option<(Page, Value)>>;
}

/// The document layouter.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pages;

impl Layouter for Pages {
	fn lay<S: PageSink>(
		&mut self,
		_engine:	&mut Engine,
		_content:	&Content,
		_styles:	&StyleChain,
		_builder:	&mut Builder,
		_sink:		&mut S,
	)
		-> Outcome<u32>
	{
		// U6b's `flow::paginate` is not on this branch yet. When it is, this is
		// `place(engine, &mut flow::paginate(content, styles), builder, sink)`, the paginator implementing
		// `PageSource` over `next_page`, `driver::place_page` and `flow::decorate::decorate_page`.
		Err(unimplemented("flow", "paginate"))
	}
}

/// Streams one pass: each page from the source is counted with its numbering, then goes to the sink and
/// is dropped. Returns the number of pages.
pub fn place<P: PageSource, S: PageSink>(
	engine:		&mut Engine,
	source:		&mut P,
	builder:	&mut Builder,
	sink:		&mut S,
)
	-> Outcome<u32>
{
	let mut n = 0;
	while let Some((page, numbering)) = res!(source.next_page(engine, builder)) {
		n += 1;
		builder.page(page.number, &numbering);
		res!(sink.page(engine, page));
	}
	Ok(n)
}

pub fn run<S: PageSink>(engine: &mut Engine, module: &Module, sink: &mut S) -> Outcome<Laid> {
	run_with(engine, module, &mut Pages, sink)
}

/// The fixpoint over any layouter. The first pass reads the engine's introspector as it stands, empty
/// for a fresh engine, so a caller holding a previous compilation's introspector starts warm.
pub fn run_with<L: Layouter, S: PageSink>(
	engine:		&mut Engine,
	module:		&Module,
	layouter:	&mut L,
	sink:		&mut S,
)
	-> Outcome<Laid>
{
	let mark = engine.diags.len();
	let fuel = engine.fuel;
	let styles = StyleChain::root();
	let mut passes = 0;
	loop {
		passes += 1;
		engine.diags.truncate(mark);
		engine.locator.reset();
		engine.reads = ReadLog::default();
		engine.context = Context::default();
		engine.fuel = fuel;
		let mut builder = Builder::new();
		let laid = layouter.lay(engine, &module.content, &styles, &mut builder, sink);
		let pages = match laid {
			Ok(n)	=> n,
			Err(e)	=> {
				res!(sink.discard_pass());
				return Err(e);
			}
		};
		let intro = Arc::new(builder.finish());
		let converged = res!(engine.reads.holds(&intro));
		if !converged && passes < MAX_PASSES {
			res!(sink.discard_pass());
			engine.intro = intro;
			continue;
		}
		if !converged {
			res!(warn_unsettled(engine, &intro));
		}
		engine.intro = intro.clone();
		if let Some(d) = engine.diags[mark..].iter().find(|d| d.is_error()) {
			let e = err!("{}", d.message; Input, Invalid);
			res!(sink.discard_pass());
			return Err(e);
		}
		res!(sink.finish(engine, &intro));
		return Ok(Laid { intro, pages, passes, converged });
	}
}

/// Typst's non-convergence report: a summary, then one warning for each counter, state, query or
/// location that still moved.
fn warn_unsettled(engine: &mut Engine, intro: &Introspector) -> Outcome<()> {
	let mut notes: Vec<String> = Vec::new();
	for r in res!(engine.reads.changed(intro)) {
		let note = intro::describe(r, intro);
		if !notes.contains(&note) {
			notes.push(note);
		}
	}
	engine.diags.push(Diagnostic::warning(DiagnosticKind::Limit, Span::detached(), "document did not converge within five attempts")
		.with_hint(fmt!("see {} additional warning{} for more details", notes.len(),
			if notes.len() == 1 { "" } else { "s" }))
		.with_hint("see https://typst.app/help/convergence for help"));
	for n in notes {
		engine.diags.push(Diagnostic::warning(DiagnosticKind::Limit, Span::detached(), n));
	}
	Ok(())
}
