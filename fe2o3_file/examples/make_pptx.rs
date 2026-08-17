//! Writes a `.pptx` from a Markdown file, so an external reader can be asked what it makes of it.
//!
//! A deck is the format where a self-consistent test proves least, because the whole of the risk is
//! the SKELETON: a slide points at a layout, a layout at a master, a master at a theme, and a file
//! with any link missing opens with a repair prompt rather than an error. Nothing this crate's own
//! reader does would notice.
//!
//! ```text
//! cargo run -p oxedyne_fe2o3_file --example make_pptx -- in.md out.pptx
//! soffice --headless --convert-to pdf out.pptx
//! ```
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_file::office::deck::Deck;
use oxedyne_fe2o3_file::office::pptx;
use oxedyne_fe2o3_text::doc::markdown;

fn main() -> Outcome<()> {
	let args: Vec<String> = std::env::args().collect();
	if args.len() != 3 {
		return Err(err!("Usage: make_pptx <markdown in> <pptx out>"; Invalid, Input));
	}
	let src = res!(std::fs::read_to_string(&args[1]), IO, File);
	let doc = res!(markdown::parse(&src));
	let deck = Deck::from_doc(&doc);
	let (bytes, left) = res!(pptx::write(&deck));
	res!(std::fs::write(&args[2], &bytes), IO, File);
	println!("{}: {} bytes, {} slides", args[2], bytes.len(), deck.slides.len());
	for src in &left.images {
		println!("not carried: the image at {}", src);
	}
	if left.notes > 0 {
		println!("not carried: speaker's notes on {} slide(s)", left.notes);
	}
	Ok(())
}
