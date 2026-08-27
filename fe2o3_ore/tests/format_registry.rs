//! The format registry, checked against the code it describes.
//!
//! `format.jdat` beside this crate's manifest holds the version constants, the
//! wire codes, the kind bytes and the golden byte tests as data. Everything here
//! reads it and asks the tree whether it is still true.
//!
//! It exists because the prose contract it replaces went stale without anybody
//! noticing. Its table named `snapshot::VERSION` at a line in a file that had
//! been deleted four days earlier, and listed a golden test that had gone with
//! it; both were found by a person reading carefully, and only because a person
//! was asked to look. A registry a test reads cannot go stale quietly, which is
//! the whole of the argument for moving these facts out of the document.

use oxedyne_fe2o3_ore::{
	op,
	segment,
	sync::msg,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::{
	collections::BTreeSet,
	fs,
	path::{
		Path,
		PathBuf,
	},
};


// The registry, as it is written.

#[derive(Clone, Debug, Default, FromDatMap)]
struct Registry {
	about:		String,
	constants:	Vec<Dat>,
	files:		Vec<Dat>,
	goldens:	Vec<Dat>,
	open:		Vec<Dat>,
	removed:	Vec<Dat>,
	settled:	Vec<Dat>,
	versions:	Vec<Dat>,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct Constant {
	axis:	String,
	file:	String,
	holds:	String,
	line:	u64,
	name:	String,
	value:	Dat,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct FileRow {
	exempt:	Vec<String>,
	path:	String,
	role:	String,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct VersionRow {
	highest_code:	String,
	note:			String,
	version:		u64,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct Golden {
	file:	String,
	holds:	String,
	magic:	String,
	name:	String,
	pins:	Vec<Dat>,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct Pin {
	at:	u64,
	is:	String,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct Removed {
	commit:	String,
	file:	String,
	gone:	String,
	name:	String,
	why:	String,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct Settled {
	answer:		String,
	answered:	String,
	names:		Vec<String>,
	note:		String,
	question:	String,
	refused:	Vec<String>,
}

#[derive(Clone, Debug, Default, FromDatMap)]
struct Open {
	asked:		String,
	guard:		Vec<String>,
	note:		String,
	question:	String,
	why:		String,
}

// The sections the registry is expected to carry. An unknown key would otherwise
// be read past in silence, so a section renamed by a typo would take its whole
// contents out of the checking with nothing to show for it.
const SECTIONS: usize = 8;

// The axes a name may sit on. Which axis a name belongs to decides whether a
// version moves for it, so a new one is a design decision and belongs in a
// commit rather than in a string nobody read.
const AXES: [&str; 5] = ["container", "convention", "field", "frame", "vocabulary"];


/// The value this crate compiles for a name the registry declares.
///
/// A registry entry with no arm here fails, so a constant cannot be written into
/// the registry without the compiler agreeing that it exists and is public. The
/// arms name the constants and never their values, so this is a bridge and not a
/// second copy of the registry.
fn compiled(name: &str) -> Option<Dat> {
	Some(match name {
		"segment::MAGIC"			=> magic(&segment::MAGIC),
		"segment::VERSION"			=> Dat::U8(segment::VERSION),
		"segment::VERSION_MIN"		=> Dat::U8(segment::VERSION_MIN),
		"segment::KIND_BARE"		=> Dat::U8(segment::KIND_BARE),
		"segment::KIND_SEALED"		=> Dat::U8(segment::KIND_SEALED),
		"segment::KIND_VEILED"		=> Dat::U8(segment::KIND_VEILED),
		"segment::KIND_PACKED"		=> Dat::U8(segment::KIND_PACKED),
		"segment::PACKED_MAX"		=> Dat::U64(segment::PACKED_MAX as u64),
		"op::CODE_FILE_CREATE"		=> Dat::U8(op::CODE_FILE_CREATE),
		"op::CODE_FILE_DELETE"		=> Dat::U8(op::CODE_FILE_DELETE),
		"op::CODE_FILE_RENAME"		=> Dat::U8(op::CODE_FILE_RENAME),
		"op::CODE_MARK"				=> Dat::U8(op::CODE_MARK),
		"op::CODE_SPLICE"			=> Dat::U8(op::CODE_SPLICE),
		"op::CODE_MOVE"				=> Dat::U8(op::CODE_MOVE),
		"op::CODE_NOTE"				=> Dat::U8(op::CODE_NOTE),
		"op::CODE_FILE_MODE"		=> Dat::U8(op::CODE_FILE_MODE),
		"op::CODE_MARK_TIMED"		=> Dat::U8(op::CODE_MARK_TIMED),
		"op::CODE_PROPOSAL"			=> Dat::U8(op::CODE_PROPOSAL),
		"op::CODE_SAID"				=> Dat::U8(op::CODE_SAID),
		"op::CODE_SETTLED"			=> Dat::U8(op::CODE_SETTLED),
		"op::CODE_REVERTS"			=> Dat::U8(op::CODE_REVERTS),
		"op::CODE_AMENDED"			=> Dat::U8(op::CODE_AMENDED),
		"op::MODE_NORMAL"			=> Dat::U8(op::MODE_NORMAL),
		"op::MODE_EXECUTABLE"		=> Dat::U8(op::MODE_EXECUTABLE),
		"op::MODE_SYMLINK"			=> Dat::U8(op::MODE_SYMLINK),
		"op::SETTLED_OPEN"			=> Dat::U8(op::SETTLED_OPEN),
		"op::SETTLED_ACCEPTED"		=> Dat::U8(op::SETTLED_ACCEPTED),
		"op::SETTLED_DECLINED"		=> Dat::U8(op::SETTLED_DECLINED),
		"op::SETTLED_DONE"			=> Dat::U8(op::SETTLED_DONE),
		"op::AUTO_MARK_PREFIX"		=> Dat::Str(op::AUTO_MARK_PREFIX.to_string()),
		"op::AUTHOR_TRAILER"		=> Dat::Str(op::AUTHOR_TRAILER.to_string()),
		"sync::msg::MAGIC"			=> magic(&msg::MAGIC),
		"sync::msg::VERSION"		=> Dat::U8(msg::VERSION),
		"sync::msg::VERSION_MIN"	=> Dat::U8(msg::VERSION_MIN),
		"sync::msg::KIND_HELLO"		=> Dat::U8(msg::KIND_HELLO),
		"sync::msg::KIND_SKETCH"	=> Dat::U8(msg::KIND_SKETCH),
		"sync::msg::KIND_SEND"		=> Dat::U8(msg::KIND_SEND),
		"sync::msg::KIND_DONE"		=> Dat::U8(msg::KIND_DONE),
		"sync::msg::KIND_PART"		=> Dat::U8(msg::KIND_PART),
		"sync::msg::PART_MAX"		=> Dat::U64(msg::PART_MAX as u64),
		_ => return None,
	})
}

fn magic(bytes: &[u8]) -> Dat {
	Dat::Str(String::from_utf8_lossy(bytes).into_owned())
}


// Reading the registry, and reading the tree.

fn root() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn slurp(path: &Path) -> Outcome<String> {
	match fs::read_to_string(path) {
		Ok(s) => Ok(s),
		Err(e) => Err(err!(e,
			"While reading {} for the format registry.", path.display();
			IO, File, Read)),
	}
}

fn registry() -> Outcome<Registry> {
	let path = root().join("format.jdat");
	let dat = res!(Dat::decode_string(res!(slurp(&path)))).normalise();
	let map = match dat {
		Dat::Map(m) => m,
		other => return Err(err!(
			"The format registry at {} decodes to a {:?}, not a map.",
			path.display(), other.kind(); Input, Invalid)),
	};
	if map.len() != SECTIONS {
		let keys: Vec<String> = map.keys().map(|k| fmt!("{:?}", k)).collect();
		return Err(err!(
			"The format registry carries {} top level keys and this test knows {}. \
			A section this test does not read is a section nothing checks. Found: {}.",
			map.len(), SECTIONS, keys.join(", "); Input, Invalid));
	}
	Registry::from_datmap(map)
}

fn rows<T: FromDatMap>(list: &[Dat], section: &str) -> Outcome<Vec<T>> {
	let mut out = Vec::new();
	for (i, d) in list.iter().enumerate() {
		match d {
			Dat::Map(m) => out.push(res!(T::from_datmap(m.clone()))),
			other => return Err(err!(
				"Entry {} of the registry's '{}' section is a {:?}, not a map.",
				i, section, other.kind(); Input, Invalid)),
		}
	}
	Ok(out)
}

fn constants() -> Outcome<Vec<Constant>> {
	let reg = res!(registry());
	rows(&reg.constants, "constants")
}

fn code_of(line: &str) -> &str {
	match line.find("//") {
		Some(i) => &line[..i],
		None => line,
	}
}

/// The name of the screaming case constant this line declares, where it declares
/// one. `pub const fn` is not one, which is why the name is required to be
/// screaming case rather than merely to follow the keyword.
fn declared_const(line: &str) -> Option<&str> {
	let code = code_of(line).trim_start();
	let rest = match code.strip_prefix("pub const ") {
		Some(r) => r,
		None => return None,
	};
	let end = rest
		.find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
		.unwrap_or(rest.len());
	if end == 0 {
		return None;
	}
	let name = &rest[..end];
	match rest[end..].trim_start().starts_with(':') {
		true	=> Some(name),
		false	=> None,
	}
}

/// The last component of a registry name, which is what the source declares.
fn short(name: &str) -> &str {
	match name.rfind("::") {
		Some(i) => &name[i + 2..],
		None => name,
	}
}

/// Every `.rs` file under `src`, so that a check over the tree cannot be fooled
/// by a name moving to a module the check did not know about.
fn sources() -> Outcome<Vec<PathBuf>> {
	let mut out = Vec::new();
	let mut stack = vec![root().join("src")];
	while let Some(dir) = stack.pop() {
		let entries = match fs::read_dir(&dir) {
			Ok(e) => e,
			Err(e) => return Err(err!(e,
				"While walking {} for the format registry.", dir.display();
				IO, File, Read)),
		};
		for entry in entries {
			let entry = res!(entry);
			let path = entry.path();
			if path.is_dir() {
				stack.push(path);
			} else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
				out.push(path);
			}
		}
	}
	out.sort();
	Ok(out)
}

/// The bytes a golden test freezes, read out of its source as hexadecimal
/// tokens. The arrays these tests carry are written one byte to a token with the
/// prose beside them in comments, so the tokens of the function body in order are
/// the frozen bytes in order.
fn frozen_bytes(src: &str, test: &str) -> Outcome<Vec<u8>> {
	let want = fmt!("fn {}(", test);
	let lines: Vec<&str> = src.lines().collect();
	let start = match lines.iter().position(|l| l.contains(&want)) {
		Some(i) => i,
		None => return Err(err!(
			"The golden test '{}' is not in the source it was looked for in.",
			test; Missing)),
	};
	let indent: String = lines[start]
		.chars()
		.take_while(|c| c.is_whitespace())
		.collect();
	let close = fmt!("{}}}", indent);
	let mut out = Vec::new();
	for line in &lines[start + 1..] {
		if *line == close {
			return Ok(out);
		}
		let code = code_of(line);
		let mut rest = code;
		while let Some(i) = rest.find("0x") {
			let tail = &rest[i + 2..];
			let end = tail
				.find(|c: char| !c.is_ascii_hexdigit())
				.unwrap_or(tail.len());
			if end == 2 {
				match u8::from_str_radix(&tail[..2], 16) {
					Ok(b) => out.push(b),
					Err(e) => return Err(err!(e,
						"While reading a frozen byte of '{}'.", test; Input, Invalid)),
				}
			}
			rest = &tail[end..];
		}
	}
	Err(err!(
		"The golden test '{}' has no closing brace at its own indent, so its \
		frozen bytes could not be read.", test; Input, Invalid))
}

fn byte_of(name: &str) -> Outcome<u8> {
	match compiled(name) {
		Some(Dat::U8(v)) => Ok(v),
		Some(other) => Err(err!(
			"The registry pins a byte to '{}', which is a {:?} and not a byte.",
			name, other.kind(); Input, Mismatch)),
		None => Err(err!(
			"The registry names '{}', which this crate does not declare.",
			name; Missing)),
	}
}


// What the registry claims, and whether it is still true.

/// Every constant is declared in the file and at the line the registry gives.
///
/// This is the check that the deleted `snapshot.rs` would have tripped: the file
/// named is opened, and a missing one is the failure rather than a silent pass.
#[test]
fn every_constant_is_where_the_registry_says_it_is() -> Outcome<()> {
	for c in res!(constants()) {
		let path = root().join(&c.file);
		if !path.exists() {
			return Err(err!(
				"The registry says {} is declared in {}, and there is no such file. \
				Either the constant moved and the registry did not, or the format it \
				belongs to was taken out and its row belongs under 'removed'.",
				c.name, c.file; Missing, File));
		}
		let src = res!(slurp(&path));
		let mut found = None;
		for (i, line) in src.lines().enumerate() {
			if declared_const(line) == Some(short(&c.name)) {
				found = Some(i + 1);
				break;
			}
		}
		match found {
			Some(line) if line as u64 == c.line => {},
			Some(line) => return Err(err!(
				"The registry says {} is declared at {}:{}, and it is at line {}.",
				c.name, c.file, c.line, line; Mismatch)),
			None => return Err(err!(
				"The registry says {} is declared in {}, and that file declares no \
				public constant of that name.", c.name, c.file; Missing)),
		}
	}
	Ok(())
}

/// The axis check is here rather than on its own because an axis nobody knows is
/// a value nobody checked, which is the same failure as a value that has moved.
#[test]
fn every_constant_has_the_value_the_registry_declares() -> Outcome<()> {
	for c in res!(constants()) {
		let got = match compiled(&c.name) {
			Some(v) => v,
			None => return Err(err!(
				"The registry declares {} and this test has no arm for it, so its \
				value is a claim nothing checks. Add the arm, or the constant is not \
				one this crate declares.", c.name; Missing)),
		};
		if got != c.value {
			return Err(err!(
				"The registry declares {} as {:?} and this crate compiles {:?}. \
				A reader elsewhere is holding the registry's number.",
				c.name, c.value, got; Mismatch));
		}
		if !AXES.contains(&c.axis.as_str()) {
			return Err(err!(
				"The registry puts {} on the axis '{}', which this test does not \
				know. The axis a name sits on decides whether a version moves for \
				it, so it is a decision and not a label. Known: {}.",
				c.name, c.axis, AXES.join(", "); Input, Invalid));
		}
	}
	Ok(())
}

/// Every public constant in a format bearing file is in the registry.
///
/// Without this the registry would only ever be as complete as whoever last
/// remembered it, which is the failure it was written to end: a shared name
/// nobody registered is a name each lane satisfies in isolation.
#[test]
fn every_public_constant_in_a_format_file_is_registered() -> Outcome<()> {
	let reg = res!(registry());
	let files: Vec<FileRow> = res!(rows(&reg.files, "files"));
	let known: BTreeSet<String> = res!(rows::<Constant>(&reg.constants, "constants"))
		.iter()
		.map(|c| fmt!("{}::{}", c.file, short(&c.name)))
		.collect();
	for f in files {
		let path = root().join(&f.path);
		let src = res!(slurp(&path));
		for line in src.lines() {
			if let Some(name) = declared_const(line) {
				if f.exempt.iter().any(|e| e == name) {
					continue;
				}
				let key = fmt!("{}::{}", f.path, name);
				if !known.contains(&key) {
					return Err(err!(
						"{} declares the public constant {}, which is not in the \
						registry. {} carries {}, so a name in it is a name another \
						reader has to agree on. Register it, or exempt it and say why.",
						f.path, name, f.path, f.role; Missing));
				}
			}
		}
	}
	Ok(())
}

/// The registry names every golden byte test there is, and no others.
///
/// Both halves matter. The contract this replaces said each format had one
/// golden test when there were four, and went on listing a fifth that had been
/// deleted with the format it froze.
#[test]
fn the_registry_names_every_golden_test_and_no_others() -> Outcome<()> {
	let reg = res!(registry());
	let goldens: Vec<Golden> = res!(rows(&reg.goldens, "goldens"));
	let claimed: BTreeSet<String> = goldens.iter().map(|g| g.name.clone()).collect();

	let mut present = BTreeSet::new();
	for path in res!(sources()) {
		let src = res!(slurp(&path));
		for line in src.lines() {
			let code = code_of(line).trim_start();
			if let Some(rest) = code.strip_prefix("fn ") {
				let end = rest.find('(').unwrap_or(0);
				let name = &rest[..end];
				if name.ends_with("_bytes_are_frozen") {
					present.insert(name.to_string());
				}
			}
		}
	}

	if let Some(name) = claimed.difference(&present).next() {
		return Err(err!(
			"The registry names the golden test {}, and the crate has no such test. \
			A format nothing freezes can change by accident and orphan every store \
			already written in it.", name; Missing));
	}
	if let Some(name) = present.difference(&claimed).next() {
		return Err(err!(
			"The crate has the golden test {} and the registry does not name it, so \
			what it freezes is not written down anywhere a reader elsewhere can find \
			it.", name; Missing));
	}
	Ok(())
}

/// Every golden test freezes the constants the registry says it pins.
///
/// This is the check that turns the contract's sentence about a version bump --
/// index 6, `0x03` to `0x04`, and nothing else -- into an instruction the failure
/// prints for itself.
#[test]
fn every_golden_test_freezes_the_constants_it_pins() -> Outcome<()> {
	let reg = res!(registry());
	for g in res!(rows::<Golden>(&reg.goldens, "goldens")) {
		let path = root().join(&g.file);
		if !path.exists() {
			return Err(err!(
				"The registry says the golden test {} is in {}, and there is no such \
				file.", g.name, g.file; Missing, File));
		}
		let bytes = res!(frozen_bytes(&res!(slurp(&path)), &g.name));

		let magic = match compiled(&g.magic) {
			Some(Dat::Str(s)) => s,
			_ => return Err(err!(
				"The registry says {} begins with {}, which this crate does not \
				declare as a magic.", g.name, g.magic; Missing)),
		};
		let head: String = bytes
			.iter()
			.take(magic.len())
			.map(|b| *b as char)
			.collect();
		if head != magic {
			return Err(err!(
				"{} freezes bytes beginning {:?}, and the registry says it freezes \
				{}, which is {:?}.", g.name, head, g.magic, magic; Mismatch));
		}

		for p in res!(rows::<Pin>(&g.pins, "pins")) {
			let want = res!(byte_of(&p.is));
			let at = p.at as usize;
			if at >= bytes.len() {
				return Err(err!(
					"The registry pins byte {} of {} to {}, and that test freezes \
					only {} bytes.", p.at, g.name, p.is, bytes.len(); Mismatch));
			}
			if bytes[at] != want {
				return Err(err!(
					"{} freezes byte {} as {:#04x}, and {} is {:#04x}. The frozen \
					array has not been moved, so the bytes this test calls the \
					format are not the bytes this crate writes.",
					g.name, p.at, bytes[at], p.is, want; Mismatch));
			}
		}
	}
	Ok(())
}

/// The three claims here are separable: what a version admits, that the table
/// reaches as far as `VERSION`, and that the vocabulary only ever grew.
#[test]
fn the_version_table_agrees_with_highest_code() -> Outcome<()> {
	let reg = res!(registry());
	let table: Vec<VersionRow> = res!(rows(&reg.versions, "versions"));

	let mut want = segment::VERSION_MIN as u64;
	let mut last = 0u8;
	for row in &table {
		if row.version != want {
			return Err(err!(
				"The registry's version table goes to version {} where {} was next. \
				It must cover every version from segment::VERSION_MIN ({}) to \
				segment::VERSION ({}), because that range is what this crate promises \
				to read.",
				row.version, want, segment::VERSION_MIN, segment::VERSION;
				Mismatch));
		}
		let top = res!(byte_of(&row.highest_code));
		let got = segment::highest_code(row.version as u8);
		if got != top {
			return Err(err!(
				"The registry says a version {} segment carries up to {} ({}), and \
				segment::highest_code says {}. A writer continuing somebody else's \
				segment asks highest_code, so the registry is telling a reader \
				something no writer obeys.",
				row.version, row.highest_code, top, got; Mismatch));
		}
		if top < last {
			return Err(err!(
				"The registry says version {} carries up to {} and the version \
				before it carried up to {}. The vocabulary only ever grows upwards; \
				a version admitting less than its predecessor breaks the subset \
				promise VERSION_MIN rests on.",
				row.version, top, last; Mismatch));
		}
		last = top;
		want += 1;
	}
	if want != segment::VERSION as u64 + 1 {
		return Err(err!(
			"segment::VERSION is {} and the registry's version table stops at {}. \
			A version rose without a row saying which operation codes it admits, so \
			highest_code has gained a branch nothing checks.",
			segment::VERSION, want - 1; Missing));
	}

	// A code above what the current version admits could never be written, so it
	// is a code somebody added without moving the version with it.
	let top = segment::highest_code(segment::VERSION);
	for c in res!(rows::<Constant>(&reg.constants, "constants")) {
		if c.name.starts_with("op::CODE_") {
			let code = res!(byte_of(&c.name));
			if code > top {
				return Err(err!(
					"{} is {} and a segment at segment::VERSION ({}) admits at most \
					{}, so nothing could ever write it: Store::append refuses it and \
					no fallback catches that.",
					c.name, code, segment::VERSION, top; Mismatch));
			}
		}
	}
	Ok(())
}

/// No message kind sits above what the message version admits.
///
/// The mirror of the last block of the test above, for the other vocabulary.
/// `sync::msg::highest_kind` is the rule the message kinds grow by and nothing
/// checked it: a kind added without the version moving is a message no peer
/// could ever legally stamp, and a version moving with no kind above the old top
/// is a version that bought nothing and refuses old peers for no reason.
#[test]
fn no_message_kind_sits_above_what_its_version_admits() -> Outcome<()> {
	let top = msg::highest_kind(msg::VERSION);
	for c in res!(constants()) {
		if c.name.starts_with("sync::msg::KIND_") {
			let kind = res!(byte_of(&c.name));
			if kind > top {
				return Err(err!(
					"{} is {} and a peer at sync::msg::VERSION ({}) may send up to \
					kind {}, so nothing could ever stamp it: Message::decode refuses \
					a kind its declared version does not admit.",
					c.name, kind, msg::VERSION, top; Mismatch));
			}
		}
	}
	let was = msg::highest_kind(msg::VERSION_MIN);
	if msg::VERSION != msg::VERSION_MIN && was >= top {
		return Err(err!(
			"sync::msg::VERSION is {} and sync::msg::VERSION_MIN is {}, and \
			highest_kind admits up to kind {} at both. A version that adds no kind \
			refuses every older peer and buys nothing for it.",
			msg::VERSION, msg::VERSION_MIN, top; Mismatch));
	}
	Ok(())
}

/// A name the registry records as removed really is gone.
///
/// The inversion is the point. The row that named a deleted file sat in the live
/// table for four days; here the same row asserts the deletion, so it reddens if
/// the name comes back rather than if it does not.
#[test]
fn every_removed_name_is_really_gone() -> Outcome<()> {
	let reg = res!(registry());
	for r in res!(rows::<Removed>(&reg.removed, "removed")) {
		// A bare `VERSION` means one thing in each module that declares one, so a
		// removed constant is looked for only in the module it was removed from.
		// A test function's name is its own, so that is looked for everywhere.
		let paths = match r.name.contains("::") {
			true => {
				let path = root().join(&r.file);
				match path.exists() {
					true	=> vec![path],
					false	=> continue,
				}
			},
			false => res!(sources()),
		};
		let name = short(&r.name);
		for path in paths {
			let src = res!(slurp(&path));
			for (i, line) in src.lines().enumerate() {
				let code = code_of(line).trim_start();
				let hit = declared_const(line) == Some(name)
					|| code.strip_prefix("fn ").map(|s| s.starts_with(name)) == Some(true);
				if hit {
					return Err(err!(
						"The registry records {} as removed on {} in {}, and {}:{} \
						declares it. If it is back, its row belongs in the live \
						section where a test checks its value.",
						r.name, r.gone, r.commit,
						path.display(), i + 1; Mismatch));
				}
			}
		}
	}
	Ok(())
}

/// No name the owner has not yet fixed has been invented.
///
/// A lane that needs a name the registry does not carry is required to report the
/// seam rather than invent one, and until now nothing enforced that. Each open
/// question guards the spellings whose appearance would mean somebody acted: a
/// hit is either an answer the registry was not told about, or a name invented in
/// passing.
#[test]
fn no_open_name_has_been_invented() -> Outcome<()> {
	let reg = res!(registry());
	let open: Vec<Open> = res!(rows(&reg.open, "open"));
	let mut haystack: Vec<(String, String)> = Vec::new();
	for path in res!(sources()) {
		haystack.push((fmt!("{}", path.display()), res!(slurp(&path))));
	}
	let manifest = root().join("Cargo.toml");
	haystack.push((fmt!("{}", manifest.display()), res!(slurp(&manifest))));

	for o in open {
		for name in &o.guard {
			for (where_, src) in &haystack {
				for (i, line) in src.lines().enumerate() {
					if !introduces(line, name) {
						continue;
					}
					return Err(err!(
						"The registry records '{}' as open, put to the owner on {} \
						and unanswered, and {}:{} introduces '{}'. Either it was \
						answered and the registry was not told, or a name was \
						invented that the reasoning in {} says cannot be guessed: {}.",
						o.question, o.asked, where_, i + 1, name, o.note, o.why;
						Mismatch));
				}
			}
		}
	}
	Ok(())
}

/// A question the owner has answered still refuses what the answer ruled out.
///
/// The inversion the `removed` section makes, made again. While a question is
/// open every candidate spelling is guarded so that nobody quietly picks one;
/// once it is answered, the spellings the answer did NOT pick have to stay
/// picked-against, or the second lane to arrive invents the alternative that was
/// considered and rejected and nothing says it was.
///
/// The answer itself is checked too, where it names a constant: a settled row
/// pointing at a name the registry does not carry is a decision recorded and
/// never built.
#[test]
fn every_settled_question_still_refuses_what_it_ruled_out() -> Outcome<()> {
	let reg = res!(registry());
	let known: BTreeSet<String> = res!(constants()).into_iter().map(|c| c.name).collect();
	let settled: Vec<Settled> = res!(rows(&reg.settled, "settled"));
	if settled.is_empty() {
		return Err(err!(
			"The registry carries no settled questions. Every one that has been \
			answered belongs here, or the reasoning behind it lives only in whatever \
			document nobody opens."; Missing));
	}
	let mut haystack: Vec<(String, String)> = Vec::new();
	for path in res!(sources()) {
		haystack.push((fmt!("{}", path.display()), res!(slurp(&path))));
	}
	let manifest = root().join("Cargo.toml");
	haystack.push((fmt!("{}", manifest.display()), res!(slurp(&manifest))));

	for row in settled {
		for name in &row.names {
			if !known.contains(name) {
				return Err(err!(
					"The registry says '{}' was answered on {} by {}, and the constants \
					section does not carry that name. A decision recorded and never \
					built is worse than one still open, because nothing looks for it.",
					row.question, row.answered, name; Missing));
			}
		}
		for name in &row.refused {
			for (where_, src) in &haystack {
				for (i, line) in src.lines().enumerate() {
					if !introduces(line, name) {
						continue;
					}
					return Err(err!(
						"'{}' was answered on {} -- {} -- and {}:{} introduces '{}', \
						which the answer ruled out. Either the answer has changed and \
						the registry was not told, or the alternative that was \
						considered and rejected has been built beside the one that was \
						chosen. The reasoning is in {}.",
						row.question, row.answered, row.answer, where_, i + 1, name,
						row.note; Mismatch));
				}
			}
		}
	}
	Ok(())
}

/// Does this line introduce the given name, rather than merely mention it?
///
/// A local binding is not an introduction; a declaration, a field, a module and a
/// dependency are. Comments are stripped first, so a note about compression in
/// prose is not a name somebody fixed.
fn introduces(line: &str, name: &str) -> bool {
	let code = code_of(line).trim();
	for kw in ["const ", "fn ", "struct ", "enum ", "static ", "mod ", "use "] {
		if let Some(i) = code.find(kw) {
			let rest = &code[i + kw.len()..];
			if rest.strip_prefix(name).map(ends_name) == Some(true) {
				return true;
			}
		}
	}
	// A struct field, a literal's field, or a dependency in the manifest.
	match code.strip_prefix(name) {
		Some(rest) => {
			let rest = rest.trim_start();
			rest.starts_with(':') || rest.starts_with('=')
		},
		None => false,
	}
}

fn ends_name(rest: &str) -> bool {
	match rest.chars().next() {
		Some(c) => !(c.is_alphanumeric() || c == '_'),
		None => true,
	}
}
