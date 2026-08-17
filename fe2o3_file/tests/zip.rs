use oxedyne_fe2o3_file::zip::{
	Method,
	Zip,
};

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};

/// An archive written by Info-ZIP, holding a deflated member, a stored member and one in a
/// subdirectory. It carries extra fields in both its local headers and its directory entries, which
/// is the part a rebuilt archive loses and a copied one keeps.
const FOREIGN: &[u8] = include_bytes!("data/foreign.zip");

/// A `.docx` written by LibreOffice: ten members, a foreign writer, and the shape every Office
/// document has. Nothing here reads WordprocessingML; this is an archive from somebody else's
/// implementation, which is the only kind worth testing a round trip against.
const DOCX: &[u8] = include_bytes!("data/loffice.docx");

pub fn test_zip(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["A foreign archive survives a round trip byte for byte 000", "all", "zip"], || {
		// The property everything above this rests on. Not "opens afterwards" and not "holds the same
		// content" -- the same bytes, so a member nobody understood is a member nobody damaged.
		for (what, src) in [("the Info-ZIP archive", FOREIGN), ("the LibreOffice .docx", DOCX)] {
			let zip = res!(Zip::read(src.to_vec()));
			assert!(zip.is_pristine(), "{} was read and nothing was touched", what);
			let out = res!(zip.write());
			assert_eq!(out.len(), src.len(), "{} changed length on a round trip", what);
			match out == src {
				true	=> {}
				false	=> {
					let at = out.iter().zip(src).position(|(a, b)| a != b);
					panic!("{} differs at byte {:?} after a round trip", what, at);
				}
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["A member's content is what was put in it 001", "all", "zip"], || {
		let zip = res!(Zip::read(FOREIGN.to_vec()));
		assert_eq!(zip.len(), 3);
		assert_eq!(zip.names(), vec!["alpha.txt", "tiny.bin", "sub/beta.txt"]);
		// The deflated one.
		let alpha = res!(zip.text("alpha.txt"));
		assert!(alpha.starts_with("hello hello"), "got {:?}", alpha);
		assert_eq!(alpha.len(), 72);
		assert_eq!(res!(zip.member("alpha.txt").ok_or_else(|| err!("no alpha"; Missing))).method,
			Method::Deflate);
		// The stored one, which must not go near the inflater.
		assert_eq!(res!(zip.content("tiny.bin")), b"x".to_vec());
		assert_eq!(res!(zip.member("tiny.bin").ok_or_else(|| err!("no tiny"; Missing))).method,
			Method::Store);
		// And one with a path in its name.
		assert_eq!(res!(zip.text("sub/beta.txt")), "The quick brown fox jumps over the lazy dog.\n");
		Ok(())
	}));

	res!(test_it(filter, &["An unknown member is named rather than lost 002", "all", "zip"], || {
		// A `.docx` holds ten parts and this crate understands none of them. It still names all ten,
		// which is what lets the layer above decide what to parse and what to copy.
		let zip = res!(Zip::read(DOCX.to_vec()));
		assert_eq!(zip.len(), 10);
		assert!(zip.has("word/document.xml"));
		assert!(zip.has("[Content_Types].xml"));
		assert!(zip.has("word/theme/theme1.xml"), "the part an editor most often loses");
		let doc = res!(zip.text("word/document.xml"));
		assert!(doc.contains("<w:body>"), "the document part is XML");
		Ok(())
	}));

	res!(test_it(filter, &["Replacing one member leaves the others byte for byte 003", "all", "zip"], || {
		// The second property: an edit reaches the part it names and no other. Checked against the
		// bytes in the archive rather than against the content, because two members can hold the same
		// content and different bytes, and it is the bytes a colleague's reader will parse.
		let before = res!(Zip::read(DOCX.to_vec()));
		let mut after = res!(Zip::read(DOCX.to_vec()));
		let edited = "<?xml version=\"1.0\"?><w:document/>";
		after.set("word/document.xml", edited.as_bytes().to_vec(), Method::Deflate);
		assert!(!after.is_pristine(), "the archive was touched");
		let out = res!(after.write());
		assert_ne!(out, DOCX.to_vec(), "the edit reached the file");
		let back = res!(Zip::read(out));
		assert_eq!(back.len(), before.len(), "no member was added or lost");
		assert_eq!(back.names(), before.names(), "no member moved");
		let mut differ = Vec::new();
		for m in before.members() {
			let was = res!(before.content(&m.name));
			let now = res!(back.content(&m.name));
			if was != now {
				differ.push(m.name.clone());
			}
		}
		assert_eq!(differ, vec!["word/document.xml".to_string()],
			"exactly one member changed");
		assert_eq!(res!(back.text("word/document.xml")), edited);
		// And the untouched members are the same bytes, not merely the same content.
		for m in before.members() {
			if m.name == "word/document.xml" {
				continue;
			}
			let a = res!(m.raw(&before));
			let b = res!(res!(back.member(&m.name).ok_or_else(|| err!(
				"'{}' went missing", m.name; Missing))).raw(&back));
			assert_eq!(a, b, "'{}' was re-compressed rather than copied", m.name);
		}
		Ok(())
	}));

	res!(test_it(filter, &["An archive built from nothing reads back 004", "all", "zip"], || {
		let mut zip = Zip::new();
		zip.set("mimetype", b"application/vnd.oasis.opendocument.text".to_vec(), Method::Store);
		zip.set("a/b.txt", b"some content that is long enough to be worth deflating, twice over, \
			some content that is long enough to be worth deflating".to_vec(), Method::Deflate);
		zip.set("empty.txt", Vec::new(), Method::Deflate);
		let out = res!(zip.write());
		let back = res!(Zip::read(out.clone()));
		assert_eq!(back.names(), vec!["mimetype", "a/b.txt", "empty.txt"]);
		assert_eq!(res!(back.text("mimetype")), "application/vnd.oasis.opendocument.text");
		assert_eq!(res!(back.content("empty.txt")), Vec::<u8>::new());
		assert!(res!(back.text("a/b.txt")).starts_with("some content"));
		// Written twice, the same bytes: there is no clock in the file, so a build that has to be
		// reproducible stays reproducible.
		let again = res!(zip.write());
		assert_eq!(out, again, "the writer put the time of day in the archive");
		Ok(())
	}));

	res!(test_it(filter, &["A member may be put at the head of the archive 005", "all", "zip"], || {
		// OpenDocument requires it: `mimetype` first and stored, or a reader calls the file a ZIP.
		let mut zip = Zip::new();
		zip.set("content.xml", b"<x/>".to_vec(), Method::Deflate);
		zip.set_first("mimetype", b"application/vnd.oasis.opendocument.text".to_vec(), Method::Store);
		assert_eq!(zip.names(), vec!["mimetype", "content.xml"]);
		let out = res!(zip.write());
		// Stored and first means the media type sits at a known offset in the raw bytes, which is how
		// every reader identifies the file.
		assert_eq!(&out[30..38], b"mimetype");
		assert_eq!(&out[38..77], b"application/vnd.oasis.opendocument.text");
		Ok(())
	}));

	res!(test_it(filter, &["Damage is refused rather than guessed at 006", "all", "zip"], || {
		// Not an archive at all.
		assert!(Zip::read(b"not an archive, not even close".to_vec()).is_err());
		// The directory cut off, which is what a half-downloaded file looks like.
		let cut = FOREIGN[..FOREIGN.len() - 40].to_vec();
		assert!(Zip::read(cut).is_err(), "a truncated archive is refused");
		// A member whose bytes no longer hash to what the directory says.
		let mut bent = FOREIGN.to_vec();
		// The first member's deflated data begins after its local header, name and extra field.
		let nlen = u16::from_le_bytes([bent[26], bent[27]]) as usize;
		let elen = u16::from_le_bytes([bent[28], bent[29]]) as usize;
		let at = 30 + nlen + elen;
		bent[at] ^= 0xFF;
		let zip = res!(Zip::read(bent));
		assert!(zip.content("alpha.txt").is_err(), "a damaged member is refused, not returned");
		Ok(())
	}));

	res!(test_it(filter, &["A member is not inflated past a stated ceiling 007", "all", "zip"], || {
		let zip = res!(Zip::read(FOREIGN.to_vec()));
		assert!(zip.content_capped("alpha.txt", 71).is_err(), "72 bytes is over a 71 byte ceiling");
		assert_eq!(res!(zip.content_capped("alpha.txt", 72)).len(), 72);
		Ok(())
	}));

	res!(test_it(filter, &["A member that is not there is said to not be there 008", "all", "zip"], || {
		let mut zip = res!(Zip::read(FOREIGN.to_vec()));
		assert!(!zip.has("nothing.txt"));
		let e = zip.content("nothing.txt");
		assert!(e.is_err());
		assert!(!zip.remove("nothing.txt"));
		assert!(zip.is_pristine(), "removing nothing touches nothing");
		Ok(())
	}));

	Ok(())
}
