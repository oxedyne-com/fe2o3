//! The warm-against-cold harness. A long-lived [`Instance`] is driven through a script of edits and, after
//! every state, a fresh one compiles the same project; the two must agree on everything a consumer reads:
//! every page's SVG, the PDF with its `/ID` masked, the delta's page ids, and the report. The delta call is
//! played as the live view plays it, holding the pages it was sent and rebuilding the document from `order`.
//!
//! The fixtures under `tests/fixtures/incremental/` are small and feature-dense. `corpus` is the same
//! comparison over whole documents; it is `#[ignore]`d and run alone in release:
//!
//! `AUST_DIFF_DOCS=a.typ,b.typ=/root AUST_DIFF_FONTS=<dir> cargo test --release --test incremental -- --ignored --test-threads=1`
//!
//! with `AUST_DIFF_EDITS` (default 10; edit j types a letter at position j mod 5 of the body paragraphs),
//! `AUST_DIFF_KINDS` (default `pdf,delta`), `AUST_DIFF_LOG` (a line per state with warm and cold ms) and
//! `AUST_DIFF_DUMP` (a directory for the first divergent pair). The corpus prints numbers and kinds only.

use oxedyne_fe2o3_austenite::compile::{
	self,
	Report,
};
use oxedyne_fe2o3_austenite::door::{
	Instance,
	Project,
};
use oxedyne_fe2o3_austenite::emit::sinks::VectorSink;
use oxedyne_fe2o3_austenite::flow::text::FontStore;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{
	BTreeMap,
	HashMap,
	HashSet,
};
use std::io::Write;
use std::path::{
	Path,
	PathBuf,
};
use std::time::Instant;

const LETTERS:	[&str; 5]	= ["q", "x", "z", "j", "k"];
const KINDS:	[&str; 3]	= ["delta", "svg", "pdf"];
const EXTRA:	&str		= "An added paragraph, set between two others, with enough ordinary words in it to run on to a second line of the page.";

// The document

#[derive(Clone, Debug, Default)]
struct Doc {
	name:		String,
	main:		String,						// empty for `/main.typ`
	sources:	Vec<(String, String)>,		// project path, text
	assets:		Vec<(String, Vec<u8>)>,
	fonts:		Vec<(String, Vec<u8>)>,
}

impl Doc {
	fn main_path(&self) -> String {
		if self.main.is_empty() { "/main.typ".to_string() } else { self.main.clone() }
	}

	fn index(&self, path: &str) -> Option<usize> {
		self.sources.iter().position(|(p, _)| p == path)
	}

	/// The project of this document with the sources in `over` replaced.
	fn project(&self, over: &[(usize, String)], known: &[u64]) -> Project {
		let mut sources = self.sources.clone();
		for (i, text) in over {
			sources[*i].1 = text.clone();
		}
		Project {
			main:		self.main.clone(),
			sources,
			assets:		self.assets.clone(),
			fonts:		self.fonts.clone(),
			strict:		false,
			known:		known.to_vec(),
		}
	}

	fn fixture(name: &str) -> Outcome<Self> {
		let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/incremental").join(fmt!("{}.typ", name));
		let text = res!(std::fs::read_to_string(&path));
		Ok(Self { name: name.to_string(), sources: vec![("/main.typ".to_string(), text)], ..Self::default() })
	}
}

// The paragraphs a script edits

#[derive(Clone, Copy, Debug, PartialEq)]
struct Para {
	src:	usize,
	start:	usize,		// the first byte of the paragraph's first line
	at:		usize,		// just after its first word, where a letter is typed
	end:	usize,		// the end of its last line, before the newline
}

/// The body paragraphs of `text`: a line that begins with an uppercase ASCII letter (any letter when `any`),
/// after a blank line, a heading line or the top of the file, whose first word is followed by a space.
fn paragraphs(src: usize, text: &str, any: bool) -> Vec<Para> {
	let lines: Vec<&str>	= text.split('\n').collect();
	let mut starts			= Vec::with_capacity(lines.len());
	let mut off				= 0;
	for l in &lines {
		starts.push(off);
		off += l.len() + 1;
	}
	let mut out		= Vec::new();
	let mut after	= true;
	for (i, line) in lines.iter().enumerate() {
		let b		= line.as_bytes();
		let first	= b.first().copied().unwrap_or(b' ');
		let letter	= if any { first.is_ascii_alphabetic() } else { first.is_ascii_uppercase() };
		if after && letter {
			let w = b.iter().take_while(|c| c.is_ascii_alphanumeric()).count();
			if w < b.len() && b[w] == b' ' {
				let mut j = i;
				while j + 1 < lines.len() && !lines[j + 1].trim().is_empty() {
					j += 1;
				}
				out.push(Para { src, start: starts[i], at: starts[i] + w, end: starts[j] + lines[j].len() });
			}
		}
		after = line.trim().is_empty() || line.starts_with('=');
	}
	out
}

/// The paths `#include "…"` names in `text`.
fn includes(text: &str) -> Vec<String> {
	let mut out		= Vec::new();
	let mut rest	= text;
	while let Some(i) = rest.find("#include \"") {
		rest = &rest[i + 10..];
		if let Some(j) = rest.find('"') {
			out.push(rest[..j].to_string());
			rest = &rest[j..];
		}
	}
	out
}

/// A path resolved against the including file's directory, `/x` against the root, `.` and `..` folded.
fn resolve(dir: &str, rel: &str) -> String {
	let joined = if rel.starts_with('/') { rel.to_string() } else { fmt!("{}/{}", dir, rel) };
	let mut parts: Vec<&str> = Vec::new();
	for seg in joined.split('/') {
		match seg {
			"" | "."	=> {},
			".."		=> { parts.pop(); },
			s			=> parts.push(s),
		}
	}
	fmt!("/{}", parts.join("/"))
}

/// The main source and the sources it includes, depth first.
fn pool(doc: &Doc) -> Vec<usize> {
	fn walk(doc: &Doc, i: usize, seen: &mut HashSet<usize>, out: &mut Vec<usize>) {
		if !seen.insert(i) {
			return;
		}
		out.push(i);
		let path	= &doc.sources[i].0;
		let dir		= match path.rfind('/') { Some(k) => path[..k].to_string(), None => String::new() };
		for target in includes(&doc.sources[i].1) {
			if let Some(j) = doc.index(&resolve(&dir, &target)) {
				walk(doc, j, seen, out);
			}
		}
	}
	let mut out = Vec::new();
	if let Some(i) = doc.index(&doc.main_path()) {
		walk(doc, i, &mut HashSet::new(), &mut out);
	}
	out
}

/// The body paragraphs of the pool in order; any letter starts one when the pool has fewer than five.
fn pool_paras(doc: &Doc) -> Vec<Para> {
	let sources = pool(doc);
	for any in [false, true] {
		let mut all = Vec::new();
		for &s in &sources {
			all.extend(paragraphs(s, &doc.sources[s].1, any));
		}
		if all.len() >= 5 || any {
			return all;
		}
	}
	Vec::new()
}

/// The letters typed so far, keyed by source and offset in the base text, rendered into the sources.
type Typed = BTreeMap<(usize, usize), String>;

fn render(doc: &Doc, typed: &Typed) -> Vec<(usize, String)> {
	let mut srcs: Vec<usize> = typed.keys().map(|k| k.0).collect();
	srcs.dedup();
	let mut out = Vec::new();
	for s in srcs {
		let base		= &doc.sources[s].1;
		let mut text	= String::with_capacity(base.len() + 8);
		let mut last	= 0;
		for ((_, at), t) in typed.range((s, 0)..(s + 1, 0)) {
			text.push_str(&base[last..*at]);
			text.push_str(t);
			last = *at;
		}
		text.push_str(&base[last..]);
		out.push((s, text));
	}
	out
}

// The script

#[derive(Clone, Copy, Debug, PartialEq)]
enum Bite {
	Free,		// no claim
	Base,		// the cold PDF is the base's
	Moves,		// the cold PDF is not the base's
}

#[derive(Clone, Debug)]
struct State {
	label:	String,
	over:	Vec<(usize, String)>,
	reload:	bool,					// the consumer drops what it holds before the call
	bite:	Bite,
}

fn state(label: &str, over: Vec<(usize, String)>, reload: bool, bite: Bite) -> State {
	State { label: label.to_string(), over, reload, bite }
}

fn edit_once(text: &str, find: &str, rep: &str) -> Outcome<String> {
	let n = text.matches(find).count();
	if n != 1 {
		return Err(err!("The anchor {:?} occurs {} times, not once.", find, n; Test));
	}
	Ok(text.replacen(find, rep, 1))
}

/// The fixture script: open, a no-op save, five letters one after another, the revert, each table edit and
/// its undo, a paragraph added and one removed, the consumer's reload, a reload after an edit, the base.
fn fixture_script(doc: &Doc, tables: &[(&str, &str, &str)]) -> Outcome<Vec<State>> {
	let paras = pool_paras(doc);
	if paras.len() < 5 {
		return Err(err!("{} has {} body paragraphs, under five.", doc.name, paras.len(); Test));
	}
	let mut out = vec![
		state("open", Vec::new(), false, Bite::Free),
		state("save", Vec::new(), false, Bite::Base),
	];
	let mut typed = Typed::new();
	for j in 0..5 {
		let p = paras[j * paras.len() / 5];
		typed.entry((p.src, p.at)).or_default().push_str(LETTERS[j]);
		out.push(state(&fmt!("edit {}", j + 1), render(doc, &typed), false, Bite::Moves));
	}
	out.push(state("revert", Vec::new(), false, Bite::Base));
	for (label, find, rep) in tables {
		let text = res!(edit_once(&doc.sources[0].1, find, rep));
		out.push(state(label, vec![(0, text)], false, Bite::Moves));
		out.push(state(&fmt!("{} undone", label), Vec::new(), false, Bite::Base));
	}
	let add = paras[1];
	let mut text = doc.sources[add.src].1.clone();
	text.insert_str(add.end, &fmt!("\n\n{}", EXTRA));
	out.push(state("paragraph added", vec![(add.src, text)], false, Bite::Moves));
	let cut = paras[paras.len() / 2];
	let mut text = doc.sources[cut.src].1.clone();
	text.replace_range(cut.start..cut.end, "");
	out.push(state("paragraph removed", vec![(cut.src, text)], false, Bite::Moves));
	out.push(state("reload", Vec::new(), true, Bite::Base));
	let mut one = Typed::new();
	one.entry((paras[0].src, paras[0].at)).or_default().push_str(LETTERS[0]);
	out.push(state("reload after edit", render(doc, &one), true, Bite::Moves));
	out.push(state("base again", Vec::new(), false, Bite::Base));
	Ok(out)
}

/// The corpus script: open, a no-op save, then `edits` letters, edit j at position j mod 5, kept.
fn corpus_script(doc: &Doc, edits: usize) -> Outcome<Vec<State>> {
	let paras = pool_paras(doc);
	if paras.is_empty() {
		return Err(err!("{} has no body paragraph to edit.", doc.name; Test));
	}
	let mut out = vec![
		state("open", Vec::new(), false, Bite::Free),
		state("save", Vec::new(), false, Bite::Base),
	];
	let mut typed = Typed::new();
	for j in 0..edits {
		let p = paras[(j % 5) * paras.len() / 5];
		typed.entry((p.src, p.at)).or_default().push_str(LETTERS[j % 5]);
		out.push(state(&fmt!("edit {}", j + 1), render(doc, &typed), false, Bite::Moves));
	}
	Ok(out)
}

// What one call showed

#[derive(Clone, Debug, Default)]
struct Seen {
	n:		usize,
	order:	Vec<u64>,
	pages:	Vec<String>,
	pdf:	Vec<u8>,
	diags:	Vec<String>,	// kind first, then the whole site; the summary lines follow
}

fn diags_of(report: &Report, needs: &[String]) -> Vec<String> {
	let mut v: Vec<String> = report.diagnostics.iter().map(|d| fmt!("{} {:?}", d.kind.as_str(), d)).collect();
	v.push(fmt!("skipped {:?}", report.skipped));
	v.push(fmt!("summary {:?}", report.summary));
	v.push(fmt!("needs {:?}", needs));
	v.push(fmt!("empty {}", report.empty));
	v
}

/// Blanks both halves of the trailer's `/ID`, which is a hash of the body and so moves with it.
fn mask_id(pdf: &mut [u8]) {
	let key = b"/ID [";
	if let Some(i) = pdf.windows(key.len()).position(|w| w == key) {
		let mut k = i + key.len();
		while k < pdf.len() && pdf[k] != b']' {
			if pdf[k].is_ascii_hexdigit() {
				pdf[k] = b'0';
			}
			k += 1;
		}
	}
}

/// One side of the comparison: an instance and the pages a live-view consumer holds from its deltas.
struct Side {
	inst:		Instance,
	known:		Vec<u64>,
	held:		HashMap<u64, String>,
	version:	u32,
}

impl Side {
	fn new() -> Self {
		Self { inst: Instance::new(), known: Vec::new(), held: HashMap::new(), version: 0 }
	}

	fn call(&mut self, kind: &str, doc: &Doc, st: &State) -> Result<Seen, String> {
		match kind {
			"delta"	=> self.delta(doc, st),
			"svg"	=> self.svg(doc, st),
			_		=> self.pdf(doc, st),
		}
	}

	fn delta(&mut self, doc: &Doc, st: &State) -> Result<Seen, String> {
		if st.reload {
			self.known.clear();
			self.held.clear();
		}
		let was_empty	= self.known.is_empty();
		let p			= doc.project(&st.over, &self.known);
		let made = match self.inst.compile_delta(&p) {
			Ok(m)	=> m,
			Err(f)	=> return Err(fmt!("{:?}", f)),
		};
		let d = made.product;
		if d.reset != was_empty {
			return Err(fmt!("protocol: reset is {} though the consumer held {} pages", d.reset, self.known.len()));
		}
		if d.rendered as usize != d.changed.len() {
			return Err(fmt!("protocol: rendered {} but changed holds {}", d.rendered, d.changed.len()));
		}
		if d.version != self.version + 1 {
			return Err(fmt!("protocol: version {} after {}", d.version, self.version));
		}
		self.version = d.version;
		for (id, svg) in d.changed {
			self.held.insert(id, svg);
		}
		let mut pages = Vec::with_capacity(d.order.len());
		for id in &d.order {
			match self.held.get(id) {
				Some(s)	=> pages.push(s.clone()),
				None	=> return Err(fmt!("protocol: the delta lacks page {:016x}", id)),
			}
		}
		let keep: HashSet<u64> = d.order.iter().copied().collect();
		self.held.retain(|id, _| keep.contains(id));
		self.known = d.order.clone();
		Ok(Seen { n: made.report.pages, order: d.order, pages, pdf: Vec::new(), diags: diags_of(&made.report, &made.needs) })
	}

	fn svg(&mut self, doc: &Doc, st: &State) -> Result<Seen, String> {
		let p = doc.project(&st.over, &[]);
		match self.inst.compile_svg(&p) {
			Ok(m)	=> Ok(Seen { n: m.report.pages, order: Vec::new(), pages: m.product, pdf: Vec::new(), diags: diags_of(&m.report, &m.needs) }),
			Err(f)	=> Err(fmt!("{:?}", f)),
		}
	}

	fn pdf(&mut self, doc: &Doc, st: &State) -> Result<Seen, String> {
		let p = doc.project(&st.over, &[]);
		match self.inst.compile_pdf(&p) {
			Ok(m)	=> {
				let mut bytes = m.product.to_vec();
				mask_id(&mut bytes);
				Ok(Seen { n: m.report.pages, order: Vec::new(), pages: Vec::new(), pdf: bytes, diags: diags_of(&m.report, &m.needs) })
			},
			Err(f)	=> Err(fmt!("{:?}", f)),
		}
	}
}

// The comparison

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
	let n = a.len().min(b.len());
	for i in 0..n {
		if a[i] != b[i] {
			return Some(i);
		}
	}
	if a.len() != b.len() { Some(n) } else { None }
}

fn excerpt(b: &[u8], at: usize) -> String {
	let lo = at.saturating_sub(40);
	let hi = (at + 40).min(b.len());
	String::from_utf8_lossy(&b[lo..hi]).into_owned()
}

/// Why two results differ, naming the field and where, or `None` when they agree. Quiet gives numbers and
/// kinds only, never a word of the document.
fn diff_seen(warm: &Result<Seen, String>, cold: &Result<Seen, String>, quiet: bool) -> Option<String> {
	let (w, c) = match (warm, cold) {
		(Ok(w), Ok(c))		=> (w, c),
		(Ok(_), Err(e))		=> return Some(if quiet { "warm compiled, cold refused".to_string() } else { fmt!("warm compiled, cold refused: {}", e) }),
		(Err(e), Ok(_))		=> return Some(if quiet { "warm refused, cold compiled".to_string() } else { fmt!("warm refused, cold compiled: {}", e) }),
		(Err(a), Err(b))	=> {
			return match first_diff(a.as_bytes(), b.as_bytes()) {
				None		=> None,
				Some(at)	=> Some(if quiet {
					fmt!("both refused, differently, at byte {} (lengths {} and {})", at, a.len(), b.len())
				} else {
					fmt!("both refused, differently, at byte {}: warm {:?}, cold {:?}", at, excerpt(a.as_bytes(), at), excerpt(b.as_bytes(), at))
				}),
			};
		},
	};
	if w.n != c.n {
		return Some(fmt!("page count: warm {}, cold {}", w.n, c.n));
	}
	if w.order != c.order {
		let slot = (0..w.order.len().max(c.order.len())).find(|&i| w.order.get(i) != c.order.get(i)).unwrap_or(0);
		return Some(fmt!("order differs at slot {} (lengths {} and {})", slot, w.order.len(), c.order.len()));
	}
	if w.pages.len() != c.pages.len() {
		return Some(fmt!("pages: warm {}, cold {}", w.pages.len(), c.pages.len()));
	}
	for (i, (a, b)) in w.pages.iter().zip(c.pages.iter()).enumerate() {
		if let Some(at) = first_diff(a.as_bytes(), b.as_bytes()) {
			return Some(if quiet {
				fmt!("page {} differs at byte {} (lengths {} and {})", i + 1, at, a.len(), b.len())
			} else {
				fmt!("page {} differs at byte {}: warm {:?}, cold {:?}", i + 1, at, excerpt(a.as_bytes(), at), excerpt(b.as_bytes(), at))
			});
		}
	}
	if let Some(at) = first_diff(&w.pdf, &c.pdf) {
		return Some(fmt!("pdf differs at byte {} (lengths {} and {})", at, w.pdf.len(), c.pdf.len()));
	}
	if w.diags.len() != c.diags.len() {
		return Some(fmt!("diagnostics: warm {}, cold {}", w.diags.len(), c.diags.len()));
	}
	for (i, (a, b)) in w.diags.iter().zip(c.diags.iter()).enumerate() {
		if a != b {
			let kind = |s: &str| s.split(' ').next().unwrap_or("").to_string();
			return Some(if quiet {
				fmt!("diagnostic {} differs (kinds {} and {})", i, kind(a), kind(b))
			} else {
				fmt!("diagnostic {} differs: warm {}, cold {}", i, a, b)
			});
		}
	}
	None
}

// The run

#[derive(Default)]
struct Opts {
	log:	Option<PathBuf>,
	dump:	Option<PathBuf>,
}

#[derive(Clone, Copy, Debug)]
struct Kinds {
	delta:	bool,
	svg:	bool,
	pdf:	bool,
}

impl Kinds {
	const ALL: Self = Self { delta: true, svg: true, pdf: true };

	fn parse(s: &str) -> Outcome<Self> {
		let mut k = Self { delta: false, svg: false, pdf: false };
		for w in s.split(',') {
			match w.trim() {
				"delta"	=> k.delta = true,
				"svg"	=> k.svg = true,
				"pdf"	=> k.pdf = true,
				other	=> return Err(err!("AUST_DIFF_KINDS names {:?}, not delta, svg or pdf.", other; Test)),
			}
		}
		Ok(k)
	}

	fn has(&self, kind: &str) -> bool {
		match kind { "delta" => self.delta, "svg" => self.svg, _ => self.pdf }
	}
}

/// What a state's cold compile gave, for the claims a script makes about its edits.
struct Trace {
	label:	String,
	n:		Option<usize>,
	ok:		bool,
	pdf:	Vec<u8>,
	diags:	Vec<String>,
}

struct Verdict {
	diverged:	Option<String>,
	traces:		Vec<Trace>,
	ran:		usize,
}

fn run_script(doc: &Doc, states: &[State], kinds: Kinds, quiet: bool, opts: &Opts) -> Verdict {
	let mut v		= Verdict { diverged: None, traces: Vec::new(), ran: 0 };
	let mut warm	= Side::new();
	for (idx, st) in states.iter().enumerate() {
		let mut trace = Trace { label: st.label.clone(), n: None, ok: false, pdf: Vec::new(), diags: Vec::new() };
		let mut first = true;
		for kind in KINDS {
			if !kinds.has(kind) {
				continue;
			}
			let t0		= Instant::now();
			let w		= warm.call(kind, doc, st);
			let warm_ms	= t0.elapsed().as_secs_f64() * 1000.0;
			let t1		= Instant::now();
			let c		= Side::new().call(kind, doc, st);
			let cold_ms	= t1.elapsed().as_secs_f64() * 1000.0;
			if let Some(log) = &opts.log {
				if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
					let _ = writeln!(f, "{}\t{}\t{}\t{:.1}\t{:.1}", doc.name, st.label, kind, warm_ms, cold_ms);
				}
			}
			if first {
				trace.ok = c.is_ok();
				match &c {
					Ok(s)	=> { trace.n = Some(s.n); trace.diags = s.diags.clone(); },
					Err(e)	=> trace.diags = vec![e.clone()],
				}
				first = false;
			}
			if let Ok(s) = &c {
				if kind == "pdf" {
					trace.pdf = s.pdf.clone();
				}
			}
			if let Some(why) = diff_seen(&w, &c, quiet) {
				if let Some(dir) = &opts.dump {
					let _ = std::fs::create_dir_all(dir);
					for (side, r) in [("warm", &w), ("cold", &c)] {
						if let Ok(s) = r {
							let bytes = if kind == "pdf" { s.pdf.clone() } else { s.pages.join("\n<!-- page -->\n").into_bytes() };
							let _ = std::fs::write(dir.join(fmt!("{}_{}_{}_{}.bin", doc.name, idx, kind, side)), bytes);
						}
					}
				}
				v.diverged = Some(fmt!("{} / {} / {}: {}", doc.name, st.label, kind, why));
				v.traces.push(trace);
				v.ran = idx + 1;
				return v;
			}
		}
		v.traces.push(trace);
		v.ran = idx + 1;
	}
	v
}

// The fixtures

struct Fixture {
	name:	&'static str,
	edits:	&'static [(&'static str, &'static str, &'static str)],
	warns:	bool,								// the document never settles, and both sides say so
	pages:	&'static [(&'static str, usize)],	// the cold page count at named states
}

fn fixture(f: &Fixture) -> Outcome<()> {
	let doc		= res!(Doc::fixture(f.name));
	let states	= res!(fixture_script(&doc, f.edits));
	let v		= run_script(&doc, &states, Kinds::ALL, false, &Opts::default());
	if let Some(d) = &v.diverged {
		return Err(err!("{}", d; Test));
	}
	let base = &v.traces[0];
	let mut counts = String::new();
	for (st, tr) in states.iter().zip(v.traces.iter()) {
		assert!(tr.ok, "{} / {}: the cold compile refused: {:?}", f.name, st.label, tr.diags);
		counts.push_str(&fmt!(" {}={}", st.label, tr.n.unwrap_or(0)));
		match st.bite {
			Bite::Base	=> assert!(tr.pdf == base.pdf, "{} / {}: the state should give the base's PDF and does not", f.name, st.label),
			Bite::Moves	=> assert!(tr.pdf != base.pdf, "{} / {}: the edit changes nothing in the cold PDF, so it does not bite", f.name, st.label),
			Bite::Free	=> {},
		}
		let said = tr.diags.iter().any(|d| d.contains("did not converge"));
		assert_eq!(said, f.warns, "{} / {}: the non-convergence warning should be {}: {:?}", f.name, st.label, f.warns, tr.diags);
	}
	println!("{} pages:{}", f.name, counts);
	for (label, n) in f.pages {
		let tr = res!(v.traces.iter().find(|t| t.label == *label).ok_or_else(|| err!("{} has no state {:?}.", f.name, label; Test)));
		assert_eq!(tr.n, Some(*n), "{} / {}: the cold page count", f.name, label);
	}
	Ok(())
}

const FOOTNOTE_SPILL: Fixture = Fixture {
	name:	"footnote_spill",
	edits:	&[
		("third note grown", "hour by hour through the evening", "hour by hour through the evening and then through the small hours of the night, and again at each turn of the tide"),
		("second note shortened", "so that it competes with the text above for the room that is left, and a good deal of it carries over onto the following page when the paragraph that cites it falls near the bottom", "so that it is brief"),
		("paragraph lengthened", "both were still warm when the first of the party arrived at the door.", "both were still warm when the first of the party arrived at the door. It had been a long walk from the harbour, and the party was glad of the rest, and of the fire."),
		("footnote dropped", "#footnote[Short.] ", ""),
		("footnote added", "moved on without comment.", "moved on without comment.#footnote[A late note, added after the fact.]"),
	],
	warns:	false,
	pages:	&[("open", 7), ("third note grown", 8), ("second note shortened", 6), ("footnote added", 8), ("paragraph removed", 6)],
};

const FLOATS: Fixture = Fixture {
	name:	"floats",
	edits:	&[
		("float one taller", "height: 44pt", "height: 90pt"),
		("float two to the bottom", "#place(top + center, float: true, clearance: 6pt)", "#place(bottom + center, float: true, clearance: 6pt)"),
		("clause added", "Another paragraph arrives here,", "Another paragraph arrives here, and it goes on for rather longer than it did before, with a clause that takes a line or two more,"),
		("tall float shortened", "height: 70pt", "height: 20pt"),
	],
	warns:	false,
	pages:	&[("open", 5), ("float one taller", 6), ("paragraph added", 6)],
};

const COUNTERS_CONTEXT: Fixture = Fixture {
	name:	"counters_context",
	edits:	&[
		("second step", "Third paragraph. #note.step()", "Third paragraph. #note.step() #note.step()"),
		("long clause", "It steps both again and carries on", "It steps both again, and then runs on for a good many words more than it did before, so that the page fills sooner and a new page is added at the end of the text, and carries on"),
		("state update dropped", "#seen.update(n => n + 1) A fourth", "A fourth"),
	],
	warns:	false,
	pages:	&[("open", 3), ("long clause", 4), ("paragraph added", 4)],
};

const HEADINGS_REFS_OUTLINE: Fixture = Fixture {
	name:	"headings_refs_outline",
	edits:	&[
		("long clause in method", "It is set out here at some length, since", "It is set out here at some length, and then at greater length still, with a good many more words than before to push the later headings onto a later page, since the later parts depend on it and on what is said here about the method, the data, the checks that were made on the data, the steps taken when a check failed, the notes kept for each step, and the reasons for each choice. Further"),
		("results renamed", "= Results <results>", "= Findings and results <results>"),
		("more details demoted", "== More details <more>", "=== More details <more>"),
		("references swapped", "@details and in @more", "@more and in @details"),
	],
	warns:	false,
	pages:	&[("open", 5), ("long clause in method", 6), ("paragraph added", 6)],
};

const PAGEBREAK_PARITY: Fixture = Fixture {
	name:	"pagebreak_parity",
	edits:	&[
		("first section grows", "It has a second short paragraph,", "It has a second short paragraph, followed by a good deal more text than before, enough to run the first section over onto a second page and so flip the parity of the break that follows it,"),
		("first section shrinks", "The first section sits on the first page and runs on for a few lines, long enough to matter but short enough to leave room.", "The first section."),
		("second section grows", "A second paragraph of the second section,", "A second paragraph of the second section, which now runs a good deal longer than it did, to carry the section onto a further page and move the break after it,"),
		("odd break made even", "#pagebreak(to: \"odd\")", "#pagebreak(to: \"even\")"),
		("even break made weak", "#pagebreak(to: \"even\")", "#pagebreak(weak: true)"),
	],
	warns:	false,
	pages:	&[("open", 6), ("odd break made even", 4), ("even break made weak", 5), ("paragraph removed", 4)],
};

const PAGE_COLUMNS: Fixture = Fixture {
	name:	"page_columns",
	edits:	&[
		("longer note", "#footnote[A note in the columns.]", "#footnote[A note in the columns, now a good deal longer, with several lines of words that take room from the column it stands in.]"),
		("column break removed", "#colbreak()", ""),
		("one-column paragraph long", "Another paragraph in the one column follows it,", "Another paragraph in the one column follows it, and this time it is a long one, with enough text in it to fill the page from the top to the foot and a good way onto the page after it, which is the whole point of putting it here, in the middle of the one-column part of the document, where the flow is simplest and the effect of an edit is the easiest to see on the page that follows it,"),
		("three columns to two", "#set page(columns: 3)", "#set page(columns: 2)"),
	],
	warns:	false,
	pages:	&[("open", 4), ("one-column paragraph long", 5)],
};

const TABLE_SPLIT: Fixture = Fixture {
	name:	"table_split",
	edits:	&[
		("cell wraps", "[Blankets]", "[Blankets, woollen, grey, in bales of ten, stored in the loft above the stores room]"),
		("row added", "[Buckets, galvanised, some with holes], [11],", "[Buckets, galvanised, some with holes], [11],\n\t[13], [Nails, box, assorted], [40],"),
		("long cell shortened", "[Charts of the coast and the approaches, folded, with the harbour plans and the tide tables bound in]", "[Charts]"),
		("header renamed", "table.header([No.], [Item], [Qty])", "table.header([Number], [Item and description], [Quantity])"),
	],
	warns:	false,
	pages:	&[("open", 7), ("header renamed", 8), ("paragraph added", 8)],
};

const NONCONVERGE: Fixture = Fixture {
	name:	"nonconverge",
	edits:	&[
		("a line longer", "set before the state is read and long enough", "set before the state is read and now a good many words longer than it was, enough to take another line or more, and long enough"),
		("a paragraph removed", "The fifth paragraph is a short one, and the sixth follows it.\n\n", ""),
	],
	warns:	true,
	pages:	&[("open", 3), ("paragraph added", 4)],
};

#[test]
fn footnote_spill_warm_equals_cold() -> Outcome<()> { fixture(&FOOTNOTE_SPILL) }

#[test]
fn floats_warm_equals_cold() -> Outcome<()> { fixture(&FLOATS) }

#[test]
fn counters_context_warm_equals_cold() -> Outcome<()> { fixture(&COUNTERS_CONTEXT) }

#[test]
fn headings_refs_outline_warm_equals_cold() -> Outcome<()> { fixture(&HEADINGS_REFS_OUTLINE) }

#[test]
fn pagebreak_parity_warm_equals_cold() -> Outcome<()> { fixture(&PAGEBREAK_PARITY) }

#[test]
fn page_columns_warm_equals_cold() -> Outcome<()> { fixture(&PAGE_COLUMNS) }

#[test]
fn table_split_warm_equals_cold() -> Outcome<()> { fixture(&TABLE_SPLIT) }

#[test]
fn nonconverge_warm_equals_cold_and_both_warn() -> Outcome<()> { fixture(&NONCONVERGE) }

// The harness's own claims

#[test]
fn the_comparator_names_the_first_differing_page_and_field() {
	let a = Seen { n: 3, order: vec![1, 2, 3], pages: vec!["<a/>".into(), "<b/>".into(), "<c/>".into()], pdf: b"%PDF".to_vec(), diags: vec!["limit x".into()] };
	let ok = |s: &Seen| -> Result<Seen, String> { Ok(s.clone()) };
	assert_eq!(diff_seen(&ok(&a), &ok(&a), false), None);
	let mut b = a.clone();
	b.pages[1] = "<b>!</b>".to_string();
	let why = diff_seen(&ok(&a), &ok(&b), false).expect("a differing page");
	assert!(why.contains("page 2 differs at byte 2"), "{}", why);
	let quiet = diff_seen(&ok(&a), &ok(&b), true).expect("a differing page");
	assert!(quiet.contains("page 2 differs at byte 2") && !quiet.contains("<b"), "{}", quiet);
	let mut b = a.clone();
	b.n = 4;
	assert!(diff_seen(&ok(&a), &ok(&b), false).expect("a count").starts_with("page count"));
	let mut b = a.clone();
	b.order[2] = 9;
	assert!(diff_seen(&ok(&a), &ok(&b), false).expect("an order").contains("order differs at slot 2"));
	let mut b = a.clone();
	b.pages.pop();
	assert!(diff_seen(&ok(&a), &ok(&b), false).expect("a length").starts_with("pages: warm 3, cold 2"));
	let mut b = a.clone();
	b.pdf = b"%PDX".to_vec();
	assert!(diff_seen(&ok(&a), &ok(&b), false).expect("a pdf").contains("pdf differs at byte 3"));
	let mut b = a.clone();
	b.diags = vec!["lint x".into()];
	assert!(diff_seen(&ok(&a), &ok(&b), true).expect("a diagnostic").contains("kinds limit and lint"));
	let e: Result<Seen, String> = Err("no".to_string());
	assert!(diff_seen(&ok(&a), &e, false).expect("a refusal").starts_with("warm compiled, cold refused"));
	assert!(diff_seen(&e, &e, false).is_none());
}

#[test]
fn the_paragraph_finder_returns_the_expected_offsets() {
	let text = "// Head.\n#set text(size: 9pt)\n\nFirst para line one\nline two\n\n= Title\nSecond here\n\n  Indented one\n\nlower case start\n\nOneWord\n";
	let at = |s: &str| text.find(s).expect("an anchor");
	let p = paragraphs(0, text, false);
	assert_eq!(p.len(), 2, "{:?}", p);
	assert_eq!(p[0], Para { src: 0, start: at("First"), at: at("First") + 5, end: at("\n\n= Title") });
	assert_eq!(p[1], Para { src: 0, start: at("Second"), at: at("Second") + 6, end: at("\n\n  Indented") });
	let any = paragraphs(0, text, true);
	assert_eq!(any.len(), 3, "a lower-case start counts when any letter will do: {:?}", any);
}

#[test]
fn the_include_pool_runs_depth_first_and_resolves_paths() {
	let src = |p: &str, t: &str| (p.to_string(), t.to_string());
	let doc = Doc {
		name:		"pool".to_string(),
		main:		"/book/main.typ".to_string(),
		sources:	vec![
			src("/book/main.typ", "Intro.\n#include \"ch/one.typ\"\n#include \"/shared/two.typ\"\n"),
			src("/book/ch/one.typ", "One.\n#include \"../three.typ\"\n"),
			src("/shared/two.typ", "Two.\n#include \"/book/main.typ\"\n"),
			src("/book/three.typ", "Three.\n"),
			src("/book/unused.typ", "Unused.\n"),
		],
		..Doc::default()
	};
	assert_eq!(pool(&doc), vec![0, 1, 3, 2]);
	assert_eq!(resolve("/a/b", "../c/./d.typ"), "/a/c/d.typ");
	assert_eq!(resolve("/a/b", "/x.typ"), "/x.typ");
}

#[test]
fn the_id_mask_blanks_both_halves_and_nothing_else() {
	let mut pdf = b"<< /Size 19 /ID [<3869b46160a40fed> <3869b46160a40fed>] >>\n/Other <abcdef>".to_vec();
	mask_id(&mut pdf);
	assert_eq!(String::from_utf8_lossy(&pdf), "<< /Size 19 /ID [<0000000000000000> <0000000000000000>] >>\n/Other <abcdef>");
}

// The corpus

fn env_opt(name: &str) -> Option<String> {
	std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// A document read through one real compile: the files the compile read are the project, so that a long
/// book is not loaded whole.
fn prime(main: &Path, root: &Path, fonts_dir: Option<&Path>) -> Outcome<Doc> {
	compile::supply_typst_package_cache();
	let mut fonts = FontStore::default();
	if let Some(d) = fonts_dir {
		fonts.add_dir(d.to_path_buf());
	}
	let done	= res!(compile::assemble_eval(main, root, fonts, &mut VectorSink::default()));
	let root	= res!(std::fs::canonicalize(root));
	let main_c	= res!(std::fs::canonicalize(main));
	let mut doc	= Doc::default();
	for p in done.files_read() {
		if !p.is_file() {
			continue;
		}
		let bytes	= res!(std::fs::read(&p));
		let ext		= p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
		if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
			doc.fonts.push((p.display().to_string(), bytes));
			continue;
		}
		let rel = res!(p.strip_prefix(&root).map_err(|_| err!("A file was read from outside the root: {}", p.display(); Test)));
		let at = fmt!("/{}", rel.display());
		if ext == "typ" {
			doc.sources.push((at, res!(String::from_utf8(bytes).map_err(|_| err!("{} is not UTF-8.", p.display(); Test)))));
		} else {
			doc.assets.push((at, bytes));
		}
	}
	doc.sources.sort();
	doc.assets.sort_by(|a, b| a.0.cmp(&b.0));
	doc.main = fmt!("/{}", res!(main_c.strip_prefix(&root).map_err(|_| err!("The main file is outside the root."; Test))).display());
	doc.name = main.file_stem().and_then(|s| s.to_str()).unwrap_or("doc").to_string();
	Ok(doc)
}

#[test]
#[ignore = "the corpus: reads AUST_DIFF_DOCS, run alone in release (see the header)"]
fn corpus() -> Outcome<()> {
	let docs	= res!(std::env::var("AUST_DIFF_DOCS").map_err(|_| err!("Set AUST_DIFF_DOCS to a comma-separated list of doc[=root] paths."; Test)));
	let fonts	= env_opt("AUST_DIFF_FONTS").map(PathBuf::from);
	let edits	= match env_opt("AUST_DIFF_EDITS") {
		Some(s)	=> res!(s.parse::<usize>().map_err(|e| err!("AUST_DIFF_EDITS: {}", e; Test))),
		None	=> 10,
	};
	let kinds	= res!(Kinds::parse(&env_opt("AUST_DIFF_KINDS").unwrap_or_else(|| "pdf,delta".to_string())));
	let opts	= Opts { log: env_opt("AUST_DIFF_LOG").map(PathBuf::from), dump: env_opt("AUST_DIFF_DUMP").map(PathBuf::from) };
	let mut bad	= Vec::new();
	for spec in docs.split(',').filter(|s| !s.trim().is_empty()) {
		let (main, root) = match spec.split_once('=') {
			Some((m, r))	=> (PathBuf::from(m), PathBuf::from(r)),
			None			=> {
				let m = PathBuf::from(spec);
				let r = m.parent().map(|p| p.to_path_buf()).unwrap_or_default();
				(m, r)
			},
		};
		let doc		= res!(prime(&main, &root, fonts.as_deref()));
		let states	= res!(corpus_script(&doc, edits));
		let v		= run_script(&doc, &states, kinds, true, &opts);
		let pages	= v.traces.first().and_then(|t| t.n).unwrap_or(0);
		let files	= doc.sources.len() + doc.assets.len();
		match &v.diverged {
			None		=> println!("corpus {}: {} files, {} paragraphs in the pool, {} pages, {} of {} states equal", doc.name, files, pool_paras(&doc).len(), pages, v.ran, states.len()),
			Some(d)		=> {
				println!("corpus {}: DIVERGED after {} of {} states: {}", doc.name, v.ran, states.len(), d);
				bad.push(d.clone());
			},
		}
	}
	if !bad.is_empty() {
		return Err(err!("{} document(s) diverged: {}", bad.len(), bad.join("; "); Test));
	}
	Ok(())
}
