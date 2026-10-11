//! Ported from hayagriva 0.10.1, Copyright (c) Martin Haug and the hayagriva contributors,
//! MIT OR Apache-2.0: `src/csl/mod.rs` (`BibliographyDriver::finish`, `do_citation`,
//! `do_bibliography`) and `src/csl/sort.rs`.
//!
//! hayagriva collates text keys with ICU for the style's locale. `collate` approximates the
//! root collation's primary level (spaces, then punctuation, digits and letters, with accents and
//! case set aside) and breaks ties by code point.
use crate::unicode::norm::{
	combining_class,
	nfd,
};
use crate::csl::{
	item::{
		var_kind,
		DateVal,
		Item,
		NumVal,
		Numeric,
		VarKind,
	},
	locale::Chain,
	page::PageVal,
	node::{
		self,
		Formatting,
		Meta,
		Node,
	},
	render::{
		self,
		Cite,
		Ctx,
		Ibid,
		Sorting,
	},
	style::{
		Demote,
		NameOpts,
		Sort,
		SortKey,
		SortSrc,
		Style,
		VarForm,
	},
};

use std::cmp::Ordering;


/// Typst's citation forms, hayagriva's `CitePurpose`; `Normal` is the style's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CiteForm { #[default] Normal, Author, Year, Full, Prose }

/// One reference within a citation, to an entry by its index in the item slice.
#[derive(Clone, Debug, Default)]
pub struct CiteRef {
	pub entry:	usize,
	pub locator:	Option<(String, NumVal)>,	// kind (`page`, `custom`) and value
	pub form:	CiteForm,
	pub hidden:	bool,				// listed in the bibliography, never printed
}

#[derive(Clone, Debug, Default)]
pub struct Citation {
	pub refs:	Vec<CiteRef>,
	pub note:	Option<usize>,	// the footnote number, for note styles
}

/// A bibliography entry: second-field-align's first field, then the rest.
pub type BibEntry = (Option<Node>, Vec<Node>);

/// One cite as the driver carries it between rounds.
#[derive(Clone, Debug)]
struct Pending {
	entry:		usize,
	initial:	usize,				// its index in the citation as written
	locator:	Option<(String, NumVal)>,
	form:		CiteForm,
	hidden:		bool,
	cite:		Cite,
	checked_disamb:	bool,
	delim:		Option<String>,			// the delimiter before it, when grouping sets one
	nodes:		Vec<Node>,
}

/// What every citation in a document decides together: the bibliography's order, each entry's
/// citation number, and every citation's final rendering. Built once from all the document's
/// citations, in order, as hayagriva's `BibliographyDriver::finish` is.
#[derive(Clone, Debug, Default)]
pub struct CiteState {
	order:	Vec<usize>,		// entry indices in bibliography order
	number:	Vec<usize>,		// per entry, from 1; 0 when uncited
	bib:	Vec<Option<Cite>>,	// per entry, what its bibliography entry inherits
	cites:	Vec<Vec<Node>>,		// each citation, rendered
}

impl CiteState {

	pub fn new<'a, I: Item>(
		style:	&'a Style,
		chain:	&'a Chain<'a>,
		lang:	Option<&'a str>,
		items:	&'a [I],
		cites:	&[Citation],
	)
		-> Self
	{
		// Each entry once, in the order it is first cited.
		let mut seen = vec![false; items.len()];
		let mut order = Vec::new();
		for r in cites.iter().flat_map(|c| c.refs.iter()) {
			if r.entry < items.len() && !seen[r.entry] {
				seen[r.entry] = true;
				order.push(r.entry);
			}
		}
		let bib_sort = style.bibliography.as_ref().and_then(|b| b.sort.as_ref());
		let nil: Vec<usize> = vec![0; items.len()];
		sort(style, chain, lang, items, &mut order, |&e| e, bib_sort, &nil);
		let mut number = vec![0; items.len()];
		for (i, &e) in order.iter().enumerate() {
			number[e] = i + 1;
		}

		// 1. Sort each citation, decide its cites' positions and render them without locators.
		let mut seen = vec![false; items.len()];
		let mut res: Vec<(Option<usize>, Vec<Pending>)> = Vec::new();
		let mut last: Option<(usize, Option<(String, NumVal)>, bool)> = None;
		for c in cites {
			let mut refs: Vec<(usize, &CiteRef)> = c.refs.iter().enumerate()
				.filter(|(_, r)| r.entry < items.len())
				.collect();
			sort(style, chain, lang, items, &mut refs, |(_, r)| r.entry, style.citation.sort.as_ref(), &number);
			let mut ps = Vec::new();
			for (initial, r) in refs {
				let e = r.entry;
				let near_note = c.note.is_some() && res.iter().rev()
					.take(style.citation.near_note_distance as usize)
					.any(|(n, ps)| n.is_some() && ps.iter().any(|p| p.entry == e));
				let first_note = c.note.map(|n| res.iter()
					.find_map(|(m, ps)| m.filter(|_| ps.iter().any(|p| p.entry == e)))
					.unwrap_or(n));
				let first = !seen[e];
				seen[e] = true;
				let cite = Cite {
					number:		Some(number[e]),
					first_note,
					first,
					ibid:		ibid(e, &r.locator, last.as_ref()),
					near_note,
					..Cite::default()
				};
				last = Some((e, r.locator.clone(), r.hidden));
				let mut p = Pending {
					entry:		e,
					initial,
					locator:	r.locator.clone(),
					form:		r.form,
					hidden:		r.hidden,
					cite,
					checked_disamb:	false,
					delim:		None,
					nodes:		Vec::new(),
				};
				let out = render::citation_entry(style, chain, &items[e], &p.cite, lang);
				p.checked_disamb = out.checked_disamb;
				p.nodes = out.nodes;
				ps.push(p);
			}
			res.push((c.note, ps));
		}

		// 4. Final positions on the final order, and the locators.
		let mut prev: Option<(usize, Option<(String, NumVal)>, bool)> = None;
		for (_, ps) in res.iter_mut() {
			for p in ps.iter_mut() {
				p.cite.ibid = ibid(p.entry, &p.locator, prev.as_ref());
				p.cite.locator = p.locator.clone();
				prev = Some((p.entry, p.locator.clone(), false));
			}
		}
		// Without a bibliography sort, numbers follow the cites as they now stand.
		if bib_sort.is_none() {
			let mut n = vec![0; items.len()];
			let mut next = 1;
			for p in res.iter_mut().flat_map(|(_, ps)| ps.iter_mut()) {
				if n[p.entry] == 0 {
					n[p.entry] = next;
					next += 1;
				}
				p.cite.number = Some(n[p.entry]);
			}
			number = n;
		}

		// The bibliography inherits each entry's first cite.
		let mut bib: Vec<Option<Cite>> = vec![None; items.len()];
		for p in res.iter().flat_map(|(_, ps)| ps.iter()) {
			if bib[p.entry].is_none() {
				bib[p.entry] = Some(Cite { locator: None, ..p.cite.clone() });
			}
		}

		// 5-6. The final render of each cite, then the citation's affixes and delimiters.
		let layout = &style.citation.layout;
		let fmt = Formatting::default().apply(&layout.look.fmt);
		let mut out = Vec::with_capacity(res.len());
		for (_, ps) in res.iter_mut() {
			for p in ps.iter_mut() {
				p.nodes = render::citation_entry(style, chain, &items[p.entry], &p.cite, lang).nodes;
			}
			if ps.iter().all(|p| p.hidden) {
				out.push(Vec::new());
				continue;
			}
			let affix = ps.iter().all(|p| p.form == CiteForm::Normal);
			let mut nodes = Vec::new();
			if let (true, Some(pre)) = (affix, layout.look.prefix.as_ref()) {
				nodes.push(Node::Text(pre.clone(), fmt));
			}
			for (i, p) in ps.iter().enumerate() {
				if p.hidden {
					continue;
				}
				if i != 0 {
					if let Some(d) = p.delim.as_ref().or(layout.delimiter.as_ref()) {
						nodes.push(Node::Text(d.clone(), fmt));
					}
				}
				nodes.push(Node::Elem(node::Elem {
					children:	p.nodes.clone(),
					display:	None,
					meta:		Some(Meta::Entry(p.initial)),
				}));
			}
			if let (true, Some(suf)) = (affix, layout.look.suffix.as_ref()) {
				if !node::last_text(&nodes).map_or(false, |t| t.ends_with(suf.as_str())) {
					nodes.push(Node::Text(suf.clone(), fmt));
				}
			}
			out.push(node::simplify(nodes));
		}
		Self { order, number, bib, cites: out }
	}

	/// The entries in bibliography order.
	pub fn order(&self) -> &[usize] {
		&self.order
	}

	pub fn number(&self, entry: usize) -> Option<usize> {
		self.number.get(entry).copied().filter(|&n| n > 0)
	}

	/// The `i`th citation as given to [`CiteState::new`], rendered; empty when every cite in it is
	/// hidden.
	pub fn citation(&self, i: usize) -> &[Node] {
		self.cites.get(i).map_or(&[], |c| c.as_slice())
	}
}

/// hayagriva's `IbidState::with_last`: the same entry as the last cite, when that was printed, is
/// ibid with an equal locator, and ibid-with-locator when only this one has a locator or they
/// differ; without a locator after one with it, it is only subsequent.
fn ibid(
	entry:	usize,
	loc:	&Option<(String, NumVal)>,
	last:	Option<&(usize, Option<(String, NumVal)>, bool)>,
)
	-> Ibid
{
	match last {
		Some((e, l, hidden)) if *e == entry && !*hidden => match (l == loc, loc.is_none()) {
			(true, _)	=> Ibid::Ibid,
			(false, true)	=> Ibid::No,
			(false, false)	=> Ibid::WithLocator,
		},
		_ => Ibid::No,
	}
}

/// Renders the bibliography: every cited entry in the style's order, each split into its first
/// field and the rest when the style asks for `second-field-align`. Empty when the style has no
/// bibliography.
pub fn render_bibliography<'a, I: Item>(
	style:	&'a Style,
	chain:	&'a Chain<'a>,
	lang:	Option<&'a str>,
	items:	&'a [I],
	state:	&CiteState,
)
	-> Vec<BibEntry>
{
	let align = style.bibliography.as_ref().map_or(false, |b| b.second_field_align.is_some());
	let mut out = Vec::new();
	for &e in &state.order {
		let cite = match state.bib.get(e).cloned().flatten() {
			Some(c)	=> Cite { number: state.number(e), ..c },
			None	=> Cite { number: state.number(e), first: true, ..Cite::default() },
		};
		let mut body = match render::bibliography_entry(style, chain, &items[e], &cite, lang) {
			Some(n)	=> node::simplify(n),
			None	=> return Vec::new(),
		};
		let prefix = match align {
			true	=> node::remove_any_meta(&mut body),
			false	=> None,
		};
		out.push((prefix, body));
	}
	out
}

// Sorting

/// Sorts cites, each naming its entry through `ent`, by a style's keys, stably. `num` gives each entry's citation number for a
/// `citation-number` key.
fn sort<'a, I: Item, T>(
	style:	&'a Style,
	chain:	&'a Chain<'a>,
	lang:	Option<&'a str>,
	items:	&'a [I],
	idx:	&mut [T],
	ent:	impl Fn(&T) -> usize,
	keys:	Option<&'a Sort>,
	num:	&[usize],
) {
	let keys = match keys {
		Some(k)	=> k,
		None	=> return,
	};
	idx.sort_by(|a, b| {
		let (a, b) = (ent(a), ent(b));
		for k in &keys.keys {
			let o = cmp_key(style, chain, lang, (&items[a], num[a]), (&items[b], num[b]), k);
			if o != Ordering::Equal {
				return o;
			}
		}
		Ordering::Equal
	});
}

/// One key's order. As in hayagriva, a key that one side lacks reverses the order as a
/// descending key does, which puts the entry without the value last.
fn cmp_key<'a, I: Item>(
	style:	&'a Style,
	chain:	&'a Chain<'a>,
	lang:	Option<&'a str>,
	(a, an):	(&'a I, usize),
	(b, bn):	(&'a I, usize),
	key:	&'a SortKey,
)
	-> Ordering
{
	fn opt<T>(a: Option<T>, b: Option<T>, f: impl Fn(&T, &T) -> Ordering) -> (Ordering, bool) {
		match (a, b) {
			(Some(a), Some(b))	=> (f(&a, &b), false),
			(Some(_), None)		=> (Ordering::Greater, true),
			(None, Some(_))		=> (Ordering::Less, true),
			(None, None)		=> (Ordering::Equal, true),
		}
	}
	let (ord, empty) = match &key.src {
		SortSrc::Var(v) => match var_kind(v) {
			VarKind::Standard => {
				let f = |i: &I| i.text(v, VarForm::Long).map(|c| c.to_string().to_lowercase());
				opt(f(a), f(b), |x, y| collate(x, y))
			}
			VarKind::Date => opt(a.date(v), b.date(v), date_cmp),
			VarKind::Name => {
				let demote = style.demote_particle != Demote::Never;
				let (pa, pb) = (a.names(v), b.names(v));
				for (x, y) in pa.iter().zip(pb.iter()) {
					let o = x.sort_cmp(y, demote);
					if o != Ordering::Equal {
						return o;
					}
				}
				(pa.len().cmp(&pb.len()), false)
			}
			VarKind::Number => {
				let f = |i: &I, n: usize| match v.as_str() {
					"citation-number"	=> Some(n.to_string()),
					_			=> i.number(v),
				};
				opt(f(a, an), f(b, bn), |x, y| num_cmp(x, y))
			}
			VarKind::Page => opt(a.page().map(|p| page_str(&p)), b.page().map(|p| page_str(&p)), |x, y| x.cmp(y)),
		},
		SortSrc::Macro(m) => {
			let ka = macro_key(style, chain, lang, a, an, m, key);
			let kb = macro_key(style, chain, lang, b, bn, m, key);
			opt(ka, kb, |x, y| collate(x, y))
		}
	};
	match empty || !key.ascending {
		true	=> ord.reverse(),
		false	=> ord,
	}
}

/// Compares two sort strings at primary strength, then by code point.
pub fn collate(a: &str, b: &str) -> Ordering {
	fn primary(s: &str) -> Vec<(u8, char)> {
		nfd(s).chars().filter(|&c| combining_class(c) == 0).map(|c| match c {
			c if c.is_whitespace()	=> (0, ' '),
			c if c.is_alphabetic()	=> (3, c.to_lowercase().next().unwrap_or(c)),
			c if c.is_numeric()	=> (2, c),
			c			=> (1, c),
		}).collect()
	}
	primary(a).cmp(&primary(b)).then_with(|| a.cmp(b))
}

/// hayagriva's `Date::csl_cmp`: year, then month and day, a missing part first.
fn date_cmp(a: &DateVal, b: &DateVal) -> Ordering {
	let part = |x: Option<u8>, y: Option<u8>| match (x, y) {
		(Some(x), Some(y))	=> x.cmp(&y),
		(Some(_), None)		=> Ordering::Greater,
		(None, Some(_))		=> Ordering::Less,
		(None, None)		=> a.approximate.cmp(&b.approximate),
	};
	a.year.cmp(&b.year).then_with(|| part(a.month, b.month)).then_with(|| part(a.day, b.day))
}

fn page_str(p: &PageVal) -> String {
	match p {
		PageVal::Typed(r)	=> { let mut s = String::new(); r.write(&mut s, None, None); s }
		PageVal::Str(s)		=> s.clone(),
	}
}

/// Numbers compare by their values in turn; anything not numeric as a string.
fn num_cmp(a: &str, b: &str) -> Ordering {
	match (Numeric::parse(a), Numeric::parse(b)) {
		(Some(x), Some(y))	=> x.nums.iter().map(|n| n.0).cmp(y.nums.iter().map(|n| n.0)),
		_			=> a.cmp(b),
	}
}

/// A macro key renders the macro as plain lower-cased text, with the key's name options laid
/// over the style's.
fn macro_key<'a, I: Item>(
	style:	&'a Style,
	chain:	&'a Chain<'a>,
	lang:	Option<&'a str>,
	item:	&'a I,
	num:	usize,
	name:	&str,
	key:	&SortKey,
)
	-> Option<String>
{
	let elems = match style.macros.get(name) {
		Some(e)	=> e,
		None	=> return None,
	};
	let cite = Cite { number: Some(num), first: true, ..Cite::default() };
	let names = NameOpts {
		et_al_min:			key.names_min,
		et_al_subsequent_min:		key.names_min,
		et_al_use_first:		key.names_use_first,
		et_al_subsequent_use_first:	key.names_use_first,
		et_al_use_last:			key.names_use_last,
		..NameOpts::default()
	};
	let mut ctx = Ctx::new(style, chain, item, &cite, lang, false).sorting(Sorting::Macro, names);
	ctx.render_all(elems);
	Some(node::plain(&ctx.flush()).to_lowercase())
}
