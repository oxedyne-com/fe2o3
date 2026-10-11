//! `oxeweb/doc/1` is lossless: `export::to_doc(import::from_doc(d)) == d`, through the canonical
//! bytes, for the page fixtures of `fe2o3_text/tests/html_page.rs` and for a generated corpus that
//! reaches every block and inline variant, nested four deep.

use oxedyne_fe2o3_sbj::{
	canon,
	export,
	import,
	validate,
	SCHEMA_DOC_1,
};

use oxedyne_fe2o3_text::doc::{
	html::read,
	Align,
	Attrs,
	Block,
	Cell,
	Doc,
	Inline,
	Row,
	Term,
};

use oxedyne_fe2o3_core::prelude::*;

/// Imports, validates, encodes, decodes and exports a document, and asserts it came back unchanged.
fn round_trip(d: &Doc) -> Outcome<()> {
	let tree = import::from_doc(d, &import::Options::default());
	res!(validate::validate(&tree, SCHEMA_DOC_1));
	let bytes = res!(canon::encode(&tree));
	let back = res!(export::to_doc(&res!(canon::decode(&bytes))));
	if &back != d {
		return Err(err!(
			"The document did not survive the round trip.\nIn:  {:?}\nOut: {:?}", d, back;
		Test, Mismatch));
	}
	Ok(())
}

fn kept(src: &str) -> Outcome<Doc> {
	Ok(Doc { blocks: res!(read::parse_with(src, &read::Opts { keep_classed: true })) })
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ F1'S FIXTURES                                                              │
// └───────────────────────────────────────────────────────────────────────────┘

const FIGURE: &str = "<figure class=\"fig\">\n<img src=\"img/paw.jpg\" alt=\"Dog offering a paw\">\n\
	<figcaption><span class=\"fig-caption\">Hachikō giving a paw.</span> \
	<span class=\"credit\">Unknown author, public domain</span></figcaption>\n</figure>";

const TERMS_WRAPPED: &str = "<dl class=\"terms\">\n<div><dt>Jaw joint</dt><dd>The dentary meets the \
	squamosal.</dd></div>\n<div><dt>Middle ear</dt><dd>Three bones.</dd></div>\n</dl>";

const TERMS_BARE: &str = "<dl class=\"terms\"><dt>Jaw joint</dt><dd>The dentary meets the squamosal.\
	</dd><dt>Middle ear</dt><dd>Three bones.</dd></dl>";

const TABLE_FACTS: &str = "<table>\n<caption>Facts</caption>\n<tbody>\n\
	<tr><th scope=\"row\">Breed</th><td>Akita</td></tr>\n\
	<tr><th scope=\"row\">Sex</th><td>Male</td></tr>\n</tbody>\n</table>";

const CLASSED: &str = "<div class=\"table-wrap\"><div><p>Inner.</p></div><p>After.</p></div>\
	<p>A <span class=\"id\">1375772297</span> and <span>bare</span> span.</p>";

const ARTICLE: &str = r#"
<h2>Life story</h2>
<p>Hachikō, a creamy-white Akita, was born in mid-November 1923, at a farm in Ōdate, Akita Prefecture, Japan. His father was Ōshinai-yama-gō (<i>'Ōshinai mountain'</i>).</p>
<figure class="fig">
	<img class="diagram" src="img/paw.jpg" width="896" height="654" alt="Dog offering a paw">
	<figcaption><span class="fig-caption">Hachikō giving a paw.</span> <span class="credit">Unknown author, public domain</span></figcaption>
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
fn test_the_page_fixtures_round_trip_00() -> Outcome<()> {
	for src in [FIGURE, TERMS_WRAPPED, TERMS_BARE, TABLE_FACTS, CLASSED, ARTICLE] {
		let d = res!(kept(src));
		assert!(!d.blocks.is_empty(), "The fixture read as nothing: {}", src);
		res!(round_trip(&d));
	}
	// The article holds every page construct, so its tree must carry them.
	let d = res!(kept(ARTICLE));
	let found = |f: fn(&Block) -> bool| d.blocks.iter().any(|b| match b {
		Block::Div { content, .. } => content.iter().any(f) || f(b),
		_ => f(b),
	});
	assert!(found(|b| matches!(b, Block::Figure { .. })), "No figure in the article.");
	assert!(found(|b| matches!(b, Block::Terms { .. })), "No definition list in the article.");
	assert!(found(|b| matches!(b, Block::Table { head: Some(_), .. })), "No headed table.");
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ GENERATED CORPUS                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// A small deterministic generator, so a failure names a seed that reproduces it.
struct Gen(u64);

impl Gen {
	fn next(&mut self) -> u64 {
		// xorshift64*
		self.0 ^= self.0 >> 12;
		self.0 ^= self.0 << 25;
		self.0 ^= self.0 >> 27;
		self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
	}

	fn below(&mut self, n: usize) -> usize {
		(self.next() % n as u64) as usize
	}

	fn coin(&mut self) -> bool {
		self.below(2) == 0
	}

	/// A string already in §3 rule 5's form, the empty one and whitespace among them.
	fn s(&mut self) -> String {
		const POOL: [&str; 8] = ["", " ", "word", "Ōdate", "a\tb", "two\nlines", "x < y & z", "5,416"];
		POOL[self.below(POOL.len())].to_string()
	}

	fn attrs(&mut self) -> Attrs {
		let mut a = Attrs::default();
		if self.coin() {
			a.id = Some(self.s());
		}
		for _ in 0..self.below(3) {
			a.classes.push(self.s());
		}
		for _ in 0..self.below(3) {
			a.pairs.push((self.s(), self.s()));
		}
		a
	}

	fn inlines(&mut self, depth: usize, cov: &mut Coverage) -> Vec<Inline> {
		(0..self.below(4)).map(|_| self.inline(depth, cov)).collect()
	}

	fn inline(&mut self, depth: usize, cov: &mut Coverage) -> Inline {
		// At the floor only the leaves remain.
		let n = if depth == 0 { 4 } else { 9 };
		let pick = self.below(n);
		cov.inline[pick] = true;
		let d = depth.saturating_sub(1);
		match pick {
			0 => Inline::Text(self.s()),
			1 => Inline::Image { src: self.s(), alt: self.s(), attrs: self.attrs() },
			2 => Inline::Code(self.s()),
			3 => Inline::Break,
			4 => Inline::Emph { strong: self.coin(), content: self.inlines(d, cov) },
			5 => Inline::Link { to: self.s(), content: self.inlines(d, cov) },
			6 => Inline::Span { attrs: self.attrs(), content: self.inlines(d, cov) },
			7 => Inline::Sup(self.inlines(d, cov)),
			_ => Inline::Sub(self.inlines(d, cov)),
		}
	}

	fn blocks(&mut self, depth: usize, cov: &mut Coverage) -> Vec<Block> {
		(0..self.below(4)).map(|_| self.block(depth, cov)).collect()
	}

	fn row(&mut self, depth: usize, cov: &mut Coverage) -> Row {
		Row((0..self.below(4)).map(|_| Cell {
			content:	self.inlines(depth, cov),
			header:		self.coin(),
		}).collect())
	}

	fn block(&mut self, depth: usize, cov: &mut Coverage) -> Block {
		let n = if depth == 0 { 5 } else { 11 };
		let pick = self.below(n);
		cov.block[pick] = true;
		let d = depth.saturating_sub(1);
		match pick {
			0 => Block::Heading { level: 1 + self.below(6) as u8, content: self.inlines(d, cov) },
			1 => Block::Para(self.inlines(depth, cov)),
			2 => Block::Code { lang: if self.coin() { Some(self.s()) } else { None }, text: self.s() },
			3 => Block::Rule,
			4 => Block::Table {
				head:		if self.coin() { Some(self.row(d, cov)) } else { None },
				rows:		(0..self.below(3)).map(|_| self.row(d, cov)).collect(),
				cols:		(0..self.below(4)).map(|i| [Align::None, Align::Start, Align::Centre,
					Align::End][(i + self.below(4)) % 4]).collect(),
				caption:	self.inlines(d, cov),
			},
			5 => Block::List {
				ordered:	self.coin(),
				items:		(0..self.below(3)).map(|_| self.blocks(d, cov)).collect(),
			},
			6 => Block::Quote(self.blocks(d, cov)),
			7 => Block::Div { attrs: self.attrs(), content: self.blocks(d, cov) },
			8 => Block::Figure {
				attrs:		self.attrs(),
				content:	self.blocks(d, cov),
				caption:	self.inlines(d, cov),
			},
			9 => Block::Terms {
				attrs:	self.attrs(),
				items:	(0..self.below(3)).map(|_| Term {
					term:	self.inlines(d, cov),
					defs:	(0..self.below(3)).map(|_| self.blocks(d, cov)).collect(),
				}).collect(),
			},
			_ => Block::Para(vec![self.inline(depth, cov)]),
		}
	}
}

#[derive(Default)]
struct Coverage {
	block:	[bool; 11],
	inline:	[bool; 9],
}

#[test]
fn test_a_generated_corpus_round_trips_01() -> Outcome<()> {
	let mut cov = Coverage::default();
	for seed in 1..=400u64 {
		let mut g = Gen(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
		let d = Doc { blocks: g.blocks(4, &mut cov) };
		match round_trip(&d) {
			Ok(()) => (),
			Err(e) => return Err(err!(e, "Seed {} failed.", seed; Test)),
		}
	}
	assert!(cov.block.iter().all(|b| *b), "Not every block variant was generated: {:?}", cov.block);
	assert!(cov.inline.iter().all(|b| *b), "Not every inline variant was generated: {:?}", cov.inline);
	Ok(())
}

/// Every variant at once, nested four deep: a figure in a division in a definition in a quote.
#[test]
fn test_every_variant_nested_four_deep_round_trips_02() -> Outcome<()> {
	let leaf = vec![
		Inline::Text("a".to_string()),
		Inline::Emph { strong: true, content: vec![Inline::Sup(vec![Inline::Span {
			attrs:		Attrs { id: Some("s".to_string()), classes: vec!["c".to_string()],
				pairs: vec![("k".to_string(), "v".to_string())] },
			content:	vec![Inline::Sub(vec![Inline::Code("x".to_string())])],
		}])] },
		Inline::Link { to: "b".to_string(), content: vec![Inline::Break] },
		Inline::Image { src: "i.png".to_string(), alt: String::new(), attrs: Attrs::default() },
		Inline::Image {
			src:	"img/skull.png".to_string(),
			alt:	"A skull".to_string(),
			attrs:	Attrs {
				id:		Some("skull".to_string()),
				classes:	vec!["diagram".to_string()],
				pairs:		vec![
					("width".to_string(), "568".to_string()),
					("height".to_string(), "260".to_string()),
				],
			},
		},
	];
	let table = Block::Table {
		head:		Some(Row(vec![Cell { content: leaf.clone(), header: true }])),
		rows:		vec![Row(vec![Cell::new(leaf.clone()), Cell { content: Vec::new(), header: true }]),
			Row(Vec::new())],
		cols:		vec![Align::None, Align::Start, Align::Centre, Align::End],
		caption:	leaf.clone(),
	};
	let d = Doc { blocks: vec![Block::Quote(vec![Block::Terms {
		attrs:	Attrs::default(),
		items:	vec![
			Term { term: Vec::new(), defs: Vec::new() },
			Term { term: leaf.clone(), defs: vec![vec![Block::Div {
				attrs:		Attrs { classes: vec!["box".to_string()], ..Default::default() },
				content:	vec![Block::Figure {
					attrs:		Attrs::default(),
					content:	vec![table, Block::Rule, Block::List { ordered: true, items: vec![
						Vec::new(),
						vec![Block::Code { lang: Some(String::new()), text: String::new() }],
					] }],
					caption:	leaf.clone(),
				}, Block::Heading { level: 6, content: Vec::new() }, Block::Para(Vec::new())],
			}], Vec::new()] },
		],
	}])] };
	round_trip(&d)
}

/// `to_doc` refuses what `from_doc` never writes, by name, rather than guessing a block for it.
#[test]
fn test_a_kind_from_doc_never_writes_is_refused_03() -> Outcome<()> {
	use oxedyne_fe2o3_jdat::prelude::*;
	use oxedyne_fe2o3_sbj::{
		kinds::NodeKind,
		text::ukid,
	};
	let mut root = DaticleMap::new();
	root.insert(dat!("title"), dat!("T"));
	root.insert(dat!("lang"), dat!("en"));
	let mut section = DaticleMap::new();
	section.insert(dat!("title"), dat!("S"));
	root.insert(dat!("children"), Dat::List(vec![
		Dat::Usr(ukid(NodeKind::Section), Some(Box::new(Dat::Map(section)))),
	]));
	let tree = Dat::Usr(ukid(NodeKind::Doc), Some(Box::new(Dat::Map(root))));
	match export::to_doc(&tree) {
		Ok(d) => Err(err!("A section came back as {:?}.", d; Test, Unexpected)),
		Err(e) => {
			assert!(fmt!("{}", e).contains("section"), "Unhelpful refusal: {}", e);
			Ok(())
		},
	}
}
