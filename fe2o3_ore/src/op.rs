//! The operation vocabulary: what a single unit of history can say.
//!
//! History here is a sequence of operations rather than a sequence of
//! snapshots. An operation states an intent -- create this file, put these
//! bytes beside those -- so the intent survives into the record and can be
//! reasoned about later, instead of being inferred back out of a diff.
//!
//! # Nothing names a position
//!
//! Every operation that speaks about bytes speaks about them by name. A splice
//! says which gap its bytes go in, by naming the content either side of that
//! gap, and says which content it removes, by naming that content; a move says
//! the same of the run it relocates. No operation carries a byte offset, a line
//! number or anything else that a concurrent edit could invalidate, which is
//! the property [`crate::seq`] is built on and the reason the two vocabularies
//! are now one.
//!
//! # Nothing names a file either
//!
//! A content operation names content and stops there. [`Op::Splice`] and
//! [`Op::Move`] carry no file, because the file a placement lands in is read off
//! the content it anchors to rather than asserted beside it: an author who
//! recorded a file could contradict the anchor, and the format would have no way
//! to say which of the two was right.
//!
//! What makes that total is the **origin anchor**. [`Op::FileCreate`] mints a
//! file, whose identity is the creating operation's own identity, and with it one
//! byte of content born dead -- [`crate::id::ContentId::origin`] -- so that an
//! empty file is not empty in identifier space. A splice into an empty file
//! anchors after that byte exactly as any other splice anchors after any other
//! byte, and the rule that replaces the file field is
//! [`Op::check_placement`]: an operation that places anything must carry at least
//! one origin, since that origin is what says where it lands.
//!
//! [`Op::FileRename`], [`Op::FileMode`] and [`Op::FileDelete`] name a file by
//! that identity. A path is metadata carried by the lifecycle operations and
//! nothing else, and it is bytes rather than a string, because a path is not
//! required to be UTF-8. A file's mode is metadata of the same kind, asserted by
//! naming the file rather than by naming its bytes, so it survives every edit
//! those bytes go on to have.
//!
//! # Naming content is not only for editing it
//!
//! [`Op::Note`] says something *about* content by naming it, and thereby inherits
//! the whole of the anchoring machinery: the note narrows when the content is
//! edited, travels when the content is moved, and crosses a file boundary when
//! the content does, none of which anything had to be written to make happen. A
//! note is not sequence content -- it mints no bytes and renders none -- so what
//! the render does with it is resolve it into spans; see
//! [`crate::seq::render::Note`].
//!
//! # Some operations are about the history and not about the bytes
//!
//! [`Op::Mark`] names a point in history; [`Op::Proposal`], [`Op::Said`] and
//! [`Op::Settled`] carry an argument about what the bytes ought to become; and
//! [`Op::Reverts`] says what a set of edits was written to undo. None of them
//! mints an atom, claims a byte or renders one, and the sequence keeps them for
//! the reason it keeps a note: the causal graph has to be whole.
//!
//! They are operations rather than records kept beside the repository because a
//! clone that arrives without them arrives without the reasons. A proposal held
//! in a forge's own store is readable only through that forge; a revert that
//! leaves no trace is an unexplained deletion by a stranger when it reaches
//! somebody else's machine, and the author of the work being undone has nothing
//! to read and nothing to be credited by.
//!
//! What is deliberately *not* here is a ballot. An operation is signed by its
//! author, so a vote written into the log would name its voter permanently and
//! irrevocably; tallies are published and voters are not, and that promise
//! cannot be kept by a format that records the votes.
//!
//! # Every operation carries its parents
//!
//! An operation records the frontier its author could see when they wrote it,
//! in [`Header::parents`]. That is what makes the history a graph rather than a
//! list: with it, [`crate::log::OpLog`] can say whether a set is causally
//! complete, and [`crate::seq`] can say whether two operations that touched the
//! same bytes were concurrent or merely consecutive. Parents live on the header
//! and not on the variants, because causality is a property of every operation
//! alike and duplicating it once per variant would let the copies drift.
//!
//! A time is not the same thing and is not on the header. Only the operations
//! that carry one have one, it is the author's own clock rather than a position
//! in the order, and nothing decides anything by it: see [`Op::Mark`].
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::{
	varint_decode,
	varint_encode,
	Anchor,
	ContentId,
	ContentRange,
	OpId,
	Side,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_hash::api::{
	Hash,
	Hasher,
};
use oxedyne_fe2o3_jdat::prelude::*;

use std::sync::Arc;


//// Operation wire codes.
pub const CODE_FILE_CREATE:	u8 = 1;
pub const CODE_FILE_DELETE:	u8 = 2;
pub const CODE_FILE_RENAME:	u8 = 3;
pub const CODE_MARK:		u8 = 4;
pub const CODE_SPLICE:		u8 = 5;
pub const CODE_MOVE:		u8 = 6;
pub const CODE_NOTE:		u8 = 7;
pub const CODE_FILE_MODE:	u8 = 8;
pub const CODE_MARK_TIMED:	u8 = 9;	// a mark carrying a body, a time, or both
pub const CODE_PROPOSAL:	u8 = 10;
pub const CODE_SAID:		u8 = 11;
pub const CODE_SETTLED:		u8 = 12;
pub const CODE_REVERTS:		u8 = 13;


/// The character beginning the name of an [`Op::Mark`] a tool wrote rather than
/// a person, and which a person's mark may not begin with.
///
/// One character is the whole of the convention, and it is here rather than in
/// whichever tool authors the marks because every reader of a history has to
/// apply it and they are not all the same program. A tool that names a point at
/// the end of every command writes a great many marks, and telling them apart by
/// name is what lets a reader be shown the handful somebody chose.
pub const AUTO_MARK_PREFIX: char = '@';

/// Is this the name of a mark a tool wrote rather than one a person chose?
///
/// Takes the name rather than the [`Op`], since a caller reading a reference
/// somebody typed has nothing else to hand it.
///
/// ```
/// use oxedyne_fe2o3_ore::op::{is_auto_mark, Op};
///
/// let op = Op::Mark {
///     name: String::from("@2026-08-17T04:12:09.482913Z"),
///     body: None,
///     time: Some(1_755_403_929),
/// };
/// assert!(match &op {
///     Op::Mark { name, .. }	=> is_auto_mark(name),
///     _						=> false,
/// });
/// assert!(!is_auto_mark("release 1.0"));
/// ```
///
/// It says nothing about whether the name is a datetime, and deliberately so.
/// What a tool spells after the prefix is that tool's business and may differ
/// between them; what every reader has to agree on is which marks are somebody's
/// own, and that is one character.
pub fn is_auto_mark(name: &str) -> bool {
	name.starts_with(AUTO_MARK_PREFIX)
}


/// What a mark's body calls the identity line the commit it was imported from was
/// authored under.
///
/// A trailer in git's own sense -- last line, `Key: value` -- so that a body
/// already ending in `Co-Authored-By:` lines gains it inside the block those lines
/// are in. Here rather than in the importer for the reason [`AUTO_MARK_PREFIX`] is
/// here: an importer writes it, a mirror reads it back to author the commit under
/// the name it arrived with, and a forge reads it to show a person the name and not
/// the bookkeeping. Three programs, one convention, and the day two of them spell
/// it differently is the day the forge and the mirror disagree about what a mark
/// says.
///
/// # What the value is
///
/// Git's whole author line, `Name <email> 1735089438 +0800`: the identity, the
/// moment and the zone offset the author's own clock was reading. Not the identity
/// alone. [`Op::Mark`] carries a time in UTC and no zone, so an offset survives an
/// import only here, and a mirror writing a commit back out reads it from here.
///
/// **A reader showing this to a person shows the name, not the line.** The moment
/// is bookkeeping, and a page that prints the value whole prints a timestamp in the
/// middle of an author's name. Neither function below looks at the shape: the value
/// is opaque bytes to both, so a caller that wants the name splits the last two
/// space-separated fields off itself.
pub const AUTHOR_TRAILER: &str = "Ore-Author: ";

/// Adds the identity line a commit was authored under to the body its mark will
/// carry.
///
/// No blank line before it, so that the trailer joins whatever block ends the body
/// -- which is where git puts its own -- and taking exactly one line back off is
/// unambiguous however the body ended.
pub fn with_author(body: Option<&[u8]>, identity: &[u8]) -> Vec<u8> {
	let mut out = body.unwrap_or_default().to_vec();
	if !out.is_empty() && !out.ends_with(b"\n") {
		out.push(b'\n');
	}
	out.extend_from_slice(AUTHOR_TRAILER.as_bytes());
	out.extend_from_slice(identity);
	out.push(b'\n');
	out
}

/// Splits the identity line back off a mark's body, where it carries one.
///
/// # One line, and never a loop
///
/// A commit message may say anything, including a last line of its own beginning
/// `Ore-Author:`, and [`with_author`] writes the importer's trailer *after*
/// whatever was already there. So the importer's is always the last line and
/// exactly one line comes off. A reader that stripped until no trailer remained
/// would eat the person's line as well and author the commit under the name in it,
/// which is a round trip that silently rewrites history rather than reproducing it.
///
/// # What it cannot decide
///
/// A mark authored in Ore rather than imported, whose body's last line a person
/// typed as `Ore-Author: ...`, is indistinguishable from an imported one and is
/// split. Nothing in a mark says whether it was imported, and adding something
/// would be a discriminator in the history for the benefit of one importer. What a
/// caller loses is one line of a body, which for a mirror is a name it would have
/// derived anyway and for a reader is a line shown as an author instead of as
/// text.
///
/// The identity is bytes and is not checked: git says nothing about the encoding
/// of an identity line, and a caller that has to put it on a page decodes it there.
///
/// ```
/// use oxedyne_fe2o3_ore::op::{with_author, without_author};
///
/// let said = b"Tidy the parser.\n";
/// let line = b"Jason Hoogland <hoogland@gmail.com> 1735089438 +0800";
/// let carried = with_author(Some(said), line);
/// let (body, who) = without_author(&carried);
/// assert_eq!(body, said);
/// assert_eq!(who, Some(&line[..]));
///
/// // A commit message ending in a line of its own that looks like the trailer.
/// // One line comes off, and the person's line survives untouched.
/// let awkward = b"Fix it.\nOre-Author: Somebody Else <else@example.com>\n";
/// let carried = with_author(Some(awkward), b"Jason Hoogland <hoogland@gmail.com>");
/// let (body, who) = without_author(&carried);
/// assert_eq!(body, awkward);
/// assert_eq!(who, Some(&b"Jason Hoogland <hoogland@gmail.com>"[..]));
///
/// // A mark nobody imported carries no trailer and is handed back whole.
/// let (body, who) = without_author(b"Ready to cut.\n");
/// assert_eq!(body, b"Ready to cut.\n");
/// assert_eq!(who, None);
/// ```
pub fn without_author(body: &[u8]) -> (&[u8], Option<&[u8]>) {
	// A body the importer wrote ends in the newline that terminates its trailer, so
	// one that does not end in a newline cannot be carrying one.
	let above = match body.strip_suffix(b"\n") {
		Some(above)	=> above,
		None		=> return (body, None),
	};
	let start = match above.iter().rposition(|b| *b == b'\n') {
		Some(at)	=> at + 1,
		None		=> 0,
	};
	match above[start..].strip_prefix(AUTHOR_TRAILER.as_bytes()) {
		Some(identity)	=> (&body[..start], Some(identity)),
		None			=> (body, None),
	}
}


//// File mode wire codes.
pub const MODE_NORMAL:		u8 = 0;
pub const MODE_EXECUTABLE:	u8 = 1;
pub const MODE_SYMLINK:		u8 = 2;


//// Proposal state wire codes.
pub const SETTLED_OPEN:		u8 = 0;
pub const SETTLED_ACCEPTED:	u8 = 1;
pub const SETTLED_DECLINED:	u8 = 2;
pub const SETTLED_DONE:		u8 = 3;


/// What a file is, over and above the bytes in it.
///
/// A three-value enum and not a number: a mode outside the set is not a state a
/// working copy can be in, and a reader should not have to guess what one would
/// mean. [`Mode::Normal`] is the default, and that is what makes the operation
/// additive -- a file no [`Op::FileMode`] ever named is a normal file, so every
/// history written before the operation existed means today what it meant
/// yesterday.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Mode {
	#[default]
	Normal,
	Executable,
	Symlink,	// whose bytes are the path it points at
}

impl Mode {
	pub const fn code(&self) -> u8 {
		match self {
			Self::Normal		=> MODE_NORMAL,
			Self::Executable	=> MODE_EXECUTABLE,
			Self::Symlink		=> MODE_SYMLINK,
		}
	}

	pub const fn name(&self) -> &'static str {
		match self {
			Self::Normal		=> "normal",
			Self::Executable	=> "executable",
			Self::Symlink		=> "symlink",
		}
	}

	pub const fn is_normal(&self) -> bool {
		matches!(self, Self::Normal)
	}

	pub const fn to_dat(&self) -> Dat {
		Dat::U8(self.code())
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let code = match dat {
			Dat::U8(c) => *c,
			other => return Err(err!(
				"A file mode expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		match code {
			MODE_NORMAL		=> Ok(Self::Normal),
			MODE_EXECUTABLE	=> Ok(Self::Executable),
			MODE_SYMLINK	=> Ok(Self::Symlink),
			other => Err(err!(
				"File mode {} is not one of {} for normal, {} for executable and {} \
				for a symbolic link.",
				other, MODE_NORMAL, MODE_EXECUTABLE, MODE_SYMLINK;
			Decode, Input, Invalid)),
		}
	}
}

impl std::fmt::Display for Mode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.name())
	}
}


/// What became of a proposal. Four states and no workflow.
///
/// There is no transition table beside it, because a state is asserted rather
/// than stepped to: an author says what a proposal now is, and the assertion
/// stands until another one is written. [`Settled::Open`] is the default, so an
/// [`Op::Proposal`] that nothing has settled yet is open without anything having
/// had to say so.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Settled {
	#[default]
	Open,
	Accepted,
	Declined,
	Done,		// agreed to and carried out
}

impl Settled {
	pub const fn code(&self) -> u8 {
		match self {
			Self::Open		=> SETTLED_OPEN,
			Self::Accepted	=> SETTLED_ACCEPTED,
			Self::Declined	=> SETTLED_DECLINED,
			Self::Done		=> SETTLED_DONE,
		}
	}

	pub const fn name(&self) -> &'static str {
		match self {
			Self::Open		=> "open",
			Self::Accepted	=> "accepted",
			Self::Declined	=> "declined",
			Self::Done		=> "done",
		}
	}

	pub const fn is_open(&self) -> bool {
		matches!(self, Self::Open)
	}

	pub const fn to_dat(&self) -> Dat {
		Dat::U8(self.code())
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let code = match dat {
			Dat::U8(c) => *c,
			other => return Err(err!(
				"A settled state expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		match code {
			SETTLED_OPEN		=> Ok(Self::Open),
			SETTLED_ACCEPTED	=> Ok(Self::Accepted),
			SETTLED_DECLINED	=> Ok(Self::Declined),
			SETTLED_DONE		=> Ok(Self::Done),
			other => Err(err!(
				"Settled state {} is not one of {} for open, {} for accepted, {} for \
				declined and {} for done.",
				other, SETTLED_OPEN, SETTLED_ACCEPTED, SETTLED_DECLINED, SETTLED_DONE;
			Decode, Input, Invalid)),
		}
	}
}

impl std::fmt::Display for Settled {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.name())
	}
}


/// The one thing an operation's inverse needs that the operation does not say,
/// and which only the state it was written against holds.
///
/// An operation records an intent and not the state it displaced, so undoing the
/// three that assert a value has to read the value they replaced from somewhere.
/// That somewhere is a render at the operation's parents, which the engine does
/// not have and every caller does. This says which question to ask; asking it is
/// the caller's.
///
/// Rendering the operation set *without* the operation is not the way to answer
/// any of these, and cannot be made to work: removing an operation from the
/// middle of a history leaves anchors naming atoms nothing created, which
/// `Sequence::render_with` refuses rather than guesses at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Prior {
	Path {
		file: OpId,
	},
	Mode {
		file: OpId,
	},
	// One gap per run, in the order the runs are named.
	Place {
		src: Vec<ContentRange>,
	},
}

/// What undoing an operation amounts to, as far as the operation itself can say.
///
/// It is here rather than in whichever tool authors a revert for the reason
/// [`AUTO_MARK_PREFIX`] is here: more than one program will offer to undo an
/// operation, and two tables of what an inverse is would be two tables that
/// could disagree about a history they both write into.
///
/// The three fields are three different kinds of answer, and a caller that
/// serves only the first is a caller that silently half-undoes a splice:
///
/// - [`Undoing::written`] is the part the operation is enough for, exactly.
/// - [`Undoing::copies`] is content the operation killed, which comes back only
///   as a **copy** under a new identity, because nothing here un-buries: the
///   tombstone set is grow-only and is recomputed from the operation set on every
///   render. See [`Op::restoring`].
/// - [`Undoing::prior`] is a question for the state the operation was written
///   against.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Undoing {
	pub written:	Vec<Op>,
	pub copies:		Vec<ContentRange>,	// one restoring splice per run
	pub prior:		Option<Prior>,
}

/// A single unit of history: one whole edit.
///
/// The vocabulary is an enum rather than a trait object, so a reader can
/// enumerate everything history is able to say and the compiler can insist that
/// every consumer handles all of it.
///
/// [`Op::Move`] is why the vocabulary needs more than a splice. A move is not a
/// delete plus an insert: it names the run it relocates by the identity of the
/// bytes themselves, so an edit made concurrently inside that run lands in the
/// run's new home rather than on a tombstone. Nothing in a move says where
/// anything is -- the source is content, the destination is an anchor -- and
/// since a splice says the same, the sequence structure in [`crate::seq`] can
/// resolve any two of them against each other however they happen to arrive, in
/// one file or across two.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op {
	FileCreate {
		path: Vec<u8>,
	},
	// A file's content is held back rather than destroyed, so whatever moved out
	// of it before it went still renders where it went.
	FileDelete {
		file: OpId,
	},
	FileRename {
		file: OpId,
		path: Vec<u8>,
	},
	// A file no such operation names is normal, and two of these written
	// concurrently settle the way two concurrent renames do: the later in
	// operation order is the one the render reports.
	FileMode {
		file: OpId,
		mode: Mode,
	},
	// Two wire spellings of the one variant, chosen by what it carries: see
	// Op::code.
	Mark {
		name: String,
		body: Option<Vec<u8>>,
		time: Option<u64>,				// unix epoch seconds, and it orders nothing
	},
	// The single primitive insertion, deletion and replacement all follow from:
	// an insertion removes nothing, a deletion inserts nothing, a replacement
	// does both at once.
	//
	// One buffer, three owners. `insert` is shared rather than owned outright
	// because the log's record, the sequence's cloned `Applied` and the
	// `crate::seq::atom::Atoms` entry all want the same bytes: three copies cost
	// 7.63 kB of resident memory per operation on a real 44,628-operation
	// history. The FIXED structures come to about 344 bytes of that; the rest is
	// not all content, and saying so was an error in an earlier draft of this
	// comment -- the remainder also holds envelope payloads, the segment buffer
	// read whole, and BTreeMap nodes that split badly under ascending keys.
	// Sharing took the figure to 5.79 kB, so `Sequence::apply_record`'s clone of
	// an operation is a refcount bump and an atom is a handle. The wire form is
	// untouched -- `to_dat` hands `Dat::BU64` the same byte sequence whatever
	// owns it -- which had to be true, since a format that moved here would make
	// every existing store unreadable.
	Splice {
		left: Option<Anchor>,			// binds after a byte
		right: Option<Anchor>,			// binds before a byte
		remove: Vec<ContentRange>,
		insert: Arc<[u8]>,				// shared, so a clone copies no bytes
	},
	Move {
		src: Vec<ContentRange>,			// in the order it lands in
		left: Option<Anchor>,
		right: Option<Anchor>,
	},
	Note {
		on: Vec<ContentRange>,
		text: Vec<u8>,					// not this crate's to decode
	},
	Proposal {
		title: String,
		body: Vec<u8>,
		voice: String,					// the forge name the author wrote under
		time: u64,						// unix epoch seconds
	},
	Said {
		on: OpId,						// the proposal spoken about
		text: Vec<u8>,
		voice: String,
		time: u64,
	},
	// Later in operation order wins, as with a rename.
	Settled {
		on: OpId,
		state: Settled,
		mark: Option<OpId>,				// an identifier, never a mark's name
		time: u64,
	},
	Reverts {
		undone: Vec<OpId>,				// ascending, without repetition, never empty
	},
}

impl Op {
	/// A mark answers with the code it is written at, which is a function of
	/// what it carries rather than of the variant: a mark with neither a body
	/// nor a time is [`CODE_MARK`] with the two elements it has always had, so
	/// every mark already signed still verifies, and a mark with either is
	/// [`CODE_MARK_TIMED`] with four. That is what
	/// [`crate::segment::highest_code`] reads, so a bodyless, timeless mark goes
	/// into a segment of any version this crate reads, and a mark carrying
	/// either does not go into one written before the fields existed.
	pub fn code(&self) -> u8 {
		match self {
			Self::FileCreate { .. }	=> CODE_FILE_CREATE,
			Self::FileDelete { .. }	=> CODE_FILE_DELETE,
			Self::FileRename { .. }	=> CODE_FILE_RENAME,
			Self::FileMode { .. }	=> CODE_FILE_MODE,
			Self::Mark { body: None, time: None, .. }
									=> CODE_MARK,
			Self::Mark { .. }		=> CODE_MARK_TIMED,
			Self::Splice { .. }		=> CODE_SPLICE,
			Self::Move { .. }		=> CODE_MOVE,
			Self::Note { .. }		=> CODE_NOTE,
			Self::Proposal { .. }	=> CODE_PROPOSAL,
			Self::Said { .. }		=> CODE_SAID,
			Self::Settled { .. }	=> CODE_SETTLED,
			Self::Reverts { .. }	=> CODE_REVERTS,
		}
	}

	/// One name for both of a mark's spellings, since the two are one operation
	/// and a reader told otherwise would go looking for a variant that is not
	/// there.
	pub fn name(&self) -> &'static str {
		match self {
			Self::FileCreate { .. }	=> "FileCreate",
			Self::FileDelete { .. }	=> "FileDelete",
			Self::FileRename { .. }	=> "FileRename",
			Self::FileMode { .. }	=> "FileMode",
			Self::Mark { .. }		=> "Mark",
			Self::Splice { .. }		=> "Splice",
			Self::Move { .. }		=> "Move",
			Self::Note { .. }		=> "Note",
			Self::Proposal { .. }	=> "Proposal",
			Self::Said { .. }		=> "Said",
			Self::Settled { .. }	=> "Settled",
			Self::Reverts { .. }	=> "Reverts",
		}
	}

	/// Only a lifecycle change names a file. A content operation gives `None`
	/// because it names content and the file follows from that; a file's
	/// creation gives `None` because the file it names is itself.
	pub fn names_file(&self) -> Option<OpId> {
		match self {
			Self::FileDelete { file }		=> Some(*file),
			Self::FileRename { file, .. }	=> Some(*file),
			Self::FileMode { file, .. }		=> Some(*file),
			_								=> None,
		}
	}

	pub fn origins(&self) -> (Option<Anchor>, Option<Anchor>) {
		match self {
			Self::Splice { left, right, .. }	=> (*left, *right),
			Self::Move { left, right, .. }		=> (*left, *right),
			_									=> (None, None),
		}
	}

	/// The content the operation **acts on**, which is what decides whether two
	/// operations were in conflict.
	///
	/// A note is not here, although it names content too: it asserts nothing,
	/// neither killing content nor taking it anywhere, so a note and a
	/// concurrent deletion of the same run are not two authors disagreeing. See
	/// [`Op::note_on`] for the other reading.
	pub fn regions(&self) -> &[ContentRange] {
		match self {
			Self::Splice { remove, .. }	=> remove,
			Self::Move { src, .. }		=> src,
			_							=> &[],
		}
	}

	pub fn note_on(&self) -> &[ContentRange] {
		match self {
			Self::Note { on, .. }	=> on,
			_						=> &[],
		}
	}

	pub fn is_move(&self) -> bool {
		matches!(self, Self::Move { .. })
	}

	pub fn placed_len(&self) -> u64 {
		match self {
			Self::Splice { insert, .. }	=> insert.len() as u64,
			Self::Move { src, .. }		=> src.iter().map(|r| r.len()).sum(),
			_							=> 0,
		}
	}

	/// Checks the rule that replaces the file field: an operation that places
	/// anything must carry at least one origin, that origin being what says
	/// which file it lands in.
	///
	/// Enforced on the way off the wire as well as on the way into the sequence,
	/// because an operation satisfying neither origin belongs to no file and
	/// there is nowhere for a reader to put it.
	pub fn check_placement(&self)
		-> Outcome<()>
	{
		let (left, right) = self.origins();
		if left.is_some() || right.is_some() {
			return Ok(());
		}
		match self {
			Self::Splice { insert, .. } if !insert.is_empty() => Err(err!(
				"A Splice inserting {} bytes carries no origin; an operation that \
				places anything names at least one, an empty file's origin anchor \
				being what a splice into an empty file names.", insert.len();
			Invalid, Input, Missing)),
			Self::Move { .. } => Err(err!(
				"A Move carries no origin; a move always places what it names, so it \
				always names where.";
			Invalid, Input, Missing)),
			_ => Ok(()),
		}
	}

	/// Checks the rule that a note is about something: [`Op::Note`] must name at
	/// least one byte, an empty list and a list of empty ranges naming none.
	///
	/// Such a note could never resolve to a span and would be reported forever
	/// as a note on dead content, which is not what "dead" is for. Checked on
	/// the way off the wire too, for the reason [`Op::check_placement`] is.
	pub fn check_note(&self)
		-> Outcome<()>
	{
		let on = match self {
			Self::Note { on, .. }	=> on,
			_						=> return Ok(()),
		};
		if on.iter().any(|r| !r.is_empty()) {
			return Ok(());
		}
		Err(err!(
			"A Note is about {} content ranges, none of which names a byte; a note \
			is about something, and a Mark is what says something about a point in \
			history.", on.len();
		Invalid, Input, Missing))
	}

	/// Checks the rule that a revert names what it undoes, exactly once each and
	/// in order.
	///
	/// The list is held ascending and without repetition, for the reason
	/// [`Header::from_dat`] holds the parents that way: the same set written two
	/// ways would be two byte strings, both of which a signature would verify,
	/// and a provenance chain cannot afford a record with two spellings. It is
	/// refused rather than sorted, so that whoever wrote it finds out.
	pub fn check_reverts(&self)
		-> Outcome<()>
	{
		let undone = match self {
			Self::Reverts { undone }	=> undone,
			_							=> return Ok(()),
		};
		if undone.is_empty() {
			return Err(err!(
				"A Reverts names no operation; a revert undoes something, and a Mark \
				is what says something about a point in history.";
			Invalid, Input, Missing));
		}
		for pair in undone.windows(2) {
			if pair[1] <= pair[0] {
				return Err(err!(
					"A Reverts lists {} after {}; what a revert undoes is named \
					ascending and without repetition.", pair[1], pair[0];
				Decode, Input, Order));
			}
		}
		Ok(())
	}

	/// Why nothing in the vocabulary undoes this operation, or `None` where
	/// something does.
	///
	/// The sentence is here, and not in whoever refuses, so that a person told
	/// no by a command and a person told no by a forge are told the same thing.
	pub fn no_inverse(&self) -> Option<&'static str> {
		match self {
			Self::FileDelete { .. } => Some(
				"a file's deletion holds its content back rather than destroying it, and \
				no operation revives a file; a file with those bytes written again is a \
				new file, which is what `undo` does and says"),
			Self::Mark { .. } => Some(
				"a mark says where somebody was at a point in the history, and the \
				history only grows; there is nothing about it to take back"),
			Self::Note { .. } => Some(
				"a note is something somebody said about content, and nothing un-says \
				it; when the content it is about goes, the note reports itself as a note \
				on dead content"),
			Self::Proposal { .. } => Some(
				"a proposal is something somebody asked for, and the record of the \
				asking grows rather than retracts; what became of it is said by writing \
				a Settled"),
			Self::Said { .. } => Some(
				"a remark is something somebody said, and nothing un-says it"),
			Self::Settled { .. } => Some(
				"a settlement asserts what a proposal now is, and is superseded by \
				writing another rather than undone"),
			Self::Reverts { .. } => Some(
				"this names what some edits were written to undo; taking the name away \
				would leave the edits and lose the only record of what they were for, so \
				it is those edits that are reverted"),
			_ => None,
		}
	}

	/// What undoing this operation amounts to, `id` being the identity the
	/// operation was recorded under.
	///
	/// The identity is asked for rather than carried because an operation does
	/// not hold one -- the same edit written by two authors is two operations --
	/// and an inverse needs it: what a splice inserted is named by the splice,
	/// so undoing the insertion is a removal naming that identity and nothing
	/// else, exact and costing no render.
	///
	/// A splice's two halves are not alike. The half that inserted is undone
	/// exactly. The half that removed is not: nothing here un-buries, so what
	/// comes back is a copy under a new identity, [`Undoing::copies`] says which
	/// runs, and a caller must say so rather than report a clean restoration.
	pub fn undoing(&self, id: OpId)
		-> Outcome<Undoing>
	{
		if let Some(why) = self.no_inverse() {
			return Err(err!(
				"Nothing undoes the {} {}: {}.", self.name(), id, why;
			Invalid, Input, Unimplemented));
		}
		match self {
			// The file did not exist before, so there is nothing to look up and the
			// inverse is the one operation that retires it. Note what follows: a
			// deletion has no inverse of its own, so this is the one undoing in the
			// vocabulary that cannot itself be undone.
			Self::FileCreate { .. } => Ok(Undoing {
				written:	vec![Self::FileDelete { file: id }],
				..Undoing::default()
			}),
			Self::FileRename { file, .. } => Ok(Undoing {
				prior:	Some(Prior::Path { file: *file }),
				..Undoing::default()
			}),
			Self::FileMode { file, .. } => Ok(Undoing {
				prior:	Some(Prior::Mode { file: *file }),
				..Undoing::default()
			}),
			Self::Splice { remove, insert, .. } => {
				let mut written = Vec::new();
				if !insert.is_empty() {
					written.push(Self::Splice {
						left:	None,
						right:	None,
						remove:	vec![res!(ContentRange::new(id, 0, insert.len() as u64))],
						insert:	Arc::from(Vec::new()),
					});
				}
				Ok(Undoing {
					written,
					copies:	remove.iter().filter(|r| !r.is_empty()).copied().collect(),
					prior:	None,
				})
			},
			Self::Move { src, .. } => Ok(Undoing {
				prior:	Some(Prior::Place {
					src: src.iter().filter(|r| !r.is_empty()).copied().collect(),
				}),
				..Undoing::default()
			}),
			// Everything left is refused above, and the arm is here so that a
			// variant added later fails loudly rather than being quietly undoable
			// by nothing.
			other => Err(err!(
				"The {} {} is neither undone nor refused; a new operation belongs in \
				one of the two.", other.name(), id;
			Bug, Missing)),
		}
	}

	/// Builds the splice that puts a copy of dead content back where it was.
	///
	/// The anchor is fixed here so that two authors of a revert produce the same
	/// shape: the copy binds **after the last byte of the run it restores**. An
	/// anchor names content whether it is alive or dead, so this lands the copy
	/// exactly where the original is buried, however much of what surrounded it
	/// has gone since -- which no other anchor can promise, the neighbours being
	/// the very thing a deletion took away.
	///
	/// It is also what makes the copy readable afterwards, [`Op::restored`]
	/// taking the anchor and the length back apart. Nothing else in the record
	/// connects the two, the bytes having a new identity from the moment they
	/// come back.
	pub fn restoring(was: &ContentRange, bytes: Vec<u8>)
		-> Outcome<Self>
	{
		if was.is_empty() {
			return Err(err!(
				"The content {} names no byte, so there is nothing to restore.", was;
			Invalid, Input, Missing));
		}
		if bytes.len() as u64 != was.len() {
			return Err(err!(
				"A copy of {} bytes was offered for the content {}, which is {} bytes; a \
				restoration puts back what was there.", bytes.len(), was, was.len();
			Invalid, Input, Mismatch));
		}
		Ok(Self::Splice {
			left:	Some(Anchor::after(ContentId::new(was.op(), was.to() - 1))),
			right:	None,
			remove:	Vec::new(),
			insert:	bytes.into(),
		})
	}

	/// The content this operation is a copy of, where it has the shape
	/// [`Op::restoring`] gives one: a splice that removes nothing, ends nothing,
	/// and binds after the last byte of the run restored.
	///
	/// **This shape is not unique and is not evidence on its own.** An ordinary
	/// insertion at the end of a file has it too. What makes a copy a copy is
	/// that an [`Op::Reverts`] vouches for it, and a reader that skips that
	/// check will credit an author for text somebody merely appended.
	pub fn restored(&self) -> Option<ContentRange> {
		let (left, right, remove, insert) = match self {
			Self::Splice { left, right, remove, insert }	=> (left, right, remove, insert),
			_											=> return None,
		};
		if right.is_some() || !remove.is_empty() || insert.is_empty() {
			return None;
		}
		let anchor = match left {
			Some(a) if a.side == Side::After	=> a,
			_									=> return None,
		};
		// The anchored byte is the last of the run, so the run began that many
		// bytes earlier. A run reaching back past the start of its own atom is not
		// one this ever wrote.
		let to = anchor.content.off + 1;
		let from = to.checked_sub(insert.len() as u64)?;
		ContentRange::new(anchor.content.op, from, to).ok()
	}

	/// Checks the operation is one the sequence structure can resolve: a left
	/// origin binds after a byte and a right origin before one, a move may not
	/// name the same byte twice since a byte has one owning slot, and
	/// [`Op::check_placement`] must hold.
	pub fn validate(&self)
		-> Outcome<()>
	{
		let (left, right) = self.origins();
		if let Some(a) = left {
			if a.side != Side::After {
				return Err(err!(
					"An {} names {} as its left origin; a left origin binds after a \
					byte, not before it.", self.name(), a;
				Invalid, Input));
			}
		}
		if let Some(a) = right {
			if a.side != Side::Before {
				return Err(err!(
					"An {} names {} as its right origin; a right origin binds before a \
					byte, not after it.", self.name(), a;
				Invalid, Input));
			}
		}
		if let Self::Move { src, .. } = self {
			// Sorted by creating operation and then by offset, any overlap at all
			// shows up between neighbours.
			let mut spans: Vec<&ContentRange> = src.iter()
				.filter(|r| !r.is_empty())
				.collect();
			spans.sort_by_key(|r| (r.op(), r.from()));
			for pair in spans.windows(2) {
				if pair[0].intersects(pair[1]) {
					return Err(err!(
						"A Move names {} and {}, which overlap; one byte cannot be \
						moved to two places by one operation.", pair[0], pair[1];
					Invalid, Input, Conflict));
				}
			}
		}
		res!(self.check_placement());
		res!(self.check_note());
		res!(self.check_reverts());
		Ok(())
	}

	/// The shape is `[code, field, ...]`, the fields in declaration order.
	///
	/// Byte payloads use [`Dat::BU64`] rather than [`Dat::BU8`], whose length
	/// field is a single byte and so keeps only the low eight bits of the
	/// length of anything longer than 255 bytes. A path is a byte payload for the
	/// same reason a path is not a string: neither is the caller's to constrain.
	pub fn to_dat(&self) -> Dat {
		match self {
			Self::FileCreate { path } => Dat::List(vec![
				Dat::U8(CODE_FILE_CREATE),
				Dat::BU64(path.clone()),
			]),
			Self::FileDelete { file } => Dat::List(vec![
				Dat::U8(CODE_FILE_DELETE),
				file.to_dat(),
			]),
			Self::FileRename { file, path } => Dat::List(vec![
				Dat::U8(CODE_FILE_RENAME),
				file.to_dat(),
				Dat::BU64(path.clone()),
			]),
			Self::FileMode { file, mode } => Dat::List(vec![
				Dat::U8(CODE_FILE_MODE),
				file.to_dat(),
				mode.to_dat(),
			]),
			// Two spellings of one variant, chosen by what the mark carries: the
			// short one is what every mark written before the fields existed
			// says, byte for byte, and the long one is what a mark carrying
			// either of them says. Which is written is not the author's choice,
			// so the encoding stays canonical and a mark has one signature.
			Self::Mark { name, body: None, time: None } => Dat::List(vec![
				Dat::U8(CODE_MARK),
				Dat::Str(name.clone()),
			]),
			Self::Mark { name, body, time } => Dat::List(vec![
				Dat::U8(CODE_MARK_TIMED),
				Dat::Str(name.clone()),
				opt_bytes_to_dat(body),
				opt_u64_to_dat(time),
			]),
			Self::Splice { left, right, remove, insert } => Dat::List(vec![
				Dat::U8(CODE_SPLICE),
				Anchor::opt_to_dat(left),
				Anchor::opt_to_dat(right),
				Dat::List(remove.iter().map(|r| r.to_dat()).collect()),
				Dat::BU64(insert.to_vec()),
			]),
			Self::Move { src, left, right } => Dat::List(vec![
				Dat::U8(CODE_MOVE),
				Dat::List(src.iter().map(|r| r.to_dat()).collect()),
				Anchor::opt_to_dat(left),
				Anchor::opt_to_dat(right),
			]),
			Self::Note { on, text } => Dat::List(vec![
				Dat::U8(CODE_NOTE),
				Dat::List(on.iter().map(|r| r.to_dat()).collect()),
				Dat::BU64(text.clone()),
			]),
			Self::Proposal { title, body, voice, time } => Dat::List(vec![
				Dat::U8(CODE_PROPOSAL),
				Dat::Str(title.clone()),
				Dat::BU64(body.clone()),
				Dat::Str(voice.clone()),
				Dat::U64(*time),
			]),
			Self::Said { on, text, voice, time } => Dat::List(vec![
				Dat::U8(CODE_SAID),
				on.to_dat(),
				Dat::BU64(text.clone()),
				Dat::Str(voice.clone()),
				Dat::U64(*time),
			]),
			Self::Settled { on, state, mark, time } => Dat::List(vec![
				Dat::U8(CODE_SETTLED),
				on.to_dat(),
				state.to_dat(),
				opt_id_to_dat(mark),
				Dat::U64(*time),
			]),
			Self::Reverts { undone } => Dat::List(vec![
				Dat::U8(CODE_REVERTS),
				Dat::List(undone.iter().map(|u| u.to_dat()).collect()),
			]),
		}
	}

	/// The placement rule is checked here rather than left to the sequence,
	/// because an operation that places bytes and names no origin belongs to no
	/// file and no later stage could decide one for it. [`Op::check_note`] is
	/// checked here for the same reason: a note about nothing resolves to nothing,
	/// wherever it is read. So is [`Op::check_reverts`], since a list that arrives
	/// out of order is a second byte spelling of a set that has one.
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if !v.is_empty() => v,
			_ => return Err(err!(
				"An Op expects a non-empty Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let code = match &v[0] {
			Dat::U8(c) => *c,
			other => return Err(err!(
				"An Op code expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		let op = match code {
			CODE_FILE_CREATE => {
				res!(expect_len(v, 2, "FileCreate"));
				Self::FileCreate {
					path: res!(as_bytes(&v[1], "FileCreate path")),
				}
			},
			CODE_FILE_DELETE => {
				res!(expect_len(v, 2, "FileDelete"));
				Self::FileDelete {
					file: res!(OpId::from_dat(&v[1])),
				}
			},
			CODE_FILE_RENAME => {
				res!(expect_len(v, 3, "FileRename"));
				Self::FileRename {
					file:	res!(OpId::from_dat(&v[1])),
					path:	res!(as_bytes(&v[2], "FileRename path")),
				}
			},
			CODE_FILE_MODE => {
				res!(expect_len(v, 3, "FileMode"));
				Self::FileMode {
					file:	res!(OpId::from_dat(&v[1])),
					mode:	res!(Mode::from_dat(&v[2])),
				}
			},
			// Both spellings decode to the one variant, so nothing downstream has
			// to know which of them it was read from.
			CODE_MARK => {
				res!(expect_len(v, 2, "Mark"));
				Self::Mark {
					name:	res!(as_str(&v[1], "Mark name")),
					body:	None,
					time:	None,
				}
			},
			CODE_MARK_TIMED => {
				res!(expect_len(v, 4, "Mark"));
				let body = res!(as_opt_bytes(&v[2], "Mark body"));
				let time = res!(as_opt_u64(&v[3], "Mark time"));
				// The long spelling is refused where it says nothing the short one
				// could not, rather than being quietly read as the short one.
				// Otherwise a mark would have two encodings, both verifying against
				// a signature, which is the thing [`Header::from_dat`] refuses for
				// the same reason.
				if body.is_none() && time.is_none() {
					return Err(err!(
						"A Mark named {:?} is written at wire code {} carrying neither a \
						body nor a time; a mark with neither is written at code {}, and \
						an operation has one encoding.",
						res!(as_str(&v[1], "Mark name")), CODE_MARK_TIMED, CODE_MARK;
					Decode, Input, Invalid));
				}
				Self::Mark {
					name:	res!(as_str(&v[1], "Mark name")),
					body,
					time,
				}
			},
			CODE_SPLICE => {
				res!(expect_len(v, 5, "Splice"));
				Self::Splice {
					left:	res!(Anchor::opt_from_dat(&v[1])),
					right:	res!(Anchor::opt_from_dat(&v[2])),
					remove:	res!(as_ranges(&v[3], "Splice remove")),
					insert:	res!(as_bytes(&v[4], "Splice insert")).into(),
				}
			},
			CODE_MOVE => {
				res!(expect_len(v, 4, "Move"));
				Self::Move {
					src:	res!(as_ranges(&v[1], "Move src")),
					left:	res!(Anchor::opt_from_dat(&v[2])),
					right:	res!(Anchor::opt_from_dat(&v[3])),
				}
			},
			CODE_NOTE => {
				res!(expect_len(v, 3, "Note"));
				Self::Note {
					on:		res!(as_ranges(&v[1], "Note on")),
					text:	res!(as_bytes(&v[2], "Note text")),
				}
			},
			CODE_PROPOSAL => {
				res!(expect_len(v, 5, "Proposal"));
				Self::Proposal {
					title:	res!(as_str(&v[1], "Proposal title")),
					body:	res!(as_bytes(&v[2], "Proposal body")),
					voice:	res!(as_str(&v[3], "Proposal voice")),
					time:	res!(as_u64(&v[4], "Proposal time")),
				}
			},
			CODE_SAID => {
				res!(expect_len(v, 5, "Said"));
				Self::Said {
					on:		res!(OpId::from_dat(&v[1])),
					text:	res!(as_bytes(&v[2], "Said text")),
					voice:	res!(as_str(&v[3], "Said voice")),
					time:	res!(as_u64(&v[4], "Said time")),
				}
			},
			CODE_SETTLED => {
				res!(expect_len(v, 5, "Settled"));
				Self::Settled {
					on:		res!(OpId::from_dat(&v[1])),
					state:	res!(Settled::from_dat(&v[2])),
					mark:	res!(as_opt_id(&v[3], "Settled mark")),
					time:	res!(as_u64(&v[4], "Settled time")),
				}
			},
			CODE_REVERTS => {
				res!(expect_len(v, 2, "Reverts"));
				Self::Reverts {
					undone: res!(as_ids(&v[1], "Reverts undone")),
				}
			},
			other => return Err(err!(
				"Op code {} is not recognised.", other;
			Decode, Input, Invalid)),
		};
		res!(op.check_placement());
		res!(op.check_note());
		res!(op.check_reverts());
		Ok(op)
	}

	/// A varint length followed by the binary daticle form. The prefix lets a
	/// consumer skip an operation it does not need to read, and lets several be
	/// laid end to end in one buffer.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		varint_encode(body.len() as u64, buf);
		buf.extend_from_slice(&body);
		Ok(())
	}

	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (dat, end) = res!(decode_framed(buf, "Op"));
		Ok((res!(Self::from_dat(&dat)), end))
	}

	/// Hashes the canonical encoding. The choice of hash function is deliberately
	/// not made here: which one is right depends on what else has to compute the
	/// same value -- a browser limited to what its platform offers, a peer group
	/// that has already agreed on one -- so the caller brings it.
	pub fn hash<H: Hasher, const S: usize>(&self, hasher: H, salt: [u8; S])
		-> Outcome<Hash<S>>
	{
		let bytes = res!(self.encode());
		Ok(hasher.hash(&[&bytes], salt))
	}

	pub fn decode_all(buf: &[u8])
		-> Outcome<Self>
	{
		let (op, len) = res!(Self::decode(buf));
		if len != buf.len() {
			return Err(err!(
				"An Op consumed {} of {} bytes, leaving {} trailing.",
				len, buf.len(), buf.len() - len;
			Decode, Input, Excessive));
		}
		Ok(op)
	}
}


/// What every operation carries whatever it says: its own name, and the names
/// of the operations its author had already seen.
///
/// The parents are the author's frontier at the moment of writing, which is
/// what turns a heap of operations into a partial order. Two operations are
/// concurrent exactly when neither is reachable from the other by following
/// parents, and that question -- not the accident of which arrived first -- is
/// what decides whether two edits to the same bytes were a conflict or a
/// sequence.
///
/// Both fields are private, because the canonical form is an invariant and not a
/// convention: parents are held sorted and without repetition, and an operation
/// is never its own parent. Two byte spellings of one frontier would both verify
/// against a signature, which is not a property a provenance chain can afford.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Header {
	id:			OpId,
	parents:	Vec<OpId>,	// ascending, without repetition
}

impl Header {
	/// Sorts the parents and drops repetitions, which is where the canonical
	/// form is established.
	pub fn new(id: OpId, parents: Vec<OpId>)
		-> Outcome<Self>
	{
		let mut parents = parents;
		parents.sort();
		parents.dedup();
		if parents.binary_search(&id).is_ok() {
			return Err(err!(
				"The operation {} names itself as one of its own parents.", id;
			Invalid, Input, Conflict));
		}
		Ok(Self { id, parents })
	}

	/// A root operation is one written against nothing.
	pub fn root(id: OpId) -> Self {
		Self { id, parents: Vec::new() }
	}

	pub const fn id(&self) -> OpId {
		self.id
	}

	/// The author's frontier when the operation was written, ascending and
	/// without repetition.
	pub fn parents(&self) -> &[OpId] {
		&self.parents
	}

	pub fn is_root(&self) -> bool {
		self.parents.is_empty()
	}

	/// The shape is `[id, [parent, ...]]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.id.to_dat(),
			Dat::List(self.parents.iter().map(|p| p.to_dat()).collect()),
		])
	}

	/// Parents out of order, repeated, or naming the operation itself are
	/// refused rather than normalised, so that the encoding stays canonical.
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Header expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let id = res!(OpId::from_dat(&v[0]));
		let listed = match &v[1] {
			Dat::List(p) => p,
			other => return Err(err!(
				"A Header's parents expect Dat::List, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		let mut parents = Vec::with_capacity(listed.len());
		for item in listed {
			let p = res!(OpId::from_dat(item));
			if let Some(last) = parents.last() {
				if p <= *last {
					return Err(err!(
						"A Header of {} lists the parent {} after {}; parents are \
						encoded ascending and without repetition.", id, p, last;
					Decode, Input, Order));
				}
			}
			if p == id {
				return Err(err!(
					"A Header of {} names itself as one of its own parents.", id;
				Decode, Input, Conflict));
			}
			parents.push(p);
		}
		Ok(Self { id, parents })
	}
}


/// One whole operation as history records it: the header that names it and
/// places it in the graph, and the operation itself.
///
/// This is the unit that is logged, sealed into an [`crate::envelope::Envelope`]
/// and written into a segment. The header is not part of the operation because
/// the same edit written by two authors is two operations, and the vocabulary
/// should not have to say so six times over.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
	pub head:	Header,
	pub op:		Op,
}

impl Record {
	pub fn new(head: Header, op: Op) -> Self {
		Self { head, op }
	}

	pub fn root(id: OpId, op: Op) -> Self {
		Self { head: Header::root(id), op }
	}

	pub fn id(&self) -> OpId {
		self.head.id()
	}

	pub fn parents(&self) -> &[OpId] {
		self.head.parents()
	}

	/// The shape is `[head, op]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.head.to_dat(),
			self.op.to_dat(),
		])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Record expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		Ok(Self {
			head:	res!(Header::from_dat(&v[0])),
			op:		res!(Op::from_dat(&v[1])),
		})
	}

	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		varint_encode(body.len() as u64, buf);
		buf.extend_from_slice(&body);
		Ok(())
	}

	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (dat, end) = res!(decode_framed(buf, "Record"));
		Ok((res!(Self::from_dat(&dat)), end))
	}

	pub fn decode_all(buf: &[u8])
		-> Outcome<Self>
	{
		let (rec, len) = res!(Self::decode(buf));
		if len != buf.len() {
			return Err(err!(
				"A Record consumed {} of {} bytes, leaving {} trailing.",
				len, buf.len(), buf.len() - len;
			Decode, Input, Excessive));
		}
		Ok(rec)
	}

	/// Covers the parents along with the operation.
	///
	/// This value is not the digest a segment stores for the record: a segment
	/// digests the record's kind byte and unframed body, while this hashes the
	/// framed encoding, so the two are computed over different byte strings
	/// and will not match.
	pub fn hash<H: Hasher, const S: usize>(&self, hasher: H, salt: [u8; S])
		-> Outcome<Hash<S>>
	{
		let bytes = res!(self.encode());
		Ok(hasher.hash(&[&bytes], salt))
	}
}


fn decode_framed(buf: &[u8], what: &str)
	-> Outcome<(Dat, usize)>
{
	let (len, hdr) = res!(varint_decode(buf));
	let len = len as usize;
	let end = match hdr.checked_add(len) {
		Some(e) => e,
		None => return Err(err!(
			"A {} declares a length of {} bytes, which overflows the buffer \
			offset.", what, len;
		Decode, Input, Overflow)),
	};
	if end > buf.len() {
		return Err(err!(
			"A {} declares {} bytes of body but only {} remain.",
			what, len, buf.len() - hdr;
		Decode, Input, Missing));
	}
	let (dat, used) = res!(Dat::from_bytes(&buf[hdr..end]));
	if used != len {
		return Err(err!(
			"A {} body of {} bytes decoded from only {} of them.", what, len, used;
		Decode, Input, Mismatch));
	}
	Ok((dat, end))
}

fn expect_len(v: &[Dat], want: usize, what: &str)
	-> Outcome<()>
{
	if v.len() != want {
		return Err(err!(
			"An Op::{} expects {} list elements, got {}.", what, want, v.len();
		Decode, Input, Mismatch));
	}
	Ok(())
}

fn as_str(dat: &Dat, what: &str)
	-> Outcome<String>
{
	match dat {
		Dat::Str(s) => Ok(s.clone()),
		other => Err(err!(
			"An Op {} expects Dat::Str, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

fn as_ranges(dat: &Dat, what: &str)
	-> Outcome<Vec<ContentRange>>
{
	match dat {
		Dat::List(v) => {
			let mut out = Vec::with_capacity(v.len());
			for item in v {
				out.push(res!(ContentRange::from_dat(item)));
			}
			Ok(out)
		},
		other => Err(err!(
			"An Op {} expects Dat::List, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

fn as_bytes(dat: &Dat, what: &str)
	-> Outcome<Vec<u8>>
{
	match dat {
		Dat::BU64(b) => Ok(b.clone()),
		other => Err(err!(
			"An Op {} expects Dat::BU64, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

fn as_ids(dat: &Dat, what: &str)
	-> Outcome<Vec<OpId>>
{
	match dat {
		Dat::List(v) => {
			let mut out = Vec::with_capacity(v.len());
			for item in v {
				out.push(res!(OpId::from_dat(item)));
			}
			Ok(out)
		},
		other => Err(err!(
			"An Op {} expects Dat::List, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

/// A time is exactly this and nothing narrower: seconds since the Unix epoch, in
/// UTC, read by whoever wrote the operation and never recomputed afterwards,
/// since a second reading would be a different operation under the same
/// signature.
fn as_u64(dat: &Dat, what: &str)
	-> Outcome<u64>
{
	match dat {
		Dat::U64(n) => Ok(*n),
		other => Err(err!(
			"An Op {} expects Dat::U64, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

fn opt_bytes_to_dat(body: &Option<Vec<u8>>) -> Dat {
	Dat::Opt(Box::new(body.as_ref().map(|b| Dat::BU64(b.clone()))))
}

fn as_opt_bytes(dat: &Dat, what: &str)
	-> Outcome<Option<Vec<u8>>>
{
	match dat {
		Dat::Opt(boxed) => match boxed.as_ref() {
			Some(inner)	=> Ok(Some(res!(as_bytes(inner, what)))),
			None		=> Ok(None),
		},
		other => Err(err!(
			"An optional Op {} expects Dat::Opt, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

fn opt_u64_to_dat(time: &Option<u64>) -> Dat {
	Dat::Opt(Box::new(time.map(Dat::U64)))
}

fn as_opt_u64(dat: &Dat, what: &str)
	-> Outcome<Option<u64>>
{
	match dat {
		Dat::Opt(boxed) => match boxed.as_ref() {
			Some(inner)	=> Ok(Some(res!(as_u64(inner, what)))),
			None		=> Ok(None),
		},
		other => Err(err!(
			"An optional Op {} expects Dat::Opt, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

fn opt_id_to_dat(id: &Option<OpId>) -> Dat {
	Dat::Opt(Box::new(id.as_ref().map(|i| i.to_dat())))
}

fn as_opt_id(dat: &Dat, what: &str)
	-> Outcome<Option<OpId>>
{
	match dat {
		Dat::Opt(boxed) => match boxed.as_ref() {
			Some(inner)	=> Ok(Some(res!(OpId::from_dat(inner)))),
			None		=> Ok(None),
		},
		other => Err(err!(
			"An optional Op {} expects Dat::Opt, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::id::{
		ContentId,
		ReplicaId,
	};

	/// A content range of the given replica's first operation. The bounds are put
	/// in order before the constructor sees them, so the helper is total and can
	/// be called from fixtures that return an operation rather than an
	/// [`Outcome`].
	fn range(replica: u64, from: u64, to: u64) -> ContentRange {
		let op = OpId::new(ReplicaId::new(replica), 1);
		ContentRange::new(op, from.min(to), from.max(to)).unwrap_or_default()
	}

	fn content(replica: u64, off: u64) -> ContentId {
		ContentId::new(OpId::new(ReplicaId::new(replica), 1), off)
	}

	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// One of every variant, including payloads that stress the encoding.
	fn samples() -> Vec<Op> {
		vec![
			Op::FileCreate { path: b"src/lib.rs".to_vec() },
			Op::FileDelete { file: oid(1, 1) },
			Op::FileRename {
				file:	oid(2, 5),
				path:	b"c/d.txt".to_vec(),
			},
			// Every mode the vocabulary spells, including the one a file has
			// anyway, which an author may still want said out loud.
			Op::FileMode { file: oid(2, 5), mode: Mode::Normal },
			Op::FileMode { file: oid(2, 5), mode: Mode::Executable },
			Op::FileMode { file: oid(7, 1), mode: Mode::Symlink },
			// An insertion into an empty file, anchored after its origin anchor,
			// which is what an empty file has instead of nothing.
			Op::Splice {
				left:	Some(Anchor::origin(oid(1, 1))),
				right:	None,
				remove:	Vec::new(),
				insert:	b"hello".to_vec().into(),
			},
			// A deletion, which places nothing and so needs no origin.
			Op::Splice {
				left:	None,
				right:	None,
				remove:	vec![range(1, 12, 17)],
				insert:	Vec::new().into(),
			},
			// A replacement whose payload exceeds what a BU8 length can hold,
			// killing several fragmented runs at once.
			Op::Splice {
				left:	Some(Anchor::after(content(2, u64::MAX))),
				right:	Some(Anchor::before(content(3, 0))),
				remove:	vec![
					range(1, 0, u64::MAX),
					range(4, 7, 9),
				],
				insert:	vec![0xa5; 1000].into(),
			},
			// An empty path, and a path that is not UTF-8 at all, which the old
			// vocabulary could not spell.
			Op::FileCreate { path: Vec::new() },
			Op::FileCreate { path: vec![0xff, 0xfe, 0x2f, 0x00, 0x80] },
			Op::Mark { name: fmt!("release-caf\u{e9}"), body: None, time: None },
			// Every combination the mark's two spellings cover: the short one,
			// then a body alone, a time alone, and both. A body longer than a
			// single byte length field could hold, and one that is not UTF-8,
			// since a mark's body is no more this crate's to decode than a note's
			// text is.
			Op::Mark {
				name:	fmt!("v2"),
				body:	Some(b"what this release is for".to_vec()),
				time:	None,
			},
			Op::Mark {
				name:	fmt!("v3"),
				body:	None,
				time:	Some(1_755_400_329),
			},
			Op::Mark {
				name:	fmt!("v4"),
				body:	Some(vec![0xc3; 900]),
				time:	Some(u64::MAX),
			},
			Op::Mark {
				name:	String::new(),
				body:	Some(Vec::new()),
				time:	Some(0),
			},
			// A proposal, its discussion and its outcome.
			Op::Proposal {
				title:	fmt!("Carry a body on a mark"),
				body:	b"A mark names a point and says nothing about it.".to_vec(),
				voice:	fmt!("wren"),
				time:	1_755_400_000,
			},
			Op::Proposal {
				title:	String::new(),
				body:	vec![0xff; 700],
				voice:	String::new(),
				time:	0,
			},
			Op::Said {
				on:		oid(3, 4),
				text:	b"Agreed, provided the old bytes do not move.".to_vec(),
				voice:	fmt!("caf\u{e9}"),
				time:	1_755_400_100,
			},
			Op::Said {
				on:		oid(u64::MAX, u64::MAX),
				text:	Vec::new(),
				voice:	String::new(),
				time:	u64::MAX,
			},
			// Every state, and both spellings of the mark that closed it.
			Op::Settled {
				on:		oid(3, 4),
				state:	Settled::Open,
				mark:	None,
				time:	1_755_400_200,
			},
			Op::Settled {
				on:		oid(3, 4),
				state:	Settled::Accepted,
				mark:	Some(oid(9, 2)),
				time:	1_755_400_201,
			},
			Op::Settled {
				on:		oid(3, 4),
				state:	Settled::Declined,
				mark:	None,
				time:	1_755_400_202,
			},
			Op::Settled {
				on:		oid(3, 4),
				state:	Settled::Done,
				mark:	Some(oid(1, u64::MAX)),
				time:	u64::MAX,
			},
			// A revert of one operation, of several by one author, and of several
			// spread across authors and out to the ends of the identifier space.
			Op::Reverts { undone: vec![oid(2, 7)] },
			Op::Reverts { undone: vec![oid(1, 1), oid(1, 2), oid(3, 1)] },
			Op::Reverts { undone: vec![
				oid(0, 1),
				oid(4, u64::MAX),
				oid(u64::MAX, 1),
			] },
			// A move of one run into the middle of a file.
			Op::Move {
				src:	vec![range(1, 0, 40)],
				left:	Some(Anchor::after(content(2, 3))),
				right:	Some(Anchor::before(content(2, 4))),
			},
			// A move to the very start of a file, of a run fragmented by the
			// edits it has already survived: the destination is the gap after
			// that file's origin anchor.
			Op::Move {
				src:	vec![
					range(1, 0, 4),
					range(3, 7, 9),
					range(1, 4, 40),
				],
				left:	Some(Anchor::origin(oid(9, 1))),
				right:	Some(Anchor::before(content(1, 41))),
			},
			// A move to the end of a file, whose destination is bounded by
			// nothing on the right.
			Op::Move {
				src:	vec![range(u64::MAX, 0, u64::MAX)],
				left:	Some(Anchor::after(content(7, u64::MAX))),
				right:	None,
			},
			// A move of nothing to somewhere in particular: the degenerate shape
			// the codec still has to spell.
			Op::Move {
				src:	Vec::new(),
				left:	None,
				right:	Some(Anchor::before(content(1, 0))),
			},
			// A note on one run.
			Op::Note {
				on:		vec![range(1, 4, 19)],
				text:	b"this loop is quadratic".to_vec(),
			},
			// A note on content already fragmented across two atoms, whose text is
			// longer than a single byte length field could hold and is not UTF-8.
			Op::Note {
				on:		vec![
					range(2, 0, u64::MAX),
					range(5, 7, 9),
				],
				text:	vec![0xc3; 900],
			},
			// A note whose list carries an empty range beside a real one, which is
			// legal: what is refused is a note that names no byte at all.
			Op::Note {
				on:		vec![range(3, 5, 5), range(3, 5, 6)],
				text:	Vec::new(),
			},
		]
	}

	/// Headers spanning no parents, one, and many.
	fn sample_heads() -> Outcome<Vec<Header>> {
		Ok(vec![
			Header::root(oid(1, 1)),
			res!(Header::new(oid(2, 9), vec![oid(1, 1)])),
			res!(Header::new(oid(3, 4), vec![
				oid(1, 1),
				oid(2, 9),
				oid(9, u64::MAX),
			])),
			res!(Header::new(oid(4, u64::MAX), (1..=200)
				.map(|i| oid(i % 13, i))
				.collect())),
		])
	}

	#[test]
	fn op_dat_round_trip() -> Outcome<()> {
		for op in samples() {
			let back = res!(Op::from_dat(&op.to_dat()));
			assert_eq!(op, back, "variant {}", op.name());
		}
		Ok(())
	}

	#[test]
	fn op_byte_round_trip() -> Outcome<()> {
		for op in samples() {
			let buf = res!(op.encode());
			let back = res!(Op::decode_all(&buf));
			assert_eq!(op, back, "variant {}", op.name());
		}
		Ok(())
	}

	/// A payload longer than 255 bytes keeps its full length, which a `Dat::BU8`
	/// length field could not express.
	#[test]
	fn payloads_survive_beyond_a_byte_length() -> Outcome<()> {
		for len in [255usize, 256, 257, 4096, 70_000] {
			let op = Op::Splice {
				left:	Some(Anchor::origin(oid(1, 1))),
				right:	None,
				remove:	Vec::new(),
				insert:	vec![0x5a; len].into(),
			};
			let back = res!(Op::decode_all(&res!(op.encode())));
			match back {
				Op::Splice { insert, .. } => assert_eq!(insert.len(), len),
				other => return Err(err!(
					"Expected a Splice, got {}.", other.name(); Test, Mismatch)),
			}
			let op = Op::FileCreate { path: vec![0x2f; len] };
			match res!(Op::decode_all(&res!(op.encode()))) {
				Op::FileCreate { path } => assert_eq!(path.len(), len),
				other => return Err(err!(
					"Expected a FileCreate, got {}.", other.name(); Test, Mismatch)),
			}
			let op = Op::Note {
				on:		vec![range(1, 0, 1)],
				text:	vec![0x21; len],
			};
			match res!(Op::decode_all(&res!(op.encode()))) {
				Op::Note { text, .. } => assert_eq!(text.len(), len),
				other => return Err(err!(
					"Expected a Note, got {}.", other.name(); Test, Mismatch)),
			}
		}
		Ok(())
	}

	#[test]
	fn ops_decode_back_to_back() -> Outcome<()> {
		let ops = samples();
		let mut buf = Vec::new();
		for op in &ops {
			res!(op.encode_into(&mut buf));
		}
		let mut at = 0;
		for want in &ops {
			let (got, used) = res!(Op::decode(&buf[at..]));
			assert_eq!(&got, want);
			at += used;
		}
		assert_eq!(at, buf.len());
		Ok(())
	}

	#[test]
	fn codes_are_distinct() -> Outcome<()> {
		let mut seen = Vec::new();
		for op in samples() {
			let code = op.code();
			assert!(code != 0, "variant {} has a zero code", op.name());
			if !seen.contains(&(code, op.name())) {
				seen.push((code, op.name()));
			}
		}
		for (i, (code, name)) in seen.iter().enumerate() {
			for (other_code, other_name) in seen.iter().skip(i + 1) {
				assert!(
					code != other_code,
					"{} and {} share code {}", name, other_name, code,
				);
			}
		}
		Ok(())
	}

	#[test]
	fn only_a_lifecycle_change_names_a_file() -> Outcome<()> {
		assert_eq!(Op::FileDelete { file: oid(3, 1) }.names_file(), Some(oid(3, 1)));
		assert_eq!(
			Op::FileRename { file: oid(3, 1), path: b"x".to_vec() }.names_file(),
			Some(oid(3, 1)),
		);
		assert_eq!(Op::FileCreate { path: b"c.txt".to_vec() }.names_file(), None,
			"a file's creation is its identity, so it names nothing else");
		assert_eq!(Op::Mark { name: fmt!("v1"), body: None, time: None }.names_file(), None);
		assert_eq!(Op::Note {
			on:		vec![range(1, 0, 2)],
			text:	b"x".to_vec(),
		}.names_file(), None, "a note follows its content, wherever that is");
		assert_eq!(Op::Splice {
			left:	Some(Anchor::origin(oid(1, 1))),
			right:	None,
			remove:	Vec::new(),
			insert:	b"x".to_vec().into(),
		}.names_file(), None);
		assert_eq!(Op::Move {
			src:	Vec::new(),
			left:	Some(Anchor::origin(oid(1, 1))),
			right:	None,
		}.names_file(), None);
		Ok(())
	}

	#[test]
	fn a_placement_names_where_it_lands() -> Outcome<()> {
		// A splice inserting bytes with neither origin belongs to no file.
		let stray = Op::Splice {
			left:	None,
			right:	None,
			remove:	Vec::new(),
			insert:	b"x".to_vec().into(),
		};
		assert!(stray.check_placement().is_err());
		assert!(stray.validate().is_err());
		// And the decoder refuses it rather than leaving it to a later stage.
		assert!(Op::from_dat(&stray.to_dat()).is_err());
		assert!(Op::decode_all(&res!(stray.encode())).is_err());
		// Either origin alone satisfies the rule.
		for (left, right) in [
			(Some(Anchor::origin(oid(1, 1))), None),
			(None, Some(Anchor::before(content(1, 0)))),
		] {
			let op = Op::Splice { left, right, remove: Vec::new(), insert: b"x".to_vec().into() };
			res!(op.check_placement());
			assert_eq!(op, res!(Op::from_dat(&op.to_dat())));
		}
		// A splice that only removes places nothing and needs no origin.
		let del = Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![range(1, 0, 4)],
			insert:	Vec::new().into(),
		};
		res!(del.validate());
		assert_eq!(del, res!(Op::from_dat(&del.to_dat())));
		// A move always places what it names, so it always names where.
		let nowhere = Op::Move { src: vec![range(1, 0, 4)], left: None, right: None };
		assert!(nowhere.check_placement().is_err());
		assert!(Op::from_dat(&nowhere.to_dat()).is_err());
		// Even a move of nothing, since the rule is about the operation and not
		// about how much it happens to carry.
		let empty = Op::Move { src: Vec::new(), left: None, right: None };
		assert!(empty.check_placement().is_err());
		Ok(())
	}

	#[test]
	fn a_note_is_about_something() -> Outcome<()> {
		// An empty list names nothing.
		let vacant = Op::Note { on: Vec::new(), text: b"about what?".to_vec() };
		assert!(vacant.check_note().is_err());
		assert!(vacant.validate().is_err());
		assert!(Op::from_dat(&vacant.to_dat()).is_err());
		assert!(Op::decode_all(&res!(vacant.encode())).is_err());
		// A list of empty ranges names nothing either.
		let hollow = Op::Note {
			on:		vec![range(1, 3, 3), range(2, 0, 0)],
			text:	b"still nothing".to_vec(),
		};
		assert!(hollow.check_note().is_err());
		assert!(Op::from_dat(&hollow.to_dat()).is_err());
		// One byte is enough.
		let real = Op::Note {
			on:		vec![range(1, 3, 3), range(2, 0, 1)],
			text:	Vec::new(),
		};
		res!(real.validate());
		assert_eq!(real, res!(Op::from_dat(&real.to_dat())));
		// And every other variant is unaffected by the rule.
		for op in samples() {
			if matches!(op, Op::Note { .. }) {
				continue;
			}
			res!(op.check_note());
		}
		Ok(())
	}

	/// A note is not among the regions two operations could be in conflict over.
	#[test]
	fn a_note_refers_without_claiming() -> Outcome<()> {
		let note = Op::Note {
			on:		vec![range(1, 4, 9), range(2, 0, 3)],
			text:	b"see the ticket".to_vec(),
		};
		assert!(note.regions().is_empty(), "a note claims nothing");
		assert_eq!(note.note_on().len(), 2);
		assert_eq!(note.origins(), (None, None));
		assert_eq!(note.placed_len(), 0);
		assert!(!note.is_move());
		res!(note.check_placement());
		// The other variants refer to nothing, whatever they claim.
		assert!(Op::Move {
			src:	vec![range(1, 0, 4)],
			left:	Some(Anchor::origin(oid(9, 1))),
			right:	None,
		}.note_on().is_empty());
		assert!(Op::Mark { name: fmt!("v1"), body: None, time: None }.note_on().is_empty());
		Ok(())
	}

	#[test]
	fn an_operation_reports_its_origins_and_its_content() -> Outcome<()> {
		let mv = Op::Move {
			src:	vec![range(1, 0, 4), range(2, 0, 6)],
			left:	Some(Anchor::origin(oid(9, 1))),
			right:	None,
		};
		assert_eq!(mv.origins(), (Some(Anchor::origin(oid(9, 1))), None));
		assert_eq!(mv.regions().len(), 2);
		assert_eq!(mv.placed_len(), 10);
		assert!(mv.is_move());
		let create = Op::FileCreate { path: b"f".to_vec() };
		assert_eq!(create.origins(), (None, None));
		assert!(create.regions().is_empty());
		assert_eq!(create.placed_len(), 0);
		assert!(!create.is_move());
		Ok(())
	}

	/// An origin on the wrong side, or a move naming one byte twice.
	#[test]
	fn validate_refuses_what_cannot_be_resolved() -> Outcome<()> {
		let cid = content(1, 0);
		assert!(Op::Splice {
			left:	Some(Anchor::before(cid)),
			right:	None,
			remove:	Vec::new(),
			insert:	b"x".to_vec().into(),
		}.validate().is_err());
		assert!(Op::Splice {
			left:	None,
			right:	Some(Anchor::after(cid)),
			remove:	Vec::new(),
			insert:	b"x".to_vec().into(),
		}.validate().is_err());
		assert!(Op::Move {
			src:	vec![range(1, 0, 4), range(1, 2, 6)],
			left:	Some(Anchor::origin(oid(9, 1))),
			right:	None,
		}.validate().is_err());
		Ok(())
	}

	#[test]
	fn a_mode_is_one_of_three_things() -> Outcome<()> {
		assert_eq!(Mode::default(), Mode::Normal, "silence means an ordinary file");
		assert!(Mode::Normal.is_normal());
		assert!(!Mode::Executable.is_normal());
		assert!(!Mode::Symlink.is_normal());
		let all = [Mode::Normal, Mode::Executable, Mode::Symlink];
		for mode in all {
			assert_eq!(mode, res!(Mode::from_dat(&mode.to_dat())), "mode {}", mode);
			assert_eq!(fmt!("{}", mode), mode.name());
		}
		// The codes are distinct, and pinned: they are on the wire.
		assert_eq!(Mode::Normal.code(), 0);
		assert_eq!(Mode::Executable.code(), 1);
		assert_eq!(Mode::Symlink.code(), 2);
		// A fourth mode is refused rather than guessed at, and so is a spelling
		// that is not a number at all.
		assert!(Mode::from_dat(&Dat::U8(3)).is_err());
		assert!(Mode::from_dat(&Dat::U8(255)).is_err());
		assert!(Mode::from_dat(&Dat::Str(fmt!("executable"))).is_err());
		assert!(Mode::from_dat(&Dat::U64(1)).is_err());
		Ok(())
	}

	#[test]
	fn a_mode_operation_names_a_file() -> Outcome<()> {
		for mode in [Mode::Normal, Mode::Executable, Mode::Symlink] {
			let op = Op::FileMode { file: oid(3, 1), mode };
			assert_eq!(op.code(), CODE_FILE_MODE);
			assert_eq!(op.code(), 8, "the wire code is what the event fixed");
			assert_eq!(op.name(), "FileMode");
			// It is a lifecycle change, so it names its file the way a rename and
			// a delete do.
			assert_eq!(op.names_file(), Some(oid(3, 1)));
			// And it says nothing about content: it places nothing, claims
			// nothing and refers to nothing.
			assert_eq!(op.origins(), (None, None));
			assert!(op.regions().is_empty());
			assert!(op.note_on().is_empty());
			assert_eq!(op.placed_len(), 0);
			assert!(!op.is_move());
			res!(op.validate());
			assert_eq!(op, res!(Op::from_dat(&op.to_dat())));
			assert_eq!(op, res!(Op::decode_all(&res!(op.encode()))));
		}
		// The arity is exact, as it is everywhere else, and the mode is checked
		// on the way off the wire rather than left to a later stage.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_FILE_MODE),
			oid(3, 1).to_dat(),
		])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_FILE_MODE),
			oid(3, 1).to_dat(),
			Dat::U8(MODE_SYMLINK),
			Dat::U8(0),
		])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_FILE_MODE),
			oid(3, 1).to_dat(),
			Dat::U8(200),
		])).is_err());
		// A file named by something that is not an identifier.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_FILE_MODE),
			Dat::Str(fmt!("src/lib.rs")),
			Dat::U8(MODE_EXECUTABLE),
		])).is_err());
		Ok(())
	}

	/// The short spelling is the whole of the compatibility claim: a mark saying
	/// nothing beyond its name is [`CODE_MARK`] with two elements, byte for byte
	/// what was written before the fields existed, so every mark already signed
	/// still verifies.
	#[test]
	fn a_mark_has_two_spellings_and_one_variant() -> Outcome<()> {
		// The short spelling, pinned as a daticle and as bytes.
		let plain = Op::Mark { name: fmt!("v1"), body: None, time: None };
		assert_eq!(plain.code(), CODE_MARK);
		assert_eq!(plain.code(), 4, "the wire code a mark has always had");
		assert_eq!(plain.name(), "Mark");
		assert_eq!(plain.to_dat(), Dat::List(vec![
			Dat::U8(CODE_MARK),
			Dat::Str(fmt!("v1")),
		]));
		assert_eq!(
			res!(plain.encode()),
			vec![0x0a, 0x33, 0x21, 0x07, 0x0a, 0x04, 0x29, 0x21, 0x02, 0x76, 0x31],
			"the bytes of a bodyless, timeless mark have moved",
		);
		// The long spelling, at every combination that reaches it.
		for (body, time) in [
			(Some(b"why".to_vec()), None),
			(None, Some(1_755_400_329u64)),
			(Some(Vec::new()), Some(0)),
		] {
			let op = Op::Mark { name: fmt!("v1"), body: body.clone(), time };
			assert_eq!(op.code(), CODE_MARK_TIMED, "body {:?} time {:?}", body, time);
			assert_eq!(op.code(), 9);
			assert_eq!(op.name(), "Mark", "one variant, so one name");
			match op.to_dat() {
				Dat::List(v) => assert_eq!(v.len(), 4, "the long spelling is four"),
				other => return Err(err!(
					"A Mark encodes to {:?}.", other; Test, Mismatch)),
			}
			// And back, through both round trips.
			assert_eq!(op, res!(Op::from_dat(&op.to_dat())));
			assert_eq!(op, res!(Op::decode_all(&res!(op.encode()))));
		}
		// A code 4 mark and a code 9 mark decode to the same variant.
		let short = res!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK),
			Dat::Str(fmt!("v1")),
		])));
		let long = res!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK_TIMED),
			Dat::Str(fmt!("v1")),
			Dat::Opt(Box::new(None)),
			Dat::Opt(Box::new(Some(Dat::U64(9)))),
		])));
		assert!(matches!(short, Op::Mark { .. }));
		assert!(matches!(long, Op::Mark { .. }));
		assert_eq!(short.name(), long.name());
		assert_eq!(short, plain);
		match (&short, &long) {
			(
				Op::Mark { name: a, body: None, time: None },
				Op::Mark { name: b, body: None, time: Some(9) },
			) => assert_eq!(a, b),
			_ => return Err(err!(
				"The two spellings did not read back as one variant."; Test, Mismatch)),
		}
		// The arity of each spelling is exact, as it is everywhere else.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK),
			Dat::Str(fmt!("v1")),
			Dat::Opt(Box::new(None)),
			Dat::Opt(Box::new(None)),
		])).is_err(), "code 4 takes two elements");
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK_TIMED),
			Dat::Str(fmt!("v1")),
		])).is_err(), "code 9 takes four elements");
		// A body is bytes and a time is a number, not the other way about.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK_TIMED),
			Dat::Str(fmt!("v1")),
			Dat::Opt(Box::new(Some(Dat::Str(fmt!("not bytes"))))),
			Dat::Opt(Box::new(None)),
		])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK_TIMED),
			Dat::Str(fmt!("v1")),
			Dat::Opt(Box::new(None)),
			Dat::Opt(Box::new(Some(Dat::U8(9)))),
		])).is_err(), "a time is a Dat::U64 and nothing narrower");
		// A bare field where an optional one belongs.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK_TIMED),
			Dat::Str(fmt!("v1")),
			Dat::BU64(b"why".to_vec()),
			Dat::Opt(Box::new(None)),
		])).is_err());
		// And the long spelling is refused where it says nothing the short one
		// could not: an operation has one encoding.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MARK_TIMED),
			Dat::Str(fmt!("v1")),
			Dat::Opt(Box::new(None)),
			Dat::Opt(Box::new(None)),
		])).is_err(), "code 9 carrying neither is code 4 spelled twice");
		// A body beyond what a single byte length field could hold keeps its
		// length, which is why it is a BU64.
		for len in [255usize, 256, 70_000] {
			let op = Op::Mark {
				name:	fmt!("v1"),
				body:	Some(vec![0x5a; len]),
				time:	Some(1),
			};
			match res!(Op::decode_all(&res!(op.encode()))) {
				Op::Mark { body: Some(b), .. }	=> assert_eq!(b.len(), len),
				other => return Err(err!(
					"Expected a Mark with a body, got {}.", other.name();
				Test, Mismatch)),
			}
		}
		Ok(())
	}

	#[test]
	fn the_proposal_operations_round_trip() -> Outcome<()> {
		let prop = Op::Proposal {
			title:	fmt!("Carry a body on a mark"),
			body:	b"the case for it".to_vec(),
			voice:	fmt!("wren"),
			time:	1_755_400_000,
		};
		let said = Op::Said {
			on:		oid(3, 4),
			text:	b"agreed".to_vec(),
			voice:	fmt!("caf\u{e9}"),
			time:	1_755_400_100,
		};
		let settled = Op::Settled {
			on:		oid(3, 4),
			state:	Settled::Accepted,
			mark:	Some(oid(9, 2)),
			time:	1_755_400_200,
		};
		let reverts = Op::Reverts { undone: vec![oid(1, 1), oid(2, 9)] };
		for (op, code, name) in [
			(&prop,		CODE_PROPOSAL,	"Proposal"),
			(&said,		CODE_SAID,		"Said"),
			(&settled,	CODE_SETTLED,	"Settled"),
			(&reverts,	CODE_REVERTS,	"Reverts"),
		] {
			assert_eq!(op.code(), code, "variant {}", name);
			assert_eq!(op.name(), name);
			res!(op.validate());
			assert_eq!(*op, res!(Op::from_dat(&op.to_dat())));
			assert_eq!(*op, res!(Op::decode_all(&res!(op.encode()))));
		}
		// The codes are pinned: they are on the wire.
		assert_eq!(CODE_PROPOSAL, 10);
		assert_eq!(CODE_SAID, 11);
		assert_eq!(CODE_SETTLED, 12);
		assert_eq!(CODE_REVERTS, 13);
		// Every settled state survives, alongside both spellings of the mark that
		// closed the proposal.
		for state in [Settled::Open, Settled::Accepted, Settled::Declined, Settled::Done] {
			for mark in [None, Some(oid(9, 2))] {
				let op = Op::Settled { on: oid(3, 4), state, mark, time: 7 };
				assert_eq!(op, res!(Op::decode_all(&res!(op.encode()))), "state {}", state);
			}
		}
		// The arities are exact.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_PROPOSAL),
			Dat::Str(fmt!("t")),
			Dat::BU64(Vec::new()),
			Dat::Str(fmt!("v")),
		])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_REVERTS),
			Dat::List(Vec::new()),
			Dat::U64(0),
		])).is_err());
		// A body is bytes and a title is a string, not the other way about.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_PROPOSAL),
			Dat::BU64(b"t".to_vec()),
			Dat::BU64(Vec::new()),
			Dat::Str(fmt!("v")),
			Dat::U64(0),
		])).is_err());
		// A time is a Dat::U64 and nothing narrower.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SAID),
			oid(3, 4).to_dat(),
			Dat::BU64(Vec::new()),
			Dat::Str(fmt!("v")),
			Dat::U8(0),
		])).is_err());
		// A proposal is named by an identifier, never by a title.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SAID),
			Dat::Str(fmt!("Carry a body on a mark")),
			Dat::BU64(Vec::new()),
			Dat::Str(fmt!("v")),
			Dat::U64(0),
		])).is_err());
		// A mark is named by an identifier too, and optionally.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SETTLED),
			oid(3, 4).to_dat(),
			Dat::U8(SETTLED_DONE),
			Dat::Str(fmt!("v1")),
			Dat::U64(0),
		])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SETTLED),
			oid(3, 4).to_dat(),
			Dat::U8(SETTLED_DONE),
			oid(9, 2).to_dat(),
			Dat::U64(0),
		])).is_err(), "a bare identifier where an optional one belongs");
		Ok(())
	}

	#[test]
	fn a_settled_state_is_one_of_four_things() -> Outcome<()> {
		assert_eq!(Settled::default(), Settled::Open, "silence means still asking");
		assert!(Settled::Open.is_open());
		assert!(!Settled::Accepted.is_open());
		for state in [Settled::Open, Settled::Accepted, Settled::Declined, Settled::Done] {
			assert_eq!(state, res!(Settled::from_dat(&state.to_dat())), "state {}", state);
			assert_eq!(fmt!("{}", state), state.name());
		}
		// The codes are distinct, and pinned: they are on the wire.
		assert_eq!(Settled::Open.code(), 0);
		assert_eq!(Settled::Accepted.code(), 1);
		assert_eq!(Settled::Declined.code(), 2);
		assert_eq!(Settled::Done.code(), 3);
		// A fifth state is refused rather than guessed at, and so is a spelling
		// that is not a number at all.
		assert!(Settled::from_dat(&Dat::U8(4)).is_err());
		assert!(Settled::from_dat(&Dat::U8(255)).is_err());
		assert!(Settled::from_dat(&Dat::Str(fmt!("accepted"))).is_err());
		assert!(Settled::from_dat(&Dat::U64(1)).is_err());
		// And an operation carrying one is refused with it.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SETTLED),
			oid(3, 4).to_dat(),
			Dat::U8(4),
			Dat::Opt(Box::new(None)),
			Dat::U64(0),
		])).is_err());
		Ok(())
	}

	/// The ordering rule is [`Header`]'s parents, for the same reason: two byte
	/// spellings of one set would both verify against a signature. The rule that
	/// it names anything at all is [`Op::check_note`]'s, for the same reason: a
	/// revert of nothing is a mark with extra spelling.
	#[test]
	fn a_revert_names_what_it_undoes_once_in_order() -> Outcome<()> {
		// In order, with no repetition, at every length above nothing.
		for undone in [
			vec![oid(1, 1)],
			vec![oid(1, 1), oid(1, 2), oid(2, 1), oid(9, u64::MAX)],
		] {
			let op = Op::Reverts { undone: undone.clone() };
			res!(op.check_reverts());
			res!(op.validate());
			assert_eq!(op, res!(Op::from_dat(&op.to_dat())));
			assert_eq!(op, res!(Op::decode_all(&res!(op.encode()))));
		}
		// Naming nothing, out of order, and repeated, on the way into the structure
		// and on the way off the wire alike.
		for undone in [
			Vec::new(),
			vec![oid(2, 1), oid(1, 1)],
			vec![oid(1, 2), oid(1, 1)],
			vec![oid(1, 1), oid(1, 1)],
			vec![oid(1, 1), oid(2, 1), oid(2, 1)],
		] {
			let op = Op::Reverts { undone: undone.clone() };
			assert!(op.check_reverts().is_err(), "list {:?}", undone);
			assert!(op.validate().is_err());
			assert!(Op::from_dat(&op.to_dat()).is_err());
			assert!(Op::decode_all(&res!(op.encode())).is_err());
		}
		// An empty list is refused for saying nothing, not for being out of order,
		// so the message sends its author to the operation that does say something
		// about a point in history.
		let vacant = Op::Reverts { undone: Vec::new() };
		let e = match vacant.check_reverts() {
			Ok(()) => return Err(err!(
				"A Reverts naming nothing was accepted."; Test)),
			Err(e) => e,
		};
		let msg = fmt!("{}", e);
		assert!(msg.contains("Mark"), "message was {}", msg);
		// Every other variant is unaffected by the rule.
		for op in samples() {
			if matches!(op, Op::Reverts { .. }) {
				continue;
			}
			res!(op.check_reverts());
		}
		// What it names are identifiers, not names.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_REVERTS),
			Dat::List(vec![Dat::Str(fmt!("v1"))]),
		])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_REVERTS),
			Dat::Str(fmt!("not a list")),
		])).is_err());
		Ok(())
	}

	/// This is what every catch-all arm in [`crate::seq`] is relying on: an
	/// operation that mints no atom and claims no byte is carried for the sake
	/// of the causal graph and does nothing to the render.
	#[test]
	fn the_history_operations_touch_no_bytes() -> Outcome<()> {
		for op in samples() {
			match op {
				Op::Mark { .. }
				| Op::Note { .. }
				| Op::Proposal { .. }
				| Op::Said { .. }
				| Op::Settled { .. }
				| Op::Reverts { .. } => (),
				_ => continue,
			}
			assert_eq!(op.origins(), (None, None), "variant {}", op.name());
			assert!(op.regions().is_empty(), "variant {} claims content", op.name());
			assert_eq!(op.placed_len(), 0, "variant {} places bytes", op.name());
			assert!(!op.is_move());
			assert_eq!(op.names_file(), None, "variant {} names a file", op.name());
			res!(op.check_placement());
			// Only a note refers to content, and it is the one that resolves into
			// spans; the rest refer to operations or to nothing.
			if !matches!(op, Op::Note { .. }) {
				assert!(op.note_on().is_empty(), "variant {} refers to content", op.name());
				res!(op.check_note());
			}
		}
		Ok(())
	}

	#[test]
	fn op_from_dat_rejects_rubbish() -> Outcome<()> {
		assert!(Op::from_dat(&Dat::U8(CODE_MARK)).is_err());
		assert!(Op::from_dat(&Dat::List(vec![])).is_err());
		assert!(Op::from_dat(&Dat::List(vec![Dat::U8(200), Dat::Str(fmt!("x"))])).is_err());
		// Right code, wrong arity.
		assert!(Op::from_dat(&Dat::List(vec![Dat::U8(CODE_FILE_RENAME)])).is_err());
		// Right arity, wrong field kind: a path that is a string rather than
		// bytes, which is exactly what the old vocabulary spelled.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_FILE_CREATE),
			Dat::Str(fmt!("src/lib.rs")),
		])).is_err());
		// A file named by something that is not an identifier.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_FILE_DELETE),
			Dat::Str(fmt!("src/lib.rs")),
		])).is_err());
		// A Splice whose payload is not a byte vector.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SPLICE),
			Anchor::opt_to_dat(&Some(Anchor::origin(oid(1, 1)))),
			Anchor::opt_to_dat(&None),
			Dat::List(vec![]),
			Dat::Str(fmt!("not bytes")),
		])).is_err());
		// A Splice whose removed runs are not ranges.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_SPLICE),
			Anchor::opt_to_dat(&None),
			Anchor::opt_to_dat(&None),
			Dat::Str(fmt!("not ranges")),
			Dat::BU64(Vec::new()),
		])).is_err());
		// A Move whose source is not a list of ranges.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MOVE),
			Dat::Str(fmt!("not ranges")),
			Anchor::opt_to_dat(&Some(Anchor::origin(oid(1, 1)))),
			Anchor::opt_to_dat(&None),
		])).is_err());
		// A Move whose anchor is bare rather than optional.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_MOVE),
			Dat::List(vec![range(1, 0, 2).to_dat()]),
			Anchor::after(content(1, 0)).to_dat(),
			Anchor::opt_to_dat(&None),
		])).is_err());
		// A Note at the wrong arity.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_NOTE),
			Dat::List(vec![range(1, 0, 2).to_dat()]),
		])).is_err());
		// A Note whose subject is not a list of ranges.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_NOTE),
			Dat::Str(fmt!("not ranges")),
			Dat::BU64(b"text".to_vec()),
		])).is_err());
		// A Note whose text is a string rather than bytes, which is the mistake a
		// reader of Op::Mark would make.
		assert!(Op::from_dat(&Dat::List(vec![
			Dat::U8(CODE_NOTE),
			Dat::List(vec![range(1, 0, 2).to_dat()]),
			Dat::Str(fmt!("not bytes")),
		])).is_err());
		Ok(())
	}

	#[test]
	fn move_keeps_its_source_runs_in_order() -> Outcome<()> {
		let src: Vec<ContentRange> = (0..300u64)
			.map(|i| range(i % 7 + 1, i, i + 5))
			.collect();
		let op = Op::Move {
			src:	src.clone(),
			left:	Some(Anchor::after(content(1, 9))),
			right:	None,
		};
		for back in [
			res!(Op::from_dat(&op.to_dat())),
			res!(Op::decode_all(&res!(op.encode()))),
		] {
			match back {
				Op::Move { src: got, .. } => assert_eq!(got, src),
				other => return Err(err!(
					"Expected a Move, got {}.", other.name(); Test, Mismatch)),
			}
		}
		Ok(())
	}

	/// The side decides whether an insertion abutting the moved run travels with
	/// it.
	#[test]
	fn move_anchors_keep_their_side() -> Outcome<()> {
		let cid = content(2, 11);
		for (left, right) in [
			(Some(Anchor::after(cid)), None),
			(None, Some(Anchor::before(cid))),
			(Some(Anchor::after(cid)), Some(Anchor::before(cid))),
			// The sides the sequence structure refuses are still spelled
			// faithfully; refusing them is the structure's business, not the
			// codec's. What the codec does refuse is an operation carrying no
			// origin at all, which belongs to no file.
			(Some(Anchor::before(cid)), Some(Anchor::after(cid))),
		] {
			let op = Op::Move {
				src:	vec![range(1, 0, 3)],
				left,
				right,
			};
			assert_eq!(op, res!(Op::decode_all(&res!(op.encode()))));
			assert_eq!(op, res!(Op::from_dat(&op.to_dat())));
			// A splice carrying the same origins spells them the same way.
			let sp = Op::Splice {
				left,
				right,
				remove:	vec![range(1, 0, 3)],
				insert:	b"x".to_vec().into(),
			};
			assert_eq!(sp, res!(Op::decode_all(&res!(sp.encode()))));
			assert_eq!(sp, res!(Op::from_dat(&sp.to_dat())));
		}
		Ok(())
	}

	/// Under the identity hasher the result is exactly the canonical encoding.
	#[test]
	fn op_hashes_its_canonical_encoding() -> Outcome<()> {
		let op = sample_op_for_hashing();
		let want = res!(op.encode());
		let got = res!(op.hash((), [0u8; 0])).as_vec();
		assert_eq!(got, want);
		// An operation differing in one field hashes differently.
		let other = Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![range(1, 12, 16)],
			insert:	Vec::new().into(),
		};
		assert!(res!(other.hash((), [0u8; 0])).as_vec() != want);
		Ok(())
	}

	fn sample_op_for_hashing() -> Op {
		Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![range(1, 12, 15)],
			insert:	Vec::new().into(),
		}
	}

	#[test]
	fn op_decode_rejects_truncation() -> Outcome<()> {
		let op = Op::Splice {
			left:	Some(Anchor::after(content(1, 0))),
			right:	None,
			remove:	vec![range(1, 1, 3)],
			insert:	b"abcdef".to_vec().into(),
		};
		let buf = res!(op.encode());
		for cut in 1..buf.len() {
			assert!(Op::decode(&buf[..cut]).is_err(), "cut at {}", cut);
		}
		let note = Op::Note {
			on:		vec![range(1, 1, 3), range(2, 0, 8)],
			text:	b"a note that is cut short".to_vec(),
		};
		let buf = res!(note.encode());
		for cut in 1..buf.len() {
			assert!(Op::decode(&buf[..cut]).is_err(), "note cut at {}", cut);
		}
		Ok(())
	}

	#[test]
	fn header_round_trips_at_every_arity() -> Outcome<()> {
		for head in res!(sample_heads()) {
			assert_eq!(head, res!(Header::from_dat(&head.to_dat())));
			let rec = Record::new(head.clone(), Op::Mark { name: fmt!("m"), body: None, time: None });
			assert_eq!(rec, res!(Record::decode_all(&res!(rec.encode()))));
		}
		Ok(())
	}

	/// The same frontier given in any order has one encoding.
	#[test]
	fn parents_are_canonical() -> Outcome<()> {
		let a = res!(Header::new(oid(9, 1), vec![oid(1, 2), oid(3, 1), oid(1, 2)]));
		let b = res!(Header::new(oid(9, 1), vec![oid(3, 1), oid(1, 2)]));
		assert_eq!(a, b);
		assert_eq!(a.parents(), &[oid(1, 2), oid(3, 1)]);
		assert_eq!(a.id(), oid(9, 1));
		assert_eq!(res!(a.to_dat().to_bytes(Vec::new())), res!(b.to_dat().to_bytes(Vec::new())));
		// The decoder refuses the non-canonical spellings the constructor fixes.
		let unsorted = Dat::List(vec![
			oid(9, 1).to_dat(),
			Dat::List(vec![oid(3, 1).to_dat(), oid(1, 2).to_dat()]),
		]);
		assert!(Header::from_dat(&unsorted).is_err());
		let repeated = Dat::List(vec![
			oid(9, 1).to_dat(),
			Dat::List(vec![oid(1, 2).to_dat(), oid(1, 2).to_dat()]),
		]);
		assert!(Header::from_dat(&repeated).is_err());
		Ok(())
	}

	#[test]
	fn an_operation_is_not_its_own_parent() -> Outcome<()> {
		assert!(Header::new(oid(1, 4), vec![oid(2, 1), oid(1, 4)]).is_err());
		let itself = Dat::List(vec![
			oid(1, 4).to_dat(),
			Dat::List(vec![oid(1, 4).to_dat()]),
		]);
		assert!(Header::from_dat(&itself).is_err());
		Ok(())
	}

	#[test]
	fn a_root_header_has_no_parents() -> Outcome<()> {
		let head = Header::root(oid(1, 1));
		assert!(head.is_root());
		assert!(head.parents().is_empty());
		assert!(!res!(Header::new(oid(1, 2), vec![oid(1, 1)])).is_root());
		Ok(())
	}

	#[test]
	fn record_round_trips_every_variant() -> Outcome<()> {
		let head = res!(Header::new(oid(5, 7), vec![oid(1, 1), oid(2, 2)]));
		for op in samples() {
			let rec = Record::new(head.clone(), op);
			assert_eq!(rec, res!(Record::from_dat(&rec.to_dat())));
			assert_eq!(rec, res!(Record::decode_all(&res!(rec.encode()))));
		}
		assert!(Record::from_dat(&Dat::U8(1)).is_err());
		assert!(Record::from_dat(&Dat::List(vec![Dat::U8(1)])).is_err());
		Ok(())
	}

	#[test]
	fn the_parents_are_covered_by_the_hash() -> Outcome<()> {
		let op = Op::Mark { name: fmt!("v1"), body: None, time: None };
		let one = Record::new(res!(Header::new(oid(1, 5), vec![oid(2, 1)])), op.clone());
		let two = Record::new(res!(Header::new(oid(1, 5), vec![oid(2, 2)])), op);
		assert!(
			res!(one.hash((), [0u8; 0])).as_vec() != res!(two.hash((), [0u8; 0])).as_vec(),
			"re-parenting must change the hash",
		);
		Ok(())
	}

	#[test]
	fn record_decode_rejects_truncation() -> Outcome<()> {
		let rec = Record::new(
			res!(Header::new(oid(2, 3), vec![oid(1, 1), oid(1, 2)])),
			Op::Splice {
				left:	Some(Anchor::origin(oid(1, 1))),
				right:	None,
				remove:	Vec::new(),
				insert:	b"abcdef".to_vec().into(),
			},
		);
		let buf = res!(rec.encode());
		for cut in 1..buf.len() {
			assert!(Record::decode(&buf[..cut]).is_err(), "cut at {}", cut);
		}
		Ok(())
	}

	#[test]
	fn records_decode_back_to_back() -> Outcome<()> {
		let heads = res!(sample_heads());
		let recs: Vec<Record> = samples()
			.into_iter()
			.enumerate()
			.map(|(i, op)| Record::new(heads[i % heads.len()].clone(), op))
			.collect();
		let mut buf = Vec::new();
		for rec in &recs {
			res!(rec.encode_into(&mut buf));
		}
		let mut at = 0;
		for want in &recs {
			let (got, used) = res!(Record::decode(&buf[at..]));
			assert_eq!(&got, want);
			at += used;
		}
		assert_eq!(at, buf.len());
		Ok(())
	}

	/// By one character and by nothing else: what a tool spells after the prefix
	/// is that tool's business, and two of them may spell it differently.
	#[test]
	fn a_mark_a_tool_wrote_is_known_by_its_first_character() -> Outcome<()> {
		assert!(is_auto_mark("@2026-08-17T04:12:09.482913Z"));
		assert!(is_auto_mark("@"), "the prefix alone is still the prefix");
		assert!(is_auto_mark("@whatever a tool likes to spell"),
			"nothing after the prefix is read");

		assert!(!is_auto_mark("release 1.0"));
		assert!(!is_auto_mark(""), "a nameless mark is nobody's automatic one");
		assert!(!is_auto_mark("2026-08-17T04:12:09Z"),
			"a datetime is not the convention; the prefix is");
		assert!(!is_auto_mark(" @2026-08-17T04:12:09Z"),
			"the character begins the name or it does not count");

		// And through an operation, which is how every consumer meets one.
		let op = Op::Mark {
			name:	fmt!("{}2026-08-17T04:12:09.482913Z", AUTO_MARK_PREFIX),
			body:	None,
			time:	Some(1_755_403_929),
		};
		match &op {
			Op::Mark { name, .. }	=> assert!(is_auto_mark(name)),
			other					=> return Err(err!(
				"A mark decoded as {}.", other.name(); Test, Mismatch)),
		}
		// It carries a time, so it is the four element spelling.
		assert_eq!(op.code(), CODE_MARK_TIMED);
		Ok(())
	}

	/// This is what stops a new operation joining the vocabulary and being
	/// silently undoable by nothing: [`Op::undoing`] ends on an arm that fails,
	/// so the pair of answers has to stay exhaustive.
	#[test]
	fn every_operation_is_undone_or_says_why_not() -> Outcome<()> {
		let refused = [
			CODE_FILE_DELETE, CODE_MARK, CODE_MARK_TIMED, CODE_NOTE,
			CODE_PROPOSAL, CODE_SAID, CODE_SETTLED, CODE_REVERTS,
		];
		for op in samples() {
			let id = oid(77, 3);
			match op.no_inverse() {
				Some(why) => {
					assert!(refused.contains(&op.code()),
						"the {} at code {} refuses an inverse", op.name(), op.code());
					// The refusal says why, and the sentence reaches whoever asked.
					assert!(why.len() > 20, "the {} gives no reason", op.name());
					let e = match op.undoing(id) {
						Ok(_) => return Err(err!(
							"The {} was undone though nothing undoes it.", op.name(); Test)),
						Err(e) => e,
					};
					assert!(fmt!("{}", e).contains(why),
						"the {} refuses without saying why", op.name());
				},
				None => {
					assert!(!refused.contains(&op.code()),
						"the {} at code {} has an inverse", op.name(), op.code());
					let undoing = res!(op.undoing(id));
					// An inverse that is nothing at all would be a silent refusal,
					// which is the failure this pair of answers exists to prevent.
					assert!(
						!undoing.written.is_empty()
							|| !undoing.copies.is_empty()
							|| undoing.prior.is_some(),
						"the {} is undone by nothing at all", op.name());
					for inverse in &undoing.written {
						res!(inverse.validate());
					}
				},
			}
		}
		Ok(())
	}

	/// Only one half is exact. The insertion is named by the operation that made
	/// it, so its inverse names that identity and no render is needed; what the
	/// splice removed can come back only as a copy.
	#[test]
	fn a_splice_is_undone_in_two_halves() -> Outcome<()> {
		let id = oid(5, 12);
		// A replacement: it inserted five bytes and killed two runs.
		let op = Op::Splice {
			left:	Some(Anchor::after(content(1, 3))),
			right:	None,
			remove:	vec![range(1, 4, 9), range(2, 0, 2)],
			insert:	b"hello".to_vec().into(),
		};
		let undoing = res!(op.undoing(id));
		assert_eq!(undoing.prior, None, "a splice records everything its inverse needs");
		assert_eq!(undoing.copies, vec![range(1, 4, 9), range(2, 0, 2)]);
		// The insertion half names the atom the splice minted, whole, and inserts
		// nothing, so it carries no origin and needs none.
		assert_eq!(undoing.written, vec![Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![res!(ContentRange::new(id, 0, 5))],
			insert:	Vec::new().into(),
		}]);
		res!(undoing.written[0].validate());

		// A pure insertion has an exact inverse and nothing to copy.
		let op = Op::Splice {
			left:	Some(Anchor::origin(oid(1, 1))),
			right:	None,
			remove:	Vec::new(),
			insert:	b"abc".to_vec().into(),
		};
		let undoing = res!(op.undoing(id));
		assert!(undoing.copies.is_empty(), "an insertion buried nothing");
		assert_eq!(undoing.written.len(), 1);

		// A pure deletion minted no atom, so there is nothing to remove and the
		// whole of its inverse is copies.
		let op = Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![range(1, 4, 9)],
			insert:	Vec::new().into(),
		};
		let undoing = res!(op.undoing(id));
		assert!(undoing.written.is_empty(), "a deletion minted nothing to take back");
		assert_eq!(undoing.copies, vec![range(1, 4, 9)]);

		// An empty run in a remove list names no byte, so it is not a copy owed.
		let op = Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![range(1, 4, 4), range(1, 6, 8)],
			insert:	Vec::new().into(),
		};
		assert_eq!(res!(op.undoing(id)).copies, vec![range(1, 6, 8)]);
		Ok(())
	}

	#[test]
	fn undoing_an_assertion_asks_the_state_it_replaced() -> Outcome<()> {
		let file = oid(2, 5);
		let undoing = res!(Op::FileRename { file, path: b"b.txt".to_vec() }
			.undoing(oid(3, 1)));
		assert_eq!(undoing.prior, Some(Prior::Path { file }));
		assert!(undoing.written.is_empty() && undoing.copies.is_empty());

		let undoing = res!(Op::FileMode { file, mode: Mode::Executable }
			.undoing(oid(3, 2)));
		assert_eq!(undoing.prior, Some(Prior::Mode { file }));

		let undoing = res!(Op::Move {
			src:	vec![range(1, 0, 4), range(1, 9, 9), range(2, 2, 6)],
			left:	Some(Anchor::after(content(3, 0))),
			right:	None,
		}.undoing(oid(3, 3)));
		// The empty run is dropped: it names no byte, so it has no former place.
		assert_eq!(undoing.prior, Some(Prior::Place {
			src: vec![range(1, 0, 4), range(2, 2, 6)],
		}));

		// A file's creation is the one that needs nothing looked up, the file
		// having had no prior state at all. It is also the one undoing that cannot
		// itself be undone, a deletion having no inverse.
		let made = oid(4, 1);
		let undoing = res!(Op::FileCreate { path: b"a.txt".to_vec() }.undoing(made));
		assert_eq!(undoing.prior, None);
		assert_eq!(undoing.written, vec![Op::FileDelete { file: made }]);
		assert!(undoing.written[0].no_inverse().is_some(),
			"undoing a creation is a one way journey and the vocabulary should say so");
		Ok(())
	}

	/// The anchor is the only record connecting a copy to the writing it is a
	/// copy of: the bytes take a new identity the moment they come back, so an
	/// accounting that read only the identity would credit whoever reverted.
	#[test]
	fn a_copy_says_what_it_is_a_copy_of() -> Outcome<()> {
		for was in [range(1, 0, 5), range(1, 4, 9), range(7, 0, 1)] {
			let bytes = vec![b'x'; was.len() as usize];
			let op = res!(Op::restoring(&was, bytes.clone()));
			res!(op.validate());
			// It binds after the last byte of the run, which is where the run is
			// buried, whatever became of what used to surround it.
			assert_eq!(op.origins().0, Some(Anchor::after(
				ContentId::new(was.op(), was.to() - 1))));
			assert_eq!(op.origins().1, None);
			assert!(op.regions().is_empty(), "a restoration kills nothing");
			assert_eq!(op.restored(), Some(was), "the range {}", was);
			// And it survives the wire, which is where a reader meets it.
			assert_eq!(res!(Op::from_dat(&op.to_dat())).restored(), Some(was));
		}
		// A copy of nothing, and a copy of the wrong length, are refused: a
		// restoration puts back what was there.
		assert!(Op::restoring(&range(1, 4, 4), Vec::new()).is_err());
		assert!(Op::restoring(&range(1, 4, 9), b"abc".to_vec()).is_err());
		assert!(Op::restoring(&range(1, 4, 9), Vec::new()).is_err());

		// Nothing else answers. A splice that removes, one bounded on the right,
		// and one anchored before a byte are all ordinary edits.
		for op in [
			Op::Splice {
				left:	Some(Anchor::after(content(1, 4))),
				right:	None,
				remove:	vec![range(2, 0, 1)],
				insert:	b"ab".to_vec().into(),
			},
			Op::Splice {
				left:	Some(Anchor::after(content(1, 4))),
				right:	Some(Anchor::before(content(2, 0))),
				remove:	Vec::new(),
				insert:	b"ab".to_vec().into(),
			},
			Op::Splice {
				left:	None,
				right:	Some(Anchor::before(content(2, 0))),
				remove:	Vec::new(),
				insert:	b"ab".to_vec().into(),
			},
		] {
			assert_eq!(op.restored(), None, "the {} answered", op.name());
		}
		for op in samples() {
			if !matches!(op, Op::Splice { .. }) {
				assert_eq!(op.restored(), None, "the {} answered", op.name());
			}
		}
		// A copy longer than the offset it is anchored at names a run reaching
		// back past the start of its own atom, which nothing ever wrote.
		assert_eq!(Op::Splice {
			left:	Some(Anchor::after(content(1, 1))),
			right:	None,
			remove:	Vec::new(),
			insert:	b"abcdef".to_vec().into(),
		}.restored(), None);
		Ok(())
	}
}
