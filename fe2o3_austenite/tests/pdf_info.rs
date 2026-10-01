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
		.and_then(compile::author_and_run);
	let _ = vfs::clear();
	let rendered = res!(result);
	let mut out = rendered.out;
	compile::emit_pdf(&mut out, &rendered.heads, &rendered.doc_info)
}

/// Compiles a project of several sources, rooted at the first, as `compile_pdf` compiles a lone one.
fn compile_project(files: &[(&str, &str)]) -> Outcome<Vec<u8>> {
	let _turn = VFS.lock().unwrap_or_else(|p| p.into_inner());
	let main = match files.first() {
		Some((path, _))	=> PathBuf::from(path),
		None			=> return Err(err!("A project needs a root source."; Test, Missing)),
	};
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (path, text) in files {
		map.insert(PathBuf::from(path), text.as_bytes().to_vec());
	}
	res!(vfs::install(map));
	let set		= Arc::new(res!(fonts::libertinus()));
	let result	= compile::assemble(&main, || Ok(set.clone()))
		.and_then(compile::author_and_run);
	let _ = vfs::clear();
	let rendered = res!(result);
	let mut out = rendered.out;
	compile::emit_pdf(&mut out, &rendered.heads, &rendered.doc_info)
}

/// Typst's own Info fields for the project `files`, written by their file names into a scratch directory
/// of their own and compiled from the first, or `None` when no `typst` is installed to ask.
fn typst_project_info(files: &[(&str, &str)], name: &str) -> Outcome<Option<BTreeMap<String, String>>> {
	let dir = scratch(&fmt!("pdf_info_{}", name));
	res!(std::fs::create_dir_all(&dir));
	let mut root: Option<PathBuf> = None;
	for (path, text) in files {
		let file = match Path::new(path).file_name() {
			Some(f)	=> dir.join(f),
			None	=> return Err(err!("{:?} names no file.", path; Test, Invalid)),
		};
		res!(std::fs::write(&file, text));
		root.get_or_insert(file);
	}
	let root = match root {
		Some(r)	=> r,
		None	=> return Err(err!("A project needs a root source."; Test, Missing)),
	};
	let pdf = dir.join("typst.pdf");
	let out = match Command::new("typst").arg("compile").arg(&root).arg(&pdf).output() {
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

/// An included file's own top-level `#set document` is applied where the include stands, between the root's
/// rules before and after it, field by field, as Typst applies it.
#[test]
fn an_included_files_set_document_is_applied_where_it_stands() -> Outcome<()> {
	let files = [
		("/doc/main.typ", "#set document(title: \"Root Title\", author: \"Root Author\")\n= H\n\nBody.\n\n\
			#include \"ch1.typ\"\n\n#set document(keywords: \"after\")\n"),
		("/doc/ch1.typ", "#set document(title: \"From Chapter\")\n== Sub\n\nText.\n"),
	];
	let pdf = scratch("pdf_info_included.pdf");
	res!(std::fs::write(&pdf, res!(compile_project(&files))));
	let ours = res!(info_of(&pdf));
	let want = [("Title", "From Chapter"), ("Author", "Root Author"), ("Keywords", "after")];
	for (key, value) in want {
		assert_eq!(ours.get(key).map(|s| s.as_str()), Some(value), "{}: {:?}", key, ours);
	}
	if let Some(theirs) = res!(typst_project_info(&files, "included")) {
		for key in METADATA {
			assert_eq!(ours.get(key), theirs.get(key), "{} (ours {:?}, typst {:?})", key, ours, theirs);
		}
	}
	Ok(())
}

/// Compiles `files`, rooted at the first, as `compile_project` does, and returns the PDF with every site the
/// compile reports, as `(line, col, message)`.
fn compile_with_sites(files: &[(&str, &str)]) -> Outcome<(Vec<u8>, Vec<(usize, usize, String)>)> {
	let _turn = VFS.lock().unwrap_or_else(|p| p.into_inner());
	let main = match files.first() {
		Some((path, _))	=> PathBuf::from(path),
		None			=> return Err(err!("A project needs a root source."; Test, Missing)),
	};
	let mut map: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	for (path, text) in files {
		map.insert(PathBuf::from(path), text.as_bytes().to_vec());
	}
	res!(vfs::install(map));
	let set = Arc::new(res!(fonts::libertinus()));
	// The report reads each site's line from the installed source, so it is built before the map is cleared.
	let run = || -> Outcome<(compile::Rendered, compile::Report)> {
		let assembled	= res!(compile::assemble(&main, || Ok(set.clone())));
		let empty		= assembled.blocks.is_empty();
		let rendered	= res!(compile::author_and_run(assembled));
		let report		= compile::Report::new(rendered.out.pages.len(), &rendered.refusals, empty);
		Ok((rendered, report))
	};
	let result = run();
	let _ = vfs::clear();
	let (mut rendered, report) = res!(result);
	let sites = report.diagnostics.iter().map(|d| (d.line, d.col, d.message.clone())).collect();
	let pdf = res!(compile::emit_pdf(&mut rendered.out, &rendered.heads, &rendered.doc_info));
	Ok((pdf, sites))
}

/// The first error Typst reports for `src`, or `None` when no `typst` is installed to ask. An error is
/// required: the source is one Typst refuses.
fn typst_error(src: &str, name: &str) -> Outcome<Option<String>> {
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
	if out.status.success() {
		return Err(err!("typst compiled {}, which it should refuse.", name; Test));
	}
	let text = String::from_utf8_lossy(&out.stderr).to_string();
	Ok(Some(text.lines().find(|l| l.starts_with("error:")).unwrap_or("").to_string()))
}

/// The text `pdftotext` reads from a PDF, the words joined by single spaces.
fn text_of(pdf: &[u8], name: &str) -> Outcome<String> {
	let path = scratch(&fmt!("pdf_info_{}_text.pdf", name));
	res!(std::fs::write(&path, pdf));
	let out = res!(Command::new("pdftotext").arg(&path).arg("-").output());
	Ok(res!(String::from_utf8(out.stdout)).split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A `#set document` in a container -- a list item, strong or emphasis, or after a `*` or `_` that opens
/// one prose never closes -- sets no field and is refused at its site, as Typst refuses it. Each of the
/// cases below is a probe of the lexer audit (`d4`, `e4`, `e5`, `u4`, `u2`).
fn assert_container_refused(name: &str, src: &str, line: usize, col: usize, typst: &str) -> Outcome<()> {
	let (pdf, sites) = res!(compile_with_sites(&[("/doc/main.typ", src)]));
	let path = scratch(&fmt!("pdf_info_{}.pdf", name));
	res!(std::fs::write(&path, &pdf));
	let ours = res!(info_of(&path));
	assert!(!ours.contains_key("Title"), "{}: the rule in a container set a title: {:?}", name, ours);
	let held: Vec<&(usize, usize, String)> = sites.iter()
		.filter(|(_, _, m)| m.contains("not allowed inside of containers")).collect();
	assert_eq!(held.len(), 1, "{}: one site for the rule: {:?}", name, sites);
	assert_eq!((held[0].0, held[0].1), (line, col), "{}: the site stands at the rule's `#`: {:?}", name, sites);
	assert!(held[0].2.starts_with("#set document"), "{}: {:?}", name, held[0]);
	if let Some(theirs) = res!(typst_error(src, name)) {
		assert!(theirs.contains(typst), "{}: typst refuses with {:?}", name, theirs);
	}
	Ok(())
}

#[test]
fn set_document_in_strong_is_refused_at_its_site() -> Outcome<()> {
	assert_container_refused("strong", "= Root\n\n*bold [\n#set document(title: \"Strong Title\")\nstill bold*\n\nBody.\n",
		4, 1, "not allowed inside of containers")
}

#[test]
fn set_document_in_emphasis_is_refused_at_its_site() -> Outcome<()> {
	assert_container_refused("emph", "= Root\n\n_emph\n#set document(title: \"Emph Title\")\nstill_\n\nBody.\n",
		4, 1, "not allowed inside of containers")
}

#[test]
fn set_document_in_list_item_is_refused_at_its_site() -> Outcome<()> {
	assert_container_refused("item", "= Root\n\n- item\n  #set document(title: \"Item Title\")\n\nBody.\n",
		4, 3, "not allowed inside of containers")
}

/// A `*` between spaces opens strong that nothing closes, so the rule after it is in a container; Typst
/// refuses the file for the unclosed delimiter before it reaches the rule.
#[test]
fn set_document_after_unclosed_star_is_refused() -> Outcome<()> {
	assert_container_refused("star", "= Root\n\n2 * 3 = 6\n#set document(title: \"Star Title\")\n\nBody.\n",
		4, 1, "unclosed delimiter")
}

/// A `_` between letters of a script that writes no spaces between words opens emphasis, as Typst reads it,
/// so the rule after an unclosed one is in a container. One between Latin letters is within a word, and
/// opens nothing.
#[test]
fn set_document_after_cjk_underscore_is_refused() -> Outcome<()> {
	assert_container_refused("cjk", "= Root\n\n\u{65E5}\u{672C}_\u{8A9E}\n#set document(title: \"Cjk Title\")\n\nBody.\n",
		4, 1, "unclosed delimiter")?;
	let word = "= Root\n\nsnake_case and a*b\n#set document(title: \"Word Title\")\n\nBody.\n";
	let (pdf, sites) = res!(compile_with_sites(&[("/doc/main.typ", word)]));
	let path = scratch("pdf_info_word.pdf");
	res!(std::fs::write(&path, &pdf));
	assert_eq!(res!(info_of(&path)).get("Title").map(|s| s.as_str()), Some("Word Title"));
	assert!(sites.is_empty(), "markers within a word open nothing: {:?}", sites);
	Ok(())
}

/// A bare content block is not a container: Typst joins it into the markup around it, so a `#set document` in
/// it applies where it stands, and its brackets are not shown. One in a `#box` is in a container, and refused.
#[test]
fn set_document_in_bare_block_applies() -> Outcome<()> {
	let src = "= Root\n\n#[\n#set document(title: \"Block Title\")\nIn block.\n]\n\nBody.\n";
	let (pdf, sites) = res!(compile_with_sites(&[("/doc/main.typ", src)]));
	let path = scratch("pdf_info_bare.pdf");
	res!(std::fs::write(&path, &pdf));
	let ours = res!(info_of(&path));
	assert_eq!(ours.get("Title").map(|s| s.as_str()), Some("Block Title"), "{:?}", ours);
	assert!(sites.is_empty(), "nothing in the block is refused: {:?}", sites);
	let text = res!(text_of(&pdf, "bare"));
	assert!(text.contains("In block.") && text.contains("Body."), "{}", text);
	assert!(!text.contains("#[") && !text.contains(']'), "the block's brackets are not prose: {}", text);
	if let Some(theirs) = res!(typst_info(src, "bare")) {
		assert_eq!(ours.get("Title"), theirs.get("Title"), "typst: {:?}", theirs);
	}
	Ok(())
}

#[test]
fn set_document_in_box_is_refused() -> Outcome<()> {
	let src = "= Root\n\n#box[\n#set document(title: \"X\")\n]\n\nBody.\n";
	let (pdf, sites) = res!(compile_with_sites(&[("/doc/main.typ", src)]));
	let path = scratch("pdf_info_box.pdf");
	res!(std::fs::write(&path, &pdf));
	assert!(!res!(info_of(&path)).contains_key("Title"));
	assert!(!sites.is_empty(), "the box and the rule in it are refused at a site");
	if let Some(theirs) = res!(typst_error(src, "box")) {
		assert!(theirs.contains("not allowed inside of containers"), "{:?}", theirs);
	}
	Ok(())
}

/// Whatever a `#set document` stands in, it either sets a field or is refused at a site: none is passed over
/// in silence.
#[test]
fn no_set_document_is_passed_without_a_site() -> Outcome<()> {
	let cases = [
		("a", "= R\n\n*b [\n#set document(title: \"T\")\nb*\n\nB.\n"),
		("b", "= R\n\n_e\n#set document(title: \"T\")\ne_\n\nB.\n"),
		("c", "= R\n\n- i\n  #set document(title: \"T\")\n\nB.\n"),
		("d", "= R\n\n2 * 3\n#set document(title: \"T\")\n\nB.\n"),
		("e", "= R\n\n#[\n#set document(title: \"T\")\nx\n]\n\nB.\n"),
		("f", "= R\n\n#box[\n#set document(title: \"T\")\n]\n\nB.\n"),
		("g", "= R\n\n#set document(title: \"T\")\n\nB.\n"),
	];
	for (name, src) in cases {
		let (pdf, sites) = res!(compile_with_sites(&[("/doc/main.typ", src)]));
		let path = scratch(&fmt!("pdf_info_none_{}.pdf", name));
		res!(std::fs::write(&path, &pdf));
		let titled = res!(info_of(&path)).contains_key("Title");
		assert!(titled || !sites.is_empty(), "{}: no title and no site", name);
		assert!(!(titled && sites.iter().any(|(_, _, m)| m.contains("#set document"))),
			"{}: a rule is applied and refused at once: {:?}", name, sites);
	}
	Ok(())
}

/// A `#set document` in code -- in a bare content block in the branch of a conditional, or in a list item
/// there -- is joined into nothing the file's own lines hold, so it is refused where it stands rather than
/// passed over.
#[test]
fn set_document_in_a_conditional_branch_below_its_level_is_refused() -> Outcome<()> {
	let cases = [
		("block",	"#let media = \"ebook\"\n\n= Root\n\n#if media == \"ebook\" [\n#[\n#set document(title: \"T\")\n]\n]\n\nBody.\n"),
		("item",	"#let media = \"ebook\"\n\n= Root\n\n#if media == \"ebook\" [\n- x\n  #set document(title: \"T\")\n]\n\nBody.\n"),
	];
	for (name, src) in cases {
		let (pdf, sites) = res!(compile_with_sites(&[("/doc/main.typ", src)]));
		let path = scratch(&fmt!("pdf_info_incode_{}.pdf", name));
		res!(std::fs::write(&path, &pdf));
		assert!(!res!(info_of(&path)).contains_key("Title"), "{}", name);
		let held: Vec<&(usize, usize, String)> = sites.iter().filter(|(_, _, m)| m.contains("#set document")).collect();
		assert_eq!(held.len(), 1, "{}: the rule is refused once: {:?}", name, sites);
		assert!(held[0].2.contains("inside a body, where it is not applied"), "{}: {:?}", name, held[0]);
	}
	Ok(())
}
