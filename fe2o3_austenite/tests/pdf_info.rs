//! The PDF Info dictionary, read back from the file by `pdfinfo` and held against Typst's own for the same
//! source: `Title`, `Author`, `Subject` and `Keywords` from the document's `#set document(...)`, and
//! `Creator` and `Producer` naming the engine. Compiled through `compile::emit_pdf`, the emit both the
//! wasm surface and the native binary call.

use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{
	Path,
	PathBuf,
};
use std::process::Command;
use std::sync::{
	Arc,
	Mutex,
};

// The source map is a process-wide global, so the cases take turns.
static VFS: Mutex<()> = Mutex::new(());

const METADATA: [&str; 4] = ["Title", "Author", "Subject", "Keywords"];

fn compile_pdf(src: &str) -> Outcome<Vec<u8>> {
	let _turn = VFS.lock().unwrap_or_else(|p| p.into_inner());
	let main = PathBuf::from("/doc/main.typ");
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	files.insert(main.clone(), src.as_bytes().to_vec());
	res!(vfs::install(files));
	let set		= Arc::new(res!(fonts::libertinus()));
	let result	= compile::assemble(&main, || Ok(set.clone()))
		.and_then(|(a, _, _)| compile::author_and_run(a));
	let _ = vfs::clear();
	let rendered = res!(result);
	let mut out = rendered.out;
	compile::emit_pdf(&mut out, &rendered.heads, &rendered.doc_info)
}

fn scratch(name: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

/// The Info fields `pdfinfo` reads from a PDF, by key.
fn info_of(pdf: &Path) -> Outcome<BTreeMap<String, String>> {
	let out = res!(Command::new("pdfinfo").arg(pdf).output());
	if !out.status.success() {
		return Err(err!("pdfinfo could not read {:?}: {}", pdf, String::from_utf8_lossy(&out.stderr); Test));
	}
	let mut fields = BTreeMap::new();
	for line in res!(String::from_utf8(out.stdout)).lines() {
		if let Some((key, value)) = line.split_once(':') {
			if METADATA.contains(&key) || key == "Creator" || key == "Producer" {
				fields.insert(key.to_string(), value.trim().to_string());
			}
		}
	}
	Ok(fields)
}

/// Typst's own Info fields for `src`, or `None` when no `typst` is installed to ask.
fn typst_info(src: &str, name: &str) -> Outcome<Option<BTreeMap<String, String>>> {
	let typ = scratch(&fmt!("pdf_info_{}.typ", name));
	let pdf = scratch(&fmt!("pdf_info_{}_typst.pdf", name));
	res!(std::fs::write(&typ, src));
	let out = match Command::new("typst").arg("compile").arg(&typ).arg(&pdf).output() {
		Ok(o)	=> o,
		Err(_)	=> {
			eprintln!("SKIP: `typst` is not installed, so {} is not compared with it.", name);
			return Ok(None);
		},
	};
	if !out.status.success() {
		return Err(err!("typst refused {}: {}", name, String::from_utf8_lossy(&out.stderr); Test));
	}
	Ok(Some(res!(info_of(&pdf))))
}

#[test]
fn the_info_dictionary_is_typsts_for_the_same_source() -> Outcome<()> {
	let cases = [
		("all", "#set document(\n  title: \"A Title\",\n  author: (\"Ann Author\", \"Bob\"),\n  \
			description: \"A subject line\",\n  keywords: (\"one\", \"two\"),\n)\n\n= Heading\n\nBody words.\n"),
		("content", "#set document(title: [A *bold* Title], \
			description: \"\\u{dc}n\\u{ef}c\\u{f6}d\\u{e9} \\\"quoted\\\"\", author: \"Solo\")\n\n= Heading\n\nBody.\n"),
		("override", "#set document(title: \"One\", keywords: \"k\")\n#set document(title: \"Two\")\n\n= H\n\nBody.\n"),
		("none", "= Heading\n\nBody words.\n"),
	];
	for (name, src) in cases {
		let pdf = scratch(&fmt!("pdf_info_{}.pdf", name));
		res!(std::fs::write(&pdf, res!(compile_pdf(src))));
		let ours = res!(info_of(&pdf));
		assert_eq!(ours.get("Creator").map(|s| s.as_str()), Some("Austenite"), "{}", name);
		assert_eq!(ours.get("Producer").map(|s| s.as_str()), Some("Austenite"), "{}", name);
		if let Some(theirs) = res!(typst_info(src, name)) {
			for key in METADATA {
				assert_eq!(ours.get(key), theirs.get(key), "{}: {} (ours {:?}, typst {:?})", name, key, ours, theirs);
			}
		}
	}

	// Typst 0.15.1's own reading of the first case, so the fields are checked where no typst is installed.
	let ours = res!(info_of(&scratch("pdf_info_all.pdf")));
	let want = [("Title", "A Title"), ("Author", "Ann Author, Bob"), ("Subject", "A subject line"),
		("Keywords", "one, two")];
	for (key, value) in want {
		assert_eq!(ours.get(key).map(|s| s.as_str()), Some(value), "{}", key);
	}
	let none = res!(info_of(&scratch("pdf_info_none.pdf")));
	assert!(METADATA.iter().all(|k| !none.contains_key(*k)), "an unset field writes no entry: {:?}", none);
	Ok(())
}

/// A `#set document` shown in a raw block or written in a comment is text, so it writes no Info entry, as
/// in Typst.
#[test]
fn a_shown_or_commented_set_document_writes_no_info() -> Outcome<()> {
	let src = "```typst\n#set document(title: \"Example Title\")\n```\n\
		/*\n#set document(author: \"Commented Out\")\n*/\n= Heading\n\nBody.\n";
	let pdf = scratch("pdf_info_shown.pdf");
	res!(std::fs::write(&pdf, res!(compile_pdf(src))));
	let ours = res!(info_of(&pdf));
	assert!(METADATA.iter().all(|k| !ours.contains_key(*k)), "no field was set: {:?}", ours);
	if let Some(theirs) = res!(typst_info(src, "shown")) {
		assert!(METADATA.iter().all(|k| !theirs.contains_key(*k)), "typst sets none either: {:?}", theirs);
	}
	Ok(())
}
