//! A parser for git's fast-import stream, the format `git fast-export` emits.
//!
//! Git keeps one documented interface for foreign consumers of a repository:
//! the byte stream described by `git-fast-import(1)`. Everything else about
//! how git stores history -- packfiles, the reference backend, the object hash
//! -- is internal and has changed more than once. Reading the stream instead of
//! the repository means none of that reaches this code. The Fossil version
//! control system imports git the same way, and for the same reason.
//!
//! # A pure parser
//!
//! Bytes in, events out. Nothing here spawns a process, opens a file or reads
//! a clock: obtaining the stream is the caller's job, whether that is a pipe
//! from `git fast-export`, a file, or a socket. That keeps the module portable
//! to `wasm32-unknown-unknown` along with the rest of the crate, and it keeps
//! it testable against hand-written streams.
//!
//! # Incremental
//!
//! [`Parser::feed`] takes whatever bytes have arrived, and
//! [`Parser::next_event`] yields events as they complete, so a whole repository
//! never has to be held in memory. Memory is bounded by the largest single
//! object in the stream rather than by the stream: one blob payload, or one
//! commit with its file changes, is assembled before it is handed over. Feeding
//! a stream one byte at a time and feeding it all at once produce exactly the
//! same events.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_ore::fastexport::{Event, Parser};
//!
//! # fn main() -> Outcome<()> {
//! let mut parser = Parser::new();
//! parser.feed(b"blob\nmark :1\ndata 5\nhello\n");
//! parser.end();
//! while let Some(event) = res!(parser.next_event()) {
//! 	match event {
//! 		Event::Blob(blob) => assert_eq!(blob.data, b"hello"),
//! 		other => return Err(err!("Unexpected event {:?}.", other; Test)),
//! 	}
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::op::Mode;

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;


const COMPACT_THRESHOLD: usize = 64 * 1024;	// consumed prefix tolerated
const SHOW_LIMIT: usize = 120;			// bytes of a line quoted in an error


// ---------------------------------------------------------------------------
// Stream vocabulary.
// ---------------------------------------------------------------------------

/// A reference to an object, as the stream is able to spell one.
///
/// The stream does not distinguish an object identifier from a branch name or
/// a revision expression, and a parser that has not read the repository cannot
/// tell them apart either, so both arrive as [`ObjRef::Name`]. Only a mark,
/// which the stream mints itself, is unambiguous.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ObjRef {
	Mark(u64),	// `:N`
	Name(String),	// hex oid, branch name, or revision expression
}

impl fmt::Display for ObjRef {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Mark(n)		=> write!(f, ":{}", n),
			Self::Name(name)	=> write!(f, "{}", name),
		}
	}
}


/// Where the content of a file entry comes from.
///
/// The stream either points at an object it has already described, or carries
/// the bytes in line with the file command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlobRef {
	Mark(u64),		// `:N`
	Name(String),		// hex oid or revision expression
	Inline(Vec<u8>),	// bytes that followed the file command as a `data` payload
}


/// The file modes git records in a tree.
///
/// Git stores a small fixed set, so this is an enum and not a number: a mode
/// outside the set is a stream this parser will not guess at.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FileMode {
	Normal,		// 100644
	Executable,	// 100755
	Symlink,	// 120000
	Gitlink,	// 160000, a commit of another repository
	Subdirectory,	// 040000, a directory entry
}

impl FileMode {
	/// Accepts both the six digit form and the shortened form git also permits.
	pub fn from_bytes(bytes: &[u8])
		-> Outcome<Self>
	{
		match bytes {
			b"100644" | b"644"		=> Ok(Self::Normal),
			b"100755" | b"755"		=> Ok(Self::Executable),
			b"120000"			=> Ok(Self::Symlink),
			b"160000"			=> Ok(Self::Gitlink),
			b"040000" | b"40000"		=> Ok(Self::Subdirectory),
			other => Err(err!(
				"File mode {:?} is not one of the modes git records.", show(other);
			Decode, Input, Invalid)),
		}
	}

	/// `None` is a gitlink or a directory entry: both are things a tree can point
	/// at and neither is a file with bytes, so there is nothing for
	/// [`crate::op::Op::FileMode`] to say about them. A consumer that meets one
	/// has a decision to make -- refuse, or leave the entry out -- and it is the
	/// consumer that knows which path and which commit to name, so it makes that
	/// decision rather than being handed a guess.
	pub const fn as_op_mode(&self) -> Option<Mode> {
		match self {
			Self::Normal						=> Some(Mode::Normal),
			Self::Executable					=> Some(Mode::Executable),
			Self::Symlink						=> Some(Mode::Symlink),
			Self::Gitlink | Self::Subdirectory	=> None,
		}
	}

	pub const fn as_str(&self) -> &'static str {
		match self {
			Self::Normal		=> "100644",
			Self::Executable	=> "100755",
			Self::Symlink		=> "120000",
			Self::Gitlink		=> "160000",
			Self::Subdirectory	=> "040000",
		}
	}
}

impl fmt::Display for FileMode {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.as_str())
	}
}


/// An offset from UTC, as an identity line carries it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TzOffset {
	pub mins:	i32,	// minutes, positive east of UTC
	pub neg:	bool,	// minus as written; only `-0000`, the unknown zone, needs it
}

impl TzOffset {
	pub const fn new(mins: i32) -> Self {
		Self { mins, neg: mins < 0 }
	}

	pub const fn secs(&self) -> i32 {
		self.mins * 60
	}
}

impl fmt::Display for TzOffset {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mag = self.mins.abs();
		write!(f, "{}{:02}{:02}", if self.neg { '-' } else { '+' }, mag / 60, mag % 60)
	}
}


/// The moment an identity line records.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct When {
	pub secs:	i64,	// since the Unix epoch, which git allows to be negative
	pub tz:		TzOffset,
}

impl fmt::Display for When {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} {}", self.secs, self.tz)
	}
}


/// One `author`, `committer` or `tagger` line.
///
/// The name and the email arrive as bytes because git does not require either
/// to be UTF-8, and a repository old enough to be worth importing is exactly
/// the kind that has a Latin-1 name in it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Person {
	pub name:	Vec<u8>,	// may be empty
	pub email:	Vec<u8>,	// without its angle brackets
	pub when:	When,
}

impl Person {
	pub fn name_lossy(&self) -> String {
		String::from_utf8_lossy(&self.name).into_owned()
	}

	pub fn email_lossy(&self) -> String {
		String::from_utf8_lossy(&self.email).into_owned()
	}
}


/// Splits git's `<seconds> <offset>` off the end of an identity line.
///
/// The line is git's own -- `Jason Hoogland <hoogland@gmail.com> 1735089438 +0800`
/// -- and this is the one place that decides where the name ends and the moment
/// begins. It is here rather than in each reader because the readers cannot be
/// allowed to disagree: [`crate::op::AUTHOR_TRAILER`] carries such a line inside a
/// mark, a mirror reads it to author a commit under the name it arrived with, and
/// a forge reads it to show a person that name. Two implementations agreeing
/// because they were written to agree is the arrangement this project has paid for
/// before.
///
/// # All of it or none of it
///
/// A tail that is not a moment leaves the whole line as the identity, and this is
/// the case worth stating: `J H <j@h.test>` splits on spaces perfectly well and
/// would come back as `J` under a rule that only counted fields. So the tail is
/// handed to [`parse_when`], the one reader of git's raw date format here, and its
/// refusal is the answer -- an error is what "there is no moment on the end of
/// this" looks like, rather than something being wrong.
///
/// A negative second is a commit dated before 1970, which git permits and which an
/// import writes back as it read it.
///
/// ```
/// use oxedyne_fe2o3_ore::fastexport::split_identity_line;
///
/// let (who, when) = match split_identity_line(
///     "Jason Hoogland <hoogland@gmail.com> 1735089438 +0800")
/// {
///     Some(split)	=> split,
///     None		=> panic!("a whole identity line splits"),
/// };
/// assert_eq!(who, "Jason Hoogland <hoogland@gmail.com>");
/// assert_eq!(when.secs, 1735089438);
/// assert_eq!(format!("{}", when.tz), "+0800");
///
/// // A tail that is not a moment, however much it looks like one. Losing
/// // somebody over a malformed offset would defeat the point of carrying the
/// // name at all, so all of the line is the person.
/// for line in [
///     "J H <j@h.test>",
///     "J H <j@h.test> 1735089438 tomorrow",
///     "J H <j@h.test> whenever +0800",
///     "J H <j@h.test> +0800",
///     "J H <j@h.test> 1735089438 +08:00",
///     "replica 4256968235 <4256968235@replica.invalid>",
/// ] {
///     assert!(split_identity_line(line).is_none(), "{:?} ends in no moment", line);
/// }
///
/// // A commit dated before 1970, which git writes with a negative second, in a
/// // zone that is not a whole hour.
/// let (who, when) = match split_identity_line("J H <j@h.test> -14182940 -0330") {
///     Some(split)	=> split,
///     None		=> panic!("that is a moment"),
/// };
/// assert_eq!(who, "J H <j@h.test>");
/// assert_eq!(when.secs, -14182940);
/// assert_eq!(format!("{}", when.tz), "-0330");
/// ```
pub fn split_identity_line(line: &str) -> Option<(&str, When)> {
	let offset = match line.rfind(' ') {
		Some(at)	=> at,
		None		=> return None,
	};
	let secs = match line[..offset].rfind(' ') {
		Some(at)	=> at,
		None		=> return None,
	};
	match parse_when(line[secs + 1..].as_bytes(), "identity") {
		Ok(when)	=> Some((&line[..secs], when)),
		Err(_)		=> None,
	}
}

/// Returns the identity an identity line names, which is all of it where the tail
/// is not a moment.
///
/// What a reader showing a person a name wants: the moment is bookkeeping, and a
/// page printing the value whole prints a timestamp in the middle of somebody's
/// name.
pub fn identity_in(line: &str) -> &str {
	match split_identity_line(line) {
		Some((identity, _))	=> identity,
		None				=> line,
	}
}


/// A commit signature carried verbatim through the stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpgSig {
	pub algo:	String,		// `sha1` or `sha256`
	pub format:	String,		// `openpgp`, `x509`, `ssh`
	pub sig:	Vec<u8>,	// unexamined
}


/// One change a commit makes to the tree.
///
/// Paths are bytes: git permits any byte but NUL and the path separator in a
/// path component, and the stream's C-style quoting exists precisely so that
/// such paths survive. They are unquoted here, so what a consumer receives is
/// the path itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileChange {
	Modify {	// `M`, sets the content and mode of a path
		mode:	FileMode,
		data:	BlobRef,
		path:	Vec<u8>,
	},
	Delete {	// `D`
		path:	Vec<u8>,
	},
	Copy {		// `C`, leaving the source in place
		src:	Vec<u8>,
		dst:	Vec<u8>,
	},
	Rename {	// `R`
		src:	Vec<u8>,
		dst:	Vec<u8>,
	},
	DeleteAll,	// `deleteall`, emptying the tree before the rest apply
	Note {		// `N`, annotating a commit
		data:	BlobRef,
		commit:	ObjRef,
	},
}


/// A `blob` command and its payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Blob {
	pub mark:		Option<u64>,
	pub original_oid:	Option<String>,	// its name in the exporting repository
	// The payload is never line-parsed, so a blob holding NUL bytes or bare
	// line feeds survives untouched.
	pub data:		Vec<u8>,
}


/// A `commit` command, complete with its file changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
	pub refname:		String,
	pub mark:		Option<u64>,
	pub original_oid:	Option<String>,
	pub author:		Option<Person>,		// absent where it is the committer
	pub committer:		Person,
	pub encoding:		Option<Vec<u8>>,	// of the message
	pub gpgsig:		Option<GpgSig>,
	pub message:		Vec<u8>,
	// Parents. `from` is absent for a root commit, and where the branch
	// already stands where the commit builds on.
	pub from:		Option<ObjRef>,
	pub merges:		Vec<ObjRef>,		// the rest, in order
	pub changes:		Vec<FileChange>,	// stream order is apply order
}


/// A `tag` command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tag {
	pub name:		String,		// short, without the `refs/tags/` prefix
	pub mark:		Option<u64>,
	pub original_oid:	Option<String>,
	pub from:		ObjRef,		// the object tagged
	pub tagger:		Option<Person>,	// an older repository's stream may omit it
	pub message:		Vec<u8>,
}


/// One complete command from the stream.
///
/// A commit arrives whole, with its file changes attached, because a consumer
/// has to see the whole commit before it can apply any of it. A blob arrives
/// whole for the same reason: its payload is what the consumer wanted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
	Blob(Blob),
	Commit(Commit),
	Tag(Tag),
	// A `reset` points a reference somewhere. Without `from` the reference
	// is only declared, and the commit that follows defines it.
	Reset {
		refname:	String,
		from:		Option<ObjRef>,
	},
	Progress(Vec<u8>),	// text for the operator; means nothing to the import
	Checkpoint,		// asks the consumer to make its work durable
	Alias {			// a second mark for an object already named
		mark:	u64,
		to:	ObjRef,
	},
	Feature {		// what the stream requires of its consumer
		name:	String,
		arg:	Option<String>,
	},
	Opt(Vec<u8>),		// an `option`, which a consumer may ignore
	Done,			// the stream is complete, not merely cut off
}


// ---------------------------------------------------------------------------
// The parser.
// ---------------------------------------------------------------------------

/// An incremental parser for the fast-import stream.
///
/// Bytes go in through [`Parser::feed`], events come out through
/// [`Parser::next_event`], and [`Parser::end`] declares that no more bytes are
/// coming. Until `end` has been called, `next_event` returning `None` means
/// only that the next command is not yet complete; afterwards it means the
/// stream is finished.
#[derive(Debug, Default)]
pub struct Parser {
	buf:	Vec<u8>,	// fed but not yet turned into events, prefix included
	pos:	usize,		// how much of `buf` has been consumed
	eof:	bool,
}

impl Parser {
	pub fn new() -> Self {
		Self::default()
	}

	/// Chunk boundaries carry no meaning: a command may be split anywhere, and
	/// the same stream delivered in different chunkings yields the same
	/// events.
	///
	/// A command that arrives split is parsed again from its first byte once
	/// more input lands, so the work done per chunk is bounded by the size of
	/// one command and not by the stream. Feeding whole reads rather than
	/// single bytes keeps that cost where it belongs.
	pub fn feed(&mut self, chunk: &[u8]) {
		self.buf.extend_from_slice(chunk);
	}

	/// After this, a command that is still incomplete is an error rather than
	/// a request for more input, which is how a truncated stream is caught.
	pub fn end(&mut self) {
		self.eof = true;
	}

	/// Has the stream ended and every byte of it become an event?
	pub fn is_exhausted(&self) -> bool {
		self.eof && self.remaining().iter().all(|b| *b == b'\n')
	}

	pub fn remaining(&self) -> &[u8] {
		&self.buf[self.pos..]
	}

	/// `None` means that the next command is not yet complete, or, once
	/// [`Parser::end`] has been called, that the stream is finished. An error
	/// names the offending line.
	pub fn next_event(&mut self)
		-> Outcome<Option<Event>>
	{
		// Blank lines separate commands and carry nothing.
		while self.pos < self.buf.len() && self.buf[self.pos] == b'\n' {
			self.pos += 1;
		}
		self.compact();
		if self.pos >= self.buf.len() {
			return Ok(None);
		}
		match res!(parse_command(&self.buf[self.pos..], self.eof)) {
			Some((event, used)) => {
				self.pos += used;
				self.compact();
				Ok(Some(event))
			},
			None => {
				if self.eof {
					Err(err!(
						"The stream ends part way through a command beginning \
						\"{}\".", show(first_line(&self.buf[self.pos..]));
					Decode, Input, Missing))
				} else {
					Ok(None)
				}
			},
		}
	}

	/// Drops the consumed prefix of the buffer once it is worth the move.
	fn compact(&mut self) {
		if self.pos == self.buf.len() {
			self.buf.clear();
			self.pos = 0;
		} else if self.pos >= COMPACT_THRESHOLD {
			self.buf.drain(..self.pos);
			self.pos = 0;
		}
	}
}

/// The convenience form of [`Parser`] for a stream small enough to have
/// already been read, used mostly in tests.
pub fn parse_all(bytes: &[u8])
	-> Outcome<Vec<Event>>
{
	let mut parser = Parser::new();
	parser.feed(bytes);
	parser.end();
	let mut events = Vec::new();
	while let Some(event) = res!(parser.next_event()) {
		events.push(event);
	}
	Ok(events)
}


// ---------------------------------------------------------------------------
// Line reading.
// ---------------------------------------------------------------------------

/// The result of asking for the next line.
enum Line<'a> {
	Got(&'a [u8], usize),	// the line, and the offset just past its line feed
	End,			// no bytes remain and none are coming
	Need,			// more input is needed before the line can be read
}

/// Reads the line beginning at `pos`, without its line feed.
///
/// A final line with no line feed is a truncated stream, not a line, so it is
/// refused rather than guessed at.
fn read_line(src: &[u8], pos: usize, eof: bool)
	-> Outcome<Line<'_>>
{
	if pos >= src.len() {
		return Ok(if eof { Line::End } else { Line::Need });
	}
	match src[pos..].iter().position(|b| *b == b'\n') {
		Some(i) => Ok(Line::Got(&src[pos..pos + i], pos + i + 1)),
		None => {
			if eof {
				Err(err!(
					"The stream ends with an unterminated line \"{}\".",
					show(&src[pos..]);
				Decode, Input, Missing))
			} else {
				Ok(Line::Need)
			}
		},
	}
}

fn first_line(src: &[u8]) -> &[u8] {
	match src.iter().position(|b| *b == b'\n') {
		Some(i) => &src[..i],
		None => src,
	}
}

/// Renders stream bytes for an error message, lossily and shortened.
pub(crate) fn show(bytes: &[u8]) -> String {
	let cut = bytes.len().min(SHOW_LIMIT);
	let mut out = String::new();
	for b in &bytes[..cut] {
		match *b {
			b'\n'			=> out.push_str("\\n"),
			b'\t'			=> out.push_str("\\t"),
			0x20..=0x7e		=> out.push(*b as char),
			other			=> out.push_str(&fmt!("\\{:03o}", other)),
		}
	}
	if bytes.len() > cut {
		out.push_str("...");
	}
	out
}


// ---------------------------------------------------------------------------
// Command parsing.
// ---------------------------------------------------------------------------

/// Parses one command from the front of `src`, giving the event and the bytes
/// it occupied. `None` where the command is not yet complete.
fn parse_command(src: &[u8], eof: bool)
	-> Outcome<Option<(Event, usize)>>
{
	let (line, next) = match res!(read_line(src, 0, eof)) {
		Line::Got(line, next)	=> (line, next),
		Line::End		=> return Ok(None),
		Line::Need		=> return Ok(None),
	};
	if line == b"blob" {
		parse_blob(src, next, eof)
	} else if let Some(rest) = after(line, b"commit ") {
		parse_commit(src, next, rest, eof)
	} else if let Some(rest) = after(line, b"tag ") {
		parse_tag(src, next, rest, eof)
	} else if let Some(rest) = after(line, b"reset ") {
		parse_reset(src, next, rest, eof)
	} else if line == b"alias" {
		parse_alias(src, next, eof)
	} else if let Some(rest) = after(line, b"progress ") {
		Ok(Some((Event::Progress(rest.to_vec()), next)))
	} else if line == b"progress" {
		Ok(Some((Event::Progress(Vec::new()), next)))
	} else if line == b"checkpoint" {
		Ok(Some((Event::Checkpoint, next)))
	} else if line == b"done" {
		Ok(Some((Event::Done, next)))
	} else if let Some(rest) = after(line, b"feature ") {
		let event = res!(parse_feature(rest));
		Ok(Some((event, next)))
	} else if let Some(rest) = after(line, b"option ") {
		Ok(Some((Event::Opt(rest.to_vec()), next)))
	} else if line.starts_with(b"ls ")
		|| line.starts_with(b"cat-blob ")
		|| line.starts_with(b"get-mark ")
	{
		Err(err!(
			"Command line \"{}\" asks the consumer a question. Such commands \
			exist for a frontend driving fast-import interactively; a stream \
			from git fast-export never contains one, and this parser answers \
			nothing.", show(line);
		Decode, Input, NoImpl))
	} else {
		Err(err!(
			"Command line \"{}\" is not a fast-import command this parser \
			knows.", show(line);
		Decode, Input, Invalid))
	}
}

fn after<'a>(line: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
	if line.starts_with(prefix) {
		Some(&line[prefix.len()..])
	} else {
		None
	}
}

/// The `blob` line has already been consumed.
fn parse_blob(src: &[u8], mut p: usize, eof: bool)
	-> Outcome<Option<(Event, usize)>>
{
	let mut mark = None;
	let mut original_oid = None;
	loop {
		let (line, next) = match res!(read_line(src, p, eof)) {
			Line::Got(line, next)	=> (line, next),
			Line::End		=> return Err(err!(
				"A blob command ends before its data.";
			Decode, Input, Missing)),
			Line::Need		=> return Ok(None),
		};
		if let Some(rest) = after(line, b"mark ") {
			mark = Some(res!(parse_mark(rest)));
			p = next;
		} else if let Some(rest) = after(line, b"original-oid ") {
			original_oid = Some(res!(parse_ascii(rest, "original-oid")));
			p = next;
		} else if line.starts_with(b"data") {
			break;
		} else {
			return Err(err!(
				"Line \"{}\" belongs to a blob command, where only mark, \
				original-oid and data are allowed.", show(line);
			Decode, Input, Invalid));
		}
	}
	let (data, p) = match res!(take_data(src, p, eof)) {
		Some(pair) => pair,
		None => return Ok(None),
	};
	Ok(Some((Event::Blob(Blob { mark, original_oid, data }), p)))
}

/// The `commit` line has already been consumed.
fn parse_commit(src: &[u8], mut p: usize, refname: &[u8], eof: bool)
	-> Outcome<Option<(Event, usize)>>
{
	let refname = res!(parse_ascii(refname, "commit reference"));
	let mut mark = None;
	let mut original_oid = None;
	let mut author = None;
	let mut committer = None;
	let mut encoding = None;
	let mut gpgsig = None;
	// The header runs to the message, which is the one part every commit has.
	loop {
		let (line, next) = match res!(read_line(src, p, eof)) {
			Line::Got(line, next)	=> (line, next),
			Line::End		=> return Err(err!(
				"Commit on {} ends before its message.", refname;
			Decode, Input, Missing)),
			Line::Need		=> return Ok(None),
		};
		if let Some(rest) = after(line, b"mark ") {
			mark = Some(res!(parse_mark(rest)));
			p = next;
		} else if let Some(rest) = after(line, b"original-oid ") {
			original_oid = Some(res!(parse_ascii(rest, "original-oid")));
			p = next;
		} else if let Some(rest) = after(line, b"author ") {
			author = Some(res!(parse_person(rest, "author")));
			p = next;
		} else if let Some(rest) = after(line, b"committer ") {
			committer = Some(res!(parse_person(rest, "committer")));
			p = next;
		} else if let Some(rest) = after(line, b"encoding ") {
			encoding = Some(rest.to_vec());
			p = next;
		} else if let Some(rest) = after(line, b"gpgsig ") {
			let (algo, format) = res!(parse_gpgsig_head(rest));
			let (sig, after_sig) = match res!(take_data(src, next, eof)) {
				Some(pair) => pair,
				None => return Ok(None),
			};
			gpgsig = Some(GpgSig { algo, format, sig });
			p = after_sig;
		} else if line.starts_with(b"data") {
			break;
		} else {
			return Err(err!(
				"Line \"{}\" appears in the header of the commit on {}, where \
				it is not a header this parser knows.", show(line), refname;
			Decode, Input, Invalid));
		}
	}
	let committer = match committer {
		Some(person) => person,
		None => return Err(err!(
			"Commit on {} has no committer line, which the format requires.",
			refname;
		Decode, Input, Missing)),
	};
	let (message, mut p) = match res!(take_data(src, p, eof)) {
		Some(pair) => pair,
		None => return Ok(None),
	};

	// Parents.
	let mut from = None;
	let mut merges = Vec::new();
	loop {
		let (line, next) = match res!(read_line(src, p, eof)) {
			Line::Got(line, next)	=> (line, next),
			Line::End		=> break,
			Line::Need		=> return Ok(None),
		};
		if let Some(rest) = after(line, b"from ") {
			if from.is_some() {
				return Err(err!(
					"Commit on {} carries a second from line, \"{}\".",
					refname, show(line);
				Decode, Input, Duplicate));
			}
			from = Some(res!(parse_objref(rest, "from")));
			p = next;
		} else if let Some(rest) = after(line, b"merge ") {
			merges.push(res!(parse_objref(rest, "merge")));
			p = next;
		} else {
			break;
		}
	}

	// Changes to the tree.
	let mut changes = Vec::new();
	loop {
		let (line, next) = match res!(read_line(src, p, eof)) {
			Line::Got(line, next)	=> (line, next),
			Line::End		=> break,
			Line::Need		=> return Ok(None),
		};
		match res!(parse_file_change(src, line, next, eof)) {
			Some(Some((change, after_change))) => {
				changes.push(change);
				p = after_change;
			},
			// Not a file command: the commit ends here, and the line stays.
			Some(None) => break,
			// A file command whose inline data has not all arrived.
			None => return Ok(None),
		}
	}

	Ok(Some((Event::Commit(Commit {
		refname,
		mark,
		original_oid,
		author,
		committer,
		encoding,
		gpgsig,
		message,
		from,
		merges,
		changes,
	}), p)))
}

/// The outer `Option` is `None` when more input is needed. The inner `Option`
/// is `None` when the line is not a file command at all, which is how a commit
/// ends.
fn parse_file_change<'a>(src: &'a [u8], line: &'a [u8], next: usize, eof: bool)
	-> Outcome<Option<Option<(FileChange, usize)>>>
{
	if let Some(rest) = after(line, b"M ") {
		let (mode_bytes, at) = res!(take_token(rest, 0, "file mode"));
		let mode = res!(FileMode::from_bytes(mode_bytes));
		let (dataref, at) = res!(take_token(rest, at, "file data reference"));
		let (path, _) = res!(take_path(rest, at, true));
		if dataref == b"inline" {
			let (bytes, after_data) = match res!(take_data(src, next, eof)) {
				Some(pair) => pair,
				None => return Ok(None),
			};
			Ok(Some(Some((
				FileChange::Modify { mode, data: BlobRef::Inline(bytes), path },
				after_data,
			))))
		} else {
			let data = res!(parse_blobref(dataref));
			Ok(Some(Some((FileChange::Modify { mode, data, path }, next))))
		}
	} else if let Some(rest) = after(line, b"D ") {
		let (path, _) = res!(take_path(rest, 0, true));
		Ok(Some(Some((FileChange::Delete { path }, next))))
	} else if let Some(rest) = after(line, b"C ") {
		let (src_path, at) = res!(take_path(rest, 0, false));
		let (dst_path, _) = res!(take_path(rest, at, true));
		Ok(Some(Some((FileChange::Copy { src: src_path, dst: dst_path }, next))))
	} else if let Some(rest) = after(line, b"R ") {
		let (src_path, at) = res!(take_path(rest, 0, false));
		let (dst_path, _) = res!(take_path(rest, at, true));
		Ok(Some(Some((FileChange::Rename { src: src_path, dst: dst_path }, next))))
	} else if line == b"deleteall" {
		Ok(Some(Some((FileChange::DeleteAll, next))))
	} else if let Some(rest) = after(line, b"N ") {
		let (dataref, at) = res!(take_token(rest, 0, "note data reference"));
		let commit = res!(parse_objref(&rest[at..], "note commit"));
		if dataref == b"inline" {
			let (bytes, after_data) = match res!(take_data(src, next, eof)) {
				Some(pair) => pair,
				None => return Ok(None),
			};
			Ok(Some(Some((
				FileChange::Note { data: BlobRef::Inline(bytes), commit },
				after_data,
			))))
		} else {
			let data = res!(parse_blobref(dataref));
			Ok(Some(Some((FileChange::Note { data, commit }, next))))
		}
	} else {
		Ok(Some(None))
	}
}

/// The `tag` line has already been consumed.
fn parse_tag(src: &[u8], mut p: usize, name: &[u8], eof: bool)
	-> Outcome<Option<(Event, usize)>>
{
	let name = res!(parse_ascii(name, "tag name"));
	let mut mark = None;
	let mut original_oid = None;
	let mut from = None;
	let mut tagger = None;
	loop {
		let (line, next) = match res!(read_line(src, p, eof)) {
			Line::Got(line, next)	=> (line, next),
			Line::End		=> return Err(err!(
				"Tag {} ends before its message.", name;
			Decode, Input, Missing)),
			Line::Need		=> return Ok(None),
		};
		if let Some(rest) = after(line, b"mark ") {
			mark = Some(res!(parse_mark(rest)));
			p = next;
		} else if let Some(rest) = after(line, b"original-oid ") {
			original_oid = Some(res!(parse_ascii(rest, "original-oid")));
			p = next;
		} else if let Some(rest) = after(line, b"from ") {
			from = Some(res!(parse_objref(rest, "from")));
			p = next;
		} else if let Some(rest) = after(line, b"tagger ") {
			tagger = Some(res!(parse_person(rest, "tagger")));
			p = next;
		} else if line.starts_with(b"data") {
			break;
		} else {
			return Err(err!(
				"Line \"{}\" appears in tag {}, where it is not a header this \
				parser knows.", show(line), name;
			Decode, Input, Invalid));
		}
	}
	let from = match from {
		Some(objref) => objref,
		None => return Err(err!(
			"Tag {} has no from line, so it names nothing.", name;
		Decode, Input, Missing)),
	};
	let (message, p) = match res!(take_data(src, p, eof)) {
		Some(pair) => pair,
		None => return Ok(None),
	};
	Ok(Some((Event::Tag(Tag {
		name,
		mark,
		original_oid,
		from,
		tagger,
		message,
	}), p)))
}

/// The `reset` line has already been consumed.
fn parse_reset(src: &[u8], p: usize, refname: &[u8], eof: bool)
	-> Outcome<Option<(Event, usize)>>
{
	let refname = res!(parse_ascii(refname, "reset reference"));
	match res!(read_line(src, p, eof)) {
		Line::Got(line, next) => {
			match after(line, b"from ") {
				Some(rest) => {
					let from = Some(res!(parse_objref(rest, "from")));
					Ok(Some((Event::Reset { refname, from }, next)))
				},
				None => Ok(Some((Event::Reset { refname, from: None }, p))),
			}
		},
		Line::End	=> Ok(Some((Event::Reset { refname, from: None }, p))),
		Line::Need	=> Ok(None),
	}
}

/// The `alias` line has already been consumed.
fn parse_alias(src: &[u8], p: usize, eof: bool)
	-> Outcome<Option<(Event, usize)>>
{
	let (mark_line, p) = match res!(read_line(src, p, eof)) {
		Line::Got(line, next)	=> (line, next),
		Line::End		=> return Err(err!(
			"An alias command ends before its mark.";
		Decode, Input, Missing)),
		Line::Need		=> return Ok(None),
	};
	let mark = match after(mark_line, b"mark ") {
		Some(rest) => res!(parse_mark(rest)),
		None => return Err(err!(
			"An alias command must be followed by a mark line, not \"{}\".",
			show(mark_line);
		Decode, Input, Invalid)),
	};
	let (to_line, p) = match res!(read_line(src, p, eof)) {
		Line::Got(line, next)	=> (line, next),
		Line::End		=> return Err(err!(
			"Alias of mark :{} ends before the object it names.", mark;
		Decode, Input, Missing)),
		Line::Need		=> return Ok(None),
	};
	let to = match after(to_line, b"to ") {
		Some(rest) => res!(parse_objref(rest, "alias target")),
		None => return Err(err!(
			"An alias mark must be followed by a to line, not \"{}\".",
			show(to_line);
		Decode, Input, Invalid)),
	};
	Ok(Some((Event::Alias { mark, to }, p)))
}

fn parse_feature(rest: &[u8])
	-> Outcome<Event>
{
	let (name, arg) = match rest.iter().position(|b| *b == b'=') {
		Some(i) => (
			res!(parse_ascii(&rest[..i], "feature name")),
			Some(res!(parse_ascii(&rest[i + 1..], "feature argument"))),
		),
		None => (res!(parse_ascii(rest, "feature name")), None),
	};
	// Every date this parser reads is the raw format, so a stream declaring
	// another one has to be refused rather than silently misread.
	if name == "date-format" {
		match arg.as_deref() {
			Some("raw") | Some("raw-permissive") => {},
			other => return Err(err!(
				"The stream declares date-format {}, but only the raw format \
				is understood here.", other.unwrap_or("with no argument");
			Decode, Input, NoImpl)),
		}
	}
	Ok(Event::Feature { name, arg })
}


// ---------------------------------------------------------------------------
// Field parsing.
// ---------------------------------------------------------------------------

/// Reads a `data` payload, in either the counted or the delimited form.
fn take_data(src: &[u8], p: usize, eof: bool)
	-> Outcome<Option<(Vec<u8>, usize)>>
{
	let (line, p) = match res!(read_line(src, p, eof)) {
		Line::Got(line, next)	=> (line, next),
		Line::End		=> return Err(err!(
			"The stream ends where a data command was expected.";
		Decode, Input, Missing)),
		Line::Need		=> return Ok(None),
	};
	let rest = match after(line, b"data ") {
		Some(rest) => rest,
		None => return Err(err!(
			"Line \"{}\" was expected to be a data command.", show(line);
		Decode, Input, Invalid)),
	};
	if let Some(delim) = after(rest, b"<<") {
		take_delimited_data(src, p, delim, eof)
	} else {
		let text = res!(parse_ascii(rest, "data byte count"));
		let count: usize = match text.parse() {
			Ok(n) => n,
			Err(_) => return Err(err!(
				"Data byte count \"{}\" is not a number.", text;
			Decode, Input, Invalid)),
		};
		if src.len() - p < count {
			return Ok(None);
		}
		let payload = src[p..p + count].to_vec();
		match skip_optional_lf(src, p + count, eof) {
			Some(end)	=> Ok(Some((payload, end))),
			None		=> Ok(None),
		}
	}
}

/// Reads a delimited `data` payload, the terminator being a line holding
/// `delim` and nothing else.
fn take_delimited_data(src: &[u8], p: usize, delim: &[u8], eof: bool)
	-> Outcome<Option<(Vec<u8>, usize)>>
{
	if delim.is_empty() {
		return Err(err!(
			"A delimited data command gives an empty delimiter, which no line \
			can be distinguished by.";
		Decode, Input, Invalid));
	}
	let mut at = p;
	loop {
		let (line, next) = match res!(read_line(src, at, eof)) {
			Line::Got(line, next)	=> (line, next),
			Line::End		=> return Err(err!(
				"A delimited data command is never closed by its delimiter \
				\"{}\".", show(delim);
			Decode, Input, Missing)),
			Line::Need		=> return Ok(None),
		};
		if line == delim {
			// The line feed before the delimiter closes the payload and is not
			// part of it.
			let mut end = at;
			if end > p {
				end -= 1;
			}
			let payload = src[p..end].to_vec();
			return match skip_optional_lf(src, next, eof) {
				Some(after_lf)	=> Ok(Some((payload, after_lf))),
				None		=> Ok(None),
			};
		}
		at = next;
	}
}

/// Steps over the line feed that may follow a payload, returning the offset
/// after it.
///
/// The format allows one, and git writes one, but it is not part of the
/// payload and it is not required. `None` asks for the one byte needed to tell
/// whether it is there, which is why a payload at the very end of a chunk waits
/// for the next chunk.
fn skip_optional_lf(src: &[u8], p: usize, eof: bool) -> Option<usize> {
	match src.get(p) {
		Some(b'\n')	=> Some(p + 1),
		Some(_)		=> Some(p),
		None		=> if eof { Some(p) } else { None },
	}
}

/// Parses a mark line's argument, `:N`.
fn parse_mark(rest: &[u8])
	-> Outcome<u64>
{
	match after(rest, b":") {
		Some(digits) => {
			let text = res!(parse_ascii(digits, "mark number"));
			match text.parse() {
				Ok(n) => Ok(n),
				Err(_) => Err(err!(
					"Mark number \"{}\" is not a number.", text;
				Decode, Input, Invalid)),
			}
		},
		None => Err(err!(
			"Mark \"{}\" does not begin with a colon.", show(rest);
		Decode, Input, Invalid)),
	}
}

/// Parses an object reference, which is either a mark or a name.
fn parse_objref(rest: &[u8], what: &str)
	-> Outcome<ObjRef>
{
	if rest.is_empty() {
		return Err(err!(
			"The {} names nothing.", what;
		Decode, Input, Missing));
	}
	if rest[0] == b':' {
		Ok(ObjRef::Mark(res!(parse_mark(rest))))
	} else {
		Ok(ObjRef::Name(res!(parse_ascii(rest, what))))
	}
}

/// Parses a file data reference, which is either a mark or a name. The inline
/// form is handled by the caller, which has to read the payload that follows.
fn parse_blobref(token: &[u8])
	-> Outcome<BlobRef>
{
	if token.is_empty() {
		return Err(err!(
			"A file command names no content.";
		Decode, Input, Missing));
	}
	if token[0] == b':' {
		Ok(BlobRef::Mark(res!(parse_mark(token))))
	} else {
		Ok(BlobRef::Name(res!(parse_ascii(token, "file data reference"))))
	}
}

/// Parses the `<algo> <format>` that introduces a commit signature.
fn parse_gpgsig_head(rest: &[u8])
	-> Outcome<(String, String)>
{
	match rest.iter().position(|b| *b == b' ') {
		Some(i) => Ok((
			res!(parse_ascii(&rest[..i], "signature hash algorithm")),
			res!(parse_ascii(&rest[i + 1..], "signature format")),
		)),
		None => Err(err!(
			"A gpgsig line must give a hash algorithm and a format, not \
			\"{}\".", show(rest);
		Decode, Input, Invalid)),
	}
}

/// Parses an identity line: an optional name, an email in angle brackets, and
/// a time.
fn parse_person(rest: &[u8], what: &str)
	-> Outcome<Person>
{
	let open = match rest.iter().position(|b| *b == b'<') {
		Some(i) => i,
		None => return Err(err!(
			"The {} line \"{}\" has no email in angle brackets.",
			what, show(rest);
		Decode, Input, Invalid)),
	};
	let close = match rest[open..].iter().position(|b| *b == b'>') {
		Some(i) => open + i,
		None => return Err(err!(
			"The {} line \"{}\" opens an email but never closes it.",
			what, show(rest);
		Decode, Input, Invalid)),
	};
	// One space separates the name from the email, and belongs to neither.
	let mut name_end = open;
	if name_end > 0 && rest[name_end - 1] == b' ' {
		name_end -= 1;
	}
	let name = rest[..name_end].to_vec();
	let email = rest[open + 1..close].to_vec();
	let tail = &rest[close + 1..];
	let tail = match after(tail, b" ") {
		Some(t) => t,
		None => return Err(err!(
			"The {} line \"{}\" has no time after its email.", what, show(rest);
		Decode, Input, Missing)),
	};
	let when = res!(parse_when(tail, what));
	Ok(Person { name, email, when })
}

/// Parses the raw date format: seconds since the epoch, then an offset.
fn parse_when(tail: &[u8], what: &str)
	-> Outcome<When>
{
	let sp = match tail.iter().position(|b| *b == b' ') {
		Some(i) => i,
		None => return Err(err!(
			"The {} time \"{}\" gives no timezone offset. Only git's raw date \
			format is read here.", what, show(tail);
		Decode, Input, Invalid)),
	};
	let secs_text = res!(parse_ascii(&tail[..sp], "timestamp"));
	let secs: i64 = match secs_text.parse() {
		Ok(n) => n,
		Err(_) => return Err(err!(
			"The {} timestamp \"{}\" is not a number of seconds.",
			what, secs_text;
		Decode, Input, Invalid)),
	};
	let tz = res!(parse_tz(&tail[sp + 1..], what));
	Ok(When { secs, tz })
}

/// Parses a timezone offset written as a sign and four digits.
fn parse_tz(bytes: &[u8], what: &str)
	-> Outcome<TzOffset>
{
	let bad = || err!(
		"The {} timezone offset \"{}\" is not a sign followed by four digits.",
		what, show(bytes);
	Decode, Input, Invalid);
	if bytes.len() != 5 {
		return Err(bad());
	}
	let neg = match bytes[0] {
		b'+' => false,
		b'-' => true,
		_ => return Err(bad()),
	};
	if !bytes[1..].iter().all(|b| b.is_ascii_digit()) {
		return Err(bad());
	}
	let hours = i32::from(bytes[1] - b'0') * 10 + i32::from(bytes[2] - b'0');
	let mins = i32::from(bytes[3] - b'0') * 10 + i32::from(bytes[4] - b'0');
	let total = hours * 60 + mins;
	Ok(TzOffset { mins: if neg { -total } else { total }, neg })
}

fn parse_ascii(bytes: &[u8], what: &str)
	-> Outcome<String>
{
	match std::str::from_utf8(bytes) {
		Ok(s) => Ok(s.to_string()),
		Err(_) => Err(err!(
			"The {} \"{}\" is not valid UTF-8.", what, show(bytes);
		Decode, Input, Invalid)),
	}
}

/// Takes the next space separated token, returning it and the offset just past
/// the separating space.
fn take_token<'a>(line: &'a [u8], pos: usize, what: &str)
	-> Outcome<(&'a [u8], usize)>
{
	match line[pos..].iter().position(|b| *b == b' ') {
		Some(i) => Ok((&line[pos..pos + i], pos + i + 1)),
		None => Err(err!(
			"Line \"{}\" ends where the {} and what follows it were expected.",
			show(line), what;
		Decode, Input, Missing)),
	}
}


// ---------------------------------------------------------------------------
// Path quoting.
// ---------------------------------------------------------------------------

/// Reads one path from `line`, unquoting it if it is quoted.
///
/// An unquoted path runs to the next space, or to the end of the line when it
/// is the last on the line. A quoted path runs to its closing quote whatever
/// it contains.
fn take_path(line: &[u8], pos: usize, last: bool)
	-> Outcome<(Vec<u8>, usize)>
{
	if pos > line.len() {
		return Err(err!(
			"Line \"{}\" ends where a path was expected.", show(line);
		Decode, Input, Missing));
	}
	if line.get(pos) == Some(&b'"') {
		return unquote_path(line, pos);
	}
	match line[pos..].iter().position(|b| *b == b' ') {
		Some(i) if !last	=> Ok((line[pos..pos + i].to_vec(), pos + i + 1)),
		_ if last		=> Ok((line[pos..].to_vec(), line.len())),
		_ => Err(err!(
			"Line \"{}\" gives one path where two were expected. An unquoted \
			path cannot contain a space, so a path that does must be quoted.",
			show(line);
		Decode, Input, Missing)),
	}
}

/// Unquotes a C-style quoted path beginning at `pos`, which is its opening
/// quote.
fn unquote_path(line: &[u8], pos: usize)
	-> Outcome<(Vec<u8>, usize)>
{
	let mut out = Vec::new();
	let mut i = pos + 1;
	while i < line.len() {
		match line[i] {
			b'"' => {
				// A space after the closing quote separates this path from the
				// next, and belongs to neither.
				let mut next = i + 1;
				if line.get(next) == Some(&b' ') {
					next += 1;
				}
				return Ok((out, next));
			},
			b'\\' => {
				if i + 1 >= line.len() {
					return Err(err!(
						"Quoted path \"{}\" ends in a backslash.", show(line);
					Decode, Input, Invalid));
				}
				let esc = line[i + 1];
				i += 2;
				match esc {
					b'a'	=> out.push(0x07),
					b'b'	=> out.push(0x08),
					b'f'	=> out.push(0x0c),
					b'n'	=> out.push(b'\n'),
					b'r'	=> out.push(b'\r'),
					b't'	=> out.push(b'\t'),
					b'v'	=> out.push(0x0b),
					b'"'	=> out.push(b'"'),
					b'\\'	=> out.push(b'\\'),
					b'0'..=b'7' => {
						// Up to three octal digits, the first already read.
						let mut value = u32::from(esc - b'0');
						let mut digits = 1;
						while digits < 3 {
							match line.get(i) {
								Some(d @ b'0'..=b'7') => {
									value = value * 8 + u32::from(*d - b'0');
									i += 1;
									digits += 1;
								},
								_ => break,
							}
						}
						if value > 0xff {
							return Err(err!(
								"Quoted path \"{}\" contains the octal escape \
								\\{:o}, which is not a byte.", show(line), value;
							Decode, Input, Range));
						}
						out.push(value as u8);
					},
					other => return Err(err!(
						"Quoted path \"{}\" contains the escape \\{}, which \
						the format does not define.",
						show(line), show(&[other]);
					Decode, Input, Invalid)),
				}
			},
			byte => {
				out.push(byte);
				i += 1;
			},
		}
	}
	Err(err!(
		"Quoted path \"{}\" is never closed.", show(line);
	Decode, Input, Missing))
}


#[cfg(test)]
mod test {
	use super::*;

	/// This is where a boundary bug shows: a parser that peeks past what it has
	/// been given behaves differently when the bytes trickle in.
	fn parse_both_ways(stream: &[u8])
		-> Outcome<Vec<Event>>
	{
		let whole = res!(parse_all(stream));

		let mut parser = Parser::new();
		let mut trickled = Vec::new();
		for byte in stream {
			parser.feed(&[*byte]);
			loop {
				match res!(parser.next_event()) {
					Some(event) => trickled.push(event),
					None => break,
				}
			}
		}
		parser.end();
		while let Some(event) = res!(parser.next_event()) {
			trickled.push(event);
		}

		// And once more in chunks of seven, an unhelpful size on purpose.
		let mut parser = Parser::new();
		let mut chunked = Vec::new();
		for chunk in stream.chunks(7) {
			parser.feed(chunk);
			loop {
				match res!(parser.next_event()) {
					Some(event) => chunked.push(event),
					None => break,
				}
			}
		}
		parser.end();
		while let Some(event) = res!(parser.next_event()) {
			chunked.push(event);
		}

		assert_eq!(whole, trickled, "byte at a time differed from all at once");
		assert_eq!(whole, chunked, "seven byte chunks differed from all at once");
		Ok(whole)
	}

	fn ada() -> Person {
		Person {
			name:	b"Ada Lovelace".to_vec(),
			email:	b"ada@example.org".to_vec(),
			when:	When { secs: 1700000000, tz: TzOffset { mins: 600, neg: false } },
		}
	}

	#[test]
	fn blob_with_mark() -> Outcome<()> {
		let events = res!(parse_both_ways(b"blob\nmark :1\ndata 5\nhello\n"));
		assert_eq!(events, vec![Event::Blob(Blob {
			mark:		Some(1),
			original_oid:	None,
			data:		b"hello".to_vec(),
		})]);
		Ok(())
	}

	/// A blob payload holding NUL bytes and bare line feeds survives whole, and
	/// is never mistaken for stream syntax.
	#[test]
	fn blob_payload_is_binary_safe() -> Outcome<()> {
		let payload: &[u8] = b"\x00\x01\ndata 99\nblob\n\x00commit refs/heads/x\n";
		let mut stream = Vec::new();
		stream.extend_from_slice(b"blob\nmark :7\noriginal-oid ");
		stream.extend_from_slice(b"1234567890abcdef1234567890abcdef12345678\n");
		stream.extend_from_slice(fmt!("data {}\n", payload.len()).as_bytes());
		stream.extend_from_slice(payload);
		stream.extend_from_slice(b"\n");
		let events = res!(parse_both_ways(&stream));
		assert_eq!(events, vec![Event::Blob(Blob {
			mark:		Some(7),
			original_oid:	Some(fmt!("1234567890abcdef1234567890abcdef12345678")),
			data:		payload.to_vec(),
		})]);
		Ok(())
	}

	#[test]
	fn empty_payload() -> Outcome<()> {
		let events = res!(parse_both_ways(b"blob\ndata 0\n"));
		assert_eq!(events, vec![Event::Blob(Blob {
			mark:		None,
			original_oid:	None,
			data:		Vec::new(),
		})]);
		Ok(())
	}

	/// A payload with no line feed of its own after it still parses, since the
	/// trailing line feed is optional.
	#[test]
	fn payload_without_trailing_lf() -> Outcome<()> {
		let events = res!(parse_both_ways(b"blob\ndata 2\nhi"));
		assert_eq!(events, vec![Event::Blob(Blob {
			mark:		None,
			original_oid:	None,
			data:		b"hi".to_vec(),
		})]);
		Ok(())
	}

	#[test]
	fn commit_with_changes() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			mark :3\n\
			author Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			committer Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			data 13\n\
			first commit\n\
			M 100644 :1 a.txt\n\
			M 100755 :2 run.sh\n\
			D gone.txt\n\
			\n";
		let events = res!(parse_both_ways(stream));
		assert_eq!(events, vec![Event::Commit(Commit {
			refname:	fmt!("refs/heads/main"),
			mark:		Some(3),
			original_oid:	None,
			author:		Some(ada()),
			committer:	ada(),
			encoding:	None,
			gpgsig:		None,
			message:	b"first commit\n".to_vec(),
			from:		None,
			merges:		Vec::new(),
			changes:	vec![
				FileChange::Modify {
					mode:	FileMode::Normal,
					data:	BlobRef::Mark(1),
					path:	b"a.txt".to_vec(),
				},
				FileChange::Modify {
					mode:	FileMode::Executable,
					data:	BlobRef::Mark(2),
					path:	b"run.sh".to_vec(),
				},
				FileChange::Delete { path: b"gone.txt".to_vec() },
			],
		})]);
		Ok(())
	}

	/// A merge commit carries a first parent and every other parent in order.
	#[test]
	fn merge_commit_has_two_parents() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			mark :9\n\
			committer Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			data 11\n\
			merge side\n\
			from :4\n\
			merge :6\n\
			merge 0123456789abcdef0123456789abcdef01234567\n\
			\n";
		let events = res!(parse_both_ways(stream));
		match &events[0] {
			Event::Commit(commit) => {
				assert_eq!(commit.from, Some(ObjRef::Mark(4)));
				assert_eq!(commit.merges, vec![
					ObjRef::Mark(6),
					ObjRef::Name(fmt!("0123456789abcdef0123456789abcdef01234567")),
				]);
				assert!(commit.author.is_none());
			},
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	/// A copy, a rename and a deleteall all parse, and the deleteall keeps its
	/// place in the order.
	#[test]
	fn copy_rename_and_deleteall() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			committer A <a@b> 0 +0000\n\
			data 0\n\
			deleteall\n\
			C src.txt copy.txt\n\
			R old.txt new.txt\n\
			\n";
		let events = res!(parse_both_ways(stream));
		match &events[0] {
			Event::Commit(commit) => assert_eq!(commit.changes, vec![
				FileChange::DeleteAll,
				FileChange::Copy {
					src: b"src.txt".to_vec(),
					dst: b"copy.txt".to_vec(),
				},
				FileChange::Rename {
					src: b"old.txt".to_vec(),
					dst: b"new.txt".to_vec(),
				},
			]),
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn inline_file_content() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			committer A <a@b> 0 +0000\n\
			data 0\n\
			M 100644 inline hello.txt\n\
			data 6\n\
			world\n\
			M 120000 inline link\n\
			data 7\n\
			target\n\
			\n";
		let events = res!(parse_both_ways(stream));
		match &events[0] {
			Event::Commit(commit) => assert_eq!(commit.changes, vec![
				FileChange::Modify {
					mode:	FileMode::Normal,
					data:	BlobRef::Inline(b"world\n".to_vec()),
					path:	b"hello.txt".to_vec(),
				},
				FileChange::Modify {
					mode:	FileMode::Symlink,
					data:	BlobRef::Inline(b"target\n".to_vec()),
					path:	b"link".to_vec(),
				},
			]),
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn quoted_paths_unquote() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			committer A <a@b> 0 +0000\n\
			data 0\n\
			M 100644 :1 \"has space.txt\"\n\
			M 100644 :1 \"new\\nline\"\n\
			M 100644 :1 \"tab\\there\"\n\
			M 100644 :1 \"back\\\\slash\"\n\
			M 100644 :1 \"quote\\\"mark\"\n\
			M 100644 :1 \"bell\\a\\b\\f\\r\\v\"\n\
			M 100644 :1 \"octal\\303\\251\"\n\
			M 100644 :1 \"short\\7octal\"\n\
			D \"deleted \\303\\266.txt\"\n\
			R \"old name.txt\" \"new name.txt\"\n\
			C \"a b\" plain.txt\n\
			\n";
		let events = res!(parse_both_ways(stream));
		let want: Vec<FileChange> = vec![
			b"has space.txt".to_vec(),
			b"new\nline".to_vec(),
			b"tab\there".to_vec(),
			b"back\\slash".to_vec(),
			b"quote\"mark".to_vec(),
			b"bell\x07\x08\x0c\r\x0b".to_vec(),
			b"octal\xc3\xa9".to_vec(),
			b"short\x07octal".to_vec(),
		].into_iter().map(|path| FileChange::Modify {
			mode:	FileMode::Normal,
			data:	BlobRef::Mark(1),
			path,
		}).collect();
		match &events[0] {
			Event::Commit(commit) => {
				assert_eq!(&commit.changes[..8], &want[..]);
				assert_eq!(commit.changes[8], FileChange::Delete {
					path: b"deleted \xc3\xb6.txt".to_vec(),
				});
				assert_eq!(commit.changes[9], FileChange::Rename {
					src: b"old name.txt".to_vec(),
					dst: b"new name.txt".to_vec(),
				});
				assert_eq!(commit.changes[10], FileChange::Copy {
					src: b"a b".to_vec(),
					dst: b"plain.txt".to_vec(),
				});
			},
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	/// A path that is last on its line runs to the end of the line, spaces and
	/// all. Git quotes such a path, but the format allows it unquoted and
	/// fast-import reads it, so this parser does too.
	#[test]
	fn unquoted_path_may_hold_spaces() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			committer A <a@b> 0 +0000\n\
			data 0\n\
			M 100644 :1 plain path.txt\n\
			D another path.txt\n\
			\n";
		let events = res!(parse_both_ways(stream));
		match &events[0] {
			Event::Commit(commit) => assert_eq!(commit.changes, vec![
				FileChange::Modify {
					mode:	FileMode::Normal,
					data:	BlobRef::Mark(1),
					path:	b"plain path.txt".to_vec(),
				},
				FileChange::Delete { path: b"another path.txt".to_vec() },
			]),
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn unknown_escape_is_refused() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			committer A <a@b> 0 +0000\n\
			data 0\n\
			M 100644 :1 \"what\\qnow\"\n\
			\n";
		assert!(parse_all(stream).is_err());
		Ok(())
	}

	/// A delimited payload runs to a line holding just the delimiter, and that
	/// line's own content may appear inside the payload without ending it.
	#[test]
	fn delimited_data() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			committer A <a@b> 0 +0000\n\
			data <<END\n\
			a message\n\
			with ENDish lines\n\
			END\n\
			\n";
		let events = res!(parse_both_ways(stream));
		match &events[0] {
			Event::Commit(commit) => assert_eq!(
				commit.message,
				b"a message\nwith ENDish lines".to_vec(),
			),
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn delimited_data_empty() -> Outcome<()> {
		let events = res!(parse_both_ways(b"blob\ndata <<EOF\nEOF\n"));
		assert_eq!(events, vec![Event::Blob(Blob {
			mark:		None,
			original_oid:	None,
			data:		Vec::new(),
		})]);
		Ok(())
	}

	#[test]
	fn tag_parses() -> Outcome<()> {
		let stream: &[u8] = b"tag v1\n\
			mark :12\n\
			from :7\n\
			tagger Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			data 12\n\
			release one\n\
			\n";
		let events = res!(parse_both_ways(stream));
		assert_eq!(events, vec![Event::Tag(Tag {
			name:		fmt!("v1"),
			mark:		Some(12),
			original_oid:	None,
			from:		ObjRef::Mark(7),
			tagger:		Some(ada()),
			message:	b"release one\n".to_vec(),
		})]);
		Ok(())
	}

	/// A reset parses with and without a from line, and one without does not
	/// swallow the command that follows.
	#[test]
	fn reset_with_and_without_from() -> Outcome<()> {
		let stream: &[u8] = b"reset refs/heads/side\n\
			from :3\n\
			reset refs/heads/other\n\
			blob\n\
			data 1\n\
			x\n";
		let events = res!(parse_both_ways(stream));
		assert_eq!(events.len(), 3);
		assert_eq!(events[0], Event::Reset {
			refname:	fmt!("refs/heads/side"),
			from:		Some(ObjRef::Mark(3)),
		});
		assert_eq!(events[1], Event::Reset {
			refname:	fmt!("refs/heads/other"),
			from:		None,
		});
		assert_eq!(events[2], Event::Blob(Blob {
			mark:		None,
			original_oid:	None,
			data:		b"x".to_vec(),
		}));
		Ok(())
	}

	/// The commands that carry no repository content each parse.
	#[test]
	fn housekeeping_commands() -> Outcome<()> {
		let stream: &[u8] = b"feature done\n\
			feature date-format=raw\n\
			option quiet\n\
			progress 3 of 9 objects\n\
			checkpoint\n\
			alias\n\
			mark :5\n\
			to :4\n\
			done\n";
		let events = res!(parse_both_ways(stream));
		assert_eq!(events, vec![
			Event::Feature { name: fmt!("done"), arg: None },
			Event::Feature { name: fmt!("date-format"), arg: Some(fmt!("raw")) },
			Event::Opt(b"quiet".to_vec()),
			Event::Progress(b"3 of 9 objects".to_vec()),
			Event::Checkpoint,
			Event::Alias { mark: 5, to: ObjRef::Mark(4) },
			Event::Done,
		]);
		Ok(())
	}

	/// A date format other than raw is refused, since reading it as raw would
	/// silently misdate every commit.
	#[test]
	fn foreign_date_format_is_refused() -> Outcome<()> {
		assert!(parse_all(b"feature date-format=rfc2822\n").is_err());
		Ok(())
	}

	#[test]
	fn file_modes() -> Outcome<()> {
		let cases: [(&[u8], FileMode); 8] = [
			(b"100644",	FileMode::Normal),
			(b"644",	FileMode::Normal),
			(b"100755",	FileMode::Executable),
			(b"755",	FileMode::Executable),
			(b"120000",	FileMode::Symlink),
			(b"160000",	FileMode::Gitlink),
			(b"040000",	FileMode::Subdirectory),
			(b"40000",	FileMode::Subdirectory),
		];
		for (bytes, want) in cases {
			assert_eq!(res!(FileMode::from_bytes(bytes)), want);
		}
		assert!(FileMode::from_bytes(b"100664").is_err());
		Ok(())
	}

	/// Three of git's five modes name something the vocabulary can hold, and the
	/// other two name something that is not a file with bytes in it.
	#[test]
	fn modes_that_name_a_file() -> Outcome<()> {
		assert_eq!(FileMode::Normal.as_op_mode(), Some(Mode::Normal));
		assert_eq!(FileMode::Executable.as_op_mode(), Some(Mode::Executable));
		assert_eq!(FileMode::Symlink.as_op_mode(), Some(Mode::Symlink));
		assert_eq!(FileMode::Gitlink.as_op_mode(), None,
			"a submodule is another repository, not this one's file");
		assert_eq!(FileMode::Subdirectory.as_op_mode(), None,
			"a tree entry is not content");
		Ok(())
	}

	/// An identity line with no name, and one with an offset west of UTC,
	/// both parse.
	#[test]
	fn identity_lines() -> Outcome<()> {
		let person = res!(parse_person(b"<nobody@example.org> 1700000200 -0530", "author"));
		assert_eq!(person.name, Vec::<u8>::new());
		assert_eq!(person.email, b"nobody@example.org".to_vec());
		assert_eq!(person.when.secs, 1700000200);
		assert_eq!(person.when.tz.mins, -330);
		assert_eq!(fmt!("{}", person.when), "1700000200 -0530");
		Ok(())
	}

	/// The unknown timezone `-0000` keeps its sign, which is the only thing
	/// that distinguishes it from `+0000`.
	#[test]
	fn unknown_timezone_keeps_its_sign() -> Outcome<()> {
		let west = res!(parse_person(b"A <a@b> 0 -0000", "author"));
		let east = res!(parse_person(b"A <a@b> 0 +0000", "author"));
		assert_eq!(west.when.tz.mins, 0);
		assert_eq!(east.when.tz.mins, 0);
		assert_ne!(west.when.tz, east.when.tz);
		assert_eq!(fmt!("{}", west.when.tz), "-0000");
		assert_eq!(fmt!("{}", east.when.tz), "+0000");
		Ok(())
	}

	#[test]
	fn non_utf8_name_survives() -> Outcome<()> {
		let mut line = Vec::new();
		line.extend_from_slice(b"Rene\xe9 <r@example.org> 0 +0000");
		let person = res!(parse_person(&line, "author"));
		assert_eq!(person.name, b"Rene\xe9".to_vec());
		assert_eq!(person.name_lossy(), "Rene\u{fffd}");
		Ok(())
	}

	#[test]
	fn gpgsig_is_carried() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/heads/main\n\
			committer A <a@b> 0 +0000\n\
			gpgsig sha1 openpgp\n\
			data 20\n\
			-----BEGIN PGP-----\n\
			data 2\n\
			m\n\
			\n";
		let events = res!(parse_both_ways(stream));
		match &events[0] {
			Event::Commit(commit) => {
				assert_eq!(commit.gpgsig, Some(GpgSig {
					algo:	fmt!("sha1"),
					format:	fmt!("openpgp"),
					sig:	b"-----BEGIN PGP-----\n".to_vec(),
				}));
				assert_eq!(commit.message, b"m\n".to_vec());
			},
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn notemodify_parses() -> Outcome<()> {
		let stream: &[u8] = b"commit refs/notes/commits\n\
			committer A <a@b> 0 +0000\n\
			data 0\n\
			N :2 :1\n\
			N inline :3\n\
			data 5\n\
			note\n\
			\n";
		let events = res!(parse_both_ways(stream));
		match &events[0] {
			Event::Commit(commit) => assert_eq!(commit.changes, vec![
				FileChange::Note {
					data:	BlobRef::Mark(2),
					commit:	ObjRef::Mark(1),
				},
				FileChange::Note {
					data:	BlobRef::Inline(b"note\n".to_vec()),
					commit:	ObjRef::Mark(3),
				},
			]),
			other => return Err(err!("Expected a commit, got {:?}.", other; Test)),
		}
		Ok(())
	}

	#[test]
	fn unknown_command_names_the_line() -> Outcome<()> {
		match parse_all(b"blob\ndata 0\nfrobnicate everything\n") {
			Ok(events) => Err(err!(
				"Expected an error, parsed {:?}.", events;
			Test)),
			Err(e) => {
				let text = fmt!("{}", e);
				assert!(
					text.contains("frobnicate everything"),
					"error did not name the line: {}", text,
				);
				Ok(())
			},
		}
	}

	/// The commands a frontend uses to interrogate fast-import are refused by
	/// name, since there is nothing here to answer them.
	#[test]
	fn interrogation_commands_are_refused() -> Outcome<()> {
		for line in [
			&b"ls :1 path\n"[..],
			&b"cat-blob :1\n"[..],
			&b"get-mark :1\n"[..],
		] {
			assert!(parse_all(line).is_err(), "accepted {:?}", line);
		}
		Ok(())
	}

	/// A stream cut off part way through a payload is an error once the caller
	/// says no more is coming, not a short blob.
	#[test]
	fn truncated_stream_is_an_error() -> Outcome<()> {
		assert!(parse_all(b"blob\ndata 10\nshort").is_err());
		assert!(parse_all(b"blob\nmark :1\n").is_err());
		assert!(parse_all(b"commit refs/heads/main\ncommitter A <a@b> 0 +0000\n").is_err());
		Ok(())
	}

	/// A commit with no committer is refused, since the format requires one.
	#[test]
	fn commit_without_committer_is_refused() -> Outcome<()> {
		assert!(parse_all(b"commit refs/heads/main\ndata 0\n").is_err());
		Ok(())
	}

	/// Until the caller declares the end of the stream, an incomplete command
	/// asks for more input rather than failing.
	#[test]
	fn incomplete_command_waits() -> Outcome<()> {
		let mut parser = Parser::new();
		parser.feed(b"blob\ndata 5\nhel");
		assert_eq!(res!(parser.next_event()), None);
		assert!(!parser.is_exhausted());
		parser.feed(b"lo\n");
		assert!(res!(parser.next_event()).is_some());
		parser.end();
		assert_eq!(res!(parser.next_event()), None);
		assert!(parser.is_exhausted());
		Ok(())
	}

	/// A whole stream of every construct parses the same however it is chunked.
	#[test]
	fn whole_stream_round_trip() -> Outcome<()> {
		let mut stream = Vec::new();
		stream.extend_from_slice(b"feature done\n");
		stream.extend_from_slice(b"blob\nmark :1\ndata 6\nhello\n\n");
		stream.extend_from_slice(b"blob\nmark :2\ndata 11\n");
		stream.extend_from_slice(b"\x00\x01\n\x02binary\x00");
		stream.extend_from_slice(b"\n");
		stream.extend_from_slice(b"reset refs/heads/side\n");
		stream.extend_from_slice(
			b"commit refs/heads/side\n\
			mark :3\n\
			author Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			committer Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			data 13\n\
			first commit\n\
			M 100644 :1 a.txt\n\
			M 100644 :2 \"bin \\303\\251.dat\"\n\
			\n");
		stream.extend_from_slice(
			b"commit refs/heads/main\n\
			mark :4\n\
			committer Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			data <<EOM\n\
			rename a\n\
			EOM\n\
			from :3\n\
			R a.txt renamed.txt\n\
			\n");
		stream.extend_from_slice(
			b"tag v1\n\
			from :4\n\
			tagger Ada Lovelace <ada@example.org> 1700000000 +1000\n\
			data 12\n\
			release one\n\
			\n");
		stream.extend_from_slice(b"done\n");
		let events = res!(parse_both_ways(&stream));
		assert_eq!(events.len(), 8);
		match &events[6] {
			Event::Tag(tag) => assert_eq!(tag.name, "v1"),
			other => return Err(err!("Expected a tag, got {:?}.", other; Test)),
		}
		assert_eq!(events[7], Event::Done);
		Ok(())
	}
}
