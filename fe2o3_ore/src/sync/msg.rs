//! What one peer says to another.
//!
//! Four messages, and both peers may send all four. A message has a daticle
//! form, for a caller that keeps its wire in daticles, and a byte form that
//! begins with a magic and a version, for a caller that keeps a wire of bytes.
//! The version is there because these bytes cross between machines that were
//! built at different times, and a reader that cannot tell an old spelling from
//! a new one will eventually mistake one for the other.
//!
//! # Framing belongs to the transport
//!
//! [`Message::decode`] reads a message that occupies the whole of the buffer it
//! is given. Where one message ends and the next begins is a question the
//! carrier already answers -- a datagram has a length, a stream has whatever
//! framing it was given -- and answering it twice is how the two answers come to
//! disagree.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::id::OpId;
use crate::segment::Entry;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;


/// The bytes every sync message begins with.
pub const MAGIC: [u8; 6] = *b"ORESYN";

/// The newest format version this module writes.
///
/// Raised to 2 when the vocabulary gained [`Message::Part`], a piece of an
/// operation too large for the carrier to take whole. The four older kinds did
/// not move a byte and are still stamped [`VERSION_MIN`], because a message
/// declares the version it needs rather than the version its writer was built
/// at: a peer that speaks only version 1 still reads every message a version 1
/// peer could have sent, and refuses the one it could not have.
///
/// That is the whole of why the version moved rather than a kind byte being
/// added to [`crate::segment::Entry`] beside [`crate::segment::KIND_PACKED`].
/// A kind was right there because a segment stamps one version over many
/// records, so a bump would condemn every plain record beside the new one. A
/// message stamps its own version in its own bytes, so a bump condemns nothing
/// it does not have to, and it puts the refusal at the header -- before a
/// decode, naming both versions -- rather than inside `Entry::from_dat`, after
/// the handshake has already said the frame was compatible.
pub const VERSION: u8 = 2;

/// The oldest format version this module reads, and the version a message is
/// stamped with when it needs nothing newer.
///
/// It stays at 1 because version 2 is a strict superset: the framing is
/// identical, the four original kinds are spelled exactly as they were, and the
/// only thing a version 1 reader cannot do is read a kind that did not exist
/// when it was built. [`highest_kind`] is what keeps that true of the bytes and
/// not merely of the intention.
pub const VERSION_MIN: u8 = 1;

// The kind byte each message is tagged with on the wire.
pub const KIND_HELLO:	u8 = 1;
pub const KIND_SKETCH:	u8 = 2;
pub const KIND_SEND:	u8 = 3;
pub const KIND_DONE:	u8 = 4;
pub const KIND_PART:	u8 = 5;

/// Most bytes an operation may come to when its pieces are put back together.
///
/// A declared piece count is an instruction to allocate and it arrives from
/// wherever the message did, so nothing is ever sized from it: [`Parts`] grows
/// by what has actually arrived and refuses at this. The number is the largest
/// single operation whose arrival costs no more than opening the repository it
/// belongs to already costs -- an operation of *n* bytes is held about three
/// times over while it is taken in, and fe2o3's whole history peaks at 263 MB --
/// and it is the same sixty-four mebibytes a packed run may inflate to.
pub const PART_MAX: usize = 64 << 20;


/// The highest message kind a peer speaking the given format version may send.
///
/// The rule the vocabulary grows by, and it is the same rule
/// [`crate::segment::highest_code`] states for operations: a new kind goes
/// strictly above the existing ones, [`VERSION`] rises by one, this gains a
/// branch, and [`VERSION_MIN`] stays where it is. A message declaring version 1
/// and carrying kind 5 is refused rather than read, because a peer that would
/// read it is a peer that would also have to guess at what else the sender
/// thought version 1 meant.
pub const fn highest_kind(version: u8) -> u8 {
	if version <= VERSION_MIN {
		KIND_DONE
	} else {
		KIND_PART
	}
}


/// One thing a peer says.
///
/// The fields are public because a caller may legitimately want to work on a
/// message before it goes out -- to seal the records of a [`Message::Send`] with
/// a key this crate knows nothing about, most obviously. The encoding is
/// canonical whatever a caller builds: frontiers are written ascending and
/// without repetition, and a decoder refuses anything else.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
	// Opens a frontier walk.
	Hello {
		heads: Vec<OpId>,	// the speaker's frontier
	},
	// Opens a reconciliation.  The frontier rides along so that a receiver whose
	// decode stalls can answer with the walk in the same turn, rather than
	// spending a round trip asking for what it was already told.  The count is
	// advisory: it lets a peer judge whether the estimate the table was sized
	// from was sensible, and nothing is decided by it.
	Sketch {
		heads:	Vec<OpId>,	// the speaker's frontier
		cells:	Vec<u8>,	// the serialised table
		count:	u64,		// operations the speaker's log holds
	},
	// Operations the speaker owes, causally closed against what the receiver
	// holds.  Order carries no meaning -- OpLog::absorb places a batch however it
	// is shuffled -- but the sender writes them in a causal order anyway, so that
	// one pass places them all.
	Send {
		entries: Vec<Entry>,	// bare or sealed
	},
	Done,
	// One piece of an operation the carrier cannot take whole.  A part is not an
	// operation and never becomes one on its own: nothing places it, nothing
	// signs it, no log holds it and no segment may carry it.  What it carries is
	// a slice of the bytes an [`Entry`] encodes to, and the entry those slices
	// make is byte for byte the entry that was signed, so a signature crosses a
	// carrier that had to cut it up exactly as it crosses one that did not.
	//
	// The pieces of one operation arrive in order and are put back together by
	// [`Parts`], which is the transport's work and not a session's.
	Part {
		id:		OpId,		// the operation the pieces make
		seq:	u64,		// which piece this is, counted from zero
		total:	u64,		// how many pieces there are
		bytes:	Vec<u8>,	// this piece of the entry's encoded form
	},
}

impl Message {

	/// Puts the frontier in canonical order.
	pub fn hello(heads: Vec<OpId>) -> Self {
		Self::Hello { heads: canonical(heads) }
	}

	/// Puts the frontier in canonical order.
	pub fn sketch(heads: Vec<OpId>, cells: Vec<u8>, count: u64) -> Self {
		Self::Sketch { heads: canonical(heads), cells, count }
	}

	pub fn kind(&self) -> u8 {
		match self {
			Self::Hello { .. }	=> KIND_HELLO,
			Self::Sketch { .. }	=> KIND_SKETCH,
			Self::Send { .. }	=> KIND_SEND,
			Self::Done			=> KIND_DONE,
			Self::Part { .. }	=> KIND_PART,
		}
	}

	/// The oldest format version that can express this message.
	///
	/// A message is stamped with what it needs and not with what its writer was
	/// built at, so every message a version 1 peer could have sent is still
	/// spelled and stamped exactly as it was.
	pub fn version(&self) -> u8 {
		match self {
			Self::Part { .. }	=> VERSION,
			_					=> VERSION_MIN,
		}
	}

	/// For messages about messages.
	pub fn name(&self) -> &'static str {
		match self {
			Self::Hello { .. }	=> "hello",
			Self::Sketch { .. }	=> "sketch",
			Self::Send { .. }	=> "send",
			Self::Done			=> "done",
			Self::Part { .. }	=> "part",
		}
	}

	/// Does the message open an exchange, which is what a peer answers?
	pub fn is_opening(&self) -> bool {
		matches!(self, Self::Hello { .. } | Self::Sketch { .. })
	}

	/// Empty for the messages that carry no frontier.
	pub fn heads(&self) -> &[OpId] {
		match self {
			Self::Hello { heads }			=> heads,
			Self::Sketch { heads, .. }		=> heads,
			Self::Send { .. }
			| Self::Done
			| Self::Part { .. }				=> &[],
		}
	}

	/// Where a caller that requires provenance does its checking, before the
	/// message reaches a session: every [`Entry::Sealed`] can be put to
	/// [`crate::envelope::Envelope::verify`] under whatever scheme the caller
	/// holds. No scheme is chosen here and none is assumed.
	pub fn entries(&self) -> &[Entry] {
		match self {
			Self::Send { entries }	=> entries,
			Self::Hello { .. }
			| Self::Sketch { .. }
			| Self::Done
			| Self::Part { .. }		=> &[],
		}
	}

	/// Does the message hand operations over, whole or in pieces?
	///
	/// A carrier bounding what it will send has to know the difference between a
	/// message that carries work and one that only says something, because a
	/// bound that stops before any work has gone is a turn that tells the far end
	/// nothing it did not know -- and it comes back, and is told nothing again.
	/// A part carries work and holds no entries, which is why asking
	/// [`Message::entries`] is not the same question.
	pub fn carries_operations(&self) -> bool {
		match self {
			Self::Send { entries }	=> !entries.is_empty(),
			Self::Part { .. }		=> true,
			_						=> false,
		}
	}

	/// The shape is `[kind, body]`. The table of a sketch is a [`Dat::BU64`]: it
	/// readily exceeds the 255 bytes a [`Dat::BU8`] length field can express, and
	/// a truncated length there would corrupt silently.
	pub fn to_dat(&self) -> Dat {
		let body = match self {
			Self::Hello { heads } => Dat::List(
				canonical(heads.clone()).iter().map(|h| h.to_dat()).collect(),
			),
			Self::Sketch { heads, cells, count } => Dat::List(vec![
				Dat::List(canonical(heads.clone()).iter().map(|h| h.to_dat()).collect()),
				Dat::BU64(cells.clone()),
				Dat::U64(*count),
			]),
			Self::Send { entries } => Dat::List(
				entries.iter().map(|e| e.to_dat()).collect(),
			),
			Self::Done => Dat::List(Vec::new()),
			Self::Part { id, seq, total, bytes } => Dat::List(vec![
				id.to_dat(),
				Dat::U64(*seq),
				Dat::U64(*total),
				Dat::BU64(bytes.clone()),
			]),
		};
		Dat::List(vec![Dat::U8(self.kind()), body])
	}

	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Message expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let kind = match &v[0] {
			Dat::U8(k) => *k,
			other => return Err(err!(
				"A Message kind expects Dat::U8, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		match kind {
			KIND_HELLO => Ok(Self::Hello {
				heads: res!(heads_from_dat(&v[1], "hello")),
			}),
			KIND_SKETCH => {
				let f = match &v[1] {
					Dat::List(f) if f.len() == 3 => f,
					other => return Err(err!(
						"A sketch message expects a 3-element Dat::List, got {:?}.",
						other;
					Decode, Input, Mismatch)),
				};
				let cells = match &f[1] {
					Dat::BU64(b) => b.clone(),
					other => return Err(err!(
						"A sketch message's table expects Dat::BU64, got {:?}.", other;
					Decode, Input, Mismatch)),
				};
				let count = match &f[2] {
					Dat::U64(n) => *n,
					other => return Err(err!(
						"A sketch message's count expects Dat::U64, got {:?}.", other;
					Decode, Input, Mismatch)),
				};
				Ok(Self::Sketch {
					heads: res!(heads_from_dat(&f[0], "sketch")),
					cells,
					count,
				})
			},
			KIND_SEND => {
				let listed = match &v[1] {
					Dat::List(e) => e,
					other => return Err(err!(
						"A send message's operations expect Dat::List, got {:?}.", other;
					Decode, Input, Mismatch)),
				};
				let mut entries = Vec::with_capacity(listed.len());
				for item in listed {
					entries.push(res!(Entry::from_dat(item)));
				}
				Ok(Self::Send { entries })
			},
			KIND_DONE => match &v[1] {
				Dat::List(f) if f.is_empty() => Ok(Self::Done),
				other => Err(err!(
					"A done message expects an empty Dat::List, got {:?}.", other;
				Decode, Input, Mismatch)),
			},
			KIND_PART => {
				let f = match &v[1] {
					Dat::List(f) if f.len() == 4 => f,
					other => return Err(err!(
						"A part message expects a 4-element Dat::List, got {:?}.", other;
					Decode, Input, Mismatch)),
				};
				let id = res!(OpId::from_dat(&f[0]));
				let seq = match &f[1] {
					Dat::U64(n) => *n,
					other => return Err(err!(
						"A part message's position expects Dat::U64, got {:?}.", other;
					Decode, Input, Mismatch)),
				};
				let total = match &f[2] {
					Dat::U64(n) => *n,
					other => return Err(err!(
						"A part message's count expects Dat::U64, got {:?}.", other;
					Decode, Input, Mismatch)),
				};
				// One width and not the narrower ones a shorter piece would also
				// fit, for the reason a veiled body is written the same way: two
				// spellings of one piece would put back together into the same
				// operation and hash differently on the way.
				let bytes = match &f[3] {
					Dat::BU64(b) => b.clone(),
					other => return Err(err!(
						"The piece {} of {} is encoded {:?}; it is written under a \
						64-bit length, whatever its size.", seq, id, other;
					Decode, Input, Mismatch)),
				};
				if total == 0 {
					return Err(err!(
						"A part message says the operation {} was cut into no pieces \
						at all.", id;
					Decode, Input, Invalid));
				}
				if seq >= total {
					return Err(err!(
						"A part message is piece {} of the {} the operation {} was cut \
						into, counting from zero.", seq, total, id;
					Decode, Input, Range));
				}
				if bytes.is_empty() {
					return Err(err!(
						"The piece {} of {} carries no bytes, so it makes no progress \
						towards the operation and a peer could send them for ever.",
						seq, id;
					Decode, Input, Invalid));
				}
				Ok(Self::Part { id, seq, total, bytes })
			},
			other => Err(err!(
				"A Message is tagged {}, which names no message this version knows.",
				other;
			Decode, Input, Invalid)),
		}
	}

	/// The magic, then the version, then the daticle form.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		buf.extend_from_slice(&MAGIC);
		buf.push(self.version());
		let body = res!(self.to_dat().to_bytes(Vec::new()));
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


	/// Cuts one entry into pieces, each of which encodes to at most `cap` bytes.
	///
	/// This is the answer to an operation the carrier cannot take whole, and it
	/// is the only one that does not cost something else. Raising the carrier's
	/// limit fixes one operation and leaves the next; splitting the operation
	/// itself would change what was signed; compressing it moves the threshold
	/// rather than removing it. Cutting the *bytes* leaves the operation exactly
	/// as it was, which is what [`Parts`] puts back.
	///
	/// The pieces must be sent in the order they are handed back, and nothing may
	/// come between them.
	///
	/// # Arguments
	///
	/// * `cap` - The most one piece may encode to, framing excluded; a carrier
	///   that adds a length prefix subtracts it before asking.
	pub fn part(entry: &Entry, cap: usize)
		-> Outcome<Vec<Self>>
	{
		let id = res!(entry.id());
		let whole = res!(entry.to_dat().to_bytes(Vec::new()));
		if whole.len() > PART_MAX {
			return Err(err!(
				"The operation {} encodes to {} bytes, and a carrier will put back \
				together at most {}.", id, whole.len(), PART_MAX;
			Invalid, Data, Excessive));
		}
		// A piece's framing is fixed except for the two compact length prefixes a
		// daticle list carries, which widen with what they measure. So the room is
		// taken from an empty piece and then corrected against a real one, rather
		// than guessed at with a margin that would be wrong at some size nobody
		// tried.
		let empty = Self::Part { id, seq: 0, total: 1, bytes: Vec::new() };
		let mut room = cap.saturating_sub(res!(empty.encode()).len());
		loop {
			if room == 0 {
				return Err(err!(
					"A carrier taking {} bytes cannot carry a piece of the operation \
					{}: the framing of a piece comes to {} on its own.",
					cap, id, res!(empty.encode()).len();
				Invalid, Input, Range));
			}
			let sample = Self::Part {
				id,
				seq:	0,
				total:	1,
				bytes:	whole[..std::cmp::min(room, whole.len())].to_vec(),
			};
			let sized = res!(sample.encode()).len();
			if sized <= cap {
				break;
			}
			room = room.saturating_sub(sized - cap);
		}
		let total = whole.len().div_ceil(room) as u64;
		let mut out = Vec::with_capacity(total as usize);
		for (seq, piece) in whole.chunks(room).enumerate() {
			out.push(Self::Part {
				id,
				seq:	seq as u64,
				total,
				bytes:	piece.to_vec(),
			});
		}
		Ok(out)
	}

	/// The message must occupy the whole of `buf`; where one ends is the
	/// transport's question.
	pub fn decode(buf: &[u8])
		-> Outcome<Self>
	{
		let at = MAGIC.len() + 1;
		if buf.len() < at {
			return Err(err!(
				"A sync message of {} byte{} is too short to carry even its header.",
				buf.len(), if buf.len() == 1 { "" } else { "s" };
			Decode, Input, Missing));
		}
		if buf[..MAGIC.len()] != MAGIC {
			return Err(err!(
				"A sync message begins {:02x?}, which is not the magic {:02x?}.",
				&buf[..MAGIC.len()], MAGIC;
			Decode, Input, Invalid));
		}
		let version = buf[MAGIC.len()];
		if version < VERSION_MIN || version > VERSION {
			return Err(err!(
				"A sync message declares format version {}, and this reader knows \
				versions {} to {}.", version, VERSION_MIN, VERSION;
			Decode, Input, Version, Mismatch));
		}
		let (dat, used) = res!(Dat::from_bytes(&buf[at..]));
		if used != buf.len() - at {
			return Err(err!(
				"A sync message body of {} bytes decoded from only {} of them.",
				buf.len() - at, used;
			Decode, Input, Mismatch));
		}
		let msg = res!(Self::from_dat(&dat));
		// The version and the kind have to agree, or the superset promise
		// VERSION_MIN rests on is a promise about intentions.  A message stamped
		// with a version that could not have expressed it is refused here rather
		// than read, since a sender that spelled one of them wrong may have
		// spelled others wrong too.
		if msg.kind() > highest_kind(version) {
			return Err(err!(
				"A sync message declares format version {} and carries a {} message, \
				which is kind {}; version {} carries up to kind {}.",
				version, msg.name(), msg.kind(), version, highest_kind(version);
			Decode, Input, Version, Mismatch));
		}
		Ok(msg)
	}
}



/// One operation being put back together out of the pieces it crossed in.
///
/// A carrier holds one of these for each peer it is taking a push from, and a
/// peer that stops halfway leaves one holding bytes that will never be
/// completed. **Nothing is done with those bytes and nothing durable is written
/// from them.** A run begins at piece zero and a piece zero discards whatever
/// was held, so a push that died and was run again completes rather than
/// doubling; and since the operation was never absorbed, the rerun offers it
/// again of its own accord. That is Ore's resume-is-rerun, unchanged: the only
/// state a carrier keeps is a buffer it is free to throw away, and throwing it
/// away costs the sender a repetition and nothing else.
///
/// Every other message is handed straight back, so a caller folds one of these
/// over an arriving run and gets the run it would have had if the carrier had
/// been able to take the operation whole.
#[derive(Clone, Debug, Default)]
pub struct Parts {
	held:	Option<Holding>,
}

#[derive(Clone, Debug)]
struct Holding {
	id:		OpId,		// the operation the pieces make
	total:	u64,		// how many pieces were declared
	next:	u64,		// which piece is expected now
	bytes:	Vec<u8>,	// what has arrived, in the order it arrived
}

impl Parts {

	pub fn new() -> Self {
		Self::default()
	}

	/// Is something half arrived?
	pub fn pending(&self) -> bool {
		self.held.is_some()
	}

	/// How many bytes are being held towards an operation that has not finished
	/// arriving.
	pub fn held(&self) -> usize {
		match &self.held {
			Some(h)	=> h.bytes.len(),
			None	=> 0,
		}
	}

	/// Throws away whatever is half arrived.
	pub fn forget(&mut self) {
		self.held = None;
	}

	/// Takes one message, and hands back the message the caller should act on.
	///
	/// A piece that does not complete an operation yields nothing, because there
	/// is nothing yet to act on. The piece that completes one yields the send the
	/// operation would have crossed in had the carrier been able to take it
	/// whole, so everything downstream sees exactly what it would have seen.
	///
	/// A run that is interrupted by anything at all is refused rather than
	/// patched over: the pieces of one operation are sent together, so a message
	/// between them means the two ends disagree about what is being carried, and
	/// a carrier that guessed would be putting together an operation nobody sent.
	pub fn absorb(&mut self, msg: Message)
		-> Outcome<Option<Message>>
	{
		let (id, seq, total, bytes) = match msg {
			Message::Part { id, seq, total, bytes } => (id, seq, total, bytes),
			other => {
				if let Some(h) = self.held.take() {
					return Err(err!(
						"A {} message arrived between the pieces of {}, of which {} of \
						{} had crossed. The pieces of one operation are sent together \
						and nothing comes between them.",
						other.name(), h.id, h.next, h.total;
					Invalid, Input, Order));
				}
				return Ok(Some(other));
			},
		};
		if seq == 0 {
			// A run begins, and whatever was held for this peer is a run that
			// stopped. Nothing was absorbed from it, so nothing is lost by letting
			// it go; keeping it would be the carrier deciding which of two attempts
			// the sender meant.
			self.held = Some(Holding { id, total, next: 0, bytes: Vec::new() });
		}
		let held = match &mut self.held {
			Some(h) => h,
			None => return Err(err!(
				"The piece {} of {} arrived with nothing before it. A run of pieces \
				begins at zero, and a carrier that started in the middle would be \
				putting together an operation it had only part of.", seq, id;
			Invalid, Input, Order, Missing)),
		};
		if held.id != id || held.total != total || held.next != seq {
			let (was_id, was_total, want) = (held.id, held.total, held.next);
			self.held = None;
			return Err(err!(
				"The piece {} of {} of {} arrived where piece {} of {} of {} was \
				expected. The pieces of one operation are sent in order and nothing \
				comes between them.",
				seq, total, id, want, was_total, was_id;
			Invalid, Input, Order, Mismatch));
		}
		// Grown by what has arrived and never sized from what was declared: a
		// count is an instruction to allocate and it comes from wherever the
		// message did.
		if held.bytes.len() + bytes.len() > PART_MAX {
			let (was_id, sofar) = (held.id, held.bytes.len());
			self.held = None;
			return Err(err!(
				"The pieces of {} come to more than the {} bytes a carrier will put \
				back together: {} had arrived and another {} followed.",
				was_id, PART_MAX, sofar, bytes.len();
			Invalid, Data, Excessive));
		}
		held.bytes.extend_from_slice(&bytes);
		held.next += 1;
		if held.next < held.total {
			return Ok(None);
		}
		let done = match self.held.take() {
			Some(h) => h,
			None => return Err(err!(
				"The pieces of {} completed and there is nothing held.", id;
			Bug, Unreachable)),
		};
		let (dat, used) = res!(Dat::from_bytes(&done.bytes));
		if used != done.bytes.len() {
			return Err(err!(
				"The {} pieces of {} came to {} bytes and decoded from only {} of \
				them.", done.total, done.id, done.bytes.len(), used;
			Decode, Input, Mismatch));
		}
		let entry = res!(Entry::from_dat(&dat));
		// The clear identifier and the one inside are compared for the reason a
		// veiled entry's two headers are: a carrier that cut an operation up is a
		// carrier that could have relabelled the pieces, and the record inside is
		// the signed one.
		let inside = res!(entry.id());
		if inside != done.id {
			return Err(err!(
				"The pieces said they made the operation {} and they make {}. The \
				record inside is the signed one and is what to believe.",
				done.id, inside;
			Invalid, Input, Security, Mismatch));
		}
		Ok(Some(Message::Send { entries: vec![entry] }))
	}
}


/// The order the encoding spells a frontier in: ascending, without repetition.
fn canonical(heads: Vec<OpId>) -> Vec<OpId> {
	let mut heads = heads;
	heads.sort();
	heads.dedup();
	heads
}

/// Refuses a frontier that is not in canonical order, rather than sorting it.
fn heads_from_dat(dat: &Dat, what: &str)
	-> Outcome<Vec<OpId>>
{
	let listed = match dat {
		Dat::List(v) => v,
		other => return Err(err!(
			"A {} message's frontier expects Dat::List, got {:?}.", what, other;
		Decode, Input, Mismatch)),
	};
	let mut heads: Vec<OpId> = Vec::with_capacity(listed.len());
	for item in listed {
		let id = res!(OpId::from_dat(item));
		if let Some(last) = heads.last() {
			if id <= *last {
				return Err(err!(
					"A {} message lists {} after {}; a frontier is encoded ascending \
					and without repetition.", what, id, last;
				Decode, Input, Order));
			}
		}
		heads.push(id);
	}
	Ok(heads)
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::envelope::Envelope;
	use crate::id::ReplicaId;
	use crate::op::{
		Header,
		Op,
		Record,
	};
	use crate::test_support::StubSigner;

	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// One of each message, with both entry forms among them.
	fn samples()
		-> Outcome<Vec<Message>>
	{
		let rec = Record::new(
			res!(Header::new(oid(2, 3), vec![oid(1, 1), oid(1, 2)])),
			Op::FileCreate { path: b"notes.md".to_vec() },
		);
		let sealed = res!(Envelope::seal_record(
			&StubSigner::with_seed(3),
			&Record::root(oid(1, 1), Op::Mark { name: fmt!("start"), body: None, time: None }),
		));
		Ok(vec![
			Message::hello(vec![oid(3, 9), oid(1, 2)]),
			Message::hello(Vec::new()),
			Message::sketch(vec![oid(1, 2)], vec![0x11; 300], 42),
			Message::Send { entries: vec![
				Entry::Bare(rec),
				Entry::Sealed(sealed),
			] },
			Message::Send { entries: Vec::new() },
			Message::Done,
			Message::Part { id: oid(2, 3), seq: 0, total: 3, bytes: vec![0x5a; 40] },
			Message::Part { id: oid(2, 3), seq: 2, total: 3, bytes: vec![0x01] },
		])
	}

	#[test]
	fn messages_round_trip() -> Outcome<()> {
		for msg in res!(samples()) {
			assert_eq!(res!(Message::from_dat(&msg.to_dat())), msg, "as a daticle");
			let bytes = res!(msg.encode());
			assert_eq!(res!(Message::decode(&bytes)), msg, "as bytes");
			assert_eq!(&bytes[..MAGIC.len()], &MAGIC);
			// The version a message is stamped with is the version it needs, so
			// every message a version 1 peer could have sent still says 1.
			assert_eq!(bytes[MAGIC.len()], msg.version(), "{} is stamped wrongly", msg.name());
		}
		Ok(())
	}

	/// A frontier is a set, so it is encoded ascending and without repetition
	/// however it was handed over, and a decoder refuses any other spelling.
	/// A run of pieces puts the entry back byte for byte, signature included.
	///
	/// This is the whole claim chunking makes and the one thing it may not get
	/// wrong. The bytes compared are the entry's own encoding, so a reassembly
	/// that produced an equal-looking entry by re-encoding it would still fail;
	/// and the envelope is verified afterwards, because a signature is over bytes
	/// and equality of a struct is not equality of bytes.
	#[test]
	fn a_run_of_pieces_puts_the_signed_entry_back_byte_for_byte() -> Outcome<()> {
		let signer = StubSigner::with_seed(11);
		let rec = Record::new(
			res!(Header::new(oid(4, 12), vec![oid(4, 11)])),
			Op::Mark {
				name:	fmt!("a mark whose body is large enough to need cutting up"),
				body:	Some(vec![0xa7u8; 20_000]),
				time:	Some(1_755_000_000),
			},
		);
		let entry = Entry::Sealed(res!(Envelope::seal_record(&signer, &rec)));
		let was = res!(entry.to_dat().to_bytes(Vec::new()));
		let pieces = res!(Message::part(&entry, 4_096));
		assert!(pieces.len() > 4, "an entry of {} bytes made {} pieces", was.len(), pieces.len());

		let mut parts = Parts::new();
		let mut got = None;
		for (at, piece) in pieces.iter().enumerate() {
			// Every piece crosses the wire, because that is where a spelling that
			// only round-trips in memory would go wrong.
			let piece = res!(Message::decode(&res!(piece.encode())));
			match res!(parts.absorb(piece)) {
				Some(msg) => {
					assert_eq!(at, pieces.len() - 1, "the run completed early");
					got = Some(msg);
				},
				None => assert!(parts.pending(), "a piece was taken and nothing is held"),
			}
		}
		assert!(!parts.pending(), "the run completed and something is still held");
		let back = match got {
			Some(Message::Send { entries }) => entries,
			other => return Err(err!(
				"A completed run yielded {:?} rather than a send.", other; Test, Mismatch)),
		};
		assert_eq!(back.len(), 1);
		assert_eq!(res!(back[0].to_dat().to_bytes(Vec::new())), was,
			"the entry that came back is not the entry that went, byte for byte");
		// And the signature over those bytes still verifies.
		match &back[0] {
			Entry::Sealed(env) => assert!(res!(env.verify(&signer)),
				"the signature did not survive being cut up"),
			other => return Err(err!(
				"A sealed entry came back as a {}.", other.name(); Test, Mismatch)),
		}
		Ok(())
	}

	/// Every piece fits the carrier that asked for it, at every cap, and the
	/// pieces are the whole in order.
	///
	/// BOTH HALVES, because either alone is satisfied by something useless: one
	/// piece per byte respects any cap, and one piece carrying everything
	/// preserves any order. The first half is the defect this exists for -- a
	/// carrier told a limit and handed something over it is the failure that
	/// reads as the relay being down.
	#[test]
	fn every_piece_fits_the_cap_and_the_pieces_are_the_whole() -> Outcome<()> {
		let entry = Entry::Bare(Record::new(
			res!(Header::new(oid(9, 2), vec![oid(9, 1)])),
			Op::Mark { name: fmt!("wide"), body: Some(vec![0x7eu8; 30_000]), time: None },
		));
		let was = res!(entry.to_dat().to_bytes(Vec::new()));
		for cap in [96usize, 200, 1_024, 4_096, 65_536, 1 << 20] {
			let pieces = res!(Message::part(&entry, cap));
			let mut seen: Vec<u8> = Vec::new();
			for (at, piece) in pieces.iter().enumerate() {
				let sized = res!(piece.encode()).len();
				assert!(sized <= cap,
					"piece {} of {} encodes to {} against a cap of {}",
					at, pieces.len(), sized, cap);
				match piece {
					Message::Part { id, seq, total, bytes } => {
						assert_eq!(*id, res!(entry.id()));
						assert_eq!(*seq, at as u64, "the pieces are out of order");
						assert_eq!(*total as usize, pieces.len(), "the count is wrong");
						seen.extend_from_slice(bytes);
					},
					other => return Err(err!(
						"Parting an entry produced a {} message.", other.name();
					Test, Mismatch)),
				}
			}
			assert_eq!(seen, was, "the pieces at a cap of {} are not the whole", cap);
		}
		// A cap under the framing of a piece is refused by name rather than
		// producing a piece that does not fit it.
		assert!(Message::part(&entry, 8).is_err(), "a cap of eight bytes was accepted");
		Ok(())
	}

	/// A run that is broken in any way is refused, and nothing half assembled is
	/// kept.
	#[test]
	fn a_broken_run_is_refused_rather_than_patched() -> Outcome<()> {
		let entry = Entry::Bare(Record::new(
			res!(Header::new(oid(5, 2), vec![oid(5, 1)])),
			Op::Mark { name: fmt!("five"), body: Some(vec![0x33u8; 2_000]), time: None },
		));
		let pieces = res!(Message::part(&entry, 512));
		assert!(pieces.len() >= 4);

		// A run that begins in the middle.
		let mut parts = Parts::new();
		assert!(parts.absorb(pieces[1].clone()).is_err(), "a run beginning at piece one");
		assert!(!parts.pending());

		// A piece skipped.
		let mut parts = Parts::new();
		res!(parts.absorb(pieces[0].clone()));
		assert!(parts.absorb(pieces[2].clone()).is_err(), "a piece skipped");
		assert!(!parts.pending(), "a refused run left something held");

		// Another operation's piece in the middle of this one.
		let other = res!(Message::part(&Entry::Bare(Record::new(
			res!(Header::new(oid(6, 2), vec![oid(6, 1)])),
			Op::Mark { name: fmt!("six"), body: Some(vec![0x44u8; 2_000]), time: None },
		)), 512));
		let mut parts = Parts::new();
		res!(parts.absorb(pieces[0].clone()));
		assert!(parts.absorb(other[1].clone()).is_err(), "another operation's piece");

		// Anything at all between the pieces.
		let mut parts = Parts::new();
		res!(parts.absorb(pieces[0].clone()));
		assert!(parts.absorb(Message::Done).is_err(), "a done between the pieces");
		assert!(!parts.pending());

		// Pieces that make bytes which are not an entry.
		let mut parts = Parts::new();
		let rubbish: Vec<Message> = (0..2u64)
			.map(|seq| Message::Part {
				id:		oid(5, 2),
				seq,
				total:	2,
				bytes:	vec![0xffu8; 8],
			})
			.collect();
		res!(parts.absorb(rubbish[0].clone()));
		assert!(parts.absorb(rubbish[1].clone()).is_err(), "bytes that are not an entry");

		// Pieces that make an entry which is not the one they claim.
		let whole = res!(entry.to_dat().to_bytes(Vec::new()));
		let mut parts = Parts::new();
		assert!(parts.absorb(Message::Part {
			id:		oid(7, 7),
			seq:	0,
			total:	1,
			bytes:	whole,
		}).is_err(), "pieces labelled as another operation");

		// And the spellings a decoder refuses outright.
		for wrong in [
			Message::Part { id: oid(5, 2), seq: 0, total: 0, bytes: vec![1] },
			Message::Part { id: oid(5, 2), seq: 3, total: 3, bytes: vec![1] },
			Message::Part { id: oid(5, 2), seq: 0, total: 2, bytes: Vec::new() },
		] {
			assert!(Message::from_dat(&wrong.to_dat()).is_err(),
				"a part {:?} was accepted", wrong);
		}
		Ok(())
	}

	/// A push that died halfway and was run again completes rather than doubling.
	///
	/// The carrier keeps nothing durable and the operation was never absorbed, so
	/// the rerun offers the whole of it again; piece zero is what says a run has
	/// begun, and it throws away what the attempt before it left.
	#[test]
	fn a_rerun_completes_rather_than_doubling() -> Outcome<()> {
		let entry = Entry::Bare(Record::new(
			res!(Header::new(oid(8, 2), vec![oid(8, 1)])),
			Op::Mark { name: fmt!("eight"), body: Some(vec![0x21u8; 4_000]), time: None },
		));
		let was = res!(entry.to_dat().to_bytes(Vec::new()));
		let pieces = res!(Message::part(&entry, 512));
		assert!(pieces.len() >= 4);

		let mut parts = Parts::new();
		// The push gets halfway and stops.
		for piece in &pieces[..2] {
			assert!(res!(parts.absorb(piece.clone())).is_none());
		}
		assert!(parts.pending(), "nothing was held when the push stopped");
		assert!(parts.held() > 0);

		// It is run again from the beginning, against a carrier still holding the
		// remains of the first attempt.
		let mut got = None;
		for piece in &pieces {
			if let Some(msg) = res!(parts.absorb(piece.clone())) {
				got = Some(msg);
			}
		}
		let back = match got {
			Some(Message::Send { entries }) => entries,
			other => return Err(err!(
				"The rerun yielded {:?} rather than a send.", other; Test, Mismatch)),
		};
		assert_eq!(back.len(), 1, "the rerun produced {} entries", back.len());
		assert_eq!(res!(back[0].to_dat().to_bytes(Vec::new())), was,
			"the rerun did not put the operation back as it was");
		assert!(!parts.pending());
		Ok(())
	}

	/// A peer that speaks only version 1 refuses a part by name, and reads every
	/// message a version 1 peer could have sent.
	///
	/// The refusal is at the header, before a decode, which is the reason the
	/// version moved rather than a kind byte being added to an entry: a kind
	/// would have been refused inside `Entry::from_dat`, after the handshake had
	/// already said the frame was compatible.
	#[test]
	fn a_version_and_a_kind_have_to_agree() -> Outcome<()> {
		let part = Message::Part { id: oid(1, 1), seq: 0, total: 1, bytes: vec![0x01] };
		let bytes = res!(part.encode());
		assert_eq!(bytes[MAGIC.len()], VERSION, "a part is stamped with the version it needs");

		// The same message stamped as version 1, which is what an old peer would
		// have to believe to read it.
		let mut lying = bytes.clone();
		lying[MAGIC.len()] = VERSION_MIN;
		let e = match Message::decode(&lying) {
			Ok(got) => return Err(err!(
				"A part stamped version {} decoded as a {}.", VERSION_MIN, got.name();
			Test, Mismatch)),
			Err(e) => e,
		};
		assert!(fmt!("{}", e).contains("kind"), "message was {}", e);

		assert_eq!(highest_kind(VERSION_MIN), KIND_DONE);
		assert_eq!(highest_kind(VERSION), KIND_PART);
		// Every older message still says 1, so an old peer reads it unchanged.
		for msg in res!(samples()) {
			let stamped = res!(msg.encode())[MAGIC.len()];
			match msg {
				Message::Part { .. }	=> assert_eq!(stamped, VERSION),
				_						=> assert_eq!(stamped, VERSION_MIN,
					"the {} message stopped being a version 1 message", msg.name()),
			}
		}
		Ok(())
	}

	/// The bytes of a piece, frozen.
	///
	/// A part is the one message that crosses in pieces, so a change to its
	/// spelling stops two builds putting the same operation back together while
	/// each remains correct against itself -- the failure a golden exists for.
	/// The payload is the entry's own bytes and not a re-encoding of them, which
	/// is what keeps the signature over them intact, and freezing them here is
	/// what says so.
	#[test]
	fn the_part_bytes_are_frozen() -> Outcome<()> {
		let entry = Entry::Bare(Record::new(
			res!(Header::new(oid(2, 3), vec![oid(1, 7)])),
			Op::Mark { name: fmt!("v1"), body: None, time: None },
		));
		let pieces = res!(Message::part(&entry, 96));
		assert_eq!(pieces.len(), 2, "the entry was not cut into two pieces");
		let want: &[u8] = &[
			// The magic, and the version a part needs.
			0x4f, 0x52, 0x45, 0x53, 0x59, 0x4e,
			0x02,
			// The message: a two-element list of the kind and the body, 86 bytes.
			0x33, 0x21, 0x56,
				// The kind: part.
				0x0a, 0x05,
				// The body: identifier, position, count, bytes; 81 bytes.
				0x33, 0x21, 0x51,
					// The operation the pieces make, r2:3.
					0x33, 0x21, 0x12,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
					// Piece zero.
					0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
					// Of two.
					0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
					// Thirty-three bytes, under a 64-bit length whatever its size.
					0x47, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21,
						// The first thirty-three bytes of the entry, which are the
						// entry's own bytes and not a re-encoding of it: the kind, the
						// record, the header, the identifier r2:3.
						0x33, 0x21, 0x3f,
							0x0a, 0x01,
							0x33, 0x21, 0x3a,
								0x33, 0x21, 0x2d,
									0x33, 0x21, 0x12,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
									0x33,
		];
		assert_eq!(res!(pieces[0].encode()), want, "the part message format has changed");
		// And the frozen bytes still read.
		assert_eq!(res!(Message::decode(want)), pieces[0]);
		// The payload is a prefix of the entry's own bytes, which is the claim.
		let whole = res!(entry.to_dat().to_bytes(Vec::new()));
		assert_eq!(&want[63..], &whole[..33],
			"a piece carries something other than the entry's own bytes");
		Ok(())
	}

	#[test]
	fn a_frontier_is_encoded_canonically() -> Outcome<()> {
		let msg = Message::hello(vec![oid(3, 1), oid(1, 1), oid(3, 1), oid(2, 1)]);
		assert_eq!(msg.heads(), vec![oid(1, 1), oid(2, 1), oid(3, 1)]);
		// Built by hand, out of order, it still encodes canonically.
		let odd = Message::Hello { heads: vec![oid(3, 1), oid(1, 1)] };
		assert_eq!(
			res!(Message::from_dat(&odd.to_dat())),
			Message::hello(vec![oid(1, 1), oid(3, 1)]),
		);
		// And a daticle spelling it otherwise is refused rather than sorted.
		let wrong = Dat::List(vec![
			Dat::U8(KIND_HELLO),
			Dat::List(vec![oid(3, 1).to_dat(), oid(1, 1).to_dat()]),
		]);
		let _ = msg;
		let e = match Message::from_dat(&wrong) {
			Ok(_) => return Err(err!("A frontier out of order was accepted."; Test)),
			Err(e) => e,
		};
		assert!(fmt!("{}", e).contains("ascending"), "message was {}", e);
		// Repetition likewise.
		let twice = Dat::List(vec![
			Dat::U8(KIND_HELLO),
			Dat::List(vec![oid(1, 1).to_dat(), oid(1, 1).to_dat()]),
		]);
		assert!(Message::from_dat(&twice).is_err());
		Ok(())
	}

	/// Truncating a message anywhere is a typed error, never a panic and never a
	/// half-read message.
	#[test]
	fn truncation_at_every_offset_is_clean() -> Outcome<()> {
		for msg in res!(samples()) {
			let bytes = res!(msg.encode());
			for cut in 0..bytes.len() {
				match Message::decode(&bytes[..cut]) {
					Ok(got) => return Err(err!(
						"A {} message cut at {} of {} decoded as a {}.",
						msg.name(), cut, bytes.len(), got.name();
					Test, Mismatch)),
					Err(_) => {},
				}
			}
			assert_eq!(res!(Message::decode(&bytes)), msg);
			// And trailing rubbish is refused, not ignored.
			let mut extra = bytes.clone();
			extra.push(0x00);
			assert!(Message::decode(&extra).is_err(), "{} with a trailing byte", msg.name());
		}
		Ok(())
	}

	#[test]
	fn a_message_that_is_not_one_is_refused() -> Outcome<()> {
		assert!(Message::decode(b"").is_err());
		assert!(Message::decode(b"not a sync message").is_err());
		let mut wrong = MAGIC.to_vec();
		wrong.push(VERSION + 1);
		wrong.push(0);
		let e = match Message::decode(&wrong) {
			Ok(_) => return Err(err!("An unknown version was accepted."; Test)),
			Err(e) => e,
		};
		assert!(fmt!("{}", e).contains("version"), "message was {}", e);
		// A kind nobody knows.
		let odd = Dat::List(vec![Dat::U8(99), Dat::List(Vec::new())]);
		assert!(Message::from_dat(&odd).is_err());
		// A done message carrying something.
		let heavy = Dat::List(vec![
			Dat::U8(KIND_DONE),
			Dat::List(vec![Dat::U64(1)]),
		]);
		assert!(Message::from_dat(&heavy).is_err());
		Ok(())
	}

	#[test]
	fn accessors_report_what_is_there() -> Outcome<()> {
		let msgs = res!(samples());
		assert!(msgs[0].is_opening());
		assert!(msgs[2].is_opening());
		assert!(!msgs[3].is_opening());
		assert!(!Message::Done.is_opening());
		assert_eq!(msgs[2].heads(), vec![oid(1, 2)]);
		assert!(msgs[3].heads().is_empty());
		assert_eq!(msgs[3].entries().len(), 2);
		assert!(msgs[0].entries().is_empty());
		assert_eq!(Message::Done.name(), "done");
		assert_eq!(Message::Done.kind(), KIND_DONE);
		Ok(())
	}

	/// A format that changes by accident leaves two versions of this crate unable
	/// to speak to each other, and every other test in this file would pass
	/// regardless: they all encode and decode with the same code. This one is the
	/// fixed point. If it fails and the change was deliberate, the version byte is
	/// the thing to raise.
	#[test]
	fn the_message_bytes_are_frozen() -> Outcome<()> {
		let msg = Message::Send {
			entries: vec![Entry::Bare(Record::new(
				res!(Header::new(oid(2, 3), vec![oid(1, 7)])),
				Op::Mark { name: fmt!("v1"), body: None, time: None },
			))],
		};
		let want: &[u8] = &[
			// The magic and the version.
			0x4f, 0x52, 0x45, 0x53, 0x59, 0x4e,
			0x01,
			// The message: a two-element list of the kind and the body, 71 bytes.
			0x33, 0x21, 0x47,
				// The kind: send.
				0x0a, 0x03,
				// The body: a list of one entry, 66 bytes.
				0x33, 0x21, 0x42,
					// The entry, 63 bytes: the kind, and the record.
					0x33, 0x21, 0x3f,
						// Bare.
						0x0a, 0x01,
						// The record, 58 bytes: the header, the operation.
						0x33, 0x21, 0x3a,
							// The header, 45 bytes: the identifier, then the parents.
							0x33, 0x21, 0x2d,
								// The identifier r2:3, as two 64-bit integers.
								0x33, 0x21, 0x12,
									0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
									0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
								// One parent, r1:7.
								0x33, 0x21, 0x15,
									0x33, 0x21, 0x12,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07,
							// The operation, 7 bytes: the Mark code, and the name "v1".
							0x33, 0x21, 0x07,
								0x0a, 0x04,
								0x29, 0x21, 0x02, 0x76, 0x31,
		];
		assert_eq!(res!(msg.encode()), want, "the sync message format has changed");
		// And the frozen bytes still read.
		assert_eq!(res!(Message::decode(want)), msg);
		Ok(())
	}
}
