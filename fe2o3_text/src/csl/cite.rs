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
		Disamb,
		Ibid,
		NForm,
		NameDisamb,
		Sorting,
		Special,
	},
	person::Person,
	style::{
		Collapse,
		Demote,
		GivenRule,
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
	first_name:	Option<NameDisamb>,		// the first `names` element's forms, as first rendered
	delim:		Option<String>,			// the delimiter before it, when grouping sets one
	group:		Option<usize>,			// its cite group, numbered from 1
	special:	Option<Special>,		// what collapsing leaves of it
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
					first_name:	None,
					delim:		None,
					group:		None,
					special:	None,
					nodes:		Vec::new(),
				};
				let out = render::citation_entry(style, chain, &items[e], &p.cite, lang, None, p.form);
				p.checked_disamb = out.checked_disamb;
				p.first_name = out.first_name;
				p.nodes = out.nodes;
				ps.push(p);
			}
			res.push((c.note, ps));
		}

		// 2. Up to three rounds of disambiguation. Each set of cites that read alike tries names,
		// then `choose`, then a year-suffix, and each entry takes the furthest state its cites
		// reached.
		for _ in 0..3 {
			let sets = ambiguous(&res);
			if sets.is_empty() {
				break;
			}
			let mut marks: Vec<Option<Disamb>> = vec![None; items.len()];
			for set in &sets {
				let mut found = disamb_names(style, &res, set);
				if found.is_empty() && marks.iter().all(Option::is_none) {
					found = disamb_choose(&res, set);
				}
				if found.is_empty() && marks.iter().all(Option::is_none) {
					found = disamb_year_suffix(style, items, &res, set);
				}
				for (e, d) in found {
					marks[e] = Some(match marks[e].take() {
						Some(m)	=> m.max(d),
						None	=> d,
					});
				}
			}
			if marks.iter().all(Option::is_none) {
				break;
			}
			for p in res.iter_mut().flat_map(|(_, ps)| ps.iter_mut()) {
				if let Some(d) = &marks[p.entry] {
					p.cite.disamb = d.clone();
					p.nodes = render::citation_entry(style, chain, &items[p.entry], &p.cite, lang, None, p.form).nodes;
				}
			}
		}

		// 3. Cites with equal names gather behind the first of them.
		let layout = &style.citation.layout;
		let group_delim = style.citation.cite_group_delimiter.clone()
			.or_else(|| layout.delimiter.clone().filter(|_| style.citation.collapse.is_some()));
		if let Some(delim) = group_delim {
			for (_, ps) in res.iter_mut() {
				group(ps, &delim);
			}
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

		// 5-6. Collapsing, the final render of each cite, then the citation's affixes and
		// delimiters.
		let fmt = Formatting::default().apply(&layout.look.fmt);
		let mut out = Vec::with_capacity(res.len());
		for (_, ps) in res.iter_mut() {
			if ps.iter().all(|p| p.form == CiteForm::Normal) {
				collapse(style, ps);
			}
			for p in ps.iter_mut() {
				p.nodes = match p.form {
					CiteForm::Year	=> render::year_only(chain, &items[p.entry]),
					f		=> render::citation_entry(style, chain, &items[p.entry], &p.cite, lang, p.special.clone(), f).nodes,
				};
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

// Disambiguation

type Set = Vec<(usize, usize)>;	// cites by citation and place in it
type Res = [(Option<usize>, Vec<Pending>)];

/// The sets of cites, still open to a year-suffix, that render alike though their entries differ.
fn ambiguous(res: &Res) -> Vec<Set> {
	let mut keys: Vec<(String, Option<usize>, (usize, usize))> = Vec::new();	// key, its set, first cite
	let mut sets: Vec<Set> = Vec::new();
	for (i, (_, ps)) in res.iter().enumerate() {
		for (j, p) in ps.iter().enumerate() {
			if !p.cite.disamb.may_year_suffix() {
				continue;
			}
			let key = node::styled(&p.nodes);
			match keys.iter_mut().find(|(k, _, _)| *k == key) {
				Some((_, Some(set), _))	=> sets[*set].push((i, j)),
				Some((_, set, first))	=> {
					*set = Some(sets.len());
					sets.push(vec![*first, (i, j)]);
				}
				None => keys.push((key, None, (i, j))),
			}
		}
	}
	sets.into_iter().filter(|set| {
		let first = set.first().map(|&(i, j)| res[i].1[j].entry);
		set.iter().any(|&(i, j)| Some(res[i].1[j].entry) != first)
	}).collect()
}

/// Every person a cite's `names` elements hold, written or not.
fn names_of(nodes: &[Node], out: &mut Vec<Person>) {
	for n in nodes {
		if let Node::Elem(e) = n {
			if let Some(Meta::Names(ns)) = &e.meta {
				out.extend(ns.iter().flat_map(|(ps, _)| ps.iter()).cloned());
			}
			names_of(&e.children, out);
		}
	}
}

/// One cite of an ambiguous set, with its people.
struct DisambData<'r> {
	at:	(usize, usize),
	names:	Vec<Person>,
	p:	&'r Pending,
}

/// Do the first `n` names, as the given-name rule compares them, tell `x` from the rest?
fn is_disambiguated(n: usize, xs: &[DisambData], x: &DisambData, rule: Option<GivenRule>) -> bool {
	let init = |p: &Person| {
		let mut g = String::new();
		let _ = p.initials(&mut g, None, false);
		Person { given: Some(g), ..p.clone() }
	};
	let bare = |p: &Person| Person { given: None, ..p.clone() };
	let parts = |d: &DisambData| -> Vec<Person> {
		d.names.iter().take(n).enumerate().map(|(i, p)| match rule {
			Some(GivenRule::AllNames | GivenRule::ByCite)	=> p.clone(),
			Some(GivenRule::AllNamesWithInitials)		=> init(p),
			Some(GivenRule::PrimaryName) if i == 0		=> p.clone(),
			Some(GivenRule::PrimaryNameWithInitials) if i == 0	=> init(p),
			Some(GivenRule::PrimaryName | GivenRule::PrimaryNameWithInitials) | None => bare(p),
		}).collect()
	};
	let mine = parts(x);
	xs.iter().all(|y| y.at == x.at || parts(y) != mine)
}

/// hayagriva's `disamb_cite_add_given_name`: a namesake with other initials needs the initial, one
/// with the same initials the full given name.
fn add_given(x: &Person, xs: &[&Person], form: &mut NForm) -> bool {
	if *form == NForm::Count || *form == NForm::LongFull {
		return false;
	}
	let mut changed = false;
	for p in xs {
		if x == *p || x.family != p.family {
			continue;
		}
		let (mut xi, mut pi) = (String::new(), String::new());
		let _ = x.initials(&mut xi, None, false);
		let _ = p.initials(&mut pi, None, false);
		if xi == pi {
			*form = NForm::LongFull;
			return true;
		}
		if *form != NForm::LongInit {
			*form = NForm::LongInit;
			changed = true;
		}
	}
	changed
}

/// hayagriva's `disambiguate_names`: show more names, one more each pass, and under the by-cite
/// rule widen the forms of namesakes, until each cite stands apart.
fn disamb_names(style: &Style, res: &Res, set: &Set) -> Vec<(usize, Disamb)> {
	let group: Vec<DisambData> = set.iter().map(|&(i, j)| {
		let mut names = Vec::new();
		names_of(&res[i].1[j].nodes, &mut names);
		DisambData { at: (i, j), names, p: &res[i].1[j] }
	}).collect();
	let max = match style.citation.add_names {
		true	=> group.iter().map(|d| d.names.len()).max().unwrap_or(0),
		false	=> 1,
	};
	let rule = match style.citation.add_givenname {
		true	=> Some(style.citation.givenname_rule),
		false	=> None,
	};
	let mut changed: Vec<((usize, usize), NameDisamb)> = Vec::new();
	let slot = |changed: &Vec<((usize, usize), NameDisamb)>, d: &DisambData| -> Option<NameDisamb> {
		match changed.iter().find(|(at, _)| *at == d.at) {
			Some((_, n))	=> Some(n.clone()),
			None		=> match &d.p.cite.disamb {
				Disamb::Names(n)	=> Some(n.clone()),
				_			=> d.p.first_name.clone(),
			},
		}
	};
	let put = |changed: &mut Vec<((usize, usize), NameDisamb)>, at: (usize, usize), n: NameDisamb| {
		match changed.iter_mut().find(|(a, _)| *a == at) {
			Some((_, old))	=> *old = n,
			None		=> changed.push((at, n)),
		}
	};
	let mut done: Vec<(usize, usize)> = Vec::new();
	for i in 1..=max {
		let before = done.len();
		if before == group.len() {
			break;
		}
		for d in &group {
			if done.contains(&d.at) || !d.p.cite.disamb.may_names() || !is_disambiguated(i, &group, d, rule) {
				continue;
			}
			done.push(d.at);
			if rule != Some(GivenRule::ByCite) {
				continue;
			}
			if let (Some(mut props), Some(x)) = (slot(&changed, d), d.names.get(i - 1)) {
				let others: Vec<&Person> = group.iter()
					.filter(|o| o.at != d.at)
					.flat_map(|o| o.names.iter())
					.collect();
				if add_given(x, &others, props.form_mut(i - 1)) {
					put(&mut changed, d.at, props);
				}
			}
		}
		if done.len() > before {
			for d in &group {
				if let Some(mut s) = slot(&changed, d) {
					s.add_name(i);
					put(&mut changed, d.at, s);
				}
			}
		}
	}
	changed.into_iter().map(|((i, j), n)| (res[i].1[j].entry, Disamb::Names(n))).collect()
}

/// hayagriva's `disambiguate_with_choose`: every cite that asked `disambiguate`, but the first,
/// takes the `choose` branch.
fn disamb_choose(res: &Res, set: &Set) -> Vec<(usize, Disamb)> {
	let ps: Vec<&Pending> = set.iter().map(|&(i, j)| &res[i].1[j]).collect();
	if !ps.iter().any(|p| p.checked_disamb && p.cite.disamb.may_choose()) {
		return Vec::new();
	}
	ps.iter().filter(|p| p.checked_disamb).skip(1).map(|p| (p.entry, Disamb::Choose)).collect()
}

/// hayagriva's `disambiguate_year_suffix`: when some cite writes a date and the style allows it,
/// each entry of the set takes a suffix in the order it first appears.
fn disamb_year_suffix<I: Item>(style: &Style, items: &[I], res: &Res, set: &Set) -> Vec<(usize, Disamb)> {
	const DATES: [&str; 6] = ["issued", "accessed", "available-date", "event-date", "submitted", "original-date"];
	let dated = res.iter().flat_map(|(_, ps)| ps.iter()).any(|p| {
		let has = DATES.iter().any(|v| items[p.entry].date(v).is_some());
		node::find_elem(&p.nodes, &|e| e.meta == Some(Meta::Date) || (has && e.meta == Some(Meta::CitationLabel))).is_some()
	});
	let open = |&(i, j): &(usize, usize)| res[i].1[j].cite.disamb.may_year_suffix();
	if !dated || !style.citation.add_year_suffix || !set.iter().any(open) {
		return Vec::new();
	}
	let mut entries: Vec<usize> = Vec::new();
	for at in set.iter().filter(|at| open(at)) {
		let e = res[at.0].1[at.1].entry;
		if !entries.contains(&e) {
			entries.push(e);
		}
	}
	entries.into_iter().enumerate().map(|(i, e)| (e, Disamb::YearSuffix(i as u8))).collect()
}

// Grouping and collapsing

/// hayagriva's grouping: each cite whose first `names` element equals an earlier one's moves up
/// behind the last of them and takes `delim` before it, which the one it follows gives up. As in
/// hayagriva, the group number recorded is the count of groups so far, and the indices kept for
/// earlier groups are not shifted by a later move.
fn group(ps: &mut [Pending], delim: &str) {
	let mut map: Vec<(node::Elem, usize)> = Vec::new();	// a names element, the last index with it
	let mut count = 0;
	for i in 0..ps.len() {
		let names = match node::find_elem(&ps[i].nodes, &|e| matches!(e.meta, Some(Meta::Names(_)))) {
			Some(e)	=> e.clone(),
			None	=> continue,
		};
		let mut prev = None;
		let target = match map.iter_mut().find(|(n, _)| *n == names) {
			Some((_, last)) => {
				prev = Some(*last);
				*last += 1;
				*last
			}
			None => {
				count += 1;
				map.push((names, i));
				i
			}
		};
		let mut pos = i;
		while target < pos {
			ps.swap(pos, pos - 1);
			pos -= 1;
		}
		ps[target].delim = Some(delim.to_string());
		ps[target].group = Some(count);
		if let Some(p) = prev {
			ps[p].delim = None;
		}
	}
}

/// hayagriva's `collapse_items`: runs of three or more consecutive citation numbers become a
/// range, and a group's later cites keep only their year, or only their year-suffix.
fn collapse(style: &Style, ps: &mut [Pending]) {
	let c = &style.citation;
	let after = c.after_collapse_delimiter.clone().or_else(|| c.layout.delimiter.clone());
	let group_delim = c.cite_group_delimiter.clone();
	let suffix_delim = c.year_suffix_delimiter.clone().or_else(|| group_delim.clone());
	match c.collapse {
		Some(Collapse::CitationNumber) => {
			let mut range: Option<(usize, usize)> = None;	// its first and last index
			let mut just = false;				// the last range collapsed
			let end_range = |ps: &mut [Pending], range: &mut Option<(usize, usize)>, just: &mut bool| {
				let use_after = *just;
				*just = false;
				if let Some((start, end)) = *range {
					if use_after {
						ps[start].delim = after.clone();
					}
					if start + 1 < end {
						ps[end].delim = Some("–".to_string());
						for p in &mut ps[start + 1..end] {
							p.hidden = true;
						}
						*just = true;
					}
				}
				*range = None;
			};
			for i in 0..ps.len() {
				let numbered = node::find_elem(&ps[i].nodes, &|e| e.meta == Some(Meta::CitationNumber)).is_some();
				if ps[i].hidden || !numbered {
					end_range(ps, &mut range, &mut just);
					continue;
				}
				let n = ps[i].cite.number;
				match range {
					Some((start, end)) if end + 1 == i && ps[end].cite.number.map(|m| m + 1) == n => {
						range = Some((start, i));
					}
					_ => {
						end_range(ps, &mut range, &mut just);
						range = Some((i, i));
					}
				}
			}
			end_range(ps, &mut range, &mut just);
		}
		Some(kind @ (Collapse::Year | Collapse::YearSuffix | Collapse::YearSuffixRanged)) => {
			let mut cur: Option<(usize, usize)> = None;	// where the group began, and its number
			for i in 0..ps.len() {
				match cur {
					Some((_, g)) if Some(g) == ps[i].group => {
						let suffixed = matches!(ps[i].cite.disamb, render::Disamb::YearSuffix(s) if s > 0);
						if kind == Collapse::YearSuffix && suffixed {
							ps[i].delim = suffix_delim.clone();
							ps[i].special = Some(Special::OnlyYearSuffix);
							continue;
						}
						ps[i].delim = group_delim.clone();
						ps[i].special = Some(Special::OnlyFirstDate);
					}
					Some((start, _)) if start + 1 < i => {
						ps[i].delim = after.clone();
						cur = ps[i].group.map(|g| (i, g));
					}
					_ => cur = ps[i].group.map(|g| (i, g)),
				}
			}
		}
		None => {}
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
