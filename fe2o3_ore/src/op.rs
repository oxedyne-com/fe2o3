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

use crate::id::{
	varint_decode,
	varint_encode,
	Anchor,
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


/// Wire code for [`Op::FileCreate`].
pub const CODE_FILE_CREATE:	u8 = 1;
/// Wire code for [`Op::FileDelete`].
pub const CODE_FILE_DELETE:	u8 = 2;
/// Wire code for [`Op::FileRename`].
pub const CODE_FILE_RENAME:	u8 = 3;
/// Wire code for [`Op::Mark`].
pub const CODE_MARK:		u8 = 4;
/// Wire code for [`Op::Splice`].
pub const CODE_SPLICE:		u8 = 5;
/// Wire code for [`Op::Move`].
pub const CODE_MOVE:		u8 = 6;
/// Wire code for [`Op::Note`].
pub const CODE_NOTE:		u8 = 7;
/// Wire code for [`Op::FileMode`].
pub const CODE_FILE_MODE:	u8 = 8;
/// Wire code for an [`Op::Mark`] carrying a body, a time, or both.
pub const CODE_MARK_TIMED:	u8 = 9;
/// Wire code for [`Op::Proposal`].
pub const CODE_PROPOSAL:	u8 = 10;
/// Wire code for [`Op::Said`].
pub const CODE_SAID:		u8 = 11;
/// Wire code for [`Op::Settled`].
pub const CODE_SETTLED:		u8 = 12;
/// Wire code for [`Op::Reverts`].
pub const CODE_REVERTS:		u8 = 13;


/// Wire code for [`Mode::Normal`].
pub const MODE_NORMAL:		u8 = 0;
/// Wire code for [`Mode::Executable`].
pub const MODE_EXECUTABLE:	u8 = 1;
/// Wire code for [`Mode::Symlink`].
pub const MODE_SYMLINK:		u8 = 2;


/// Wire code for [`Settled::Open`].
pub const SETTLED_OPEN:		u8 = 0;
/// Wire code for [`Settled::Accepted`].
pub const SETTLED_ACCEPTED:	u8 = 1;
/// Wire code for [`Settled::Declined`].
pub const SETTLED_DECLINED:	u8 = 2;
/// Wire code for [`Settled::Done`].
pub const SETTLED_DONE:		u8 = 3;


/// What a file is, over and above the bytes in it.
///
/// A working copy records exactly one bit of metadata about an ordinary file --
/// whether it may be run -- and one further kind of thing that lives at a path
/// and holds bytes, which is a symbolic link. So this is a three-value enum and
/// not a number: a mode outside the set is not a state a working copy can be
/// in, and a reader should not have to guess what one would mean.
///
/// [`Mode::Normal`] is the default, and that is what makes the operation
/// additive: a file no [`Op::FileMode`] ever named is a normal file, so every
/// history written before the operation existed means today what it meant
/// yesterday.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Mode {
	/// An ordinary file, which is what a file is until something says otherwise.
	#[default]
	Normal,
	/// A file a working copy marks executable.
	Executable,
	/// A symbolic link, whose bytes are the path it points at.
	Symlink,
}

impl Mode {
	/// Returns the wire code identifying the mode.
	pub const fn code(&self) -> u8 {
		match self {
			Self::Normal		=> MODE_NORMAL,
			Self::Executable	=> MODE_EXECUTABLE,
			Self::Symlink		=> MODE_SYMLINK,
		}
	}

	/// Returns the mode's name, for messages and logs.
	pub const fn name(&self) -> &'static str {
		match self {
			Self::Normal		=> "normal",
			Self::Executable	=> "executable",
			Self::Symlink		=> "symlink",
		}
	}

	/// Reports whether the mode is the one a file has said nothing about.
	pub const fn is_normal(&self) -> bool {
		matches!(self, Self::Normal)
	}

	/// Serialises the mode as the single [`Dat::U8`] it is spelled as.
	pub const fn to_dat(&self) -> Dat {
		Dat::U8(self.code())
	}

	/// Reconstructs a mode from a [`Dat`] produced by [`Mode::to_dat`].
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
/// Shaped like [`Mode`], and for the same reason: a state outside the set is not
/// a state a proposal can be in, and a reader should not have to guess what one
/// would mean. There is no transition table beside it, because a state is
/// asserted rather than stepped to -- an author says what a proposal now is, and
/// the assertion stands until another one is written.
///
/// [`Settled::Open`] is the default, so an [`Op::Proposal`] that nothing has
/// settled yet is open without anything having had to say so.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Settled {
	/// Still being asked for, which is what a proposal is until something says
	/// otherwise.
	#[default]
	Open,
	/// Agreed to.
	Accepted,
	/// Refused.
	Declined,
	/// Agreed to and carried out.
	Done,
}

impl Settled {
	/// Returns the wire code identifying the state.
	pub const fn code(&self) -> u8 {
		match self {
			Self::Open		=> SETTLED_OPEN,
			Self::Accepted	=> SETTLED_ACCEPTED,
			Self::Declined	=> SETTLED_DECLINED,
			Self::Done		=> SETTLED_DONE,
		}
	}

	/// Returns the state's name, for messages and logs.
	pub const fn name(&self) -> &'static str {
		match self {
			Self::Open		=> "open",
			Self::Accepted	=> "accepted",
			Self::Declined	=> "declined",
			Self::Done		=> "done",
		}
	}

	/// Reports whether the proposal is still being asked for.
	pub const fn is_open(&self) -> bool {
		matches!(self, Self::Open)
	}

	/// Serialises the state as the single [`Dat::U8`] it is spelled as.
	pub const fn to_dat(&self) -> Dat {
		Dat::U8(self.code())
	}

	/// Reconstructs a state from a [`Dat`] produced by [`Settled::to_dat`].
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
	/// Brings a file into existence, empty.
	///
	/// The operation's own identity is the file's identity, for as long as the
	/// history lasts and through any number of renames. It also mints the file's
	/// origin anchor, one byte of content born dead and named
	/// [`crate::id::ContentId::origin`] of that identity, which is what a splice
	/// into the empty file binds after.
	FileCreate {
		/// Where the file sits, as bytes rather than as a string.
		path: Vec<u8>,
	},
	/// Retires a file. Its content is held back rather than destroyed, so
	/// whatever moved out of it before it went still renders where it went.
	FileDelete {
		/// The file, named by the operation that created it.
		file: OpId,
	},
	/// Moves a file to another path, leaving its contents untouched.
	FileRename {
		/// The file, named by the operation that created it.
		file: OpId,
		/// Where it moves to.
		path: Vec<u8>,
	},
	/// Asserts what a file is, from this point on, leaving its contents
	/// untouched.
	///
	/// Shaped like [`Op::FileRename`], and for the same reason: it names a file
	/// by identity and states a piece of that file's metadata, so the assertion
	/// survives every rename and every edit the file goes on to have. A file no
	/// such operation names is [`Mode::Normal`], which is why the operation could
	/// join the vocabulary without disturbing a byte of what was written before
	/// it.
	///
	/// Two of these written concurrently settle the way two concurrent renames
	/// settle: the later in operation order is the one the render reports.
	FileMode {
		/// The file, named by the operation that created it.
		file: OpId,
		/// What the file is from here on.
		mode: Mode,
	},
	/// Names a point in history, so that it can be referred to later, and says
	/// what was going on when it was named.
	///
	/// The body and the time joined the operation after it had already been
	/// written a great many times, so both are optional and the wire carries two
	/// spellings of the one variant: a mark with neither is [`CODE_MARK`] with
	/// the two elements it has always had, and a mark with either is
	/// [`CODE_MARK_TIMED`] with four. Spelling the old marks the new way would
	/// have changed the bytes under every signature ever put on one, and
	/// splitting the type in two would have pushed a distinction that is only a
	/// spelling into every consumer that reads a mark.
	///
	/// The time is the author's own clock at the moment of writing, and it
	/// **orders nothing**. A clock that is wrong is still a clock, so a reader
	/// shows a mark by its time and decides by [`crate::seq::OpOrder`]; sorting
	/// or arbitrating by the time would let a machine set to next year win every
	/// contest it entered.
	Mark {
		/// The name given to this point.
		name: String,
		/// What was said about it, as bytes, or nothing where nothing was said.
		body: Option<Vec<u8>>,
		/// When it was made, in seconds since the Unix epoch, or nothing for a
		/// mark written before the log carried a clock.
		time: Option<u64>,
	},
	/// Puts bytes in a gap and kills runs of existing content, which is the
	/// single primitive from which insertion, deletion and replacement all
	/// follow: an insertion removes nothing, a deletion inserts nothing, and a
	/// replacement does both in one operation.
	///
	/// The gap is named by the content either side of it and the removed runs
	/// are named by their content, so neither half carries a position, and
	/// neither names a file.
	Splice {
		/// Left origin of the inserted bytes, binding after a byte.
		left: Option<Anchor>,
		/// Right origin of the inserted bytes, binding before a byte.
		right: Option<Anchor>,
		/// What dies.
		remove: Vec<ContentRange>,
		/// What is inserted.
		insert: Vec<u8>,
	},
	/// Relocates runs of existing content, which keep their identity, to a
	/// position named by the content already there.
	///
	/// The source names bytes, and a byte's identity is repository-wide, so one
	/// operation moves a run within a file or from one file to another with
	/// nothing in it saying which case it is. The destination anchor is what
	/// decides.
	Move {
		/// What moves, in the order it lands in.
		src: Vec<ContentRange>,
		/// The gap the content lands in, on its left.
		left: Option<Anchor>,
		/// The gap the content lands in, on its right.
		right: Option<Anchor>,
	},
	/// Says something about content, and goes on saying it about that same
	/// content wherever the content ends up.
	///
	/// A note names bytes, not a line and not an offset, so the machinery that
	/// carries an anchor through an edit carries the note with it for nothing: the
	/// content is edited around and the note narrows to what survived; the content
	/// is moved, within a file or into another, and the note goes with it.
	///
	/// It is history and not sequence content. A note mints no atom, claims no
	/// byte and renders no byte; the sequence keeps it so that the causal graph is
	/// whole, exactly as it keeps a [`Op::Mark`], and the render resolves it into
	/// the spans a margin can be drawn against.
	///
	/// `on` must name something. A note about nothing is a mark with extra
	/// spelling, and [`Op::Mark`] already says "about this point in history".
	Note {
		/// The content the note is about.
		on: Vec<ContentRange>,
		/// What it says, as bytes: a note is not this crate's to decode.
		text: Vec<u8>,
	},
	/// Asks for something that is not yet in the bytes.
	///
	/// A proposal is history rather than a record beside history, so it travels
	/// the way every other operation travels: it is signed by whoever wrote it,
	/// it carries the frontier they could see, and a replica that has the
	/// operations has the discussion. Kept in a forge's own store instead, it
	/// would be readable only through that forge, and a clone would arrive with
	/// the argument for the change missing.
	///
	/// What is *not* here is a ballot. A vote in the log would name its voter
	/// permanently and irrevocably, under a signature, and the guarantee that
	/// tallies are published and voters are not could then never be given again.
	Proposal {
		/// What is being asked for, in one line.
		title: String,
		/// The case for it, as bytes.
		body: Vec<u8>,
		/// The forge name the author was writing under.
		voice: String,
		/// When it was opened, in seconds since the Unix epoch.
		time: u64,
	},
	/// One remark in a proposal's discussion.
	///
	/// It names the proposal by that operation's identity, not by a title and
	/// not by a number, so a remark still finds what it is about after the two
	/// have crossed between replicas in either order.
	Said {
		/// The proposal spoken about.
		on: OpId,
		/// What was said, as bytes.
		text: Vec<u8>,
		/// The forge name the author was writing under.
		voice: String,
		/// When it was said, in seconds since the Unix epoch.
		time: u64,
	},
	/// States what became of a proposal, from this point on.
	///
	/// Shaped like [`Op::FileMode`]: it names a thing by identity and asserts
	/// one piece of that thing's state, so the assertion survives everything
	/// said about the proposal afterwards. Two of these on one proposal settle
	/// the way two concurrent renames settle -- the later in operation order is
	/// the one a reader reports -- and that is the only concurrency rule these
	/// operations need.
	Settled {
		/// The proposal settled.
		on: OpId,
		/// What it became.
		state: Settled,
		/// The mark that closed it, where one did.
		///
		/// An identifier, never a mark's name: a name does not pick out the same
		/// operation on two replicas, and this has to.
		mark: Option<OpId>,
		/// When it was settled, in seconds since the Unix epoch.
		time: u64,
	},
	/// Says that operations undo others, so that a revert is not an unexplained
	/// deletion by a stranger when it reaches somebody else's machine.
	///
	/// The inverse edits are ordinary operations authored beside this one, and
	/// nothing here does the undoing; what this adds is the record of what they
	/// are for. Without it the bytes say only that somebody deleted work, and
	/// whoever wrote that work first has no way to see that it was reverted
	/// rather than lost, nor to be credited for it when it comes back.
	///
	/// Renders nothing, claims no byte and mints no atom, which is the species
	/// [`Op::Mark`] and [`Op::Note`] are already in.
	///
	/// `undone` must name something, for the reason [`Op::Note`]'s `on` must: a
	/// revert of nothing is a mark with extra spelling, and [`Op::Mark`] already
	/// says "about this point in history".
	Reverts {
		/// What is undone, in operation order, and never nothing.
		///
		/// Held ascending and without repetition, for the reason
		/// [`Header::parents`] is: two byte spellings of one set would both
		/// verify against a signature.
		undone: Vec<OpId>,
	},
}

impl Op {
	/// Returns the wire code identifying the variant.
	///
	/// A mark answers with the code it is written at, which is a function of
	/// what it carries rather than of the variant: see [`Op::Mark`]. That is what
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

	/// Returns the variant name, for messages and logs.
	///
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

	/// Returns the file an operation names by identity, which only a lifecycle
	/// change does.
	///
	/// A content operation gives `None`, and that is the whole of candidate B: it
	/// names content, and the file follows from the content. A file's creation
	/// gives `None` too, because the file it names is itself, and the identity is
	/// the operation's own.
	pub fn names_file(&self) -> Option<OpId> {
		match self {
			Self::FileDelete { file }		=> Some(*file),
			Self::FileRename { file, .. }	=> Some(*file),
			Self::FileMode { file, .. }		=> Some(*file),
			_								=> None,
		}
	}

	/// Returns the operation's two origins, absent for anything that places
	/// nothing.
	pub fn origins(&self) -> (Option<Anchor>, Option<Anchor>) {
		match self {
			Self::Splice { left, right, .. }	=> (*left, *right),
			Self::Move { left, right, .. }		=> (*left, *right),
			_									=> (None, None),
		}
	}

	/// Returns the content the operation **acts on**: what a splice removes, or
	/// what a move takes with it.
	///
	/// A note is not here, although it names content too. What this answers is
	/// which bytes an operation asserts something about -- which is what decides
	/// whether two operations were in conflict -- and a note asserts nothing: it
	/// neither kills content nor takes it anywhere, so a note and a concurrent
	/// deletion of the same run are not two authors disagreeing. See
	/// [`Op::note_on`] for the other reading.
	pub fn regions(&self) -> &[ContentRange] {
		match self {
			Self::Splice { remove, .. }	=> remove,
			Self::Move { src, .. }		=> src,
			_							=> &[],
		}
	}

	/// Returns the content a note is about, empty for everything else.
	///
	/// Kept apart from [`Op::regions`] because the two are asked different
	/// questions: what an operation claims, and what an operation refers to. Both
	/// have to exist for the render to resolve them, which is the one place the two
	/// are read together.
	pub fn note_on(&self) -> &[ContentRange] {
		match self {
			Self::Note { on, .. }	=> on,
			_						=> &[],
		}
	}

	/// Reports whether the operation is a move.
	pub fn is_move(&self) -> bool {
		matches!(self, Self::Move { .. })
	}

	/// Returns the number of bytes the operation places, which is what a splice
	/// inserts or what a move brings with it.
	pub fn placed_len(&self) -> u64 {
		match self {
			Self::Splice { insert, .. }	=> insert.len() as u64,
			Self::Move { src, .. }		=> src.iter().map(|r| r.len()).sum(),
			_							=> 0,
		}
	}

	/// Checks the rule that replaces the file field.
	///
	/// An operation that places anything -- a splice with a non-empty `insert`,
	/// or any move -- must carry at least one origin, because that origin is what
	/// says which file it lands in. A splice that only removes places nothing and
	/// needs none: it names the content it kills, and content is repository-wide.
	///
	/// This is enforced on the way off the wire as well as on the way into the
	/// sequence, because an operation that satisfies neither origin belongs to no
	/// file and there is nowhere for a reader to put it.
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

	/// Checks the rule that a note is about something.
	///
	/// [`Op::Note`] must name at least one byte. An empty list names nothing, and
	/// a list of empty ranges names nothing either, so both are refused: such a
	/// note could never resolve to a span and would be reported forever as a note
	/// on dead content, which is not what "dead" is for. An author wanting to say
	/// something about a point in history rather than about content writes an
	/// [`Op::Mark`].
	///
	/// Checked on the way off the wire as well as on the way into the sequence,
	/// for the reason [`Op::check_placement`] is.
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
	/// [`Op::Reverts`] must name at least one operation. An empty list undoes
	/// nothing, and a revert of nothing is a mark with extra spelling, which is
	/// the reason [`Op::check_note`] refuses a note about nothing: there is
	/// already an operation for saying something about a point in history, and
	/// [`Op::Mark`] is it. Nothing would ever author one, so a decoder meeting one
	/// has met damage rather than an intention.
	///
	/// The list is held ascending and without repetition, for the reason
	/// [`Header::from_dat`] holds the parents that way: the same set of operations
	/// written two ways would be two byte strings, both of which a signature would
	/// verify, and a provenance chain cannot afford a record with two spellings.
	/// It is refused rather than sorted, so that whoever wrote it finds out.
	///
	/// Checked on the way off the wire as well as on the way into the sequence,
	/// for the reason [`Op::check_placement`] is.
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

	/// Checks the operation is one the sequence structure can resolve.
	///
	/// A left origin binds after a byte and a right origin before one; a move may
	/// not name the same byte twice, since a byte has exactly one owning slot and
	/// could not otherwise be shown once; and [`Op::check_placement`] must hold.
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

	/// Serialises the operation to a [`Dat`]. The shape is
	/// `[code, field, ...]`, the fields in declaration order.
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
				Dat::BU64(insert.clone()),
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

	/// Reconstructs an operation from a [`Dat`] produced by [`Op::to_dat`].
	///
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
					insert:	res!(as_bytes(&v[4], "Splice insert")),
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

	/// Appends the byte encoding of the operation to `buf`, as a varint length
	/// followed by the binary daticle form.
	///
	/// The length prefix lets a consumer skip an operation it does not need to
	/// read, and lets several be laid end to end in one buffer.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		varint_encode(body.len() as u64, buf);
		buf.extend_from_slice(&body);
		Ok(())
	}

	/// Returns the byte encoding of the operation.
	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	/// Decodes an operation from the front of `buf`, returning it and the
	/// number of bytes consumed.
	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (dat, end) = res!(decode_framed(buf, "Op"));
		Ok((res!(Self::from_dat(&dat)), end))
	}

	/// Hashes the operation's canonical encoding under a hasher the caller
	/// supplies.
	///
	/// The choice of hash function is deliberately not made here. Which
	/// function is right depends on what else has to compute the same value --
	/// a browser limited to what its platform offers, a peer group that has
	/// already agreed on one -- so the caller brings it.
	pub fn hash<H: Hasher, const S: usize>(&self, hasher: H, salt: [u8; S])
		-> Outcome<Hash<S>>
	{
		let bytes = res!(self.encode());
		Ok(hasher.hash(&[&bytes], salt))
	}

	/// Decodes an operation that must occupy the whole of `buf`.
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
/// is never its own parent. A public field would let a struct literal build a
/// header that breaks all three, and two byte spellings of one frontier would
/// both verify against a signature, which is not a property a provenance chain
/// can afford. [`Header::new`] establishes the invariant and
/// [`Header::from_dat`] refuses anything that arrives without it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Header {
	/// The operation's own name.
	id:			OpId,
	/// The author's frontier when the operation was written, ascending.
	parents:	Vec<OpId>,
}

impl Header {
	/// Constructs a header, sorting the parents and dropping repetitions.
	///
	/// Fails if the operation names itself as its own parent, which no frontier
	/// can contain.
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

	/// Constructs the header of a root operation, one written against nothing.
	pub fn root(id: OpId) -> Self {
		Self { id, parents: Vec::new() }
	}

	/// Returns the operation's own name.
	pub const fn id(&self) -> OpId {
		self.id
	}

	/// Returns the author's frontier when the operation was written, ascending
	/// and without repetition.
	pub fn parents(&self) -> &[OpId] {
		&self.parents
	}

	/// Reports whether the operation was written against nothing, which is what
	/// the first operation of a history looks like.
	pub fn is_root(&self) -> bool {
		self.parents.is_empty()
	}

	/// Serialises the header to a [`Dat`]. The shape is `[id, [parent, ...]]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.id.to_dat(),
			Dat::List(self.parents.iter().map(|p| p.to_dat()).collect()),
		])
	}

	/// Reconstructs a header from a [`Dat`] produced by [`Header::to_dat`].
	///
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
	/// Who this operation is and what it was written against.
	pub head:	Header,
	/// What it says.
	pub op:		Op,
}

impl Record {
	/// Constructs a record from a header and an operation.
	pub fn new(head: Header, op: Op) -> Self {
		Self { head, op }
	}

	/// Constructs a record of an operation written against nothing.
	pub fn root(id: OpId, op: Op) -> Self {
		Self { head: Header::root(id), op }
	}

	/// Returns the operation's name.
	pub fn id(&self) -> OpId {
		self.head.id()
	}

	/// Returns the author's frontier when the operation was written.
	pub fn parents(&self) -> &[OpId] {
		self.head.parents()
	}

	/// Serialises the record to a [`Dat`]. The shape is `[head, op]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.head.to_dat(),
			self.op.to_dat(),
		])
	}

	/// Reconstructs a record from a [`Dat`] produced by [`Record::to_dat`].
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

	/// Appends the byte encoding of the record to `buf`, as a varint length
	/// followed by the binary daticle form.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		varint_encode(body.len() as u64, buf);
		buf.extend_from_slice(&body);
		Ok(())
	}

	/// Returns the byte encoding of the record.
	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	/// Decodes a record from the front of `buf`, returning it and the number of
	/// bytes consumed.
	pub fn decode(buf: &[u8])
		-> Outcome<(Self, usize)>
	{
		let (dat, end) = res!(decode_framed(buf, "Record"));
		Ok((res!(Self::from_dat(&dat)), end))
	}

	/// Decodes a record that must occupy the whole of `buf`.
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

	/// Hashes the record's canonical encoding under a hasher the caller
	/// supplies, so that the parents are covered along with the operation.
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


/// Reads a varint length prefix and the daticle it frames, returning the
/// daticle and the offset just past it.
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

/// Checks that a decoded operation list has exactly the expected length.
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

/// Extracts a string field, naming it if the kind is wrong.
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

/// Extracts a list of content ranges, naming it if the kind is wrong.
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

/// Extracts a byte vector field, naming it if the kind is wrong.
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

/// Extracts a list of operation identifiers, naming it if the kind is wrong.
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

/// Extracts a 64-bit unsigned field, naming it if the kind is wrong.
///
/// A time is exactly this and nothing wider: seconds since the Unix epoch, in
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

/// Serialises an optional byte payload, absence being that nothing was said.
fn opt_bytes_to_dat(body: &Option<Vec<u8>>) -> Dat {
	Dat::Opt(Box::new(body.as_ref().map(|b| Dat::BU64(b.clone()))))
}

/// Reconstructs an optional byte payload from a [`Dat`] produced by
/// [`opt_bytes_to_dat`].
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

/// Serialises an optional time, absence being that none was recorded.
fn opt_u64_to_dat(time: &Option<u64>) -> Dat {
	Dat::Opt(Box::new(time.map(Dat::U64)))
}

/// Reconstructs an optional time from a [`Dat`] produced by [`opt_u64_to_dat`].
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

/// Serialises an optional operation identifier, absence being that none was
/// named.
fn opt_id_to_dat(id: &Option<OpId>) -> Dat {
	Dat::Opt(Box::new(id.as_ref().map(|i| i.to_dat())))
}

/// Reconstructs an optional operation identifier from a [`Dat`] produced by
/// [`opt_id_to_dat`].
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

	/// A content range of the given replica's first operation.
	///
	/// The bounds are put in order before the constructor sees them, so the
	/// helper is total and can be called from the fixtures that return an
	/// operation rather than an [`Outcome`].
	fn range(replica: u64, from: u64, to: u64) -> ContentRange {
		let op = OpId::new(ReplicaId::new(replica), 1);
		ContentRange::new(op, from.min(to), from.max(to)).unwrap_or_default()
	}

	/// A content identifier of the given replica's first operation.
	fn content(replica: u64, off: u64) -> ContentId {
		ContentId::new(OpId::new(ReplicaId::new(replica), 1), off)
	}

	/// An operation identifier.
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
				insert:	b"hello".to_vec(),
			},
			// A deletion, which places nothing and so needs no origin.
			Op::Splice {
				left:	None,
				right:	None,
				remove:	vec![range(1, 12, 17)],
				insert:	Vec::new(),
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
				insert:	vec![0xa5; 1000],
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

	/// Every variant survives a [`Dat`] round trip.
	#[test]
	fn op_dat_round_trip() -> Outcome<()> {
		for op in samples() {
			let back = res!(Op::from_dat(&op.to_dat()));
			assert_eq!(op, back, "variant {}", op.name());
		}
		Ok(())
	}

	/// Every variant survives a byte round trip.
	#[test]
	fn op_byte_round_trip() -> Outcome<()> {
		for op in samples() {
			let buf = res!(op.encode());
			let back = res!(Op::decode_all(&buf));
			assert_eq!(op, back, "variant {}", op.name());
		}
		Ok(())
	}

	/// A payload longer than 255 bytes keeps its full length, which a
	/// `Dat::BU8` length field could not express. The same goes for a path.
	#[test]
	fn payloads_survive_beyond_a_byte_length() -> Outcome<()> {
		for len in [255usize, 256, 257, 4096, 70_000] {
			let op = Op::Splice {
				left:	Some(Anchor::origin(oid(1, 1))),
				right:	None,
				remove:	Vec::new(),
				insert:	vec![0x5a; len],
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

	/// Operations laid end to end each decode in turn, consuming only their
	/// own bytes.
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

	/// Wire codes are distinct and match the variants they name.
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

	/// Only a rename and a delete name a file, and they name it by identity. A
	/// content operation names content and nothing else, which is the whole of
	/// what file identity changed here.
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
			insert:	b"x".to_vec(),
		}.names_file(), None);
		assert_eq!(Op::Move {
			src:	Vec::new(),
			left:	Some(Anchor::origin(oid(1, 1))),
			right:	None,
		}.names_file(), None);
		Ok(())
	}

	/// An operation that places bytes names at least one origin, and one that
	/// places nothing need not.
	#[test]
	fn a_placement_names_where_it_lands() -> Outcome<()> {
		// A splice inserting bytes with neither origin belongs to no file.
		let stray = Op::Splice {
			left:	None,
			right:	None,
			remove:	Vec::new(),
			insert:	b"x".to_vec(),
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
			let op = Op::Splice { left, right, remove: Vec::new(), insert: b"x".to_vec() };
			res!(op.check_placement());
			assert_eq!(op, res!(Op::from_dat(&op.to_dat())));
		}
		// A splice that only removes places nothing and needs no origin.
		let del = Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![range(1, 0, 4)],
			insert:	Vec::new(),
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

	/// A note is about something: a note naming no byte is refused, on the way
	/// into the structure and on the way off the wire alike.
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

	/// A note names content and claims none: it is not among the regions two
	/// operations could be in conflict over, and it places nothing.
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

	/// The origins an operation carries and the content it names read back
	/// whatever the variant, and a lifecycle change names neither.
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

	/// An operation the structure cannot resolve is refused: an origin on the
	/// wrong side, or a move naming one byte twice.
	#[test]
	fn validate_refuses_what_cannot_be_resolved() -> Outcome<()> {
		let cid = content(1, 0);
		assert!(Op::Splice {
			left:	Some(Anchor::before(cid)),
			right:	None,
			remove:	Vec::new(),
			insert:	b"x".to_vec(),
		}.validate().is_err());
		assert!(Op::Splice {
			left:	None,
			right:	Some(Anchor::after(cid)),
			remove:	Vec::new(),
			insert:	b"x".to_vec(),
		}.validate().is_err());
		assert!(Op::Move {
			src:	vec![range(1, 0, 4), range(1, 2, 6)],
			left:	Some(Anchor::origin(oid(9, 1))),
			right:	None,
		}.validate().is_err());
		Ok(())
	}

	/// A mode is one of three things and says which, on the wire and in a
	/// message, and a file that has said nothing about its mode is normal.
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

	/// The mode operation names a file by identity, carries the code the
	/// vocabulary event gave it, and survives both round trips at every mode.
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

	/// A mark is one variant with two spellings, and which one it is written in
	/// is decided by what it carries rather than by the author.
	///
	/// The short spelling is the whole of the compatibility claim: a mark saying
	/// nothing beyond its name is [`CODE_MARK`] with two elements, byte for byte
	/// what was written before the fields existed, so every mark already signed
	/// still verifies. The long one is [`CODE_MARK_TIMED`] with four. Both arrive
	/// as [`Op::Mark`], and nothing downstream can tell which it was read from
	/// except by asking what the mark carries.
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

	/// A proposal, a remark upon it and its outcome each carry their fields
	/// through both round trips, at the codes the format change fixed.
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

	/// A proposal's outcome is one of four states and says which, on the wire and
	/// in a message, and a proposal nothing has settled is open.
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

	/// A revert names something, ascending and without repetition, and is refused
	/// rather than sorted or excused where it does not.
	///
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

	/// The operations that speak about history rather than about bytes claim
	/// nothing, refer to nothing, place nothing and name no file.
	///
	/// This is what puts them in the same species as [`Op::Mark`] and
	/// [`Op::Note`], and it is what every catch-all arm in [`crate::seq`] is
	/// relying on: an operation that mints no atom and claims no byte is carried
	/// for the sake of the causal graph and does nothing to the render.
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

	/// An unrecognised code, a wrong shape or a wrong field kind is refused.
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

	/// A move carrying many source runs keeps every one of them, in order,
	/// through both round trips.
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

	/// Both destination anchors keep their side, which decides whether an
	/// insertion abutting the moved run travels with it.
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
				insert:	b"x".to_vec(),
			};
			assert_eq!(sp, res!(Op::decode_all(&res!(sp.encode()))));
			assert_eq!(sp, res!(Op::from_dat(&sp.to_dat())));
		}
		Ok(())
	}

	/// Hashing goes through the canonical encoding: under the identity hasher
	/// the result is exactly those bytes, and equal operations hash equal while
	/// differing ones do not.
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
			insert:	Vec::new(),
		};
		assert!(res!(other.hash((), [0u8; 0])).as_vec() != want);
		Ok(())
	}

	/// The operation used by the hashing test.
	fn sample_op_for_hashing() -> Op {
		Op::Splice {
			left:	None,
			right:	None,
			remove:	vec![range(1, 12, 15)],
			insert:	Vec::new(),
		}
	}

	/// A truncated byte encoding is refused rather than half read.
	#[test]
	fn op_decode_rejects_truncation() -> Outcome<()> {
		let op = Op::Splice {
			left:	Some(Anchor::after(content(1, 0))),
			right:	None,
			remove:	vec![range(1, 1, 3)],
			insert:	b"abcdef".to_vec(),
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

	/// A header survives both round trips with no parents, with one, and with
	/// many.
	#[test]
	fn header_round_trips_at_every_arity() -> Outcome<()> {
		for head in res!(sample_heads()) {
			assert_eq!(head, res!(Header::from_dat(&head.to_dat())));
			let rec = Record::new(head.clone(), Op::Mark { name: fmt!("m"), body: None, time: None });
			assert_eq!(rec, res!(Record::decode_all(&res!(rec.encode()))));
		}
		Ok(())
	}

	/// Parents are sorted and deduplicated on construction, so the same frontier
	/// given in any order has one encoding, and nothing outside the module can
	/// build a header that says otherwise.
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

	/// An operation may not be its own parent, on the way in or on the way out.
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

	/// A root header carries no parents and says so.
	#[test]
	fn a_root_header_has_no_parents() -> Outcome<()> {
		let head = Header::root(oid(1, 1));
		assert!(head.is_root());
		assert!(head.parents().is_empty());
		assert!(!res!(Header::new(oid(1, 2), vec![oid(1, 1)])).is_root());
		Ok(())
	}

	/// A record round trips whatever operation it carries, and a malformed one
	/// is refused.
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

	/// The parents are inside what a record hashes, so an operation re-parented
	/// hashes differently.
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

	/// A truncated record is refused at every cut.
	#[test]
	fn record_decode_rejects_truncation() -> Outcome<()> {
		let rec = Record::new(
			res!(Header::new(oid(2, 3), vec![oid(1, 1), oid(1, 2)])),
			Op::Splice {
				left:	Some(Anchor::origin(oid(1, 1))),
				right:	None,
				remove:	Vec::new(),
				insert:	b"abcdef".to_vec(),
			},
		);
		let buf = res!(rec.encode());
		for cut in 1..buf.len() {
			assert!(Record::decode(&buf[..cut]).is_err(), "cut at {}", cut);
		}
		Ok(())
	}

	/// Records laid end to end each decode in turn.
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
}
