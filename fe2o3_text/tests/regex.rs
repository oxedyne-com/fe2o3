//! The regular expression engine and the Unicode property tables, held to an external oracle.
//!
//! `tests/regex_oracle/expected.txt` is what Perl's engine says of every pattern and haystack in
//! `corpus.txt`, written by `oracle.pl`; `props_expected.txt` is Perl's own Unicode property data,
//! written by `props.pl`.  Neither answer comes from fe2o3_text, so agreement is evidence rather
//! than self-consistency.  Rerun the scripts after changing the corpus.

use oxedyne_fe2o3_text::{
	regex::{
		Regex,
		Span,
	},
	unicode::{
		lookup::Partitioned,
		property::{
			Binary,
			CharClass,
			Extensions,
		},
		prop::{
			GeneralCategory,
			Script,
		},
	},
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
	collections::BTreeMap,
	fs,
	path::PathBuf,
};


fn data(name: &str) -> String {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("regex_oracle").join(name);
	match fs::read_to_string(&path) {
		Ok(s)	=> s,
		Err(e)	=> panic!("reading {:?}: {}", path, e),
	}
}

fn unesc(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	let mut it = s.chars();
	while let Some(c) = it.next() {
		if c != '\\' {
			out.push(c);
			continue;
		}
		match it.next() {
			Some('t')	=> out.push('\t'),
			Some('n')	=> out.push('\n'),
			Some('r')	=> out.push('\r'),
			Some(x)		=> out.push(x),
			None		=> out.push('\\'),
		}
	}
	out
}

/// The oracle's notation for one match's groups: `start,end` per group, `-` for one that took no
/// part.
fn spans(v: &[Option<Span>]) -> String {
	v.iter()
		.map(|s| match s {
			Some(s)	=> fmt!("{},{}", s.start, s.end),
			None	=> "-".to_string(),
		})
		.collect::<Vec<_>>()
		.join(" ")
}

#[test]
fn test_regex_agrees_with_the_perl_oracle() {
	let text = data("expected.txt");
	let mut fails: Vec<String> = Vec::new();
	let (mut cases, mut searches, mut hits, mut grouped, mut matches, mut reps) = (0, 0, 0, 0, 0, 0);

	let mut pat = String::new();
	let mut hay = String::new();
	let mut re: Option<Regex> = None;
	let mut want_m: Vec<String> = Vec::new();

	for line in text.lines() {
		let (tag, rest) = match line.split_once('\t') {
			Some(x)	=> x,
			None	=> (line, ""),
		};
		match tag {
			"P" => {
				pat = unesc(rest);
				re = match Regex::new(&pat) {
					Ok(r)	=> Some(r),
					Err(e)	=> {
						fails.push(fmt!("'{}' did not compile: {}", pat, e));
						None
					},
				};
			},
			"H" => {
				hay = unesc(rest);
				want_m.clear();
				cases += 1;
			},
			"A" => {
				let (at, want) = match rest.split_once('\t') {
					Some(x)	=> x,
					None	=> panic!("bad A line: {}", line),
				};
				let at: usize = match at.parse() {
					Ok(n)	=> n,
					Err(e)	=> panic!("bad offset in {}: {}", line, e),
				};
				let r = match &re {
					Some(r)	=> r,
					None	=> continue,
				};
				searches += 1;
				let got = match r.captures_at(&hay, at) {
					Ok(Some(c))	=> {
						hits += 1;
						if c.len() > 1 && c.spans()[1..].iter().any(|s| s.is_some()) {
							grouped += 1;
						}
						spans(c.spans())
					},
					Ok(None)	=> "none".to_string(),
					Err(e)		=> fmt!("error {}", e),
				};
				if got != want {
					fails.push(fmt!("'{}' on {:?} from byte {}: got {}, Perl says {}",
						pat, hay, at, got, want));
				}
			},
			"M" => want_m.push(rest.to_string()),
			"S" => {
				let r = match &re {
					Some(r)	=> r,
					None	=> continue,
				};
				let mut got_m = Vec::new();
				for c in r.captures_iter(&hay) {
					match c {
						Ok(c)	=> got_m.push(spans(c.spans())),
						Err(e)	=> got_m.push(fmt!("error {}", e)),
					}
				}
				matches += want_m.len();
				if got_m != want_m {
					fails.push(fmt!("'{}' on {:?} iterates to {:?}, Perl's emulation of the regex \
						crate says {:?}", pat, hay, got_m, want_m));
				}
				let want_s: Vec<String> = rest.split('\u{1F}').map(unesc).collect();
				match r.split(&hay) {
					Ok(got_s) => if got_s != want_s {
						fails.push(fmt!("'{}' splits {:?} into {:?}, not {:?}",
							pat, hay, got_s, want_s));
					},
					Err(e) => fails.push(fmt!("'{}' could not split {:?}: {}", pat, hay, e)),
				}
			},
			"R" => {
				let r = match &re {
					Some(r)	=> r,
					None	=> continue,
				};
				let (tpl, want) = match rest.split_once('\t') {
					Some((t, w))	=> (unesc(t), unesc(w)),
					None			=> panic!("bad R line: {}", line),
				};
				reps += 1;
				match r.replace_all(&hay, &tpl) {
					Ok(got) => if got != want {
						fails.push(fmt!("'{}' with '{}' on {:?} gives {:?}, not {:?}",
							pat, tpl, hay, got, want));
					},
					Err(e) => fails.push(fmt!("'{}' could not replace in {:?}: {}", pat, hay, e)),
				}
			},
			_ => {},
		}
	}

	// The comparison must have had something to compare: a truncated or empty oracle file would
	// otherwise pass.
	assert!(cases >= 150, "only {} cases were read", cases);
	assert!(searches >= 1100 && hits >= 800, "{} searches, {} hits", searches, hits);
	assert!(grouped >= 80, "only {} searches exercised capture groups", grouped);
	assert!(matches >= 250 && reps >= 4, "{} iterated matches, {} replacements", matches, reps);
	if !fails.is_empty() {
		panic!("{} disagreements with Perl, the first:\n{}", fails.len(),
			fails.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
	}
}

/// A run table from props_expected.txt: start code point and value, sorted.
fn runs(text: &str, tag: &str) -> Vec<(u32, String)> {
	let mut out = Vec::new();
	for line in text.lines() {
		let f: Vec<&str> = line.splitn(3, '\t').collect();
		if f.len() == 3 && f[0] == tag {
			match u32::from_str_radix(f[1], 16) {
				Ok(cp)	=> out.push((cp, f[2].to_string())),
				Err(e)	=> panic!("bad code point in {}: {}", line, e),
			}
		}
	}
	out
}

fn at(runs: &[(u32, String)], cp: u32) -> &str {
	let i = runs.partition_point(|(s, _)| *s <= cp);
	match i.checked_sub(1).and_then(|i| runs.get(i)) {
		Some((_, v))	=> v.as_str(),
		None			=> "",
	}
}

#[test]
fn test_unicode_properties_agree_with_perl() {
	let text = data("props_expected.txt");
	let gc	= runs(&text, "GC");
	let sc	= runs(&text, "SC");
	let scx	= runs(&text, "SCX");
	assert!(gc.len() > 3000 && sc.len() > 1500 && scx.len() > 1500,
		"the oracle file is short: {} {} {}", gc.len(), sc.len(), scx.len());

	let mut bins: Vec<(Binary, Vec<u32>)> = Vec::new();
	for line in text.lines() {
		let f: Vec<&str> = line.split('\t').collect();
		if f.len() != 3 || f[0] != "BIN" || f[2] == "unknown" {
			continue;
		}
		let b = match Binary::find(f[1]) {
			Some(b)	=> b,
			None	=> panic!("Perl knows the binary property {} and the tables do not", f[1]),
		};
		let inv: Vec<u32> = f[2].split(' ').map(|x| match u32::from_str_radix(x, 16) {
			Ok(v)	=> v,
			Err(e)	=> panic!("bad inversion list entry {}: {}", x, e),
		}).collect();
		bins.push((b, inv));
	}
	assert!(bins.len() >= 50, "only {} binary properties compared", bins.len());

	// Perl's Unicode is older than the tables', so a disagreement is accepted only where
	// props_changed.txt, a diff of the two UCD releases' own files, says Unicode changed that
	// property of that code point.
	let mut changed: std::collections::BTreeSet<(String, u32)> = std::collections::BTreeSet::new();
	for line in data("props_changed.txt").lines() {
		if let Some((p, cp)) = line.split_once('\t') {
			match u32::from_str_radix(cp, 16) {
				Ok(v)	=> { changed.insert((p.to_string(), v)); },
				Err(e)	=> panic!("bad code point in {}: {}", line, e),
			}
		}
	}
	assert!(changed.len() > 500, "the change list is short: {}", changed.len());

	let mut diffs: BTreeMap<String, Vec<String>> = BTreeMap::new();
	let mut explained = 0usize;
	let mut note = |prop: &str, cp: u32, what: String| {
		if changed.contains(&(prop.to_string(), cp)) {
			explained += 1;
		} else {
			diffs.entry(prop.to_string()).or_default().push(fmt!("U+{:04X} {}", cp, what));
		}
	};
	let mut compared = 0u32;
	for cp in 0..=0x10FFFFu32 {
		let c = match char::from_u32(cp) {
			Some(c)	=> c,
			None	=> continue,
		};
		let pgc = at(&gc, cp);
		if pgc == "Cn" {
			continue;
		}
		compared += 1;
		let ours = GeneralCategory::of(c).abbr();
		if ours != pgc {
			note("gc", cp, fmt!("{} vs {}", ours, pgc));
		}
		let ours = Script::of(c).name();
		let psc = at(&sc, cp);
		if ours != psc {
			note("sc", cp, fmt!("{} vs {}", ours, psc));
		}
		let mut ours: Vec<&str> = Extensions::of(c).as_slice().iter().map(|s| s.name()).collect();
		ours.sort();
		let ours = ours.join(" ");
		let pscx = at(&scx, cp);
		if ours != pscx {
			note("scx", cp, fmt!("{} vs {}", ours, pscx));
		}
		for (b, inv) in &bins {
			let theirs = inv.partition_point(|s| *s <= cp) % 2 == 1;
			if b.contains(c) != theirs {
				note(b.name(), cp, fmt!("{} vs {}", b.contains(c), theirs));
			}
		}
	}
	assert!(compared > 280_000, "only {} code points compared", compared);

	let total: usize = diffs.values().map(|v| v.len()).sum();
	let report = diffs.iter()
		.map(|(k, v)| fmt!("{}: {} ({})", k, v.len(), v.iter().take(6).cloned().collect::<Vec<_>>().join(", ")))
		.collect::<Vec<_>>()
		.join("\n");
	assert!(total == 0, "{} differences from Perl's Unicode data that Unicode did not make \
		({} that it did):\n{}", total, explained, report);
}

#[test]
fn test_property_names_resolve_as_the_regex_crate_resolves_them() {
	for (name, want) in [
		("L",					CharClass::parse("gc=L")),
		("Greek",				CharClass::parse("sc=Greek")),
		("isGreek",				CharClass::parse("Script=Grek")),
		("cf",					CharClass::parse("gc=Cf")),
		("sc",					CharClass::parse("gc=Sc")),
		("lc",					CharClass::parse("gc=LC")),
		("White Space",			CharClass::parse("wspace")),
	] {
		let got = CharClass::parse(name);
		match (got, want) {
			(Ok(g), Ok(w))	=> assert_eq!(g, w, "'{}'", name),
			(g, w)			=> panic!("'{}': {:?} vs {:?}", name, g, w),
		}
	}
	assert!(CharClass::parse("scx=Greek").is_ok());
	assert!(CharClass::parse("Age=6.0").is_err(), "an unsupported property is refused, not guessed");
	assert!(CharClass::parse("Klingon").is_err());
}

#[test]
fn test_regex_agrees_with_the_regex_crate_suite() {
	// rust_suite.txt is the Rust `regex` crate's own test data, converted by rust_suite.py: the
	// answers Typst's `regex(...)` gives, stated by the crate that gives them.
	let text = data("rust_suite.txt");
	let mut fails: Vec<String> = Vec::new();
	let mut lenient: Vec<String> = Vec::new();
	let (mut cases, mut compared) = (0, 0);

	let (mut name, mut flags, mut pat, mut hay) = (String::new(), String::new(), String::new(), String::new());
	let mut refuse = false;
	let mut limit: Option<usize> = None;
	let mut want: Vec<String> = Vec::new();

	for line in text.lines() {
		let f: Vec<&str> = line.splitn(3, '\t').collect();
		match f.first().copied() {
			Some("T") => {
				name	= f.get(1).copied().unwrap_or("").to_string();
				flags	= f.get(2).copied().unwrap_or("").to_string();
				refuse	= false;
				limit	= None;
				want.clear();
			},
			Some("P") => pat = unesc(f.get(1).copied().unwrap_or("")),
			Some("H") => hay = unesc(f.get(1).copied().unwrap_or("")),
			Some("C") => refuse = true,
			Some("L") => limit = f.get(1).and_then(|n| n.parse().ok()),
			Some("M") => want.push(f.get(1).copied().unwrap_or("").to_string()),
			Some("E") => {
				cases += 1;
				let re = match Regex::with_case(&pat, flags.contains('i')) {
					Ok(r) => {
						if refuse {
							lenient.push(fmt!("{}: '{}'", name, pat));
							continue;
						}
						r
					},
					Err(e) => {
						if !refuse {
							fails.push(fmt!("{}: '{}' did not compile: {}", name, pat, e));
						}
						continue;
					},
				};
				let mut got: Vec<String> = Vec::new();
				for c in re.captures_iter(&hay) {
					if limit.map(|n| got.len() >= n).unwrap_or(false) {
						break;
					}
					match c {
						Ok(c) => {
							// An anchored search may only begin at the start.
							if flags.contains('a') && c.whole().start != 0 {
								break;
							}
							// The crate states some cases by the whole match alone.
							let whole_only = want.first().map(|w| !w.contains(' ')).unwrap_or(true);
							got.push(if whole_only { spans(&c.spans()[..1]) } else { spans(c.spans()) });
						},
						Err(e) => {
							got.push(fmt!("error {}", e));
							break;
						},
					}
					if flags.contains('a') {
						break;
					}
				}
				compared += 1;
				if got != want {
					fails.push(fmt!("{}: '{}' on {:?} gives {:?}, the crate says {:?}",
						name, pat, hay, got, want));
				}
			},
			_ => {},
		}
	}
	assert!(cases >= 700 && compared >= 650, "{} cases, {} compared", cases, compared);
	// Where the crate refuses a pattern this engine reads, the difference is a documented
	// leniency; list them so a new one is seen.
	assert!(lenient.len() <= LENIENT, "accepted what the crate refuses:\n{}", lenient.join("\n"));
	if !fails.is_empty() {
		panic!("{} disagreements with the regex crate, the first:\n{}", fails.len(),
			fails.iter().take(60).cloned().collect::<Vec<_>>().join("\n"));
	}
}

/// Patterns the `regex` crate refuses and this engine reads; see the module notes on `{`.
const LENIENT: usize = 0;

#[test]
fn test_a_group_in_a_repetition_keeps_its_last_capture() {
	// Perl clears a group's capture on each new iteration of an enclosing repetition; the regex
	// crate, like Python's `re`, keeps it.  These answers are Python 3's:
	//   re.search(r'(a(b)?)+', 'aba')   -> spans (0,3) (2,3) (1,2)
	//   re.search(r'((a)|b)+', 'ab')    -> spans (0,2) (1,2) (0,1)
	//   re.search(r'(?:(a)|b)*', 'ab')  -> spans (0,2) (0,1)
	//   re.search(r'(a|(b))+', 'ba')    -> spans (0,2) (1,2) (0,1)
	for (pat, hay, want) in [
		("(a(b)?)+",	"aba",	"0,3 2,3 1,2"),
		("((a)|b)+",	"ab",	"0,2 1,2 0,1"),
		("(?:(a)|b)*",	"ab",	"0,2 0,1"),
		("(a|(b))+",	"ba",	"0,2 1,2 0,1"),
	] {
		let re = Regex::new(pat).expect("compile");
		let c = re.captures(hay).expect("search").expect("a match");
		assert_eq!(spans(c.spans()), want, "'{}' on '{}'", pat, hay);
	}
}
