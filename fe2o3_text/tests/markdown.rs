use oxedyne_fe2o3_text::doc::{
	Block,
	Doc,
	Inline,
	markdown,
};

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};

/// Prose using every construct the tree has a node for.
const SOURCE: &str = "\
# A title

A paragraph with **strong**, *emphasis*, `code`, a [link](https://example.com) and an
![image](pic.png).

## Second

- One
- Two
    - Nested
    - Also nested
- Three

1. First
2. Second

> Quoted, and it goes on.

| Name | Count |
| :-- | --: |
| Widget | 12 |

```rust
fn main() {}
```

---

The end.
";

pub fn test_markdown(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["A tree written and read back is the same tree 000", "all", "markdown"], || {
		// The writer's oracle is the reader, which is tested on its own terms elsewhere. This says the
		// two AGREE; it says nothing about either matching CommonMark, and is not offered as if it did.
		// What it does catch, and catches nothing else in this crate does, is an escape that changes
		// the meaning of a character and an indent that changes what a list is nested inside.
		let once = res!(markdown::parse(SOURCE));
		let out = markdown::write::render(&once);
		let twice = res!(markdown::parse(&out));
		assert_eq!(once, twice, "written as:\n{}", out);
		// And it is stable: writing the tree again gives the same text.
		assert_eq!(markdown::write::render(&twice), out);
		Ok(())
	}));

	res!(test_it(filter, &["A nested list nests under its parent's text 001", "all", "markdown"], || {
		let doc = res!(markdown::parse(SOURCE));
		let out = markdown::write::render(&doc);
		assert!(out.contains("- Two\n  - Nested\n"), "got:\n{}", out);
		// Which is to say the indent is the width of the marker above it, and not a fixed depth that
		// doubles at every level.
		assert!(!out.contains("        - Nested"), "the indent compounded:\n{}", out);
		Ok(())
	}));

	res!(test_it(filter, &["An escape goes where it changes the meaning 002", "all", "markdown"], || {
		// A hyphen opening a line opens a list; one in the middle of a sentence is a hyphen. A writer
		// that escaped both would fill the document with backslashes, and one that escaped neither
		// would turn a sentence into a bullet the next time it was read.
		let doc = Doc {
			blocks: vec![
				Block::Para(vec![Inline::Text("- not a list".to_string())]),
				Block::Para(vec![Inline::Text("a - b - c".to_string())]),
				Block::Para(vec![Inline::Text("1. not a list".to_string())]),
				Block::Table {
					head:	None,
					rows:	vec![oxedyne_fe2o3_text::doc::Row(vec![
						oxedyne_fe2o3_text::doc::Cell(vec![Inline::Text("3.40".to_string())]),
						oxedyne_fe2o3_text::doc::Cell(vec![Inline::Text("a | b".to_string())]),
					])],
					cols:	vec![],
				},
			],
		};
		let out = markdown::write::render(&doc);
		assert!(out.contains("\\- not a list"), "got:\n{}", out);
		assert!(out.contains("a - b - c"), "a hyphen mid-sentence is left alone:\n{}", out);
		assert!(out.contains("1\\. not a list"), "got:\n{}", out);
		// A price in a cell is not the start of a line, whatever it looks like.
		assert!(out.contains("| 3.40 |"), "a cell got a spurious escape:\n{}", out);
		assert!(out.contains("a \\| b"), "a pipe in a cell would end it:\n{}", out);
		// And all of it reads back as what it said.
		let back = res!(markdown::parse(&out));
		assert_eq!(back.blocks[0], doc.blocks[0]);
		assert_eq!(back.blocks[1], doc.blocks[1]);
		assert_eq!(back.blocks[2], doc.blocks[2]);
		Ok(())
	}));

	res!(test_it(filter, &["A fence is longer than anything inside it 003", "all", "markdown"], || {
		// A listing about Markdown holds a fence, and a writer that always used three backticks would
		// close its own block at the first one.
		let doc = Doc {
			blocks: vec![Block::Code {
				lang:	None,
				text:	"```\nnot the end\n```\n".to_string(),
			}],
		};
		let out = markdown::write::render(&doc);
		assert!(out.starts_with("````\n"), "got:\n{}", out);
		let back = res!(markdown::parse(&out));
		assert_eq!(back.blocks, doc.blocks);
		Ok(())
	}));

	Ok(())
}
