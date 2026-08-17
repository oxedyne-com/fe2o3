//! An emitter for git's fast-import stream, the format `git fast-import` reads.
//!
//! This is [`crate::fastexport`] in the other direction, and it exists for the
//! same reason: the byte stream described by `git-fast-import(1)` is the one
//! interface git maintains for foreign producers of a repository, so writing it
//! keeps packfiles, the reference backend and the object hash somebody else's
//! problem. A history held in this crate's vocabulary becomes a git repository
//! by being spelled as this stream and handed to git.
//!
//! # A pure emitter
//!
//! Bytes out, and nothing else. Nothing here spawns a process, opens a file or
//! reads a clock: running `git fast-import` over what this produces is the
//! caller's job. That keeps the module portable to `wasm32-unknown-unknown`
//! along with the rest of the crate, and it keeps it testable against a stream
//! read back by the parser next door.
//!
//! # One vocabulary, both directions
//!
//! [`FileMode`], [`Person`], [`When`], [`TzOffset`], [`ObjRef`] and [`BlobRef`]
//! are the parser's types, used here rather than declared again, so a stream
//! parsed and re-emitted is expressible without a translation layer.
//!
//! # Refuse rather than write something wrong
//!
//! The parser refuses a line it cannot place rather than reading past it, and
//! this keeps the same posture on the way out. Git will take a stream that puts
//! a file and a directory at one path and will silently drop the file; it will
//! abort part way through on a path holding `.git`, leaving a half-built
//! repository behind. Both are checked here, before a byte is emitted, and named
//! in the error.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_ore::fastexport::{BlobRef, FileMode, Person, TzOffset, When};
//! use oxedyne_fe2o3_ore::gitexport::{Change, Commit, Stream};
//!
//! # fn main() -> Outcome<()> {
//! let who = Person {
//! 	name:	b"Ada Lovelace".to_vec(),
//! 	email:	b"ada@example.org".to_vec(),
//! 	when:	When { secs: 0, tz: TzOffset::new(0) },
//! };
//! let mut stream = Stream::new();
//! res!(stream.commit(&Commit {
//! 	refname:	fmt!("refs/heads/main"),
//! 	mark:		Some(1),
//! 	author:		None,
//! 	committer:	who,
//! 	message:	b"first".to_vec(),
//! 	from:		None,
//! 	merges:		Vec::new(),
//! 	changes:	vec![Change::Modify {
//! 		mode:	FileMode::Normal,
//! 		data:	BlobRef::Inline(b"hello\n".to_vec()),
//! 		path:	b"greeting.txt".to_vec(),
//! 	}],
//! }));
//! stream.done();
//! assert!(stream.bytes().starts_with(b"commit refs/heads/main\n"));
//! # Ok(())
//! # }
//! ```
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::fastexport::{
	show,
	BlobRef,
	FileMode,
	ObjRef,
	Person,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{
	BTreeMap,
	BTreeSet,
};


// Bytes a ref name may not contain, beyond the shapes `check_refname` refuses
// outright.
const REF_FORBIDDEN: &[u8] = b" ~^:?*[\\";


// ---------------------------------------------------------------------------
// What a commit says about the tree.
// ---------------------------------------------------------------------------

/// One change a commit makes to the tree.
///
/// The parser's `FileChange` carries the copy, the rename and the note as well,
/// which git infers rather than records; a tree is fully described by what each
/// path holds, so those are not spelled here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change {
	Modify {	// `M`, sets the content and mode of a path
		mode:	FileMode,
		data:	BlobRef,
		path:	Vec<u8>,
	},
	// `D`. Removing a path that names a directory removes everything under it,
	// which is git's own rule.
	Delete {
		path:	Vec<u8>,
	},
	DeleteAll,	// `deleteall`, emptying the tree before the rest apply
}


/// A `blob` command and its payload.
///
/// Content may be given a mark here and referred to by that mark from any
/// number of later commits, which is what a stream does when one blob appears in
/// many trees. Content named once is simpler given inline, as
/// [`BlobRef::Inline`], whose bytes follow the file command as a `data` payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Blob {
	pub mark:	Option<u64>,
	pub data:	Vec<u8>,
}


/// A `commit` command, complete with its file changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
	pub refname:	String,
	pub mark:	Option<u64>,
	pub author:	Option<Person>,	// absent where it is the committer
	pub committer:	Person,
	pub message:	Vec<u8>,
	pub from:	Option<ObjRef>,	// first parent, absent for a root commit
	pub merges:	Vec<ObjRef>,	// the rest, in order
	pub changes:	Vec<Change>,	// in the order they are applied
}


/// One entry of a tree: what a path holds, and what it is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
	pub mode:	FileMode,
	pub data:	Vec<u8>,
}


/// A whole tree, by path.
pub type Tree = BTreeMap<Vec<u8>, Entry>;


// ---------------------------------------------------------------------------
// The emitter.
// ---------------------------------------------------------------------------

/// A fast-import stream under construction.
#[derive(Clone, Debug, Default)]
pub struct Stream {
	out:	Vec<u8>,
	ended:	bool,		// after `done`, nothing more may be written
}

impl Stream {

	pub fn new() -> Self {
		Self::default()
	}

	pub fn bytes(&self) -> &[u8] {
		&self.out
	}

	/// Leaves the stream empty and unfinished.
	pub fn take(&mut self) -> Vec<u8> {
		self.ended = false;
		std::mem::take(&mut self.out)
	}

	pub fn len(&self) -> usize {
		self.out.len()
	}

	pub fn is_empty(&self) -> bool {
		self.out.is_empty()
	}

	pub fn blob(&mut self, blob: &Blob)
		-> Outcome<()>
	{
		res!(self.open("blob"));
		self.out.extend_from_slice(b"blob\n");
		if let Some(mark) = blob.mark {
			res!(self.mark(mark));
		}
		self.data(&blob.data);
		Ok(())
	}

	/// Writes a `commit` command and every change it carries.
	pub fn commit(&mut self, commit: &Commit)
		-> Outcome<()>
	{
		res!(self.open("commit"));
		res!(check_refname(&commit.refname));
		for change in &commit.changes {
			res!(check_change(change));
		}
		self.out.extend_from_slice(b"commit ");
		self.out.extend_from_slice(commit.refname.as_bytes());
		self.out.push(b'\n');
		if let Some(mark) = commit.mark {
			res!(self.mark(mark));
		}
		if let Some(author) = &commit.author {
			res!(self.identity("author", author));
		}
		res!(self.identity("committer", &commit.committer));
		self.data(&commit.message);
		if let Some(from) = &commit.from {
			self.out.extend_from_slice(b"from ");
			self.out.extend_from_slice(fmt!("{}", from).as_bytes());
			self.out.push(b'\n');
		}
		for merge in &commit.merges {
			self.out.extend_from_slice(b"merge ");
			self.out.extend_from_slice(fmt!("{}", merge).as_bytes());
			self.out.push(b'\n');
		}
		for change in &commit.changes {
			self.change(change);
		}
		self.out.push(b'\n');
		Ok(())
	}

	/// Writes a `reset` command, which points a reference somewhere.
	///
	/// This is also how a lightweight tag is made: a reset of `refs/tags/<name>`
	/// mints no object of its own, which is exactly what a lightweight tag is.
	pub fn reset(&mut self, refname: &str, from: Option<&ObjRef>)
		-> Outcome<()>
	{
		res!(self.open("reset"));
		res!(check_refname(refname));
		self.out.extend_from_slice(b"reset ");
		self.out.extend_from_slice(refname.as_bytes());
		self.out.push(b'\n');
		if let Some(at) = from {
			self.out.extend_from_slice(b"from ");
			self.out.extend_from_slice(fmt!("{}", at).as_bytes());
			self.out.push(b'\n');
		}
		self.out.push(b'\n');
		Ok(())
	}

	/// Writes a `checkpoint` command, asking git to make its work durable.
	pub fn checkpoint(&mut self)
		-> Outcome<()>
	{
		res!(self.open("checkpoint"));
		self.out.extend_from_slice(b"checkpoint\n\n");
		Ok(())
	}

	/// Writes a `progress` command, whose text is for the operator.
	pub fn progress(&mut self, text: &[u8])
		-> Outcome<()>
	{
		res!(self.open("progress"));
		if text.contains(&b'\n') {
			return Err(err!(
				"Progress text \"{}\" holds a line feed, which would end the command \
				early.", show(text);
			Invalid, Input));
		}
		self.out.extend_from_slice(b"progress ");
		self.out.extend_from_slice(text);
		self.out.extend_from_slice(b"\n\n");
		Ok(())
	}

	/// Writes the `done` command, which says the stream ended where it meant to.
	///
	/// Run git with `--done` and a stream cut short is refused rather than
	/// half-applied, which is the difference between a mirror that failed and a
	/// mirror that is quietly wrong.
	pub fn done(&mut self) {
		if !self.ended {
			self.out.extend_from_slice(b"done\n");
			self.ended = true;
		}
	}

	/// Refuses a command written after `done`.
	fn open(&self, what: &str)
		-> Outcome<()>
	{
		if self.ended {
			return Err(err!(
				"A {} command was written after the stream said done.", what;
			Invalid, Input, Order));
		}
		Ok(())
	}

	fn mark(&mut self, mark: u64)
		-> Outcome<()>
	{
		if mark == 0 {
			return Err(err!(
				"A mark is numbered from one, and zero is how the stream spells no \
				mark at all.";
			Invalid, Input, Range));
		}
		self.out.extend_from_slice(fmt!("mark :{}\n", mark).as_bytes());
		Ok(())
	}

	/// Writes an `author`, `committer` or `tagger` line.
	fn identity(&mut self, what: &str, who: &Person)
		-> Outcome<()>
	{
		res!(check_identity(who));
		self.out.extend_from_slice(what.as_bytes());
		self.out.push(b' ');
		self.out.extend_from_slice(&who.name);
		self.out.extend_from_slice(b" <");
		self.out.extend_from_slice(&who.email);
		self.out.extend_from_slice(b"> ");
		self.out.extend_from_slice(fmt!("{}", who.when).as_bytes());
		self.out.push(b'\n');
		Ok(())
	}

	/// Writes a `data` command and its payload, with the trailing line feed the
	/// format permits and git's own exporter writes.
	fn data(&mut self, payload: &[u8]) {
		self.out.extend_from_slice(fmt!("data {}\n", payload.len()).as_bytes());
		self.out.extend_from_slice(payload);
		self.out.push(b'\n');
	}

	/// Writes one file change, its path already checked.
	fn change(&mut self, change: &Change) {
		match change {
			Change::Modify { mode, data, path } => {
				self.out.extend_from_slice(b"M ");
				self.out.extend_from_slice(mode.as_str().as_bytes());
				self.out.push(b' ');
				match data {
					BlobRef::Mark(n)	=> self.out.extend_from_slice(fmt!(":{}", n).as_bytes()),
					BlobRef::Name(name)	=> self.out.extend_from_slice(name.as_bytes()),
					BlobRef::Inline(_)	=> self.out.extend_from_slice(b"inline"),
				}
				self.out.push(b' ');
				self.out.extend_from_slice(&quote_path(path));
				self.out.push(b'\n');
				if let BlobRef::Inline(bytes) = data {
					self.data(bytes);
				}
			},
			Change::Delete { path } => {
				self.out.extend_from_slice(b"D ");
				self.out.extend_from_slice(&quote_path(path));
				self.out.push(b'\n');
			},
			Change::DeleteAll => self.out.extend_from_slice(b"deleteall\n"),
		}
	}
}


// ---------------------------------------------------------------------------
// From one tree to the next.
// ---------------------------------------------------------------------------

/// A commit's changes are applied over its first parent's tree, so what a commit
/// has to say is the difference and not the whole. Removals come first, so that a
/// path which was a file and is now a directory, or the reverse, is emptied
/// before it is filled.
///
/// The whole of `next` is checked as a tree before anything is produced: git
/// takes a stream naming both `a` and `a/b` and drops `a` without a word, which
/// is the one failure a mirror must never have.
pub fn changes(prev: &Tree, next: &Tree)
	-> Outcome<Vec<Change>>
{
	res!(check_tree(next.keys().map(|p| p.as_slice())));
	let mut out = Vec::new();
	for path in prev.keys() {
		if !next.contains_key(path) {
			out.push(Change::Delete { path: path.clone() });
		}
	}
	for (path, entry) in next {
		if prev.get(path) == Some(entry) {
			continue;
		}
		out.push(Change::Modify {
			mode:	entry.mode,
			data:	BlobRef::Inline(entry.data.clone()),
			path:	path.clone(),
		});
	}
	for change in &out {
		res!(check_change(change));
	}
	Ok(out)
}


// ---------------------------------------------------------------------------
// What git will hold.
// ---------------------------------------------------------------------------

fn check_change(change: &Change)
	-> Outcome<()>
{
	match change {
		Change::Modify { mode, path, .. } => {
			res!(check_path(path));
			match mode {
				FileMode::Normal | FileMode::Executable | FileMode::Symlink => Ok(()),
				other => Err(err!(
					"The path {:?} is given the mode {}, which names something that is \
					not a file with bytes and so has no content to write.",
					show(path), other;
				Invalid, Input, NoImpl)),
			}
		},
		Change::Delete { path }	=> check_path(path),
		Change::DeleteAll		=> Ok(()),
	}
}

/// Git's own refusals are fatal and part way through: a path holding `.git`
/// aborts the import with a crash report and whatever was already applied left
/// behind. Refusing here instead costs nothing and leaves the repository
/// untouched.
pub fn check_path(path: &[u8])
	-> Outcome<()>
{
	if path.is_empty() {
		return Err(err!(
			"A file change names the empty path.";
		Invalid, Input, Missing));
	}
	if path.contains(&0) {
		return Err(err!(
			"The path \"{}\" holds a NUL byte, which no git tree can name.", show(path);
		Invalid, Input));
	}
	if path[0] == b'/' {
		return Err(err!(
			"The path \"{}\" is absolute, and a git tree names everything relative to \
			the repository root.", show(path);
		Invalid, Input));
	}
	if path[path.len() - 1] == b'/' {
		return Err(err!(
			"The path \"{}\" ends in a separator, so it names a directory rather than \
			a file.", show(path);
		Invalid, Input));
	}
	for part in path.split(|b| *b == b'/') {
		if part.is_empty() {
			return Err(err!(
				"The path \"{}\" holds an empty component.", show(path);
			Invalid, Input));
		}
		if part == b"." || part == b".." {
			return Err(err!(
				"The path \"{}\" holds the component \"{}\", which names a directory \
				relative to another rather than a file.", show(path), show(part);
			Invalid, Input));
		}
		if part.eq_ignore_ascii_case(b".git") {
			return Err(err!(
				"The path \"{}\" holds the component \"{}\", and a git repository \
				keeps its own workings there.", show(path), show(part);
			Invalid, Input, Conflict));
		}
	}
	Ok(())
}

/// Git names a path either a file or a directory and never both. Handed both it
/// keeps the directory and drops the file, saying nothing, so a caller that
/// might hold such a state asks here first.
pub fn check_tree<'a, I>(paths: I)
	-> Outcome<()>
where
	I: IntoIterator<Item = &'a [u8]>,
{
	let mut files: BTreeSet<&[u8]> = BTreeSet::new();
	let mut dirs: BTreeMap<Vec<u8>, &[u8]> = BTreeMap::new();
	for path in paths {
		res!(check_path(path));
		files.insert(path);
		let mut at = 0;
		while let Some(i) = path[at..].iter().position(|b| *b == b'/') {
			at += i;
			dirs.entry(path[..at].to_vec()).or_insert(path);
			at += 1;
		}
	}
	for path in &files {
		if let Some(under) = dirs.get(*path) {
			return Err(err!(
				"The path \"{}\" is a file and is also the directory holding \"{}\". \
				Git names a path one or the other, and given both it keeps the \
				directory and drops the file without a word.",
				show(path), show(under);
			Invalid, Input, Conflict));
		}
	}
	Ok(())
}

fn check_identity(who: &Person)
	-> Outcome<()>
{
	for (what, bytes) in [("name", &who.name), ("email", &who.email)] {
		for b in bytes.iter() {
			match *b {
				0 | b'\n' | b'<' | b'>' => return Err(err!(
					"The {} \"{}\" holds the byte \"{}\", which an identity line uses \
					to say where its parts begin and end.",
					what, show(bytes), show(&[*b]);
				Invalid, Input)),
				_ => (),
			}
		}
	}
	if who.email.is_empty() {
		return Err(err!(
			"An identity line gives no email, and the angle brackets that hold one are \
			not optional.";
		Invalid, Input, Missing));
	}
	Ok(())
}

/// The rules are `git-check-ref-format(1)`'s, less the ones that concern a
/// reference spelled on a command line rather than in a stream.
pub fn check_refname(name: &str)
	-> Outcome<()>
{
	let bytes = name.as_bytes();
	if bytes.is_empty() {
		return Err(err!(
			"A command names the empty reference.";
		Invalid, Input, Missing));
	}
	if bytes[bytes.len() - 1] == b'/' || bytes[0] == b'/' {
		return Err(err!(
			"The reference {:?} begins or ends with a separator.", name;
		Invalid, Input));
	}
	if name.ends_with(".lock") || name.ends_with('.') {
		return Err(err!(
			"The reference {:?} ends in {:?}, which git reserves.",
			name, if name.ends_with(".lock") { ".lock" } else { "." };
		Invalid, Input, Conflict));
	}
	if name.contains("..") || name.contains("@{") {
		return Err(err!(
			"The reference {:?} holds a sequence git reads as a revision expression \
			rather than as a name.", name;
		Invalid, Input));
	}
	for b in bytes.iter() {
		if *b < 0x20 || *b == 0x7f || REF_FORBIDDEN.contains(b) {
			return Err(err!(
				"The reference {:?} holds the byte \"{}\", which git does not allow in \
				a name.", name, show(&[*b]);
			Invalid, Input));
		}
	}
	for part in name.split('/') {
		if part.is_empty() {
			return Err(err!(
				"The reference {:?} holds an empty component.", name;
			Invalid, Input));
		}
		if part.starts_with('.') {
			return Err(err!(
				"The reference {:?} holds the component {:?}, and a component may not \
				begin with a full stop.", name, part;
			Invalid, Input));
		}
	}
	Ok(())
}

/// Returns a name git will accept as a reference component, derived from `name`.
///
/// Every byte git refuses becomes a hyphen and a leading full stop is replaced,
/// so the result is a function of the input and of nothing else. It is not
/// reversible and it is not injective: a caller that needs two different names
/// to stay different appends something that distinguishes them, since only the
/// caller knows what that is.
pub fn sanitise_refname(name: &str) -> String {
	let mut out = String::new();
	for ch in name.chars() {
		let ok = !ch.is_control()
			&& ch != '\u{7f}'
			&& !REF_FORBIDDEN.contains(&(ch as u32 as u8).min(0x7f))
			&& !"~^:?*[\\ ".contains(ch)
			&& ch != '/';
		if ok {
			out.push(ch);
		} else {
			out.push('-');
		}
	}
	while out.contains("..") {
		out = out.replace("..", ".-.");
	}
	while out.contains("@{") {
		out = out.replace("@{", "@-{");
	}
	if out.starts_with('.') {
		out.replace_range(0..1, "-");
	}
	if out.ends_with('.') {
		let n = out.len();
		out.replace_range(n - 1..n, "-");
	}
	if out.ends_with(".lock") {
		let n = out.len();
		out.replace_range(n - 5..n, "-lock");
	}
	if out.is_empty() {
		out.push('-');
	}
	out
}

/// Returns a path as the stream spells it, quoted where it has to be.
///
/// An unquoted path runs to the next space, so a path holding one is quoted; so
/// is a path holding a line feed, a quotation mark, a backslash, or any byte
/// outside printable ASCII, since those are what the C-style escapes exist for.
/// The escapes are the ones [`crate::fastexport`] reads back.
pub fn quote_path(path: &[u8]) -> Vec<u8> {
	let plain = path.iter().all(|b| {
		(0x21..=0x7e).contains(b) && *b != b'"' && *b != b'\\'
	});
	if plain {
		return path.to_vec();
	}
	let mut out = vec![b'"'];
	for b in path {
		match *b {
			0x07		=> out.extend_from_slice(b"\\a"),
			0x08		=> out.extend_from_slice(b"\\b"),
			0x0c		=> out.extend_from_slice(b"\\f"),
			b'\n'		=> out.extend_from_slice(b"\\n"),
			b'\r'		=> out.extend_from_slice(b"\\r"),
			b'\t'		=> out.extend_from_slice(b"\\t"),
			0x0b		=> out.extend_from_slice(b"\\v"),
			b'"'		=> out.extend_from_slice(b"\\\""),
			b'\\'		=> out.extend_from_slice(b"\\\\"),
			0x20..=0x7e	=> out.push(*b),
			other		=> out.extend_from_slice(fmt!("\\{:03o}", other).as_bytes()),
		}
	}
	out.push(b'"');
	out
}


#[cfg(test)]
mod test {
	use super::*;
	use crate::fastexport::{
		Event,
		FileChange,
		Parser,
		TzOffset,
		When,
	};

	fn ada() -> Person {
		Person {
			name:	b"Ada Lovelace".to_vec(),
			email:	b"ada@example.org".to_vec(),
			when:	When { secs: 1_700_000_000, tz: TzOffset::new(600) },
		}
	}

	fn one_file(path: &[u8], data: &[u8]) -> Commit {
		Commit {
			refname:	fmt!("refs/heads/main"),
			mark:		Some(1),
			author:		None,
			committer:	ada(),
			message:	b"only".to_vec(),
			from:		None,
			merges:		Vec::new(),
			changes:	vec![Change::Modify {
				mode:	FileMode::Normal,
				data:	BlobRef::Inline(data.to_vec()),
				path:	path.to_vec(),
			}],
		}
	}

	/// Reads a stream back through the parser.
	///
	/// The `done` the emitter ends with is checked for and then dropped, so a
	/// caller counts the commands it wrote rather than the commands plus one.
	fn reparse(bytes: &[u8])
		-> Outcome<Vec<Event>>
	{
		let mut parser = Parser::new();
		parser.feed(bytes);
		parser.end();
		let mut out = Vec::new();
		while let Some(event) = res!(parser.next_event()) {
			out.push(event);
		}
		match out.pop() {
			Some(Event::Done)	=> (),
			other => return Err(err!(
				"The stream did not end with done, but with {:?}.", other; Test)),
		}
		Ok(out)
	}

	/// What is emitted is what the parser next door reads.
	#[test]
	fn a_commit_reparses_to_itself() -> Outcome<()> {
		let mut stream = Stream::new();
		res!(stream.commit(&one_file(b"greeting.txt", b"hello\n")));
		stream.done();
		let events = res!(reparse(stream.bytes()));
		assert_eq!(events.len(), 1, "one commit, one event: {:?}", events);
		let commit = match &events[0] {
			Event::Commit(c) => c,
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		};
		assert_eq!(commit.refname, "refs/heads/main");
		assert_eq!(commit.mark, Some(1));
		assert_eq!(commit.committer, ada());
		assert_eq!(commit.message, b"only");
		assert_eq!(commit.changes, vec![FileChange::Modify {
			mode:	FileMode::Normal,
			data:	BlobRef::Inline(b"hello\n".to_vec()),
			path:	b"greeting.txt".to_vec(),
		}]);
		Ok(())
	}

	/// A payload holding what looks like a command is length-prefixed, so the
	/// parser reads it as content and not as a command.
	#[test]
	fn a_payload_that_looks_like_a_command_is_content() -> Outcome<()> {
		let payload = b"commit refs/heads/other\ndata 3\nxxx\n\x00\xff\n";
		let mut stream = Stream::new();
		res!(stream.commit(&one_file(b"f", payload)));
		stream.done();
		let events = res!(reparse(stream.bytes()));
		assert_eq!(events.len(), 1, "one command was written and one was read");
		match &events[0] {
			Event::Commit(c) => assert_eq!(c.changes, vec![FileChange::Modify {
				mode:	FileMode::Normal,
				data:	BlobRef::Inline(payload.to_vec()),
				path:	b"f".to_vec(),
			}]),
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn odd_paths_round_trip() -> Outcome<()> {
		let paths: Vec<Vec<u8>> = vec![
			b"plain.txt".to_vec(),
			b"a space.txt".to_vec(),
			b"quote\"mark".to_vec(),
			b"back\\slash".to_vec(),
			b"line\nfeed".to_vec(),
			b"tab\there".to_vec(),
			vec![0xff, 0xfe, b'.', b't'],
			"caf\u{e9}.txt".as_bytes().to_vec(),
		];
		for path in &paths {
			let mut stream = Stream::new();
			res!(stream.commit(&one_file(path, b"x")));
			stream.done();
			let events = res!(reparse(stream.bytes()));
			let commit = match &events[0] {
				Event::Commit(c) => c,
				other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
			};
			match &commit.changes[0] {
				FileChange::Modify { path: got, .. } => assert_eq!(
					got, path, "the path {:?} came back as {:?}", show(path), show(got)),
				other => return Err(err!("Expected a modify, got {:?}.", other; Test)),
			}
		}
		Ok(())
	}

	/// A symbolic link is a mode and a blob whose bytes are the target, which is
	/// git's own model and needs no invention.
	#[test]
	fn a_symlink_is_a_mode_and_a_target() -> Outcome<()> {
		let mut stream = Stream::new();
		res!(stream.commit(&Commit {
			changes: vec![Change::Modify {
				mode:	FileMode::Symlink,
				data:	BlobRef::Inline(b"elsewhere/target".to_vec()),
				path:	b"link".to_vec(),
			}],
			..one_file(b"unused", b"")
		}));
		stream.done();
		let text = String::from_utf8_lossy(stream.bytes()).into_owned();
		assert!(text.contains("M 120000 inline link\n"), "the mode is spelled: {}", text);
		assert!(text.contains("elsewhere/target"), "the target is the content: {}", text);
		Ok(())
	}

	/// A file and the directory above it at one path is refused, because git
	/// takes it and drops the file without a word.
	#[test]
	fn a_file_that_is_also_a_directory_is_refused() -> Outcome<()> {
		let mut next = Tree::new();
		next.insert(b"a".to_vec(), Entry { mode: FileMode::Normal, data: b"one".to_vec() });
		next.insert(b"a/b".to_vec(), Entry { mode: FileMode::Normal, data: b"two".to_vec() });
		let e = match changes(&Tree::new(), &next) {
			Ok(_) => return Err(err!("The clash between a and a/b was not refused."; Test)),
			Err(e) => fmt!("{}", e.plain()),
		};
		assert!(e.contains("\"a\""), "the file is named: {}", e);
		assert!(e.contains("a/b"), "and so is what is under it: {}", e);
		Ok(())
	}

	#[test]
	fn a_clash_is_found_across_intervening_paths() -> Outcome<()> {
		let mut next = Tree::new();
		for path in [&b"a"[..], b"a!x", b"a-y", b"a/b/c"] {
			next.insert(path.to_vec(), Entry {
				mode: FileMode::Normal, data: b"x".to_vec(),
			});
		}
		assert!(changes(&Tree::new(), &next).is_err(),
			"a and a/b/c clash however many paths sort between them");
		Ok(())
	}

	/// A path git aborts on is refused before a byte is emitted.
	#[test]
	fn paths_git_will_not_hold_are_refused() -> Outcome<()> {
		for path in [
			&b""[..], b"/absolute", b"trailing/", b"double//slash", b"here/./there",
			b"up/../out", b".git/config", b"sub/.GIT/x", b"nul\0byte",
		] {
			let mut stream = Stream::new();
			assert!(stream.commit(&one_file(path, b"x")).is_err(),
				"the path {:?} is refused", show(path));
			assert!(stream.is_empty(),
				"and nothing is emitted for it: {:?}", show(stream.bytes()));
		}
		Ok(())
	}

	#[test]
	fn a_mode_with_no_content_is_refused() -> Outcome<()> {
		for mode in [FileMode::Gitlink, FileMode::Subdirectory] {
			let mut stream = Stream::new();
			let outcome = stream.commit(&Commit {
				changes: vec![Change::Modify {
					mode,
					data:	BlobRef::Inline(Vec::new()),
					path:	b"thing".to_vec(),
				}],
				..one_file(b"unused", b"")
			});
			assert!(outcome.is_err(), "the mode {} is refused", mode);
		}
		Ok(())
	}

	#[test]
	fn an_identity_that_would_not_parse_is_refused() -> Outcome<()> {
		for (name, email) in [
			(&b"Ada <the first>"[..], &b"ada@example.org"[..]),
			(b"Ada", b"ada@example.org>x"),
			(b"Ada\nLovelace", b"ada@example.org"),
			(b"Ada", b""),
		] {
			let mut stream = Stream::new();
			let outcome = stream.commit(&Commit {
				committer: Person {
					name:	name.to_vec(),
					email:	email.to_vec(),
					when:	When { secs: 0, tz: TzOffset::new(0) },
				},
				..one_file(b"f", b"x")
			});
			assert!(outcome.is_err(),
				"the identity {:?} <{:?}> is refused", show(name), show(email));
		}
		Ok(())
	}

	/// Only the difference between two trees is written, and a removal is
	/// written before the path it frees is filled again.
	#[test]
	fn only_the_difference_is_written() -> Outcome<()> {
		let mut prev = Tree::new();
		prev.insert(b"kept".to_vec(), Entry {
			mode: FileMode::Normal, data: b"same".to_vec() });
		prev.insert(b"gone".to_vec(), Entry {
			mode: FileMode::Normal, data: b"away".to_vec() });
		prev.insert(b"a".to_vec(), Entry {
			mode: FileMode::Normal, data: b"file".to_vec() });
		let mut next = Tree::new();
		next.insert(b"kept".to_vec(), Entry {
			mode: FileMode::Normal, data: b"same".to_vec() });
		next.insert(b"a/b".to_vec(), Entry {
			mode: FileMode::Normal, data: b"now a directory".to_vec() });
		let got = res!(changes(&prev, &next));
		assert_eq!(got, vec![
			Change::Delete { path: b"a".to_vec() },
			Change::Delete { path: b"gone".to_vec() },
			Change::Modify {
				mode:	FileMode::Normal,
				data:	BlobRef::Inline(b"now a directory".to_vec()),
				path:	b"a/b".to_vec(),
			},
		], "the unchanged path is not restated and the removals come first");
		Ok(())
	}

	#[test]
	fn a_mode_alone_is_a_change() -> Outcome<()> {
		let mut prev = Tree::new();
		prev.insert(b"s.sh".to_vec(), Entry {
			mode: FileMode::Normal, data: b"#!/bin/sh\n".to_vec() });
		let mut next = Tree::new();
		next.insert(b"s.sh".to_vec(), Entry {
			mode: FileMode::Executable, data: b"#!/bin/sh\n".to_vec() });
		let got = res!(changes(&prev, &next));
		assert_eq!(got.len(), 1, "the file is restated: {:?}", got);
		match &got[0] {
			Change::Modify { mode, .. } => assert_eq!(*mode, FileMode::Executable),
			other => return Err(err!("Expected a modify, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn reference_names_are_checked() -> Outcome<()> {
		for name in [
			"", "refs/heads/", "/refs/heads/main", "refs/heads/a..b",
			"refs/heads/main.lock", "refs/heads/.hidden", "refs/heads/a b",
			"refs/heads/a~1", "refs/heads/a@{0}", "refs/heads//main",
		] {
			assert!(check_refname(name).is_err(), "the reference {:?} is refused", name);
		}
		for name in ["refs/heads/main", "refs/tags/v1.0", "refs/tags/a.b-c_d"] {
			res!(check_refname(name));
		}
		Ok(())
	}

	#[test]
	fn names_are_made_usable() -> Outcome<()> {
		for name in [
			"plain", "with a space", "a~b^c:d?e*f[g\\h", ".leading", "trailing.",
			"a..b", "a@{0}", "", "\u{7f}control", "sub/path", "ends.lock",
		] {
			let made = sanitise_refname(name);
			res!(check_refname(&fmt!("refs/tags/{}", made)));
		}
		assert_eq!(sanitise_refname("with a space"), "with-a-space");
		assert_eq!(sanitise_refname("sub/path"), "sub-path");
		Ok(())
	}

	#[test]
	fn nothing_follows_done() -> Outcome<()> {
		let mut stream = Stream::new();
		res!(stream.commit(&one_file(b"f", b"x")));
		stream.done();
		assert!(stream.commit(&one_file(b"g", b"y")).is_err(),
			"a command after done is refused");
		stream.done();
		let text = String::from_utf8_lossy(stream.bytes()).into_owned();
		assert_eq!(text.matches("done\n").count(), 1, "done is written once: {}", text);
		Ok(())
	}

	/// A lightweight tag is a reset and mints no object.
	#[test]
	fn a_lightweight_tag_is_a_reset() -> Outcome<()> {
		let mut stream = Stream::new();
		res!(stream.commit(&one_file(b"f", b"x")));
		res!(stream.reset("refs/tags/v1", Some(&ObjRef::Mark(1))));
		stream.done();
		let events = res!(reparse(stream.bytes()));
		assert_eq!(events.len(), 2, "a commit and a reset: {:?}", events);
		match &events[1] {
			Event::Reset { refname, from } => {
				assert_eq!(refname, "refs/tags/v1");
				assert_eq!(*from, Some(ObjRef::Mark(1)));
			},
			other => return Err(err!("Expected a reset, got {:?}.", other; Test)),
		}
		Ok(())
	}

	/// A merge is spelled with the first parent apart from the rest, which is
	/// the only place the stream records parent order.
	#[test]
	fn a_merge_names_its_parents_in_order() -> Outcome<()> {
		let mut stream = Stream::new();
		res!(stream.commit(&Commit {
			mark:	Some(3),
			from:	Some(ObjRef::Mark(1)),
			merges:	vec![ObjRef::Mark(2)],
			..one_file(b"f", b"joined")
		}));
		stream.done();
		let events = res!(reparse(stream.bytes()));
		match &events[0] {
			Event::Commit(c) => {
				assert_eq!(c.from, Some(ObjRef::Mark(1)));
				assert_eq!(c.merges, vec![ObjRef::Mark(2)]);
			},
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	/// A blob given a mark is referred to by it, so one payload serves many
	/// commits.
	#[test]
	fn a_marked_blob_is_referred_to() -> Outcome<()> {
		let mut stream = Stream::new();
		res!(stream.blob(&Blob { mark: Some(9), data: b"shared\n".to_vec() }));
		res!(stream.commit(&Commit {
			changes: vec![Change::Modify {
				mode:	FileMode::Normal,
				data:	BlobRef::Mark(9),
				path:	b"f".to_vec(),
			}],
			..one_file(b"unused", b"")
		}));
		stream.done();
		let events = res!(reparse(stream.bytes()));
		assert_eq!(events.len(), 2, "a blob and a commit: {:?}", events);
		match &events[0] {
			Event::Blob(b) => {
				assert_eq!(b.mark, Some(9));
				assert_eq!(b.data, b"shared\n");
			},
			other => return Err(err!("Expected a blob, got {:?}.", other; Test)),
		}
		Ok(())
	}

	/// A mark is numbered from one, zero being how the stream spells none.
	#[test]
	fn mark_zero_is_refused() -> Outcome<()> {
		let mut stream = Stream::new();
		assert!(stream.blob(&Blob { mark: Some(0), data: Vec::new() }).is_err(),
			"a blob marked zero is refused");
		Ok(())
	}

	#[test]
	fn an_empty_tree_is_a_commit_with_no_changes() -> Outcome<()> {
		let got = res!(changes(&Tree::new(), &Tree::new()));
		assert!(got.is_empty(), "nothing to say: {:?}", got);
		let mut stream = Stream::new();
		res!(stream.commit(&Commit { changes: got, ..one_file(b"unused", b"") }));
		stream.done();
		let events = res!(reparse(stream.bytes()));
		match &events[0] {
			Event::Commit(c) => assert!(c.changes.is_empty()),
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}
}
