//! The Typst reader: line-oriented source into the surface tree of [`ast::Item`](super::ast::Item).
//!
//! The scan is deliberately simple, one pass over the lines. A line whose first non-blank character
//! is `=` is a heading, its level the run of leading `=`; a line opening with `-` or `+` (and a space)
//! is a list item, a run of them a bullet or numbered list; every other non-blank line accumulates into
//! a paragraph. A blank line, a heading, or the start of the other kind of block closes whatever is
//! open. Whitespace within a paragraph is made insignificant here, so the line breaker downstream owns
//! the measure. Byte offsets are tracked across the raw lines so each [`Item`] carries a true [`Span`].
//!
//! A closed paragraph's text is then scanned for inline emphasis by [`parse_inlines`]: `*strong*` and
//! `_emph_`. A delimiter pairs only when it flanks a word, so a stray asterisk, a date's slash, or
//! `and/or` is left as ordinary text rather than opening an emphasis that never closes.
//!
//! A Typst code statement (`#import`/`#let`/`#set`/`#show`) or a line-leading standalone template call
//! (`#name(...)` or `#name[...]`) is not set: it is skipped. When its delimiters do not balance on the
//! opening line -- a `#figure(...)`, `#table(...)`, `#aside-box[...]`, or a `#let x = (...)` data array
//! that spans many lines -- the reader consumes following lines, tracking nesting across `()`, `[]` and
//! `{}` and respecting string literals, until the delimiters balance, so the whole span renders nothing.
//! Every such skip is recorded by name into a [`Refusals`] the parse returns beside its items, so a
//! caller reports the constructs it dropped rather than losing them silently. A `#columns(n)[ ... ]`
//! wrapper is the exception the reader does not drop whole: its body is re-parsed and set single-column.
//!
//! Typst comments are stripped before a line is classified: a `//` runs to the line's end, and a
//! `/* ... */` spans lines, both dropped -- except within a `"..."` string or a `` `code` `` span, and a
//! `//` right after `:` is kept, so a bare URL survives. Inline glossary and index calls, defined in the
//! book template (`#gs`, `#gscap`, `#gsi`, `#gscapi`, `#glossind`, `#glossindcap`, the term-dictionary
//! family `#g`, `#gcap`, `#gi`, `#gcapi`, `#t`, `#tcap`, `#graw`, and `#idx`, `#idx-main`, `#idx-as`,
//! `#idx-main-as`, `#index`, `#index-main`, `#idx-nested`), plus a `#link(dest)[text]` hyperlink, are read
//! by [`parse_inlines`]: a glossary term sets its display text, bold-italic on its first document use; a
//! visible term or index call sets its display text plain; a link sets its text and drops the destination;
//! a pure index marker sets nothing. An inline `#func[...]` the reader does not know is consumed, recorded
//! in the summary, and its bracketed body folded in, so its words survive but its raw markup never leaks.

use crate::ir::FloatPlacement;
use crate::ir::Length;
use crate::ir::Span;
use crate::table::Align;

use super::ast::{AlignSpec, ClosureAlign, FigureBody, Inline, Item, ListItem, TableSpec};
use super::mathparse;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::RwLock;

// The book's `term-dict`, set once by the loader before parsing so the term-dictionary glossary family
// (`t`, `tcap`, `graw`, `g`, `gi`, `gcap`, `gcapi`) resolves a key to its display value while the key
// identity is still known. A process-global rather than a threaded argument, mirroring the image base:
// the inline reader sets one run at a time and carries no book context of its own. `None` until the
// loader installs one, in which case every key falls back to its own text.
static TERM_DICT: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Records the book's `term-dict`, read from a sibling `terms.typ`, so the term-dictionary glossary
/// family resolves each key to its value at parse time. Installing a fresh map replaces any prior one.
pub fn set_term_dict(dict: HashMap<String, String>) -> Outcome<()> {
	let mut guard = lock_write!(TERM_DICT, "While recording the term dictionary");
	*guard = Some(dict);
	Ok(())
}

/// The display value a `term-dict` key resolves to, or `None` when no map is installed or it holds no
/// such key. A poisoned lock reads as absent rather than failing the parse: a missing value falls back
/// to the key text, which is exactly the safe degradation here.
pub(crate) fn term_value(key: &str) -> Option<String> {
	match TERM_DICT.read() {
		Ok(guard)	=> guard.as_ref().and_then(|m| m.get(key).cloned()),
		Err(_)		=> None,
	}
}

/// Why a construct was refused rather than set: the axis a per-site diagnostic reports alongside its
/// name and location, so a reader can tell a categorical limit from a todo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalClass {
	// Typst's own general evaluation primitives -- `#import`, `#let`, `#set`, `#show` -- which run an
	// arbitrary expression to a value. Austenite's driver converges a fixed, two-pass ledger to a point;
	// it does not carry a Typst-style evaluator, so these are a categorical limit of the architecture
	// (see the crate root's own note that Typst treats a document as a program and Austenite does not),
	// not a specific feature waiting to be added.
	FixedPoint,
	// A construct that reads the page's own laid-out state back into the document -- `#context`,
	// `#query`, `#locate`, a counter's or state's `.at`/`.get`, `#measure`, `#layout`. This is exactly
	// Typst's self-observation model, which the ledger exists to answer in Austenite's own terms; a
	// construct landing here is a real gap the ledger could plausibly close, not a limit of the design.
	Introspective,
	// Anything else skipped: a specific call or wrapper (`#columns`, an unknown standalone or inline
	// `#func`, an unknown term-dictionary key) that names no fundamental barrier -- just not yet read.
	Unsupported,
}

impl RefusalClass {
	/// Classifies a refusal by the source name it was recorded under. Matched by substring rather than
	/// an exact keyword, since the same construct is recorded under different shapes depending on how it
	/// was written (`#context`, a bare `context` inside a longer call name) -- this is a diagnostic
	/// classifier, not a parser, so a generous match that occasionally over-reaches is the right trade.
	fn classify(name: &str) -> Self {
		let lower = name.to_lowercase();
		for kw in ["#import", "#let", "#set", "#show"] {
			if lower.starts_with(kw) {
				return RefusalClass::FixedPoint;
			}
		}
		const INTROSPECTIVE: [&str; 8] =
			["context", "query", "locate", "counter.at", "counter.get", "state", "measure", "layout"];
		if INTROSPECTIVE.iter().any(|kw| lower.contains(kw)) {
			return RefusalClass::Introspective;
		}
		RefusalClass::Unsupported
	}

	/// The word `--explain` prints for this class.
	pub fn label(&self) -> &'static str {
		match self {
			RefusalClass::FixedPoint		=> "fixed-point",
			RefusalClass::Introspective	=> "introspective",
			RefusalClass::Unsupported		=> "unsupported",
		}
	}
}

/// One site the reader passed over rather than set: the source name it was written with (carrying its
/// leading `#`, so it reads back as source), the byte span it was found at, and why it was refused.
/// The span is the whole containing line for a code statement or standalone call, or the whole
/// containing item (a paragraph, a heading) for an inline call found within one -- Austenite's inline
/// scanner does not keep the fine per-character offset once a paragraph's lines have been joined and its
/// whitespace collapsed, so the enclosing item is the finest boundary available without a deeper rework
/// of the reader than this diagnostic upgrade is for.
#[derive(Clone, Debug)]
pub struct Refusal {
	pub name:	String,
	pub span:	Span,
	pub class:	RefusalClass,
	// The source file this site was read from, for `--explain`'s "file:line:col". Empty immediately
	// after parsing, since a lone parse of a source string carries no filename of its own; the book
	// assembler ([`crate::book::assemble`]) tags each chapter's (and the root's own) refusals with the
	// real path once assembly is back in a context that has one -- see [`Refusals::tag_file`].
	pub file:	String,
}

/// Every site the reader refused across one parse (or, once [`Refusals::merge`] has folded chapters
/// together, across a whole book). Kept as a flat list of [`Refusal`]s rather than the old name-keyed
/// tally, so a caller can still print the terse one-line [`Refusals::report`] but can also walk every
/// site for `--explain`'s per-site listing. Empty when the reader set everything it met.
#[derive(Clone, Debug, Default)]
pub struct Refusals {
	sites: Vec<Refusal>,
}

impl Refusals {
	/// Records one refused construct by the source name it was written with (with its leading `#`) and
	/// the span it was found at, classifying it from the name.
	pub(crate) fn record(&mut self, name: &str, span: Span) {
		let class = RefusalClass::classify(name);
		self.sites.push(Refusal { name: name.to_string(), span, class, file: String::new() });
	}

	/// Builds a table directly from a caller's own sites, for a test (or another future caller outside
	/// the parser) that wants a known `Refusals` without driving a real parse to produce one.
	pub fn from_sites(sites: Vec<Refusal>) -> Self {
		Self { sites }
	}

	pub fn is_empty(&self) -> bool { self.sites.is_empty() }

	/// Sets every site's `file` that is not already set, so a caller assembling several chapters can tag
	/// each chapter's refusals with its own path right after parsing it, before folding them into the
	/// book's running total with [`merge`](Self::merge) -- at which point every site already carries the
	/// file it came from, and a second tagging (the root's own trailing markup, read after every
	/// include) touches only the sites still unset.
	pub fn tag_file(&mut self, file: &str) {
		for r in &mut self.sites {
			if r.file.is_empty() {
				r.file = file.to_string();
			}
		}
	}

	/// Every refused site, in the order the reader met them.
	pub fn sites(&self) -> &[Refusal] { &self.sites }

	/// The number of distinct construct names refused.
	pub fn kinds(&self) -> usize { self.entries().len() }

	/// The total count of refused sites across every name.
	pub fn total(&self) -> usize { self.sites.len() }

	/// Each refused construct name with its count, ordered by descending count then name, so the report
	/// leads with the construct that cost the most.
	pub fn entries(&self) -> Vec<(String, usize)> {
		let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
		for r in &self.sites {
			*counts.entry(r.name.as_str()).or_insert(0) += 1;
		}
		let mut v: Vec<(String, usize)> = counts.into_iter().map(|(k, c)| (k.to_string(), c)).collect();
		v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
		v
	}

	/// Folds another parse's refusals into this one, so a caller assembling several chapters reports one
	/// total rather than a summary per file. `other` is consumed rather than borrowed: a caller merging a
	/// just-parsed chapter's refusals into the book's running total has no further use for its own copy.
	pub fn merge(&mut self, other: Refusals) {
		self.sites.extend(other.sites);
	}

	/// A one-line report -- "skipped 3 unsupported constructs: #show (2), #columns (1)" -- or `None` when
	/// nothing was skipped, so a caller prints the line only when it has something to say. Unchanged in
	/// wording from before this unit: `--explain` is the new, detailed report, this terse one stays the
	/// default.
	pub fn report(&self) -> Option<String> {
		if self.sites.is_empty() {
			return None;
		}
		let parts: Vec<String> = self.entries().into_iter()
			.map(|(n, c)| fmt!("{} ({})", n, c))
			.collect();
		let n = self.total();
		Some(fmt!("skipped {} unsupported construct{}: {}",
			n, if n == 1 { "" } else { "s" }, parts.join(", ")))
	}
}

/// Parses a whole Ingot source string into its surface items. The only error is an empty heading --
/// a `=` marker with no title -- which names the offending 1-based line.
pub fn document(src: &str) -> Outcome<Vec<Item>> {
	let (items, _) = res!(document_with_refusals(src));
	Ok(items)
}

/// Parses a source string into its surface items and, alongside, the [`Refusals`] of every construct
/// the reader passed over rather than set -- a `#let`/`#set`/`#show`/`#import` code line, an unknown
/// line-leading `#func(...)` call, a `#columns` wrapper, and any unhandled inline `#func[...]`. The
/// caller prints the summary so a dropped construct is reported rather than lost silently.
pub fn document_with_refusals(src: &str) -> Outcome<(Vec<Item>, Refusals)> {
	let tfns = crate::lang::rules::TemplateFns::new();
	let cfns = crate::lang::rules::ContentFns::new();
	document_with_templates(src, crate::lang::rules::Bindings::new(&tfns, &cfns))
}

/// Records a refusal for every claim reference (`#claim-refs`/`#claim-label`) that sits in a context the
/// layout does not gather into the reverse claim index. A top-level body run -- a paragraph, a list entry, a
/// callout body -- and a table cell both feed the index (see `doc::build_pieces`, reached for a cell through
/// `doc::build_grid`); a claim code in a heading title, a figure or table caption, or a footnote body is
/// dropped by the layout, so it would otherwise vanish from the index (and, for a `#claim-label`, from the
/// margin) with no trace. Making that a refusal keeps the silent-loss class this project guards against out
/// of the reverse index. Gathering from a heading or caption needs anchor support there and is a later
/// increment; until then the code is reported, not dropped.
fn flag_unindexed_claim_refs(items: &[Item], skips: &mut Refusals) {
	for item in items {
		match item {
			// A body run and a list entry are gathered; only a claim reference nested inside a footnote of one
			// escapes the index, so the top-level runs are scanned as indexed and their footnotes are not.
			Item::Paragraph { runs, span, .. }	=> scan_claim_refs(runs, true, *span, "a paragraph", skips),
			Item::List { items: entries, .. }	=> for e in entries { flag_list_item_claim_refs(e, skips); },
			// A callout body is gathered like the main flow; recurse so a claim reference in it is indexed and
			// only its non-body sub-contexts (a caption, a footnote) are flagged.
			Item::Box { items: inner, .. }		=> flag_unindexed_claim_refs(inner, skips),
			Item::Scoped { items: inner, .. }	=> flag_unindexed_claim_refs(inner, skips),
			// A heading title and a caption are still not gathered, so a claim reference in either is refused.
			// A table cell now runs through the body's own segment pipeline (`doc::build_grid` ->
			// `doc::build_pieces`), which weaves the cell's `#claim-refs`/`#claim-label` anchor into the reverse
			// claim index exactly as a body run does, so it is no longer refused.
			Item::Heading { runs, span, .. }	=> scan_claim_refs(runs, false, *span, "a heading title", skips),
			Item::Figure { caption, span, .. } => {
				if let Some(cap) = caption {
					scan_claim_refs(cap, false, *span, "a figure caption", skips);
				}
			},
			_ => {},
		}
	}
}

/// [`flag_unindexed_claim_refs`] for one list entry: its own runs are gathered (indexed), and its nested
/// child items are walked as their own contexts.
fn flag_list_item_claim_refs(entry: &ListItem, skips: &mut Refusals) {
	scan_claim_refs(&entry.runs, true, Span::new(0, 0), "a list entry", skips);
	flag_unindexed_claim_refs(&entry.children, skips);
}

/// Scans an inline run for claim references and records a refusal for each that will not reach the reverse
/// index. `indexed` is true for a top-level body run (a paragraph, list entry or callout body), where a
/// claim reference IS gathered and so is left alone; a footnote body is never gathered, so its own runs are
/// always scanned as unindexed regardless of where the footnote sits.
fn scan_claim_refs(runs: &[Inline], indexed: bool, span: Span, context: &str, skips: &mut Refusals) {
	for run in runs {
		match run {
			Inline::MarginNote { codes, .. } if !indexed && !codes.is_empty() =>
				skips.record(&fmt!("claim reference in {} is not indexed", context), span),
			Inline::Footnote(inner) => scan_claim_refs(inner, false, span, "a footnote body", skips),
			_ => {},
		}
	}
}

/// As [`document_with_refusals`], with the `#let` bindings (`binds`) in scope: a call to a furniture
/// function -- `#pr-note[ ... ]`, `#aside-box(title: [..])[ ... ]` -- expands into a padded box, and a
/// reference to a content binding -- `#greet("world")`, a bare `#intro` -- expands into its re-read markup,
/// rather than either being tallied as a skipped construct. A body re-parsed here carries the same `binds`,
/// so a call nested inside another's body expands too. With empty maps this is exactly
/// [`document_with_refusals`].
///
/// After the surface tree is built, [`flag_unindexed_claim_refs`] records a refusal for any claim reference
/// that landed in a context the layout does not gather into the reverse claim index. This runs once, on the
/// whole assembled tree -- the recursive re-parse of a `#columns`/`#styled-box` body reaches for
/// [`parse_items`] directly, so a nested claim reference is flagged once here rather than again per level.
pub fn document_with_templates(src: &str, binds: crate::lang::rules::Bindings<'_, '_>)
	-> Outcome<(Vec<Item>, Refusals)>
{
	let (items, mut skips) = res!(parse_items(src, binds));
	flag_unindexed_claim_refs(&items, &mut skips);
	Ok((items, skips))
}

/// The surface-tree parse proper, without the [`flag_unindexed_claim_refs`] post-pass -- so a recursively
/// re-parsed body (a `#columns`/`#styled-box` wrapper's content) is not validated twice, once here and again
/// when its parent walks the spliced items. [`document_with_templates`] wraps this with that one validation.
fn parse_items(src: &str, binds: crate::lang::rules::Bindings<'_, '_>)
	-> Outcome<(Vec<Item>, Refusals)>
{
	let mut skips:		Refusals	= Refusals::default();
	let mut items:		Vec<Item>	= Vec::new();
	let mut lines:		Vec<String>	= Vec::new();	// the current paragraph's constituent lines
	let mut para_start:	u32			= 0;			// byte offset of the paragraph's first line
	let mut para_end:	u32			= 0;			// byte offset just past its last line's content
	let mut offset:		u32			= 0;			// running byte offset of the current line's start
	let mut line_no					= 0usize;		// 1-based, for a diagnostic

	// The stack of open list levels, innermost last. Each level records the leading-space indent of its
	// markers, so a deeper marker opens a sub-list under the current item and a shallower one closes back
	// to the matching level; an empty stack means no list is open. A list is a run of marker lines that a
	// blank line does not break (Typst continues an enum across a gap), but any other content flushes.
	let mut stack:		Vec<ListFrame>	= Vec::new();

	// A fenced code block, while one is open: the verbatim lines gathered so far and the byte offset it
	// began at. A ```-fence opens it, the next ```-fence closes it; between them every line is kept as it
	// stands, its indentation and markup untouched.
	let mut code:		Option<(Vec<String>, u32)>	= None;

	// A multi-line Typst code statement or standalone template call being skipped: the net bracket depth
	// still open across the lines consumed so far, and whether a string literal is currently open. `None`
	// when not skipping. While it is `Some`, every line is consumed and nothing is set until the delimiters
	// balance.
	let mut skip:		Option<SkipState>	= None;

	// A multi-line construct whose whole text is gathered so it can be parsed rather than skipped: a
	// `#figure(...)`, a bare `#table(...)`, or a `#let name = (...)` data array feeding a table. `None`
	// when none is open. The accumulated text is dispatched by its kind when the delimiters balance.
	let mut capture:	Option<Capture>		= None;

	// Data arrays declared by `#let name = (...)` and referenced by a table's `..name.flatten()` spread:
	// the name maps to the flat sequence of cells the array holds, each cell a run of inline markup.
	// Populated as the arrays are read, so a later figure resolves its cells against them.
	let mut arrays:		HashMap<String, Vec<Vec<Inline>>>	= HashMap::new();

	// Whether a `/* ... */` block comment is open across the line break. A `//` line comment never
	// straddles a line, so it needs no carried state.
	let mut comment	= CommentState { in_block: false };

	// `split_inclusive` keeps the trailing newline on each piece, so the running offset stays a true
	// byte position into the source rather than drifting by the count of stripped terminators.
	for raw in src.split_inclusive('\n') {
		line_no += 1;
		let start = offset;
		offset = offset.saturating_add(raw.len() as u32);

		// Strip the line terminator without consuming a real character: the final line may carry
		// neither a newline nor a carriage return.
		let mut line = raw;
		if let Some(s) = line.strip_suffix('\n') { line = s; }
		if let Some(s) = line.strip_suffix('\r') { line = s; }
		let end = start.saturating_add(line.len() as u32);

		// Strip Typst comments before classifying the line, but not while a fenced code block or a
		// multi-line call skip is open: inside a fence a `//` is verbatim, and a skipped span is dropped
		// whole regardless. The span above is computed from the raw line, so a diagnostic caret still
		// points into the source.
		let stripped;
		let line = if code.is_none() && skip.is_none() {
			stripped = strip_comments(line, &mut comment);
			stripped.as_str()
		} else {
			line
		};

		let trimmed = line.trim_start();

		// A multi-line code statement or standalone call is being skipped: keep consuming lines, tracking
		// bracket nesting across `()`, `[]` and `{}` and respecting string literals, until the delimiters
		// balance. Nothing between the opener and its close is set. This takes precedence over every other
		// rule, since the span is code, not markup.
		if let Some(state) = skip.as_mut() {
			scan_brackets(line, state);
			if !state.has_open_bracket() {
				skip = None;
			}
			continue;
		}

		// A multi-line construct is being gathered whole: keep appending its lines and tracking the bracket
		// balance until the delimiters close, then dispatch the accumulated text by its kind. Like the skip
		// above, this takes precedence over the markup rules, since the span is a code construct.
		if let Some(cap) = capture.as_mut() {
			cap.buf.push_str(line);
			cap.buf.push('\n');
			scan_brackets(line, &mut cap.state);
			if !cap.state.has_open_bracket() {
				let done = capture.take();
				if let Some(cap) = done {
					dispatch_capture(cap, &mut items, &mut arrays, &mut skips, binds);
				}
			}
			continue;
		}

		// A fenced code block takes precedence over every other rule: inside it, only a closing fence is
		// special and every other line is verbatim, so its own `=`, `-` or `*` carry no markup meaning.
		if let Some((buf, cstart)) = code.as_mut() {
			if is_fence(trimmed) {
				items.push(Item::Code { lines: std::mem::take(buf), span: Span::new(*cstart, end) });
				code = None;
			} else {
				buf.push(line.to_string());
			}
			continue;
		}
		if is_fence(trimmed) {
			// An opening fence closes any paragraph or list, then begins a verbatim block. The fence line
			// itself (and any language tag on it) is not kept.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
			flush_list(&mut items, &mut stack);
			code = Some((Vec::new(), start));
			continue;
		}

		// A display maths block ($ ... $) spans several source lines, but every branch below classifies a
		// line on its own -- so, left unchecked, a `=`-lead alignment row reads as a heading, a `-`-lead row
		// as a list marker, and a blank row between stacked lines flushes the paragraph early, each stealing
		// the line before the paragraph ever reaches `mathparse` whole. While an odd number of unescaped `$`
		// have accumulated in the running paragraph the block is still open, so the blank/heading/list checks
		// below are skipped for as long as it is: a fence opening or a capture opener (a `#`-led construct)
		// still takes precedence regardless, since neither shape occurs inside genuine display maths.
		let math_block_open = math_open(&lines);

		if trimmed.is_empty() && !math_block_open {
			// A blank line closes the paragraph it follows, but not an open list: Typst continues an enum
			// (or bullet list) across a blank line between items, restarting the numbering only when other
			// content intervenes. The list is therefore held open here; the marker branch joins a following
			// item of the same kind, while any other line -- a paragraph, heading, figure, fence or code
			// line -- flushes it first, so two lists parted by real content still restart.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
		} else if let Some(kind) = capture_opener(trimmed, binds)
			.filter(|k| !(matches!(k, CaptureKind::ContentCall(_)) && !lines.is_empty()))
		{
			// A standalone content-binding reference mid-paragraph joins the paragraph inline rather than
			// splicing a block, matching Typst's inline value flow: only a reference with no paragraph open
			// splices its expanded blocks (the `filter` above lets an open-paragraph `#name` fall through to
			// the paragraph arm). A markup builtin (`#lorem`, `#v`, `#pagebreak`) is block-position but not
			// blank-line-gated: `capture_opener` already admits it only own-line (a balanced call with nothing
			// but whitespace after its `)`, or a multi-line span), so a builtin directly beneath a prose line
			// with no blank between (`prose\n#pagebreak()`) closes the paragraph and sets its own block, exactly
			// as `= heading\n#pagebreak()` and `#section-banner` already do -- Typst turns the page there whether
			// or not a blank line parts the two, so a paragraph before the break must not silently swallow it.
			// A builtin with prose on the SAME line (`#lorem(5) more`) is not own-line, so it never reaches here;
			// inline mid-prose support is a later unit. Every other capture kind -- a figure, a bare table, a
			// data array -- flushes the paragraph and is gathered as before.
			//
			// A multi-line construct the reader sets rather than skips. It closes any open block, then its
			// whole text is gathered by the check
			// at the top of the loop until the delimiters balance, and parsed by [`dispatch_capture`].
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
			flush_list(&mut items, &mut stack);
			let mut state	= SkipState::new();
			scan_brackets(line, &mut state);
			let mut buf		= String::new();
			buf.push_str(line);
			buf.push('\n');
			let cap = Capture { kind, buf, state, start };
			if !cap.state.has_open_bracket() {
				dispatch_capture(cap, &mut items, &mut arrays, &mut skips, binds);	// the whole construct closed on one line
			} else {
				capture = Some(cap);
			}
		} else if trimmed.starts_with("#line(") && call_inner(trimmed, "line").is_some() {
			// A standalone `#line(length:.., stroke:..)` horizontal divider (the appendix brackets a note
			// with one above and below). It closes any open block and sets a stroked rule; a multi-line
			// `#line(` that does not close on this line falls through to the skip path below.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
			flush_list(&mut items, &mut stack);
			if let Some(rule) = parse_line_rule(trimmed) {
				items.push(rule);
			}
		} else if trimmed.starts_with("#print-glossary(") {
			// A line-leading `#print-glossary()`: the glossary section's Term/Definition table. It closes any
			// open block and emits a placeholder the book layer fills once the whole document's glossary terms
			// are known -- unlike the surrounding template calls it is set in place, not recorded as a skip.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
			flush_list(&mut items, &mut stack);
			items.push(Item::PrintGlossary { span: Span::new(start, end) });
		} else if let Some(decision) = code_skip(trimmed) {
			// A Typst code statement (`#import`, `#let`, `#set`, `#show`) or a line-leading standalone call
			// to a template function Austenite does not yet run: it closes any open block and is skipped.
			// The styling and computation layer is a later increment; the prose around it still sets. When
			// its delimiters do not balance on this line, the multi-line span is consumed by the check at the
			// top of the loop until they do.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
			flush_list(&mut items, &mut stack);
			// The recorded span is the opening line alone, even for a construct whose delimiters run on
			// for several more: that is where a reader wants `--explain`'s caret to land, and the true
			// closing offset is not known until the multi-line skip above closes, several iterations on.
			skips.record(&construct_name(trimmed), Span::new(start, end));
			if let CodeSkip::Multi(state) = decision {
				skip = Some(state);
			}
		} else if lines.is_empty() && is_code_reference(trimmed) && !names_scalar_alone(trimmed, binds.sfns) {
			// A line-leading code-mode reference the reader cannot run -- a bare `#name` bound to nothing, a
			// field/method access `#name.foo`, an `#if`/`#for`/`#while` control keyword, or an anonymous
			// `#{ ... }`/`#( ... )` block. A bound `#name` was expanded by `capture_opener` above; a
			// `#name(`/`#name[` call and the `#let`/`#set`/`#show`/`#import` keywords were refused by
			// `code_skip`. Each of these resolves to a value or runs code in Typst, so setting its source as
			// literal prose would leak a `#` onto the page (the very thing an expanded content-binding body
			// carrying `#if`/`#{` would do); it is refused with its span instead. The `lines.is_empty()` guard
			// keeps a reference mid-paragraph joining the line inline, as Typst does, rather than refusing it.
			// A bare `#name` naming a SCALAR binding is exempt (`names_scalar_alone`): it falls through to the
			// paragraph arm below, where `flush_para`'s own `substitute_scalars` replaces it with the bound
			// value, so a scalar standing alone on its line sets its value just as one mid-prose already does.
			// An UNBOUND standalone name is not exempt, so it stays the visible refusal it has always been.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
			flush_list(&mut items, &mut stack);
			skips.record(&construct_name(trimmed), Span::new(start, end));
		} else if trimmed.starts_with('=') && !math_block_open {
			// A heading closes any paragraph or list above it, then stands on its own line.
			flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
			flush_list(&mut items, &mut stack);
			let level = trimmed.chars().take_while(|&c| c == '=').count();
			let raw = trimmed[level..].trim();	// '=' is ASCII, so a byte slice at the count is safe
			if raw.is_empty() {
				return Err(err!(
					"Empty heading on line {}: a `=` marker must be followed by a title.", line_no;
					Input, Invalid, Missing));
			}
			let (title, label) = split_label(raw);
			if title.is_empty() {
				return Err(err!(
					"Heading on line {} has a label but no title.", line_no; Input, Invalid, Missing));
			}
			// The title carries inline markup like any run, so a glossary term, an index call, emphasis or a
			// maths span in a heading sets its display text rather than leaking its raw source into the head
			// and the table of contents. An inline reference to a content binding (`= Product #stamp`) splices
			// its expanded body first, and a bare `#name` naming a scalar `#let` value binding (`= Product
			// #version`) substitutes its display text next, the same as a paragraph's.
			let head_span = Span::new(start, end);
			let title = substitute_content_calls(&title, binds, &mut skips, head_span);
			let title = substitute_scalars(&title, binds.sfns);
			items.push(Item::Heading {
				level:	level as u8,
				runs:	parse_inlines_in(&title, head_span, &mut skips),
				label,
				span:	head_span,
			});
		} else {
			// A list marker joins the list stack, unless a display maths block is open, in which case a
			// `-`/`+`-lead row is part of the equation, not a bullet: it falls through to the paragraph arm
			// below like every other captured line.
			match if math_block_open { None } else { marker(trimmed) } {
				Some((ord, text)) => {
					// A list item. It closes any open paragraph, then joins the list stack by its indentation:
					// a deeper marker opens a sub-list under the current item, a shallower one closes back to
					// the matching level, and a same-indent marker of the other kind ends the list and starts
					// one of the new kind. The item's text carries inline emphasis like any run.
					flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
					let indent = line.chars().take_while(|c| c.is_whitespace()).count();
					// A list item is running prose like a paragraph, so an inline content-fn reference in it --
					// `+ ... written as M#oxe.` -- splices its expanded body first, exactly as `flush_para` does
					// for a paragraph. Without this the item bypassed the pre-pass and leaked a raw `#oxe`/`#name`
					// while the same reference in a paragraph beside it expanded, an inconsistency a reader sees.
					let item_span	= Span::new(start, end);
					let text		= substitute_content_calls(&text, binds, &mut skips, item_span);
					let runs		= parse_inlines_in(&text, item_span, &mut skips);
					list_marker(&mut items, &mut stack, indent, ord, runs, start, end);
				},
				None => {
					// Any other non-blank line joins the running paragraph, closing a list first; its own line
					// break and indentation carry no meaning, only its words. This is also where a blank,
					// heading-lead or list-marker-lead line lands while a display maths block is open.
					flush_list(&mut items, &mut stack);
					if lines.is_empty() {
						para_start = start;
					}
					lines.push(line.to_string());
					para_end = end;
				},
			}
		}
	}

	// A source that ends without a closing blank line still closes its last paragraph or list; an
	// unterminated code fence still yields the block it had gathered.
	flush_para(&mut items, &mut lines, para_start, para_end, &mut skips, binds);
	flush_list(&mut items, &mut stack);
	if let Some((buf, cstart)) = code {
		items.push(Item::Code { lines: buf, span: Span::new(cstart, offset) });
	}
	// A construct left open at end of source is dispatched with what it gathered, so a missing closer
	// still yields its best-effort figure or table rather than swallowing the tail silently.
	if let Some(cap) = capture {
		dispatch_capture(cap, &mut items, &mut arrays, &mut skips, binds);
	}
	Ok((items, skips))
}

/// Is a display maths block still open across the paragraph lines gathered so far? A `$` toggles the
/// state; a `\`-escaped one (`\$`) is skipped, mirroring the same escape in [`parse_inlines_in`], so it
/// never toggles. An odd running count means the block opened on some earlier line and has not yet met
/// its close, which is what lets [`document_with_refusals`] keep capturing lines the per-line classifier
/// would otherwise steal as a heading, a list marker, or a paragraph-flushing blank.
fn math_open(lines: &[String]) -> bool {
	let mut open = false;
	for line in lines {
		let mut chars = line.chars();
		while let Some(c) = chars.next() {
			if c == '\\' {
				chars.next();	// the escaped character, taken literally
			} else if c == '$' {
				open = !open;
			}
		}
	}
	open
}

/// Is this already-left-trimmed line a ```` ``` ```` code fence? An opening fence may carry a language
/// tag (```` ```rust ````); a closing fence is bare. Either way it opens with three backticks.
fn is_fence(trimmed: &str) -> bool {
	trimmed.starts_with("```")
}

/// Reads a list marker at the start of an already-left-trimmed line: `-` opens a bullet item, `+` a
/// numbered one. The marker must be the whole line or be followed by whitespace, so a dash inside a word
/// or a `+1` is ordinary prose, not a marker. Returns the item's kind and its text with the marker and
/// surrounding whitespace removed.
fn marker(trimmed: &str) -> Option<(bool, String)> {
	let first	= trimmed.chars().next()?;
	let ordered	= match first {
		'-'	=> false,
		'+'	=> true,
		_	=> return None,
	};
	let rest = &trimmed[first.len_utf8()..];
	if rest.is_empty() {
		return Some((ordered, String::new()));
	}
	if rest.starts_with(|c: char| c.is_whitespace()) {
		return Some((ordered, rest.trim().to_string()));
	}
	None
}

/// One open level of a possibly-nested list while the reader accumulates it. `indent` is the leading-space
/// width of the level's markers, so a deeper marker opens a child level and a shallower one closes this
/// level back into the item it hung under.
struct ListFrame {
	indent:		usize,
	ordered:	bool,
	items:		Vec<ListItem>,
	start:		u32,
	end:		u32,
}

/// Attaches a marker line to the open list stack by its `indent`, opening or closing nested levels as the
/// indentation and kind require. A deeper marker opens a sub-list under the current item; a shallower one
/// closes the deeper level(s) first; a same-indent marker continues the level when its kind matches and
/// otherwise ends it and starts a fresh list of the new kind, as the flat reader did.
fn list_marker(
	items:	&mut Vec<Item>,
	stack:	&mut Vec<ListFrame>,
	indent:	usize,
	ord:	bool,
	runs:	Vec<Inline>,
	start:	u32,
	end:	u32,
)
{
	// Close every open level deeper than this marker: a dedent ends the nested list(s), each folding into
	// the item it hung under.
	while stack.last().map_or(false, |f| f.indent > indent) {
		if let Some(frame) = stack.pop() {
			fold(items, stack, frame);
		}
	}
	match stack.last_mut() {
		Some(top) if top.indent == indent && top.ordered == ord => {
			// Same level, same kind: another item of the open list.
			top.items.push(ListItem { runs, children: Vec::new() });
			top.end = end;
		},
		Some(top) if top.indent == indent => {
			// Same indent, the other kind: the open list ends and a fresh one of the new kind begins.
			if let Some(frame) = stack.pop() {
				fold(items, stack, frame);
			}
			stack.push(ListFrame {
				indent, ordered: ord, items: vec![ListItem { runs, children: Vec::new() }], start, end });
		},
		// Deeper than the current level (a sub-list), or the first marker of a list: open a new level. A
		// deeper level becomes a child of the current item when it folds.
		_ => stack.push(ListFrame {
			indent, ordered: ord, items: vec![ListItem { runs, children: Vec::new() }], start, end }),
	}
}

/// Folds a closed list level into the tree: it becomes an [`Item::List`] hanging under the current item of
/// the level below, or a top-level item when no level remains open.
fn fold(items: &mut Vec<Item>, stack: &mut Vec<ListFrame>, frame: ListFrame) {
	let list = Item::List {
		ordered:	frame.ordered,
		items:		frame.items,
		span:		Span::new(frame.start, frame.end),
	};
	match stack.last_mut() {
		Some(parent) => match parent.items.last_mut() {
			Some(item)	=> {
				item.children.push(list);
				parent.end = frame.end;
			},
			// A nested level always opens after its parent item exists, so this arm is unreachable in
			// practice; a stray level is kept as a top-level item rather than dropped.
			None		=> items.push(list),
		},
		None => items.push(list),
	}
}

/// Closes every open list level into the item tree. The deepest level folds into its parent's current item
/// first, so a nested list lands under the item it hung under; the outermost becomes a top-level
/// [`Item::List`]. An empty stack flushes nothing, so a stray flush between two paragraphs costs nothing.
fn flush_list(items: &mut Vec<Item>, stack: &mut Vec<ListFrame>) {
	while let Some(frame) = stack.pop() {
		fold(items, stack, frame);
	}
}

/// Closes the paragraph being accumulated, if any: its lines are joined, their whitespace collapsed,
/// and the result pushed as one [`Item::Paragraph`] spanning the source it came from. An empty
/// accumulator flushes nothing, so a run of blank lines closes a paragraph only once.
fn flush_para(
	items:	&mut Vec<Item>,
	lines:	&mut Vec<String>,
	start:	u32,
	end:	u32,
	skips:	&mut Refusals,
	binds:	crate::lang::rules::Bindings<'_, '_>,
)
{
	if lines.is_empty() {
		return;
	}
	let text = normalise_ws(&lines.join(" "));
	// A trailing `<name>` labels the block -- in practice a display equation, `$ ... $ <eq_x>` -- and is
	// stripped before the runs are read, so the maths span stands alone and lowers to a numbered equation
	// rather than a rich paragraph. Ordinary prose ends in a full stop, so the conservative `split_label`
	// (a single whitespace-free token in angle brackets at the very end) does not fire on it.
	let (body, label) = split_label(&text);
	let span = Span::new(start, end);
	// An inline mid-prose reference to a bound content binding -- `M#oxe`, `see #stamp("v2") for details` --
	// splices its argument-substituted body into the surrounding prose here, before the scalar pass and the
	// inline scanner, exactly as the own-line reference splices its blocks: the words before and after the
	// call are kept, and the body's own markup is then read by the one downstream scanner. Runs first so a
	// scalar reference the body carries still substitutes below.
	let body = substitute_content_calls(&body, binds, skips, span);
	// A bare `#name` naming a scalar `#let` value binding substitutes its display text before the inline
	// scanner runs, so "Version #version." reads the same as if the number or string had been typed in
	// place. Runs before `parse_inlines_in`, never inside it, so it never touches a raw code span, inline
	// maths, or any other captured construct (a table, a figure, a `#context` block) -- those are gathered
	// and dispatched on a wholly separate path and never reach here.
	let body = substitute_scalars(&body, binds.sfns);
	let runs = parse_inlines_in(&body, span, skips);
	items.push(Item::Paragraph { runs, label, span });
	lines.clear();
}

/// Collapses every run of whitespace to a single space and trims the ends, so a paragraph's set width
/// is left to the line breaker rather than to the source's own line breaks and indentation.
fn normalise_ws(s: &str) -> String {
	s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits a whitespace-collapsed paragraph into inline runs in Typst's markup: `*strong*`, `_emph_`, a
/// `@label` cross-reference, and `\`-escapes. An emphasis delimiter pairs only when it flanks a word --
/// whitespace or an opening bracket before it and a non-space after to open, the reverse to close -- so
/// `fe2o3_net`, `5 * 3` and a lone `_` are ordinary text. A backslash sets the next character literally,
/// so `\$`, `\#`, `\_` and `\@` appear as themselves. An unpaired delimiter, or an `@` with no label
/// after it, is ordinary text. Nesting is a later increment: the first valid closer ends a run.
pub(crate) fn parse_inlines(text: &str) -> Vec<Inline> {
	let mut skips = Refusals::default();
	// A table cell, a caption or a flattened array cell has no item-level span of its own to attribute a
	// refusal to (see `Refusal`'s own doc comment on why the item, not the character, is the finest
	// boundary kept); this thin wrapper already threw the summary away before this unit, so a zero span
	// changes nothing a caller could observe.
	parse_inlines_in(text, Span::new(0, 0), &mut skips)
}

/// The inline scanner proper, recording every unhandled inline call into `skips`, at `span` (the whole
/// containing item -- a paragraph, a heading, a list item -- rather than the call's own narrower
/// position; see `Refusal`'s doc comment), so a `#func[...]` the reader cannot set is reported rather
/// than leaked into the running text. [`parse_inlines`] is the thin wrapper for callers -- table cells,
/// captions, flattening -- that do not surface the summary.
fn parse_inlines_in(text: &str, span: Span, skips: &mut Refusals) -> Vec<Inline> {
	let chars:	Vec<char>	= text.chars().collect();
	let n					= chars.len();
	let mut runs:	Vec<Inline>	= Vec::new();
	let mut plain			= String::new();	// ordinary text gathered before the next run
	let mut i				= 0usize;
	while i < n {
		let c = chars[i];
		// A backslash escapes the next character, which is then set as itself.
		if c == '\\' && i + 1 < n {
			plain.push(chars[i + 1]);
			i += 2;
			continue;
		}
		// An inline maths span between dollars. A `\$` was already turned into a literal above, so a `$`
		// reaching here opens maths. If it parses, it is a maths run; if not, the literal `$...$` is kept.
		if c == '$' {
			if let Some(close) = (i + 1..n).find(|&j| chars[j] == '$') {
				let inner: String = chars[i + 1..close].iter().collect();
				if let Ok(atom) = mathparse::parse(&inner) {
					if !plain.is_empty() {
						runs.push(Inline::Text(std::mem::take(&mut plain)));
					}
					runs.push(Inline::Math(atom));
					i = close + 1;
					continue;
				}
			}
		}
		// An inline code span, `raw` between backticks: its content is verbatim, no markup within.
		if c == '`' {
			if let Some(close) = (i + 1..n).find(|&j| chars[j] == '`') {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Code(chars[i + 1..close].iter().collect()));
				i = close + 1;
				continue;
			}
		}
		// An inline footnote. Its bracketed content is markup, carried as inline runs so the note sets its
		// own emphasis at the foot of the page. The mark falls after the run before it.
		if c == '#' {
			if let Some((note, next)) = footnote_call(&chars, i, span, skips) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Footnote(note));
				i = next;
				continue;
			}
		}
		// The function-call form of emphasis, `#emph[...]`, set exactly as `_..._`: its bracketed content is
		// markup, so it takes the same expansion, and a call nested within it renders its display text.
		if c == '#' {
			if let Some((inner, next)) = emph_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				push_emphasis(&mut runs, false, &inner, span, skips);
				i = next;
				continue;
			}
		}
		// The function-call form of strength, `#strong[...]` or `#strong("...")`, set exactly as `*...*`:
		// the bracket form's content is markup and takes the same expansion as the emph call above; the
		// paren form's quoted-string argument is plain text bold in full. A paren argument that is not a
		// plain string -- a bare identifier or an expression this reader cannot evaluate -- is left for
		// the generic call handler below, which records the refusal rather than guessing at its text.
		if c == '#' {
			if let Some((inner, next)) = strong_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				push_emphasis(&mut runs, true, &inner, span, skips);
				i = next;
				continue;
			}
		}
		// Typst's superscript, `#super[...]` or `#super("...")`. Its content is usually a short string or
		// number, reduced to display text here and set raised and smaller by the block layer.
		if c == '#' {
			if let Some((text, next)) = super_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Super(text));
				i = next;
				continue;
			}
		}
		// Typst's subscript, `#sub[...]` or `#sub("...")`, e.g. `CO#sub[2]`. Its content reduces to
		// display text here and is set dropped and smaller by the block layer.
		if c == '#' {
			if let Some((text, next)) = sub_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Sub(text));
				i = next;
				continue;
			}
		}
		// An inline glossary or index call defined in the book template. A glossary term is a run of its
		// own, so [`doc::author`] can set it bold-italic on first use; a visible index call sets its
		// display text, which may itself carry markup, so it is parsed and folded in; a pure index marker
		// sets nothing.
		if c == '#' {
			if let Some((call, next)) = glossary_call(&chars, i, span, skips) {
					// An index marker is a run of its own, woven in before the display so the block layer
					// records the term's occurrence at this point; the display, where the call has one, follows.
					let push_index = |runs: &mut Vec<Inline>, plain: &mut String, index: Option<IndexKey>, skips: &mut Refusals| {
						if let Some(k) = index {
							if !plain.is_empty() {
								runs.push(Inline::Text(std::mem::take(plain)));
							}
							// The display markup becomes its own runs, so the index page sets an emphasised entry
							// italic and a display/sort split shows the display -- parsed exactly as the body's is.
							let display = parse_inlines_in(&k.display, span, skips);
							runs.push(Inline::Index { term: k.term, sub: k.sub, display, main: k.main });
						}
					};
					match call {
						Call::Glossary { term, display, index } => {
							push_index(&mut runs, &mut plain, index, skips);
							if !plain.is_empty() {
								runs.push(Inline::Text(std::mem::take(&mut plain)));
							}
							runs.push(Inline::Glossary { term, display });
						},
						Call::Visible { display, index } => {
							push_index(&mut runs, &mut plain, index, skips);
							let sub = parse_inlines_in(&display, span, skips);
							// A plain display folds back into the running text, keeping the fast single-run
							// path; a display carrying markup becomes its own runs.
							if let [Inline::Text(t)] = sub.as_slice() {
								plain.push_str(t);
							} else {
								if !plain.is_empty() {
									runs.push(Inline::Text(std::mem::take(&mut plain)));
								}
								runs.extend(sub);
							}
						},
						Call::Invisible { index } => {
							push_index(&mut runs, &mut plain, index, skips);
						},
					}
				i = next;
				continue;
			}
		}
		// An inline citation, `#cite(<key>)` or `#cite(<a>, <b>)`. Its keys become a cite run the block
		// layer resolves to "(Author Year)" against the bibliography; a citation with no readable key is
		// dropped rather than left as raw source.
		if c == '#' {
			if let Some((keys, next)) = cite_call(&chars, i) {
				if !keys.is_empty() {
					if !plain.is_empty() {
						runs.push(Inline::Text(std::mem::take(&mut plain)));
					}
					runs.push(Inline::Cite(keys));
				}
				i = next;
				continue;
			}
		}
		// An inline claim marker, `#claim-label(...)` or `#claim-refs(...)`, from the book's claims
		// machinery. A `claim-label` registers invisible metadata AND sets a compressed code in the outside
		// margin: it emits a `MarginNote` run, which sets nothing in the body column but records a zero-width
		// anchor the overlay pass draws the code against after convergence. A `claim-refs` registers metadata
		// only, with no visible output, so it is consumed and emits nothing. Either way the body prose closes
		// over the marker's place, matching Typst's own body flow.
		if c == '#' {
			if let Some((display, codes, next)) = claim_call(&chars, i) {
				// A call naming a margin display or any reference code emits a MarginNote; a call naming
				// neither is consumed and sets nothing.
				if !display.is_empty() || !codes.is_empty() {
					if !plain.is_empty() {
						runs.push(Inline::Text(std::mem::take(&mut plain)));
					}
					runs.push(Inline::MarginNote { display, codes });
				}
				i = next;
				continue;
			}
		}
		// The `#raw("...")` call form of inline code.
		if c == '#' {
			if let Some((text, next)) = raw_call(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::Code(text));
				i = next;
				continue;
			}
		}
		// A Typst hyperlink, `#link("url")[text]` or `#link(<label>)[text]`. The link text is what a print
		// reader sees, so its markup is parsed and folded into the running line; the destination has no place
		// on a page with no clickable annotation and is dropped. A `#link("url")` with no bracket sets the
		// URL itself as its text, as Typst does.
		if c == '#' {
			if let Some((body, next)) = link_call(&chars, i, span, skips) {
				if let [Inline::Text(t)] = body.as_slice() {
					plain.push_str(t);
				} else {
					if !plain.is_empty() {
						runs.push(Inline::Text(std::mem::take(&mut plain)));
					}
					runs.extend(body);
				}
				i = next;
				continue;
			}
		}
		// A Typst cross-reference: `@` then a label.
		if c == '@' {
			if let Some((label, next)) = at_label(&chars, i) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				runs.push(Inline::PageRef(label));
				i = next;
				continue;
			}
		}
		if (c == '*' || c == '_') && is_opener(&chars, i) {
			if let Some(close) = find_closer(&chars, i + 1, c) {
				if !plain.is_empty() {
					runs.push(Inline::Text(std::mem::take(&mut plain)));
				}
				let inner: String = chars[i + 1..close].iter().collect();
				push_emphasis(&mut runs, c == '*', &inner, span, skips);
				i = close + 1;
				continue;
			}
		}
		// An inline `#func(...)` or `#func[...]` call none of the handlers above claimed: a template function
		// the reader cannot yet run. It is recorded by name and consumed whole, so its raw markup no longer
		// leaks into the set text. When it wraps a single `[...]` content group -- the common shape of a Typst
		// content function -- that body is parsed and folded in, keeping its words rather than dropping them;
		// a call with only paren arguments (`#v(1em)`, `#colbreak()`) sets nothing where it stood.
		if c == '#' {
			if let Some((body, next, name)) = unknown_call(&chars, i, span, skips) {
				skips.record(&name, span);
				if let Some(body) = body {
					if let [Inline::Text(t)] = body.as_slice() {
						plain.push_str(t);
					} else {
						if !plain.is_empty() {
							runs.push(Inline::Text(std::mem::take(&mut plain)));
						}
						runs.extend(body);
					}
				}
				i = next;
				continue;
			}
		}
		// Typst's smartypants: a run of hyphens in markup text becomes em/en dashes, longest match first
		// (`---` before `--`), and three-or-more dots become an ellipsis. Every other branch above has
		// already claimed `` ` `` and `$` before falling through here, so this never touches a raw code
		// span or a maths span -- only ordinary prose reaches this fallback. A `\-`/`\.` was already turned
		// into a literal above and never joins a run counted here, matching Typst's own escape.
		if c == '-' || c == '.' {
			let mut run = 0usize;
			while chars.get(i + run) == Some(&c) { run += 1; }
			let mut left = run;
			if c == '-' {
				while left > 0 {
					if left >= 3			{ plain.push('\u{2014}'); left -= 3; }	// --- em dash
					else if left == 2		{ plain.push('\u{2013}'); left = 0; }	// -- en dash
					else					{ plain.push('-'); left = 0; }
				}
			} else if left >= 3 {
				plain.push('\u{2026}');	// ... ellipsis
				left -= 3;
				for _ in 0..left { plain.push('.'); }
			} else {
				for _ in 0..left { plain.push('.'); }
			}
			i += run;
			continue;
		}
		plain.push(c);
		i += 1;
	}
	if !plain.is_empty() {
		runs.push(Inline::Text(plain));
	}
	if runs.is_empty() {
		runs.push(Inline::Text(String::new()));	// a paragraph of pure delimiters keeps one empty run
	}
	runs
}

/// Pushes an emphasised run (`*strong*` when `strong`, else `_emph_`) onto `runs`, reading its inner
/// markup. When the inner is plain text the run keeps the fast flat path -- one [`Inline::Strong`] or
/// [`Inline::Emph`]. When it carries a glossary term, an index call or a maths span -- as
/// `*The captation #gsi[attractor]*` does -- the emphasis is expanded: its plain stretches take the
/// emphasis face and the embedded calls become their own runs, so a call nested in emphasis renders its
/// display text rather than leaking its raw source. A glossary term keeps its own first-use bold-italic
/// (which subsumes the surrounding emphasis), so only the plain stretches carry the emphasis face.
fn push_emphasis(runs: &mut Vec<Inline>, strong: bool, inner: &str, span: Span, skips: &mut Refusals) {
	let sub = parse_inlines_in(inner, span, skips);
	if let [Inline::Text(t)] = sub.as_slice() {
		runs.push(if strong { Inline::Strong(t.clone()) } else { Inline::Emph(t.clone()) });
		return;
	}
	for run in sub {
		match run {
			// A plain stretch takes the emphasis face.
			Inline::Text(t)					=> runs.push(if strong { Inline::Strong(t) } else { Inline::Emph(t) }),
			// An inner run of the opposite face (`*_word_*`, `_*word*_`) multiplies the two faces to a
			// bold-italic run rather than losing the outer; a run of the same face, a glossary term or a
			// maths span keeps its own face, one level of nesting being all the flat vocabulary carries.
			Inline::Emph(t) if strong		=> runs.push(Inline::BoldItalic(t)),
			Inline::Strong(t) if !strong	=> runs.push(Inline::BoldItalic(t)),
			other							=> runs.push(other),
		}
	}
}

/// Does the delimiter at `i` flank the left of a word? A non-space must follow it, and the start of the
/// paragraph, whitespace, or an opening bracket must precede it.
fn is_opener(chars: &[char], i: usize) -> bool {
	match chars.get(i + 1) {
		Some(c) if !c.is_whitespace()	=> {},
		_								=> return false,
	}
	match i.checked_sub(1).and_then(|p| chars.get(p)) {
		None		=> true,
		Some(&p)	=> p.is_whitespace() || matches!(p, '(' | '[' | '{' | '"' | '\''),
	}
}

/// Does the delimiter at `j` flank the right of a word? A non-space must precede it, and the end of the
/// paragraph, whitespace, or closing punctuation must follow.
fn is_closer(chars: &[char], j: usize) -> bool {
	match j.checked_sub(1).and_then(|p| chars.get(p)) {
		Some(p) if !p.is_whitespace()	=> {},
		_								=> return false,
	}
	match chars.get(j + 1) {
		None		=> true,
		Some(&c)	=> c.is_whitespace()
			|| matches!(c, ')' | ']' | '}' | '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\''),
	}
}

/// The index of the first valid closing `delim` at or after `start`, or `None` when the run never
/// closes -- in which case the opener is ordinary text.
fn find_closer(chars: &[char], start: usize, delim: char) -> Option<usize> {
	(start..chars.len()).find(|&j| chars[j] == delim && is_closer(chars, j))
}

/// Reads a Typst cross-reference at `i` (an `@`): the label of letters, digits and `- _ :` that follows,
/// and the index just past it. `None` when no label char follows, so a bare or escaped `@` is ordinary
/// text. A trailing `.` is not a label character, so `@intro.` at the end of a sentence keeps its stop.
fn at_label(chars: &[char], i: usize) -> Option<(String, usize)> {
	let start	= i + 1;
	let mut j	= start;
	while j < chars.len() && is_label_char(chars[j]) {
		j += 1;
	}
	if j == start {
		return None;
	}
	Some((chars[start..j].iter().collect(), j))
}

/// A character legal within a Typst label. Deliberately excludes `.`, so a label does not swallow the
/// full stop that ends a sentence.
fn is_label_char(c: char) -> bool {
	c.is_alphanumeric() || matches!(c, '-' | '_' | ':')
}

/// Reads an inline `#raw("...")` at `i`, returning its literal content and the index past the closing
/// `")`. `None` when the shape does not match, so a `#raw` written any other way is left as ordinary
/// text. Escaped quotes inside the string are not handled -- a later refinement.
fn raw_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open	= at_lit(chars, i, "#raw(\"")?;
	let close	= (open..chars.len()).find(|&j| chars[j] == '"')?;
	if chars.get(close + 1) != Some(&')') {
		return None;
	}
	Some((chars[open..close].iter().collect(), close + 2))
}

/// Reads an inline `#link(dest)[text]` (or a bare `#link(dest)`) at `i` (a `#`), returning the link's
/// display runs and the index just past it. The destination -- a `"url"` string or a `<label>` -- is read
/// and discarded, the page carrying no clickable annotation; the bracketed text is what the reader sees,
/// so it is parsed for its own markup. A `#link(dest)` with no following `[...]` sets the destination
/// string itself as its text, as Typst does. Any unhandled inline call within the text is recorded into
/// `skips`. `None` when the shape is not a link call or its arguments do not close.
fn link_call(chars: &[char], i: usize, span: Span, skips: &mut Refusals) -> Option<(Vec<Inline>, usize)> {
	let Some(open) = at_lit(chars, i, "#link") else { return None; };
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let Some((dest, after_dest)) = read_group(chars, open) else { return None; };
	// A following `[...]` group is the link text; without one, the destination stands as the text.
	if chars.get(after_dest) == Some(&'[') {
		let Some((body, next)) = read_group(chars, after_dest) else { return None; };
		return Some((parse_inlines_in(&body, span, skips), next));
	}
	let text = link_dest_text(&dest);
	Some((vec![Inline::Text(text)], after_dest))
}

/// The display text of a bare `#link(dest)` with no bracketed body: a `"url"` string loses its quotes, a
/// `<label>` its angle brackets, and anything else stands as written.
fn link_dest_text(dest: &str) -> String {
	let t = dest.trim();
	if let Some(inner) = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
		return inner.to_string();
	}
	unwrap_arg(t)
}

/// Reads an inline `#name(...)`/`#name[...]` call at `i` (a `#`) that no earlier handler claimed, so the
/// reader can consume it whole rather than leak its raw markup. Returns the bracketed body's runs (parsed,
/// so its own markup survives) when the call is a single `[...]` content group, `None` for the body when it
/// carries only paren arguments, together with the index just past the call and its `#name` for the skip
/// report. The final `None` is returned when `i` does not open a `#name(`/`#name[` call at all, so a bare
/// `#` or a `#variable` interpolation is left as ordinary text.
fn unknown_call(chars: &[char], i: usize, span: Span, skips: &mut Refusals)
	-> Option<(Option<Vec<Inline>>, usize, String)>
{
	if chars.get(i) != Some(&'#') {
		return None;
	}
	let start	= i + 1;
	let mut j	= start;
	while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-' || chars[j] == '_' || chars[j] == '.') {
		j += 1;
	}
	if j == start {
		return None;
	}
	let name: String = chars[start..j].iter().collect();
	match chars.get(j) {
		// A `#name[body]`: the bracketed content is the call's displayable body.
		Some('[') => {
			let Some((body, next)) = read_group(chars, j) else { return None; };
			Some((Some(parse_inlines_in(&body, span, skips)), next, fmt!("#{}", name)))
		},
		// A `#name(args)` and any following `[body]`: read the arguments away, then fold a body if one trails.
		Some('(') => {
			let Some((_, after_args)) = read_group(chars, j) else { return None; };
			if chars.get(after_args) == Some(&'[') {
				let Some((body, next)) = read_group(chars, after_args) else { return None; };
				return Some((Some(parse_inlines_in(&body, span, skips)), next, fmt!("#{}", name)));
			}
			Some((None, after_args, fmt!("#{}", name)))
		},
		_ => None,
	}
}

/// The `#name` of a skipped line-leading code statement or standalone call, for the skip report: the
/// keyword itself for a block statement (`#let`, `#set`, `#show`, `#import`), or `#` and the identifier of
/// a standalone call. Falls back to the first whitespace-delimited token when neither shape reads, so the
/// tally always names something rather than nothing.
fn construct_name(trimmed: &str) -> String {
	for kw in ["#import", "#let", "#set", "#show"] {
		if trimmed.starts_with(kw) {
			return kw.to_string();
		}
	}
	let mut cs = trimmed.chars();
	if cs.next() == Some('#') {
		let mut ident = String::new();
		for c in cs {
			if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
				ident.push(c);
			} else {
				break;
			}
		}
		if !ident.is_empty() {
			return fmt!("#{}", ident);
		}
	}
	trimmed.split_whitespace().next().unwrap_or(trimmed).to_string()
}

/// If the literal `s` sits at `i` in `chars`, the index just past it; otherwise `None`.
fn at_lit(chars: &[char], i: usize, s: &str) -> Option<usize> {
	let mut k = i;
	for ch in s.chars() {
		if chars.get(k) != Some(&ch) {
			return None;
		}
		k += 1;
	}
	Some(k)
}

/// One open delimiter context on the scanner's stack. Which frame is on top decides whether the next
/// bracket is structural: a `(` inside a `[...]` content block is author prose, not nesting, and a `(`
/// or `$` inside a `"..."` string or a `$...$` maths span never counts at all. This is what lets a caption
/// whose prose carries an unbalanced `(` still close at its `]`, where a flat depth counter stuck open to
/// end of source and swallowed the figure and everything after it.
#[derive(Clone, Copy, PartialEq)]
enum Frame {
	Code,		// a `(...)`/`{...}`/`#name(...)` group, or the top level: brackets nest, `,`/`:` part
	Content,	// a `[...]` content block: only `[` `]` nest; author `(` `)` `{` `}` are literal prose
	Str,		// a `"..."` string literal: every character is literal until the closing quote
	Math,		// a `$...$` maths span: every character is literal until the closing `$`
	Comment,	// a `/* ... */` block comment: every character, brackets included, is literal until `*/`
	Raw,		// a `` `...` `` code span: every character, `//`/`/*` included, is literal until the closing backtick
}

/// The running delimiter balance while a bracketed span is scanned. The stack of [`Frame`]s replaces the
/// old flat `depth`: the span is closed when the stack is empty (was `depth <= 0`), and a multi-line code
/// skip is still open while it is not. `escaped` records that the previous character was a `\` inside a
/// string, maths span or content block, so a `\"`, `\$` or `\]` is passed over rather than closing its
/// frame. Both persist across the lines of a span, since a frame may straddle the line break.
pub(crate) struct SkipState {
	frames:		Vec<Frame>,
	escaped:	bool,
	in_quote:	bool,	// an odd number of literal `"` seen since the start of the current line, in Content mode
}

/// Does a `//` at `i` open a line comment, or is it a URL's double slash (`https://...`) and so literal?
/// Mirrors the `://` exception in [`strip_comments`]: a `/` immediately after a `:` never starts a
/// comment, in code or in content prose alike.
fn is_line_comment(chars: &[char], i: usize) -> bool {
	chars.get(i + 1) == Some(&'/') && !(i > 0 && chars[i - 1] == ':')
}

/// How many characters a `//` line comment opened at `i` consumes. `chars` may be a whole multi-line
/// capture buffer -- [`read_group`], [`split_top_args`] and [`named_arg`] all run on one -- so the comment
/// is bounded to the next `'\n'`, not to the end of the slice; a `//` on one line must never eat the lines
/// that follow it.
fn line_comment_len(chars: &[char], i: usize) -> usize {
	match chars[i..].iter().position(|&c| c == '\n') {
		Some(off)	=> off,
		None		=> chars.len() - i,
	}
}

impl SkipState {
	pub(crate) fn new() -> Self {
		SkipState { frames: Vec::new(), escaped: false, in_quote: false }
	}

	/// Is any frame still open? The top-level test for [`read_group`], [`split_top_args`] and [`named_arg`],
	/// where a comma or colon parts only when nothing at all is open and a group closes when the stack empties.
	fn is_open(&self) -> bool {
		!self.frames.is_empty()
	}

	/// Is a structural bracket -- a `(`/`{`/`[` group -- still unclosed? This is the multi-line skip and
	/// capture test, matching the old flat `depth > 0`: a dangling `"` or `$` left open at the end of a line
	/// does not keep a construct open, since in prose a stray quote (an author's `"no bound"` split across
	/// two lines after an inline `#raw("...")`) or a lone `$` is a character, not the start of a code span.
	pub(crate) fn has_open_bracket(&self) -> bool {
		self.frames.iter().any(|f| matches!(f, Frame::Code | Frame::Content))
	}

	/// How many structural `(`/`{`/`[` frames are nested right now -- the depth [`has_open_bracket`] only
	/// asks a yes/no of. A guard tracking its own single opening bracket uses this to tell its own matching
	/// closer (depth falls to 1) from an inner content block's closer (depth still above 1) on the same
	/// `]` text.
	pub(crate) fn open_brackets(&self) -> usize {
		self.frames.iter().filter(|f| matches!(f, Frame::Code | Frame::Content)).count()
	}

	/// Folds the character (or, in content mode, the `#ident` run) at `i` into the stack, returning how
	/// many characters were consumed from `chars` -- always at least one, more for a `#name(`/`#name[`/`#x`
	/// run whose opener decides the frame it enters. All four scanners share this one transition so a
	/// bracket is counted at exactly one place, whatever their outer loops do with the characters.
	fn step(&mut self, chars: &[char], i: usize) -> usize {
		let c = chars[i];
		match self.frames.last().copied() {
			Some(Frame::Str) => {
				if self.escaped			{ self.escaped = false; }
				else if c == '\\'		{ self.escaped = true; }
				else if c == '"'		{ self.frames.pop(); }
				1
			},
			Some(Frame::Math) => {
				if self.escaped			{ self.escaped = false; }
				else if c == '\\'		{ self.escaped = true; }
				else if c == '$'		{ self.frames.pop(); }
				1
			},
			// A `/* ... */` block comment: every character, including a stray `}`/`]`/`)` an author's note
			// mentions, is literal until the comment's own closer -- the twin of Str/Math above, so a
			// `#context` guard's brace balance is never corrupted by a comment inside its body.
			Some(Frame::Comment) => {
				if c == '*' && chars.get(i + 1) == Some(&'/')	{ self.frames.pop(); 2 }
				else											{ 1 }
			},
			// A `` `...` `` code span: literal until the closing backtick, the twin of Comment above, so a
			// `//`/`/*` a prose note quotes as a raw code token (`` the `//` operator ``) is never mistaken
			// for a comment opener. Mirrors [`strip_comments`]' `in_raw`, which does not persist an
			// unterminated span past its own line, so an unclosed backtick is dropped at the newline rather
			// than swallowing the lines that follow.
			Some(Frame::Raw) => {
				match c {
					'`'		=> { self.frames.pop(); 1 },
					'\n'	=> { self.frames.pop(); 1 },
					_		=> 1,
				}
			},
			Some(Frame::Content) => {
				// A `\`-escaped `\$ \[ \] \#` is literal content, so the escaped character is passed over
				// before any of the structural cases below can act on it.
				if self.escaped {
					self.escaped = false;
					return 1;
				}
				match c {
					'\\'	=> { self.escaped = true; 1 },
					'['		=> { self.frames.push(Frame::Content); 1 },
					']'		=> { self.frames.pop(); 1 },
					'$'		=> { self.frames.push(Frame::Math); 1 },
					'#'		=> self.content_hash(chars, i),
					'`'		=> { self.frames.push(Frame::Raw); 1 },
					// A literal `"` in prose is not a string (content mode never opens `Frame::Str`), but
					// `strip_comments` still treats a quoted phrase as opaque to `//`/`/*`, so a bare count
					// mirrors that here without disturbing the bracket balance a real quote would otherwise
					// leave alone. Line-scoped, as `strip_comments` is called once per line.
					'"'		=> { self.in_quote = !self.in_quote; 1 },
					'\n'	=> { self.in_quote = false; 1 },
					// A line comment runs to the next `\n` in `chars` (which may hold a whole multi-line
					// capture buffer, not just this one line) -- never past it, and never at all inside a
					// quoted phrase or a raw span. The `://` exception mirrors `strip_comments`, so a bare
					// URL's slashes stay literal prose. A block comment opens a `Comment` frame that can
					// straddle the line break, same as Str/Math above.
					'/' if is_line_comment(chars, i) && !self.in_quote		=> line_comment_len(chars, i),
					'/' if chars.get(i + 1) == Some(&'*') && !self.in_quote	=> { self.frames.push(Frame::Comment); 2 },
					// A `(` `)` `{` `}` in content mode is author prose, never nesting: this is the whole
					// point of tracking the frame, so a caption's unbalanced paren does not stick.
					_		=> 1,
				}
			},
			// A code frame, or the top level (an empty stack): brackets nest as the flat counter had them,
			// the closer kind is not checked, and a `[` opens a content child, a `$` a maths span. A `"`
			// opens a real `Str` frame here, which already keeps a `//`/`/*` inside it literal, so no
			// separate quote count is needed the way Content mode's prose-only quote does.
			_ => {
				match c {
					'"'									=> { self.frames.push(Frame::Str); 1 },
					'`'									=> { self.frames.push(Frame::Raw); 1 },
					'(' | '{'							=> { self.frames.push(Frame::Code); 1 },
					'['									=> { self.frames.push(Frame::Content); 1 },
					'$'									=> { self.frames.push(Frame::Math); 1 },
					')' | '}'							=> { self.frames.pop(); 1 },
					'/' if is_line_comment(chars, i)		=> line_comment_len(chars, i),
					'/' if chars.get(i + 1) == Some(&'*')	=> { self.frames.push(Frame::Comment); 2 },
					_									=> 1,
				}
			},
		}
	}

	/// Handles a `#` met in content mode: a `#name` identifier follows, and its first non-identifier
	/// character decides the frame -- `(` opens the call's code arguments, `[` a content block, anything
	/// else (or end of input) is a bare `#name` field access with no group. Returns the count consumed:
	/// the `#`, the identifier, and, for a call or content opener, that opener too.
	fn content_hash(&mut self, chars: &[char], i: usize) -> usize {
		let mut j = i + 1;
		while j < chars.len() && is_call_ident(chars[j]) {
			j += 1;
		}
		match chars.get(j) {
			Some('(')	=> { self.frames.push(Frame::Code); j + 1 - i },
			Some('[')	=> { self.frames.push(Frame::Content); j + 1 - i },
			_			=> j - i,	// a bare `#name` (or a lone `#`): open no frame
		}
	}
}

/// What to do with a line-leading Typst code statement or standalone template call.
enum CodeSkip {
	Line,				// the call closes on this line; skip the one line, as before
	Multi(SkipState),	// the delimiters are still open; begin a multi-line skip carrying the depth
}

/// If this already-left-trimmed line begins a Typst code statement Austenite skips for now, decides how
/// much to skip: `Line` for a statement or standalone call that closes on this line, `Multi` for one
/// whose delimiters are still open at the end of it. `None` when the line is not code the reader skips,
/// so the caller sets it as prose.
///
/// The four block statements (`#import`, `#let`, `#set`, `#show`) are always code; a line-leading call
/// (`#name(` or `#name[`) is skipped only as a whole -- either it closes on the line, or it opens a
/// multi-line span. A balanced `#name[...]` with prose trailing it (`#index-main[x]More prose...`) is
/// left to set, since its content is a marker within a real paragraph, not a standalone call.
fn code_skip(trimmed: &str) -> Option<CodeSkip> {
	let keyword	= code_keyword(trimmed);
	if !keyword && !opens_standalone_call(trimmed) {
		return None;
	}
	let mut state = SkipState::new();
	scan_brackets(trimmed, &mut state);
	if state.has_open_bracket() {
		return Some(CodeSkip::Multi(state));
	}
	// The delimiters balance on this line. A block statement is skipped whatever trails it; a standalone
	// call is skipped only when it truly ends with its own closer, so a marker inside a paragraph sets.
	// The `}` closer is the code-block call form (`#context{ ... }`) that closes on its own line.
	if keyword || trimmed.ends_with(')') || trimmed.ends_with(']') || trimmed.ends_with('}') {
		return Some(CodeSkip::Line);
	}
	None
}

/// Does this already-left-trimmed line open one of the Typst block statements the reader skips?
///
/// `#include` sits here too, guarding against the shape it would otherwise fall through to: it takes a
/// bare string argument, with neither a `(` nor a `[` for [`opens_standalone_call`] to catch, so with no
/// entry here it reads as an ordinary paragraph line and its raw `#include "path"` prints as literal body
/// text. The book assembler ([`crate::book::assemble`]) resolves and follows a real `#include` itself,
/// before a chapter's source ever reaches this parser -- this is the belt-and-braces net for any source
/// that bypasses that assembler (a bare `to_blocks`/`to_blocks_with_templates` call, a test fixture): the
/// line is skipped and recorded rather than ever standing a chance of being set as prose.
fn code_keyword(trimmed: &str) -> bool {
	for kw in ["#import ", "#import\"", "#let ", "#set ", "#show ", "#show:", "#include ", "#include\""] {
		if trimmed.starts_with(kw) {
			return true;
		}
	}
	false
}

/// Does this already-left-trimmed line open with a standalone call -- `#`, an identifier, then `(`, `[`
/// or `{`? A crude test, enough to recognise the opener of a call to an unrecognised template function
/// without inspecting where or whether it closes; the balance decides single- versus multi-line.
///
/// The `{` opener catches the code-block call form -- `#context { ... }`, the brace twin of
/// `#context[ ... ]` -- which is always Typst code at a line's head (a `{` in content mode is literal
/// prose, never a call opener). Typst attaches such a block to its keyword with optional whitespace
/// (`#context {`, the shape a book's reverse-reference index is written with, Lucronics ch29.8), so a run
/// of spaces before the `{` is skipped; a space before `(`, by contrast, breaks a Typst call, so only an
/// immediate `(` counts. No inline-call name is written with a `{`, so the `is_inline_call` guard, which
/// still holds off the glossary/index family, never fires on the brace form.
fn opens_standalone_call(trimmed: &str) -> bool {
	let mut cs = trimmed.chars();
	if cs.next() != Some('#') {
		return false;
	}
	let mut ident	= String::new();
	let mut after	= None;	// the first character past the identifier
	for c in cs {
		if c.is_alphanumeric() || c == '-' || c == '_' {
			ident.push(c);
			continue;
		}
		after = Some(c);
		break;
	}
	if ident.is_empty() || is_inline_call(&ident) {
		// A line-leading inline glossary or index call is content, not a skippable standalone call, even
		// when it closes on its own line, so [`parse_inlines`] sets its display text rather than dropping it.
		return false;
	}
	match after {
		Some('(') | Some('[') | Some('{')	=> true,
		// A `{` code block may follow the keyword across whitespace (`#context {`); a `(` may not, since a
		// space before it breaks a Typst call. The second whitespace-split word is the block opener.
		Some(c) if c.is_whitespace()		=>
			trimmed.split_whitespace().nth(1).map(|w| w.starts_with('{')).unwrap_or(false),
		_									=> false,
	}
}

/// Is this already-left-trimmed line a line-leading code-mode reference the reader cannot run and must
/// refuse rather than leak as prose? Recognises an anonymous `#{ ... }`/`#( ... )` block, an `#if`/`#for`/
/// `#while` control keyword, a field or method access `#name.foo`, and a bare `#name` (with only whitespace
/// after -- a `// comment` is stripped upstream). A `#name(`/`#name[` standalone call and the `#let`/`#set`/
/// `#show`/`#import` keywords are refused earlier by [`code_skip`], and a bound `#name` is expanded by
/// [`capture_opener`], so this catches exactly what is left -- above all a `#if`/`#{` surfacing in a re-read
/// content-binding body, which must be refused, not set with its leading `#`.
fn is_code_reference(trimmed: &str) -> bool {
	let rest = match trimmed.strip_prefix('#') {
		Some(r)	=> r,
		None	=> return false,
	};
	let chars: Vec<char> = rest.chars().collect();
	// An anonymous code block or expression.
	if matches!(chars.first(), Some('{') | Some('(')) {
		return true;
	}
	// A control keyword: `#if`/`#for`/`#while` followed by whitespace, `(` or `{`.
	for kw in ["if", "for", "while"] {
		if let Some(after) = rest.strip_prefix(kw) {
			match after.chars().next() {
				Some(c) if c.is_whitespace() || c == '(' || c == '{'	=> return true,
				_													=> {},
			}
		}
	}
	// A bare identifier, or a field/method access on one.
	let name_len = chars.iter().take_while(|&&c| c.is_alphanumeric() || c == '-' || c == '_').count();
	if name_len == 0 {
		return false;
	}
	match chars.get(name_len) {
		None			=> true,	// a bare `#name`
		Some('.')		=> true,	// a field or method access `#name.foo`
		// A bare `#name` with only whitespace trailing it (a comment was stripped upstream); a `#name` with
		// trailing prose is an inline reference the paragraph keeps, and a `#name(`/`#name[` is a call handled
		// elsewhere.
		Some(c) if c.is_whitespace()	=> chars[name_len..].iter().all(|c| c.is_whitespace()),
		_				=> false,
	}
}

/// Does this already-left-trimmed standalone line consist of a bare `#name` that names a scalar `#let`
/// binding in scope? Only a bare reference -- an identifier with nothing but whitespace after it -- with a
/// name `sfns` actually binds qualifies; a `#name.field` access, a `#name(`/`#name[` call and an unbound
/// name all return false. It exempts exactly the standalone scalar reference from the code-reference skip
/// so its value is substituted (through the paragraph's own [`substitute_scalars`]), while every other
/// standalone code reference, above all an unbound name, stays the visible refusal it is.
fn names_scalar_alone(trimmed: &str, sfns: &crate::lang::rules::ScalarFns) -> bool {
	let rest = match trimmed.strip_prefix('#') {
		Some(r)	=> r,
		None	=> return false,
	};
	let chars: Vec<char> = rest.chars().collect();
	let name_len = chars.iter().take_while(|&&c| c.is_alphanumeric() || c == '-' || c == '_').count();
	if name_len == 0 {
		return false;
	}
	// A bare `#name`: nothing but whitespace after the identifier (no `.field`, no `(args)`, no `[body]`).
	if !chars[name_len..].iter().all(|c| c.is_whitespace()) {
		return false;
	}
	let name: String = chars[..name_len].iter().collect();
	sfns.contains_key(&name)
}

/// Is this identifier one of the book template's inline functions the reader sets in place -- a glossary
/// or index call, a term-dictionary lookup, a hyperlink, a citation or an emphasis call? These emit body
/// text (or an invisible marker) mid-paragraph, so a line that opens with one is prose the inline scanner
/// reads, never a standalone call the line scanner skips.
fn is_inline_call(name: &str) -> bool {
	matches!(name,
		// The simple string-keyed glossary/index family, keyed on their own display text.
		"gs" | "gscap" | "gsi" | "gscapi" | "glossind" | "glossindcap"
		// The term-dictionary family, keyed on a `term-dict` entry: `g`/`gcap`/`gi`/`gcapi` set the value
		// with first-use styling, `t`/`tcap` set it plain, `graw` sets it in the mono face.
		| "g" | "gcap" | "gi" | "gcapi" | "t" | "tcap" | "graw"
		| "idx" | "idx-main" | "idx-as" | "idx-main-as" | "idx-nested"
		| "index" | "index-main" | "cite" | "link"
		| "emph" | "strong" | "super" | "sub"
		| "claim-label" | "claim-refs")
}

/// Folds one line's delimiters into the running [`SkipState`]. A bracket inside a `"..."` string, a `$...$`
/// maths span or a `[...]` content block is not counted as structural nesting; the frame stack decides.
/// The state carries into the next line, so a frame that straddles the break is tracked correctly.
pub(crate) fn scan_brackets(line: &str, state: &mut SkipState) {
	let chars: Vec<char> = line.chars().collect();
	let mut i = 0;
	while i < chars.len() {
		i += state.step(&chars, i);
	}
}

/// Splits a trailing `<label>` off a heading title: a `<name>` with no inner whitespace at the very end
/// labels the heading and is removed from its text. A title that merely contains angle brackets, or a
/// `< >` with a space inside, keeps them as ordinary characters.
fn split_label(title: &str) -> (String, Option<String>) {
	let t = title.trim_end();
	if let Some(inner) = t.strip_suffix('>') {
		if let Some(p) = inner.rfind('<') {
			let label = &inner[p + 1..];
			if !label.is_empty() && !label.contains(char::is_whitespace) {
				return (inner[..p].trim_end().to_string(), Some(label.to_string()));
			}
		}
	}
	(t.to_string(), None)
}

/// Reads an inline `#footnote[...]` at `i` (a `#`), returning the note's inline markup -- parsed so a
/// `*strong*` or `_emph_` in the note sets with its own face -- and the index just past the closing `]`.
/// `None` when the shape is not a footnote call or its bracket does not close, so anything else is left as
/// ordinary text.
fn footnote_call(chars: &[char], i: usize, span: Span, skips: &mut Refusals) -> Option<(Vec<Inline>, usize)> {
	let Some(open) = at_lit(chars, i, "#footnote") else { return None; };
	if chars.get(open) != Some(&'[') {
		return None;
	}
	let Some((inner, next)) = read_group(chars, open) else { return None; };
	Some((parse_inlines_in(&inner, span, skips), next))
}

/// Reads an inline `#emph[...]` at `i` (a `#`), returning its inner markup unreduced -- it is the call
/// form of `_..._` and the caller expands it the same way -- and the index just past the closing `]`.
/// `None` when the shape is not an emph call or its bracket does not close, so anything else is left as
/// ordinary text.
fn emph_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = at_lit(chars, i, "#emph")?;
	if chars.get(open) != Some(&'[') {
		return None;
	}
	read_group(chars, open)
}

/// Reads an inline `#strong[...]` or `#strong("...")` at `i` (a `#`), returning the text to set bold and
/// the index just past the closing bracket. The bracket form's content is markup, unreduced -- it is the
/// call form of `*...*` and the caller expands it the same way emph does. The paren form's argument is
/// only resolved when it is a plain `"..."` string; a bare identifier or any other expression is not a
/// text this reader can evaluate, so `None` is returned and the generic call handler records the refusal
/// instead of guessing. `None` also when the shape is not a strong call or its group does not close.
fn strong_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = at_lit(chars, i, "#strong")?;
	match chars.get(open) {
		Some('[')	=> read_group(chars, open),
		Some('(')	=> {
			let (inner, next) = read_group(chars, open)?;
			let t = inner.trim();
			if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
				Some((t[1..t.len() - 1].to_string(), next))
			} else {
				None
			}
		},
		_			=> None,
	}
}

/// Reads an inline `#super[...]` or `#super("...")` at `i` (a `#`), returning its content reduced to
/// display text by [`flatten_markup`] -- usually a short string or number -- and the index just past the
/// closing bracket. `None` when the shape is not a super call or its argument does not close.
fn super_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = at_lit(chars, i, "#super")?;
	match chars.get(open) {
		Some('[') | Some('(')	=> {},
		_						=> return None,
	}
	let (inner, next) = read_group(chars, open)?;
	Some((flatten_markup(&unwrap_arg(&inner)), next))
}

/// Reads an inline `#sub[...]` or `#sub("...")` at `i` (a `#`), returning its content reduced to display
/// text by [`flatten_markup`] -- usually a short string or number, as in `CO#sub[2]` -- and the index just
/// past the closing bracket. `None` when the shape is not a sub call or its argument does not close.
fn sub_call(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = at_lit(chars, i, "#sub")?;
	match chars.get(open) {
		Some('[') | Some('(')	=> {},
		_						=> return None,
	}
	let (inner, next) = read_group(chars, open)?;
	Some((flatten_markup(&unwrap_arg(&inner)), next))
}

/// Reads an inline `#cite(...)` at `i` (a `#`), returning the citation keys and the index past the
/// closing `)`. Every `<label>` token inside the parentheses is a key; a named argument such as
/// `form: "prose"` carries no label and is ignored. `None` when the shape is not a cite call or its
/// parentheses do not close, so anything else is left as ordinary text.
fn cite_call(chars: &[char], i: usize) -> Option<(Vec<String>, usize)> {
	let open = at_lit(chars, i, "#cite")?;
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let (inner, next) = read_group(chars, open)?;
	let keys = cite_keys(&inner);
	Some((keys, next))
}

/// Extracts the `<label>` citation keys from the inside of a `#cite(...)` call, in order. A `<` opens a
/// key and the next `>` closes it; anything outside a `<...>` pair (a named argument, a separating comma)
/// is skipped.
fn cite_keys(inner: &str) -> Vec<String> {
	let chars:	Vec<char>	= inner.chars().collect();
	let mut keys			= Vec::new();
	let mut i				= 0usize;
	while i < chars.len() {
		if chars[i] == '<' {
			if let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == '>') {
				let key: String = chars[i + 1..close].iter().collect();
				let key = key.trim().to_string();
				if !key.is_empty() {
					keys.push(key);
				}
				i = close + 1;
				continue;
			}
		}
		i += 1;
	}
	keys
}

/// Reads an inline `#claim-label(...)` or `#claim-refs(...)` at `i` (a `#`), returning the compressed
/// margin code it sets, the raw reference codes it registers for the reverse claim index, and the index
/// just past the closing `)`. `#claim-label(..codes)` places a compressed code string in the outside
/// margin, so its display is the codes joined with a run of three or more consecutive same-prefix codes
/// collapsed to a range (`B1 B2 B3 B4` -> `B1–4`), matching the book's `claims.typ` `_compress-codes`;
/// `#claim-refs(..codes)` sets no visible margin code (empty display). Both register each of their raw codes
/// for the reverse index, matching `claims.typ`, where a label and a bare reference both emit the
/// `<claim-ref>` metadata `collect-claim-refs()` queries -- so a code contributes to the page list whether it
/// was labelled or merely referenced. Neither sets anything in the body text column, so the surrounding prose
/// closes over the marker's place as Typst's own body flow does. The outer `None` is returned when the shape
/// is not a claim call or its parentheses do not close.
fn claim_call(chars: &[char], i: usize) -> Option<(String, Vec<String>, usize)> {
	let (open, is_label) = match at_lit(chars, i, "#claim-label") {
		Some(o)	=> (o, true),
		None	=> (at_lit(chars, i, "#claim-refs")?, false),
	};
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let (inner, next) = read_group(chars, open)?;
	let codes	= claim_codes(&inner);
	let display	= if is_label { compress_codes(&codes) } else { String::new() };
	Some((display, codes, next))
}

/// The claim codes named inside a `#claim-label`/`#claim-refs` argument list, in source order: each
/// `<name>` label's name and each `"string"` argument's text, a `<...>` or `"..."` wrapper stripped and
/// anything else taken as written -- `claims.typ`'s own `str(c)` fallback for a bare argument.
fn claim_codes(inner: &str) -> Vec<String> {
	let mut out = Vec::new();
	for arg in split_claim_args(inner) {
		let a = arg.trim();
		if a.is_empty() {
			continue;
		}
		let code = if let Some(rest) = a.strip_prefix('<') {
			rest.strip_suffix('>').unwrap_or(rest).trim().to_string()
		} else if a.starts_with('"') {
			unwrap_arg(a)
		} else {
			a.to_string()
		};
		if !code.is_empty() {
			out.push(code);
		}
	}
	out
}

/// Splits a claim argument list on its top-level commas, holding a `<...>`, `(...)` or `[...]` nesting and
/// a `"..."` string together so a comma inside one does not split an argument.
fn split_claim_args(inner: &str) -> Vec<String> {
	let mut out		= Vec::new();
	let mut cur		= String::new();
	let mut depth	= 0i32;
	let mut in_str	= false;
	for c in inner.chars() {
		match c {
			'"'								=> { in_str = !in_str; cur.push(c); },
			'<' | '(' | '[' if !in_str		=> { depth += 1; cur.push(c); },
			'>' | ')' | ']' if !in_str		=> { depth -= 1; cur.push(c); },
			',' if depth == 0 && !in_str	=> out.push(std::mem::take(&mut cur)),
			_								=> cur.push(c),
		}
	}
	if !cur.trim().is_empty() {
		out.push(cur);
	}
	out
}

/// The margin display for a set of claim codes, a port of `claims.typ`'s `_compress-codes`: two or fewer
/// codes join with a space unchanged; a run of three or more consecutive codes sharing a letter prefix and
/// ascending by one collapses to a `first–lastnum` range (an en dash, `B1 B2 B3 B4` -> `B1–4`), a run of
/// exactly two is left expanded, and a code that does not parse as letters-then-digits is passed through.
fn compress_codes(codes: &[String]) -> String {
	if codes.len() <= 2 {
		return codes.join(" ");
	}
	// (prefix, number) for each code that matches `^([A-Za-z]+)(\d+)$`, else `None` for one that does not.
	let parsed: Vec<Option<(String, u64)>> = codes.iter().map(|s| parse_code(s)).collect();
	let mut result:	Vec<String>	= Vec::new();
	let mut i					= 0usize;
	while i < codes.len() {
		let prefix = match &parsed[i] {
			Some((p, _))	=> p.clone(),
			None			=> { result.push(codes[i].clone()); i += 1; continue; },
		};
		// The longest run from `i` sharing the prefix and ascending by one, per the Typst helper.
		let mut run_end = i;
		while run_end + 1 < codes.len() {
			match (&parsed[run_end + 1], &parsed[run_end]) {
				(Some((np, nn)), Some((_, cn))) if *np == prefix && *nn == cn + 1	=> run_end += 1,
				_																=> break,
			}
		}
		if run_end > i + 1 {
			let last_num = match &parsed[run_end] { Some((_, n)) => *n, None => 0 };
			result.push(fmt!("{}\u{2013}{}", codes[i], last_num));
		} else if run_end > i {
			result.push(codes[i].clone());
			result.push(codes[run_end].clone());
		} else {
			result.push(codes[i].clone());
		}
		i = run_end + 1;
	}
	result.join(" ")
}

/// A claim code split into its letter prefix and its number, as `claims.typ`'s `^([A-Za-z]+)(\d+)$` match
/// does: `None` when the code is not one or more ASCII letters followed by one or more ASCII digits and
/// nothing else.
fn parse_code(s: &str) -> Option<(String, u64)> {
	let s		= s.trim();
	let split	= s.find(|c: char| c.is_ascii_digit())?;
	let (pre, num) = s.split_at(split);
	if pre.is_empty() || !pre.chars().all(|c| c.is_ascii_alphabetic()) {
		return None;
	}
	if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
		return None;
	}
	num.parse::<u64>().ok().map(|n| (pre.to_string(), n))
}

/// Reduces a run of markup to plain display text: the words a reader sees, with the emphasis, code,
/// glossary and index delimiters removed. A glossary term contributes its display, a visible index call
/// its text, a pure index marker nothing; `*strong*` and `_emph_` contribute their inner words. Inline
/// maths and cross-references, which have no plain form here, contribute nothing. Used where the engine
/// takes a plain string -- a footnote's note, a table cell, a figure caption -- and cannot yet carry the
/// runs themselves.
pub fn flatten_markup(text: &str) -> String {
	let mut out = String::new();
	for run in parse_inlines(text) {
		match run {
			Inline::Text(t)					=> out.push_str(&t),
			// A `*strong*` or `_emph_` inner run may still carry markup -- `*_word_*` nests emphasis in
			// strong -- so it is flattened again to strip the inner delimiters. Parsing does not nest, so
			// each pass removes one layer and the recursion terminates on plain text.
			Inline::Strong(t)				=> out.push_str(&flatten_markup(&t)),
			Inline::Emph(t)					=> out.push_str(&flatten_markup(&t)),
			Inline::BoldItalic(t)			=> out.push_str(&t),	// already the flat inner of a nested run
			Inline::Super(t)				=> out.push_str(&t),	// a flattened string cannot raise; keep its text
			Inline::Sub(t)					=> out.push_str(&t),	// a flattened string cannot drop; keep its text
			Inline::Code(t)					=> out.push_str(&t),
			Inline::Glossary { display, .. }	=> out.push_str(&display),
			Inline::PageRef(_)				=> {},	// a page number has no plain form before layout
			Inline::Math(_)					=> {},	// maths is dropped from a flattened string
			Inline::Footnote(_)				=> {},	// a nested footnote is not set within a flattened string
			Inline::Cite(_)					=> {},	// a citation has no plain form before the bibliography resolves it
			Inline::MarginNote { .. }		=> {},	// a margin code is not part of the flattened body text
			Inline::Index { .. }			=> {},	// an index marker sets no words in the body text
		}
	}
	out
}

/// The index term a call records, with a nested entry's child term where one was given.
struct IndexKey {
	term:		String,			// the sort key (markup flattened), e.g. "March, James"
	sub:		Option<String>,
	display:	String,			// the display markup the index page sets, e.g. "James March" or "_Browder v. Gayle_"
	main:		bool,			// a primary reference (`idx-main`/`index-main`/`idx-main-as`); its folio sets bold
}

/// What an inline glossary or index call sets into the running text. `index` carries the term the call
/// adds to the back-matter index, `None` for a glossary-only call (`g`/`gs`/`gscap`/`gcap`) that indexes
/// nothing.
enum Call {
	Glossary { term: String, display: String, index: Option<IndexKey> },	// a glossary term, keyed by `term` for first-use styling
	Visible { display: String, index: Option<IndexKey> },	// display text set plain, its markup parsed by the caller
	Invisible { index: Option<IndexKey> },	// a pure index marker: nothing is set but the term is recorded
}

/// Reads an inline glossary or index call at `i` (a `#`), returning what it sets and the index just past
/// it, or `None` when the `#name` is not one the reader knows or its argument brackets do not close.
///
/// The visible glossary functions set their bracket content as the term, capitalising the display for
/// the `-cap` variants; `idx`/`idx-main` set the content plain; `idx-as`/`idx-main-as` take a second
/// argument as the display and set that; `index`/`index-main`/`idx-nested` are pure markers and set
/// nothing. First use is keyed by the term as written, matching the template's own case-sensitive
/// `glossary-seen` set.
///
/// The term-dictionary family keys a `term-dict` entry rather than carrying its own display, and the
/// reader translates the key to that value at parse time (the loader installs the map from `terms.typ`
/// before parsing): `g`/`gi` set the value bold-italic on first use, `gcap`/`gcapi` capitalised, `t`/`tcap`
/// plain and `graw` plain (its mono face is not reproduced). A key with no `term-dict` entry falls back to
/// the key text and is recorded in `skips`, so an unknown key is visible on the terse skip line rather
/// than silently wrong -- the template panics on a miss, which the reader must not.
fn glossary_call(chars: &[char], i: usize, span: Span, skips: &mut Refusals) -> Option<(Call, usize)> {
	if chars.get(i) != Some(&'#') {
		return None;
	}
	let start	= i + 1;
	let mut j	= start;
	while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
		j += 1;
	}
	if j == start {
		return None;
	}
	let name: String = chars[start..j].iter().collect();
	if !is_inline_call(&name) {
		return None;
	}
	match chars.get(j) {
		Some('(') | Some('[')	=> {},
		_						=> return None,
	}
	let (a1, next1) = read_group(chars, j)?;

	// The two-argument display functions take the first argument as the index term and the second as the
	// visible text (`#idx-as("publicani")[tax farmers]` sets "tax farmers", indexes "publicani").
	if name == "idx-as" || name == "idx-main-as" {
		let (a2, next2) = read_group(chars, next1)?;
		// The first argument is the sort key ("March, James"), the second the display shown in the body and
		// set in the index ("James March"); the sort key is flattened so any markup in it does not misfile it.
		let display = unwrap_arg(&a2);
		let index = Some(IndexKey {
			term:		flatten_markup(&unwrap_arg(&a1)),
			sub:		None,
			display:	display.clone(),
			main:		name == "idx-main-as",	// `idx-main-as` marks a primary reference, its folio bold
		});
		return Some((Call::Visible { display, index }, next2));
	}
	// A nested index entry is a pure marker of `parent > child`; its second argument is the child term.
	if name == "idx-nested" {
		let (child, end) = match read_group(chars, next1) {
			Some((c, n2))	=> (Some(unwrap_arg(&c)), n2),
			None			=> (None, next1),
		};
		let parent = unwrap_arg(&a1);
		let index = Some(IndexKey { term: flatten_markup(&parent), sub: child, display: parent, main: false });
		return Some((Call::Invisible { index }, end));
	}

	let arg = unwrap_arg(&a1);
	// A glossary+index call indexes the term it displays; a glossary-only call indexes nothing. The `-i`
	// suffix families and the `glossind`/`glossindcap` and `idx`/`index` families are the indexing ones,
	// mirroring the template's `#index`/`#index-main` calls inside each. The display carries the value's own
	// markup (the index page sets it), and the sort key is that value flattened to plain text.
	let idx_of = |value: String| Some(IndexKey { term: flatten_markup(&value), sub: None, display: value, main: false });
	// A primary reference (`idx-main`/`index-main`): the same key, marked `main` so its folio sets bold.
	let idx_of_main = |value: String| Some(IndexKey { term: flatten_markup(&value), sub: None, display: value, main: true });
	let call = match name.as_str() {
		// The simple family keys its own display text (a `term-defs` entry), so no translation applies.
		"gs"									=> Call::Glossary { term: arg.clone(), display: arg, index: None },
		"gsi"									=> Call::Glossary { term: arg.clone(), display: arg.clone(), index: idx_of(arg) },
		"gscap"									=> Call::Glossary { term: arg.clone(), display: cap_first(&arg), index: None },
		"gscapi"								=> Call::Glossary { term: arg.clone(), display: cap_first(&arg), index: idx_of(arg) },
		// `glossind`/`glossindcap` auto-detect: a key that is in `term-dict` sets its value, otherwise the
		// key stands as its own display, matching the template's `if key in term-dict` branch. Both index the
		// displayed value (uncapitalised), as the template's `idx-term` default does.
		"glossind"								=> {
			let display = term_value(&arg).unwrap_or_else(|| arg.clone());
			Call::Glossary { term: arg.clone(), display: display.clone(), index: idx_of(display) }
		},
		"glossindcap"							=> {
			let display = term_value(&arg).unwrap_or_else(|| arg.clone());
			Call::Glossary { term: arg.clone(), display: cap_first(&display), index: idx_of(display) }
		},
		// The term-dictionary family translates the key to its value; first use is keyed by the key, as the
		// template keys `glossary-seen` by the key name rather than the value. The `-i` variants index the
		// translated value.
		"g"										=> Call::Glossary { term: arg.clone(), display: resolve_term(&arg, &name, span, skips), index: None },
		"gi"									=> {
			let display = resolve_term(&arg, &name, span, skips);
			Call::Glossary { term: arg.clone(), display: display.clone(), index: idx_of(display) }
		},
		"gcap"									=> Call::Glossary { term: arg.clone(), display: cap_first(&resolve_term(&arg, &name, span, skips)), index: None },
		"gcapi"									=> {
			let display = resolve_term(&arg, &name, span, skips);
			Call::Glossary { term: arg.clone(), display: cap_first(&display), index: idx_of(display) }
		},
		"t" | "graw"							=> Call::Visible { display: resolve_term(&arg, &name, span, skips), index: None },
		"tcap"									=> Call::Visible { display: cap_first(&resolve_term(&arg, &name, span, skips)), index: None },
		"idx"									=> Call::Visible { display: arg.clone(), index: idx_of(arg) },
		"idx-main"								=> Call::Visible { display: arg.clone(), index: idx_of_main(arg) },
		"index"									=> Call::Invisible { index: idx_of(arg) },
		"index-main"							=> Call::Invisible { index: idx_of_main(arg) },
		_									=> return None,
	};
	Some((call, next1))
}

/// Resolves a term-dictionary key to its display value, or -- when no map is installed or it holds no
/// such key -- falls back to the key text and records the miss in `skips` under the calling function's
/// name, so an unknown key shows on the terse skip line rather than rendering silently as the raw key.
fn resolve_term(key: &str, func: &str, span: Span, skips: &mut Refusals) -> String {
	match term_value(key) {
		Some(value)	=> value,
		None		=> {
			skips.record(&fmt!("#{} unknown term-dict key {:?}", func, key), span);
			key.to_string()
		},
	}
}

/// Reads a bracket or paren group whose opener sits at `i`, returning its inner content and the index
/// just past the matching closer. The [`SkipState`] frame stack decides what nests: strings, maths spans
/// and content blocks are respected, so a bracket inside a quoted argument, a `$...$` span or the prose of
/// a `[...]` caption does not close the group early -- a `(` an author left unbalanced in caption prose is
/// literal, and the group still closes at its own delimiter. `None` when the group never closes, so a
/// malformed call is left as ordinary text.
pub(crate) fn read_group(chars: &[char], i: usize) -> Option<(String, usize)> {
	let open = *chars.get(i)?;
	if open != '[' && open != '(' {
		return None;
	}
	let mut state	= SkipState::new();
	let mut inner	= String::new();
	// The opener pushes its frame (`[` a content block, `(` a code group) but is not part of the inner
	// content, so it is stepped over here and never appended.
	let mut j = i + state.step(chars, i);
	while j < chars.len() {
		let consumed = state.step(chars, j);
		if !state.is_open() {
			// This character closed the outer group -- the matching closer -- so the group ends just past
			// it, and the closer is dropped from the inner as the outer opener was.
			return Some((inner, j + consumed));
		}
		// Every other character is inner content, verbatim: a nested opener or closer, a string with its
		// quotes, a maths span, or a `#name(` run in content mode.
		for k in j..j + consumed {
			inner.push(chars[k]);
		}
		j += consumed;
	}
	None
}

/// Strips a `"..."` wrapper from a paren-string argument, so `#gs("surplus")` reads the same term as
/// `#gs[surplus]`. A bracket argument has no quotes to strip and is returned unchanged.
fn unwrap_arg(inner: &str) -> String {
	let t = inner.trim();
	if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
		return t[1..t.len() - 1].to_string();
	}
	inner.to_string()
}

/// Capitalises the first character of a term for the `-cap` glossary variants, leaving the rest as it
/// stands. The template capitalises the first grapheme cluster; the first `char` matches it for every
/// term in these books.
fn cap_first(s: &str) -> String {
	let mut cs = s.chars();
	match cs.next() {
		Some(c)	=> c.to_uppercase().collect::<String>() + cs.as_str(),
		None	=> String::new(),
	}
}

/// Whether a `/* ... */` block comment is open across the line break.
struct CommentState {
	in_block:	bool,
}

/// Removes Typst comments from one line: a `//` to the line's end, and any `/* ... */` span, which may
/// have opened on an earlier line ([`CommentState::in_block`] carries that across). A `//` or `/*`
/// inside a `"..."` string or a `` `code` `` span is not a comment and is kept, and a `//` immediately
/// after `:` is kept so a bare URL survives. Quotes and backticks are treated as span delimiters here,
/// which is what the reader's markup needs; a real Typst code line with string literals is skipped whole
/// by the caller, so stripping it never reaches the output.
fn strip_comments(line: &str, st: &mut CommentState) -> String {
	let chars:	Vec<char>	= line.chars().collect();
	let mut out				= String::new();
	let mut in_str			= false;
	let mut in_raw			= false;
	let mut prev			= '\0';
	let mut i				= 0usize;
	while i < chars.len() {
		let c = chars[i];
		if st.in_block {
			if c == '*' && chars.get(i + 1) == Some(&'/') {
				st.in_block = false;
				i += 2;
				prev = '\0';
				continue;
			}
			i += 1;
			continue;
		}
		if in_str {
			out.push(c);
			if c == '"' { in_str = false; }
			prev = c;
			i += 1;
			continue;
		}
		if in_raw {
			out.push(c);
			if c == '`' { in_raw = false; }
			prev = c;
			i += 1;
			continue;
		}
		if c == '"' {
			in_str = true;
			out.push(c);
			prev = c;
			i += 1;
			continue;
		}
		if c == '`' {
			in_raw = true;
			out.push(c);
			prev = c;
			i += 1;
			continue;
		}
		if c == '/' && chars.get(i + 1) == Some(&'/') {
			if prev == ':' {
				out.push(c);	// a `://` is part of a URL, not a comment
				prev = c;
				i += 1;
				continue;
			}
			break;	// a line comment: drop the rest of the line
		}
		if c == '/' && chars.get(i + 1) == Some(&'*') {
			st.in_block = true;
			i += 2;
			prev = '\0';
			continue;
		}
		out.push(c);
		prev = c;
		i += 1;
	}
	out
}

// -- Multi-line figure, table and data-array capture ----------------------------------------------

/// A multi-line construct gathered whole so it can be parsed. The buffer accumulates its lines; the
/// bracket state closes it when the delimiters balance; the kind decides how the buffer is dispatched.
struct Capture {
	kind:	CaptureKind,
	buf:	String,
	state:	SkipState,
	start:	u32,	// byte offset of the construct's opening line, for a `#columns` refusal's span
}

/// The backstop cap on content-binding expansion depth, for a pathological *non-cyclic* chain of distinct
/// bindings (a cycle is already caught precisely by the name stack, at its own length). Set well above any
/// honest nesting yet well below the level at which the reader's own frames overflow the wasm shadow stack --
/// a depth-64 cap alone was found to overflow, so it would trap rather than refuse, defeating its purpose.
const MAX_EXPANSION_DEPTH: usize = 32;

/// Which multi-line construct is being gathered.
enum CaptureKind {
	Figure,			// a `#figure(...)` call, possibly wrapping a table or an image
	Table,			// a bare `#table(...)` call
	Image,			// a line-leading `#padded-image(...)` or `#image(...)` set without a figure number
	SectionBanner,	// a line-leading `#section-banner("logo")`, a full-width grey bar carrying a section logo
	Let(String),	// a `#let name = (...)` data array bound to this name
	Columns,		// a `#columns(n)[ ... ]` wrapper: its body is set single-column
	StyledBox,		// a `#styled-box[ ... ]` callout: its body is set inside a filled, padded box
	DeclStyle,		// a `#show: <t>.with(...)` application or a lowerable `#set <target>(...)`; lowered onto the theme, not refused
	TemplateCall(String),	// a `#name(args)?[ ... ]` call to a bound `#let` furniture function, expanded into a box
	ContentCall(String),	// a `#name`, `#name(args)` or `#name[ ... ]` reference to a bound content binding, expanded into re-read markup spliced in
	Context,		// a line-leading `#context { ... }`/`#context[ ... ]`: gathered whole, then either the reverse claim index (its body calls `collect-claim-refs(`) or a refusal
	Builtin(BuiltinKind),	// a line-leading Typst markup builtin the reader now sets rather than skips (`#pagebreak`, `#lorem`, `#v`)
}

/// A line-leading Typst markup builtin the reader recognises and sets on the block path, rather than
/// tallying as a skipped construct. Each is dispatched by [`dispatch_capture`], which reads the call's
/// arguments from the gathered buffer -- so a builtin whose parentheses run across several lines is still
/// read whole.
enum BuiltinKind {
	PageBreak,	// `#pagebreak()` / `#pagebreak(weak: true)`: a forced page eject
	Lorem,		// `#lorem(<n>)`: n words of the standard lorem-ipsum placeholder, set as a paragraph
	Vspace,		// `#v(<abs len>)`: a fixed vertical space, absolute units only
}

/// Detects the opener of a multi-line construct the reader parses rather than skips: a `#figure(`, a
/// bare `#table(`, or a `#let name = (` data array. `None` for any other line, which the caller then
/// offers to [`code_skip`].
fn capture_opener(trimmed: &str, binds: crate::lang::rules::Bindings<'_, '_>) -> Option<CaptureKind> {
	// A call to a bound `#let` furniture function -- `#pr-note[ ... ]`, `#aside-box(title: [..])[ ... ]` --
	// is expanded rather than skipped. Recognised before the generic openers so a furniture name never
	// collides with one of them (none of the corpus names does), and only when the map holds it, so an
	// unbound `#name[...]` still falls through to be tallied as a skip exactly as before.
	if let Some(name) = template_call_name(trimmed, binds.tfns) {
		return Some(CaptureKind::TemplateCall(name));
	}
	// A reference to a bound content binding -- a bare `#intro`, a `#greet("world")`, a `#note[ ... ]` -- is
	// gathered whole and expanded into its re-read markup. Recognised only when the map holds the name and it
	// is not one of the inline-call family (a content binding named `idx`/`g` must not shadow the inline call
	// the scanner sets in place), so an unbound or inline reference still falls through unchanged.
	if let Some(name) = content_call_name(trimmed, binds.cfns) {
		return Some(CaptureKind::ContentCall(name));
	}
	// A line-leading `#context { ... }` (or the bracket twin `#context[ ... ]`): gathered whole so its body
	// can be inspected for the `collect-claim-refs(` signature that marks the reverse claim index, and
	// otherwise refused exactly as before. Recognised here, ahead of `code_skip`, so the block reaches
	// [`dispatch_capture`] rather than being skipped and lost -- the reader still evaluates no `#context`.
	if is_context_opener(trimmed) {
		return Some(CaptureKind::Context);
	}
	// A line-leading Typst markup builtin -- `#pagebreak()`, `#lorem(60)`, `#v(12pt)` -- the reader now sets
	// rather than skipping. Recognised here, ahead of `code_skip`, so the whole call reaches
	// [`dispatch_capture`] (which reads its arguments) rather than being tallied as an unsupported construct
	// and dropped. None of these names can be a bound furniture or content function -- they are reserved by
	// [`crate::lang::rules::is_reserved_construct`] -- so this never shadows a corpus binding.
	//
	// Own-line only, mirroring [`code_skip`]'s own rule: the call must either open a multi-line span (its
	// delimiters unbalanced on this line, gathered whole below) or close on this line with nothing but
	// whitespace after its `)`. A balanced call with trailing prose -- `#lorem(5) more`, `#v(12pt) text` --
	// is NOT own-line, so it falls through to the existing visible refusal, which keeps that trailing prose;
	// inline mid-prose support is a later unit.
	if let Some(kind) = builtin_opener(trimmed) {
		let mut state = SkipState::new();
		scan_brackets(trimmed, &mut state);
		if state.has_open_bracket() || trimmed.trim_end().ends_with(')') {
			return Some(CaptureKind::Builtin(kind));
		}
	}
	if trimmed.starts_with("#figure(") {
		return Some(CaptureKind::Figure);
	}
	if trimmed.starts_with("#table(") {
		return Some(CaptureKind::Table);
	}
	if trimmed.starts_with("#columns(") {
		return Some(CaptureKind::Columns);
	}
	// A `#styled-box[ ... ]` callout: a full-measure filled box wrapping running prose. Its body opens with
	// the `[` on this line and closes on a later one, so it is gathered whole and re-parsed rather than
	// skipped -- otherwise the bracket span reads as an unbalanced standalone call and its text is dropped.
	if trimmed.starts_with("#styled-box[") {
		return Some(CaptureKind::StyledBox);
	}
	// A documentation section opens with a line-leading `#section-banner("logo")` -- a full-width grey bar
	// carrying the section's logo -- captured here so the bar is drawn rather than the call dropped. Tried
	// before `#image(`, since the name contains none of the others as a prefix.
	if trimmed.starts_with("#section-banner(") {
		return Some(CaptureKind::SectionBanner);
	}
	// A section opener draws its logo with a line-leading `#padded-image(...)` (the Pearl section's pearlite
	// mark), and a bare `#image(...)` places a graphic likewise. Both are set centred without a figure
	// number, so they are captured here rather than skipped. The hyphen keeps `#image(` from matching the
	// tail of `#padded-image(`, which is tried first.
	if trimmed.starts_with("#padded-image(") || trimmed.starts_with("#image(") {
		return Some(CaptureKind::Image);
	}
	// A declarative styling construct the reader lowers onto the theme rather than refusing: a
	// `#show: <template>.with(...)` whole-document application, a top-level `#set` on an element the theme
	// carries a field for, or a per-element `#show <selector>: <transform>` rule the engine collects and
	// applies. Capturing the rule line here stops the reader tallying it as a skipped construct while the
	// rule engine separately reads and applies (or refuses) it -- otherwise an authored selector rule is
	// double-reported, skipped by the reader AND applied by the engine. A `#set rect(...)` names no theme
	// element and a `#show <selector>` whose selector no element answers to are not caught, so both fall
	// through to [`code_skip`], staying a visible refusal; an engine-refused rule (an introspective
	// `it => { ... }`) surfaces through the rule engine's own diagnostic, not the reader's skip tally.
	if is_show_doc_with(trimmed) || is_lowerable_set(trimmed) || crate::lang::rules::is_rule_line(trimmed) {
		return Some(CaptureKind::DeclStyle);
	}
	let_array_name(trimmed).map(CaptureKind::Let)
}

/// Does this already-left-trimmed line open a line-leading `#context` code block -- `#context {`,
/// `#context{` or `#context[` -- the shape the Logic appendix's reverse claim index is written with, and
/// the shape whose brace form once leaked verbatim as prose? The identifier must be exactly `context`
/// (`#contextual` does not match), and the delimiter must be a `{` (with any run of spaces before it, the
/// way Typst attaches a code block to its keyword) or an immediate `[`. Recognising it here routes the
/// whole block through [`dispatch_capture`], which either builds the index (its body calls
/// `collect-claim-refs(`) or refuses it, rather than skipping it as an opaque code span.
fn is_context_opener(trimmed: &str) -> bool {
	let rest = match trimmed.strip_prefix("#context") {
		Some(r)	=> r,
		None	=> return false,
	};
	// `#context[` -- the bracket twin -- attaches with no space; `#context {` -- the code block -- attaches
	// across any run of spaces, the way Typst binds a block to its keyword.
	rest.starts_with('[') || rest.trim_start().starts_with('{')
}

/// If this already-left-trimmed line opens a supported Typst markup builtin -- `#pagebreak(`, `#lorem(` or
/// `#v(` -- the builtin it names; else `None`. The name must be followed immediately by `(`, so `#voluptas(`
/// (a content-mode word that happens to start with `v`) does not match `#v`, and only a genuine call is
/// caught.
fn builtin_opener(trimmed: &str) -> Option<BuiltinKind> {
	let rest = trimmed.strip_prefix('#')?;
	for (name, kind) in [
		("pagebreak",	BuiltinKind::PageBreak),
		("lorem",		BuiltinKind::Lorem),
		("v",			BuiltinKind::Vspace),
	] {
		if let Some(after) = rest.strip_prefix(name) {
			if after.starts_with('(') {
				return Some(kind);
			}
		}
	}
	None
}

/// If this line opens a call to a bound furniture function -- `#<name>(` or `#<name>[` where `<name>` is a
/// key of `tfns` -- that name; else `None`. The name must be followed immediately by `(` (a keyword-argument
/// call) or `[` (a bare content call), so `#pr-note[` matches but a word that merely starts with a bound
/// name does not.
fn template_call_name(trimmed: &str, tfns: &crate::lang::rules::TemplateFns) -> Option<String> {
	let rest = trimmed.strip_prefix('#')?;
	let name_len = rest.chars().take_while(|&c| c.is_alphanumeric() || c == '-' || c == '_').count();
	if name_len == 0 {
		return None;
	}
	let name: String = rest.chars().take(name_len).collect();
	let next = rest.chars().nth(name_len);
	if (next == Some('(') || next == Some('[')) && tfns.contains_key(&name) {
		Some(name)
	} else {
		None
	}
}

/// If this line is a *standalone* reference to a bound content binding -- a bare `#name`, a `#name(args)`, a
/// `#name[body]` or a `#name(args)[body]` whose balanced group(s) are followed by nothing but whitespace --
/// where `<name>` is a key of `cfns` and not one of the inline-call family, that name; else `None`. The
/// only-whitespace-after rule is the guard against silent word loss: `#em[Note] the rest.` carries trailing
/// prose, so it is NOT a standalone call -- it falls through to the paragraph, where the trailing words
/// survive, rather than being expanded with the rest of the sentence discarded. The inline-call guard
/// mirrors [`opens_standalone_call`]'s: a binding named `idx`/`g` must never shadow the inline call.
fn content_call_name(trimmed: &str, cfns: &crate::lang::rules::ContentFns) -> Option<String> {
	let rest = trimmed.strip_prefix('#')?;
	let chars: Vec<char> = rest.chars().collect();
	let name_len = chars.iter().take_while(|&&c| c.is_alphanumeric() || c == '-' || c == '_').count();
	if name_len == 0 {
		return None;
	}
	let name: String = chars[..name_len].iter().collect();
	if is_inline_call(&name) || !cfns.contains_key(&name) {
		return None;
	}
	// Step past the call's balanced group(s): a `(args)` optionally followed by a `[body]`, or a lone
	// `[body]`. A bare `#name` opens neither.
	let mut j = name_len;
	if chars.get(j) == Some(&'(') {
		j = read_group(&chars, j)?.1;
	}
	if chars.get(j) == Some(&'[') {
		j = read_group(&chars, j)?.1;
	}
	// Only when nothing but whitespace trails the reference is it standalone; a trailing character makes it
	// an inline reference the paragraph keeps whole.
	if chars[j..].iter().all(|c| c.is_whitespace()) {
		Some(name)
	} else {
		None
	}
}

/// The `[ ... ]` body of a captured furniture call, and its keyword arguments if any. `#name[ body ]` has
/// no arguments (`args` empty); `#name(title: [..])[ body ]` carries the argument group before the body.
/// `None` when no `[ ... ]` content group follows the name, so a malformed call contributes no body.
fn template_call_parts(buf: &str, name: &str) -> Option<(String, String)> {
	let chars:	Vec<char>	= buf.chars().collect();
	let at		= find_lit(&chars, &fmt!("#{}", name))?;
	let mut j	= at + name.chars().count() + 1;	// past `#name`
	let mut args	= String::new();
	// An optional keyword-argument group `( ... )` immediately after the name.
	if chars.get(j) == Some(&'(') {
		let (inner, after) = read_group(&chars, j)?;
		args = inner;
		j = after;
	}
	// Whitespace between the argument group and the content body.
	while j < chars.len() && chars[j].is_whitespace() {
		j += 1;
	}
	if chars.get(j) != Some(&'[') {
		return None;
	}
	let (body, _) = read_group(&chars, j)?;
	Some((args, body))
}

/// Is this line a `#show: <ident>.with(` whole-document template application -- the form whose named
/// arguments lower onto the theme? Distinguished from an introspective `#show ...: it => { ... }`,
/// which carries no `.with(` and is left to be refused.
fn is_show_doc_with(trimmed: &str) -> bool {
	let rest = match trimmed.strip_prefix("#show:") {
		Some(r)	=> r.trim_start(),
		None	=> return false,
	};
	match rest.find(".with(") {
		Some(dot)	=> {
			let ident = &rest[..dot];
			!ident.is_empty() && ident.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
		},
		None		=> false,
	}
}

/// Is this line a lowerable top-level `#set <target>(` -- one of the elements the theme carries a field
/// for? The target list is [`crate::lang::set::LOWERABLE_SET_TARGETS`], the single source of truth the
/// lowering itself matches on, so the reader and the lowering never drift apart. A `#set` on any other
/// target returns `false` and is left to [`code_skip`] to refuse, since the reader has no field for it.
fn is_lowerable_set(trimmed: &str) -> bool {
	let rest = match trimmed.strip_prefix("#set ") {
		Some(r)	=> r.trim_start(),
		None	=> return false,
	};
	crate::lang::set::LOWERABLE_SET_TARGETS.iter().any(|target| {
		// The target must be followed immediately by `(`, so `par` does not match a `#set part(...)`.
		rest.strip_prefix(target).map_or(false, |after| after.starts_with('('))
	})
}

/// If the line is a `#let name = (` binding whose value opens a paren group, its name; else `None`. Only
/// an array or tuple value is captured -- a scalar or a function `#let` (whose name carries `(`) is left
/// to [`code_skip`].
fn let_array_name(trimmed: &str) -> Option<String> {
	let rest	= trimmed.strip_prefix("#let ")?;
	let eq		= rest.find('=')?;
	let name	= rest[..eq].trim();
	if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
		return None;
	}
	let value = rest[eq + 1..].trim_start();
	if value.starts_with('(') {
		Some(name.to_string())
	} else {
		None
	}
}

/// Dispatches a completed capture: a data array is evaluated and stored under its name; a table or a
/// figure is parsed into an [`Item`]. A construct that does not parse -- an unresolved spread, an empty
/// table -- yields no item rather than an error, so a stray call never fails the whole document.
fn dispatch_capture(
	cap:	Capture,
	items:	&mut Vec<Item>,
	arrays:	&mut HashMap<String, Vec<Vec<Inline>>>,
	skips:	&mut Refusals,
	binds:	crate::lang::rules::Bindings<'_, '_>,
)
{
	match cap.kind {
		CaptureKind::Let(name) => {
			arrays.insert(name, parse_let_array(&cap.buf));
		},
		CaptureKind::Table => {
			if let Some(inner) = call_inner(&cap.buf, "table") {
				if let Some(spec) = parse_table_spec(&inner, arrays, outer_text_size(&cap.buf)) {
					items.push(Item::Table { spec, span: Span::new(0, 0) });
				}
			}
		},
		CaptureKind::Figure => {
			if let Some(item) = parse_figure(&cap.buf, arrays) {
				items.push(item);
			}
		},
		CaptureKind::Image => {
			// A line-leading image call: its path and sizing are read the same way a figure's image body is,
			// then set as a plain centred image with no figure number. A call naming no path draws nothing.
			let (path, width, height, scale) = image_call(&cap.buf);
			if !path.is_empty() {
				items.push(Item::Image { path, width, height, scale, span: Span::new(0, 0) });
			}
		},
		CaptureKind::SectionBanner => {
			// The first positional argument is the logo path; a call naming none draws nothing.
			if let Some(path) = call_inner(&cap.buf, "section-banner").as_deref().and_then(first_string) {
				items.push(Item::SectionBanner { path, span: Span::new(0, 0) });
			}
		},
		CaptureKind::Columns => {
			// The reader has no column model: the `#columns(n)[ ... ]` wrapper is recorded as skipped and
			// its body set single-column, so the words survive even though the multi-column layout does not.
			// The body is a block sequence, so it is read through the document parser again and its items
			// spliced in; a nested refusal (a `#colbreak()`, an unknown call) folds into the same table.
			// The span is the wrapper's own opening line; a refusal recorded inside the re-parsed body
			// carries a span relative to that body text alone, not the enclosing document -- a known,
			// accepted imprecision for a wrapper nested this way (see `Refusal`'s own doc comment).
			skips.record("#columns", Span::new(cap.start, cap.start));
			if let Some(body) = columns_body(&cap.buf) {
				if let Ok((mut inner, sub)) = parse_items(&body, binds) {
					skips.merge(sub);
					// The columns body's own top-level `#set` declarations scope to the spliced subtree, the
					// way an included chapter's do (H1): its items splice in flat, so a scope marker pair
					// brackets them. An empty patch -- a body that declares no styling -- adds no markers.
					let patch = crate::lang::set::lower_declarations(&body);
					if patch == crate::theme::ThemePatch::default() {
						items.append(&mut inner);
					} else {
						items.push(Item::Scoped { patch, items: inner });
					}
				}
			}
		},
		CaptureKind::StyledBox => {
			// A `#styled-box[ ... ]` callout. Its body is a block sequence, so it is read through the document
			// parser again and wrapped in a single [`Item::Box`] the lowering sets in a filled, padded box --
			// unlike `#columns`, whose body splices in flat. The construct is set, not skipped, so it is not
			// recorded itself; a refusal within the body (an unknown inline call) still folds in.
			if let Some(body) = styled_box_body(&cap.buf) {
				if let Ok((mut inner, sub)) = parse_items(&body, binds) {
					skips.merge(sub);
					// A `#pagebreak()` nested in a callout body cannot be honoured -- the box is laid out as one
					// keep unit -- so it is refused visibly rather than dropped silently at render (see
					// [`refuse_nested_page_breaks`]).
					refuse_nested_page_breaks(&mut inner, skips);
					// The box body's own top-level `#set`/`#show: doc.with(...)` declarations lower to a patch
					// scoped to the box, applied to the box's subtree at render (H3) rather than the document.
					let patch = crate::lang::set::lower_declarations(&body);
					items.push(Item::Box { items: inner, patch, placement: None, span: Span::new(0, 0) });
				}
			}
		},
		CaptureKind::DeclStyle => {
			// A declarative styling construct -- a `#show: <template>.with(...)` application or a lowerable
			// top-level `#set`. The reader gathers it whole so it is neither leaked into the prose nor
			// blindly refused; lowering its arguments onto the theme is the book assembler's job (see
			// [`crate::lang::set`] and [`crate::book`]), which reads the same source with the theme in hand,
			// so nothing is emitted into the item stream here. A `#set` that would lower to nothing -- one
			// applying no argument, or naming an unrecognised or unconvertible one -- is recorded as a
			// refusal (H2), so it is visible rather than a silent no-op; a `#set` that fully lowers, and a
			// `#show: doc.with(...)`, record nothing.
			if let Some(name) = crate::lang::set::declstyle_refusal(&cap.buf) {
				skips.record(&name, Span::new(cap.start, cap.start));
			}
		},
		CaptureKind::Context => {
			// A gathered `#context { ... }` block. The reader evaluates no `#context` -- it is hard-stratified
			// and runs no `query` -- so the decision is by signature alone: a block whose body calls
			// `collect-claim-refs(` is the Logic appendix's reverse claim index, lowered to a `Block::ClaimIndex`
			// the author fills from the references gathered walking the body (like recognising `#print-glossary(`,
			// not like running it). Any other `#context` block is refused exactly as before, recorded once by
			// name so the section renders absent-but-reported rather than leaking its source as prose.
			if cap.buf.contains("collect-claim-refs(") {
				items.push(Item::ClaimIndex { span: Span::new(cap.start, cap.start) });
			} else {
				skips.record("#context", Span::new(cap.start, cap.start));
			}
		},
		CaptureKind::TemplateCall(name) => {
			// A call to a bound `#let` furniture function. Its `[ ... ]` body is a block sequence, re-parsed
			// through the document parser (with the same furniture in scope, so a nested call expands too) and
			// wrapped in a single `Item::Box` under the definition's lowered patch -- the callout geometry and
			// the inner-set overlay. A `title:` argument becomes a leading bold paragraph in the box. The call
			// is set, not skipped, so it is not tallied; a refusal inside the body still folds in.
			let tf = match binds.tfns.get(&name) {
				Some(tf)	=> tf,
				None		=> return,	// the opener only fires for a bound name, so this cannot happen
			};
			match template_call_parts(&cap.buf, &name) {
				Some((args, body)) => {
					if let Ok((mut inner, sub)) = parse_items(&body, binds) {
						skips.merge(sub);
						// A `#pagebreak()` nested in a furniture callout body cannot be honoured -- the box is one
						// keep unit -- so it is refused visibly rather than dropped silently at render.
						refuse_nested_page_breaks(&mut inner, skips);
						// A `title:` keyword argument, its content set as a leading bold paragraph. It is set at
						// the title size the definition named (`text(size: 0.85em)`) by nesting it in a scope, so a
						// title larger or smaller than the body reads at its own size.
						if tf.has_title {
							if let Some(title) = named_content_arg(&args, "title") {
								let runs	= parse_inlines(&title);
								let para	= Item::Paragraph {
									runs:	vec![Inline::Strong(inline_plain(&runs))],
									label:	None,
									span:	Span::new(cap.start, cap.start),
								};
								let title_item = match tf.title_size {
									Some(sz)	=> {
										let mut patch = crate::theme::ThemePatch::default();
										patch.text.body_size = Some(sz);
										Item::Scoped { patch, items: vec![para] }
									},
									None		=> para,
								};
								inner.insert(0, title_item);
							}
						}
						items.push(Item::Box { items: inner, patch: tf.patch.clone(), placement: tf.float, span: Span::new(cap.start, cap.start) });
					}
				},
				// A bound call with no `[ ... ]` body -- e.g. `#pr-note([x])`, an argument-only call this reader
				// cannot place -- is tallied as a skipped construct rather than dropped silently, so the report
				// still names it.
				None => skips.record(&fmt!("#{}", name), Span::new(cap.start, cap.start)),
			}
		},
		CaptureKind::ContentCall(name) => {
			// A reference to a bound content binding. Its positional arguments (none for a bare reference) are
			// read and substituted for each `#param` in the body, then the expanded markup is read through the
			// document parser again -- with the same bindings in scope, so a nested call expands too -- and its
			// blocks are spliced in flat. Unlike a furniture call, the body is arbitrary block markup, not a
			// wrap, so a heading in the binding becomes a real heading rather than a boxed paragraph. The call
			// is set, not skipped, so it is not tallied; a refusal inside the expanded body still folds in.
			let cf = match binds.cfns.get(&name) {
				Some(cf)	=> cf,
				None		=> return,	// the opener only fires for a bound name, so this cannot happen
			};
			// A self- or mutually-referential binding (`#let a = [#a]`, `#let a = [#b]`/`#let b = [#a]`, or a
			// function form `#let f(n) = [x #f(n)]`) would re-expand without bound. The name stack catches it
			// the instant a name recurs -- so a cycle unwinds at its own length, never deep enough to overflow
			// the native stack or the wasm shadow stack -- and the depth cap is the backstop for a pathological
			// non-cyclic chain of distinct bindings. Either way an unbounded re-read becomes a recorded refusal.
			if binds.expanding(&name) {
				skips.record(
					&fmt!("#{} (cycle: content binding refers back to itself)", name),
					Span::new(cap.start, cap.start));
				return;
			}
			if binds.depth() >= MAX_EXPANSION_DEPTH {
				skips.record(
					&fmt!("#{} (cycle: expansion depth exceeds {})", name, MAX_EXPANSION_DEPTH),
					Span::new(cap.start, cap.start));
				return;
			}
			let args		= content_call_args(&cap.buf, &name);
			let expanded	= expand_content_body(cf, &args);
			let mut nested: Vec<String> = binds.active.to_vec();
			nested.push(name.clone());
			if let Ok((mut inner, sub)) = parse_items(&expanded, binds.with_active(&nested)) {
				skips.merge(sub);
				items.append(&mut inner);
			}
		},
		CaptureKind::Builtin(kind) => {
			let span = Span::new(cap.start, cap.start);
			match kind {
				// `#pagebreak()` / `#pagebreak(weak: true)`: a forced eject. The default (`weak: false`) is a
				// STRONG break, which always opens a fresh page -- even a trailing one, and even when the current
				// page is already empty; `weak: true` ejects only a page that carries content, matching Typst
				// 0.15.1 (the strong/weak flag is honoured in the driver's compose). A `to:` argument
				// (`pagebreak(to: "odd")`) selects a parity target the reader does not model, so it is refused
				// visibly rather than set as a plain break that quietly ignores the argument.
				BuiltinKind::PageBreak => {
					let inner = call_inner(&cap.buf, "pagebreak").unwrap_or_default();
					if inner.contains("to:") {
						skips.record("#pagebreak", span);
					} else {
						items.push(Item::PageBreak { weak: pagebreak_is_weak(&inner), span });
					}
				},
				// `#lorem(<n>)`: n words of the standard placeholder, set as one plain paragraph. A malformed
				// count stays a visible refusal; a zero count sets nothing; a huge count is capped at the
				// embedded corpus by [`lorem_words`], so generation stays bounded.
				BuiltinKind::Lorem => {
					if let Some(n) = lorem_arg(&cap.buf) {
						let text = lorem_words(n);
						if !text.is_empty() {
							items.push(Item::Paragraph { runs: vec![Inline::Text(text)], label: None, span });
						}
					} else {
						skips.record("#lorem", span);	// a non-numeric argument stays a visible refusal
					}
				},
				// `#v(<abs len>)`: a fixed vertical space. Only an absolute first argument (pt/mm/cm/in) is set;
				// an `em`, `%` or `fr` length has no running size here, and a `weak:` argument asks for a
				// collapsing space the reader does not model -- either is refused visibly rather than set as the
				// wrong space, exactly as the heading-template spacer does (see [`crate::lang::rules`]).
				BuiltinKind::Vspace => {
					let inner = call_inner(&cap.buf, "v").unwrap_or_default();
					match parse_length(first_arg(&inner).trim()) {
						Some(Length::Abs(pt)) if !inner.contains("weak:")	=>
							items.push(Item::Space { height: crate::ir::Sp::from_pt(pt), span }),
						_													=> skips.record("#v", span),
					}
				},
			}
		},
	}
}

/// The standard lorem-ipsum passage Typst's `#lorem` draws from, the opening of Cicero's *De Finibus* as
/// the `lipsum` crate carries it, word for word as `typst 0.15.1` renders it. Held to the first 120 words:
/// every one is plain Latin with only commas and full stops, so a placeholder of any realistic length sets
/// byte-identically to the oracle, while the later passage's quotation marks, question marks and en dash --
/// which would need their own glyph handling to match -- are left out. `#lorem(n)` for n beyond this is
/// capped here, so a huge count generates a bounded, sane amount rather than looping.
const LOREM_CORPUS: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magnam aliquam quaerat voluptatem. Ut enim aeque doleamus animo, cum corpore dolemus, fieri tamen permagna accessio potest, si aliquod aeternum et infinitum impendere malum nobis opinemur. Quod idem licet transferre in voluptatem, ut postea variari voluptas distinguique possit, augeri amplificarique non possit. At etiam Athenis, ut e patre audiebam facete et urbane Stoicos irridente, statua est in quo a nobis philosophia defensa et collaudata est, cum id, quod maxime placeat, facere possimus, omnis voluptas assumenda est, omnis dolor repellendus. Temporibus autem quibusdam et aut officiis debitis aut rerum necessitatibus saepe eveniet, ut et voluptates repudiandae sint et molestiae non recusandae. Itaque earum rerum";

/// The first `n` words of [`LOREM_CORPUS`], joined by single spaces, with the final word's trailing
/// punctuation replaced by a full stop -- the shape Typst's `#lorem(n)` produces. `n` is capped at the
/// corpus length, so a very large count returns the whole embedded passage rather than looping unboundedly.
/// An `n` of zero yields the empty string, so the caller sets no paragraph.
fn lorem_words(n: usize) -> String {
	let words: Vec<&str> = LOREM_CORPUS.split_whitespace().collect();
	let take = n.min(words.len());
	if take == 0 {
		return String::new();
	}
	let mut out = words[..take].join(" ");
	// Typst ends the blind text with a full stop, dropping any comma, colon or semicolon the last word
	// carried in the source.
	let trimmed = out.trim_end_matches([',', ';', ':', '.']);
	out.truncate(trimmed.len());
	out.push('.');
	out
}

/// The word count of a `#lorem(<n>)` call read from the gathered buffer: the first positional argument
/// parsed as a non-negative integer. `None` when the argument is absent or not a number, so the caller
/// records a refusal rather than guessing.
fn lorem_arg(buf: &str) -> Option<usize> {
	let inner = call_inner(buf, "lorem")?;
	first_arg(&inner).trim().parse::<usize>().ok()
}

/// Does a `#pagebreak(...)` argument list ask for a WEAK break? Only an explicit `weak: true` does; a
/// `weak: false` and an absent argument are both the STRONG default (Typst 0.15.1), which always ejects.
/// The value is read as the token immediately after `weak:`, so `weak: true`, `weak:true` and
/// `weak: false` all resolve correctly.
fn pagebreak_is_weak(inner: &str) -> bool {
	match inner.find("weak:") {
		Some(at)	=> inner[at + "weak:".len()..].trim_start().starts_with("true"),
		None		=> false,
	}
}

/// The first positional argument of a call's inner argument text: the run up to the first top-level comma,
/// so `#v(12pt, weak: true)` yields `12pt` and `#lorem(60)` yields `60`.
fn first_arg(inner: &str) -> String {
	split_arg_commas(inner).into_iter().next().unwrap_or_default()
}

/// Records a visible refusal for, and removes, every `#pagebreak()` nested in a callout box's body. A
/// forced page eject has no meaning inside a box the layout keeps whole -- the box-body renderer has no
/// page to turn -- so it is refused rather than silently dropped at render. Recurses through a nested scope
/// or a nested box, so a break buried in either is caught too. The document top level and a `#columns` body
/// (which splices into the main flow, not a box) are untouched: a break there is honoured.
fn refuse_nested_page_breaks(items: &mut Vec<Item>, skips: &mut Refusals) {
	let mut kept = Vec::with_capacity(items.len());
	for mut item in items.drain(..) {
		match &mut item {
			Item::PageBreak { span, .. }	=> { skips.record("#pagebreak", *span); continue; },
			Item::Box { items: inner, .. }	=> refuse_nested_page_breaks(inner, skips),
			Item::Scoped { items: inner, .. }	=> refuse_nested_page_breaks(inner, skips),
			_								=> {},
		}
		kept.push(item);
	}
	*items = kept;
}

/// The positional arguments of a captured content-binding reference, each evaluated to its substitution
/// text: a `"quoted string"` yields its contents, a `[bracketed content]` its inner markup, and any other
/// value (a number, an identifier) its trimmed source. A bare `#name` reference, or a `#name[ ... ]` whose
/// single argument is the bracket body, is handled too. An empty list when the reference takes none.
fn content_call_args(buf: &str, name: &str) -> Vec<String> {
	let chars:	Vec<char>	= buf.chars().collect();
	let at = match find_lit(&chars, &fmt!("#{}", name)) {
		Some(a)	=> a,
		None	=> return Vec::new(),
	};
	let j = at + name.chars().count() + 1;	// past `#name`
	match chars.get(j) {
		Some('(') => {
			match read_group(&chars, j) {
				Some((inner, _))	=> split_arg_commas(&inner).into_iter()
										.map(|a| content_arg_value(a.trim()))
										.collect(),
				None				=> Vec::new(),
			}
		},
		// A `#name[ ... ]` call: the bracket body is the single positional argument.
		Some('[') => match read_group(&chars, j) {
			Some((inner, _))	=> vec![inner],
			None				=> Vec::new(),
		},
		_ => Vec::new(),	// a bare `#name` reference
	}
}

/// Evaluates one content-binding argument to its substitution text: a `"..."` string is unquoted, a
/// `[ ... ]` content block is unwrapped to its inner markup, and any other value is kept as its trimmed
/// source.
fn content_arg_value(arg: &str) -> String {
	let t = arg.trim();
	if t.starts_with('[') {
		let chars: Vec<char> = t.chars().collect();
		if let Some((inner, _)) = read_group(&chars, 0) {
			return inner;
		}
	}
	unwrap_arg(t)
}

/// Splits an argument list on its top-level commas, honouring `(`/`[`/`{` nesting and `"..."` strings so a
/// comma inside a bracketed or quoted argument does not split it.
fn split_arg_commas(s: &str) -> Vec<String> {
	let mut out		= Vec::new();
	let mut depth	= 0i32;
	let mut in_str	= false;
	let mut esc		= false;
	let mut cur		= String::new();
	for c in s.chars() {
		if in_str {
			cur.push(c);
			if esc				{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			continue;
		}
		match c {
			'"'					=> { in_str = true; cur.push(c); },
			'(' | '[' | '{'		=> { depth += 1; cur.push(c); },
			')' | ']' | '}'		=> { depth -= 1; cur.push(c); },
			',' if depth == 0	=> out.push(std::mem::take(&mut cur)),
			_					=> cur.push(c),
		}
	}
	if !cur.trim().is_empty() {
		out.push(cur);
	}
	out
}

/// Substitutes a content binding's positional arguments into its body: each line-leading or inline `#param`
/// token whose identifier names a parameter is replaced by the matching argument's text (a parameter with no
/// argument supplied substitutes empty). A `#` that opens no parameter name -- an inline call, an escaped
/// literal -- is left untouched, so the body's own markup survives the substitution. A binding with no
/// parameters returns its body verbatim.
///
/// A `#name(...)` call immediately following a non-parameter identifier -- `#t(w)`, `#g(w)` -- opens Typst's
/// own code mode inside its parentheses, where `w` is a bare variable reference rather than a markup-mode
/// `#w`. [`substitute_call_args`] re-quotes any such bare parameter reference found inside that argument
/// list, so the nested call -- most often a term-dictionary lookup -- resolves against the substituted value
/// rather than the parameter's own name. Substitution runs here, before the expanded body is re-parsed, so
/// term-dictionary (or any other) resolution downstream always sees the caller's argument, never the
/// parameter placeholder: the root fix is ordering, not a term-dictionary-specific patch.
fn expand_content_body(cf: &crate::lang::rules::ContentFn, args: &[String]) -> String {
	if cf.params.is_empty() {
		return cf.body.clone();
	}
	let chars:	Vec<char>	= cf.body.chars().collect();
	let mut out	= String::new();
	let mut i	= 0usize;
	while i < chars.len() {
		if chars[i] == '#' {
			let mut j = i + 1;
			while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
				j += 1;
			}
			let ident: String = chars[i + 1..j].iter().collect();
			if let Some(pos) = cf.params.iter().position(|p| *p == ident) {
				out.push_str(args.get(pos).map(|s| s.as_str()).unwrap_or(""));
				i = j;
				continue;
			}
			// Not a bare parameter reference; if it names a call (`#t(`, `#g(`, `#anything(`), its argument
			// list is code mode, where a parameter appears with no leading `#`. Substitute there too, then
			// resume scanning after the call's closing paren -- the call name and everything past it are
			// otherwise untouched.
			if !ident.is_empty() && chars.get(j) == Some(&'(') {
				if let Some((inner, after)) = read_group(&chars, j) {
					out.push('#');
					out.push_str(&ident);
					out.push('(');
					out.push_str(&substitute_call_args(&inner, &cf.params, args));
					out.push(')');
					i = after;
					continue;
				}
			}
		}
		out.push(chars[i]);
		i += 1;
	}
	out
}

/// Substitutes a bare parameter identifier found inside a call's argument list -- Typst code mode, where a
/// parameter carries no leading `#` of its own -- with a quoted literal of the caller's argument text, so a
/// nested call such as `#t(w)` sees the substituted value rather than the parameter's own name. A quote or
/// backslash in the substituted text is escaped, so the result is always one valid string literal. Left alone
/// inside an existing `"..."` string, so a quoted argument that merely contains the parameter's name as a
/// word is not mistaken for a reference to it. An identifier that is not a parameter -- the call's own other
/// arguments, a keyword name, a literal -- passes through unchanged.
fn substitute_call_args(inner: &str, params: &[String], args: &[String]) -> String {
	let chars:	Vec<char>	= inner.chars().collect();
	let mut out		= String::new();
	let mut i		= 0usize;
	let mut in_str	= false;
	while i < chars.len() {
		let c = chars[i];
		if in_str {
			out.push(c);
			if c == '\\' && i + 1 < chars.len() {
				out.push(chars[i + 1]);
				i += 2;
				continue;
			}
			if c == '"' {
				in_str = false;
			}
			i += 1;
			continue;
		}
		if c == '"' {
			in_str = true;
			out.push(c);
			i += 1;
			continue;
		}
		if c.is_alphabetic() || c == '_' {
			let start	= i;
			let mut j	= i + 1;
			while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
				j += 1;
			}
			let word: String = chars[start..j].iter().collect();
			match params.iter().position(|p| *p == word) {
				Some(pos)	=> {
					let value = args.get(pos).map(|s| s.as_str()).unwrap_or("");
					out.push('"');
					out.push_str(&value.replace('\\', "\\\\").replace('"', "\\\""));
					out.push('"');
				},
				None		=> out.push_str(&word),
			}
			i = j;
			continue;
		}
		out.push(c);
		i += 1;
	}
	out
}

/// Expands every inline mid-prose reference to a bound content binding within a run of already-joined
/// paragraph (or heading) text, splicing each call's argument-substituted, re-expanded body into the
/// surrounding prose at the position it stood -- the inline twin of the own-line [`CaptureKind::ContentCall`]
/// path (which splices a binding referenced alone on its line as blocks). A `M#oxe`, a
/// `see #stamp("v2") for details` or a `#note[x]` mid-sentence therefore sets its expansion with the words
/// before AND after it kept, rather than leaking its raw `#name` onto the page or refusing the call and
/// losing it.
///
/// Runs as a textual pre-pass in [`flush_para`] (and on a heading title), BEFORE [`substitute_scalars`] and
/// [`parse_inlines_in`] -- the same shape the scalar pass takes, and for the same reason: a content binding's
/// value is markup that must be re-read as the surrounding prose's own, so expanding it here lets the one
/// inline scanner downstream read the spliced result. A glossary term, an emphasis or a maths span in the
/// body sets exactly as if it had been typed in place, and an unrenderable primitive in the body (a `#h`, a
/// `#box`) reaches that scanner's own visible-refusal path rather than a parallel one here -- so the root fix
/// reuses [`expand_content_body`] and the block path's cycle rule rather than building a second expander.
///
/// `active` (carried on `binds`, seeded from an enclosing block-level expansion) is the stack of binding
/// names currently expanding: a reference to a name already on it is refused as a cycle, exactly as the block
/// path refuses one, so a self- or mutually-referential body unwinds at its own length rather than looping;
/// [`MAX_EXPANSION_DEPTH`] is the backstop for a pathological non-cyclic chain. A `#name` that names no
/// content binding, that is one of the inline-call family (so a binding named `g`/`idx` never shadows the
/// inline call the scanner sets in place), that is a field/method access (`#name.foo`), or that sits in a raw
/// `` `...` `` span, a `$...$` maths span or behind a `\`-escape is left untouched, so the downstream scanner
/// still sets or refuses it exactly as before. A binding referenced own-line is handled earlier by
/// [`capture_opener`], so this only ever sees a genuinely mid-prose reference.
pub(crate) fn substitute_content_calls(
	text:	&str,
	binds:	crate::lang::rules::Bindings<'_, '_>,
	skips:	&mut Refusals,
	span:	Span,
)
	-> String
{
	// The common case, no content bindings in scope: costs nothing beyond the check, so a document that uses
	// none reads exactly as before.
	if binds.cfns.is_empty() {
		return text.to_string();
	}
	let chars:	Vec<char>	= text.chars().collect();
	let mut out		= String::new();
	let mut i		= 0usize;
	let mut in_raw	= false;
	let mut in_math	= false;
	while i < chars.len() {
		let c = chars[i];
		// An escaped `\#` (or any `\`-escape) is literal: the backslash and its character are passed straight
		// through, so the downstream scanner still turns `\#` into a literal `#`.
		if c == '\\' && i + 1 < chars.len() {
			out.push(c);
			out.push(chars[i + 1]);
			i += 2;
			continue;
		}
		if c == '`' {
			in_raw = !in_raw;
			out.push(c);
			i += 1;
			continue;
		}
		if c == '$' && !in_raw {
			in_math = !in_math;
			out.push(c);
			i += 1;
			continue;
		}
		if c == '#' && !in_raw && !in_math {
			let start	= i + 1;
			let mut j	= start;
			while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
				j += 1;
			}
			if j > start {
				let name: String = chars[start..j].iter().collect();
				// A field or method access on the name -- `#name.foo` -- is a code-mode expression, not a bare
				// content reference: leave it for the downstream refusal path rather than expanding the name and
				// stranding the `.foo`. Only a `.` FOLLOWED BY an identifier is an access, though: a `.` before
				// whitespace, end of input or punctuation is a sentence's full stop, which Typst sets literally
				// after expanding the reference (`M#oxe.` sets `MX.`), so it must not block the expansion.
				let field_access = chars.get(j) == Some(&'.')
					&& chars.get(j + 1).map_or(false, |c| c.is_alphabetic() || *c == '_');
				if !field_access && !is_inline_call(&name) {
					if let Some(cf) = binds.cfns.get(&name) {
						// Step over the call's balanced group(s) -- a `(args)` optionally followed by a `[body]`,
						// or a lone `[body]` -- reading its positional arguments the same way [`content_call_args`]
						// reads a captured own-line call's, so both paths substitute identically.
						let mut k		= j;
						let mut args:	Vec<String>	= Vec::new();
						if chars.get(k) == Some(&'(') {
							if let Some((inner, after)) = read_group(&chars, k) {
								args = split_arg_commas(&inner).into_iter()
									.map(|a| content_arg_value(a.trim()))
									.collect();
								k = after;
							}
						}
						if chars.get(k) == Some(&'[') {
							if let Some((inner, after)) = read_group(&chars, k) {
								// A `#name[ ... ]` call with no paren group: the bracket body is the single
								// positional argument, mirroring [`content_call_args`]'s own bracket arm.
								if args.is_empty() {
									args = vec![inner];
								}
								k = after;
							}
						}
						// A self- or mutually-referential binding is refused the instant its name recurs, so a
						// cycle unwinds at its own length; the depth cap is the backstop for a pathological chain
						// of distinct bindings. Either way the call is consumed (the surrounding prose is kept)
						// and a visible refusal recorded, exactly as the own-line path does.
						if binds.expanding(&name) {
							skips.record(
								&fmt!("#{} (cycle: content binding refers back to itself)", name), span);
							i = k;
							continue;
						}
						if binds.depth() >= MAX_EXPANSION_DEPTH {
							skips.record(
								&fmt!("#{} (cycle: expansion depth exceeds {})", name, MAX_EXPANSION_DEPTH), span);
							i = k;
							continue;
						}
						let expanded		= expand_content_body(cf, &args);
						let mut nested:	Vec<String>	= binds.active.to_vec();
						nested.push(name.clone());
						// The expanded body may itself reference another binding inline, so it is expanded in
						// turn with this name pushed onto the active stack -- the recursion the cycle guard bounds.
						out.push_str(&substitute_content_calls(&expanded, binds.with_active(&nested), skips, span));
						i = k;
						continue;
					}
				}
			}
		}
		out.push(c);
		i += 1;
	}
	out
}

/// Substitutes a bare `#name` reference to a scalar `#let` value binding -- `#let title = "Foo"`, then a
/// later `#title` -- with its display text: a string's contents, or a number/length literal's own source
/// text (see [`crate::lang::rules::ScalarValue::display_text`]). Runs once, on a heading's title or a
/// flushed paragraph's joined text, BEFORE [`parse_inlines_in`] reads it, rather than as a case inside that
/// scanner: a scalar's value is plain text needing no further markup expansion, so a textual pass here is
/// the whole fix, and it leaves every other construct -- a table, a figure, a caption, a `#context` block,
/// a data array -- untouched, since none of those reach this function.
///
/// A `#name` inside a raw `` `...` `` code span or a `$...$` maths span is left alone, matching
/// [`parse_inlines_in`]'s own treatment of those as literal/foreign territory; an escaped `\#name` is left
/// alone too, so the backslash still reaches the inline scanner to produce a literal `#`. A name with no
/// scalar binding, or one immediately followed by `(` or `[` (a call, not a bare reference), is untouched --
/// it still reaches the inline scanner's own refusal path, so an unbound or non-scalar reference stays a
/// visible skip rather than becoming a silent drop here.
pub(crate) fn substitute_scalars(text: &str, sfns: &crate::lang::rules::ScalarFns) -> String {
	if sfns.is_empty() {
		return text.to_string();	// the common case, no scalar bindings in scope -- costs nothing beyond the check
	}
	let chars:	Vec<char>	= text.chars().collect();
	let mut out		= String::new();
	let mut i		= 0usize;
	let mut in_raw	= false;
	let mut in_math	= false;
	while i < chars.len() {
		let c = chars[i];
		if c == '\\' && i + 1 < chars.len() {
			out.push(c);
			out.push(chars[i + 1]);
			i += 2;
			continue;
		}
		if c == '`' {
			in_raw = !in_raw;
			out.push(c);
			i += 1;
			continue;
		}
		if c == '$' && !in_raw {
			in_math = !in_math;
			out.push(c);
			i += 1;
			continue;
		}
		if c == '#' && !in_raw && !in_math {
			let start = i + 1;
			let mut j = start;
			while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '-' || chars[j] == '_') {
				j += 1;
			}
			if j > start {
				let name: String = chars[start..j].iter().collect();
				if !matches!(chars.get(j), Some('(') | Some('[')) {
					if let Some(value) = sfns.get(&name) {
						out.push_str(value.display_text());
						i = j;
						continue;
					}
				}
			}
		}
		out.push(c);
		i += 1;
	}
	out
}

/// The content of a `key: [ ... ]` keyword argument, without its brackets. Used to read a furniture call's
/// `title:` argument. `None` when the key is absent or its value is not a content block.
fn named_content_arg(args: &str, key: &str) -> Option<String> {
	let at		= args.find(key)?;
	let after	= &args[at + key.len()..];
	let after	= after.trim_start();
	let after	= after.strip_prefix(':')?.trim_start();
	if !after.starts_with('[') {
		return None;
	}
	let chars:	Vec<char>	= after.chars().collect();
	read_group(&chars, 0).map(|(inner, _)| inner)
}

/// The plain text of a run of inline markup, dropping the markup and keeping the words -- a title is set as
/// one bold run, so its own emphasis is flattened rather than nested inside the bold.
fn inline_plain(runs: &[Inline]) -> String {
	let mut out = String::new();
	for run in runs {
		match run {
			Inline::Text(t) | Inline::Strong(t) | Inline::Emph(t) | Inline::BoldItalic(t)
			| Inline::Super(t) | Inline::Sub(t) | Inline::Code(t)	=> out.push_str(t),
			_										=> {},
		}
	}
	out
}

/// The `[ ... ]` body of a captured `#columns(n)[ ... ]` wrapper: the column count arguments are read and
/// dropped, and the bracketed block content returned for re-parsing. `None` when no `[...]` group follows
/// the arguments, so a malformed wrapper contributes no body.
fn columns_body(buf: &str) -> Option<String> {
	let chars:	Vec<char>	= buf.chars().collect();
	let Some(at) = find_lit(&chars, "#columns") else { return None; };
	let open	= at + "#columns".chars().count();
	if chars.get(open) != Some(&'(') {
		return None;
	}
	let Some((_, after_args)) = read_group(&chars, open) else { return None; };
	let mut j = after_args;
	while j < chars.len() && chars[j].is_whitespace() {
		j += 1;
	}
	if chars.get(j) != Some(&'[') {
		return None;
	}
	read_group(&chars, j).map(|(body, _)| body)
}

/// The `[ ... ]` body of a captured `#styled-box[ ... ]` callout, returned for re-parsing. The call takes
/// no arguments, so the bracket group opens immediately after the name. `None` when no `[...]` follows, so
/// a malformed callout contributes no body.
fn styled_box_body(buf: &str) -> Option<String> {
	let chars:	Vec<char>	= buf.chars().collect();
	let Some(at) = find_lit(&chars, "#styled-box") else { return None; };
	let open = at + "#styled-box".chars().count();
	if chars.get(open) != Some(&'[') {
		return None;
	}
	read_group(&chars, open).map(|(body, _)| body)
}

/// The index of the first occurrence of the literal `s` in `chars`, or `None`.
fn find_lit(chars: &[char], s: &str) -> Option<usize> {
	let pat:	Vec<char>	= s.chars().collect();
	if pat.is_empty() || chars.len() < pat.len() {
		return None;
	}
	(0..=chars.len() - pat.len()).find(|&start| chars[start..start + pat.len()] == pat[..])
}

/// Evaluates a `#let name = (...)` value into the flat sequence of cells it holds, each cell its inline
/// runs. The value is the paren group after the `=`; every `[...]` group within it, at any depth, is one
/// cell -- which is what `array.flatten()` yields for an array of content tuples.
fn parse_let_array(buf: &str) -> Vec<Vec<Inline>> {
	let chars:	Vec<char>	= buf.chars().collect();
	let eq = match chars.iter().position(|&c| c == '=') {
		Some(e)	=> e,
		None	=> return Vec::new(),
	};
	let open = match (eq + 1..chars.len()).find(|&j| !chars[j].is_whitespace()) {
		Some(v) if chars[v] == '('	=> v,
		_							=> return Vec::new(),
	};
	match read_group(&chars, open) {
		Some((inner, _))	=> collect_cells(&inner),
		None				=> Vec::new(),
	}
}

/// Collects every `[...]` group in `inner`, in order, each parsed into its inline runs. A `[` inside a
/// string is not a cell. Once a group opens, its whole content is one cell and is not descended into.
///
/// A `table.cell(colspan: n)[...]` wrapper is expanded to `n` grid cells: the bracketed content, then
/// `n - 1` empty span placeholders. A spanning cell consumes several columns in Typst, so without the
/// placeholders every cell after it slides one column to the left and the last column overruns the
/// table; the placeholders keep the flat cell stream aligned to the column grid. A bare `table.cell(...)`
/// (no `colspan`) is one cell, like a plain `[...]`.
fn collect_cells(inner: &str) -> Vec<Vec<Inline>> {
	let chars:	Vec<char>	= inner.chars().collect();
	let mut cells			= Vec::new();
	let mut in_str			= false;
	let mut esc				= false;
	let mut i				= 0usize;
	while i < chars.len() {
		let c = chars[i];
		if in_str {
			if esc			{ esc = false; }
			else if c == '\\'	{ esc = true; }
			else if c == '"'	{ in_str = false; }
			i += 1;
			continue;
		}
		if c == '"' {
			in_str = true;
			i += 1;
			continue;
		}
		// A `table.cell(args)[content]` wrapper: read the args for a `colspan`, then take the following
		// `[...]` as the content and emit it across that many columns.
		if let Some(after) = at_lit(&chars, i, "table.cell") {
			if chars.get(after) == Some(&'(') {
				if let Some((args, past_args)) = read_group(&chars, after) {
					let span	= cell_colspan(&args);
					let mut j	= past_args;
					while j < chars.len() && chars[j].is_whitespace() {
						j += 1;
					}
					if chars.get(j) == Some(&'[') {
						if let Some((content, next)) = read_group(&chars, j) {
							cells.push(parse_inlines(&content));
							for _ in 1..span {
								cells.push(vec![Inline::Text(String::new())]);
							}
							i = next;
							continue;
						}
					}
					// No content group followed the wrapper; step past its args and carry on.
					i = past_args;
					continue;
				}
			}
		}
		if c == '[' {
			if let Some((content, next)) = read_group(&chars, i) {
				cells.push(parse_inlines(&content));
				i = next;
				continue;
			}
		}
		i += 1;
	}
	cells
}

/// The `colspan:` of a `table.cell(...)` argument list, at least one. A missing or unreadable `colspan`
/// is a single column.
fn cell_colspan(args: &str) -> usize {
	for arg in split_top_args(args) {
		if let Some((key, val)) = named_arg(arg.trim()) {
			if key == "colspan" {
				if let Ok(n) = val.trim().parse::<usize>() {
					return n.max(1);
				}
			}
		}
	}
	1
}

/// Parses the inner text of a `#table(...)` call into a [`TableSpec`]. `columns:` fixes the column
/// count, `align:` the alignment, a `fill:` keyed on `row == 0` marks a header row; cells come from
/// inline `[...]` groups and from a `..name.flatten()` spread (or the row-remap idiom [`resolve_spread`]
/// evaluates) resolved against the data arrays. `None` when no cells are found, so an empty or
/// unresolved table sets nothing.
fn parse_table_spec(
	inner:			&str,
	arrays:			&HashMap<String, Vec<Vec<Inline>>>,
	outer_text_pt:	Option<f64>,
)
	-> Option<TableSpec>
{
	let mut ncols		= 1usize;
	let mut align		= AlignSpec::Uniform(Align::Left);
	let mut header		= false;
	let mut inset_pt:	Option<f64>			= None;
	let mut weights:	Vec<f64>			= Vec::new();
	let mut cells:		Vec<Vec<Inline>>	= Vec::new();
	// A spread's row-remap idiom needs the column count to chunk its array back into rows, so `columns:`
	// is read ahead of the main pass -- it names the table's shape wherever it sits in the argument list.
	let pre_ncols: usize = split_top_args(inner).iter()
		.find_map(|arg| named_arg(arg.trim()).filter(|(k, _)| k.as_str() == "columns").map(|(_, v)| parse_columns(&v)))
		.unwrap_or(1);
	for arg in split_top_args(inner) {
		let a = arg.trim();
		if a.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"columns"	=> { ncols = parse_columns(&val); weights = parse_column_weights(&val); },
				"align"		=> align = parse_align(&val),
				"fill"		=> if fill_marks_header(&val) { header = true; },
				"inset"		=> inset_pt = parse_length(&val).map(length_pt),
				_			=> {},	// stroke, gutter and the rest are not modelled
			}
			continue;
		}
		if spread_name(a).is_some() {
			if let Some(v) = resolve_spread(a, arrays, pre_ncols) {
				cells.extend(v);
			}
			continue;
		}
		// An inline positional cell, or a `table.header(...)`/`table.cell(...)` wrapper whose bracketed
		// content is the cell -- each `[...]` group in the argument is one cell, in order.
		if a.contains('[') {
			cells.extend(collect_cells(a));
		}
	}
	if cells.is_empty() {
		return None;
	}
	Some(TableSpec { ncols: ncols.max(1), header, align, weights, text_pt: outer_text_pt, inset_pt, cells })
}

/// Resolves a `..name` spread argument to the cells it contributes: the array itself for a bare `..name`
/// or `..name.flatten()`, or -- for the row-remap idiom `..name.enumerate().map(((idx, row)) => { if COND
/// { row } else { table.cell(colspan: n)[...] } }).flatten()`, as Lucronics' E. coli comparison uses to
/// merge its section-heading rows into spanning cells -- each row kept or replaced exactly as that
/// closure would evaluate it (see [`remap_enumerated_rows`]). `None` for an unknown array name; an
/// unrecognised suffix on a known array falls back to its raw cells, so an idiom this reader cannot
/// evaluate still sets a table rather than an empty one.
fn resolve_spread(
	arg:	&str,
	arrays:	&HashMap<String, Vec<Vec<Inline>>>,
	ncols:	usize,
)
	-> Option<Vec<Vec<Inline>>>
{
	let name	= spread_name(arg)?;
	let base	= arrays.get(&name)?;
	let rest	= arg.trim().strip_prefix("..")?[name.len()..].trim();
	if rest.is_empty() || rest == ".flatten()" {
		return Some(base.clone());
	}
	Some(remap_enumerated_rows(rest, base, ncols).unwrap_or_else(|| base.clone()))
}

/// Evaluates the `.enumerate().map(((idx, row)) => { if COND { A } else { B } }).flatten()` idiom
/// against `base` (chunked into `ncols`-wide row tuples), yielding the flat cell list the closure would
/// produce. `COND` is a bounded slice of Typst -- `idx == N`, `row.at(N) == [...]`/`!= [...]` literal
/// comparisons, joined by `and`/`or` and grouped by parens (see [`eval_bool`]); a branch that is the bare
/// loop variable keeps that row's own cells, one shaped `table.cell(colspan: n)[...]` replaces them with
/// one spanning cell -- its content may reference the loop variable's cells as `row.at(k)`, substituted
/// with that cell's plain text before the existing `table.cell(...)` reader ([`collect_cells`]) parses it,
/// so a `#strong(row.at(0))` inside renders through the ordinary strong-call path. `None` when the suffix
/// is not this shape, or any row's condition or branch does not evaluate, so the caller falls back to the
/// untransformed array rather than guess at a partial result.
fn remap_enumerated_rows(after: &str, base: &[Vec<Inline>], ncols: usize) -> Option<Vec<Vec<Inline>>> {
	if ncols == 0 || base.is_empty() || base.len() % ncols != 0 {
		return None;
	}
	let rest	= after.strip_prefix(".enumerate()")?.trim_start().strip_prefix(".map")?;
	let rchars:	Vec<char>	= rest.chars().collect();
	if rchars.first() != Some(&'(') {
		return None;
	}
	let (map_args, _) = read_group(&rchars, 0)?;
	let arrow	= map_args.find("=>")?;
	let params	= map_args[..arrow].trim();
	let body	= map_args[arrow + 2..].trim();
	// The closure's own `{ ... }` block wraps its one expression -- unwrap it so what remains starts at
	// the `if`, the shape [`parse_if_else`] reads.
	let bchars:	Vec<char>	= body.chars().collect();
	let body: String = if bchars.first() == Some(&'{') {
		read_brace(&bchars, 0).map(|(inner, _)| inner)?
	} else {
		body.to_string()
	};

	// The parameter is `((idx, row))`: an extra pair of parens wraps the tuple destructure, since it is
	// `.map`'s single positional argument.
	let pchars:	Vec<char>	= params.chars().collect();
	let inner_params = if pchars.first() == Some(&'(') {
		read_group(&pchars, 0)?.0
	} else {
		params.to_string()
	};
	let inner_params	= inner_params.trim();
	let inner_params	= inner_params.strip_prefix('(').and_then(|s| s.strip_suffix(')')).unwrap_or(inner_params);
	let names:	Vec<&str>	= inner_params.split(',').map(|s| s.trim()).collect();
	if names.len() != 2 || names[0].is_empty() || names[1].is_empty() {
		return None;
	}
	let (idx_name, row_name) = (names[0], names[1]);

	let (cond, a_branch, b_branch) = parse_if_else(&body)?;

	let mut out = Vec::with_capacity(base.len());
	for (i, chunk) in base.chunks(ncols).enumerate() {
		let row_text: Vec<String> = chunk.iter().map(|cell| inline_plain(cell)).collect();
		let take_a = eval_bool(&cond, i, &row_text, idx_name, row_name)?;
		let branch = if take_a { &a_branch } else { &b_branch };
		if branch.trim() == row_name {
			out.extend(chunk.iter().cloned());
			continue;
		}
		let substituted	= substitute_row_refs(branch, row_name, &row_text);
		let cells		= collect_cells(&substituted);
		if cells.is_empty() {
			return None;	// the branch is not a cell this reader can set; refuse the whole idiom
		}
		out.extend(cells);
	}
	Some(out)
}

/// Splits an `if COND { A } else { B }` closure body into its condition and branch source texts. The
/// condition runs to the first `{` not nested inside `(...)`/`[...]`; each branch is then read as a
/// brace-balanced group. `None` when the body is not this shape.
fn parse_if_else(body: &str) -> Option<(String, String, String)> {
	let rest	= body.trim().strip_prefix("if")?;
	let chars:	Vec<char>	= rest.chars().collect();
	let mut depth	= 0i32;
	let mut brace_at = None;
	for (k, &c) in chars.iter().enumerate() {
		match c {
			'(' | '['			=> depth += 1,
			')' | ']'			=> depth -= 1,
			'{' if depth == 0	=> { brace_at = Some(k); break; },
			_					=> {},
		}
	}
	let brace_at	= brace_at?;
	let cond:	String	= chars[..brace_at].iter().collect();
	let (a_branch, next)	= read_brace(&chars, brace_at)?;
	let after: String		= chars[next..].iter().collect();
	let after				= after.trim().strip_prefix("else")?.trim();
	let bchars:	Vec<char>	= after.chars().collect();
	if bchars.first() != Some(&'{') {
		return None;
	}
	let (b_branch, _) = read_brace(&bchars, 0)?;
	Some((cond.trim().to_string(), a_branch.trim().to_string(), b_branch.trim().to_string()))
}

/// Reads a `{ ... }` block beginning at `i`, matching only `{`/`}` depth -- the closure bodies this
/// idiom targets hold no literal braces of their own, so a flat counter is enough, unlike [`read_group`]'s
/// full frame tracking for `[`/`(`. `None` when the block never closes.
fn read_brace(chars: &[char], i: usize) -> Option<(String, usize)> {
	if chars.get(i) != Some(&'{') {
		return None;
	}
	let mut depth	= 1i32;
	let start		= i + 1;
	let mut j		= start;
	while j < chars.len() {
		match chars[j] {
			'{'	=> depth += 1,
			'}'	=> {
				depth -= 1;
				if depth == 0 {
					return Some((chars[start..j].iter().collect(), j + 1));
				}
			},
			_	=> {},
		}
		j += 1;
	}
	None
}

/// Evaluates a bounded boolean expression -- comparisons of `idx`/`row.at(n)` against a literal or a
/// number, joined by `and`/`or` and grouped by parens -- against one row's values. `idx_name`/`row_name`
/// are the closure's own parameter names, so the expression is read against exactly the variables that
/// idiom bound. `None` when the expression falls outside this bounded grammar.
fn eval_bool(expr: &str, idx: usize, row: &[String], idx_name: &str, row_name: &str) -> Option<bool> {
	let expr = expr.trim();
	if let Some(parts) = split_top_level(expr, "or") {
		let mut acc = false;
		for p in &parts {
			acc = acc || eval_bool(p, idx, row, idx_name, row_name)?;
		}
		return Some(acc);
	}
	if let Some(parts) = split_top_level(expr, "and") {
		let mut acc = true;
		for p in &parts {
			acc = acc && eval_bool(p, idx, row, idx_name, row_name)?;
		}
		return Some(acc);
	}
	if expr.starts_with('(') && expr.ends_with(')') {
		// Confirm the outer parens actually wrap the whole expression, rather than two disjoint groups
		// that merely happen to open and close at the ends.
		let mut depth = 0i32;
		let mut wraps_whole = true;
		let last = expr.chars().count() - 1;
		for (k, c) in expr.chars().enumerate() {
			match c {
				'('	=> depth += 1,
				')'	=> {
					depth -= 1;
					if depth == 0 && k != last {
						wraps_whole = false;
						break;
					}
				},
				_	=> {},
			}
		}
		if wraps_whole {
			return eval_bool(&expr[1..expr.len() - 1], idx, row, idx_name, row_name);
		}
	}
	eval_cmp(expr, idx, row, idx_name, row_name)
}

/// Splits `expr` at every top-level (outside `(...)`/`[...]`) whole-word occurrence of `kw` (`"and"` or
/// `"or"`), or `None` when `kw` does not occur at the top level, so the caller falls through to the next
/// precedence rather than treating an absent operator as one empty operand.
fn split_top_level(expr: &str, kw: &str) -> Option<Vec<String>> {
	let chars:	Vec<char>	= expr.chars().collect();
	let kwc:	Vec<char>	= kw.chars().collect();
	let mut depth	= 0i32;
	let mut parts	= Vec::new();
	let mut start	= 0usize;
	let mut i		= 0usize;
	let mut found	= false;
	while i < chars.len() {
		match chars[i] {
			'(' | '['	=> { depth += 1; i += 1; continue; },
			')' | ']'	=> { depth -= 1; i += 1; continue; },
			_			=> {},
		}
		if depth == 0 && i + kwc.len() <= chars.len() && chars[i..i + kwc.len()] == kwc[..] {
			let before_ok	= i == 0 || chars[i - 1].is_whitespace();
			let after_ok	= chars.get(i + kwc.len()).map(|c| c.is_whitespace()).unwrap_or(true);
			if before_ok && after_ok {
				parts.push(chars[start..i].iter().collect::<String>());
				i = i + kwc.len();
				start = i;
				found = true;
				continue;
			}
		}
		i += 1;
	}
	if !found {
		return None;
	}
	parts.push(chars[start..].iter().collect::<String>());
	Some(parts.into_iter().map(|s| s.trim().to_string()).collect())
}

/// Evaluates one `lhs (==|!=) rhs` comparison against `idx`/`row`, via [`eval_scalar`]. `None` when
/// neither `==` nor `!=` appears, or either side does not resolve.
fn eval_cmp(e: &str, idx: usize, row: &[String], idx_name: &str, row_name: &str) -> Option<bool> {
	let e = e.trim();
	let (eq, at) = if let Some(p) = e.find("!=") {
		(false, p)
	} else if let Some(p) = e.find("==") {
		(true, p)
	} else {
		return None;
	};
	let lhs = eval_scalar(e[..at].trim(), idx, row, idx_name, row_name)?;
	let rhs = eval_scalar(e[at + 2..].trim(), idx, row, idx_name, row_name)?;
	Some(if eq { lhs == rhs } else { lhs != rhs })
}

/// Reads one side of a comparison: the loop index, a `row.at(n)` cell (its plain text), a `[...]` content
/// literal (its plain text) or a bare number. `None` for anything else, refusing rather than guessing.
fn eval_scalar(s: &str, idx: usize, row: &[String], idx_name: &str, row_name: &str) -> Option<String> {
	let s = s.trim();
	if s == idx_name {
		return Some(idx.to_string());
	}
	if let Some(rest) = s.strip_prefix(row_name) {
		let n = rest.strip_prefix(".at(")?.strip_suffix(')')?.trim().parse::<usize>().ok()?;
		return row.get(n).cloned();
	}
	if let Some(lit) = s.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
		return Some(lit.trim().to_string());
	}
	if s.parse::<usize>().is_ok() {
		return Some(s.to_string());
	}
	None
}

/// Replaces every `<row_name>.at(k)` reference in `src` with a quoted string of that cell's plain text,
/// so a branch like `table.cell(colspan: 7)[#strong(row.at(0))]` becomes a call [`collect_cells`] (via
/// the ordinary `#strong("...")` reader) can already set, rather than teaching the cell reader to evaluate
/// arbitrary code.
fn substitute_row_refs(src: &str, row_name: &str, row: &[String]) -> String {
	let mut out = src.to_string();
	for (k, cell) in row.iter().enumerate() {
		let pat = fmt!("{}.at({})", row_name, k);
		if out.contains(&pat) {
			let escaped	= cell.replace('\\', "\\\\").replace('"', "\\\"");
			out = out.replace(&pat, &fmt!("\"{}\"", escaped));
		}
	}
	out
}

/// The absolute point value of a [`Length`], resolving a percentage against a nominal 100 pt so a
/// percentage inset still yields a sensible padding; a table's inset is in practice an absolute length.
pub(crate) fn length_pt(len: Length) -> f64 {
	match len {
		Length::Abs(pt)	=> pt,
		Length::Rel(f)	=> f * 100.0,
	}
}

/// The size of a `text(size: Npt)[...]` (or `#text(...)`) wrapper at the start of `text`, in points, or
/// `None` when the body is not wrapped in a sized `text` call. This lets a figure's small-set table --
/// `text(size: 7pt)[#table(...)]` -- carry its reduced size, which Typst applies to the whole table.
fn outer_text_size(text: &str) -> Option<f64> {
	let inner = call_inner(text, "text")?;
	for arg in split_top_args(&inner) {
		let a = arg.trim();
		if let Some((key, val)) = named_arg(a) {
			if key == "size" {
				return parse_length(&val).map(length_pt);
			}
		} else if a.ends_with("pt") || a.ends_with("em") {
			// The first positional length is the size, as in `text(7pt)[...]`.
			return parse_length(a).map(length_pt);
		}
	}
	None
}

/// Parses a `#figure(...)` call (its buffer, a trailing `<label>` and all) into an [`Item::Figure`]. The
/// positional argument is the body -- a wrapped `#table(...)` set in full, or an image call stood in for
/// by a placeholder; `caption:` sets the caption, `supplement:`/`kind:` the "Figure" or "Table" label.
fn parse_figure(buf: &str, arrays: &HashMap<String, Vec<Vec<Inline>>>) -> Option<Item> {
	let (body_src, label)	= strip_trailing_label(buf);
	let inner				= call_inner(&body_src, "figure")?;

	let mut caption:	Option<Vec<Inline>>	= None;
	let mut supplement:	Option<String>	= None;
	let mut kind:		Option<String>	= None;
	let mut positional:	Option<String>	= None;
	let mut placement:	Option<FloatPlacement>	= None;
	for arg in split_top_args(&inner) {
		let a = arg.trim();
		if a.is_empty() {
			continue;
		}
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"caption"		=> caption = Some(caption_inlines(&val)),
				"supplement"	=> supplement = Some(unquote(&val)),
				"kind"			=> kind = Some(unquote(&val)),
				"placement"		=> placement = parse_placement(&val),
				_				=> {},	// the rest do not affect the set figure
			}
			continue;
		}
		if positional.is_none() {
			positional = Some(a.to_string());	// the first positional argument is the figure body
		}
	}

	let body_text	= positional.unwrap_or_default();
	let body		= figure_body(&body_text, arrays);
	let supplement	= supplement.unwrap_or_else(|| match kind.as_deref() {
		Some("table")	=> "Table".to_string(),
		_				=> "Figure".to_string(),
	});
	Some(Item::Figure { body, caption, supplement, label, placement, span: Span::new(0, 0) })
}

/// Reads a `#figure` `placement:` value into a float placement. `auto` and `top` float to the page top,
/// `bottom` to the foot; `none` (and anything unrecognised) leaves the figure in the flow. Typst's own
/// default for a figure is `none`, so a figure that names no placement is not a float.
fn parse_placement(val: &str) -> Option<FloatPlacement> {
	match val.trim() {
		"auto"		=> Some(FloatPlacement::Auto),
		"top"		=> Some(FloatPlacement::Top),
		"bottom"	=> Some(FloatPlacement::Bottom),
		_			=> None,
	}
}

/// Decides a figure's body from its positional text: a wrapped `#table(...)` if one is present and
/// parses, otherwise an image carrying the path and any declared sizing (empty path when none is found).
fn figure_body(text: &str, arrays: &HashMap<String, Vec<Vec<Inline>>>) -> FigureBody {
	if let Some(inner) = call_inner(text, "table") {
		if let Some(spec) = parse_table_spec(&inner, arrays, outer_text_size(text)) {
			return FigureBody::Table(spec);
		}
	}
	// A CeTZ/Fletcher diagram, bar chart or line plot drawn inline is read into a builder that draws it
	// for real; only when the body is none of these does it fall through to the image/placeholder path.
	if let Some(cf) = super::codefig::parse_code_figure(text) {
		return FigureBody::Code(cf);
	}
	let (path, width, height, scale) = image_call(text);
	FigureBody::Image { path, width, height, scale }
}

/// The path and sizing of a `padded-image("...")` or `image("...")` call in `text`. The custom wrapper is
/// tried first, since `image` is a word boundary within it only after the hyphen. The first positional
/// argument is the path; `width`/`height` size an `image(...)`, `scale` a `padded-image(...)`. A path
/// that is not found gives an empty string, which the block layer stands in for with a placeholder.
fn image_call(text: &str) -> (String, Option<Length>, Option<Length>, Option<f64>) {
	for name in ["padded-image", "image"] {
		if let Some(inner) = call_inner(text, name) {
			let mut path:	Option<String>	= None;
			let mut width:	Option<Length>	= None;
			let mut height:	Option<Length>	= None;
			let mut scale:	Option<f64>		= None;
			for arg in split_top_args(&inner) {
				let a = arg.trim();
				if a.is_empty() {
					continue;
				}
				if let Some((key, val)) = named_arg(a) {
					match key.as_str() {
						"width"		=> width = parse_length(&val),
						"height"	=> height = parse_length(&val),
						"scale"		=> scale = parse_percent(&val),
						_			=> {},	// padding and the rest do not size the set image
					}
					continue;
				}
				if path.is_none() {
					path = first_string(a);
				}
			}
			if let Some(p) = path {
				return (p, width, height, scale);
			}
		}
	}
	(String::new(), None, None, None)
}

/// Reads a Typst length argument into a [`Length`]: a percentage as a fraction of the measure, a `pt`,
/// `mm`, `cm` or `in` length as absolute points, a bare number as points. `auto` and anything unreadable
/// give `None`, so the figure falls back to filling the measure.
pub(crate) fn parse_length(val: &str) -> Option<Length> {
	let v = val.trim();
	if let Some(pct) = v.strip_suffix('%') {
		return pct.trim().parse::<f64>().ok().map(|n| Length::Rel(n / 100.0));
	}
	for (unit, per_pt) in [("pt", 1.0), ("mm", 72.0 / 25.4), ("cm", 72.0 / 2.54), ("in", 72.0)] {
		if let Some(num) = v.strip_suffix(unit) {
			return num.trim().parse::<f64>().ok().map(|n| Length::Abs(n * per_pt));
		}
	}
	v.parse::<f64>().ok().map(Length::Abs)
}

/// Parses a standalone `#line(length:.., stroke:..)` into an [`Item::Rule`]. The length is a fraction of
/// the measure (`100%`) or an absolute length; the stroke gives the rule's thickness (a `pt` length) and
/// its grey (a `luma(N)` component). A missing length fills the measure; a missing thickness is a hairline
/// half-point; a missing colour is black, Typst's default stroke.
fn parse_line_rule(trimmed: &str) -> Option<Item> {
	let inner		= call_inner(trimmed, "line")?;
	let mut width	= Length::Rel(1.0);
	let mut thickness	= 0.5;
	let mut grey	= 0u8;
	for arg in split_top_args(&inner) {
		let a = arg.trim();
		if let Some((key, val)) = named_arg(a) {
			match key.as_str() {
				"length"	=> if let Some(l) = parse_length(&val) { width = l; },
				"stroke"	=> {
					let (t, g) = parse_stroke(&val);
					if let Some(t) = t { thickness = t; }
					if let Some(g) = g { grey = g; }
				},
				_			=> {},	// start, end, angle and the rest do not affect a horizontal divider
			}
		}
	}
	Some(Item::Rule { width, thickness, grey, span: Span::new(0, 0) })
}

/// The thickness (a `pt` length) and grey (a `luma(N)` value, 0-255) of a `stroke:` value such as
/// `0.5pt + luma(180)`; either component may be absent. A `luma` is read as a grey level; a bare colour
/// name or an `rgb(...)` is not modelled and leaves the grey unset.
fn parse_stroke(val: &str) -> (Option<f64>, Option<u8>) {
	let mut thickness:	Option<f64>	= None;
	let mut grey:		Option<u8>	= None;
	for part in val.split('+') {
		let p = part.trim();
		if let Some(inner) = call_inner(p, "luma") {
			if let Ok(n) = inner.trim().trim_end_matches('%').trim().parse::<f64>() {
				grey = Some(n.clamp(0.0, 255.0) as u8);
			}
		} else if let Some(Length::Abs(pt)) = parse_length(p) {
			thickness = Some(pt);
		}
	}
	(thickness, grey)
}

/// Reads a percentage argument (`100%`) into a fraction (`1.0`), or `None` when it is not a percentage.
fn parse_percent(val: &str) -> Option<f64> {
	val.trim().strip_suffix('%').and_then(|p| p.trim().parse::<f64>().ok()).map(|n| n / 100.0)
}

/// The content of the first `name(...)` call in `text`, balanced across nesting and strings, or `None`.
/// `name` must sit at a word boundary, so a short name does not match inside a longer identifier.
pub(crate) fn call_inner(text: &str, name: &str) -> Option<String> {
	let chars:	Vec<char>	= text.chars().collect();
	let namev:	Vec<char>	= name.chars().collect();
	let paren				= find_call(&chars, &namev, 0)?;
	read_group(&chars, paren).map(|(inner, _)| inner)
}

/// The index of the `(` of the first `name(` at a word boundary at or after `from`, or `None`.
fn find_call(chars: &[char], name: &[char], from: usize) -> Option<usize> {
	if name.is_empty() {
		return None;
	}
	let mut i = from;
	while i + name.len() < chars.len() {
		if chars[i..].starts_with(name) && chars.get(i + name.len()) == Some(&'(') {
			let boundary = i == 0 || !is_call_ident(chars[i - 1]);
			if boundary {
				return Some(i + name.len());
			}
		}
		i += 1;
	}
	None
}

/// A character that continues a Typst identifier, for the word-boundary test in [`find_call`].
fn is_call_ident(c: char) -> bool {
	c.is_alphanumeric() || c == '-' || c == '_'
}

/// The first `"..."` string literal's content in `text`, or `None`.
pub(crate) fn first_string(text: &str) -> Option<String> {
	let chars:	Vec<char>	= text.chars().collect();
	let start				= chars.iter().position(|&c| c == '"')?;
	let end					= (start + 1..chars.len()).find(|&j| chars[j] == '"')?;
	Some(chars[start + 1..end].iter().collect())
}

/// Splits the inner text of a call by its top-level commas, respecting `()[]{}` nesting and `"..."`
/// strings, so a comma inside a nested group or a string does not part an argument.
pub(crate) fn split_top_args(inner: &str) -> Vec<String> {
	let chars:	Vec<char>	= inner.chars().collect();
	let mut args:	Vec<String>	= Vec::new();
	let mut cur					= String::new();
	let mut state				= SkipState::new();
	let mut i					= 0;
	while i < chars.len() {
		// A comma parts the arguments only at the top level; inside any frame -- a nested group, a string,
		// a maths span or a content block -- it is literal and joins the current argument.
		if !state.is_open() && chars[i] == ',' {
			args.push(std::mem::take(&mut cur));
			i += 1;
			continue;
		}
		let consumed = state.step(&chars, i);
		for k in i..i + consumed {
			cur.push(chars[k]);
		}
		i += consumed;
	}
	if !cur.trim().is_empty() {
		args.push(cur);
	}
	args
}

/// Splits a `key: value` argument at its top-level colon, returning the key and the trimmed value, or
/// `None` when there is no top-level colon or the key is not a bare identifier -- so a positional cell
/// or a spread is not mistaken for a named argument.
pub(crate) fn named_arg(arg: &str) -> Option<(String, String)> {
	let chars:	Vec<char>	= arg.chars().collect();
	let mut state			= SkipState::new();
	let mut i				= 0;
	while i < chars.len() {
		// A colon names the argument only at the top level; inside any frame it is part of the value (an
		// alignment `align: (col, row) => ...`, a ratio in a caption, a dictionary key in code).
		if !state.is_open() && chars[i] == ':' {
			let key: String = chars[..i].iter().collect();
			let key = key.trim().to_string();
			if !key.is_empty() && key.chars().all(is_call_ident) {
				let val: String = chars[i + 1..].iter().collect();
				return Some((key, val.trim().to_string()));
			}
			return None;
		}
		i += state.step(&chars, i);
	}
	None
}

/// The array name of a `..name` or `..name.flatten()` spread argument, or `None`.
fn spread_name(arg: &str) -> Option<String> {
	let rest		= arg.trim().strip_prefix("..")?;
	let name: String	= rest.chars().take_while(|&c| is_call_ident(c)).collect();
	if name.is_empty() {
		None
	} else {
		Some(name)
	}
}

/// Parses a `columns:` value into a column count: an integer as itself, a track list `(a, b, c)` as its
/// entry count, anything else as one column.
fn parse_columns(val: &str) -> usize {
	let v = val.trim();
	if let Ok(n) = v.parse::<usize>() {
		return n.max(1);
	}
	if v.starts_with('(') {
		let ch: Vec<char> = v.chars().collect();
		if let Some((inner, _)) = read_group(&ch, 0) {
			let cnt = split_top_args(&inner).iter().filter(|p| !p.trim().is_empty()).count();
			return cnt.max(1);
		}
	}
	1
}

/// The per-column fractional weights of a `columns:` track list: a track `Nfr` (or a bare `fr`, weight 1)
/// contributes its weight, an `auto` or a fixed length contributes `0.0` so the column is sized to its
/// content. A bare `columns: N` gives no weights (an empty vector), leaving every column content-sized.
/// The weights let [`table::lower`](crate::table) reproduce Typst's fractional column sizing rather than
/// sizing every column from its widest cell.
fn parse_column_weights(val: &str) -> Vec<f64> {
	let v = val.trim();
	if !v.starts_with('(') {
		return Vec::new();
	}
	let ch: Vec<char> = v.chars().collect();
	let inner = match read_group(&ch, 0) {
		Some((inner, _))	=> inner,
		None				=> return Vec::new(),
	};
	let mut out = Vec::new();
	for track in split_top_args(&inner) {
		let t = track.trim();
		if t.is_empty() {
			continue;
		}
		out.push(track_weight(t));
	}
	out
}

/// The fractional weight of one `columns:` track: `Nfr` reads as `N`, a bare `fr` as `1`, and any other
/// track -- `auto`, `3cm`, `40pt`, `20%` -- as `0.0`, which marks the column content-sized.
fn track_weight(track: &str) -> f64 {
	match track.strip_suffix("fr") {
		Some(num) => {
			let n = num.trim();
			if n.is_empty() { 1.0 } else { n.parse::<f64>().unwrap_or(0.0) }
		},
		None => 0.0,
	}
}

/// Parses an `align:` value: a `(col, row) => ...` closure as [`AlignSpec::Closure`] (its parameter
/// names and body captured for per-cell evaluation), a tuple of column alignments as
/// [`AlignSpec::PerColumn`], a single alignment word as [`AlignSpec::Uniform`].
fn parse_align(val: &str) -> AlignSpec {
	let v = val.trim();
	if let Some(arrow) = v.find("=>") {
		let params	= v[..arrow].trim();
		let body	= v[arrow + 2..].trim().to_string();
		// The parameter list `(col, row)`; a bare single parameter has no parentheses. The first names the
		// column, the second the row, matching Typst's `(col, row)` order.
		let names: Vec<String> = {
			let pch: Vec<char> = params.chars().collect();
			match read_group(&pch, 0) {
				Some((inner, _))	=> split_top_args(&inner).iter().map(|s| s.trim().to_string()).collect(),
				None				=> vec![params.trim().to_string()],
			}
		};
		let col_var = names.first().cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "col".to_string());
		let row_var = names.get(1).cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "row".to_string());
		return AlignSpec::Closure(ClosureAlign { col_var, row_var, body });
	}
	if v.starts_with('(') {
		let ch: Vec<char> = v.chars().collect();
		if let Some((inner, _)) = read_group(&ch, 0) {
			let cols: Vec<Align> = split_top_args(&inner).iter().map(|p| word_align(p)).collect();
			if !cols.is_empty() {
				return AlignSpec::PerColumn(cols);
			}
		}
	}
	AlignSpec::Uniform(word_align(v))
}

/// Maps a Typst alignment word to an [`Align`], ignoring a `+ horizon`/`+ top` vertical component and
/// treating `start`/`end` as left/right. An unknown word is left-aligned.
fn word_align(s: &str) -> Align {
	let first = s.trim().split(|c: char| c.is_whitespace() || c == '+').next().unwrap_or("").trim();
	match first {
		"center" | "centre"	=> Align::Centre,
		"right" | "end"		=> Align::Right,
		_					=> Align::Left,
	}
}

/// Does a `fill:` value key on the first row, marking a header? A `fill: (col, row) => ...` closure whose
/// body tests the row index against zero (`row == 0` or the common `y == 0`) fills the first row, which is
/// the books' header idiom; a `y < n` band likewise begins at the first row. Written with or without
/// spaces, and matching either name the closure gives its second (row) parameter.
fn fill_marks_header(val: &str) -> bool {
	let compact: String = val.chars().filter(|c| !c.is_whitespace()).collect();
	compact.contains("row==0")
		|| compact.contains("y==0")
		|| compact.contains("row<")
		|| compact.contains("y<")
}

/// Parses a `caption: [...]` value into its inline runs: the bracket content scanned for markup, or the
/// whole value scanned when it is not a bracket group, so a caption's emphasis, superscript or in-caption
/// maths sets with its own face rather than flattening to upright text.
fn caption_inlines(val: &str) -> Vec<Inline> {
	let v = val.trim();
	let ch: Vec<char> = v.chars().collect();
	if ch.first() == Some(&'[') {
		if let Some((content, _)) = read_group(&ch, 0) {
			return parse_inlines(&content);
		}
	}
	parse_inlines(v)
}

/// Strips a trailing `<label>` from a captured call, returning the call text without it and the label.
/// A `<name>` with no inner whitespace at the very end labels the figure; anything else keeps the text.
fn strip_trailing_label(buf: &str) -> (String, Option<String>) {
	let t = buf.trim_end();
	if let Some(inner) = t.strip_suffix('>') {
		if let Some(p) = inner.rfind('<') {
			let label = &inner[p + 1..];
			if !label.is_empty() && !label.contains(char::is_whitespace) {
				return (inner[..p].to_string(), Some(label.to_string()));
			}
		}
	}
	(buf.to_string(), None)
}

/// Strips a surrounding `"..."` from a string-literal argument value, leaving anything else unchanged.
fn unquote(val: &str) -> String {
	let t = val.trim();
	if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
		return t[1..t.len() - 1].to_string();
	}
	t.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::{Atom, MatKind};

	/// A `#claim-label` emits a `MarginNote` carrying its compressed code as the margin display and its raw
	/// code for the reverse index, setting nothing in the body column; a `#claim-refs` emits a `MarginNote`
	/// with an empty display (nothing drawn) carrying its raw reference codes. The body prose on either side
	/// closes over the gap when flattened, so no raw markup leaks.
	#[test]
	fn claim_label_emits_a_margin_note_and_sets_nothing_inline() {
		let runs = parse_inlines("clinical authority#claim-label(<LS8>) bites hardest.");
		assert_eq!(runs.len(), 3, "text, margin note, text: got {:?}", runs);
		assert!(matches!(&runs[0], Inline::Text(t) if t == "clinical authority"));
		assert!(matches!(&runs[1], Inline::MarginNote { display, codes } if display == "LS8" && codes == &vec!["LS8".to_string()]),
			"the claim code rides in a margin note and registers for the reverse index: {:?}", runs[1]);
		assert!(matches!(&runs[2], Inline::Text(t) if t == " bites hardest."));
		// A `#claim-refs` emits a margin note with an empty display carrying its reference codes.
		let refs = parse_inlines("formalised#claim-refs(<A1>, <A2>).");
		assert!(refs.iter().any(|r| matches!(r, Inline::MarginNote { display, codes }
			if display.is_empty() && codes == &vec!["A1".to_string(), "A2".to_string()])),
			"a claim-refs registers its raw codes with no margin ink: {:?}", refs);
		// Neither the margin code nor a reference is part of the flattened body text.
		assert_eq!(
			flatten_markup("margins#claim-label(<CD14>, <CD15>, <CD4>) formalised#claim-refs(<A1>)."),
			"margins formalised.");
	}

	/// A bare `#name` naming a scalar `#let` value binding substitutes its display text: a string's contents
	/// unquoted, and a number/length literal's own source text, wherever it stands in running text -- at the
	/// start, mid-sentence, or immediately before punctuation.
	#[test]
	fn substitute_scalars_replaces_a_bound_scalar_in_prose() {
		let mut sfns = crate::lang::rules::ScalarFns::new();
		sfns.insert("version".to_string(), crate::lang::rules::ScalarValue::Str("1.2".to_string()));
		sfns.insert("edition".to_string(), crate::lang::rules::ScalarValue::Number("3".to_string()));
		assert_eq!(
			substitute_scalars("This is version #version, edition #edition, of the guide.", &sfns),
			"This is version 1.2, edition 3, of the guide.");
		// A reference at the very start of the text (the shape a heading title reads).
		assert_eq!(substitute_scalars("#version Notes", &sfns), "1.2 Notes");
	}

	/// A `#name` inside a raw `` `...` `` code span or a `$...$` maths span is left untouched -- neither is
	/// this reader's word, matching how [`parse_inlines_in`] treats them as literal/foreign territory -- and
	/// an escaped `\#name` is left alone too, so the backslash still reaches the inline scanner to produce a
	/// literal `#`. An unbound name, or a bound name immediately followed by `(`/`[` (a call, not a bare
	/// reference), is also untouched.
	#[test]
	fn substitute_scalars_leaves_raw_math_and_escaped_references_alone() {
		let mut sfns = crate::lang::rules::ScalarFns::new();
		sfns.insert("version".to_string(), crate::lang::rules::ScalarValue::Str("1.2".to_string()));
		assert_eq!(substitute_scalars("see `#version` in code", &sfns), "see `#version` in code");
		assert_eq!(substitute_scalars("the constant $#version$ here", &sfns), "the constant $#version$ here");
		assert_eq!(substitute_scalars("literal \\#version stays", &sfns), "literal \\#version stays");
		assert_eq!(substitute_scalars("#unknown-name here", &sfns), "#unknown-name here");
		assert_eq!(substitute_scalars("#version(1) call-shaped", &sfns), "#version(1) call-shaped");
	}

	/// A scalar `#let` value binding substitutes at a bare `#name` reference in both a heading title and
	/// running prose, through the whole [`document_with_templates`] pipeline (a [`Bindings::with_scalars`]
	/// scope, as [`crate::book::Scope::bindings`] builds for a real compile) -- not just in the
	/// [`substitute_scalars`] unit above.
	#[test]
	fn scalar_substitution_applies_in_a_heading_and_in_prose() {
		let tfns = crate::lang::rules::TemplateFns::new();
		let cfns = crate::lang::rules::ContentFns::new();
		let mut sfns = crate::lang::rules::ScalarFns::new();
		sfns.insert("title".to_string(), crate::lang::rules::ScalarValue::Str("Guide".to_string()));
		let binds = crate::lang::rules::Bindings::with_scalars(&tfns, &cfns, &sfns);
		let src = "= #title\n\nThe #title is version one.\n";
		let (items, _skips) = document_with_templates(src, binds).expect("parses");
		match &items[0] {
			Item::Heading { runs, .. } => assert!(
				matches!(&runs[0], Inline::Text(t) if t.contains("Guide")),
				"the heading substitutes the bound scalar: {:?}", runs),
			other => panic!("expected a heading first, got {:?}", other),
		}
		match &items[1] {
			Item::Paragraph { runs, .. } => {
				let text: String = runs.iter().map(|r| match r {
					Inline::Text(t) => t.clone(),
					_ => String::new(),
				}).collect();
				assert!(text.contains("Guide"), "the paragraph substitutes the bound scalar: {:?}", text);
			},
			other => panic!("expected a paragraph second, got {:?}", other),
		}
	}

	/// A bare `#name` standing alone on its own line -- bound to no scalar, content or template binding --
	/// is still a visible refusal, exactly as before scalar bindings existed: [`document_with_templates`]
	/// records it in the returned [`Refusals`] rather than leaking its raw source as a paragraph, whether or
	/// not the document also carries a scalar scope. Scalar substitution only ever fires for a NAME the
	/// scope actually binds ([`substitute_scalars`] is a no-op otherwise), so adding it never turns this
	/// existing refusal into a silent drop.
	#[test]
	fn unbound_standalone_reference_stays_a_visible_refusal() {
		let mut sfns = crate::lang::rules::ScalarFns::new();
		sfns.insert("title".to_string(), crate::lang::rules::ScalarValue::Str("Guide".to_string()));
		let tfns = crate::lang::rules::TemplateFns::new();
		let cfns = crate::lang::rules::ContentFns::new();
		let binds = crate::lang::rules::Bindings::with_scalars(&tfns, &cfns, &sfns);
		let src = "Some prose above.\n\n#nosuchname\n\nSome prose below.\n";
		let (items, skips) = document_with_templates(src, binds).expect("parses");
		assert!(items.iter().all(|it| !matches!(it, Item::Paragraph { runs, .. }
			if runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("#nosuchname"))))),
			"an unbound standalone reference must not leak as paragraph text: {:?}", items);
		assert!(skips.report().is_some_and(|r| r.contains("nosuchname")),
			"an unbound standalone reference is a visible refusal, not a silent drop: {:?}", skips.report());
	}

	/// A scalar `#let` binding referenced by a bare `#name` standing ALONE on its own line substitutes its
	/// value, through the whole [`document_with_templates`] pipeline -- not only mid-prose (which item 2 already
	/// handled) but where the reference is the line's only content, the case the standalone code-reference skip
	/// path swallowed before. The gate is self-non-vacuous: with the `names_scalar_alone` exemption reverted the
	/// standalone `#edition` is recorded as a `#pagebreak`-style skip and never becomes a paragraph, so the
	/// substituted value is absent and a skip is reported -- both assertions below then red. The companion
	/// [`unbound_standalone_reference_stays_a_visible_refusal`] proves an UNBOUND standalone name still refuses.
	#[test]
	fn standalone_line_scalar_reference_substitutes() {
		let tfns = crate::lang::rules::TemplateFns::new();
		let cfns = crate::lang::rules::ContentFns::new();
		let mut sfns = crate::lang::rules::ScalarFns::new();
		sfns.insert("edition".to_string(), crate::lang::rules::ScalarValue::Number("3".to_string()));
		let binds = crate::lang::rules::Bindings::with_scalars(&tfns, &cfns, &sfns);
		let src = "Some prose above.\n\n#edition\n\nSome prose below.\n";
		let (items, skips) = document_with_templates(src, binds).expect("parses");
		assert!(items.iter().any(|it| matches!(it, Item::Paragraph { runs, .. }
			if matches!(runs.as_slice(), [Inline::Text(t)] if t == "3"))),
			"a standalone-line scalar reference substitutes its value as its own paragraph: {:?}", items);
		assert!(!skips.report().is_some_and(|r| r.contains("edition")),
			"a bound standalone scalar reference is substituted, not recorded as a skip: {:?}", skips.report());
	}

	/// `lorem_words` reproduces `typst 0.15.1`'s `#lorem(n)` verbatim: the classic opening for small counts,
	/// the last word's trailing punctuation replaced by a full stop, a zero count empty, and a huge count
	/// capped at the embedded corpus rather than looping. The pinned strings are the exact oracle output
	/// (checked against the installed typst), so a regression in the corpus or the join reds here.
	#[test]
	fn lorem_matches_the_typst_oracle() {
		assert_eq!(lorem_words(1), "Lorem.");
		assert_eq!(lorem_words(5), "Lorem ipsum dolor sit amet.");
		// Word 20 ("quaerat") carries no source punctuation; the full stop is appended.
		assert_eq!(lorem_words(20),
			"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magnam aliquam quaerat.");
		// Word 49 ("voluptatem,") carries a comma in the source, replaced by the terminal full stop.
		assert!(lorem_words(49).ends_with("transferre in voluptatem."),
			"the terminal comma must become a full stop: {:?}", lorem_words(49));
		assert_eq!(lorem_words(0), "");
		// A count past the corpus is capped, not looped: it returns the whole embedded passage, ending in a
		// full stop, with exactly the corpus's word count.
		let corpus_len = LOREM_CORPUS.split_whitespace().count();
		assert_eq!(lorem_words(100_000).split_whitespace().count(), corpus_len);
		assert!(lorem_words(100_000).ends_with('.'));
	}

	/// An own-line `#pagebreak()`, `#lorem(n)` and `#v(<abs len>)` are set on the block path rather than
	/// tallied as skipped constructs: the page break lowers to an `Item::PageBreak`, the lorem call to a
	/// plain paragraph of the oracle text, and the absolute vertical space to an `Item::Space`. A relative
	/// `#v(2em)` has no running size here, so it stays a visible refusal.
	#[test]
	fn block_builtins_are_set_not_skipped() -> Outcome<()> {
		let src = "Opening prose.\n\n#lorem(5)\n\n#v(12pt)\n\n#pagebreak()\n\nAfter.\n";
		let (items, skips) = res!(document_with_refusals(src));
		assert!(skips.report().is_none(), "no builtin should be tallied as skipped: {:?}", skips.report());
		assert!(items.iter().any(|it| matches!(it, Item::PageBreak { .. })), "the page break is set: {:?}", items);
		assert!(items.iter().any(|it| matches!(it, Item::Space { .. })), "the absolute #v is set: {:?}", items);
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if matches!(runs.as_slice(), [Inline::Text(t)] if t == "Lorem ipsum dolor sit amet."))),
			"the #lorem call sets its oracle text as a paragraph: {:?}", items);
		// A relative `#v(2em)` cannot be resolved here, so it stays a visible refusal rather than a wrong space.
		let (items2, skips2) = res!(document_with_refusals("#v(2em)\n"));
		assert!(!items2.iter().any(|it| matches!(it, Item::Space { .. })), "a relative #v must not be set: {:?}", items2);
		assert!(skips2.report().is_some(), "a relative #v is refused visibly");
		Ok(())
	}

	/// Gathers the plain text of every top-level paragraph, for the trailing-prose and continuation checks.
	#[cfg(test)]
	fn paragraph_text(items: &[Item]) -> String {
		items.iter().filter_map(|it| match it {
			Item::Paragraph { runs, .. } => Some(runs.iter().filter_map(|r| match r {
				Inline::Text(t)	=> Some(t.clone()),
				_				=> None,
			}).collect::<String>()),
			_ => None,
		}).collect::<Vec<_>>().join(" ")
	}

	/// A balanced builtin call with prose trailing its `)` is NOT own-line: it falls through to the existing
	/// visible refusal, which keeps the trailing prose. This is the silent-loss regression the milestone audit
	/// flagged -- `#pagebreak() text`, `#lorem(5) text`, `#v(12pt) text` must not drop the trailing words nor
	/// set the builtin as a block.
	#[test]
	fn trailing_prose_after_a_builtin_is_kept() -> Outcome<()> {
		for src in [
			"#pagebreak() and then more prose.\n",
			"#lorem(5) and then more prose.\n",
			"#v(12pt) and then more prose.\n",
		] {
			let (items, _skips) = res!(document_with_refusals(src));
			assert!(!items.iter().any(|it| matches!(it, Item::PageBreak { .. } | Item::Space { .. })),
				"a builtin with trailing prose must not be set as a block: {:?} -> {:?}", src, items);
			let body = paragraph_text(&items);
			assert!(body.contains("and then more prose"),
				"the trailing prose must survive for {:?}: body {:?}", src, body);
			assert!(!body.contains("Lorem ipsum dolor sit amet"),
				"a trailing-prose #lorem must not expand as a block for {:?}", src);
		}
		Ok(())
	}

	/// An own-line builtin on the line directly after prose, with no blank line between, closes the paragraph
	/// and sets its own block -- exactly as `= heading\n#pagebreak()` and `#section-banner` already do, and as
	/// Typst 0.15.1 renders it. A blank line before the builtin is not required: the earlier deferral silently
	/// swallowed a `#pagebreak()`/`#v()` set directly beneath a paragraph, which is the render bug the live
	/// drive found (a paragraph before the break collapsed the page count to one; a bare heading before it did
	/// not, because a heading line opens no paragraph). The preceding prose survives as its own paragraph. A
	/// builtin with prose on the SAME line (`#lorem(5) more`) is still not own-line -- see
	/// [`trailing_prose_after_a_builtin_is_kept`] -- so inline mid-prose support remains a later unit.
	#[test]
	fn own_line_builtin_beneath_prose_flushes_and_sets() -> Outcome<()> {
		// `#lorem` beneath prose expands its oracle text as a fresh paragraph, and the prose above it is kept.
		let (items, skips) = res!(document_with_refusals("Some opening prose here.\n#lorem(5)\n"));
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if matches!(runs.as_slice(), [Inline::Text(t)] if t == "Lorem ipsum dolor sit amet."))),
			"a #lorem directly beneath prose sets its oracle text as a block: {:?}", items);
		assert!(paragraph_text(&items).contains("Some opening prose here"),
			"the preceding prose is kept: {:?}", items);
		assert!(skips.report().is_none(), "an own-line builtin beneath prose is set, not refused: {:?}", skips.report());

		// `#pagebreak` beneath prose sets the break; the prose is kept.
		let (items, skips) = res!(document_with_refusals("Some opening prose here.\n#pagebreak()\n"));
		assert!(items.iter().any(|it| matches!(it, Item::PageBreak { .. })),
			"a #pagebreak directly beneath prose is set: {:?}", items);
		assert!(paragraph_text(&items).contains("Some opening prose here"), "the preceding prose is kept: {:?}", items);
		assert!(skips.report().is_none(), "the break is set, not refused: {:?}", skips.report());

		// `#v(<abs>)` beneath prose sets the space; the prose is kept.
		let (items, skips) = res!(document_with_refusals("Some opening prose here.\n#v(12pt)\n"));
		assert!(items.iter().any(|it| matches!(it, Item::Space { .. })),
			"an absolute #v directly beneath prose is set: {:?}", items);
		assert!(paragraph_text(&items).contains("Some opening prose here"), "the preceding prose is kept: {:?}", items);
		assert!(skips.report().is_none(), "the space is set, not refused: {:?}", skips.report());
		Ok(())
	}

	/// A `#pagebreak()` nested in a `#styled-box[ ... ]` callout body cannot be honoured -- the box is one keep
	/// unit -- so it is a visible refusal, not a silent drop, and no `Item::PageBreak` survives inside the box.
	#[test]
	fn page_break_inside_a_box_is_refused_not_dropped() -> Outcome<()> {
		let src = "#styled-box[\nInside the callout.\n\n#pagebreak()\n\nStill inside.\n]\n";
		let (items, skips) = res!(document_with_refusals(src));
		assert!(skips.report().map_or(false, |r| r.contains("#pagebreak")),
			"the boxed page break is refused visibly: {:?}", skips.report());
		fn has_page_break(items: &[Item]) -> bool {
			items.iter().any(|it| match it {
				Item::PageBreak { .. }		=> true,
				Item::Box { items, .. }		=> has_page_break(items),
				Item::Scoped { items, .. }	=> has_page_break(items),
				_							=> false,
			})
		}
		assert!(!has_page_break(&items), "no page break may survive inside the box: {:?}", items);
		Ok(())
	}

	/// An ignored keyword argument is refused, not silently set as the wrong thing: `#pagebreak(to: "odd")`
	/// selects a parity target the reader does not model, and `#v(24pt, weak: true)` asks for a collapsing
	/// space it does not model -- each stays a visible refusal rather than a plain break or a fixed space.
	#[test]
	fn unsupported_keyword_args_are_refused() -> Outcome<()> {
		let (items, skips) = res!(document_with_refusals("#pagebreak(to: \"odd\")\n"));
		assert!(!items.iter().any(|it| matches!(it, Item::PageBreak { .. })), "a `to:` pagebreak is not set: {:?}", items);
		assert!(skips.report().map_or(false, |r| r.contains("#pagebreak")), "a `to:` pagebreak is refused: {:?}", skips.report());
		let (items2, skips2) = res!(document_with_refusals("#v(24pt, weak: true)\n"));
		assert!(!items2.iter().any(|it| matches!(it, Item::Space { .. })), "a weak #v is not set: {:?}", items2);
		assert!(skips2.report().map_or(false, |r| r.contains("#v")), "a weak #v is refused: {:?}", skips2.report());
		Ok(())
	}

	/// `_compress-codes`: a run of three or more consecutive same-prefix codes collapses to an en-dash
	/// range, a pair stays expanded, two or fewer codes join unchanged, and an unparseable run passes through.
	#[test]
	fn claim_codes_compress_consecutive_runs() {
		let display = |s: &str| parse_inlines(s).into_iter()
			.find_map(|r| match r { Inline::MarginNote { display, .. } => Some(display), _ => None })
			.unwrap_or_default();
		assert_eq!(display("x#claim-label(<B1>, <B2>, <B3>, <B4>)"), "B1\u{2013}4");	// B1–4
		assert_eq!(display("x#claim-label(<A1>, <A2>)"), "A1 A2");
		assert_eq!(display("x#claim-label(<CD14>, <CD15>, <CD4>)"), "CD14 CD15 CD4");
		assert_eq!(display("x#claim-label(<LS8>)"), "LS8");
	}

	/// A line that opens with a claim marker is prose, not a standalone call the line scanner skips, so
	/// the sentence that follows the marker is set rather than dropped with it.
	#[test]
	fn line_leading_claim_marker_is_prose() {
		assert!(is_inline_call("claim-label"));
		assert!(is_inline_call("claim-refs"));
		assert!(code_skip("#claim-label(<CD18>). Equilibrium appropriation follows.").is_none());
	}

	/// A claim reference in a context the layout does not gather into the reverse claim index -- here a
	/// heading title -- is recorded as a refusal rather than dropped silently, while the same reference in a
	/// body paragraph (which IS gathered) draws no refusal. Guards the silent-loss path the audit flagged.
	#[test]
	fn claim_ref_in_a_non_body_context_is_refused_not_dropped() {
		let (_items, skips) = document_with_refusals("= Heading #claim-refs(<Z9>) here\n\nBody text follows.\n").expect("parse");
		assert!(skips.sites().iter().any(|s| s.name.contains("claim reference") && s.name.contains("heading")),
			"a claim reference in a heading title must be a refusal: {:?}", skips.sites());
		// A claim reference in a body paragraph is gathered into the index, so it is NOT refused.
		let (_i2, skips2) = document_with_refusals("Body carrying a reference#claim-refs(<Z9>) here.\n").expect("parse");
		assert!(!skips2.sites().iter().any(|s| s.name.contains("claim reference")),
			"a body claim reference is indexed, not refused: {:?}", skips2.sites());
	}

	/// A line-leading `#padded-image(...)` (a section opener's logo) is set as an [`Item::Image`] carrying
	/// its path and scale, not skipped as a template call and not wrapped in a numbered figure.
	#[test]
	fn line_leading_padded_image_reads_as_image() -> Outcome<()> {
		let (items, _skips) = res!(document_with_refusals(
			"= Pearl\n\n#padded-image(\"assets/svg/pearlite_logo_text_right.svg\", scale: 45%)\n\nPearl is the format.\n"));
		let img = res!(items.iter().find_map(|it| match it {
			Item::Image { path, scale, .. }	=> Some((path.clone(), *scale)),
			_								=> None,
		}).ok_or_else(|| err!("no Item::Image was produced for the standalone padded-image"; Test, Bug)));
		assert_eq!(img.0, "assets/svg/pearlite_logo_text_right.svg", "the image path is read");
		assert_eq!(img.1, Some(0.45), "the padded-image scale is read as a fraction");
		// The line must not have been swallowed as a skipped construct, nor turned into a figure.
		assert!(!items.iter().any(|it| matches!(it, Item::Figure { .. })), "a section logo is not a figure");
		Ok(())
	}

	/// A line-leading `#section-banner("logo")` (a documentation section opener) is read as an
	/// [`Item::SectionBanner`] carrying its logo path, not skipped as a template call and not confused with a
	/// plain `#image`.
	#[test]
	fn line_leading_section_banner_reads_as_banner() -> Outcome<()> {
		let (items, skips) = res!(document_with_refusals(
			"#section-banner(\"assets/svg/fe2o3_logo_text_right.svg\")\n\n= Steel Server\n\nSteel is the server.\n"));
		let path = res!(items.iter().find_map(|it| match it {
			Item::SectionBanner { path, .. }	=> Some(path.clone()),
			_								=> None,
		}).ok_or_else(|| err!("no Item::SectionBanner was produced for the standalone section-banner"; Test, Bug)));
		assert_eq!(path, "assets/svg/fe2o3_logo_text_right.svg", "the banner logo path is read");
		// The call must not have been swallowed as a skipped construct, nor read as a plain centred image.
		assert!(!skips.entries().iter().any(|(name, _)| name == "#section-banner"),
			"a section banner must not be reported as a skipped construct");
		assert!(!items.iter().any(|it| matches!(it, Item::Image { .. })), "a section banner is not a plain image");
		Ok(())
	}

	/// A line-leading `#styled-box[...]` callout, its body opening on the marker line and closing on a later
	/// one, is gathered whole and read as an [`Item::Box`] holding the re-parsed body -- not skipped as an
	/// unbalanced standalone call, which would drop the callout's text. The construct is set, so it is not
	/// reported as a skipped construct.
	#[test]
	fn line_leading_styled_box_reads_as_box() -> Outcome<()> {
		let (items, skips) = res!(document_with_refusals(
			"Lead prose.\n\n#styled-box[\n*Principle.* Every participant is accountable.\n]\n\nTrailing prose.\n"));
		let inner = res!(items.iter().find_map(|it| match it {
			Item::Box { items, .. }	=> Some(items.clone()),
			_						=> None,
		}).ok_or_else(|| err!("no Item::Box was produced for the standalone styled-box"; Test, Bug)));
		// The body re-parses to a paragraph, and its lead-in bold survives as a strong run.
		let has_para = inner.iter().any(|it| matches!(it, Item::Paragraph { .. }));
		assert!(has_para, "the styled-box body must re-parse to a paragraph, got: {:?}", inner);
		let has_strong = inner.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if runs.iter().any(|r| matches!(r, Inline::Strong(t) if t == "Principle."))));
		assert!(has_strong, "the body's bold lead-in must survive, got: {:?}", inner);
		// The callout must not have been swallowed as a skipped construct.
		assert!(!skips.entries().iter().any(|(name, _)| name == "#styled-box"),
			"a styled-box must not be reported as a skipped construct");
		// The prose either side of the callout still sets.
		assert!(items.iter().any(|it| matches!(it, Item::Paragraph { runs, .. }
			if runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("Lead prose."))))),
			"prose before the callout is dropped");
		Ok(())
	}

	/// A call to a bound `#let` furniture function -- `#pr-note[ ... ]` -- is gathered whole and expanded
	/// into an [`Item::Box`] carrying the definition's lowered patch (a transparent-wash, asymmetric-inset
	/// block), its `[ ... ]` body re-parsed into the box. The call is set, not skipped, so it is not tallied;
	/// an unbound `#name[...]` (no definition in scope) still falls through to be reported as a skip.
	#[test]
	fn pr_note_call_expands_to_a_box_and_is_not_skipped() -> Outcome<()> {
		let def = "#let pr-note(body) = block(inset: (left: 1.2em, right: 0.6em), above: 0.9em, below: 1.1em, \
{ set text(size: 0.88em); set par(spacing: 0.55em, first-line-indent: 0em); body })\n";
		let mut tfns = crate::lang::rules::TemplateFns::new();
		crate::lang::rules::collect_template_fns(def, crate::ir::Sp::from_pt(10.0),
			&crate::lang::rules::Palette::new(), &mut tfns);
		assert!(tfns.contains_key("pr-note"), "the definition is collected");

		let src = "Lead prose.\n\n#pr-note[\n*Baseline:* one measure.\n\nA second paragraph.\n]\n\nTrailing prose.\n";
		let (items, skips) = res!(document_with_templates(src, crate::lang::rules::Bindings::new(&tfns, &crate::lang::rules::ContentFns::new())));
		let (inner, patch) = res!(items.iter().find_map(|it| match it {
			Item::Box { items, patch, .. }	=> Some((items.clone(), patch.clone())),
			_							=> None,
		}).ok_or_else(|| err!("no Item::Box was produced for the pr-note call"; Test, Bug)));
		// The box carries the pr-note geometry: an asymmetric inset and a transparent (no-wash) fill.
		assert_eq!(patch.callout.inset_left, Some(crate::ir::Sp::from_pt(12.0)), "left inset resolved at 10pt body");
		assert_eq!(patch.callout.fill.map(|c| c.a), Some(0), "no fill -- a plain indented block");
		assert_eq!(patch.text.body_size, Some(crate::ir::Sp::from_pt(8.8)), "the body sets at 0.88em");
		// The body re-parses to paragraphs, its bold lead-in surviving as a strong run.
		assert!(inner.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if runs.iter().any(|r| matches!(r, Inline::Strong(t) if t == "Baseline:")))),
			"the body's bold lead-in must survive, got: {:?}", inner);
		// The call is not tallied as a skipped construct, and the surrounding prose still sets.
		assert!(!skips.entries().iter().any(|(name, _)| name == "#pr-note"),
			"a bound furniture call must not be reported as a skip");
		assert!(items.iter().any(|it| matches!(it, Item::Paragraph { runs, .. }
			if runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("Lead prose."))))),
			"prose before the call is dropped");

		// With no definition in scope, the same call is left to be tallied as a skip, unchanged.
		let (_it2, skips2) = res!(document_with_refusals(src));
		assert!(skips2.entries().iter().any(|(name, _)| name == "#pr-note"),
			"an unbound furniture call still reports as a skip");
		Ok(())
	}

	/// A `#aside-box(title: [...])[ ... ]` call expands into a washed box carrying the definition's fill and
	/// left stroke, its `title:` argument set as a leading bold paragraph ahead of the body, and it is not
	/// tallied as a skip. A bound call with no `[ ... ]` body (an argument-only `#aside-box(...)`) is TALLIED
	/// as a skip rather than dropped silently.
	#[test]
	fn aside_box_call_expands_with_title_and_stroke() -> Outcome<()> {
		let def = "#let aside-box(title: none, body) = box(width: 100%, inset: 8pt, \
fill: colours.yellow.lighten(50%), radius: 4pt, stroke: (left: 2pt + colours.yellow.darken(20%)), \
[#text(weight: \"bold\", size: 0.85em)[#title] #text(size: 0.85em)[#body]])\n";
		let mut palette = crate::lang::rules::Palette::new();
		crate::lang::rules::collect_palette("#let colours = (yellow: rgb(\"#f0f600\"),)\n", &mut palette);
		let mut tfns = crate::lang::rules::TemplateFns::new();
		crate::lang::rules::collect_template_fns(def, crate::ir::Sp::from_pt(11.0), &palette, &mut tfns);
		let tf = res!(tfns.get("aside-box").ok_or_else(|| err!("aside-box collected"; Test, Bug)));
		assert!(tf.patch.callout.fill.map(|c| c.a) == Some(255), "the yellow fill resolved (opaque)");
		assert!(tf.patch.callout.stroke_left_w.is_some(), "the left stroke width is set");

		let src = "Lead.\n\n#aside-box(title: [The welfare theorems])[\nMarket efficiency proved.\n]\n\nTail.\n";
		let (items, skips) = res!(document_with_templates(src, crate::lang::rules::Bindings::new(&tfns, &crate::lang::rules::ContentFns::new())));
		let inner = res!(items.iter().find_map(|it| match it {
			Item::Box { items, .. }	=> Some(items.clone()),
			_					=> None,
		}).ok_or_else(|| err!("no Item::Box for the aside-box call"; Test, Bug)));
		// The first item is the bold title (in a size scope), then the body.
		let has_title = inner.iter().any(|it| match it {
			Item::Scoped { items, .. }	=> items.iter().any(|p| matches!(p,
				Item::Paragraph { runs, .. } if runs.iter().any(|r| matches!(r, Inline::Strong(t) if t.contains("welfare"))))),
			Item::Paragraph { runs, .. }	=> runs.iter().any(|r| matches!(r, Inline::Strong(t) if t.contains("welfare"))),
			_						=> false,
		});
		assert!(has_title, "the title is set as a leading bold paragraph, got: {:?}", inner);
		assert!(!skips.entries().iter().any(|(n, _)| n == "#aside-box"), "the call is not a skip");

		// A bound call with no `[body]` is tallied as a skip, not dropped.
		let (_it, skips2) = res!(document_with_templates("#aside-box(title: [X])\n", crate::lang::rules::Bindings::new(&tfns, &crate::lang::rules::ContentFns::new())));
		assert!(skips2.entries().iter().any(|(n, _)| n == "#aside-box"),
			"an argument-only bound call with no body is tallied as a skip");
		Ok(())
	}

	/// A `#columns[...]` body's own top-level `#set` declarations scope to the spliced subtree (H1's
	/// flat-splice sibling): the reader nests the spliced items inside one `Item::Scoped` carrying the
	/// lowered patch. A columns body that declares nothing splices in flat, with no scope.
	#[test]
	fn columns_body_set_scopes_the_spliced_subtree() -> Outcome<()> {
		let (items, _skips) = res!(document_with_refusals(
			"#columns(2)[\n#set text(size: 20pt)\n\nScoped body.\n]\n"));
		let (patch, inner) = res!(items.iter().find_map(|it| match it {
			Item::Scoped { patch, items }	=> Some((patch.clone(), items)),
			_								=> None,
		}).ok_or_else(|| err!("no Item::Scoped was produced for a columns body with a #set"; Test, Bug)));
		assert_eq!(patch.text.body_size, Some(crate::ir::Sp::from_pt(20.0)),
			"the columns body's #set text(size:) did not lower into the scope patch");
		assert!(!inner.is_empty(), "the scope must carry the body's items");

		// A columns body that declares nothing splices in flat, with no scope.
		let (plain, _) = res!(document_with_refusals("#columns(2)[\nPlain body.\n]\n"));
		assert!(!plain.iter().any(|it| matches!(it, Item::Scoped { .. })),
			"a columns body with no #set must not be wrapped in a scope");
		Ok(())
	}

	/// H2: a lowerable `#set` that lowers to nothing (an unrecognised argument) is recorded as a visible
	/// refusal named for the set, rather than silently dropped, while one that fully lowers records none.
	#[test]
	fn unconsumed_set_is_recorded_as_a_refusal() -> Outcome<()> {
		let (_items, skips) = res!(document_with_refusals("#set text(lang: \"de\")\n\nBody.\n"));
		assert!(skips.entries().iter().any(|(name, _)| name == "#set text"),
			"an unconsumed #set must be recorded as a refusal, got: {:?}", skips.entries());

		let (_items2, skips2) = res!(document_with_refusals("#set text(size: 12pt)\n\nBody.\n"));
		assert!(!skips2.entries().iter().any(|(name, _)| name == "#set text"),
			"a fully-lowered #set must not be recorded as a refusal, got: {:?}", skips2.entries());
		Ok(())
	}

	/// `#emph[...]` is the call form of `_..._`: it yields an [`Inline::Emph`] run with the same inner
	/// text, and nothing raw leaks.
	#[test]
	fn emph_call_reads_as_emphasis() {
		let runs = parse_inlines("The cube asks #emph[who] does the extracting.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Emph(t) if t == "who")),
			"emph run missing: {:?}", runs);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#emph"))),
			"raw #emph leaked: {:?}", runs);
		// A call carrying its own markup expands the same way `_..._` does.
		let nested = parse_inlines("#emph[the #idx[Harvard Business Review] weekly]");
		assert!(nested.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#emph") || t.contains("#idx"))),
			"raw markup leaked from nested emph: {:?}", nested);
	}

	/// An index marker carries a sort key distinct from its display: `#idx-as[Abbott, Andrew][Andrew Abbott]`
	/// files under the surname but shows "Andrew Abbott" (in the body and on the index page), and `#idx[_x_]`
	/// keeps its emphasis as an [`Inline::Emph`] display run while its sort key is the flattened plain text --
	/// so the index sets the display, not the sort key, and an emphasised entry italicises rather than printing
	/// literal underscores.
	#[test]
	fn index_marker_splits_sort_key_from_styled_display() {
		let runs = parse_inlines("The sociologist #idx-as[Abbott, Andrew][Andrew Abbott] wrote widely.");
		let mut found = false;
		for r in &runs {
			if let Inline::Index { term, sub, display, .. } = r {
				assert_eq!(term, "Abbott, Andrew", "sort key wrong: {:?}", runs);
				assert!(sub.is_none());
				assert!(matches!(display.as_slice(), [Inline::Text(t)] if t == "Andrew Abbott"),
					"display wrong: {:?}", display);
				found = true;
			}
		}
		assert!(found, "no index marker found: {:?}", runs);
		// The visible display "Andrew Abbott" is set in the body beside the marker.
		assert!(runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("Andrew Abbott"))),
			"body display missing: {:?}", runs);

		// An emphasised entry: the sort key is flattened, the display keeps the emphasis.
		let ital = parse_inlines("the ruling #idx[_Browder v. Gayle_] held.");
		let mut seen = false;
		for r in &ital {
			if let Inline::Index { term, display, .. } = r {
				assert_eq!(term, "Browder v. Gayle", "italic sort key not flattened: {:?}", ital);
				assert!(matches!(display.as_slice(), [Inline::Emph(t)] if t == "Browder v. Gayle"),
					"italic display lost its emphasis: {:?}", display);
				seen = true;
			}
		}
		assert!(seen, "no italic index marker found: {:?}", ital);
	}

	/// `#strong[...]` and `#strong("...")` are the call forms of `*...*`: both yield an [`Inline::Strong`]
	/// run rather than a skip, and nothing raw leaks. A paren argument the reader cannot evaluate -- a
	/// bare identifier here -- keeps the current refusal rather than guessing at its text.
	#[test]
	fn strong_call_reads_as_bold() -> Outcome<()> {
		let bracket = parse_inlines("You want: #strong[the short version] first.");
		assert!(bracket.iter().any(|r| matches!(r, Inline::Strong(t) if t == "the short version")),
			"strong run missing from bracket form: {:?}", bracket);
		assert!(bracket.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#strong"))),
			"raw #strong leaked: {:?}", bracket);

		let paren = parse_inlines("#strong(\"the short version\") first.");
		assert!(paren.iter().any(|r| matches!(r, Inline::Strong(t) if t == "the short version")),
			"strong run missing from paren form: {:?}", paren);

		// A call carrying its own markup expands the same way `*...*` does.
		let nested = parse_inlines("#strong[the #idx[Harvard Business Review] weekly]");
		assert!(nested.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#strong") || t.contains("#idx"))),
			"raw markup leaked from nested strong: {:?}", nested);

		// A paren argument that is not a plain string is left for the generic call handler, so it is
		// still tallied as a skip rather than silently guessed at.
		let (_it, skips) = res!(document_with_refusals("#strong(ident)\n\nBody.\n"));
		assert!(skips.entries().iter().any(|(n, _)| n == "#strong"),
			"an unresolvable #strong(...) argument must still be recorded as a refusal, got: {:?}",
			skips.entries());
		Ok(())
	}

	/// Emphasis nested one level -- `*_x_*` or `_*x*_` -- collapses to a single [`Inline::BoldItalic`]
	/// run rather than dropping the outer face, so a bold-italic term sets in the bold-italic face in
	/// prose, a footnote or a table cell alike.
	#[test]
	fn nested_emphasis_reads_as_bold_italic() {
		for src in ["here *_both_* faces", "here _*both*_ faces"] {
			let runs = parse_inlines(src);
			assert!(runs.iter().any(|r| matches!(r, Inline::BoldItalic(t) if t == "both")),
				"expected a bold-italic run from {:?}, got {:?}", src, runs);
			assert!(runs.iter().all(|r| !matches!(r, Inline::Emph(t) if t == "both")),
				"outer face dropped to plain emph in {:?}: {:?}", src, runs);
		}
		// A lone `*strong*` or `_emph_` still yields its single face, unchanged.
		assert!(parse_inlines("just *strong* here").iter().any(|r| matches!(r, Inline::Strong(t) if t == "strong")));
		assert!(parse_inlines("just _emph_ here").iter().any(|r| matches!(r, Inline::Emph(t) if t == "emph")));
	}

	/// A `(col, row) => ...` alignment closure is evaluated per cell: a row-keyed closure centres the
	/// header and flushes the body left; a per-column closure honours each column's own alignment,
	/// including an `or` over several columns.
	#[test]
	fn align_closure_evaluates_per_cell() {
		let row_keyed = parse_align("(col, row) => { if row == 0 { center } else { left } }");
		match row_keyed {
			AlignSpec::Closure(cl) => {
				assert_eq!(cl.align_at(0, 0), Align::Centre);	// header, column 0
				assert_eq!(cl.align_at(0, 1), Align::Left);	// body, column 0 -- was wrongly centred before
				assert_eq!(cl.align_at(2, 3), Align::Left);
			},
			other => panic!("expected a closure, got {:?}", other),
		}
		let per_col = parse_align("(col, row) => { if row == 0 { center } else if col == 0 or col == 4 { left } else { center } }");
		match per_col {
			AlignSpec::Closure(cl) => {
				assert_eq!(cl.align_at(0, 1), Align::Left);
				assert_eq!(cl.align_at(4, 1), Align::Left);
				assert_eq!(cl.align_at(1, 1), Align::Centre);
				assert_eq!(cl.align_at(3, 2), Align::Centre);
			},
			other => panic!("expected a closure, got {:?}", other),
		}
		// A tuple pick indexed by the column parameter.
		let tuple = parse_align("(x, y) => (left, center, right).at(x)");
		match tuple {
			AlignSpec::Closure(cl) => {
				assert_eq!(cl.align_at(0, 5), Align::Left);
				assert_eq!(cl.align_at(1, 5), Align::Centre);
				assert_eq!(cl.align_at(2, 5), Align::Right);
			},
			other => panic!("expected a closure, got {:?}", other),
		}
	}

	/// `#super[...]` yields an [`Inline::Super`] run of its content, in both the bracket and the string
	/// argument forms, and never leaves raw source behind.
	#[test]
	fn super_call_reads_as_superscript() {
		let runs = parse_inlines("The area is 10#super[6] units, split#super[†] on the case.");
		let sups: Vec<&String> = runs.iter().filter_map(|r| match r {
			Inline::Super(t) => Some(t),
			_ => None,
		}).collect();
		assert_eq!(sups, vec!["6", "†"], "unexpected superscript runs: {:?}", runs);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#super"))),
			"raw #super leaked: {:?}", runs);
		// The string-argument form reads the same, its quotes stripped.
		let quoted = parse_inlines("x#super(\"2\")");
		assert!(quoted.iter().any(|r| matches!(r, Inline::Super(t) if t == "2")),
			"quoted super run missing: {:?}", quoted);
	}

	/// `#sub[...]` yields an [`Inline::Sub`] run of its content, as `CO#sub[2]` sets it in Lucronics, and
	/// never leaves raw source behind.
	#[test]
	fn sub_call_reads_as_subscript() {
		let runs = parse_inlines("CO#sub[2] scrubber and H#sub[2]O#sub[2] both drop.");
		let subs: Vec<&String> = runs.iter().filter_map(|r| match r {
			Inline::Sub(t) => Some(t),
			_ => None,
		}).collect();
		assert_eq!(subs, vec!["2", "2", "2"], "unexpected subscript runs: {:?}", runs);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#sub"))),
			"raw #sub leaked: {:?}", runs);
		// The string-argument form reads the same, its quotes stripped.
		let quoted = parse_inlines("x#sub(\"2\")");
		assert!(quoted.iter().any(|r| matches!(r, Inline::Sub(t) if t == "2")),
			"quoted sub run missing: {:?}", quoted);
	}

	/// The `.enumerate().map(((idx, row)) => { if COND { row } else { table.cell(colspan: n)[...] } })
	/// .flatten()` row-remap idiom -- Lucronics' E. coli comparison uses it to merge a section-heading row
	/// into one bold spanning cell while an ordinary row passes through -- resolves through the spread, not
	/// just the untransformed array. This is the reduced shape of the real table (3 columns, one heading
	/// row) rather than the full 7-column original.
	#[test]
	fn table_spread_map_merges_heading_rows_into_bold_spanning_cells() {
		let let_src = "#let data = (\n  ([Head A], [Head B], [Head C]),\n  ([Section], [], []),\n  ([Item one], [x], [y]),\n)\n";
		let mut arrays = HashMap::new();
		arrays.insert("data".to_string(), parse_let_array(let_src));

		let table_src = "\n  columns: 3,\n  ..data.enumerate().map(((idx, row)) => {\n    if idx == 0 or row.at(0) != [Section] {\n      row\n    } else {\n      table.cell(colspan: 3)[#strong(row.at(0))]\n    }\n  }).flatten()\n";
		let spec = parse_table_spec(table_src, &arrays, None).expect("the spread must resolve to a table");
		assert_eq!(spec.cells.len(), 9, "3 rows x 3 columns, the merged row padded to width");
		// Row 0 (the header) and row 2 (an ordinary item) pass through unchanged.
		assert!(matches!(spec.cells[0].as_slice(), [Inline::Text(t)] if t == "Head A"));
		assert!(matches!(spec.cells[6].as_slice(), [Inline::Text(t)] if t == "Item one"));
		// Row 1 (the section heading) is merged into one bold cell, padded to the column count.
		assert!(matches!(spec.cells[3].as_slice(), [Inline::Strong(t)] if t == "Section"),
			"expected the merged heading cell, got {:?}", spec.cells[3]);
		assert!(matches!(spec.cells[4].as_slice(), [Inline::Text(t)] if t.is_empty()), "padding cell after the span");
		assert!(matches!(spec.cells[5].as_slice(), [Inline::Text(t)] if t.is_empty()), "padding cell after the span");
	}

	/// A citation nested in emphasis keeps its own [`Inline::Cite`] run rather than leaking its source,
	/// while the surrounding words take the emphasis face.
	#[test]
	fn cite_survives_inside_emphasis() {
		let runs = parse_inlines("the classic _early work #cite(<coase1937nature>) here_.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Cite(k) if k == &vec!["coase1937nature".to_string()])),
			"cite run missing: {:?}", runs);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#cite"))),
			"raw #cite leaked: {:?}", runs);
	}

	/// `#link("url")[text]` sets the link text in the running line and drops the URL, so no raw `#link`
	/// leaks; a bare `#link("url")` with no bracket sets the URL as its own text.
	#[test]
	fn link_call_renders_text_not_markup() {
		let runs = parse_inlines("See #link(\"https://aistatement.com\")[Centre for AI Safety, May 2023] on risk.");
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#link") || t.contains("http"))),
			"raw link markup leaked: {:?}", runs);
		let joined = flatten_markup("See #link(\"https://aistatement.com\")[Centre for AI Safety, May 2023] on risk.");
		assert_eq!(joined, "See Centre for AI Safety, May 2023 on risk.");
		// The label-destination form and the bare no-body form both set text without leaking.
		assert_eq!(flatten_markup("read #link(<intro>)[the opening]"), "read the opening");
		assert_eq!(flatten_markup("at #link(\"elearnity.io\")"), "at elearnity.io");
		// A line-leading link is prose the inline scanner reads, not a standalone call the line scanner drops.
		assert!(is_inline_call("link"));
		assert!(code_skip("#link(\"https://x.io\")[click]").is_none());
	}

	/// Typst's smartypants: `--` becomes an en dash and `---` an em dash in ordinary prose, matched
	/// longest-first, but a raw code span, a maths span and an escaped hyphen are all left untouched.
	#[test]
	fn dash_runs_become_en_and_em_dashes_in_prose() {
		assert_eq!(flatten_markup("a--b"), "a\u{2013}b");
		assert_eq!(flatten_markup("a---b"), "a\u{2014}b");
		// Four hyphens is an em dash plus a literal hyphen, greedy left to right, not two en dashes.
		assert_eq!(flatten_markup("a----b"), "a\u{2014}-b");
		assert_eq!(flatten_markup("Council of Trent (1545--1563)"), "Council of Trent (1545\u{2013}1563)");
		// A raw code span keeps its `--` literal: the inline scanner claims `` ` `` before this fallback
		// is ever reached, so the substitution never sees the span's characters.
		let runs = parse_inlines("see `a--b` here");
		assert!(runs.iter().any(|r| matches!(r, Inline::Code(t) if t == "a--b")),
			"raw span lost or converted: {:?}", runs);
		// A maths span keeps its `-` as subtraction, for the same reason.
		let runs = parse_inlines("$a-b$");
		assert!(matches!(runs.as_slice(), [Inline::Math(_)]), "maths span not read as maths: {:?}", runs);
		// A `\-` escape is consumed on its own, so it never joins the hyphen after it into a run: the
		// pair stays two literal ASCII hyphens rather than folding to the single en-dash glyph `a--b`
		// (unescaped) becomes.
		assert_eq!(flatten_markup("a\\--b"), "a--b");
		// `#link`'s destination is dropped entirely (austenite renders no clickable URL), so a `--` inside
		// one is never seen at all -- the strictest match to Typst leaving an auto-linked URL unconverted.
		assert_eq!(flatten_markup("see #link(\"https://x--y.com\")[the site]"), "see the site");
	}

	/// The same fallback trivially covers Typst's other markup substitution on dots: three or more become
	/// an ellipsis, fewer stay literal, and the rule is still skipped inside code and maths.
	#[test]
	fn dot_runs_become_ellipsis_in_prose() {
		assert_eq!(flatten_markup("wait..."), "wait\u{2026}");
		assert_eq!(flatten_markup("a....b"), "a\u{2026}.b");
		assert_eq!(flatten_markup("a..b"), "a..b");	// two dots: no defined symbol, left alone
		assert_eq!(flatten_markup("v1.2.3"), "v1.2.3");	// scattered single dots: unaffected
		let runs = parse_inlines("`a...b`");
		assert!(matches!(runs.as_slice(), [Inline::Code(t)] if t == "a...b"), "raw span converted: {:?}", runs);
	}

	/// The term-dictionary aliases set their argument text with the styling of their `gs` siblings: `g`/`gi`
	/// a first-use glossary term, `t`/`tcap` plain text, and none of them leak raw markup. A line opening
	/// with one is prose, not a skipped standalone call.
	/// Installs a fixed term dictionary so the term-dictionary family resolves deterministically. The map
	/// is a process-global shared across the parallel tests, so every term-dependent test installs the
	/// same one and their order cannot matter.
	fn install_test_terms() {
		let mut m = HashMap::new();
		m.insert("org".to_string(),			"Elearnity Pty Ltd".to_string());
		m.insert("org_short".to_string(),	"Elearnity".to_string());
		m.insert("website".to_string(),		"elearnity.oxegen.io".to_string());
		m.insert("iniverse".to_string(),	"iniverse".to_string());
		set_term_dict(m).expect("install test term dict");
	}

	#[test]
	fn term_dict_aliases_render_without_leaking() {
		install_test_terms();
		// `iniverse` translates to itself, so the display equals the key here whether or not a map is set.
		let runs = parse_inlines("Call it the #g[iniverse], your inner universe.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Glossary { term, display } if term == "iniverse" && display == "iniverse")),
			"glossary alias missing: {:?}", runs);
		// A key that differs from its value now sets the value, not the key.
		assert_eq!(flatten_markup("Visit #t[website] today"), "Visit elearnity.oxegen.io today");
		// A key absent from the dictionary falls back to its own text, capitalised for the `-cap` form.
		assert_eq!(flatten_markup("#tcap[donate] to help"), "Donate to help");
		assert!(is_inline_call("g") && is_inline_call("t") && is_inline_call("graw"));
		assert!(code_skip("#t[website]").is_none());
	}

	/// The term-dictionary family sets a key's value: `t`/`graw` plain, `tcap` capitalised, `g` bold-italic
	/// on first use keyed by the key; an unknown key falls back to the key text and is recorded, never a
	/// panic (the template panics on a miss, the reader must not).
	#[test]
	fn term_dict_resolves_key_to_value() {
		install_test_terms();
		assert_eq!(flatten_markup("Visit #t[website]"), "Visit elearnity.oxegen.io");
		assert_eq!(flatten_markup("#graw[org]"), "Elearnity Pty Ltd");
		let runs = parse_inlines("The #g[org] view.");
		assert!(runs.iter().any(|r| matches!(r, Inline::Glossary { term, display }
				if term == "org" && display == "Elearnity Pty Ltd")),
			"g did not translate the key to its value: {:?}", runs);
		// An unknown key: the key text stands and the miss is recorded on the skip tally.
		let mut skips = Refusals::default();
		let runs = parse_inlines_in("A #t[nonesuch] term.", Span::new(0, 0), &mut skips);
		assert!(runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("nonesuch"))),
			"unknown term-dict key did not fall back to its text: {:?}", runs);
		assert_eq!(skips.total(), 1, "an unknown term-dict key was not recorded");
	}

	/// A term-dictionary lookup inside a `#let` content-function body, keyed on the function's own parameter
	/// (`#let cite-term(w) = [Learn about #t(w).]`), resolves against the argument the caller passed --
	/// `#cite-term("website")` must look up "website", not the literal parameter name "w" -- so substitution
	/// must run before the nested `#t` call is read. Reverting the ordering fix (substituting only bare
	/// `#param` markup references, never a bare identifier inside a call's argument list) reproduces exactly
	/// the reported failure: the key "w" is looked up, is not in the dictionary, and the fallback plus a
	/// recorded skip fire instead of the resolved value.
	#[test]
	fn term_dict_resolves_inside_a_content_fn_body_after_param_substitution() {
		install_test_terms();
		let mut cfns = crate::lang::rules::ContentFns::new();
		cfns.insert("cite-term".to_string(), crate::lang::rules::ContentFn {
			params:	vec!["w".to_string()],
			body:	"Learn about #t(w).".to_string(),
		});
		let tfns	= crate::lang::rules::TemplateFns::new();
		let binds	= crate::lang::rules::Bindings::new(&tfns, &cfns);
		let (items, skips) = document_with_templates("#cite-term(\"website\")\n", binds).expect("parse");
		let resolved = items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if runs.iter().any(|r|
				matches!(r, Inline::Text(t) if t.contains("elearnity.oxegen.io")))));
		assert!(resolved,
			"the content-fn's own parameter did not resolve as the term-dict key: {:?}", items);
		assert_eq!(skips.total(), 0, "a resolved key must not be recorded as an unknown term-dict miss: {:?}", skips);
	}

	/// The ordering fix is scoped to a call's argument list, not to ordinary prose: a parameter's name
	/// appearing as a genuine word in the body's running text (outside any call) is left exactly as written,
	/// since a bare word in markup mode is prose, never a code-mode variable reference.
	#[test]
	fn call_arg_substitution_does_not_touch_ordinary_prose() {
		let cf = crate::lang::rules::ContentFn {
			params:	vec!["w".to_string()],
			body:	"The word w on its own is prose, not #t(w).".to_string(),
		};
		let expanded = expand_content_body(&cf, &["website".to_string()]);
		assert_eq!(expanded, "The word w on its own is prose, not #t(\"website\").",
			"prose text was wrongly substituted, or the call argument was not: {:?}", expanded);
	}

	/// A blank line between numbered items does not restart the enum: Typst continues the numbering across
	/// the gap, so the items form one list. Real content between two lists still starts a fresh one.
	#[test]
	fn blank_line_between_enum_items_continues_one_list() {
		let src = "+ first item\n\n+ second item\n\n+ third item\n";
		let (items, _) = document_with_refusals(src).expect("parse");
		let lists: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::List { .. })).collect();
		assert_eq!(lists.len(), 1, "blank lines split the enum: {:?}", items);
		match lists[0] {
			Item::List { ordered, items, .. } => {
				assert!(*ordered, "the continued list lost its ordered kind");
				assert_eq!(items.len(), 3, "the enum dropped items across the blanks: {:?}", items);
			},
			_ => unreachable!(),
		}
		// A paragraph between two lists still restarts, so genuinely separate lists are not merged.
		let src2 = "+ a\n\n+ b\n\nA paragraph between.\n\n+ c\n";
		let (items2, _) = document_with_refusals(src2).expect("parse");
		let lists2 = items2.iter().filter(|it| matches!(it, Item::List { .. })).count();
		assert_eq!(lists2, 2, "prose between two lists did not restart them: {:?}", items2);
	}

	/// An indented `-` sub-bullet between two `+` steps nests under the step it follows rather than closing
	/// the enum: the ordered list stays one list of three items, and the sub-bullet hangs under the first.
	#[test]
	fn indented_sub_bullet_nests_and_enum_continues() {
		let src = "+ step one\n  - a sub point\n  - another sub point\n+ step two\n+ step three\n";
		let (items, _) = document_with_refusals(src).expect("parse");
		let lists: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::List { .. })).collect();
		assert_eq!(lists.len(), 1, "the sub-bullet split the enum into several lists: {:?}", items);
		match lists[0] {
			Item::List { ordered, items, .. } => {
				assert!(*ordered, "the parent list lost its ordered kind");
				assert_eq!(items.len(), 3, "the enum did not keep three steps: {:?}", items);
				// The sub-bullets hang under the first step, as an unordered child list of two items.
				assert_eq!(items[0].children.len(), 1, "the first step lost its sub-list: {:?}", items[0]);
				match &items[0].children[0] {
					Item::List { ordered: cord, items: citems, .. } => {
						assert!(!*cord, "the sub-list should be unordered");
						assert_eq!(citems.len(), 2, "the sub-list dropped an item: {:?}", citems);
					},
					other => panic!("the child was not a nested list: {:?}", other),
				}
				assert!(items[1].children.is_empty(), "step two wrongly gained children");
			},
			_ => unreachable!(),
		}
	}

	/// Two levels of indentation parse to two levels of nesting: a `-` under a `+`, and a deeper `-` under
	/// that `-`, so the tree is enum -> bullet -> bullet.
	#[test]
	fn two_level_nesting_parses_to_two_levels() {
		let src = "+ outer step\n  - middle bullet\n    - inner bullet\n+ next step\n";
		let (items, _) = document_with_refusals(src).expect("parse");
		let lists: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::List { .. })).collect();
		assert_eq!(lists.len(), 1, "the deep nesting split the list: {:?}", items);
		match lists[0] {
			Item::List { items, .. } => {
				assert_eq!(items.len(), 2, "the outer enum did not keep two steps: {:?}", items);
				let mid = &items[0].children;
				assert_eq!(mid.len(), 1, "the middle level is missing: {:?}", items[0]);
				match &mid[0] {
					Item::List { items: mid_items, .. } => {
						assert_eq!(mid_items.len(), 1, "the middle list should hold one bullet");
						let inner = &mid_items[0].children;
						assert_eq!(inner.len(), 1, "the inner level is missing: {:?}", mid_items[0]);
						match &inner[0] {
							Item::List { items: inner_items, .. } =>
								assert_eq!(inner_items.len(), 1, "the inner list should hold one bullet"),
							other => panic!("the inner child was not a list: {:?}", other),
						}
					},
					other => panic!("the middle child was not a list: {:?}", other),
				}
			},
			_ => unreachable!(),
		}
	}

	/// An unhandled inline `#func[...]` is consumed and recorded rather than left as raw markup, its
	/// bracketed body folded in so its words survive; a paren-only call sets nothing where it stood.
	#[test]
	fn unknown_inline_call_is_recorded_not_leaked() {
		let mut skips = Refusals::default();
		let runs = parse_inlines_in("a #smallcaps[Nato] treaty and a #v(2pt) gap", Span::new(0, 0), &mut skips);
		assert!(runs.iter().all(|r| !matches!(r, Inline::Text(t) if t.contains("#smallcaps") || t.contains("#v("))),
			"raw unknown call leaked: {:?}", runs);
		assert!(runs.iter().any(|r| matches!(r, Inline::Text(t) if t.contains("Nato"))),
			"smallcaps body dropped: {:?}", runs);
		assert_eq!(skips.total(), 2);
		let names: Vec<String> = skips.entries().into_iter().map(|(n, _)| n).collect();
		assert!(names.contains(&"#smallcaps".to_string()) && names.contains(&"#v".to_string()),
			"unexpected skip names: {:?}", names);
	}

	/// The reader tallies the code lines and unknown calls it skips, and reports them one line, so a
	/// dropped construct is visible rather than silent. Handled inline calls do not appear in the tally.
	#[test]
	fn skip_summary_reports_skipped_constructs() {
		install_test_terms();	// so `#g[iniverse]` resolves and adds no term-dict miss to the tally
		// `#import` and `#set rect` (which names no theme element) stay reader refusals. A per-element
		// `#show <selector>: ...` line is a rule the engine collects and applies (or refuses in its own
		// diagnostic), so the reader captures it as a declarative-styling construct and no longer tallies it
		// -- see `show_selector_rule_is_not_a_reader_skip` and the L0 double-report fix.
		let src = "#import \"x.typ\": *\n#set rect(stroke: 1pt)\n\nBody with #g[iniverse] and a #footnote[note].\n\n#show heading: it => it\n";
		let (_, skips) = document_with_refusals(src).expect("parse");
		assert_eq!(skips.total(), 2, "the selector show rule is captured for the engine, not tallied: {:?}", skips.sites());
		let report = skips.report().expect("a report");
		assert!(report.starts_with("skipped 2 unsupported constructs:"), "report was {:?}", report);
		for name in ["#import", "#set"] {
			assert!(report.contains(name), "{} missing from {:?}", name, report);
		}
		assert!(!report.contains("heading"),
			"a selector show rule must not appear in the reader's skip tally (it is the engine's to apply/refuse): {:?}", report);
		// A source the reader sets whole has nothing to report.
		let (_, clean) = document_with_refusals("Just prose with #g[iniverse].\n").expect("parse");
		assert!(clean.is_empty() && clean.report().is_none());
	}

	/// A `#show <selector>: <transform>` line is a per-element rule the engine collects and applies, so the
	/// reader captures it as a declarative-styling construct rather than tallying it as a skipped one -- the
	/// L0 double-report fix. Both the lowerable set-fields form and the refused introspective form are
	/// captured; the refused one surfaces through the rule engine's own diagnostic, not the reader's tally.
	#[test]
	fn show_selector_rule_is_not_a_reader_skip() {
		let (_, lowerable) = document_with_refusals("#show par: set text(size: 9pt)\n\nBody.\n").expect("parse");
		assert_eq!(lowerable.total(), 0, "a lowerable selector rule is not a reader skip: {:?}", lowerable.sites());
		let (_, introspective) = document_with_refusals("#show heading: it => it\n\nBody.\n").expect("parse");
		assert_eq!(introspective.total(), 0,
			"a refused selector rule surfaces via the engine, not the reader tally: {:?}", introspective.sites());
	}

	/// A `#show: <template>.with(...)` application and a lowerable top-level `#set` are captured rather
	/// than refused -- their styling lowers onto the theme -- so neither adds to the refusal tally, while
	/// an introspective `#show` and a `#set` on an unsupported target still do.
	#[test]
	fn lowerable_set_and_doc_with_are_captured_not_refused() {
		// A multi-line `#show: doc.with(...)` and a lowerable `#set text(...)`: both captured, no refusal.
		let lowered = "#show: doc.with(\n  title: [X],\n  heading-font: \"Graystroke\",\n)\n\n#set text(size: 11pt)\n\n= Heading\n\nBody.\n";
		let (_, skips) = document_with_refusals(lowered).expect("parse");
		assert_eq!(skips.total(), 0, "lowerable declarations should not be refused: {:?}", skips.sites());

		// The unsupported `#set` still refuses; a `#show <selector>:` rule is now the engine's to apply or
		// refuse, so it is captured here rather than tallied by the reader (see
		// `show_selector_rule_is_not_a_reader_skip`).
		let refused = "#set rect(stroke: 1pt)\n\n#show heading: it => it\n";
		let (_, skips) = document_with_refusals(refused).expect("parse");
		assert_eq!(skips.total(), 1, "the unsupported #set still refuses; the show rule is captured for the engine: {:?}", skips.sites());
	}

	/// A line-leading `#context[...]` -- Typst's self-observation entry point -- is refused as exactly
	/// one site, classed `Introspective`. It is now gathered as a capture (so its body can be inspected for
	/// the reverse-claim-index signature) and refused when that signature is absent, so its span is the
	/// zero-width caret at the construct's opening offset, as the reader's other capture refusals record.
	#[test]
	fn context_call_is_one_introspective_refusal() {
		let src = "#context[whatever]\n";
		let (_, refusals) = document_with_refusals(src).expect("parse");
		assert_eq!(refusals.total(), 1, "expected exactly one refusal: {:?}", refusals.sites());
		let site = &refusals.sites()[0];
		assert_eq!(site.name, "#context");
		assert_eq!(site.class, RefusalClass::Introspective);
		assert_eq!(site.span, Span::new(0, 0), "a captured-construct refusal records the caret at its opening offset");
	}

	/// The brace twin of the above -- a line-leading `#context{ ... }` code-block call, the shape a book's
	/// reverse-reference index is written with (Lucronics ch29.8) -- is recognised and refused the same
	/// way, not set as prose. Both a one-line block and a multi-line one are refused as one introspective
	/// site, and neither leaks its source into the body: the reader must produce no paragraph at all.
	#[test]
	fn context_brace_block_is_refused_not_set_as_prose() {
		// One line, brace immediately after the keyword.
		let one = "#context{ let x = 1 }\n";
		let (items, refusals) = document_with_refusals(one).expect("parse");
		assert_eq!(refusals.total(), 1, "expected exactly one refusal: {:?}", refusals.sites());
		assert_eq!(refusals.sites()[0].name, "#context");
		assert_eq!(refusals.sites()[0].class, RefusalClass::Introspective);
		assert!(!items.iter().any(|it| matches!(it, Item::Paragraph { .. })),
			"a #context{{}} block must not survive as body text: {:?}", items);

		// Several lines, with a space before the brace -- a multi-line `#context {` block that does NOT call
		// `collect-claim-refs(` (so it is a plain introspective refusal, not the reverse claim index). The
		// whole block, including its own `[...]` and nested `{...}`, is consumed by the capture, not one line
		// of it set as prose.
		let many = "Before.\n\n#context {\n let by = (:)\n if by.len() == 0 [\n _None._\n ] else {\n let n = 1\n }\n}\n\nAfter.\n";
		let (items, refusals) = document_with_refusals(many).expect("parse");
		assert_eq!(refusals.total(), 1, "the multi-line brace block is one refusal: {:?}", refusals.sites());
		assert_eq!(refusals.sites()[0].name, "#context");
		let bodies: Vec<String> = items.iter().filter_map(|it| match it {
			Item::Paragraph { runs, .. } => Some(fmt!("{:?}", runs)),
			_ => None,
		}).collect();
		assert!(!bodies.iter().any(|b| b.contains("None") || b.contains("let by") || b.contains("by-code")),
			"no line of the #context{{}} block may leak into a paragraph: {:?}", bodies);
		// The two real paragraphs around it still set.
		assert_eq!(bodies.len(), 2, "the prose on either side of the block must still set: {:?}", bodies);
	}

	/// A `//` or `/* ... */` comment inside a `#context { ... }` body must not fold its own `}`/`]` into the
	/// skip scanner's bracket balance -- the G3 fix. Before it, `let c = 1 // }` popped the outer brace early,
	/// so the tail of the block (the `if`/`else` and the closing `}`) leaked into the body as raw prose; this
	/// reds on a reverted `step` exactly the way the earlier form's leak did. The body calls a non-claim query
	/// (`counter(page).display()`, not `collect-claim-refs(`), so it stays a plain introspective refusal and
	/// does not trip the reverse-claim-index recognition covered by
	/// `context_collect_claim_refs_lowers_to_a_claim_index` below.
	#[test]
	fn context_brace_block_comment_does_not_close_early() {
		let many = "Before.\n\n#context {\n let refs = counter(page).display()\n let c = 1 // }\n /* a note about } */\n if refs.len() == 0 [\n _None._\n ] else {\n let by = (:)\n }\n}\n\nAfter.\n";
		let (items, refusals) = document_with_refusals(many).expect("parse");
		assert_eq!(refusals.total(), 1, "the whole commented block is still one refusal: {:?}", refusals.sites());
		assert_eq!(refusals.sites()[0].name, "#context");
		let bodies: Vec<String> = items.iter().filter_map(|it| match it {
			Item::Paragraph { runs, .. } => Some(fmt!("{:?}", runs)),
			_ => None,
		}).collect();
		assert!(!bodies.iter().any(|b| b.contains("counter") || b.contains("let refs") || b.contains("let by")),
			"a comment's `}}` must not close the guard early and leak its tail: {:?}", bodies);
		assert_eq!(bodies.len(), 2, "the prose on either side of the block must still set: {:?}", bodies);
	}

	/// The one `#context { ... }` block the reader does not refuse: the Logic appendix's reverse claim index,
	/// recognised by the `collect-claim-refs(` signature in its body (never by evaluating the `#context`). It
	/// lowers to a single `Item::ClaimIndex`, records no refusal, and leaks no line of its source as prose.
	#[test]
	fn context_collect_claim_refs_lowers_to_a_claim_index() {
		let src = "Before.\n\n#context {\n let refs = collect-claim-refs()\n if refs.len() == 0 [\n _None._\n ] else {\n let by = (:)\n }\n}\n\nAfter.\n";
		let (items, refusals) = document_with_refusals(src).expect("parse");
		assert_eq!(refusals.total(), 0, "the reverse claim index is recognised, not refused: {:?}", refusals.sites());
		assert_eq!(items.iter().filter(|it| matches!(it, Item::ClaimIndex { .. })).count(), 1,
			"the collect-claim-refs block lowers to exactly one ClaimIndex: {:?}", items);
		let bodies: Vec<String> = items.iter().filter_map(|it| match it {
			Item::Paragraph { runs, .. } => Some(fmt!("{:?}", runs)),
			_ => None,
		}).collect();
		assert!(!bodies.iter().any(|b| b.contains("collect") || b.contains("let refs") || b.contains("None")),
			"no line of the block may leak into a paragraph: {:?}", bodies);
		assert_eq!(bodies.len(), 2, "the prose on either side of the block must still set: {:?}", bodies);
	}

	/// A line-leading `#query(...)` -- reading the document's own resolved structure back -- is refused
	/// as exactly one site, classed `Introspective`.
	#[test]
	fn query_call_is_one_introspective_refusal() {
		let src = "#query(heading)\n";
		let (_, refusals) = document_with_refusals(src).expect("parse");
		assert_eq!(refusals.total(), 1, "expected exactly one refusal: {:?}", refusals.sites());
		let site = &refusals.sites()[0];
		assert_eq!(site.name, "#query");
		assert_eq!(site.class, RefusalClass::Introspective);
		assert_eq!(site.span, Span::new(0, src.len() as u32 - 1));
	}

	/// A line-leading `#state(...)` call -- a state read/write that only resolves against Typst's own
	/// layout observation -- is refused as exactly one site, classed `Introspective`.
	#[test]
	fn state_call_is_one_introspective_refusal() {
		let src = "#state(\"count\", 0)\n";
		let (_, refusals) = document_with_refusals(src).expect("parse");
		assert_eq!(refusals.total(), 1, "expected exactly one refusal: {:?}", refusals.sites());
		let site = &refusals.sites()[0];
		assert_eq!(site.name, "#state");
		assert_eq!(site.class, RefusalClass::Introspective);
		assert_eq!(site.span, Span::new(0, src.len() as u32 - 1));
	}

	/// The three classes land where the classifier's own doc comment says they should: Typst's general
	/// evaluation primitives are `FixedPoint`, an unrecognised call or wrapper is `Unsupported`.
	#[test]
	fn refusal_class_sorts_fixed_point_and_unsupported_correctly() {
		assert_eq!(RefusalClass::classify("#let"), RefusalClass::FixedPoint);
		assert_eq!(RefusalClass::classify("#show"), RefusalClass::FixedPoint);
		assert_eq!(RefusalClass::classify("#columns"), RefusalClass::Unsupported);
		assert_eq!(RefusalClass::classify("#smallcaps"), RefusalClass::Unsupported);
	}

	/// A `#columns(n)[ ... ]` wrapper is recorded as skipped and its body set single-column, so the words
	/// survive and no raw wrapper leaks into the block stream.
	#[test]
	fn columns_wrapper_flattens_to_single_column() {
		let src = "#columns(2)[\nFirst paragraph here.\n\nSecond paragraph here.\n]\n";
		let (items, skips) = document_with_refusals(src).expect("parse");
		let paras = items.iter().filter(|it| matches!(it, Item::Paragraph { .. })).count();
		assert_eq!(paras, 2, "column body not set as paragraphs: {:?}", items);
		assert_eq!(skips.entries(), vec![("#columns".to_string(), 1)]);
	}

	/// Reads a single [`Item::Paragraph`]'s runs out of a parse, failing loudly with the whole item list
	/// when the source did not yield exactly one paragraph -- the shape every `math_open` regression test
	/// below expects, since a display block that leaked a line would instead split the source into a
	/// paragraph plus a stray heading or list.
	fn one_paragraph(items: &[Item]) -> Outcome<(Vec<Inline>, Option<String>)> {
		let paras: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::Paragraph { .. })).collect();
		match paras.as_slice() {
			[Item::Paragraph { runs, label, .. }] => Ok((runs.clone(), label.clone())),
			_ => Err(err!("expected exactly one paragraph, got: {:?}", items; Test, Bug)),
		}
	}

	/// A source line inside an open `$...$` display block that begins `=` (an alignment row such as
	/// `=> 2N &= ...`, Oxegen TechSpec `app_maths.typ:533-534`) must stay in the equation, not be read as
	/// a heading -- the heading branch is gated off by `math_open` for exactly this shape.
	#[test]
	fn equals_lead_row_inside_display_math_stays_in_block() -> Outcome<()> {
		let src = "Total hashes.\n\n$\nN &= sum_(j=1)^J n_j \\\n=> 2N &= sum_(j=2)^(J+1) 2^(j-1) \\\n=> 2N - N &= 2^J - 1 \\\n$\n\nEnd of block.\n";
		let (items, _skips) = res!(document_with_refusals(src));
		assert!(!items.iter().any(|it| matches!(it, Item::Heading { .. })),
			"a `=`-lead row inside the block must not become a heading: {:?}", items);
		let has_align = items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if matches!(runs.as_slice(),
				[Inline::Math(Atom::Matrix { kind: MatKind::Align, .. })])));
		assert!(has_align, "no single-run display alignment paragraph was produced: {:?}", items);
		Ok(())
	}

	/// A source line inside an open `$...$` display block that begins `-` (a row such as
	/// `- tilde(N)_(1 0) u (...) = 0`, Oxegen TechSpec `ch04_nodes.typ:1657`) must stay in the equation,
	/// not be read as a bullet -- the list-marker branch is gated off by `math_open` for exactly this shape.
	#[test]
	fn dash_lead_row_inside_display_math_stays_in_block() -> Outcome<()> {
		let src = "$\na &= b \\\n- tilde(N)_(1 0) u (x) = 0 \\\nc &= d\n$\n";
		let (items, _skips) = res!(document_with_refusals(src));
		assert!(!items.iter().any(|it| matches!(it, Item::List { .. })),
			"a `-`-lead row inside the block must not become a list: {:?}", items);
		let (runs, _label) = res!(one_paragraph(&items));
		assert!(matches!(runs.as_slice(), [Inline::Math(Atom::Matrix { kind: MatKind::Align, .. })]),
			"expected one display alignment run, got: {:?}", runs);
		Ok(())
	}

	/// A blank source line inside an open `$...$` display block (Oxegen TechSpec `ch04_nodes.typ:1661-1666`)
	/// must stay in the equation rather than flush the paragraph early, and a closing `$ <label>` still
	/// labels the resulting equation.
	#[test]
	fn blank_line_inside_display_math_stays_in_block_and_label_still_attaches() -> Outcome<()> {
		let src = "$\na &= b \\\n\nc &= d\n$ <eq_test>\n";
		let (items, _skips) = res!(document_with_refusals(src));
		let (runs, label) = res!(one_paragraph(&items));
		assert!(matches!(runs.as_slice(), [Inline::Math(Atom::Matrix { kind: MatKind::Align, .. })]),
			"expected one display alignment run, got: {:?}", runs);
		assert_eq!(label, Some("eq_test".to_string()), "the closing label must still attach");
		Ok(())
	}

	/// The plain text of a run of inline markup, for asserting a caption or paragraph's words without
	/// caring how they were split into text, emphasis, glossary or maths runs.
	fn plain(runs: &[Inline]) -> String {
		let mut s = String::new();
		for r in runs {
			match r {
				Inline::Text(t) | Inline::Strong(t) | Inline::Emph(t)
				| Inline::BoldItalic(t) | Inline::Super(t) | Inline::Sub(t) | Inline::Code(t)	=> s.push_str(t),
				Inline::Glossary { display, .. }							=> s.push_str(display),
				_															=> {},
			}
		}
		s
	}

	/// The one [`Item::Figure`] in a parse, failing loudly with the item list when there is not exactly one.
	fn one_figure(items: &[Item]) -> Outcome<(Option<Vec<Inline>>, String, Option<String>)> {
		let figs: Vec<&Item> = items.iter().filter(|it| matches!(it, Item::Figure { .. })).collect();
		match figs.as_slice() {
			[Item::Figure { caption, supplement, label, .. }]	=>
				Ok((caption.clone(), supplement.clone(), label.clone())),
			_	=> Err(err!("expected exactly one figure, got: {:?}", items; Test, Bug)),
		}
	}

	/// A `#figure(...)` whose caption prose carries an author's unbalanced `(` (Oxegen TechSpec
	/// `app_maths.typ:550-566`: "...network messages (latency $100 unit(\"ms\")$ ... chunk sizes $d$.")
	/// must still close at its own `)`: content mode treats the stray paren as literal prose, where the old
	/// flat depth counter stuck open to end of source and swallowed both the figure and the tail after it.
	#[test]
	fn figure_caption_with_unbalanced_paren_closes_and_tail_survives() -> Outcome<()> {
		let src = "\
#figure(
  block(width: 70%)[
    #table(
      columns: (10fr, 10fr),
      [a], [b],
    )
  ],
  caption: [Representative processing times for hashing and network messages (latency $100 unit(\"ms\")$ per message for a Merkle tree of $1 unit(\"GiB\")$ of datastate with various chunk sizes $d$.],
  kind: \"table\",
  supplement: \"Table\",
) <merkle_tree_chunk_size>

Trailing prose.
";
		let (items, _skips) = res!(document_with_refusals(src));
		let (caption, supplement, label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the figure lost its caption"; Test, Bug)));
		assert!(plain(&caption).contains("Representative processing times"),
			"the caption prose was lost: {:?}", caption);
		assert_eq!(supplement, "Table", "the supplement was not read from the figure");
		assert_eq!(label, Some("merkle_tree_chunk_size".to_string()), "the figure label was lost");
		// The tail after the figure must survive rather than be swallowed by a stuck bracket count.
		let tail = items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("Trailing prose")));
		assert!(tail, "the prose after the figure was swallowed: {:?}", items);
		Ok(())
	}

	/// A `(`, a `)`, a `[` or a `]` inside a `$...$` maths span in a caption is literal maths, never a
	/// structural bracket, so an interval or a parenthesised function does not miscount and close the
	/// caption early. Without the maths frame, the `]` in `$[a, b]$` would pop the caption content block.
	#[test]
	fn caption_maths_parens_do_not_count() -> Outcome<()> {
		let src = "\
#figure(
  image(\"fig.png\"),
  caption: [see $f(x)$ over $[a, b]$ where it holds.],
) <fig_maths>

After the figure.
";
		let (items, _skips) = res!(document_with_refusals(src));
		let (caption, _supplement, label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the maths caption was lost"; Test, Bug)));
		assert!(plain(&caption).contains("see") && plain(&caption).contains("where it holds"),
			"the caption prose around the maths was truncated: {:?}", caption);
		assert!(caption.iter().any(|r| matches!(r, Inline::Math(_))),
			"the caption maths span was not parsed as maths: {:?}", caption);
		assert_eq!(label, Some("fig_maths".to_string()), "the label after a maths caption was lost");
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("After the figure"))),
			"the tail after a maths caption was swallowed: {:?}", items);
		Ok(())
	}

	/// A `(` or `)` inside a `"..."` string argument -- here an image path `image("a(b).png")` -- is literal
	/// string content, not a structural paren, so a single-line figure carrying such a path closes on its
	/// line rather than opening a run-away capture.
	#[test]
	fn code_string_paren_does_not_count() -> Outcome<()> {
		let src = "#figure(image(\"a(b).png\"), caption: [c])\n\nNext paragraph.\n";
		let (items, _skips) = res!(document_with_refusals(src));
		let (caption, _supplement, _label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the figure lost its caption"; Test, Bug)));
		assert_eq!(plain(&caption), "c", "the caption was misread past the string paren: {:?}", caption);
		let path = items.iter().find_map(|it| match it {
			Item::Figure { body: FigureBody::Image { path, .. }, .. }	=> Some(path.clone()),
			_														=> None,
		});
		assert_eq!(path, Some("a(b).png".to_string()), "the image path with parens was misread: {:?}", path);
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("Next paragraph"))),
			"the paragraph after a single-line figure was swallowed: {:?}", items);
		Ok(())
	}

	/// A `#name(...)` call met inside a caption's content re-enters code mode, so the string inside it is
	/// protected and a following unbalanced `(` in the surrounding prose is still literal: the caption
	/// closes at its own `]` and the tail survives.
	#[test]
	fn content_call_inside_caption_reenters_code() -> Outcome<()> {
		let src = "\
#figure(
  image(\"g.png\"),
  caption: [see #link(\"http://x\")[y] and note (z],
) <fig_call>

Following text.
";
		let (items, _skips) = res!(document_with_refusals(src));
		let (caption, _supplement, label) = res!(one_figure(&items));
		let caption = res!(caption.ok_or_else(|| err!("the call caption was lost"; Test, Bug)));
		assert!(plain(&caption).contains("see") && plain(&caption).contains("note (z"),
			"the caption prose around the call was truncated: {:?}", caption);
		assert_eq!(label, Some("fig_call".to_string()), "the label after a call caption was lost");
		assert!(items.iter().any(|it| matches!(it,
			Item::Paragraph { runs, .. } if plain(runs).contains("Following text"))),
			"the tail after a call caption was swallowed: {:?}", items);
		Ok(())
	}

	/// [`read_group`] and [`split_top_args`] on a caption argument list directly: an unbalanced `(` in the
	/// caption prose is literal, so the group closes at its own `]` and the top-level commas still part the
	/// arguments, with the maths span and its string protected throughout.
	#[test]
	fn read_group_and_split_handle_caption_prose_paren() -> Outcome<()> {
		// read_group on a caption bracket whose prose carries an unbalanced `(` and a `$...$` span with a
		// quoted `unit("ms")` inside: the group must end at the caption's own `]`, keeping its prose and
		// stopping before the trailing text.
		let s: Vec<char> = "[messages (latency $100 unit(\"ms\")$ per $d$.] more".chars().collect();
		let (inner, next) = res!(read_group(&s, 0)
			.ok_or_else(|| err!("the caption group did not close"; Test, Bug)));
		assert!(inner.contains("(latency"), "the caption prose was lost: {:?}", inner);
		assert!(!inner.contains("more"), "the caption group over-ran its closing bracket: {:?}", inner);
		assert_eq!(s[next..].iter().collect::<String>(), " more", "the index past the closer is wrong");

		// split_top_args across the same shape: three arguments, the middle a caption whose unbalanced paren
		// and maths span do not part it, and named_arg reads the caption key back off it.
		let args = split_top_args("image(\"p.png\"), caption: [x (y $z(w)$.], kind: \"table\"");
		assert_eq!(args.len(), 3, "the unbalanced caption paren split the arg list wrong: {:?}", args);
		let named = res!(named_arg(args[1].trim())
			.ok_or_else(|| err!("the caption argument did not read as named: {:?}", args[1]; Test, Bug)));
		assert_eq!(named.0, "caption", "the caption key was misread: {:?}", named);
		assert!(named.1.contains("(y"), "the caption value lost its unbalanced paren: {:?}", named);
		Ok(())
	}

	/// A prose line that opens with an inline `#raw("...")` and carries a stray `"` from an author's
	/// quotation split across the line break must be set as prose, not read as the start of a multi-line
	/// code skip: a dangling quote is a character, not an open code string. Only an unclosed structural
	/// bracket keeps a skip open (Hematite `sec_net.typ:679-681`, where `express "no\nbound".` split a
	/// quotation over two lines after an inline `#raw`).
	#[test]
	fn prose_line_with_inline_raw_and_dangling_quote_is_not_skipped() -> Outcome<()> {
		let src = "passes its own pair to #raw(\"read_message\"), or to\n\
#raw(\"WebSocket::with_limits\"). There is deliberately no way to express \"no\n\
bound\".\n";
		let (items, _skips) = res!(document_with_refusals(src));
		let text: String = items.iter()
			.filter_map(|it| match it {
				Item::Paragraph { runs, .. }	=> Some(plain(runs)),
				_							=> None,
			})
			.collect::<Vec<_>>()
			.join(" ");
		assert!(text.contains("no way to express"),
			"the prose after an inline #raw was swallowed as a skip: {:?}", items);
		assert!(text.contains("bound"), "the continuation line was swallowed: {:?}", items);
		Ok(())
	}

	/// A `` `//` `` and `` `/* */` `` inside a backtick code span, in a prose caption naming the operators
	/// themselves, must not be read as comment openers: [`Frame::Raw`] keeps the span literal, so the group
	/// still closes at its own `]` and the raw span survives verbatim in the inner text.
	#[test]
	fn read_group_protects_backtick_span_with_comment_markers() -> Outcome<()> {
		let s: Vec<char> = "[the `//` and `/* */` operators]".chars().collect();
		let (inner, next) = res!(read_group(&s, 0)
			.ok_or_else(|| err!("the backtick span made the group mis-scan as a comment"; Test, Bug)));
		assert_eq!(inner, "the `//` and `/* */` operators",
			"the raw span was not kept literal: {:?}", inner);
		assert_eq!(next, s.len(), "the group did not close at its own bracket");
		Ok(())
	}

	/// A `//` on one line of a multi-line capture buffer must be bounded to that line's own end, not eat
	/// every line after it: the buffer's real closing bracket, on the following line, must still be found.
	#[test]
	fn read_group_line_comment_does_not_eat_the_next_line() -> Outcome<()> {
		let s: Vec<char> = "[first line // trailing\nsecond line]".chars().collect();
		let (inner, next) = res!(read_group(&s, 0)
			.ok_or_else(|| err!("the // comment ate past its own line and swallowed the closer"; Test, Bug)));
		assert_eq!(inner, "first line // trailing\nsecond line",
			"the buffer content around the line comment was misread: {:?}", inner);
		assert_eq!(next, s.len(), "the group did not close at its own bracket");
		Ok(())
	}
}
