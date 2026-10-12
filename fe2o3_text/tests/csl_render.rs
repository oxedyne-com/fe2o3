//! B2a foundations: the case folder, numbers and the output tree. Renderer tests against Typst
//! 0.15.1 follow in tests/data/csl/expect.
use oxedyne_fe2o3_text::csl::{
	case::Case,
	item::{
		ChunkKind,
		Chunked,
		NumVal,
		Numeric,
	},
	node::{
		self,
		Formatting,
		Node,
	},
};

#[test]
fn test_title_case_as_typst() {
	// From Typst 0.15.1 with chicago-author-date (tests/data/csl/expect/chicago-author-date.html).
	assert_eq!(Case::Title.transform("Example Site: the home page"), "Example Site: The Home Page");
	assert_eq!(Case::Title.transform("on the art of war"), "On the Art of War");
	assert_eq!(Case::Title.transform("an NASA report"), "An NASA Report");
	assert_eq!(Case::Sentence.transform("The Study of Paris"), "The study of Paris");
	assert_eq!(Case::FirstUpper.transform("hello world"), "Hello world");
	assert_eq!(Case::AllUpper.transform("hello world"), "Hello World");
	assert_eq!(Case::Uppercase.transform("abc"), "ABC");
}

#[test]
fn test_numeric_parse() {
	assert_eq!(Numeric::parse("12").map(|n| n.to_string()), Some("12".to_string()));
	assert_eq!(Numeric::parse("12-15").map(|n| n.to_string()), Some("12–15".to_string()));
	assert_eq!(Numeric::parse("2nd").map(|n| (n.will_transform(), n.to_string())), Some((false, "2nd".to_string())));
	assert_eq!(Numeric::parse("1, 3 & 5").map(|n| n.is_plural(false)), Some(true));
	assert!(Numeric::parse("twelve").is_none());
	assert!(matches!(NumVal::from_str("Supplement A"), NumVal::Str(_)));
}

#[test]
fn test_chunked_and_tree() {
	let mut c = Chunked::normal("A Study of ");
	c.push(ChunkKind::Verbatim, "DNA");
	c.push(ChunkKind::Math, "x^2");
	assert_eq!(c.to_string(), "A Study of DNA$x^2$");
	let f = Formatting::default();
	let tree = node::simplify(vec![
		Node::Text("a".into(), f),
		Node::Text("b".into(), f),
		Node::Elem(node::Elem { children: vec![Node::Text("c".into(), f)], display: None, meta: None }),
	]);
	assert_eq!(tree.len(), 2);
	assert_eq!(node::plain(&tree), "abc");
}

// Renderers against Typst 0.15.1 (tests/data/csl/expect).
#[cfg(feature = "csl-archive")]
mod typst {
	use oxedyne_fe2o3_core::prelude::*;
	use oxedyne_fe2o3_text::csl::{
		archive,
		Citation,
		CiteForm,
		CiteRef,
		CiteState,
		item::{
			ChunkKind,
			Chunked,
			DateVal,
			Item,
			NumVal,
		},
		node::{
			Meta,
			Node,
		},
		page::PageVal,
		person::Person,
		render_bibliography,
		style::{
			Display,
			FontStyle,
			FontWeight,
			VAlign,
			VarForm,
		},
	};

	/// A `refs.bib` entry as hayagriva hands it to CSL.
	#[derive(Default)]
	struct Fx {
		key:		&'static str,
		ty:		&'static str,
		title:		Chunked,
		container:	Option<&'static str>,
		volume:		Option<&'static str>,
		issue:		Option<&'static str>,
		edition:	Option<&'static str>,
		publisher:	Option<&'static str>,
		doi:		Option<&'static str>,
		url:		Option<&'static str>,
		vol_title:	Option<&'static str>,
		also:		Option<&'static str>,	// a second CSL type the entry matches
		author:		Vec<Person>,
		editor:		Vec<Person>,
		issued:		Option<DateVal>,
		page:		Option<&'static str>,
	}

	impl Item for Fx {
		fn text(&self, var: &str, _form: VarForm) -> Option<Chunked> {
			match var {
				"title"			=> Some(self.title.clone()),
				"container-title"	=> self.container.map(Chunked::normal),
				"volume-title"		=> self.vol_title.map(Chunked::normal),
				"publisher"		=> self.publisher.map(Chunked::normal),
				"DOI"			=> self.doi.map(Chunked::normal),
				"URL"			=> self.url.map(Chunked::normal),
				_			=> None,
			}
		}
		fn number(&self, var: &str) -> Option<String> {
			match var {
				"volume"	=> self.volume,
				"issue"		=> self.issue,
				"edition"	=> self.edition,
				_		=> None,
			}.map(str::to_string)
		}
		fn csl_type(&self) -> &str {
			self.ty
		}
		fn is_type(&self, t: &str) -> bool {
			self.ty == t || self.also == Some(t)
		}
		fn page(&self) -> Option<PageVal> {
			self.page.map(PageVal::from_str)
		}
		fn names(&self, var: &str) -> &[Person] {
			match var {
				"author"	=> &self.author,
				"editor"	=> &self.editor,
				_		=> &[],
			}
		}
		fn date(&self, var: &str) -> Option<DateVal> {
			match var {
				"issued"	=> self.issued,
				_		=> None,
			}
		}
	}

	fn p(family: &str, given: &str) -> Person {
		Person::new(family, Some(given))
	}

	fn ymd(year: i32, month: Option<u8>, day: Option<u8>) -> Option<DateVal> {
		Some(DateVal { year, month, day, ..DateVal::default() })
	}

	fn chunks(parts: &[(ChunkKind, &str)]) -> Chunked {
		let mut c = Chunked::default();
		for (k, s) in parts {
			c.push(*k, s);
		}
		c
	}

	fn fixtures() -> Vec<Fx> {
		vec![
			Fx {
				key: "a1", ty: "article-journal",
				title: chunks(&[(ChunkKind::Normal, "A Study of "), (ChunkKind::Verbatim, "DNA"), (ChunkKind::Normal, " Repair in the Lab")]),
				container: Some("Journal of Testing"), volume: Some("12"), issue: Some("3"), doi: Some("10.1000/xyz123"),
				..Fx::default()
			},
			Fx {
				key: "b1", ty: "book", title: Chunked::normal("The Art of Testing"),
				publisher: Some("Acme Press"), edition: Some("2"),
				..Fx::default()
			},
			Fx {
				key: "w1", ty: "webpage", title: Chunked::normal("Example Site: the home page"),
				url: Some("https://example.com/page"),
				..Fx::default()
			},
			Fx {
				key: "a2", ty: "article-journal",
				// biblatex 0.12 reads `On {$x^2$} and ...` with an empty verbatim chunk either side of
				// the math. The one after it leaves the folder mid-word, which keeps title case's "and".
				title: chunks(&[
					(ChunkKind::Normal, "On "),
					(ChunkKind::Verbatim, ""),
					(ChunkKind::Math, "x^2"),
					(ChunkKind::Verbatim, ""),
					(ChunkKind::Normal, " and other things"),
				]),
				container: Some("Annals of Maths"), volume: Some("7"),
				..Fx::default()
			},
			// biblatex 0.12 reads "Doe, Jr., Jane" with the suffix, and hayagriva sets no comma
			// before it (`comma_suffix: false`) for every BibTeX name.
			Fx {
				key: "n1", ty: "article-journal", title: Chunked::normal("Names in Order"),
				container: Some("Journal of Names"), volume: Some("3"), page: Some("123-129"),
				author: vec![
					p("Smith", "John"),
					p("Beethoven", "Ludwig").with_particle("van"),
					p("Doe", "Jane").with_suffix("Jr."),
				],
				issued: Some(DateVal::year(2020)),
				..Fx::default()
			},
			Fx {
				key: "n2", ty: "book", title: Chunked::normal("Annual Report"),
				publisher: Some("Acme Press"),
				author: vec![Person::literal("Acme Corporation")],
				issued: ymd(2018, Some(3), Some(15)),
				..Fx::default()
			},
			Fx {
				key: "n3", ty: "article-journal", title: Chunked::normal("Eight Authors"),
				container: Some("Many Hands"), volume: Some("9"), issue: Some("2"), page: Some("1001-1012"),
				author: vec![
					p("Alpha", "Ann"), p("Beta", "Bob"), p("Gamma", "Gus"), p("Delta", "Dan"),
					p("Epsilon", "Eve"), p("Zeta", "Zed"), p("Eta", "Ed"), p("Theta", "Tom"),
				],
				// `2019/2021` reaches hayagriva as its end date.
				issued: Some(DateVal::year(2021)),
				..Fx::default()
			},
			// An @inproceedings is an article whose parent is the proceedings, whose title hayagriva
			// answers for both container-title and volume-title.
			Fx {
				key: "n4", ty: "paper-conference", title: Chunked::normal("Approximate Things"),
				container: Some("Proceedings of Circa"), vol_title: Some("Proceedings of Circa"),
				page: Some("45-47"),
				author: vec![p("Müller", "Hans-Peter"), p("Cruz", "María").with_particle("de la")],
				editor: vec![p("Editor", "Edith")],
				issued: Some(DateVal { year: 2015, approximate: true, ..DateVal::default() }),
				..Fx::default()
			},
			// A @misc with a URL matches both webpage and document in hayagriva.
			Fx {
				key: "n5", ty: "webpage", also: Some("document"), title: Chunked::normal("A Month Only"),
				url: Some("https://example.org/m"),
				author: vec![p("O'Neil", "Kate")],
				issued: ymd(2021, Some(7), None),
				..Fx::default()
			},
			Fx {
				key: "n6", ty: "article-journal", title: Chunked::normal("Twenty-One Authors"),
				container: Some("Crowd Science"), volume: Some("1"), page: Some("321-328"),
				author: (1..=21).map(|i| {
					let given = format!("A{}", (b'a' + (i as u8 - 1)) as char);
					Person::new(&format!("A{:02}", i), Some(&given))
				}).collect(),
				issued: Some(DateVal::year(2017)),
				..Fx::default()
			},
		]
	}

	fn esc(s: &str) -> String {
		s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
	}

	fn wrap(s: String, f: &oxedyne_fe2o3_text::csl::node::Formatting) -> String {
		let mut s = s;
		if f.style != FontStyle::Normal { s = format!("<em>{}</em>", s); }
		if f.weight == FontWeight::Bold { s = format!("<strong>{}</strong>", s); }
		if f.valign == VAlign::Sup { s = format!("<sup>{}</sup>", s); }
		s
	}

	fn html(nodes: &[Node], out: &mut String) {
		for n in nodes {
			match n {
				Node::Text(t, f)		=> out.push_str(&wrap(esc(t), f)),
				Node::Math(m)			=> { out.push('$'); out.push_str(m); out.push('$'); }
				Node::Link { text, fmt, url }	=> out.push_str(&wrap(format!("<a href=\"{}\">{}</a>", url, esc(text)), fmt)),
				Node::Cite { .. }		=> out.push_str("p. 5"),	// the only supplement used
				Node::Elem(e) => {
					let mut inner = String::new();
					html(&e.children, &mut inner);
					match (e.display, &e.meta) {
						(Some(Display::LeftMargin), _)	=> { out.push_str("<span class=\"prefix\">"); out.push_str(&inner); out.push_str("</span> "); }
						(_, Some(Meta::Entry(_)))	=> { out.push_str("<a>"); out.push_str(&inner); out.push_str("</a>"); }
						_				=> out.push_str(&inner),
					}
				}
			}
		}
	}

	/// The entry as Typst writes it: second-field-align's first field as the prefix column, and
	/// the rest after one space.
	fn entry_html(prefix: Option<Node>, body: Vec<Node>) -> String {
		let mut out = String::new();
		if let Some(p) = prefix {
			let mut inner = String::new();
			html(&[p], &mut inner);
			out.push_str(&format!("<span class=\"prefix\">{}</span> ", inner));
		}
		let mut rest = String::new();
		html(&body, &mut rest);
		out.push_str(rest.trim_start());
		out.trim().to_string()
	}

	/// Each `<li>` of an expected file, keyed by the entry it shows.
	fn expected(style: &str) -> Outcome<Vec<String>> {
		let path = format!("{}/tests/data/csl/expect/{}.html", env!("CARGO_MANIFEST_DIR"), style);
		let src = res!(std::fs::read_to_string(&path));
		Ok(src.split("<li>").skip(1).map(|li| match li.find("</li>") {
			Some(end)	=> li[..end].to_string(),
			None		=> li.to_string(),
		}).collect())
	}

	/// The whole bibliography, as Typst's `full: true` cites every entry: the count of entries
	/// that match Typst's in the same place.
	fn check(style_name: &str) -> Outcome<usize> {
		let style = res!(res!(archive::by_name(style_name).ok_or_else(|| err!("No style {}.", style_name; Missing))).style());
		let files = res!(archive::locales_for(&style, Some("en")));
		let chain = files.chain(&style, Some("en"));
		let want = res!(expected(style_name));
		let items = fixtures();
		let cites: Vec<Citation> = (0..items.len()).map(|i| Citation {
			refs: vec![CiteRef { entry: i, hidden: true, ..CiteRef::default() }],
			note: None,
		}).collect();
		let state = CiteState::new(&style, &chain, Some("en"), &items, &cites);
		let bib = render_bibliography(&style, &chain, Some("en"), &items, &state);
		let mut matched = 0;
		let mut report = String::new();
		for (i, (prefix, body)) in bib.into_iter().enumerate() {
			let got = entry_html(prefix, body);
			if want.get(i) == Some(&got) {
				matched += 1;
			} else {
				report.push_str(&format!("\n{} #{} {}: got  {}\n     want {}\n", style_name, i, items[state.order()[i]].key, got, want.get(i).map_or("", |w| w.as_str())));
			}
		}
		if !report.is_empty() {
			println!("{}", report);
		}
		Ok(matched)
	}

	/// The citations of `cites.typ`, in order, over the fixtures' keys.
	fn cites_typ(items: &[Fx]) -> Vec<Citation> {
		let at = |k: &str| items.iter().position(|f| f.key == k).unwrap_or(usize::MAX);
		let r = |k: &str| CiteRef { entry: at(k), ..CiteRef::default() };
		let loc = |k: &str| CiteRef { locator: Some(("custom".to_string(), NumVal::Transparent(0))), ..r(k) };
		let form = |k: &str, form: CiteForm| CiteRef { form, ..r(k) };
		let c = |refs: Vec<CiteRef>| Citation { refs, note: None };
		vec![
			c(vec![r("a1")]),
			c(vec![r("n1")]),
			c(vec![r("n1"), r("n2"), r("n3")]),
			c(vec![r("n5"), r("n4")]),
			c(vec![loc("n1")]),
			c(vec![loc("n1")]),
			c(vec![r("n1")]),
			c(vec![r("n2"), r("n3"), r("a1")]),
			c(vec![form("n1", CiteForm::Author)]),
			c(vec![form("n1", CiteForm::Year)]),
			c(vec![form("n1", CiteForm::Prose)]),
			c(vec![form("n1", CiteForm::Full)]),
		]
	}

	/// Typst's `<a ... role="doc-biblioref">` written as a bare `<a>`.
	fn bare_links(s: &str) -> String {
		let mut out = String::new();
		let mut rest = s;
		while let Some(i) = rest.find("<a ") {
			out.push_str(&rest[..i]);
			let tail = &rest[i..];
			match (tail.find('>'), tail.find("role=\"doc-biblioref\">")) {
				(Some(e), Some(r)) if r < e => { out.push_str("<a>"); rest = &tail[e + 1..]; }
				(Some(e), _) => { out.push_str(&tail[..e + 1]); rest = &tail[e + 1..]; }
				(None, _) => { out.push_str(tail); rest = ""; }
			}
		}
		out.push_str(rest);
		// A superscript delimiter carries the role too.
		out.replace("<sup role=\"doc-biblioref\">", "<sup>")
	}

	/// Each citation of `cites.typ` against Typst's `cites-<style>.html`: the indices that match.
	fn check_cites(style_name: &str, items: Vec<Fx>, cites: Vec<Citation>, file: &str) -> Outcome<Vec<usize>> {
		let style = res!(res!(archive::by_name(style_name).ok_or_else(|| err!("No style {}.", style_name; Missing))).style());
		let files = res!(archive::locales_for(&style, Some("en")));
		let chain = files.chain(&style, Some("en"));
		let path = format!("{}/tests/data/csl/expect/{}", env!("CARGO_MANIFEST_DIR"), file);
		let src = res!(std::fs::read_to_string(&path));
		let want: Vec<String> = src.lines().map(|l| bare_links(l.trim_start_matches("<li>").trim_end_matches("</li>"))).collect();
		let state = CiteState::new(&style, &chain, Some("en"), &items, &cites);
		let mut ok = Vec::new();
		let mut report = String::new();
		for i in 0..cites.len() {
			let mut got = String::new();
			html(state.citation(i), &mut got);
			if want.get(i) == Some(&got) {
				ok.push(i);
			} else {
				report.push_str(&format!("\n{} cite {}: got  {}\n          want {}\n", style_name, i, got, want.get(i).map_or("", |w| w.as_str())));
			}
		}
		if !report.is_empty() {
			println!("{}", report);
		}
		Ok(ok)
	}

	#[test]
	fn test_ieee_citations_as_typst() -> Outcome<()> {
		let items = fixtures();
		let cites = cites_typ(&items);
		let ok = res!(check_cites("ieee", items, cites, "cites-ieee.html"));
		assert_eq!(ok, (0..12).collect::<Vec<_>>());
		Ok(())
	}

	#[test]
	fn test_apa_citations_as_typst() -> Outcome<()> {
		let items = fixtures();
		let cites = cites_typ(&items);
		let ok = res!(check_cites("apa", items, cites, "cites-apa.html"));
		assert_eq!(ok, (0..12).collect::<Vec<_>>());
		Ok(())
	}

	/// `group.bib`: one author's three works, two in one year; a second Smith; two Browns et al.
	fn group_fixtures() -> Vec<Fx> {
		let art = |key, title, author: Vec<Person>, year| Fx {
			key, ty: "article-journal", title: Chunked::normal(title),
			container: Some("Journal of Pairs"), author, issued: Some(DateVal::year(year)),
			..Fx::default()
		};
		let book = |key, title, author: Vec<Person>, year| Fx {
			key, ty: "book", title: Chunked::normal(title), publisher: Some("Acme Press"),
			author, issued: Some(DateVal::year(year)),
			..Fx::default()
		};
		vec![
			art("d1", "First Paper", vec![p("Smith", "John")], 2020),
			art("d2", "Second Paper", vec![p("Smith", "John")], 2020),
			book("d3", "Earlier Book", vec![p("Smith", "John")], 2018),
			art("d4", "Another Smith", vec![p("Smith", "Mary")], 2019),
			art("d5", "Three Colours", vec![p("Brown", "Alice"), p("Green", "Bob"), p("White", "Carl")], 2021),
			art("d6", "Three More Colours", vec![p("Brown", "Alice"), p("Black", "Dan"), p("Blue", "Eve")], 2021),
			book("d7", "A Later Book", vec![p("Smith", "Mary")], 2018),
		]
	}

	/// The citations of `group.typ`.
	fn group_cites(items: &[Fx]) -> Vec<Citation> {
		let at = |k: &str| items.iter().position(|f| f.key == k).unwrap_or(usize::MAX);
		let c = |keys: &[&str]| Citation {
			refs: keys.iter().map(|k| CiteRef { entry: at(k), ..CiteRef::default() }).collect(),
			note: None,
		};
		vec![
			c(&["d1", "d2", "d3"]),
			c(&["d1", "d4", "d3", "d2"]),
			c(&["d4"]),
			c(&["d5"]),
			c(&["d6"]),
			c(&["d5", "d6", "d1"]),
			c(&["d3", "d1", "d2", "d5", "d6"]),
			c(&["d7"]),
			c(&["d3", "d7"]),
		]
	}

	fn check_group(style: &str) -> Outcome<Vec<usize>> {
		let items = group_fixtures();
		let cites = group_cites(&items);
		check_cites(style, items, cites, &format!("group-{}.html", style))
	}

	/// Citation-number collapsing: `1–3`, and the after-collapse delimiter.
	#[test]
	fn test_ama_collapse_as_typst() -> Outcome<()> {
		assert_eq!(res!(check_group("american-medical-association")), (0..9).collect::<Vec<_>>());
		Ok(())
	}

	/// Grouping by author with year collapsing (apa), and year-suffix collapsing (ams).
	#[test]
	fn test_group_collapse_as_typst() -> Outcome<()> {
		assert_eq!(res!(check_group("apa")), (0..9).collect::<Vec<_>>());
		assert_eq!(res!(check_group("american-meteorological-society")), (0..9).collect::<Vec<_>>());
		Ok(())
	}

	/// Styles that may add given names (chicago-author-date by primary-name, mla by-cite with no
	/// year-suffix). Typst's hayagriva separates these Smiths by year-suffix or by `choose
	/// disambiguate` instead, and so must this port.
	#[test]
	fn test_givenname_styles_as_typst() -> Outcome<()> {
		assert_eq!(res!(check_group("chicago-author-date")), (0..9).collect::<Vec<_>>());
		for style in ["modern-language-association", "chicago-author-date"] {
			let items = group_fixtures();
			let cites: Vec<Citation> = [&["d1"][..], &["d4"], &["d1", "d4"], &["d5", "d6"]].iter()
				.map(|keys| Citation {
					refs: keys.iter().map(|k| CiteRef {
						entry: items.iter().position(|f| f.key == *k).unwrap_or(usize::MAX),
						..CiteRef::default()
					}).collect(),
					note: None,
				})
				.collect();
			let ok = res!(check_cites(style, items, cites, &format!("given-{}.html", style)));
			assert_eq!(ok, (0..4).collect::<Vec<_>>(), "{}", style);
		}
		Ok(())
	}

	#[test]
	fn test_ieee_as_typst() -> Outcome<()> {
		assert_eq!(res!(check("ieee")), 10);
		Ok(())
	}

	#[test]
	fn test_apa_as_typst() -> Outcome<()> {
		assert_eq!(res!(check("apa")), 10);
		Ok(())
	}

	#[test]
	fn test_american_medical_association_as_typst() -> Outcome<()> {
		assert_eq!(res!(check("american-medical-association")), 10);
		Ok(())
	}

	#[test]
	fn test_chicago_author_date_as_typst() -> Outcome<()> {
		assert_eq!(res!(check("chicago-author-date")), 10);
		Ok(())
	}
}

// B2b types: page ranges and persons, as hayagriva and citationberg give them.
mod b2b_types {
	use oxedyne_fe2o3_text::csl::{
		page::{
			format_range,
			PageVal,
		},
		person::Person,
		style::PageRange,
	};

	fn fr(f: PageRange, a: &str, b: &str) -> String {
		let mut s = String::new();
		format_range(&mut s, f, a, b, None);
		s
	}

	#[test]
	fn test_page_range_formats() {
		assert_eq!(fr(PageRange::Chicago16, "321", "328"), "321–28");
		assert_eq!(fr(PageRange::Chicago16, "1001", "1012"), "1001–12");
		assert_eq!(fr(PageRange::Chicago16, "123", "129"), "123–29");
		assert_eq!(fr(PageRange::Chicago16, "100", "104"), "100–104");
		assert_eq!(fr(PageRange::Chicago16, "101", "108"), "101–8");
		assert_eq!(fr(PageRange::Chicago15, "1496", "1504"), "1496–1504");
		assert_eq!(fr(PageRange::Expanded, "42", "5"), "42–45");
		assert_eq!(fr(PageRange::Minimal, "321", "328"), "321–8");
		assert_eq!(fr(PageRange::MinimalTwo, "321", "328"), "321–28");
		assert_eq!(fr(PageRange::Expanded, "S1", "S12"), "S1–S12");
		let mut s = String::new();
		if let PageVal::Typed(p) = PageVal::from_str("1-5, 7 & 9") {
			assert!(p.is_plural());
			p.write(&mut s, None, None);
		}
		assert_eq!(s, "1–5, 7 & 9");
		assert!(!PageVal::from_str("12").is_plural());
		assert!(matches!(PageVal::from_str("i-x 3"), PageVal::Str(_)));
	}

	#[test]
	fn test_person_parts() {
		let mut s = String::new();
		let _ = Person::new("Müller", Some("Hans-Peter")).initials(&mut s, Some(". "), true);
		assert_eq!(s, "H.-P.");
		s.clear();
		let _ = Person::new("Müller", Some("Hans-Peter")).initials(&mut s, Some(""), false);
		assert_eq!(s, "HP");
		let p = Person::new("van der Berg", Some("Anna"));
		assert_eq!(p.particle_in_family(), Some("van der"));
		assert_eq!(p.family_without_particle(), "Berg");
		assert!(Person::literal("Acme Corporation").is_institutional());
		assert_eq!(Person::literal("The Acme Group").family_without_article(), "Acme Group");
	}
}
