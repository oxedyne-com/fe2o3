//! Turning the ordered slots back into bytes, and saying what happened.
//!
//! The forest is built in the topological order of the anchor graph and walked
//! in order, and each slot emits the parts of its claim that it still owns and
//! that are still alive, into the file whose subtree it turned out to be in.
//! Everything the walk noticed is returned with the bytes as a [`Flag`], because
//! a structure that always converges owes the reader an account of what it
//! converged to: a torn move, an anchor demoted to break a cycle, a move confined
//! because it lost a cross-file cycle, or content moved into a file that has since
//! been deleted. Flags are facts derived from the operation set, not a
//! log of what the renderer happened to do, so two replicas holding the same
//! operations report the same flags.
//!
//! # Notes are resolved here too
//!
//! An [`crate::op::Op::Note`] names content and renders none. What a reader wants
//! is where that content ended up, and the render is the one place that is known,
//! so the same walk that produced the bytes is read backwards to produce a
//! [`Note`]: the note's identity, its text, and the spans of rendered bytes its
//! content occupies. A note is resolved against a file where its content renders
//! there, against the repository once however many files it is scattered over, and
//! reported as [`RepoNote::on_dead`] where its content renders nowhere at all.
//! Like a flag, a resolved note is a function of the operation set alone.

use crate::id::{
	Anchor,
	ContentId,
	ContentRange,
	OpId,
};
use crate::op::Op;
use crate::seq::atom::Atoms;
use crate::seq::claim::{
	Claims,
	Dead,
};
use crate::seq::slot::{
	Order,
	Origin,
	Slots,
};
use crate::seq::OpOrder;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::collections::BTreeMap;
use std::ops::Range;


/// Wire code for [`Flag::Torn`].
pub const CODE_TORN:		u8 = 1;
/// Wire code for [`Flag::Demoted`].
pub const CODE_DEMOTED:		u8 = 2;
/// Wire code for [`Flag::Dropped`].
pub const CODE_DROPPED:		u8 = 3;
/// Wire code for [`Flag::Overlap`].
pub const CODE_OVERLAP:		u8 = 4;
/// Wire code for [`Flag::CrossedFile`].
pub const CODE_CROSSED_FILE:	u8 = 5;
/// Wire code for [`Flag::MovedIntoDeleted`].
pub const CODE_MOVED_INTO_DELETED: u8 = 6;
/// Wire code for [`Flag::Orphaned`].
pub const CODE_ORPHANED:	u8 = 7;
/// Wire code for [`Flag::Confined`].
pub const CODE_CONFINED:	u8 = 8;
/// Wire code for [`Flag::Won`].
pub const CODE_WON:		u8 = 9;


/// Something the renderer noticed that the reader should be told.
///
/// Every flag is a function of the operation set, so the same set flags the same
/// things everywhere. None of them means the render failed; each means the
/// render made a choice that a person might want to revisit.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Flag {
	/// A move whose source was taken from it by a **concurrent** move: the block
	/// tore at the overlap and its pieces render in two places.
	///
	/// Concurrency is decided from the operations' own parents. A move superseded
	/// by a later move of the same content, by the same author or any other, is a
	/// sequence of two decisions rather than a race, and raises nothing; the
	/// earlier move's intent was overtaken on purpose and there is nothing for a
	/// reader to reconcile.
	Torn {
		/// The move that lost ground.
		op:		OpId,
		/// The content it named and no longer shows.
		lost:	Vec<ContentRange>,
	},
	/// An origin resolved against the splice that created its content rather
	/// than against the slot that now shows it, to break a cycle.
	///
	/// The consequence is that the placement landed where its anchor content was
	/// originally written rather than where it now lives, which is deterministic
	/// and surprising in equal measure.
	Demoted {
		/// The operation whose origin was demoted.
		op:		OpId,
		/// Offset within that operation's placement.
		sub:	u64,
		/// Which of the two origins.
		origin:	Origin,
	},
	/// An origin dropped entirely because demotion did not break the cycle, so
	/// the placement fell back to whatever the partially built tree gave it.
	Dropped {
		/// The operation whose origin was dropped.
		op:		OpId,
		/// Offset within that operation's placement.
		sub:	u64,
		/// Which of the two origins.
		origin:	Origin,
	},
	/// Two concurrent operations named overlapping content: both removed it,
	/// both moved it, or one removed what the other moved.
	///
	/// Concurrency is decided from the operations' own parents, so the flag means
	/// what its name says: neither author could see what the other was doing. Two
	/// operations touching the same bytes where one was written in knowledge of
	/// the other are a sequence of edits and not a conflict, and raise nothing.
	Overlap {
		/// The operations involved, in ascending order of identifier.
		ops:	Vec<OpId>,
		/// The content they have in common.
		region:	ContentRange,
	},
	/// Breaking a cycle carried content across a file boundary: the placement was
	/// demoted, and what it holds was written into one file and now renders in
	/// another.
	///
	/// In one file a demoted origin lands at a stale position, which is bad
	/// enough and is what [`Flag::Demoted`] says. Across two it lands in another
	/// *file*, and a reader who sees a file go from four bytes to none will not
	/// read that as a stale anchor, so this names both files and says so in those
	/// terms.
	///
	/// A cross-file *cycle* no longer reaches demotion at all: it is arbitrated,
	/// and [`Flag::Confined`] is what the losers are told. What still reaches this
	/// flag is a demotion inside one file whose content had legitimately changed
	/// files earlier, and it is then telling the truth about where the content was
	/// written and where it now renders.
	CrossedFile {
		/// The operation whose origin was demoted.
		op:		OpId,
		/// Offset within that operation's placement.
		sub:	u64,
		/// The file the content it holds was written into.
		from:	OpId,
		/// The file it renders in instead.
		to:		OpId,
	},
	/// A move landed content in a file that has been deleted, so the content is
	/// held by a slot and named by the log but rendered nowhere a reader looks.
	///
	/// Nothing is lost: the file's render still holds the bytes, and a verb that
	/// recovers them is design work owed. What is lost is visibility, and this is
	/// what says so.
	MovedIntoDeleted {
		/// The move.
		op:		OpId,
		/// The file it landed in, which is not live.
		file:	OpId,
	},
	/// A slot ended up in no file at all, its origins having been dropped, so
	/// the bytes it owns render nowhere.
	///
	/// This is the last resort of the cycle rule showing through, and it is a
	/// fault rather than a choice; it is reported rather than hidden because
	/// conservation must account for the bytes either way.
	Orphaned {
		/// The operation whose placement fell out of every file.
		op:		OpId,
		/// Offset within that operation's placement.
		sub:	u64,
	},
	/// A move that lost a cross-file cycle: it did not happen, and its content is
	/// where it was before.
	///
	/// A cycle in the anchor graph that crosses a file boundary is arbitrated as
	/// one concurrent group. The member highest in op order completes; every other
	/// member is confined, which is to say its claims are not written, so its bytes
	/// stay with whoever owned them before it and its slots place nothing. Nothing
	/// has to be undone, because nothing was done, and re-issuing the move is one
	/// operation.
	///
	/// Both files are told, since the flag is about the pair of them.
	Confined {
		/// The move that did not happen.
		op:		OpId,
		/// The file its content stays in.
		home:	OpId,
		/// The file it was aimed at and did not reach.
		denied:	OpId,
	},
	/// A move that won a cross-file cycle outright and completed.
	///
	/// Derivable from the [`Flag::Confined`] flags and the operation set, and kept
	/// anyway: the loser's flag reads badly alone, and the two together are the
	/// whole story of what the arbitration did.
	Won {
		/// The move that won.
		op:		OpId,
	},
}


impl Flag {
	/// Returns the wire code identifying the variant.
	pub fn code(&self) -> u8 {
		match self {
			Self::Torn { .. }				=> CODE_TORN,
			Self::Demoted { .. }			=> CODE_DEMOTED,
			Self::Dropped { .. }			=> CODE_DROPPED,
			Self::Overlap { .. }			=> CODE_OVERLAP,
			Self::CrossedFile { .. }		=> CODE_CROSSED_FILE,
			Self::MovedIntoDeleted { .. }	=> CODE_MOVED_INTO_DELETED,
			Self::Orphaned { .. }			=> CODE_ORPHANED,
			Self::Confined { .. }			=> CODE_CONFINED,
			Self::Won { .. }				=> CODE_WON,
		}
	}

	/// Returns the variant name, for messages.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Torn { .. }				=> "Torn",
			Self::Demoted { .. }			=> "Demoted",
			Self::Dropped { .. }			=> "Dropped",
			Self::Overlap { .. }			=> "Overlap",
			Self::CrossedFile { .. }		=> "CrossedFile",
			Self::MovedIntoDeleted { .. }	=> "MovedIntoDeleted",
			Self::Orphaned { .. }			=> "Orphaned",
			Self::Confined { .. }			=> "Confined",
			Self::Won { .. }				=> "Won",
		}
	}

	/// Returns the operation the flag is chiefly about.
	pub fn op(&self) -> Option<OpId> {
		match self {
			Self::Torn { op, .. }				=> Some(*op),
			Self::Demoted { op, .. }			=> Some(*op),
			Self::Dropped { op, .. }			=> Some(*op),
			Self::Overlap { .. }				=> None,
			Self::CrossedFile { op, .. }		=> Some(*op),
			Self::MovedIntoDeleted { op, .. }	=> Some(*op),
			Self::Orphaned { op, .. }			=> Some(*op),
			Self::Confined { op, .. }			=> Some(*op),
			Self::Won { op }					=> Some(*op),
		}
	}

	/// Serialises the flag to a [`Dat`]. The shape is `[code, field, ...]`, the
	/// fields in declaration order.
	pub fn to_dat(&self) -> Dat {
		match self {
			Self::Torn { op, lost } => Dat::List(vec![
				Dat::U8(CODE_TORN),
				op.to_dat(),
				Dat::List(lost.iter().map(|r| r.to_dat()).collect()),
			]),
			Self::Demoted { op, sub, origin } => Dat::List(vec![
				Dat::U8(CODE_DEMOTED),
				op.to_dat(),
				Dat::U64(*sub),
				Dat::U8(origin.code()),
			]),
			Self::Dropped { op, sub, origin } => Dat::List(vec![
				Dat::U8(CODE_DROPPED),
				op.to_dat(),
				Dat::U64(*sub),
				Dat::U8(origin.code()),
			]),
			Self::Overlap { ops, region } => Dat::List(vec![
				Dat::U8(CODE_OVERLAP),
				Dat::List(ops.iter().map(|id| id.to_dat()).collect()),
				region.to_dat(),
			]),
			Self::CrossedFile { op, sub, from, to } => Dat::List(vec![
				Dat::U8(CODE_CROSSED_FILE),
				op.to_dat(),
				Dat::U64(*sub),
				from.to_dat(),
				to.to_dat(),
			]),
			Self::MovedIntoDeleted { op, file } => Dat::List(vec![
				Dat::U8(CODE_MOVED_INTO_DELETED),
				op.to_dat(),
				file.to_dat(),
			]),
			Self::Orphaned { op, sub } => Dat::List(vec![
				Dat::U8(CODE_ORPHANED),
				op.to_dat(),
				Dat::U64(*sub),
			]),
			Self::Confined { op, home, denied } => Dat::List(vec![
				Dat::U8(CODE_CONFINED),
				op.to_dat(),
				home.to_dat(),
				denied.to_dat(),
			]),
			Self::Won { op } => Dat::List(vec![
				Dat::U8(CODE_WON),
				op.to_dat(),
			]),
		}
	}

	/// Reconstructs a flag from a [`Dat`] produced by [`Flag::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if !v.is_empty() => v,
			_ => return Err(err!(
				"A Flag expects a non-empty Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let code = match &v[0] {
			Dat::U8(c) => *c,
			other => return Err(err!(
				"A Flag code expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		match code {
			CODE_TORN => {
				res!(flag_len(v, 3, "Torn"));
				Ok(Self::Torn {
					op:		res!(OpId::from_dat(&v[1])),
					lost:	res!(flag_ranges(&v[2], "Torn lost")),
				})
			},
			CODE_DEMOTED | CODE_DROPPED => {
				let what = if code == CODE_DEMOTED { "Demoted" } else { "Dropped" };
				res!(flag_len(v, 4, what));
				let op = res!(OpId::from_dat(&v[1]));
				let sub = res!(flag_u64(&v[2], what, "offset"));
				let origin = res!(flag_origin(&v[3], what));
				Ok(if code == CODE_DEMOTED {
					Self::Demoted { op, sub, origin }
				} else {
					Self::Dropped { op, sub, origin }
				})
			},
			CODE_OVERLAP => {
				res!(flag_len(v, 3, "Overlap"));
				let listed = match &v[1] {
					Dat::List(l) => l,
					other => return Err(err!(
						"A Flag::Overlap operation list expects Dat::List, got {:?}.",
						other;
					Decode, Input, Mismatch)),
				};
				let mut ops = Vec::with_capacity(listed.len());
				for item in listed {
					ops.push(res!(OpId::from_dat(item)));
				}
				Ok(Self::Overlap {
					ops,
					region: res!(ContentRange::from_dat(&v[2])),
				})
			},
			CODE_CROSSED_FILE => {
				res!(flag_len(v, 5, "CrossedFile"));
				Ok(Self::CrossedFile {
					op:		res!(OpId::from_dat(&v[1])),
					sub:	res!(flag_u64(&v[2], "CrossedFile", "offset")),
					from:	res!(OpId::from_dat(&v[3])),
					to:		res!(OpId::from_dat(&v[4])),
				})
			},
			CODE_MOVED_INTO_DELETED => {
				res!(flag_len(v, 3, "MovedIntoDeleted"));
				Ok(Self::MovedIntoDeleted {
					op:		res!(OpId::from_dat(&v[1])),
					file:	res!(OpId::from_dat(&v[2])),
				})
			},
			CODE_ORPHANED => {
				res!(flag_len(v, 3, "Orphaned"));
				Ok(Self::Orphaned {
					op:		res!(OpId::from_dat(&v[1])),
					sub:	res!(flag_u64(&v[2], "Orphaned", "offset")),
				})
			},
			CODE_CONFINED => {
				res!(flag_len(v, 4, "Confined"));
				Ok(Self::Confined {
					op:		res!(OpId::from_dat(&v[1])),
					home:	res!(OpId::from_dat(&v[2])),
					denied:	res!(OpId::from_dat(&v[3])),
				})
			},
			CODE_WON => {
				res!(flag_len(v, 2, "Won"));
				Ok(Self::Won {
					op:	res!(OpId::from_dat(&v[1])),
				})
			},
			other => Err(err!(
				"Flag code {} is not recognised.", other;
			Decode, Input, Invalid)),
		}
	}
}

/// Checks that a decoded flag list has exactly the expected length.
fn flag_len(v: &[Dat], want: usize, what: &str)
	-> Outcome<()>
{
	if v.len() != want {
		return Err(err!(
			"A Flag::{} expects {} list elements, got {}.", what, want, v.len();
		Decode, Input, Mismatch));
	}
	Ok(())
}

/// Extracts an unsigned field, naming it if the kind is wrong.
fn flag_u64(dat: &Dat, what: &str, field: &str)
	-> Outcome<u64>
{
	match dat {
		Dat::U64(n) => Ok(*n),
		other => Err(err!(
			"A Flag::{} {} expects Dat::U64, got {:?}.", what, field, other;
		Decode, Input, Mismatch)),
	}
}

/// Extracts an origin field, naming it if the kind or the code is wrong.
fn flag_origin(dat: &Dat, what: &str)
	-> Outcome<Origin>
{
	match dat {
		Dat::U8(c) => Origin::from_code(*c),
		other => Err(err!(
			"A Flag::{} origin expects Dat::U8, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}

/// Extracts a list of content ranges, naming it if the kind is wrong.
fn flag_ranges(dat: &Dat, what: &str)
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
			"A Flag {} expects Dat::List, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	}
}


/// What a render cost, in the terms the cost model is stated in.
///
/// The figures are the repository's, since the render is the repository's: one
/// forest is laid out and walked, and a file is a subtree of it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
	/// Operations in the set.
	pub ops:			usize,
	/// Files created, deleted ones included.
	pub files:			usize,
	/// Atoms created, one per file's origin anchor and one per inserting splice.
	pub atoms:			usize,
	/// Bytes held in atoms, alive or dead, origin anchors included.
	pub atom_bytes:		u64,
	/// Slots placed: one per file, one per splice and one per source run of a
	/// move.
	pub slots_placed:	usize,
	/// Slots after dividing at anchors.
	pub slots_divided:	usize,
	/// Intervals in the claim register, which is the standing cost of every move
	/// ever made.
	pub claim_intervals:	usize,
	/// Intervals in the tombstone set.
	pub dead_intervals:	usize,
	/// Notes the operation set holds, resolved ones and notes on dead content
	/// alike.
	pub notes:			usize,
	/// Deepest path in the Fugue forest.
	pub max_depth:		u32,
	/// Bytes rendered anywhere, in live files and deleted ones alike.
	pub rendered:		u64,
	/// Bytes rendered into files that have been deleted, which a reader does not
	/// see.
	pub withheld:		u64,
	/// Live bytes owned by a slot that belongs to no file, which render nowhere
	/// at all. Anything but zero is a fault, and is flagged.
	pub orphaned:		u64,
}


/// A run of rendered bytes, and the content it shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Run {
	/// Offset in the rendered bytes at which the run begins.
	pub at:			u64,
	/// The content the run shows.
	pub content:	ContentRange,
}


impl Run {
	/// Serialises the run to a [`Dat`]. The shape is `[at, content]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::U64(self.at),
			self.content.to_dat(),
		])
	}

	/// Reconstructs a run from a [`Dat`] produced by [`Run::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Run expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let at = match &v[0] {
			Dat::U64(n) => *n,
			other => return Err(err!(
				"A Run offset expects Dat::U64, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		Ok(Self {
			at,
			content: res!(ContentRange::from_dat(&v[1])),
		})
	}
}


/// A run of rendered bytes, named by where it begins and how far it goes.
///
/// This is what a note resolves to, and it is deliberately not a [`Run`]: a run
/// says which content is being shown, and a span says only which bytes of the
/// render a margin should be drawn against. A frontend that wants the content
/// under a span asks [`Rendered::span`] for it.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Span {
	/// Offset in the rendered bytes at which the span begins.
	pub at:		u64,
	/// How many bytes it covers.
	pub len:	u64,
}

impl Span {
	/// Constructs a span.
	pub const fn new(at: u64, len: u64) -> Self {
		Self { at, len }
	}

	/// Returns the offset just past the span.
	pub const fn end(&self) -> u64 {
		self.at + self.len
	}

	/// Serialises the span to a [`Dat`]. The shape is `[at, len]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::U64(self.at),
			Dat::U64(self.len),
		])
	}

	/// Reconstructs a span from a [`Dat`] produced by [`Span::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Span expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let mut n = [0u64; 2];
		for (i, dat) in v.iter().enumerate() {
			n[i] = match dat {
				Dat::U64(x) => *x,
				other => return Err(err!(
					"A Span bound expects Dat::U64, got {:?}.", other;
				Decode, Input, Mismatch)),
			};
		}
		Ok(Self { at: n[0], len: n[1] })
	}
}


/// A note as one file's render resolved it: what the note says, and where in this
/// file the content it is about now sits.
///
/// The spans are ascending, disjoint and maximal. There may be several, because
/// the content a note was written against can be torn apart by later edits and by
/// moves; there is never a span of no bytes, because a note that resolves to
/// nothing here is not reported here at all.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Note {
	/// The operation that wrote the note.
	note:	OpId,
	/// What it says, as bytes.
	text:	Vec<u8>,
	/// Where its content renders in this file, ascending.
	spans:	Vec<Span>,
}

impl Note {
	/// Assembles a resolved note from its parts.
	pub fn new(note: OpId, text: Vec<u8>, spans: Vec<Span>) -> Self {
		Self { note, text, spans }
	}

	/// Returns the operation that wrote the note.
	pub const fn note(&self) -> OpId {
		self.note
	}

	/// Returns what the note says, as bytes.
	pub fn text(&self) -> &[u8] {
		&self.text
	}

	/// Returns what the note says as a string, with anything that is not valid
	/// UTF-8 replaced. For messages and tests; the bytes themselves are the record.
	pub fn text_lossy(&self) -> String {
		String::from_utf8_lossy(&self.text).into_owned()
	}

	/// Returns where the note's content renders in this file, ascending.
	pub fn spans(&self) -> &[Span] {
		&self.spans
	}

	/// Returns the number of bytes the note's content occupies in this file.
	pub fn len(&self) -> u64 {
		self.spans.iter().map(|s| s.len).sum()
	}

	/// Reports whether the note covers no bytes of this file, which a resolved
	/// note never does.
	pub fn is_empty(&self) -> bool {
		self.spans.is_empty()
	}

	/// Serialises the note to a [`Dat`]. The shape is `[note, text, [span, ...]]`.
	///
	/// The text is a [`Dat::BU64`] for the reason a file's bytes are: a note may
	/// be longer than the 255 bytes a [`Dat::BU8`] length field can express, and a
	/// truncated length there would corrupt silently.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.note.to_dat(),
			Dat::BU64(self.text.clone()),
			Dat::List(self.spans.iter().map(|s| s.to_dat()).collect()),
		])
	}

	/// Reconstructs a note from a [`Dat`] produced by [`Note::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 3 => v,
			_ => return Err(err!(
				"A Note expects a 3-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let note = res!(OpId::from_dat(&v[0]));
		let text = match &v[1] {
			Dat::BU64(b) => b.clone(),
			other => return Err(err!(
				"The text of the note {} expects Dat::BU64, got {:?}.", note, other;
			Decode, Input, Mismatch)),
		};
		let listed = match &v[2] {
			Dat::List(l) => l,
			other => return Err(err!(
				"The spans of the note {} expect Dat::List, got {:?}.", note, other;
			Decode, Input, Mismatch)),
		};
		let mut spans = Vec::with_capacity(listed.len());
		for item in listed {
			spans.push(res!(Span::from_dat(item)));
		}
		Ok(Self { note, text, spans })
	}
}


/// Where a note's content renders in one file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotePlace {
	/// The file.
	pub file:	OpId,
	/// Where the content renders in it, ascending.
	pub spans:	Vec<Span>,
}


/// A note as the repository's render resolved it: what it says, every file its
/// content reaches, and whether it reaches any.
///
/// A note is listed here once however many files its content is scattered over,
/// which is the difference between this and [`Rendered::notes`]: the file view
/// answers "what should this margin show", and the repository view answers "where
/// did this note end up".
///
/// There is no codec for this type, and that is deliberate: a repository view is
/// derived from the file views a snapshot already carries, except for a note on
/// dead content, which is derived from the operation log. Storing it would be
/// storing a join.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoNote {
	/// The operation that wrote the note.
	note:		OpId,
	/// What it says, as bytes.
	text:		Vec<u8>,
	/// The files its content reaches, in ascending order of identity.
	files:		Vec<NotePlace>,
	/// Whether none of the content it names renders anywhere at all.
	on_dead:	bool,
}

impl RepoNote {
	/// Assembles a repository-wide resolved note from its parts.
	pub(super) fn new(note: OpId, text: Vec<u8>, files: Vec<NotePlace>) -> Self {
		let on_dead = files.is_empty();
		Self { note, text, files, on_dead }
	}

	/// Returns the operation that wrote the note.
	pub const fn note(&self) -> OpId {
		self.note
	}

	/// Returns what the note says, as bytes.
	pub fn text(&self) -> &[u8] {
		&self.text
	}

	/// Returns what the note says as a string, with anything that is not valid
	/// UTF-8 replaced.
	pub fn text_lossy(&self) -> String {
		String::from_utf8_lossy(&self.text).into_owned()
	}

	/// Returns the files the note's content reaches, ascending by identity.
	pub fn files(&self) -> &[NotePlace] {
		&self.files
	}

	/// Returns where the note's content renders in one file.
	pub fn spans_in(&self, file: OpId) -> &[Span] {
		self.files
			.iter()
			.find(|p| p.file == file)
			.map(|p| p.spans.as_slice())
			.unwrap_or(&[])
	}

	/// Reports whether every byte the note is about has been deleted, so that the
	/// note renders nowhere.
	///
	/// A note in this state is not lost and is not a fault: the log still holds
	/// what it says and what it was about, and this is what says a reader will not
	/// find it in any margin. A note on content that moved into a *deleted file*
	/// is not in this state, because those bytes still render -- into a file no
	/// reader looks at, which is what [`Flag::MovedIntoDeleted`] is for.
	pub const fn on_dead(&self) -> bool {
		self.on_dead
	}
}


/// One file as a render produced it: which file it is, where it sits, whether it
/// still exists, its bytes, what they are made of, and what the renderer noticed
/// about it.
///
/// A file is named by the identity of the [`Op::FileCreate`] that minted it, and
/// its path is metadata that a rename may change. Two live files may share a
/// path, that being a state the repository can genuinely be in; which of them a
/// working copy materialises under the shared name is a policy the caller owns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Rendered {
	/// The file's identity, which is the identity of its creating operation.
	file:	OpId,
	/// Where the file sits, as bytes.
	path:	Vec<u8>,
	/// Whether the file still exists.
	live:	bool,
	/// The rendered bytes.
	bytes:	Vec<u8>,
	/// Provenance, in render order and coalesced.
	runs:	Vec<Run>,
	/// What the renderer noticed that concerns this file.
	flags:	Vec<Flag>,
	/// The notes whose content renders here, in render order.
	notes:	Vec<Note>,
}

impl Rendered {

	/// Assembles a file's render from its parts.
	#[allow(clippy::too_many_arguments)]
	pub(super) fn new(
		file:	OpId,
		path:	Vec<u8>,
		live:	bool,
		bytes:	Vec<u8>,
		runs:	Vec<Run>,
		flags:	Vec<Flag>,
		notes:	Vec<Note>,
	)
		-> Self
	{
		Self { file, path, live, bytes, runs, flags, notes }
	}

	/// Returns the file's identity.
	pub const fn file(&self) -> OpId {
		self.file
	}

	/// Returns the file's path, as bytes.
	pub fn path(&self) -> &[u8] {
		&self.path
	}

	/// Returns the path as a string, with anything that is not valid UTF-8
	/// replaced. For messages and tests; the bytes themselves are the record.
	pub fn path_lossy(&self) -> String {
		String::from_utf8_lossy(&self.path).into_owned()
	}

	/// Reports whether the file still exists.
	pub const fn is_live(&self) -> bool {
		self.live
	}

	/// Returns the rendered bytes.
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	/// Returns the provenance of the rendered bytes, in render order.
	///
	/// Runs are maximal: a run continues for as long as the content it shows is
	/// contiguous, whatever the slot structure underneath.
	pub fn runs(&self) -> &[Run] {
		&self.runs
	}

	/// Returns what the renderer noticed that concerns this file.
	pub fn flags(&self) -> &[Flag] {
		&self.flags
	}

	/// Returns the notes whose content renders in this file, in the order a
	/// margin would draw them: by where each note's first span begins, and then by
	/// the identity of the note.
	///
	/// A note appears here for every file its content reaches, which is more than
	/// one where a later move split that content across two; the whole of it is
	/// [`Repo::notes`]. A note whose content has been deleted appears in no file
	/// at all, and is reported by [`RepoNote::on_dead`].
	pub fn notes(&self) -> &[Note] {
		&self.notes
	}

	/// Returns one note by the identity of the operation that wrote it, if its
	/// content renders here.
	pub fn note(&self, note: OpId)
		-> Option<&Note>
	{
		self.notes.iter().find(|n| n.note == note)
	}

	/// Returns the number of bytes rendered.
	pub fn len(&self) -> usize {
		self.bytes.len()
	}

	/// Reports whether nothing was rendered.
	pub fn is_empty(&self) -> bool {
		self.bytes.is_empty()
	}

	/// Returns the rendered bytes as a string, with anything that is not valid
	/// UTF-8 replaced. For messages and tests; the bytes themselves are the
	/// record.
	pub fn text_lossy(&self) -> String {
		String::from_utf8_lossy(&self.bytes).into_owned()
	}

	/// Returns the content identifier of the byte at a rendered index.
	pub fn content_at(&self, index: usize)
		-> Outcome<ContentId>
	{
		let at = index as u64;
		let pos = self.runs.partition_point(|r| r.at <= at);
		if pos > 0 {
			let run = self.runs[pos - 1];
			if at < run.at + run.content.len() {
				return Ok(ContentId::new(run.content.op(), run.content.from() + (at - run.at)));
			}
		}
		Err(err!(
			"Rendered index {} is beyond the {} bytes rendered.", index, self.bytes.len();
		Invalid, Input, Range))
	}

	/// Returns the content a rendered span is made of, as the fewest runs that
	/// name it.
	pub fn span(&self, at: usize, len: usize)
		-> Outcome<Vec<ContentRange>>
	{
		let end = match at.checked_add(len) {
			Some(e) => e,
			None => return Err(err!(
				"A span of {} bytes at index {} overflows.", len, at;
			Invalid, Input, Overflow)),
		};
		if end > self.bytes.len() {
			return Err(err!(
				"A span of {}..{} reaches beyond the {} bytes rendered.",
				at, end, self.bytes.len();
			Invalid, Input, Range));
		}
		// Runs are already maximal, so no two of them can be joined and the walk
		// is one step per run touched.
		let mut out: Vec<ContentRange> = Vec::new();
		let mut pos = at as u64;
		let end = end as u64;
		let mut next = self.runs.partition_point(|r| r.at <= pos);
		while pos < end {
			if next == 0 {
				return Err(err!(
					"Rendered index {} lies in no run, though {} bytes were \
					rendered.", pos, self.bytes.len();
				Bug, Missing));
			}
			let run = self.runs[next - 1];
			let within = pos - run.at;
			if within >= run.content.len() {
				return Err(err!(
					"Rendered index {} falls in the gap after the run at {}; runs \
					must cover the render.", pos, run.at;
				Bug, Missing));
			}
			let take = (run.content.len() - within).min(end - pos);
			out.push(res!(ContentRange::new(
				run.content.op(),
				run.content.from() + within,
				run.content.from() + within + take,
			)));
			pos += take;
			next += 1;
		}
		Ok(out)
	}

	/// Returns the two origins bracketing the gap at a rendered index.
	///
	/// The left origin binds after the byte before the gap; at the start of the
	/// file there is no such byte in the render, and the origin is the file's
	/// **origin anchor**, which is what makes an empty file addressable and is
	/// how an operation says which file it lands in. The right origin binds
	/// before the byte after the gap, and is absent at the end of the file, there
	/// being nothing to name it by.
	pub fn gap(&self, at: usize)
		-> Outcome<(Option<Anchor>, Option<Anchor>)>
	{
		if at > self.bytes.len() {
			return Err(err!(
				"The gap at index {} is beyond the {} bytes rendered.",
				at, self.bytes.len();
			Invalid, Input, Range));
		}
		let left = if at > 0 {
			Some(Anchor::after(res!(self.content_at(at - 1))))
		} else {
			Some(Anchor::origin(self.file))
		};
		let right = if at < self.bytes.len() {
			Some(Anchor::before(res!(self.content_at(at))))
		} else {
			None
		};
		Ok((left, right))
	}

	/// Builds a content-anchored splice from index-based editing intent:
	/// replace `len` bytes at `at` with `insert`.
	///
	/// This is the bridge a frontend crosses. An editor knows where the cursor
	/// is; the structure knows only what the bytes are called, and the render is
	/// the one place both are known at once.
	pub fn splice(&self, at: usize, len: usize, insert: Vec<u8>)
		-> Outcome<Op>
	{
		let remove = res!(self.span(at, len));
		// A splice inserting nothing places no slot, so its origins would say
		// nothing about where anything goes, nor about which file.
		let (left, right) = if insert.is_empty() {
			(None, None)
		} else {
			res!(self.gap(at))
		};
		Ok(Op::Splice { left, right, remove, insert })
	}

	/// Builds a content-anchored move from index-based editing intent: take
	/// `len` bytes at `at` to the gap at `to`, in this file.
	pub fn move_range(&self, at: usize, len: usize, to: usize)
		-> Outcome<Op>
	{
		let src = res!(self.span(at, len));
		let (left, right) = res!(self.gap(to));
		Ok(Op::Move { src, left, right })
	}

	/// Builds a content-anchored move that takes `len` bytes at `at` in this file
	/// to the gap at `to` in another.
	///
	/// Nothing distinguishes this from [`Rendered::move_range`] except which
	/// render the destination gap is read from. That is the whole of cross-file
	/// move: the source names content, which is repository-wide, and the
	/// destination names content in the file it lands in.
	pub fn move_into(&self, at: usize, len: usize, dest: &Self, to: usize)
		-> Outcome<Op>
	{
		let src = res!(self.span(at, len));
		let (left, right) = res!(dest.gap(to));
		Ok(Op::Move { src, left, right })
	}

	/// Builds a content-anchored note from index-based intent: say `text` about
	/// the `len` bytes at `at`.
	///
	/// This is the same bridge [`Rendered::splice`] is, for the same reason: the
	/// reader points at a region of the screen, and the render is where that region
	/// acquires the names that will follow it through every later edit.
	///
	/// Fails on a region of no bytes, since a note is about something.
	pub fn note_on(&self, at: usize, len: usize, text: Vec<u8>)
		-> Outcome<Op>
	{
		let on = res!(self.span(at, len));
		let op = Op::Note { on, text };
		res!(op.check_note());
		Ok(op)
	}
}


/// The repository as a render produced it: every file, everything the renderer
/// noticed, and what it cost.
///
/// Rendering is repository-wide because ordering is: a file is a subtree of one
/// forest, so laying out that forest is what decides which file each slot is in.
/// Reading one file is then [`Repo::file`], and the closure-scoped renderer that
/// would lay out less than the whole repository is design work owed rather than
/// work done.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Repo {
	/// Every file, in ascending order of identity, deleted ones included.
	files:	Vec<Rendered>,
	/// Everything the renderer noticed, sorted and without repetition.
	flags:	Vec<Flag>,
	/// Every note the operation set holds, in ascending order of identity.
	notes:	Vec<RepoNote>,
	/// Which file each placing operation landed in.
	index:	BTreeMap<OpId, OpId>,
	/// What the render cost.
	stats:	Stats,
}

impl Repo {

	/// Assembles a repository render from its parts.
	pub(super) fn new(
		files:	Vec<Rendered>,
		flags:	Vec<Flag>,
		notes:	Vec<RepoNote>,
		index:	BTreeMap<OpId, OpId>,
		stats:	Stats,
	)
		-> Self
	{
		Self { files, flags, notes, index, stats }
	}

	/// Returns every file, in ascending order of identity, deleted ones included.
	pub fn files(&self) -> &[Rendered] {
		&self.files
	}

	/// Returns one file by identity.
	pub fn file(&self, file: OpId)
		-> Option<&Rendered>
	{
		self.files
			.binary_search_by(|f| f.file.cmp(&file))
			.ok()
			.and_then(|i| self.files.get(i))
	}

	/// Returns the live files, in ascending order of path and then of identity.
	pub fn live(&self) -> Vec<&Rendered> {
		let mut v: Vec<&Rendered> = self.files.iter().filter(|f| f.live).collect();
		v.sort_by(|a, b| a.path.cmp(&b.path).then(a.file.cmp(&b.file)));
		v
	}

	/// Returns the live files at a path, in ascending order of identity.
	///
	/// More than one is legal: two branches that independently created a path
	/// minted two files, both of which exist and both of which keep their bytes.
	pub fn at_path(&self, path: &[u8]) -> Vec<&Rendered> {
		let mut v: Vec<&Rendered> = self.files.iter()
			.filter(|f| f.live && f.path == path)
			.collect();
		v.sort_by_key(|f| f.file);
		v
	}

	/// Returns the paths more than one live file is claiming, each with those
	/// files in ascending order of identity.
	///
	/// Which of them a working copy writes under the shared name is a policy for
	/// the caller: the repository's answer is that both files exist.
	pub fn clashes(&self) -> Vec<(&[u8], Vec<OpId>)> {
		let mut by_path: BTreeMap<&[u8], Vec<OpId>> = BTreeMap::new();
		for f in self.files.iter().filter(|f| f.live) {
			by_path.entry(&f.path).or_default().push(f.file);
		}
		by_path.into_iter()
			.filter(|(_, ids)| ids.len() > 1)
			.map(|(path, mut ids)| {
				ids.sort();
				(path, ids)
			})
			.collect()
	}

	/// Returns everything the renderer noticed, over the whole repository.
	pub fn flags(&self) -> &[Flag] {
		&self.flags
	}

	/// Returns every note the operation set holds, in ascending order of
	/// identity, each listed once however many files its content is scattered
	/// over.
	///
	/// A note whose content has been deleted entirely is here too, saying so; see
	/// [`RepoNote::on_dead`].
	pub fn notes(&self) -> &[RepoNote] {
		&self.notes
	}

	/// Returns one note by the identity of the operation that wrote it.
	pub fn note(&self, note: OpId)
		-> Option<&RepoNote>
	{
		self.notes
			.binary_search_by(|n| n.note.cmp(&note))
			.ok()
			.and_then(|i| self.notes.get(i))
	}

	/// Returns the notes whose content renders nowhere at all, in ascending order
	/// of identity.
	pub fn dead_notes(&self) -> Vec<&RepoNote> {
		self.notes.iter().filter(|n| n.on_dead).collect()
	}

	/// Returns what the render cost.
	pub fn stats(&self) -> &Stats {
		&self.stats
	}

	/// Returns the number of files, deleted ones included.
	pub fn len(&self) -> usize {
		self.files.len()
	}

	/// Reports whether the repository holds no files.
	pub fn is_empty(&self) -> bool {
		self.files.is_empty()
	}

	/// Returns the file an operation's placement landed in, if it placed
	/// anything that reached a file.
	///
	/// This is the derived association a wire field would have asserted. It is
	/// computed by the render and may be cached beside the log, which is what a
	/// lazy fetcher needs in order to select one file's operations without
	/// resolving every anchor; a derived index may be rebuilt when it is wrong,
	/// and a wire field may not.
	pub fn file_of(&self, op: &OpId) -> Option<OpId> {
		self.index.get(op).copied()
	}

	/// Returns the whole operation-to-file association, in ascending order of
	/// operation.
	pub fn index(&self) -> &BTreeMap<OpId, OpId> {
		&self.index
	}
}


/// Which side of its parent a node sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildSide {
	/// Visited before the parent.
	Left,
	/// Visited after the parent.
	Right,
}


/// The bytes a traversal produced, where they went, and what it cost.
pub(super) struct Traversal {
	/// Rendered bytes and provenance, by file identity.
	pub files:		BTreeMap<OpId, (Vec<u8>, Vec<Run>)>,
	/// The file each slot ended up in, absent for a slot that reached none.
	pub owner:		Vec<Option<OpId>>,
	/// Slots that belong to no file, by placing operation and offset.
	pub orphans:	Vec<(OpId, u64)>,
	/// Live bytes owned by those slots, which render nowhere.
	pub orphaned:	u64,
	/// Deepest path in the forest.
	pub max_depth:	u32,
}

/// Builds the Fugue forest in topological order and walks it in order, emitting
/// each slot's bytes into the file whose subtree it is in.
///
/// The forest's root children are the seed slots, one per file, so a slot's file
/// is whichever seed it descends from. A slot that reaches the root without being
/// a seed belongs to no file: its origins were dropped, and it is reported rather
/// than quietly discarded, because the bytes it owns have to be accounted for.
///
/// Where a slot's two origins are still adjacent the published rule applies
/// unchanged. Where a move has separated them, the rule is re-run against the
/// left origin's current in-order successor, which is Fugue's own Algorithm 1
/// with "the next element" read at render time rather than taken from the
/// recorded anchor. Without that, an insertion abutting a moved range lands at
/// the far end of its left origin's subtree, which for a document of any size is
/// the end of the file.
pub(super) fn traverse(
	slots:	&Slots,
	ord:	&Order,
	claims:	&Claims,
	dead:	&Dead,
	atoms:	&Atoms,
)
	-> Outcome<Traversal>
{
	let sl = slots.all();
	let n = sl.len();
	if n >= u32::MAX as usize {
		return Err(err!(
			"A repository of {} slots exceeds what the forest's indices can address.", n;
		Excessive, Size));
	}
	let root = n;
	// Ancestor jumps for the subtree test, in powers of two.
	let mut log = 1usize;
	while (1usize << log) <= n {
		log += 1;
	}
	let mut parent: Vec<u32> = vec![root as u32; n + 1];
	let mut side: Vec<ChildSide> = vec![ChildSide::Right; n + 1];
	let mut depth: Vec<u32> = vec![0; n + 1];
	let mut up: Vec<u32> = vec![root as u32; (n + 1) * log];
	let mut kids_l: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
	let mut kids_r: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
	let mut owner: Vec<Option<OpId>> = vec![None; n];
	let mut orphans: Vec<(OpId, u64)> = Vec::new();
	let mut max_depth = 0u32;

	for &i in &ord.order {
		// A piece that is not the first of its placement hangs off its
		// predecessor, so a divided slot stays in one piece in the order.
		let (par, sd) = match slots.prev(i) {
			Some(prev) => (prev, ChildSide::Right),
			None => match (ord.left[i], ord.right[i]) {
				(None, None)		=> (root, ChildSide::Right),
				(None, Some(r))		=> (r, ChildSide::Left),
				(Some(l), None)		=> (l, ChildSide::Right),
				(Some(l), Some(r))	=> {
					if in_right_subtree(l, r, &depth, &up, &parent, &side, log) {
						(r, ChildSide::Left)
					} else {
						// The origins have been torn apart by a move, so the
						// recorded right origin says nothing about where this
						// belongs. The left origin's successor does.
						match successor(l, &kids_l, &kids_r, &parent, &side, root) {
							Some(s) if in_right_subtree(
								l, s, &depth, &up, &parent, &side, log)
								=> (s, ChildSide::Left),
							_	=> (l, ChildSide::Right),
						}
					}
				},
			},
		};
		if par == i {
			return Err(err!(
				"The slot placed by {} at offset {} resolved to itself as its own \
				parent.", sl[i].place, sl[i].sub;
			Bug));
		}
		parent[i] = par as u32;
		side[i] = sd;
		depth[i] = depth[par] + 1;
		max_depth = max_depth.max(depth[i]);
		up[i * log] = par as u32;
		for k in 1..log {
			up[i * log + k] = up[up[i * log + k - 1] as usize * log + k - 1];
		}
		// A slot's file is read off the forest: a seed opens one, and everything
		// beneath a slot is in the same file it is.
		owner[i] = if par == root {
			if sl[i].seed {
				Some(sl[i].place)
			} else {
				orphans.push((sl[i].place, sl[i].sub));
				None
			}
		} else {
			owner[par]
		};
		// Same-side siblings sit in op order, then in placement offset, which is
		// Fugue's sibling rule and the last of the five tie-breaks.
		let key = (OpOrder::of(&sl[i].place), sl[i].sub, i);
		let list = match sd {
			ChildSide::Left		=> &mut kids_l[par],
			ChildSide::Right	=> &mut kids_r[par],
		};
		let pos = list.partition_point(
			|j| (OpOrder::of(&sl[*j].place), sl[*j].sub, *j) < key);
		list.insert(pos, i);
	}

	let mut files: BTreeMap<OpId, (Vec<u8>, Vec<Run>)> = BTreeMap::new();
	let mut orphaned = 0u64;
	let mut stack: Vec<(usize, bool)> = vec![(root, false)];
	while let Some((i, emit)) = stack.pop() {
		if emit {
			if i == root {
				continue;
			}
			let slot = &sl[i];
			// A slot shows the parts of its claim it still owns, minus whatever
			// has died. A slot that has lost all of its claim shows nothing and
			// stays as an anchor target.
			let claimed = slot.claim.op();
			for (span, holder) in claims.runs(&slot.claim) {
				if holder != slot.place {
					continue;
				}
				for live in dead.live_runs(&claimed, span.clone()) {
					let run = res!(ContentRange::new(claimed, live.start, live.end));
					let file = match owner[i] {
						Some(f) => f,
						None => {
							orphaned += run.len();
							continue;
						},
					};
					let out = files.entry(file).or_default();
					let at = out.0.len() as u64;
					out.0.extend_from_slice(res!(atoms.slice(&run)));
					match out.1.last_mut() {
						Some(last) if last.content.op() == run.op()
							&& last.content.to() == run.from()
							=> res!(last.content.set_to(run.to())),
						_ => out.1.push(Run { at, content: run }),
					}
				}
			}
			continue;
		}
		for c in kids_r[i].iter().rev() {
			stack.push((*c, false));
		}
		stack.push((i, true));
		for c in kids_l[i].iter().rev() {
			stack.push((*c, false));
		}
	}

	Ok(Traversal { files, owner, orphans, orphaned, max_depth })
}

/// Resolves every note the operation set holds against the bytes the walk
/// produced, giving the notes each file should show and the repository's own view
/// of all of them.
///
/// The work is a reverse lookup over the provenance the walk already returned:
/// each run says which content is showing and where, so a note's content is
/// intersected with the runs and each intersection becomes a span. Nothing here
/// consults an anchor, a claim or a slot, which is why a note follows a move for
/// nothing -- the move has already happened, in the runs.
///
/// Everything the function reads is a function of the operation set, and every
/// list it returns is sorted by a total order over that set, so two replicas
/// holding the same operations resolve the same notes whatever order they arrived
/// in.
pub(super) fn notes(
	ops:	&[(OpId, &Op)],
	files:	&BTreeMap<OpId, (Vec<u8>, Vec<Run>)>,
)
	-> (BTreeMap<OpId, Vec<Note>>, Vec<RepoNote>)
{
	// Where each atom's bytes render: for one atom, the offsets it shows, the
	// file showing them, and the rendered offset the run begins at. Disjoint by
	// conservation, and sorted, so a note's range is found by binary search.
	let mut shown: BTreeMap<OpId, Vec<(Range<u64>, OpId, u64)>> = BTreeMap::new();
	for (file, (_, runs)) in files {
		for run in runs {
			if run.content.is_empty() {
				continue;
			}
			shown.entry(run.content.op())
				.or_default()
				.push((run.content.offsets(), *file, run.at));
		}
	}
	for v in shown.values_mut() {
		v.sort_by_key(|(iv, file, at)| (iv.start, iv.end, *file, *at));
	}

	let mut per_file: BTreeMap<OpId, Vec<Note>> = BTreeMap::new();
	let mut repo: Vec<RepoNote> = Vec::new();
	for (id, op) in ops {
		let text = match op {
			Op::Note { text, .. }	=> text,
			_						=> continue,
		};
		let mut found: BTreeMap<OpId, Vec<Span>> = BTreeMap::new();
		for r in op.note_on() {
			if r.is_empty() {
				continue;
			}
			let line = match shown.get(&r.op()) {
				Some(v)	=> v,
				None	=> continue,
			};
			// The first entry that can reach the range, and every one after it that
			// still starts before the range ends.
			let mut k = line.partition_point(|(iv, _, _)| iv.end <= r.from());
			while k < line.len() && line[k].0.start < r.to() {
				let (iv, file, at) = &line[k];
				let lo = iv.start.max(r.from());
				let hi = iv.end.min(r.to());
				if hi > lo {
					found.entry(*file).or_default().push(Span {
						at:		at + (lo - iv.start),
						len:	hi - lo,
					});
				}
				k += 1;
			}
		}
		let mut places: Vec<NotePlace> = Vec::with_capacity(found.len());
		for (file, mut spans) in found {
			spans.sort();
			// Abutting spans are one span: the render may show a note's content in
			// several runs, and a reader wants the region rather than the seams.
			let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
			for span in spans {
				match merged.last_mut() {
					Some(last) if last.end() >= span.at => {
						let end = last.end().max(span.end());
						last.len = end - last.at;
					},
					_ => merged.push(span),
				}
			}
			per_file.entry(file)
				.or_default()
				.push(Note::new(*id, text.clone(), merged.clone()));
			places.push(NotePlace { file, spans: merged });
		}
		repo.push(RepoNote::new(*id, text.clone(), places));
	}

	// In each file, the order a margin draws them in; over the repository, the
	// order every other list in the render is in.
	for v in per_file.values_mut() {
		v.sort_by_key(|n| (n.spans.first().map(|s| s.at).unwrap_or(0), n.note));
	}
	repo.sort_by_key(|n| n.note);
	(per_file, repo)
}

/// Returns the in-order successor of `v` among the nodes placed so far.
fn successor(
	v:		usize,
	kids_l:	&[Vec<usize>],
	kids_r:	&[Vec<usize>],
	parent:	&[u32],
	side:	&[ChildSide],
	root:	usize,
)
	-> Option<usize>
{
	if let Some(c) = kids_r[v].first() {
		let mut cur = *c;
		while let Some(x) = kids_l[cur].first() {
			cur = *x;
		}
		return Some(cur);
	}
	let mut cur = v;
	loop {
		let p = parent[cur] as usize;
		if p == cur || p == root {
			return None;
		}
		if side[cur] == ChildSide::Left {
			return Some(p);
		}
		cur = p;
	}
}

/// Reports whether `r` lies in the right subtree of `l`, by climbing `r`'s
/// ancestors in powers of two.
fn in_right_subtree(
	l:		usize,
	r:		usize,
	depth:	&[u32],
	up:		&[u32],
	parent:	&[u32],
	side:	&[ChildSide],
	log:	usize,
)
	-> bool
{
	if depth[r] <= depth[l] {
		return false;
	}
	// Climb to the child of `l`'s depth, then ask whether that is `l`'s right
	// child.
	let mut climb = depth[r] - depth[l] - 1;
	let mut cur = r;
	let mut k = 0usize;
	while climb > 0 && k < log {
		if climb & 1 == 1 {
			cur = up[cur * log + k] as usize;
		}
		climb >>= 1;
		k += 1;
	}
	parent[cur] as usize == l && side[cur] == ChildSide::Right
}
