//! `sbj` -- the authoring toolchain for the SBJ document format.
//!
//! A document's source is its JDAT text form and its artefact is the signed binary file, and this is
//! what turns one into the other. `compile` reads the text, validates it against the node schema,
//! encodes it canonically, hashes it, signs the hash and writes the file; `import` does the same to
//! a Markdown file, having first mapped the prose to the node vocabulary; `verify` runs the five
//! steps of `SPEC.md` §2 that touch no content and reports what the envelope vouches for; `inspect`
//! reads the document whole and says what is in it; and `dump` writes a document back out as text,
//! so that a document can be read, edited and recompiled by whoever holds it.
//!
//! Run `sbj` with no arguments for the usage.

#![forbid(unsafe_code)]

use oxedyne_fe2o3_sbj::{
	doc,
	envelope::{
		self,
		Envelope,
	},
	import,
	index,
	key::{
		self,
		KeyPair,
	},
	kinds::{
		NodeKind,
		ReservedKind,
		KEY_STYLES,
	},
	text::{
		self,
		KindDecl,
	},
	validate,
	SCHEMA_DOC,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
	collections::BTreeMap,
	fs,
	path::{
		Path,
		PathBuf,
	},
	process,
	time::{
		SystemTime,
		UNIX_EPOCH,
	},
};

/// The key file a compiler signs with when the author names none.
const DEFAULT_KEY: &'static str = "key.jdat";

/// The stack the work is done on.
///
/// The JDAT text decoder is recursive, and a document may legally nest to the depth limit of
/// `SPEC.md` §5. A node costs about five text levels and the limit is 64, so the deepest legal
/// document needs some 320 levels, at about 2.4 KB of stack each in a build with no optimisation:
/// call it 800 KB. Eight mebibytes leaves a tenfold margin.
///
/// The limit is the format's and does not move to suit a tool, so the tool moves. The stack is
/// reserved rather than committed, and a document that never nests deeply never touches it.
const STACK_BYTES: usize = 8 * 1024 * 1024;

/// What the tool was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
	/// Read a document's text form and write the signed artefact.
	Compile,
	/// Read a Markdown file, map it to the node vocabulary, and write the signed artefact.
	Import,
	/// Run steps 1 to 5 of §2, which touch no content, and report what the envelope vouches for.
	Verify,
	/// Read a document whole and report what is in it.
	Inspect,
	/// Write a document back out in its text form.
	Dump,
}

impl Cmd {

	/// The subcommand a word names.
	fn from_word(word: &str) -> Outcome<Self> {
		match word {
			"compile"	=> Ok(Self::Compile),
			"import"	=> Ok(Self::Import),
			"verify"	=> Ok(Self::Verify),
			"inspect"	=> Ok(Self::Inspect),
			"dump"	=> Ok(Self::Dump),
			_	=> Err(err!(
				"'{}' is not an sbj subcommand. The subcommands are compile, import, verify, \
				inspect and dump.", word;
			Invalid, Input)),
		}
	}
}

/// The arguments a run was given.
#[derive(Clone, Debug)]
struct Args {
	/// The subcommand.
	cmd:	Cmd,
	/// The file to read: a `doc.jdat` to compile, or a `doc.sbj` to read.
	input:	PathBuf,
	/// The file to write, if the subcommand writes one.
	output:	Option<PathBuf>,
	/// The key file to sign with.
	keyfile:	PathBuf,
	/// The entry of the key file to sign with, where the file holds several pairs.
	entry:	Option<String>,
	/// The authoring time, in Unix milliseconds. The clock, when the author names none.
	time:	Option<u64>,
	/// Whether to append the optional index of §1.4.
	index:	bool,
	/// The kinds outside the v0 vocabulary the source names by a label of its own (§4.5).
	kinds:	Vec<KindDecl>,
	/// The title an imported document carries. Its first level 1 heading, when the author names none.
	title:	Option<String>,
	/// The language an imported document declares. [`import::DEFAULT_LANG`], when the author names
	/// none.
	lang:	Option<String>,
}

fn main() {
	// The work runs on a thread with a stack that can hold the deepest document the format permits.
	let thread = match std::thread::Builder::new()
		.name("sbj".to_string())
		.stack_size(STACK_BYTES)
		.spawn(run)
	{
		Ok(thread) => thread,
		Err(e) => {
			eprintln!("sbj: could not start: {}", e);
			process::exit(1);
		},
	};
	let outcome = match thread.join() {
		Ok(outcome) => outcome,
		Err(_) => {
			eprintln!("sbj: the working thread did not return.");
			process::exit(1);
		},
	};
	match outcome {
		Ok(()) => process::exit(0),
		Err(e) => {
			eprintln!("sbj: {}", e);
			process::exit(1);
		},
	}
}

/// Reads the arguments and runs the subcommand they name.
fn run() -> Outcome<()> {
	let words: Vec<String> = std::env::args().skip(1).collect();
	if words.is_empty() || words.iter().any(|w| w == "-h" || w == "--help") {
		usage();
		return Ok(());
	}
	let args = res!(parse(&words));
	match args.cmd {
		Cmd::Compile	=> compile(&args),
		Cmd::Import	=> import(&args),
		Cmd::Verify	=> verify(&args),
		Cmd::Inspect	=> inspect(&args),
		Cmd::Dump	=> dump(&args),
	}
}

/// Prints what the tool does and how it is asked to do it.
fn usage() {
	println!("\
sbj -- the authoring toolchain for SBJ, the oxeweb document format.

Usage:
  sbj compile <doc.jdat> -o <doc.sbj> [options]   Validate, canonicalise, hash, sign, write.
  sbj import  <doc.md|.html> [-o <doc.sbj>]        Map Markdown or HTML to a document, then compile it.
  sbj verify  <doc.sbj>                           Check the envelope without decoding the tree.
  sbj inspect <doc.sbj>                           Read the document whole and say what is in it.
  sbj dump    <doc.sbj> [-o <doc.jdat>]           Write the document back out as text.

Options:
  -o, --out <path>        Where to write. Compile requires it; import writes beside its source;
                          dump writes to stdout without it.
  -k, --key <path>        The key file to sign with, generated and saved if absent [{key}].
      --key-entry <name>  The entry of the key file to sign with, where it holds several pairs.
  -t, --time <ms>         The authoring time, in Unix milliseconds [the clock].
      --index             Append the optional offset index of SPEC.md 1.4.
      --kind <label>=<n>  Declare a kind outside the v0 vocabulary, e.g. --kind sbj_alien=99.
                          A kind written (sbj_k<n>|{{..}}) needs no declaring.
      --title <text>      Import: the document's title [its first level 1 heading, or the file name].
      --lang <tag>        Import: the document's language, BCP-47 [{lang}].
  -h, --help              This.

An import maps the prose and drops what the v0 vocabulary has no room for: a thematic break goes,
an image becomes its alt text, since v0 addresses an image by content hash and Markdown gives a
path, and an inline code span becomes its characters, since code is flow content and cannot sit in
a paragraph.

A document is written in JDAT text form, where a node is its kind label and its payload:

  (sbj_doc|(map|{{
      (str|\"title\"):    (str|\"Style without a cascade\"),
      (str|\"lang\"):     (str|\"en\"),
      (str|\"children\"): (list|[
          (sbj_para|(map|{{
              (str|\"children\"): (list|[(sbj_text|(str|\"A paragraph.\"))]),
          }})),
      ]),
  }}))
", key = DEFAULT_KEY, lang = import::DEFAULT_LANG);
}

/// Reads the arguments, refusing one that is not an argument and one that is missing its value.
fn parse(words: &[String]) -> Outcome<Args> {

	let cmd = res!(Cmd::from_word(&words[0]));

	let mut input:	Option<PathBuf> = None;
	let mut output:	Option<PathBuf> = None;
	let mut keyfile:	PathBuf = PathBuf::from(DEFAULT_KEY);
	let mut entry:	Option<String> = None;
	let mut time:	Option<u64> = None;
	let mut index:	bool = false;
	let mut kinds:	Vec<KindDecl> = Vec::new();
	let mut title:	Option<String> = None;
	let mut lang:	Option<String> = None;

	let mut i = 1;
	while i < words.len() {
		let word = words[i].as_str();
		match word {
			"-o" | "--out" => {
				output = Some(PathBuf::from(res!(value(words, &mut i, word))));
			},
			"-k" | "--key" => {
				keyfile = PathBuf::from(res!(value(words, &mut i, word)));
			},
			"--key-entry" => {
				entry = Some(res!(value(words, &mut i, word)));
			},
			"-t" | "--time" => {
				let v = res!(value(words, &mut i, word));
				time = Some(match v.parse::<u64>() {
					Ok(ms) => ms,
					Err(_) => return Err(err!(
						"The time '{}' is not a number of Unix milliseconds.", v;
					Invalid, Input)),
				});
			},
			"--index" => index = true,
			"--kind" => {
				kinds.push(res!(kind_decl(&res!(value(words, &mut i, word)))));
			},
			"--title" => {
				title = Some(res!(value(words, &mut i, word)));
			},
			"--lang" => {
				lang = Some(res!(value(words, &mut i, word)));
			},
			other if other.starts_with('-') => return Err(err!(
				"'{}' is not an sbj option. Run `sbj --help` for the ones there are.", other;
			Invalid, Input, Unknown)),
			other => {
				if input.is_some() {
					return Err(err!(
						"sbj {} reads one file, and was given both '{}' and '{}'.",
						words[0], input_name(&input), other;
					Invalid, Input, Excessive));
				}
				input = Some(PathBuf::from(other));
			},
		}
		i += 1;
	}

	let input = match input {
		Some(input) => input,
		None => return Err(err!(
			"sbj {} needs a file to read.", words[0];
		Invalid, Input, Missing)),
	};
	if cmd == Cmd::Compile && output.is_none() {
		return Err(err!(
			"sbj compile needs somewhere to write the document: `sbj compile {} -o <doc.sbj>`.",
			input.display();
		Invalid, Input, Missing));
	}

	Ok(Args {
		cmd,
		input,
		output,
		keyfile,
		entry,
		time,
		index,
		kinds,
		title,
		lang,
	})
}

/// The value of an option, or an error naming the option that was given none.
fn value(
	words:	&[String],
	i:	&mut usize,
	opt:	&str,
)
	-> Outcome<String>
{
	*i += 1;
	match words.get(*i) {
		Some(v) => Ok(v.clone()),
		None => Err(err!(
			"The option '{}' takes a value, and was given none.", opt;
		Invalid, Input, Missing)),
	}
}

/// Reads a `--kind <label>=<code>` declaration.
fn kind_decl(s: &str) -> Outcome<KindDecl> {
	let (label, code) = match s.split_once('=') {
		Some((label, code)) => (label.trim(), code.trim()),
		None => return Err(err!(
			"The kind declaration '{}' is not of the form <label>=<code>, e.g. sbj_alien=99.", s;
		Invalid, Input)),
	};
	if label.is_empty() {
		return Err(err!(
			"The kind declaration '{}' names no label.", s;
		Invalid, Input, Missing));
	}
	let code = match code.parse::<u16>() {
		Ok(code) => code,
		Err(_) => return Err(err!(
			"The kind declaration '{}' gives the code '{}', which is not a u16. A node kind code \
			runs from 0 to {}.", s, code, u16::MAX;
		Invalid, Input)),
	};
	Ok(KindDecl {
		label:	label.to_string(),
		code,
	})
}

/// The name of the input file, for an error message raised before it is known there is one.
fn input_name(input: &Option<PathBuf>) -> String {
	match input {
		Some(path) => fmt!("{}", path.display()),
		None => fmt!("nothing"),
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COMPILE AND IMPORT                                                         │
// └───────────────────────────────────────────────────────────────────────────┘

/// Compiles a document: read the text, validate, canonically encode, hash, sign, write.
fn compile(args: &Args) -> Outcome<()> {
	let src = res!(read_text(&args.input));
	let tree = match text::decode(&src, &args.kinds) {
		Ok(tree) => tree,
		Err(e) => return Err(err!(e,
			"{} is not a readable document.", args.input.display();
		Invalid, Input)),
	};
	let out = match &args.output {
		Some(out) => out.clone(),
		None => return Err(err!(
			"sbj compile needs somewhere to write the document."; Bug, Missing)),
	};
	sign_and_write(args, &tree, &out, "Compiled")
}

/// Imports a Markdown file: read the prose, map it to the node vocabulary, and compile the tree.
///
/// Everything after the mapping is the compile path exactly, because by then it is a document like
/// any other: an imported tree is held to the same schema, canonicalised the same way, and signed by
/// the same hand as one written by an author who typed the JDAT out. The mapping, and the three
/// things Markdown says that v0 has no room for, are [`import`](oxedyne_fe2o3_sbj::import)'s business.
fn import(args: &Args) -> Outcome<()> {
	let src = res!(read_text(&args.input));
	let opts = import::Options {
		title:	args.title.clone(),
		lang:	match &args.lang {
			Some(lang) => lang.clone(),
			None => import::DEFAULT_LANG.to_string(),
		},
		stem:	stem(&args.input),
	};
	// The form is read from the name, because the two forms are told apart by nothing else: HTML and
	// Markdown are both text, and a file that is one is legal input to the reader for the other.
	let form = Form::of(&args.input);
	let tree = match form.read(&src, &opts) {
		Ok(tree) => tree,
		Err(e) => return Err(err!(e,
			"{} is not readable {}.", args.input.display(), form.label();
		Invalid, Input)),
	};
	// An import writes beside its source unless it is told otherwise, since an author who has a
	// document to import has nowhere in mind to put it yet.
	let out = match &args.output {
		Some(out) => out.clone(),
		None => args.input.with_extension("sbj"),
	};
	sign_and_write(args, &tree, &out, "Imported")
}

/// The form an imported source is written in.
///
/// Both forms reach the same tree and the same mapping; they differ only in the reader that gets
/// them there. HTML earns its place by being what everything else exports: prose written in a form
/// no reader here understands is often reachable through the tool that does understand it, with the
/// author's own macros already resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
	/// Markdown, the form most existing prose is written in.
	Markdown,
	/// HTML, the form most other things export.
	Html,
}

impl Form {

	/// The form a path names, judged by its extension. Anything not plainly HTML is read as Markdown,
	/// which is the form an author is likelier to have and the likelier thing to mean.
	fn of(path: &Path) -> Self {
		let ext = match path.extension() {
			Some(ext) => ext.to_string_lossy().to_lowercase(),
			None => return Self::Markdown,
		};
		match ext.as_str() {
			"html" | "htm"	=> Self::Html,
			_		=> Self::Markdown,
		}
	}

	/// Reads a source of this form into a document tree.
	fn read(&self, src: &str, opts: &import::Options) -> Outcome<Dat> {
		match self {
			Self::Markdown	=> import::from_markdown(src, opts),
			Self::Html	=> import::from_html(src, opts),
		}
	}

	/// What this form is called, for saying which reader refused a file.
	fn label(&self) -> &'static str {
		match self {
			Self::Markdown	=> "Markdown",
			Self::Html	=> "HTML",
		}
	}
}

/// The name a source file is known by, which is an imported document's title of last resort.
fn stem(path: &Path) -> String {
	match path.file_stem() {
		Some(stem) => stem.to_string_lossy().to_string(),
		None => import::DEFAULT_STEM.to_string(),
	}
}

/// Signs a tree and writes the document: the path a compile and an import share.
///
/// Nothing this crate would refuse to read is ever given a signature and an address, so a document
/// that fails its schema is refused here rather than published and refused by every reader of it.
/// It is one path deliberately: a tree that came from Markdown is a document by the time it arrives,
/// and a second signing path for it would be a second place for the schema check to go missing.
fn sign_and_write(
	args:	&Args,
	tree:	&Dat,
	out:	&Path,
	verb:	&str,
)
	-> Outcome<()>
{
	let (pair, made) = res!(signing_key(&args.keyfile, args.entry.as_deref()));
	let signer = res!(pair.signer());
	let time = match args.time {
		Some(time) => time,
		None => res!(now()),
	};

	let buf = if args.index {
		res!(doc::write_with_index(tree, SCHEMA_DOC, &signer, time))
	} else {
		res!(doc::write(tree, SCHEMA_DOC, &signer, time))
	};

	// It must read back the way a reader would, or it is not a document, it is a file.
	let read = res!(doc::read(&buf));
	let stats = res!(validate::validate(read.tree(), &read.env().schema));

	res!(write_bytes(out, &buf));

	if made {
		println!("Generated a signing key and saved it to {}.", args.keyfile.display());
	}
	println!("{} {}", verb, args.input.display());
	res!(report_envelope(read.env()));
	println!("  nodes        {}, depth {}", stats.nodes, stats.depth);
	println!("  wrote        {} ({} bytes{})",
		out.display(),
		buf.len(),
		if args.index { ", index appended" } else { "" },
	);
	Ok(())
}

/// The key a document is signed with, generated and saved if the key file is not there yet.
///
/// The second return says whether the key was made here, since an author who did not know they had
/// no key should be told where the one they now have was put.
fn signing_key(
	path:	&Path,
	entry:	Option<&str>,
)
	-> Outcome<(KeyPair, bool)>
{
	if path.is_file() {
		return Ok((res!(key::load(path, entry)), false));
	}
	if entry.is_some() {
		return Err(err!(
			"There is no key file at {}, and an entry of it was named. A key file that holds \
			several pairs is not one this tool would have written, so it is not generated here.",
			path.display();
		Invalid, Input, Missing));
	}
	let pair = res!(KeyPair::generate());
	res!(key::save(&pair, path));
	Ok((pair, true))
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ VERIFY                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Verifies a document without decoding its tree: steps 1 to 5 of §2, which touch no content.
///
/// A document that fails renders as an error card and is never partially displayed, so this says so
/// and stops, and the shell hears about it.
fn verify(args: &Args) -> Outcome<()> {
	let buf = res!(read_bytes(&args.input));
	let env = match doc::verify_only(&buf) {
		Ok(env) => env,
		Err(e) => return Err(err!(e,
			"{} is not a document that verifies.", args.input.display();
		Invalid, Input)),
	};
	println!("Verified {} ({} bytes)", args.input.display(), buf.len());
	res!(report_envelope(&env));
	println!("  signature    good, over the signing input of SPEC.md §1.3");
	println!("  tree         {} bytes, not decoded", env.tree_len);
	Ok(())
}

/// Reports what an envelope vouches for: the schema, the author, the schemes, the time, the address.
fn report_envelope(env: &Envelope) -> Outcome<()> {
	println!("  schema       {}", env.schema);
	println!("  author       {}", hex(&env.author));
	println!("  sig scheme   {:#010X} ({})", env.sig_scheme, res!(sig_scheme_name(env.sig_scheme)));
	println!("  hash scheme  {:#010X} ({})", env.hash_scheme,
		res!(hash_scheme_name(env.hash_scheme)));
	println!("  time         {} (Unix ms)", env.time);
	println!("  address      {}", hex(&env.hash));
	Ok(())
}

/// The name of a signature scheme id, refusing one this version does not implement.
fn sig_scheme_name(id: u32) -> Outcome<&'static str> {
	match id {
		envelope::SIG_SCHEME_ED25519 => Ok("ed25519"),
		_ => Err(err!(
			"The envelope names the signature scheme {:#010X}, which this version does not \
			implement. v0 signs with Ed25519, whose scheme id is {:#010X}.",
			id, envelope::SIG_SCHEME_ED25519;
		Invalid, Input, Unimplemented)),
	}
}

/// The name of a hash scheme id, refusing one this version does not implement.
fn hash_scheme_name(id: u32) -> Outcome<&'static str> {
	match id {
		envelope::HASH_SCHEME_SHA3_256 => Ok("sha3-256"),
		_ => Err(err!(
			"The envelope names the hash scheme {:#010X}, which this version does not implement. \
			v0 hashes with SHA3-256, whose scheme id is {:#010X}.",
			id, envelope::HASH_SCHEME_SHA3_256;
		Invalid, Input, Unimplemented)),
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ INSPECT                                                                    │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads a document whole and reports what is in it: its shape, its vocabulary, its styles.
fn inspect(args: &Args) -> Outcome<()> {
	let buf = res!(read_bytes(&args.input));
	let read = match doc::read(&buf) {
		Ok(read) => read,
		Err(e) => return Err(err!(e,
			"{} is not a document that reads.", args.input.display();
		Invalid, Input)),
	};
	let stats = res!(validate::validate(read.tree(), &read.env().schema));

	println!("Inspected {} ({} bytes)", args.input.display(), buf.len());
	res!(report_envelope(read.env()));
	println!("  tree         {} bytes", read.env().tree_len);
	println!("  nodes        {}, depth {}", stats.nodes, stats.depth);

	let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
	count_kinds(read.tree(), &mut counts);
	println!("  kinds        {}", res!(kind_list(&counts)));

	println!("  styles       {}", res!(style_list(read.tree())));

	// The index of §1.4 is derived data lying outside the hash, and is never trusted: what it says is
	// checked against the tree it claims to describe before it is believed.
	let rest = res!(doc::index_region(&buf));
	if rest.is_empty() {
		println!("  index        none");
	} else {
		let idx = res!(index::parse(rest));
		let (_, region) = res!(doc::verify(&buf));
		match index::check(region, &idx) {
			Ok(()) => println!("  index        {} entries, {} bytes, checked against the tree",
				idx.len(), rest.len()),
			Err(e) => return Err(err!(e,
				"The index of {} does not describe the tree it trails.", args.input.display();
			Invalid, Input)),
		}
	}
	Ok(())
}

/// Counts the nodes of each kind, descending into children and into the fallback of §4.5.
fn count_kinds(
	node:	&Dat,
	counts:	&mut BTreeMap<u16, usize>,
) {
	let (uid, payload) = match node {
		Dat::Usr(uid, Some(payload)) => (uid, payload.as_ref()),
		_ => return,
	};
	*counts.entry(uid.code()).or_insert(0) += 1;
	let map = match payload {
		Dat::Map(map) => map,
		_ => return,
	};
	for (_, v) in map {
		if let Dat::List(list) = v {
			for kid in list {
				count_kinds(kid, counts);
			}
		}
	}
}

/// The kinds a document uses, and how often, for the report.
fn kind_list(counts: &BTreeMap<u16, usize>) -> Outcome<String> {
	if counts.is_empty() {
		return Ok(fmt!("none"));
	}
	let mut s = String::new();
	for (code, n) in counts {
		if !s.is_empty() {
			s.push_str(", ");
		}
		let name = match NodeKind::from_code(*code) {
			Ok(kind) => kind.label().to_string(),
			Err(_) => match ReservedKind::from_code(*code) {
				// A kind reserved to the chrome and to applications (§4.2). A document carrying one
				// never reads, so this names a tree that came from somewhere other than a document.
				Some(reserved) => fmt!("reserved kind {}", reserved.label()),
				// A kind the vocabulary does not know, which §4.5 admits because it carries a
				// fallback.
				None => fmt!("unknown kind {}", code),
			},
		};
		s.push_str(&fmt!("{} {}", name, n));
	}
	Ok(s)
}

/// The document's style table (§4.4), as the report spells it.
fn style_list(tree: &Dat) -> Outcome<String> {
	let map = match tree {
		Dat::Usr(_, Some(payload)) => match payload.as_ref() {
			Dat::Map(map) => map,
			_ => return Ok(fmt!("none")),
		},
		_ => return Ok(fmt!("none")),
	};
	let table = match map.get(&dat!(KEY_STYLES)) {
		Some(Dat::Map(table)) => table,
		_ => return Ok(fmt!("none")),
	};
	let mut s = String::new();
	for (name, record) in table {
		if !s.is_empty() {
			s.push_str("\n               ");
		}
		let name = match name {
			Dat::Str(name) => name.clone(),
			d => return Err(err!(
				"A style name is a str, found a {:?}.", d.kind();
			Invalid, Input)),
		};
		s.push_str(&fmt!("{}: {}", name, res!(style_record(record))));
	}
	Ok(s)
}

/// One style record, as the report spells it.
fn style_record(record: &Dat) -> Outcome<String> {
	let map = match record {
		Dat::Map(map) => map,
		d => return Err(err!(
			"A style record is a map, found a {:?}.", d.kind();
		Invalid, Input)),
	};
	let mut s = String::new();
	for (prop, v) in map {
		if !s.is_empty() {
			s.push_str(", ");
		}
		let prop = match prop {
			Dat::Str(prop) => prop.clone(),
			d => return Err(err!(
				"A style property is a str, found a {:?}.", d.kind();
			Invalid, Input)),
		};
		s.push_str(&fmt!("{} = {}", prop, style_value(v)));
	}
	Ok(s)
}

/// One style value, as the report spells it.
fn style_value(v: &Dat) -> String {
	match v {
		Dat::Str(s)	=> s.clone(),
		Dat::U8(n)	=> fmt!("{}", n),
		Dat::I8(n)	=> fmt!("{}", n),
		d	=> fmt!("a {:?}", d.kind()),
	}
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ DUMP                                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Writes a document back out in its text form, so that it can be read, edited and recompiled.
///
/// The document is verified and decoded first, so what is written is a document rather than whatever
/// the bytes happened to hold, and what comes out compiles back to the bytes that went in.
fn dump(args: &Args) -> Outcome<()> {
	let buf = res!(read_bytes(&args.input));
	let read = match doc::read(&buf) {
		Ok(read) => read,
		Err(e) => return Err(err!(e,
			"{} is not a document that reads.", args.input.display();
		Invalid, Input)),
	};
	let src = res!(text::encode(read.tree()));
	match &args.output {
		Some(out) => {
			res!(write_bytes(out, src.as_bytes()));
			println!("Wrote {} ({} bytes).", out.display(), src.len());
		},
		None => print!("{}", src),
	}
	Ok(())
}

// ┌───────────────────────────────────────────────────────────────────────────┐
// │ FILES, TIME, BYTES                                                         │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads a file whole, naming it if it will not open.
fn read_bytes(path: &Path) -> Outcome<Vec<u8>> {
	match fs::read(path) {
		Ok(byts) => Ok(byts),
		Err(e) => Err(err!(e,
			"Could not read {}.", path.display();
		IO, File)),
	}
}

/// Reads a text file whole, naming it if it will not open.
fn read_text(path: &Path) -> Outcome<String> {
	match fs::read_to_string(path) {
		Ok(s) => Ok(s),
		Err(e) => Err(err!(e,
			"Could not read {}.", path.display();
		IO, File)),
	}
}

/// Writes a file whole, making the directory it goes in.
fn write_bytes(
	path:	&Path,
	byts:	&[u8],
)
	-> Outcome<()>
{
	if let Some(dir) = path.parent() {
		if !dir.as_os_str().is_empty() {
			match fs::create_dir_all(dir) {
				Ok(()) => (),
				Err(e) => return Err(err!(e,
					"Could not make the directory {}.", dir.display();
				IO, File)),
			}
		}
	}
	match fs::write(path, byts) {
		Ok(()) => Ok(()),
		Err(e) => Err(err!(e,
			"Could not write {}.", path.display();
		IO, File)),
	}
}

/// The time now, in Unix milliseconds, which is what an envelope carries.
fn now() -> Outcome<u64> {
	let since = match SystemTime::now().duration_since(UNIX_EPOCH) {
		Ok(since) => since,
		Err(e) => return Err(err!(e,
			"The clock reads a time before the Unix epoch, which an envelope cannot carry.";
		Invalid, Input)),
	};
	Ok(try_into!(u64, since.as_millis()))
}

/// Renders bytes as hexadecimal, for a report that must name what it read.
fn hex(byts: &[u8]) -> String {
	let mut s = String::new();
	for b in byts {
		s.push_str(&fmt!("{:02x}", b));
	}
	s
}
