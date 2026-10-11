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
		item::{
			ChunkKind,
			Chunked,
			Item,
		},
		node::{
			Meta,
			Node,
		},
		render::{
			self,
			Cite,
		},
		style::{
			Display,
			FontStyle,
			FontWeight,
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
	}

	impl Item for Fx {
		fn text(&self, var: &str, _form: VarForm) -> Option<Chunked> {
			match var {
				"title"			=> Some(self.title.clone()),
				"container-title"	=> self.container.map(Chunked::normal),
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
		]
	}

	fn esc(s: &str) -> String {
		s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
	}

	fn wrap(s: String, f: &oxedyne_fe2o3_text::csl::node::Formatting) -> String {
		let mut s = s;
		if f.style != FontStyle::Normal { s = format!("<em>{}</em>", s); }
		if f.weight == FontWeight::Bold { s = format!("<strong>{}</strong>", s); }
		s
	}

	fn html(nodes: &[Node], out: &mut String) {
		for n in nodes {
			match n {
				Node::Text(t, f)		=> out.push_str(&wrap(esc(t), f)),
				Node::Math(m)			=> { out.push('$'); out.push_str(m); out.push('$'); }
				Node::Link { text, fmt, url }	=> out.push_str(&wrap(format!("<a href=\"{}\">{}</a>", url, esc(text)), fmt)),
				Node::Cite { .. }		=> {}
				Node::Elem(e) => {
					let mut inner = String::new();
					html(&e.children, &mut inner);
					match e.display {
						Some(Display::LeftMargin)	=> { out.push_str("<span class=\"prefix\">"); out.push_str(&inner); out.push_str("</span> "); }
						_				=> out.push_str(&inner),
					}
				}
			}
		}
	}

	/// The entry as Typst writes it. second-field-align (B3) is applied here as Typst shows it:
	/// the first field becomes the prefix column and the rest follows after one space.
	fn entry_html(mut nodes: Vec<Node>, align: bool) -> String {
		let mut out = String::new();
		if align && matches!(nodes.first(), Some(Node::Elem(e)) if e.meta == Some(Meta::CitationNumber)) {
			let first = nodes.remove(0);
			let mut inner = String::new();
			html(&[first], &mut inner);
			out.push_str(&format!("<span class=\"prefix\">{}</span> ", inner));
			let mut rest = String::new();
			html(&nodes, &mut rest);
			out.push_str(rest.trim_start());
		} else {
			html(&nodes, &mut out);
		}
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

	fn check(style_name: &str) -> Outcome<usize> {
		let style = res!(res!(archive::by_name(style_name).ok_or_else(|| err!("No style {}.", style_name; Missing))).style());
		let files = res!(archive::locales_for(&style, Some("en")));
		let chain = files.chain(&style, Some("en"));
		let align = style.bibliography.as_ref().map_or(false, |b| b.second_field_align.is_some());
		let want = res!(expected(style_name));
		let mut matched = 0;
		let mut report = String::new();
		for (i, fx) in fixtures().iter().enumerate() {
			let cite = Cite { number: Some(i + 1), ..Cite::default() };
			let nodes = res!(render::bibliography_entry(&style, &chain, fx, &cite, Some("en"))
				.ok_or_else(|| err!("{} has no bibliography.", style_name; Missing)));
			let got = entry_html(nodes, align);
			if want.contains(&got) {
				matched += 1;
			} else {
				report.push_str(&format!("\n{} {}: got  {}\n", style_name, fx.key, got));
			}
		}
		if !report.is_empty() {
			println!("{}\nwant:\n{}", report, want.join("\n"));
		}
		Ok(matched)
	}

	#[test]
	fn test_ieee_as_typst() -> Outcome<()> {
		assert_eq!(res!(check("ieee")), 4);
		Ok(())
	}

	#[test]
	fn test_apa_as_typst() -> Outcome<()> {
		assert_eq!(res!(check("apa")), 4);
		Ok(())
	}

	#[test]
	fn test_chicago_author_date_as_typst() -> Outcome<()> {
		assert_eq!(res!(check("chicago-author-date")), 4);
		Ok(())
	}
}
