// U8 owns this file: the outer fixpoint. Evaluate once; then up to `MAX_PASSES` times stream the pages
// from the top against the previous introspector, recording each page's located elements as it passes
// and handing the page to the sink, stopping when every recorded read answers the same against the
// introspector the pass built. A fifth pass still moving is a warning and its output stands, as in Typst.
//
// Each pass starts clean: the locator, the read log, the context and the loop fuel are reset, and the
// diagnostics of the previous pass are dropped, so a `context` error a first pass makes for want of a
// later label is gone once the label is known. Only the final pass's errors fail the run.
//
// What fails a pass outright and what waits are Typst's own division. A show rule's error (a reference
// to a label the first pass has not seen above all) is delayed by `Engine::delay`: the element shows as
// nothing, the pass completes, and the error counts only if the final pass still holds it. An error from
// anywhere else, layout above all, ends the compile in the pass it occurs in.
//
// Streaming (addendum 2026-09-23): a pass holds one page at a time. Nothing here keeps a page, a frame or
// a run; what outlives a page is its records in the [`Builder`], and what outlives a pass is the
// [`Introspector`] built from them and the sink's own output.

use crate::diag::{
	Diagnostic,
	DiagnosticKind,
};
use crate::eval::content::Content;
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
use crate::flow;
use crate::page::Page;
use crate::syntax::Span;
use crate::timings::Phase;

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

/// A pass's pages, one at a time: the document's are U6b's `flow::Paginator`, each body placed with its
/// furniture by `driver::place_page`, which records the page's located elements in the builder.
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
		engine:		&mut Engine,
		content:	&Content,
		styles:		&StyleChain,
		builder:	&mut Builder,
		sink:		&mut S,
	)
		-> Outcome<u32>
	{
		place(engine, &mut flow::paginate(content, styles), builder, sink)
	}
}

// The paginator is the document's page source: the next page's furniture is laid out and its body placed on
// its page, which records the page's located elements in the builder.
impl PageSource for flow::Paginator {
	fn next_page(&mut self, engine: &mut Engine, builder: &mut Builder) -> Outcome<Option<(Page, Value)>> {
		match res!(self.next_placed(engine, builder)) {
			None						=> Ok(None),
			Some((page, setup))	=> {
				builder.document(res!(intro::document_info(&setup.styles)));
				Ok(Some((page, setup.numbering.clone())))
			},
		}
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
		res!(engine.timed(Phase::Sink, |engine| sink.page(engine, page)));
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
		engine.body = None;
		if let Some(t) = engine.timings.as_mut() {
			t.begin_pass();
		}
		let mut builder = Builder::new();
		let laid = layouter.lay(engine, &module.content, &styles, &mut builder, sink);
		let pages = match laid {
			Ok(n)	=> n,
			Err(e)	=> {
				res!(engine.timed(Phase::Sink, |_| sink.discard_pass()));
				return Err(e);
			}
		};
		let (intro, converged) = res!(engine.timed(Phase::Settle, |engine| -> Outcome<_> {
			let intro = Arc::new(builder.finish());
			let converged = res!(engine.reads.holds(&intro));
			Ok((intro, converged))
		}));
		if !converged && passes < MAX_PASSES {
			res!(engine.timed(Phase::Sink, |_| sink.discard_pass()));
			engine.intro = intro;
			continue;
		}
		if !converged {
			res!(warn_unsettled(engine, &intro));
		}
		engine.intro = intro.clone();
		dedupe_errors(&mut engine.diags, mark);
		if let Some(d) = engine.diags[mark..].iter().find(|d| d.is_error()) {
			let e = err!("{}", d.message; Input, Invalid);
			res!(engine.timed(Phase::Sink, |_| sink.discard_pass()));
			return Err(e);
		}
		res!(engine.timed(Phase::Finish, |engine| sink.finish(engine, &intro)));
		return Ok(Laid { intro, pages, passes, converged });
	}
}

/// Typst's `deduplicate`: an error met again at one span with one message is reported once. A body laid out
/// again, measured and then placed, shows its failed element each time.
fn dedupe_errors(diags: &mut Vec<Diagnostic>, mark: usize) {
	let mut kept: Vec<Diagnostic> = Vec::with_capacity(diags.len() - mark);
	for d in diags.drain(mark..) {
		if d.is_error() && kept.iter().any(|k| k.is_error() && k.span == d.span && k.message == d.message) {
			continue;
		}
		kept.push(d);
	}
	diags.extend(kept);
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
