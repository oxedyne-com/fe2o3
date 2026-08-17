use oxedyne_fe2o3_text::xml::{
	Node,
	Xml,
	write::{
		Out,
		decode,
		escape,
		escape_attr,
	},
};

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};

/// The main part of a `.docx` LibreOffice wrote: fourteen namespace declarations, an `mc:Ignorable`,
/// and the paragraph-run-text nesting every Word document is made of. Somebody else's output, which
/// is the only kind worth reading a foreign format against.
const DOCUMENT: &str = include_str!("data/document.xml");

/// A synthetic document holding one of everything that is not an element, so the tiling check has
/// something to lose.
const MIXED: &str = concat!(
	"<?xml version=\"1.0\"?>\n",
	"<!DOCTYPE root>\n",
	"<!-- a comment -->\n",
	"<root xmlns=\"urn:d\" xmlns:a=\"urn:a\" xml:lang=\"en\">\n",
	"  text before <a:one k=\"v\" a:k=\"w\"/> text after\n",
	"  <two>a &amp; b &#65; &nbsp;</two>\n",
	"  <three><![CDATA[ <not> markup ]]></three>\n",
	"  <four attr=\"a &gt; b\">held</four>\n",
	"</root>\n",
);

/// Every node's span, in document order, concatenated.
///
/// The property this exists to check is that the result is the source: nothing lost between two
/// nodes, and nothing counted twice. A span-preserving tree that did not tile would write back a
/// document with a hole in it, and no other check would see it.
fn tiled(xml: &Xml) -> String {
	let mut out = String::new();
	for node in &xml.nodes {
		walk(xml, node, &mut out);
	}
	out
}

/// Adds a node's own bytes to the tiling, descending through an element rather than taking its whole
/// span, so a gap inside one is caught rather than papered over by its parent.
fn walk(xml: &Xml, node: &Node, out: &mut String) {
	match node {
		Node::Elem(e)	=> {
			out.push_str(xml.raw(&e.open));
			for kid in &e.kids {
				walk(xml, kid, out);
			}
			if let Some(inner) = &e.inner {
				// Whatever is left of the element after its open tag and its content is its close
				// tag, and that is the last thing it is made of.
				out.push_str(&xml.source()[inner.end..e.span.end]);
			}
		}
		other		=> out.push_str(xml.raw(&other.span())),
	}
}

pub fn test_xml(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["The spans tile the source exactly 000", "all", "xml"], || {
		for (what, src) in [("the synthetic document", MIXED), ("a real word/document.xml", DOCUMENT)] {
			let xml = res!(Xml::parse(src));
			let got = tiled(&xml);
			assert_eq!(got.len(), src.len(), "{} did not tile: length differs", what);
			match got == src {
				true	=> {}
				false	=> {
					let at = got.char_indices().zip(src.chars()).position(|((_, a), b)| a != b);
					panic!("{} did not tile: first difference at {:?}", what, at);
				}
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["A document nobody edited renders as its source 001", "all", "xml"], || {
		for src in [MIXED, DOCUMENT] {
			let xml = res!(Xml::parse(src));
			assert!(xml.is_pristine());
			assert_eq!(xml.render(), src);
		}
		Ok(())
	}));

	res!(test_it(filter, &["A splice replaces its bytes and copies the rest 002", "all", "xml"], || {
		// The whole editing model in one test. The theme, the settings and the tab stops of a real
		// document are the elements this knows nothing about, and they come through because nothing
		// re-serialises them.
		let mut xml = res!(Xml::parse(DOCUMENT));
		let texts = xml.all("w:t");
		assert_eq!(texts.len(), 3, "the document holds three runs of text");
		let target = res!(texts.iter().find(|e| xml.text_of(e).starts_with("A paragraph"))
			.ok_or_else(|| err!("the paragraph went missing"; Missing)));
		let span = target.span.clone();
		let was = xml.raw(&span).to_string();
		res!(xml.splice(span, "<w:t>Replaced.</w:t>".to_string()));
		let out = xml.render();
		assert!(!out.contains("A paragraph of ordinary prose"), "the old text went");
		assert!(out.contains("<w:t>Replaced.</w:t>"), "the new text arrived");
		// And everything else is exactly what it was: the rendered document differs from the source by
		// that one substitution and by nothing else.
		let mut expect = DOCUMENT.to_string();
		let at = res!(expect.find(&was).ok_or_else(|| err!("the run was not found"; Missing)));
		expect.replace_range(at..at + was.len(), "<w:t>Replaced.</w:t>");
		assert_eq!(out, expect);
		// It still parses, which is what says the splice did not break the document.
		let again = res!(Xml::parse(&out));
		assert_eq!(again.all("w:p").len(), 5);
		Ok(())
	}));

	res!(test_it(filter, &["Names resolve against the declarations in scope 003", "all", "xml"], || {
		let xml = res!(Xml::parse(MIXED));
		let root = res!(xml.root());
		assert_eq!(root.name.qname, "root");
		assert_eq!(root.name.local(), "root");
		assert_eq!(root.name.prefix(), "");
		// An element with no prefix takes the default namespace.
		let urn_d = res!(xml.uri_index("urn:d").ok_or_else(|| err!("urn:d not declared"; Missing)));
		assert_eq!(root.name.ns, Some(urn_d));
		let one = res!(root.child("a:one").ok_or_else(|| err!("a:one is missing"; Missing)));
		let urn_a = res!(xml.uri_index("urn:a").ok_or_else(|| err!("urn:a not declared"; Missing)));
		assert_eq!(one.name.ns, Some(urn_a), "a prefixed element takes its prefix's namespace");
		assert_eq!(one.name.local(), "one");
		// An attribute with no prefix is in NO namespace, which is not the same rule as an element's.
		// A reader that treated them alike would put every unprefixed attribute in the default.
		let bare = res!(one.attrs.iter().find(|a| a.name.qname == "k")
			.ok_or_else(|| err!("attribute k is missing"; Missing)));
		assert_eq!(bare.name.ns, None, "an unprefixed attribute is in no namespace");
		let pref = res!(one.attrs.iter().find(|a| a.name.qname == "a:k")
			.ok_or_else(|| err!("attribute a:k is missing"; Missing)));
		assert_eq!(pref.name.ns, Some(urn_a));
		assert_eq!(one.attr("k"), Some("v"));
		assert_eq!(one.attr("a:k"), Some("w"));
		// `xml:` is bound everywhere and declared nowhere.
		let lang = res!(root.attrs.iter().find(|a| a.name.qname == "xml:lang")
			.ok_or_else(|| err!("xml:lang is missing"; Missing)));
		assert!(lang.name.ns.is_some(), "the xml prefix is bound without a declaration");
		// And a real document's prefixes resolve too.
		let doc = res!(Xml::parse(DOCUMENT));
		let w = res!(doc.uri_index("http://schemas.openxmlformats.org/wordprocessingml/2006/main")
			.ok_or_else(|| err!("the wordprocessingml namespace is missing"; Missing)));
		assert_eq!(res!(doc.root()).name.ns, Some(w));
		Ok(())
	}));

	res!(test_it(filter, &["The entities are the five XML has 004", "all", "xml"], || {
		assert_eq!(decode("a &amp; b &lt; c &gt; d &quot; e &apos;"), "a & b < c > d \" e '");
		assert_eq!(decode("&#65;&#x42;&#x263A;"), "AB\u{263A}");
		// `&nbsp;` is HTML's, not XML's. Inventing a character here would make a malformed document
		// look well formed, and the bug would surface somewhere with less context.
		assert_eq!(decode("a &nbsp; b"), "a &nbsp; b");
		assert_eq!(decode("50% off & more"), "50% off & more", "a bare ampersand is left alone");
		assert_eq!(escape("a & b < c > d"), "a &amp; b &lt; c &gt; d");
		assert_eq!(escape_attr("a \"b\" & c"), "a &quot;b&quot; &amp; c");
		// The text of an element resolves them, and a CDATA section does not.
		let xml = res!(Xml::parse(MIXED));
		let root = res!(xml.root());
		let two = res!(root.child("two").ok_or_else(|| err!("two is missing"; Missing)));
		assert_eq!(xml.text_of(two), "a & b A &nbsp;");
		let three = res!(root.child("three").ok_or_else(|| err!("three is missing"; Missing)));
		assert_eq!(xml.text_of(three), " <not> markup ", "a CDATA section says what it says");
		Ok(())
	}));

	res!(test_it(filter, &["An angle bracket inside a value ends nothing 005", "all", "xml"], || {
		// The bug a reader that searched for `>` would have. `>` is legal in an attribute value and
		// appears in real documents.
		let xml = res!(Xml::parse("<a b=\"x > y\" c='p &gt; q'>held</a>"));
		let root = res!(xml.root());
		assert_eq!(root.attr("b"), Some("x > y"));
		assert_eq!(root.attr("c"), Some("p > q"), "single quotes hold a value too");
		assert_eq!(xml.text_of(root), "held");
		Ok(())
	}));

	res!(test_it(filter, &["Malformed markup is refused by name 006", "all", "xml"], || {
		// XML is not HTML: what a browser recovers from, this refuses, because the documents it reads
		// are generator output and a generator emitting these has a bug worth hearing about.
		for (what, src) in [
			("a mismatched close tag",	"<a><b></a></b>"),
			("an element left open",	"<a><b>text</b>"),
			("a close tag with nothing open",	"<a></a></b>"),
			("an unquoted attribute value",	"<a b=c/>"),
			("a bare attribute",		"<a b/>"),
			("an unclosed value",		"<a b=\"c/>"),
			("an unclosed comment",	"<a><!-- forever</a>"),
			("an unbound prefix",		"<z:a/>"),
		] {
			assert!(Xml::parse(src).is_err(), "{} was accepted: {:?}", what, src);
		}
		// And a document with no root element is not a document.
		let xml = res!(Xml::parse("<?xml version=\"1.0\"?>\n<!-- nothing here -->"));
		assert!(xml.root().is_err());
		Ok(())
	}));

	res!(test_it(filter, &["Edits that overlap are refused rather than resolved 007", "all", "xml"], || {
		let mut xml = res!(Xml::parse("<a><b>one</b><c>two</c></a>"));
		res!(xml.splice(3..13, "<b>ONE</b>".to_string()));
		assert!(!xml.is_pristine());
		// Overlapping the first.
		assert!(xml.splice(6..16, "x".to_string()).is_err());
		// Past the end.
		assert!(xml.splice(100..200, "x".to_string()).is_err());
		// Backwards.
		assert!(xml.splice(10..3, "x".to_string()).is_err());
		// A second, disjoint edit is fine, and the two render in the right order however they arrive.
		res!(xml.splice(13..23, "<c>TWO</c>".to_string()));
		assert_eq!(xml.render(), "<a><b>ONE</b><c>TWO</c></a>");
		xml.revert();
		assert!(xml.is_pristine());
		assert_eq!(xml.render(), "<a><b>one</b><c>two</c></a>");
		Ok(())
	}));

	res!(test_it(filter, &["The emitter refuses a document it would break 008", "all", "xml"], || {
		let mut out = Out::declared();
		// The declaration goes on the root, as it does in a real part: a prefix nothing bound is a
		// document the reader below refuses, which is how a missing `xmlns` is found here rather than
		// by Word.
		out.open("w:p", &[("xmlns:w", "urn:w")]);
		out.leaf("w:t", &[("xml:space", "preserve")], "a < b & c");
		assert!(out.close("w:r").is_err(), "closing what is not open is refused");
		res!(out.close("w:p"));
		let s = res!(out.finish());
		assert!(s.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>"));
		assert!(s.contains("<w:t xml:space=\"preserve\">a &lt; b &amp; c</w:t>"));
		// What it emits reads back as what was put in.
		let xml = res!(Xml::parse(&s));
		let root = res!(xml.root());
		assert_eq!(xml.text_of(root), "a < b & c");
		// And an unfinished document is refused.
		let mut bad = Out::new();
		bad.open("a", &[]);
		assert!(bad.finish().is_err());
		Ok(())
	}));

	Ok(())
}
