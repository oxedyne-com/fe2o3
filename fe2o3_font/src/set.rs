//! A set of chains, one per role, and the faces the crate carries embedded.

use crate::face::{
	Face,
	Role,
};
use crate::font::Font;

use oxedyne_fe2o3_core::prelude::*;

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THE FACES THE CRATE CARRIES                                               │
// │                                                                            │
// │ Change these seven lines and everything set with the embedded set wears a  │
// │ different typeface. Nothing else needs to know: the chain below keeps the   │
// │ coverage, so a face may be chosen for how it reads and for nothing else.    │
// └───────────────────────────────────────────────────────────────────────────┘

/// The face running text is set in.
const BODY:		&[u8] = include_bytes!("../fonts/NotoSans-Regular.ttf");
/// The same face, emphasised strongly.
const BOLD:		&[u8] = include_bytes!("../fonts/NotoSans-Bold.ttf");
/// The same face, leaning. A real italic, drawn: not the upright one sheared.
const ITALIC:		&[u8] = include_bytes!("../fonts/NotoSans-Italic.ttf");
/// The same face, leaning and strong.
const BOLD_ITALIC:	&[u8] = include_bytes!("../fonts/NotoSans-BoldItalic.ttf");
/// Preserved source, where the columns must line up.
const MONO:		&[u8] = include_bytes!("../fonts/NotoSansMono-Regular.ttf");

/// The face behind all the others: what the chain falls back to.
///
/// DejaVu is not here to be read. It is here because it holds the arrows, the mathematics, the
/// Arabic and the Hebrew that a face chosen for reading does not, and a reader who meets a `→` in a
/// document should see an arrow rather than a box. It is never the first face tried, so it shapes
/// nothing a better face can draw.
const WIDE:		&[u8] = include_bytes!("../fonts/DejaVuSans.ttf");
/// The same, behind the monospaced face, so that preserved source keeps its columns.
const WIDE_MONO:	&[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");

/// The reader's typefaces, one chain per role.
pub struct FontSet {
	/// Running text.
	body:		Font,
	/// Running text, emphasised strongly.
	bold:		Font,
	/// Running text, emphasised.
	italic:		Font,
	/// Running text, emphasised, and strongly.
	bold_italic:	Font,
	/// Preserved source.
	mono:		Font,
}

impl FontSet {

	/// The set the engine carries, so that it renders standalone, identically, anywhere.
	///
	/// Every chain ends in the wide face, so a character the chosen face lacks is still drawn. The
	/// leaning chains fall back to the UPRIGHT wide face rather than to nothing: an arrow has no
	/// italic form, and an upright arrow inside a leaning sentence is what every other renderer shows
	/// there too.
	pub fn embedded() -> Outcome<Self> {
		let wide = || Face::new(WIDE.to_vec());
		Ok(Self {
			body:		res!(Font::chain(vec![res!(Face::new(BODY.to_vec())), res!(wide())])),
			bold:		res!(Font::chain(vec![res!(Face::new(BOLD.to_vec())), res!(wide())])),
			italic:		res!(Font::chain(vec![res!(Face::new(ITALIC.to_vec())), res!(wide())])),
			bold_italic:	res!(Font::chain(vec![
				res!(Face::new(BOLD_ITALIC.to_vec())),
				res!(wide()),
			])),
			mono:		res!(Font::chain(vec![
				res!(Face::new(MONO.to_vec())),
				res!(Face::new(WIDE_MONO.to_vec())),
			])),
		})
	}

	/// A set the reader supplies.
	pub fn new(body: Font, bold: Font, italic: Font, bold_italic: Font, mono: Font) -> Self {
		Self {
			body,
			bold,
			italic,
			bold_italic,
			mono,
		}
	}

	/// The font playing a role.
	pub fn get(&self, role: Role) -> &Font {
		match role {
			Role::Body		=> &self.body,
			Role::Bold		=> &self.bold,
			Role::Italic		=> &self.italic,
			Role::BoldItalic	=> &self.bold_italic,
			Role::Mono		=> &self.mono,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shape::Dir;

	#[test]
	fn test_the_embedded_set_loads_00() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let m = res!(fs.get(Role::Body).metrics(16.0));
		assert!(m.ascent > 0.0, "the ascent should rise above the baseline, found {}", m.ascent);
		assert!(m.descent > 0.0, "the descent should fall below it, found {}", m.descent);
		assert!(m.line_height() > 16.0, "a line is taller than its type size");
		Ok(())
	}

	#[test]
	fn test_shaping_yields_a_glyph_per_letter_01() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let run = res!(fs.get(Role::Body).shape("Hello", 16.0, Dir::Ltr));
		assert_eq!(run.glyphs.len(), 5, "five letters, five glyphs, in a font with no ligature here");
		assert!(run.advance > 0.0, "the pen must travel");
		// The glyphs march rightwards.
		for w in run.glyphs.windows(2) {
			assert!(w[1].x > w[0].x, "glyphs should advance to the right");
		}
		Ok(())
	}

	#[test]
	fn test_an_empty_string_shapes_to_nothing_02() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let run = res!(fs.get(Role::Body).shape("", 16.0, Dir::Ltr));
		assert!(run.is_empty());
		assert_eq!(run.advance, 0.0);
		Ok(())
	}

	#[test]
	fn test_a_glyph_has_an_outline_03() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let font = fs.get(Role::Body);
		let run = res!(font.shape("H", 64.0, Dir::Ltr));
		assert_eq!(run.glyphs.len(), 1);
		let g = run.glyphs[0];
		let path = res!(font.outline(g.face, g.id, 64.0));
		assert!(!path.is_empty(), "the letter H has an outline");
		Ok(())
	}

	#[test]
	fn test_a_space_has_an_advance_but_no_ink_04() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let font = fs.get(Role::Body);
		let run = res!(font.shape(" ", 16.0, Dir::Ltr));
		assert_eq!(run.glyphs.len(), 1);
		assert!(run.advance > 0.0, "a space still moves the pen");
		let g = run.glyphs[0];
		let path = res!(font.outline(g.face, g.id, 16.0));
		assert!(path.is_empty(), "but it lays down no ink");
		Ok(())
	}

	/// The characters that sent me looking for a chain in the first place.
	///
	/// Every one of these appears in real documents already held in the library, and the reading face
	/// draws none of them. Before the chain they were drawn as the "not defined" box, which is the
	/// failure this whole mechanism exists to prevent, and no test caught it because every test used
	/// English. So the test is the evidence: these characters, from the corpus, must reach ink.
	#[test]
	fn test_what_the_reading_face_lacks_is_drawn_by_the_face_behind_it_06() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let font = fs.get(Role::Body);
		for (ch, what) in [
			('\u{2192}', "an arrow"),
			('\u{2295}', "a circled plus"),
			('\u{2264}', "less than or equal to"),
			('\u{2265}', "greater than or equal to"),
			('\u{2248}', "approximately equal to"),
		] {
			let run = res!(font.shape(&fmt!("{}", ch), 32.0, Dir::Ltr));
			assert_eq!(run.glyphs.len(), 1, "{} shapes to one glyph", what);
			let g = run.glyphs[0];
			assert_ne!(g.id, 0, "{} must not be the 'not defined' glyph", what);
			assert!(g.face > 0, "{} comes from a face behind the first, since the first lacks it", what);
			let path = res!(font.outline(g.face, g.id, 32.0));
			assert!(!path.is_empty(), "{} must reach ink", what);
		}
		Ok(())
	}

	/// The reading face draws what it can, and is not passed over for the wide one.
	#[test]
	fn test_ordinary_prose_is_drawn_by_the_reading_face_alone_07() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let font = fs.get(Role::Body);
		let run = res!(font.shape("The quick brown fox, jumps -- over 42 lazy dogs!", 16.0, Dir::Ltr));
		for g in &run.glyphs {
			assert_eq!(g.face, 0, "ordinary prose is the reading face's, and nothing else's");
		}
		Ok(())
	}

	/// A face already in hand keeps the spaces and the punctuation around what it is drawing.
	///
	/// Every face has a space. If the chain were consulted per character rather than the face in hand
	/// being kept, a space would be drawn by whichever face came first, ending the stretch either side
	/// of it -- and a line of prose would be shaped one word at a time, losing every kern across every
	/// join. So the mixed string must come back in as few stretches as it has real changes of face.
	#[test]
	fn test_a_face_in_hand_keeps_what_it_can_draw_08() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let font = fs.get(Role::Body);
		// One change of face and back: the arrow, and nothing else.
		let run = res!(font.shape("from here \u{2192} to there", 16.0, Dir::Ltr));
		let faces: Vec<u8> = run.glyphs.iter().map(|g| g.face).collect();
		let mut changes = 0;
		for w in faces.windows(2) {
			if w[0] != w[1] {
				changes += 1;
			}
		}
		assert_eq!(changes, 2, "into the wide face for the arrow and back out: {:?}", faces);
		Ok(())
	}

	/// A glyph's cluster is a byte offset into the WHOLE string, not into the stretch it was cut into.
	///
	/// This is what a caret is placed by. A face that shaped only the middle of a paragraph reports
	/// offsets into its own fragment, and unless they are put back where they came from every caret
	/// after the first fallback lands in the wrong place.
	#[test]
	fn test_clusters_are_offsets_into_the_whole_string_09() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let font = fs.get(Role::Body);
		let text = "ab \u{2192} cd";
		let run = res!(font.shape(text, 16.0, Dir::Ltr));
		for g in &run.glyphs {
			assert!(
				text.is_char_boundary(g.cluster),
				"cluster {} is a character boundary of {:?}", g.cluster, text,
			);
		}
		// The clusters march forwards across the join, rather than restarting at it.
		let clusters: Vec<usize> = run.glyphs.iter().map(|g| g.cluster).collect();
		let mut sorted = clusters.clone();
		sorted.sort();
		assert_eq!(clusters, sorted, "clusters do not restart at a change of face: {:?}", clusters);
		match clusters.last() {
			Some(last) => assert!(*last > 3, "the text after the arrow is offset past it: {:?}", clusters),
			None => return Err(err!("the string shaped to nothing"; Test, Bug)),
		}
		Ok(())
	}

	/// Emphasis is a real face, not a sheared one.
	#[test]
	fn test_the_set_carries_a_real_italic_and_a_real_bold_italic_10() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		// A true italic has different letterforms, so the same letter is a different SHAPE, not the
		// same shape leaning. Comparing outlines is the only way to tell a real italic from a shear:
		// an obliqued upright would have the same glyph id in the same face.
		let upright = res!(fs.get(Role::Body).shape("a", 64.0, Dir::Ltr));
		let leaning = res!(fs.get(Role::Italic).shape("a", 64.0, Dir::Ltr));
		let bold_leaning = res!(fs.get(Role::BoldItalic).shape("a", 64.0, Dir::Ltr));
		for (run, what) in [(&leaning, "the italic"), (&bold_leaning, "the bold italic")] {
			assert_eq!(run.glyphs.len(), 1, "{} draws one letter", what);
			let g = run.glyphs[0];
			let path = res!(fs.get(Role::Italic).outline(g.face, g.id, 64.0));
			assert!(!path.is_empty(), "{} 'a' has an outline", what);
		}
		// The italic 'a' is a different width from the upright one, which a shear cannot change: a
		// shear moves the tops of the letters and leaves the advance exactly where it was.
		assert!(
			(upright.advance - leaning.advance).abs() > 0.01,
			"a real italic is a drawn face, so its 'a' is not the upright's width: {} then {}",
			upright.advance, leaning.advance,
		);
		Ok(())
	}

	#[test]
	fn test_bigger_type_travels_further_05() -> Outcome<()> {
		let fs = res!(FontSet::embedded());
		let font = fs.get(Role::Body);
		let small = res!(font.shape("Hello", 16.0, Dir::Ltr));
		let big = res!(font.shape("Hello", 32.0, Dir::Ltr));
		assert!(
			(big.advance - 2.0 * small.advance).abs() < 0.5,
			"twice the size should be twice the width: {} then {}", small.advance, big.advance,
		);
		Ok(())
	}
}
