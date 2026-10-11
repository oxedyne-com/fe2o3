//! The page constructs of an encyclopaedia article -- figures, definition lists, captioned tables with
//! row headers, classed spans and divisions -- read from HTML into the document tree and written back
//! out as HTML of the same structure.

use oxedyne_fe2o3_text::doc::{
	html,
	markdown,
	policy,
	Align,
	Attrs,
	Block,
	Cell,
	Doc,
	Inline,
	Term,
};
use oxedyne_fe2o3_text::doc::html::read;

use oxedyne_fe2o3_core::prelude::*;

fn t(s: &str) -> Inline {
	Inline::Text(s.to_string())
}

fn class(c: &str) -> Attrs {
	Attrs { id: None, classes: vec![c.to_string()], pairs: Vec::new() }
}

fn keep() -> read::Opts {
	read::Opts { keep_classed: true }
}

fn parse_kept(src: &str) -> Outcome<Doc> {
	Ok(Doc { blocks: res!(read::parse_with(src, &keep())) })
}

/// HTML as a comparison sees it: each tag's attributes in sorted order, every run of whitespace one
/// space, and none between two tags.
fn norm(src: &str) -> String {
	let mut out = String::new();
	let mut rest = src;
	while let Some(at) = rest.find('<') {
		out.push_str(&rest[..at]);
		let end = match rest[at..].find('>') {
			Some(k)	=> at + k,
			None	=> break,
		};
		out.push_str(&tag(&rest[at + 1..end]));
		rest = &rest[end + 1..];
	}
	out.push_str(rest);
	// Collapse whitespace, then drop what stands between two tags.
	let mut flat = String::new();
	let mut ws = false;
	for c in out.chars() {
		if c.is_whitespace() {
			ws = true;
		} else {
			if ws && !flat.is_empty() {
				flat.push(' ');
			}
			ws = false;
			flat.push(c);
		}
	}
	flat.replace("> <", "><")
}

/// One tag with its attributes sorted. Quoted values in the fixtures hold no `>` and no spaces that
/// matter beyond the one in a style, which the split below keeps inside its quotes.
fn tag(inner: &str) -> String {
	let mut parts: Vec<String> = Vec::new();
	let mut cur = String::new();
	let mut q = false;
	for c in inner.chars() {
		match c {
			'"'			=> { q = !q; cur.push(c); }
			c if c.is_whitespace() && !q => {
				if !cur.is_empty() {
					parts.push(std::mem::take(&mut cur));
				}
			}
			_			=> cur.push(c),
		}
	}
	if !cur.is_empty() {
		parts.push(cur);
	}
	if parts.is_empty() {
		return "<>".to_string();
	}
	let name = parts.remove(0);
	parts.sort();
	let mut s = format!("<{}", name);
	for p in parts {
		s.push(' ');
		s.push_str(&p);
	}
	s.push('>');
	s
}

/// The input as the writer is expected to give it back: a `num` cell's column alignment comes out as
/// the logical style the writer has always used for `Align::End`, and an `i` as the `em` the tree
/// reads it to.
fn expected(src: &str) -> String {
	norm(&src
		.replace("class=\"num\"", "style=\"text-align: end\"")
		.replace("<i>", "<em>")
		.replace("</i>", "</em>"))
}

#[test]
fn test_a_figure_holds_its_image_and_caption_00() -> Outcome<()> {
	let src = "<figure class=\"fig\">\n<img src=\"img/paw.jpg\" alt=\"Dog offering a paw\">\n\
		<figcaption><span class=\"fig-caption\">Hachikō giving a paw.</span> \
		<span class=\"credit\">Unknown author, public domain</span></figcaption>\n</figure>";
	let doc = res!(parse_kept(src));
	assert_eq!(doc.blocks, vec![Block::Figure {
		attrs:		class("fig"),
		content:	vec![Block::Para(vec![Inline::Image {
			src:	"img/paw.jpg".to_string(),
			alt:	"Dog offering a paw".to_string(),
			attrs:	Attrs::default(),
		}])],
		caption:	vec![
			Inline::Span { attrs: class("fig-caption"), content: vec![t("Hachikō giving a paw.")] },
			t(" "),
			Inline::Span { attrs: class("credit"), content: vec![t("Unknown author, public domain")] },
		],
	}]);
	assert_eq!(norm(&html::render(&doc)), norm(src));
	// Without the option the figure stays a figure, and only the names go.
	let plain = res!(html::parse(src));
	match &plain.blocks[0] {
		Block::Figure { attrs, caption, .. }	=> {
			assert!(attrs.is_empty());
			assert_eq!(caption, &vec![t("Hachikō giving a paw. Unknown author, public domain")]);
		}
		other					=> panic!("expected a figure, got {:?}", other),
	}
	Ok(())
}

#[test]
fn test_a_definition_list_reads_wrapped_and_bare_groups_01() -> Outcome<()> {
	let wrapped = "<dl class=\"terms\">\n<div><dt>Jaw joint</dt><dd>The dentary meets the squamosal.</dd></div>\n\
		<div><dt>Middle ear</dt><dd>Three bones.</dd></div>\n</dl>";
	let bare = "<dl class=\"terms\"><dt>Jaw joint</dt><dd>The dentary meets the squamosal.</dd>\
		<dt>Middle ear</dt><dd>Three bones.</dd></dl>";
	let want = vec![Block::Terms {
		attrs:	class("terms"),
		items:	vec![
			Term {
				term:	vec![t("Jaw joint")],
				defs:	vec![vec![Block::Para(vec![t("The dentary meets the squamosal.")])]],
			},
			Term {
				term:	vec![t("Middle ear")],
				defs:	vec![vec![Block::Para(vec![t("Three bones.")])]],
			},
		],
	}];
	assert_eq!(res!(parse_kept(wrapped)).blocks, want);
	assert_eq!(res!(parse_kept(bare)).blocks, want);
	// The writer gives the wrapped form, which is the one a stylesheet can lay out by group.
	assert_eq!(norm(&html::render(&Doc { blocks: want })), norm(wrapped));
	Ok(())
}

#[test]
fn test_a_term_may_carry_several_definitions_02() -> Outcome<()> {
	let doc = res!(html::parse("<dl><dt>Bark</dt><dd>A tree's skin.</dd><dd>A dog's call.</dd></dl>"));
	match &doc.blocks[0] {
		Block::Terms { items, .. }	=> {
			assert_eq!(items.len(), 1);
			assert_eq!(items[0].defs.len(), 2);
		}
		other				=> panic!("expected terms, got {:?}", other),
	}
	let out = html::render(&doc);
	assert!(out.contains("<dd>A tree's skin.</dd><dd>A dog's call.</dd>"), "got: {}", out);
	Ok(())
}

#[test]
fn test_a_table_keeps_its_caption_and_row_headers_03() -> Outcome<()> {
	let src = "<table>\n<caption>Facts</caption>\n<tbody>\n\
		<tr><th scope=\"row\">Breed</th><td>Akita</td></tr>\n\
		<tr><th scope=\"row\">Sex</th><td>Male</td></tr>\n</tbody>\n</table>";
	let doc = res!(html::parse(src));
	match &doc.blocks[0] {
		Block::Table { head, rows, caption, .. }	=> {
			// A column of `th scope="row"` names rows, not columns, so the table has no head.
			assert!(head.is_none(), "row headers were read as a header row");
			assert_eq!(caption, &vec![t("Facts")]);
			assert_eq!(rows[0].0[0], Cell { content: vec![t("Breed")], header: true });
			assert_eq!(rows[0].0[1], Cell::new(vec![t("Akita")]));
		}
		other						=> panic!("expected a table, got {:?}", other),
	}
	assert_eq!(norm(&html::render(&doc)), norm(src));
	Ok(())
}

#[test]
fn test_a_num_column_aligns_to_the_end_04() -> Outcome<()> {
	let all = res!(html::parse("<table><tbody><tr><td>a</td><td class=\"num\">1</td></tr>\
		<tr><td>b</td><td class=\"num\">2</td></tr></tbody></table>"));
	match &all.blocks[0] {
		Block::Table { cols, .. }	=> assert_eq!(cols, &vec![Align::None, Align::End]),
		other				=> panic!("expected a table, got {:?}", other),
	}
	// A column only some of whose cells say `num` has no one alignment to give, and is given none.
	let some = res!(html::parse("<table><tbody><tr><td>a</td><td class=\"num\">1</td></tr>\
		<tr><td>b</td><td>two</td></tr></tbody></table>"));
	match &some.blocks[0] {
		Block::Table { cols, .. }	=> assert_eq!(cols, &vec![Align::None, Align::None]),
		other				=> panic!("expected a table, got {:?}", other),
	}
	Ok(())
}

#[test]
fn test_sup_and_sub_are_read_and_written_05() -> Outcome<()> {
	let src = "<p>H<sub>2</sub>O and E = mc<sup>2</sup></p>";
	let doc = res!(html::parse(src));
	assert_eq!(doc.blocks, vec![Block::Para(vec![
		t("H"),
		Inline::Sub(vec![t("2")]),
		t("O and E = mc"),
		Inline::Sup(vec![t("2")]),
	])]);
	assert_eq!(norm(&html::render(&doc)), norm(src));
	Ok(())
}

#[test]
fn test_keep_classed_keeps_only_named_spans_and_divs_06() -> Outcome<()> {
	let src = "<div class=\"table-wrap\"><div><p>Inner.</p></div><p>After.</p></div>\
		<p>A <span class=\"id\">1375772297</span> and <span>bare</span> span.</p>";
	// By default both are erased, as they always were.
	let plain = res!(html::parse(src));
	assert_eq!(plain.blocks, vec![
		Block::Para(vec![t("Inner.")]),
		Block::Para(vec![t("After.")]),
		Block::Para(vec![t("A 1375772297 and bare span.")]),
	]);
	// Kept, the named ones stay, and an unnamed `div` inside a named one closes only itself.
	let kept = res!(parse_kept(src));
	assert_eq!(kept.blocks, vec![
		Block::Div {
			attrs:		class("table-wrap"),
			content:	vec![Block::Para(vec![t("Inner.")]), Block::Para(vec![t("After.")])],
		},
		Block::Para(vec![
			t("A "),
			Inline::Span { attrs: class("id"), content: vec![t("1375772297")] },
			t(" and bare span."),
		]),
	]);
	// An id names an element as well as a class does, and other attributes travel as pairs.
	let id = res!(parse_kept("<p><span id=\"x\" lang=\"ja\">名</span></p>"));
	assert_eq!(id.blocks, vec![Block::Para(vec![Inline::Span {
		attrs:		Attrs {
			id:		Some("x".to_string()),
			classes:	Vec::new(),
			pairs:		vec![("lang".to_string(), "ja".to_string())],
		},
		content:	vec![t("名")],
	}])]);
	Ok(())
}

#[test]
fn test_the_policy_walks_every_new_node_07() -> Outcome<()> {
	let src = "<figure class=\"fig\"><img src=\"https://tracker.example/x.png\" alt=\"A picture\">\
		<figcaption><a href=\"javascript:alert(1)\">Click</a></figcaption></figure>\
		<dl class=\"terms\"><dt><a href=\"javascript:x\">Term</a></dt><dd>Def <sup>1</sup></dd></dl>\
		<table><caption><a href=\"javascript:y\">Cap</a></caption><tr><th scope=\"row\">A</th><td>1</td></tr></table>";
	let doc = policy::apply(&res!(parse_kept(src)), &policy::Policy::default());
	let out = html::render(&doc);
	assert!(!out.contains("javascript"), "a refused destination survived: {}", out);
	assert!(!out.contains("tracker.example"), "a remote image survived: {}", out);
	assert!(!out.contains("class="), "a stranger's names survived: {}", out);
	for words in ["A picture", "Click", "Term", "Def <sup>1</sup>", "Cap", "<th scope=\"row\">A</th>"] {
		assert!(out.contains(words), "lost {:?}: {}", words, out);
	}
	// A trusting policy keeps the names.
	let trusted = html::render(&policy::apply(&res!(parse_kept(src)), &policy::Policy::trusting()));
	assert!(trusted.contains("<figure class=\"fig\">"), "got: {}", trusted);
	Ok(())
}

#[test]
fn test_the_new_nodes_are_counted_and_written_as_markdown_08() -> Outcome<()> {
	let doc = res!(parse_kept("<figure><img src=\"a.png\" alt=\"x\"><figcaption>Two words</figcaption></figure>\
		<dl><dt>Term</dt><dd>One def.</dd></dl>\
		<table><caption>A caption</caption><tr><td>cell</td></tr></table>"));
	// The alt text, the caption, the term and definition, the table's caption and cell.
	assert_eq!(doc.word_count(), 1 + 2 + 1 + 2 + 2 + 1);
	let md = markdown::write::render(&doc);
	for words in ["![x](a.png)", "Two words", "**Term**", "One def.", "A caption", "| cell |"] {
		assert!(md.contains(words), "lost {:?}: {}", words, md);
	}
	Ok(())
}

/// The body of an article, modelled on the Hachikō and Mammal pages: sections' content as the editor
/// stores it, each construct as those pages write it.
///
/// One thing the pages write is not carried, and the fixture leaves it out: the `section` around each
/// part, which the application splits a page by before it reads one. An `img`'s `class`, `width` and
/// `height` are carried, as the image's [`Attrs`].
const ARTICLE: &str = r#"
<h2>Life story</h2>
<p>Hachikō, a creamy-white Akita, was born in mid-November 1923, at a farm in Ōdate, Akita Prefecture, Japan. His father was Ōshinai-yama-gō (<i>'Ōshinai mountain'</i>).</p>
<figure class="fig">
	<img src="img/paw.jpg" width="896" height="654" alt="Dog offering a paw">
	<figcaption><span class="fig-caption">Hachikō giving a paw.</span> <span class="credit">Unknown author, public domain</span></figcaption>
</figure>
<figure class="fig">
	<img class="diagram" src="img/skull.png" width="568" height="260" alt="Diagram of a synapsid skull with one temporal opening">
	<figcaption><span class="fig-caption">The original synapsid skull structure contains one temporal opening.</span> <span class="credit">Preto(m), CC BY-SA 3.0</span></figcaption>
</figure>
<p>Text from <a href="https://en.wikipedia.org/w/index.php?title=Hachik%C5%8D&amp;oldid=1375772297">Hachikō</a>, revision <span class="id">1375772297</span>, CO<sub>2</sub> and m<sup>2</sup>.</p>
<h2>Facts</h2>
<div class="table-wrap">
	<table>
		<tbody>
			<tr><th scope="row">Breed</th><td>Akita</td></tr>
			<tr><th scope="row">Cause of death</th><td>Terminal carcinosarcoma and <i>Dirofilaria immitis</i></td></tr>
		</tbody>
	</table>
</div>
<div class="table-wrap">
	<table>
		<caption>Counts of mammal species</caption>
		<thead>
			<tr><th scope="col">Source</th><th scope="col" class="num">Year</th><th scope="col" class="num">Species</th></tr>
		</thead>
		<tbody>
			<tr><td><i>Mammal Species of the World</i></td><td class="num">2006</td><td class="num">5,416</td></tr>
			<tr><td>IUCN Global Mammal Assessment</td><td class="num">2008</td><td class="num">5,488</td></tr>
		</tbody>
	</table>
</div>
<h2>Places</h2>
<dl class="terms">
	<div><dt>Ōdate, Akita Prefecture</dt><dd>Hachikō's hometown. A statue stands in front of Ōdate Station.</dd></div>
	<div><dt>Shibuya Station, Tokyo</dt><dd>The second statue, unveiled on August 15, 1948, still stands.</dd></div>
</dl>
<blockquote>
	<p>A quoted line, with <strong>strength</strong> and <em>emphasis</em>.</p>
</blockquote>
<h2>References</h2>
<ol>
	<li>Itoh, Mayumi (2017). <i>Hachikō: Solving Twenty Mysteries about the Most Famous Dog in Japan</i>. pp. 22–23.</li>
	<li>“How many species of mammals are there?”. <i>Journal of Mammalogy</i>. 99 (1): 1–14.</li>
</ol>
"#;

#[test]
fn test_an_article_body_round_trips_09() -> Outcome<()> {
	let doc = res!(parse_kept(ARTICLE));
	let out = html::render(&doc);
	assert_eq!(norm(&out), expected(ARTICLE), "\n--- rendered ---\n{}", out);
	// And the tree is a fixed point: reading what was written gives the tree back.
	assert_eq!(res!(parse_kept(&out)), doc);
	// The diagram's names are in the tree, not merely passed through as text.
	let diagram = Attrs {
		id:		None,
		classes:	vec!["diagram".to_string()],
		pairs:		vec![
			("width".to_string(), "568".to_string()),
			("height".to_string(), "260".to_string()),
		],
	};
	let found = doc.blocks.iter().any(|b| match b {
		Block::Figure { content, .. }	=> content.iter().any(|p| match p {
			Block::Para(run)	=> run.iter().any(|i| match i {
				Inline::Image { attrs, .. }	=> *attrs == diagram,
				_				=> false,
			}),
			_			=> false,
		}),
		_				=> false,
	});
	assert!(found, "the diagram's class and size were not carried: {:?}", doc.blocks);
	// The plain reader still reads an image as its source and alt text alone.
	let plain = res!(read::parse("<p><img class=\"diagram\" src=\"a.png\" width=\"5\" alt=\"x\"></p>"));
	assert_eq!(plain, vec![Block::Para(vec![Inline::Image {
		src:	"a.png".to_string(),
		alt:	"x".to_string(),
		attrs:	Attrs::default(),
	}])]);
	Ok(())
}

#[test]
fn test_the_writer_refuses_script_and_style_keys_10() -> Outcome<()> {
	let span = |k: &str| Doc { blocks: vec![Block::Para(vec![Inline::Span {
		attrs:		Attrs {
			id:		None,
			classes:	vec!["hl".to_string()],
			pairs:		vec![(k.to_string(), "alert(1)".to_string())],
		},
		content:	vec![t("x")],
	}])] };
	// Whatever a trusting policy lets through, the writer will not write a handler or a style.
	for k in ["onmouseover", "OnClick", "style", "x y", "data\"a", "class", "id", "1a"] {
		let out = html::render(&policy::apply(&span(k), &policy::Policy::trusting()));
		assert!(!out.contains("alert"), "{:?} was written: {}", k, out);
		assert!(out.contains("<span class=\"hl\">x</span>"), "got: {}", out);
	}
	let ok = html::render(&span("data-k"));
	assert!(ok.contains("data-k=\"alert(1)\""), "a plain key was refused: {}", ok);
	// Read from HTML, the handler never reaches the page either.
	let read = res!(parse_kept("<p><span class=\"hl\" onmouseover=\"alert(1)\" style=\"color:red\">x</span></p>"));
	let out = html::render(&read);
	assert!(!out.contains("alert") && !out.contains("color"), "got: {}", out);
	Ok(())
}

#[test]
fn test_the_policy_allows_attributes_by_kind_11() -> Outcome<()> {
	let doc = res!(parse_kept("<p><span id=\"a\" class=\"credit evil\" lang=\"ja\">x</span></p>"));
	let mut p = policy::Policy::default();
	p.classes = policy::AllowList::Only(vec!["credit".to_string()]);
	let out = html::render(&policy::apply(&doc, &p));
	assert!(out.contains("<span class=\"credit\">x</span>"), "got: {}", out);
	p.ids = true;
	p.pairs = policy::AllowList::Only(vec!["lang".to_string()]);
	let out = html::render(&policy::apply(&doc, &p));
	assert!(out.contains("id=\"a\"") && out.contains("lang=\"ja\"") && !out.contains("evil"), "got: {}", out);
	// Nothing permitted, and the span is unwrapped.
	let out = html::render(&policy::apply(&doc, &policy::Policy::default()));
	assert!(out.contains("<p>x</p>"), "got: {}", out);
	Ok(())
}
