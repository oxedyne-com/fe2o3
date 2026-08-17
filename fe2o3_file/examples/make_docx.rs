//! Writes a `.docx` from a Markdown file, so the result can be put to an external reader.
//!
//! The point of this is not convenience. A test that reads back what this crate wrote proves that
//! this crate agrees with itself, which is worth very little for a format whose whole purpose is to
//! be opened by somebody else's program. So the file this makes is meant to be handed to LibreOffice
//! or to Word:
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_file --example make_docx -- in.md out.docx
//! soffice --headless --convert-to txt out.docx
//! diff <(sed ... in.md) out.txt
//! ```
//!
//! `dev/docx_oracle.sh` does exactly that and reports the difference.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_file::office::docx;
use oxedyne_fe2o3_text::doc::markdown;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	if args.len() != 3 {
		return Err(err!(
			"Usage: make_docx <markdown in> <docx out>"; Invalid, Input));
	}
	let src = res!(std::fs::read_to_string(&args[1]), IO, File);
	let doc = res!(markdown::parse(&src));
	let (bytes, left) = res!(docx::write(&doc));
	res!(std::fs::write(&args[2], &bytes), IO, File);
	println!("{}: {} bytes, {} blocks", args[2], bytes.len(), doc.blocks.len());
	// What did not fit is said rather than dropped in silence.
	for src in &left.images {
		println!("not drawn: the image at {}", src);
	}
	Ok(())
}
