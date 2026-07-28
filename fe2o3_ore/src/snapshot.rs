//! A rendered state at a frontier, as bytes.
//!
//! A snapshot is what the repository looked like when the operations named by a
//! frontier had been applied and no others. It exists so that a reader does not
//! have to replay a history from its first operation to see the present: it
//! takes the snapshot, and then only the segments holding operations the
//! frontier does not already cover.
//!
//! What is stored is a render and not a state machine. Per file: the bytes, the
//! provenance of those bytes, and what the renderer noticed. The provenance is
//! there because without it the bytes are dumb -- a frontend that means to
//! author a content-anchored splice against what it is showing needs to know
//! what the byte under the cursor is called, and that is exactly what a run
//! says.
//!
//! # What a snapshot is not
//!
//! It is not a substitute for the operations. The sequence structure is rebuilt
//! from the operation set on every render, so continuing to merge concurrent
//! history needs the operations themselves; a snapshot is a materialised view
//! taken at a point, and a reader that only wants to *see* the state can stop
//! there. Nothing is lost by discarding a snapshot, and everything is lost by
//! discarding a segment, which is the asymmetry to keep in mind when deciding
//! what to prune.
//!
//! # No I/O
//!
//! As everywhere else in the crate: bytes in, bytes out, and where they live is
//! the caller's business.

use crate::id::OpId;
use crate::seq::render::{
	Flag,
	Rendered,
	Run,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;


/// The bytes every snapshot begins with.
pub const MAGIC: [u8; 6] = *b"ORESNP";

/// The format version this module writes.
pub const VERSION: u8 = 1;


/// One file, as it stood.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileState {
	/// Path of the file.
	pub path:	String,
	/// The rendered bytes.
	pub bytes:	Vec<u8>,
	/// What those bytes are made of, in render order and coalesced.
	pub runs:	Vec<Run>,
	/// What the renderer noticed while producing them.
	pub flags:	Vec<Flag>,
}

impl FileState {
	/// Takes a render as the state of the file at the given path.
	pub fn of(path: String, rendered: &Rendered) -> Self {
		Self {
			path,
			bytes:	rendered.bytes().to_vec(),
			runs:	rendered.runs().to_vec(),
			flags:	rendered.flags().to_vec(),
		}
	}

	/// Serialises the state to a [`Dat`]. The shape is
	/// `[path, bytes, [run, ...], [flag, ...]]`.
	///
	/// The bytes are a [`Dat::BU64`]: a file is routinely longer than the 255
	/// bytes a [`Dat::BU8`] length field can express, and a truncated length
	/// there would corrupt silently.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::Str(self.path.clone()),
			Dat::BU64(self.bytes.clone()),
			Dat::List(self.runs.iter().map(|r| r.to_dat()).collect()),
			Dat::List(self.flags.iter().map(|f| f.to_dat()).collect()),
		])
	}

	/// Reconstructs a state from a [`Dat`] produced by [`FileState::to_dat`].
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 4 => v,
			_ => return Err(err!(
				"A FileState expects a 4-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let path = match &v[0] {
			Dat::Str(s) => s.clone(),
			other => return Err(err!(
				"A FileState path expects Dat::Str, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		let bytes = match &v[1] {
			Dat::BU64(b) => b.clone(),
			other => return Err(err!(
				"The rendered bytes of {:?} expect Dat::BU64, got {:?}.", path, other;
			Decode, Input, Mismatch)),
		};
		let listed = match &v[2] {
			Dat::List(l) => l,
			other => return Err(err!(
				"The runs of {:?} expect Dat::List, got {:?}.", path, other;
			Decode, Input, Mismatch)),
		};
		let mut runs = Vec::with_capacity(listed.len());
		for item in listed {
			runs.push(res!(Run::from_dat(item)));
		}
		let listed = match &v[3] {
			Dat::List(l) => l,
			other => return Err(err!(
				"The flags of {:?} expect Dat::List, got {:?}.", path, other;
			Decode, Input, Mismatch)),
		};
		let mut flags = Vec::with_capacity(listed.len());
		for item in listed {
			flags.push(res!(Flag::from_dat(item)));
		}
		Ok(Self { path, bytes, runs, flags })
	}
}


/// The repository as it stood at a frontier.
///
/// Both lists are held sorted and without repetition, so that one state has
/// exactly one byte spelling and two replicas that snapshot the same frontier
/// produce the same bytes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
	/// The operations the state includes everything up to, ascending.
	frontier:	Vec<OpId>,
	/// The files, in ascending order of path.
	files:		Vec<FileState>,
}

impl Snapshot {

	/// Constructs a snapshot, sorting both lists.
	///
	/// Fails where two states name one path, since which of them the file was is
	/// then unanswerable. Repeated frontier entries are dropped, a frontier being
	/// a set.
	pub fn new(frontier: Vec<OpId>, files: Vec<FileState>)
		-> Outcome<Self>
	{
		let mut frontier = frontier;
		frontier.sort();
		frontier.dedup();
		let mut files = files;
		files.sort_by(|a, b| a.path.cmp(&b.path));
		for pair in files.windows(2) {
			if pair[0].path == pair[1].path {
				return Err(err!(
					"A snapshot names the file {:?} twice.", pair[0].path;
				Invalid, Input, Duplicate));
			}
		}
		Ok(Self { frontier, files })
	}

	/// Returns the frontier the state was rendered at.
	pub fn frontier(&self) -> &[OpId] {
		&self.frontier
	}

	/// Returns the files, in ascending order of path.
	pub fn files(&self) -> &[FileState] {
		&self.files
	}

	/// Returns the state of one file, if the snapshot holds it.
	pub fn file(&self, path: &str)
		-> Option<&FileState>
	{
		self.files
			.binary_search_by(|f| f.path.as_str().cmp(path))
			.ok()
			.and_then(|i| self.files.get(i))
	}

	/// Returns the number of files.
	pub fn len(&self) -> usize {
		self.files.len()
	}

	/// Reports whether the snapshot holds no files.
	pub fn is_empty(&self) -> bool {
		self.files.is_empty()
	}

	/// Serialises the snapshot to a [`Dat`]. The shape is
	/// `[[op, ...], [file, ...]]`.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			Dat::List(self.frontier.iter().map(|id| id.to_dat()).collect()),
			Dat::List(self.files.iter().map(|f| f.to_dat()).collect()),
		])
	}

	/// Reconstructs a snapshot from a [`Dat`] produced by [`Snapshot::to_dat`].
	///
	/// Either list out of order, or repeated, is refused rather than normalised,
	/// so that the encoding stays canonical.
	pub fn from_dat(dat: &Dat)
		-> Outcome<Self>
	{
		let v = match dat {
			Dat::List(v) if v.len() == 2 => v,
			_ => return Err(err!(
				"A Snapshot expects a 2-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let listed = match &v[0] {
			Dat::List(l) => l,
			other => return Err(err!(
				"A Snapshot frontier expects Dat::List, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		let mut frontier: Vec<OpId> = Vec::with_capacity(listed.len());
		for item in listed {
			let id = res!(OpId::from_dat(item));
			if let Some(last) = frontier.last() {
				if id <= *last {
					return Err(err!(
						"A Snapshot frontier lists {} after {}; a frontier is encoded \
						ascending and without repetition.", id, last;
					Decode, Input, Order));
				}
			}
			frontier.push(id);
		}
		let listed = match &v[1] {
			Dat::List(l) => l,
			other => return Err(err!(
				"A Snapshot file list expects Dat::List, got {:?}.", other;
			Decode, Input, Mismatch)),
		};
		let mut files: Vec<FileState> = Vec::with_capacity(listed.len());
		for item in listed {
			let state = res!(FileState::from_dat(item));
			if let Some(last) = files.last() {
				if state.path <= last.path {
					return Err(err!(
						"A Snapshot lists the file {:?} after {:?}; files are encoded \
						ascending and without repetition.", state.path, last.path;
					Decode, Input, Order));
				}
			}
			files.push(state);
		}
		Ok(Self { frontier, files })
	}

	/// Appends the byte encoding of the snapshot to `buf`: the magic, the
	/// version, and the binary daticle form.
	pub fn encode_into(&self, buf: &mut Vec<u8>)
		-> Outcome<()>
	{
		buf.extend_from_slice(&MAGIC);
		buf.push(VERSION);
		let body = res!(self.to_dat().to_bytes(Vec::new()));
		buf.extend_from_slice(&body);
		Ok(())
	}

	/// Returns the byte encoding of the snapshot.
	pub fn encode(&self)
		-> Outcome<Vec<u8>>
	{
		let mut buf = Vec::new();
		res!(self.encode_into(&mut buf));
		Ok(buf)
	}

	/// Decodes a snapshot that must occupy the whole of `buf`.
	pub fn decode(buf: &[u8])
		-> Outcome<Self>
	{
		let at = MAGIC.len() + 1;
		if buf.len() < at {
			return Err(err!(
				"A snapshot of {} bytes is too short to carry even its header.",
				buf.len();
			Decode, Input, Missing));
		}
		if buf[..MAGIC.len()] != MAGIC {
			return Err(err!(
				"A snapshot begins {:02x?}, which is not the magic {:02x?}.",
				&buf[..MAGIC.len()], MAGIC;
			Decode, Input, Invalid));
		}
		let version = buf[MAGIC.len()];
		if version != VERSION {
			return Err(err!(
				"A snapshot declares format version {}, and this reader knows only \
				version {}.", version, VERSION;
			Decode, Input, Version, Mismatch));
		}
		let (dat, used) = res!(Dat::from_bytes(&buf[at..]));
		if used != buf.len() - at {
			return Err(err!(
				"A snapshot body of {} bytes decoded from only {} of them.",
				buf.len() - at, used;
			Decode, Input, Mismatch));
		}
		Self::from_dat(&dat)
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	use crate::id::{
		ContentRange,
		ReplicaId,
	};
	use crate::op::Header;
	use crate::seq::slot::Origin;
	use crate::seq::{
		Edit,
		Sequence,
	};

	/// An operation identifier.
	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// A rendered file, made the way a frontend would make one: seed a file, move
	/// a line, and edit inside the moved line concurrently.
	fn rendered()
		-> Outcome<(Rendered, Vec<OpId>)>
	{
		let seed_id = oid(1, 1);
		let mut seq = Sequence::new();
		res!(seq.apply(Header::root(seed_id), Edit::Splice {
			left:	None,
			right:	None,
			remove:	Vec::new(),
			insert:	b"- Eggs\n- Milk\n- Cheese\n".to_vec(),
		}));
		let view = res!(seq.render());
		let mv = res!(view.move_range(7, 7, 0));
		res!(seq.apply(res!(Header::new(oid(2, 2), vec![seed_id])), mv));
		let ed = res!(view.splice(9, 1, b"Soy m".to_vec()));
		res!(seq.apply(res!(Header::new(oid(3, 2), vec![seed_id])), ed));
		let out = res!(seq.render());
		Ok((out, vec![oid(2, 2), oid(3, 2)]))
	}

	/// A snapshot of a rendered file survives the round trip with its bytes, its
	/// provenance and its flags.
	#[test]
	fn a_rendered_file_round_trips() -> Outcome<()> {
		let (out, frontier) = res!(rendered());
		let snap = res!(Snapshot::new(
			frontier.clone(),
			vec![FileState::of(fmt!("shopping.txt"), &out)],
		));
		let back = res!(Snapshot::decode(&res!(snap.encode())));
		assert_eq!(back, snap);
		assert_eq!(back.frontier(), &frontier[..]);
		let file = match back.file("shopping.txt") {
			Some(f) => f,
			None => return Err(err!("The snapshot lost its only file."; Test, Missing)),
		};
		assert_eq!(file.bytes, out.bytes());
		assert_eq!(file.runs, out.runs());
		assert_eq!(file.flags, out.flags());
		assert_eq!(String::from_utf8_lossy(&file.bytes), "- Soy milk\n- Eggs\n- Cheese\n");
		Ok(())
	}

	/// Every kind of flag survives the round trip, including the ones no ordinary
	/// render happens to produce.
	#[test]
	fn every_flag_round_trips() -> Outcome<()> {
		let flags = vec![
			Flag::Torn {
				op:		oid(1, 4),
				lost:	vec![
					res!(ContentRange::new(oid(2, 1), 0, 10)),
					res!(ContentRange::new(oid(2, 1), 20, u64::MAX)),
				],
			},
			Flag::Torn { op: oid(1, 5), lost: Vec::new() },
			Flag::Demoted { op: oid(1, 6), sub: 0, origin: Origin::Left },
			Flag::Demoted { op: oid(1, 7), sub: u64::MAX, origin: Origin::Right },
			Flag::Dropped { op: oid(1, 8), sub: 3, origin: Origin::Left },
			Flag::Dropped { op: oid(1, 9), sub: 4, origin: Origin::Right },
			Flag::Overlap {
				ops:	vec![oid(1, 1), oid(2, 2), oid(3, 3)],
				region:	res!(ContentRange::new(oid(4, 4), 5, 9)),
			},
			Flag::Overlap {
				ops:	Vec::new(),
				region:	res!(ContentRange::new(oid(4, 4), 0, 0)),
			},
		];
		for flag in &flags {
			assert_eq!(*flag, res!(Flag::from_dat(&flag.to_dat())), "flag {}", flag.name());
		}
		let snap = res!(Snapshot::new(vec![oid(1, 1)], vec![FileState {
			path:	fmt!("f"),
			bytes:	Vec::new(),
			runs:	Vec::new(),
			flags:	flags.clone(),
		}]));
		let back = res!(Snapshot::decode(&res!(snap.encode())));
		assert_eq!(back.files()[0].flags, flags);
		Ok(())
	}

	/// A snapshot of several files keeps them apart and in order, whatever order
	/// they were given in.
	#[test]
	fn files_are_sorted_and_kept_apart() -> Outcome<()> {
		let files = vec![
			FileState { path: fmt!("z.txt"), bytes: b"zed".to_vec(), runs: Vec::new(), flags: Vec::new() },
			FileState { path: fmt!("a.txt"), bytes: b"aye".to_vec(), runs: Vec::new(), flags: Vec::new() },
			FileState { path: fmt!("m.txt"), bytes: b"em".to_vec(), runs: Vec::new(), flags: Vec::new() },
		];
		let snap = res!(Snapshot::new(vec![oid(9, 1), oid(1, 1), oid(9, 1)], files));
		assert_eq!(snap.frontier(), &[oid(1, 1), oid(9, 1)], "sorted, and a set");
		let paths: Vec<&str> = snap.files().iter().map(|f| f.path.as_str()).collect();
		assert_eq!(paths, vec!["a.txt", "m.txt", "z.txt"]);
		assert_eq!(snap.len(), 3);
		match snap.file("m.txt") {
			Some(f)	=> assert_eq!(f.bytes, b"em"),
			None	=> return Err(err!("A file went missing."; Test, Missing)),
		}
		assert!(snap.file("nothing.txt").is_none());
		assert_eq!(res!(Snapshot::decode(&res!(snap.encode()))), snap);
		Ok(())
	}

	/// One path may not name two states.
	#[test]
	fn a_path_names_one_file() -> Outcome<()> {
		let twice = vec![
			FileState { path: fmt!("f"), bytes: b"one".to_vec(), runs: Vec::new(), flags: Vec::new() },
			FileState { path: fmt!("f"), bytes: b"two".to_vec(), runs: Vec::new(), flags: Vec::new() },
		];
		assert!(Snapshot::new(Vec::new(), twice).is_err());
		Ok(())
	}

	/// An empty snapshot -- an empty repository at an empty frontier -- is legal
	/// and round trips.
	#[test]
	fn an_empty_snapshot_round_trips() -> Outcome<()> {
		let snap = res!(Snapshot::new(Vec::new(), Vec::new()));
		assert!(snap.is_empty());
		assert!(snap.frontier().is_empty());
		let back = res!(Snapshot::decode(&res!(snap.encode())));
		assert_eq!(back, snap);
		Ok(())
	}

	/// A file of more bytes than a single byte length field could express keeps
	/// all of them.
	#[test]
	fn a_long_file_keeps_its_length() -> Outcome<()> {
		for len in [255usize, 256, 70_000] {
			let snap = res!(Snapshot::new(Vec::new(), vec![FileState {
				path:	fmt!("big.bin"),
				bytes:	vec![0x5a; len],
				runs:	vec![Run {
					at:			0,
					content:	res!(ContentRange::new(oid(1, 1), 0, len as u64)),
				}],
				flags:	Vec::new(),
			}]));
			let back = res!(Snapshot::decode(&res!(snap.encode())));
			assert_eq!(back.files()[0].bytes.len(), len);
		}
		Ok(())
	}

	/// Truncating a snapshot anywhere is a typed error, never a panic and never a
	/// half-read state.
	#[test]
	fn truncation_at_every_offset_is_clean() -> Outcome<()> {
		let (out, frontier) = res!(rendered());
		let snap = res!(Snapshot::new(frontier, vec![FileState::of(fmt!("f"), &out)]));
		let bytes = res!(snap.encode());
		for cut in 0..bytes.len() {
			assert!(Snapshot::decode(&bytes[..cut]).is_err(), "cut at {}", cut);
		}
		assert_eq!(res!(Snapshot::decode(&bytes)), snap);
		Ok(())
	}

	/// Rubbish where a snapshot should be is refused, and an unknown version says
	/// so rather than being parsed.
	#[test]
	fn a_snapshot_that_is_not_one_is_refused() -> Outcome<()> {
		assert!(Snapshot::decode(b"").is_err());
		assert!(Snapshot::decode(b"not a snapshot at all").is_err());
		let (out, frontier) = res!(rendered());
		let snap = res!(Snapshot::new(frontier, vec![FileState::of(fmt!("f"), &out)]));
		let mut wrong = res!(snap.encode());
		wrong[MAGIC.len()] = VERSION + 1;
		let e = match Snapshot::decode(&wrong) {
			Ok(_) => return Err(err!("An unknown version was accepted."; Test)),
			Err(e) => e,
		};
		assert!(fmt!("{}", e).contains("version"), "message was {}", e);
		// Trailing rubbish after a whole snapshot is refused too.
		let mut extra = res!(snap.encode());
		extra.extend_from_slice(b"and more");
		assert!(Snapshot::decode(&extra).is_err());
		Ok(())
	}

	/// The decoder refuses the non-canonical orderings the constructor fixes.
	#[test]
	fn the_encoding_is_canonical() -> Outcome<()> {
		let unsorted = Dat::List(vec![
			Dat::List(vec![oid(9, 1).to_dat(), oid(1, 1).to_dat()]),
			Dat::List(vec![]),
		]);
		assert!(Snapshot::from_dat(&unsorted).is_err());
		let repeated = Dat::List(vec![
			Dat::List(vec![oid(1, 1).to_dat(), oid(1, 1).to_dat()]),
			Dat::List(vec![]),
		]);
		assert!(Snapshot::from_dat(&repeated).is_err());
		let a = FileState {
			path:	fmt!("a"),
			bytes:	Vec::new(),
			runs:	Vec::new(),
			flags:	Vec::new(),
		};
		let b = FileState { path: fmt!("b"), ..a.clone() };
		let backwards = Dat::List(vec![
			Dat::List(vec![]),
			Dat::List(vec![b.to_dat(), a.to_dat()]),
		]);
		assert!(Snapshot::from_dat(&backwards).is_err());
		Ok(())
	}

	/// A malformed part is refused and named.
	#[test]
	fn a_malformed_part_is_refused() -> Outcome<()> {
		assert!(Snapshot::from_dat(&Dat::U8(1)).is_err());
		assert!(Snapshot::from_dat(&Dat::List(vec![Dat::List(vec![])])).is_err());
		assert!(FileState::from_dat(&Dat::List(vec![Dat::Str(fmt!("f"))])).is_err());
		// A path that is not a string, and bytes that are not bytes.
		assert!(FileState::from_dat(&Dat::List(vec![
			Dat::U64(1),
			Dat::BU64(Vec::new()),
			Dat::List(vec![]),
			Dat::List(vec![]),
		])).is_err());
		assert!(FileState::from_dat(&Dat::List(vec![
			Dat::Str(fmt!("f")),
			Dat::Str(fmt!("not bytes")),
			Dat::List(vec![]),
			Dat::List(vec![]),
		])).is_err());
		// A flag at an unknown code, and an origin at an unknown one.
		assert!(Flag::from_dat(&Dat::List(vec![Dat::U8(99), oid(1, 1).to_dat()])).is_err());
		assert!(Flag::from_dat(&Dat::List(vec![
			Dat::U8(2),
			oid(1, 1).to_dat(),
			Dat::U64(0),
			Dat::U8(7),
		])).is_err());
		assert!(Run::from_dat(&Dat::U64(1)).is_err());
		Ok(())
	}

	/// Randomised snapshots, of any number of files carrying any mixture of runs
	/// and flags, come back exactly as they went in.
	#[test]
	fn random_snapshots_round_trip() -> Outcome<()> {
		// A small linear congruential generator, so a failure can be reproduced.
		let mut state = 0x0f1e_2d3c_4b5a_6978u64;
		let mut next = move || {
			state = state
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			(state >> 33) as usize
		};
		for trial in 0..50 {
			let mut frontier: Vec<OpId> = Vec::new();
			for _ in 0..next() % 6 {
				frontier.push(oid((next() % 9) as u64, (next() % 900) as u64));
			}
			let mut files: Vec<FileState> = Vec::new();
			let mut paths: Vec<String> = Vec::new();
			for _ in 0..next() % 5 {
				let path = fmt!("f{}", next() % 20);
				if paths.contains(&path) {
					continue;
				}
				paths.push(path.clone());
				let mut runs: Vec<Run> = Vec::new();
				let mut at = 0u64;
				for _ in 0..next() % 6 {
					let len = (next() % 40) as u64;
					runs.push(Run {
						at,
						content: res!(ContentRange::new(
							oid((next() % 5) as u64, (next() % 50) as u64),
							0,
							len,
						)),
					});
					at += len;
				}
				let mut flags: Vec<Flag> = Vec::new();
				for _ in 0..next() % 4 {
					let op = oid((next() % 5) as u64, (next() % 50) as u64);
					flags.push(match next() % 4 {
						0 => Flag::Torn {
							op,
							lost: vec![res!(ContentRange::new(op, 0, (next() % 30) as u64))],
						},
						1 => Flag::Demoted {
							op,
							sub:	(next() % 100) as u64,
							origin:	if next() % 2 == 0 { Origin::Left } else { Origin::Right },
						},
						2 => Flag::Dropped {
							op,
							sub:	(next() % 100) as u64,
							origin:	if next() % 2 == 0 { Origin::Left } else { Origin::Right },
						},
						_ => Flag::Overlap {
							ops:	vec![op, oid((next() % 5) as u64, (next() % 50) as u64)],
							region:	res!(ContentRange::new(op, 1, (next() % 30) as u64 + 1)),
						},
					});
				}
				files.push(FileState {
					path,
					bytes: (0..at).map(|i| (i % 251) as u8).collect(),
					runs,
					flags,
				});
			}
			let snap = res!(Snapshot::new(frontier, files));
			let bytes = res!(snap.encode());
			assert_eq!(res!(Snapshot::decode(&bytes)), snap, "trial {}", trial);
			// And every truncation of it is an error rather than a panic.
			for cut in 0..bytes.len() {
				assert!(Snapshot::decode(&bytes[..cut]).is_err(),
					"trial {} cut at {}", trial, cut);
			}
		}
		Ok(())
	}

	/// The bytes of a small snapshot, frozen.
	///
	/// The same discipline as the segment's golden test: a format that changes by
	/// accident orphans every store already written in it, and every other test
	/// here would agree with itself while it happened.
	#[test]
	fn the_snapshot_bytes_are_frozen() -> Outcome<()> {
		let snap = res!(Snapshot::new(vec![oid(1, 2)], vec![FileState {
			path:	fmt!("a"),
			bytes:	b"hi".to_vec(),
			runs:	vec![Run {
				at:			0,
				content:	res!(ContentRange::new(oid(1, 2), 0, 2)),
			}],
			flags:	Vec::new(),
		}]));
		let want: &[u8] = &[
			// The magic and the version.
			0x4f, 0x52, 0x45, 0x53, 0x4e, 0x50,
			0x01,
			// A daticle list of 104 bytes: the frontier, then the files.
			0x33, 0x21, 0x68,
				// The frontier, 21 bytes: one identifier, r1:2.
				0x33, 0x21, 0x15,
					0x33, 0x21, 0x12,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
				// The files, 77 bytes: one of them.
				0x33, 0x21, 0x4d,
					// The file, 74 bytes.
					0x33, 0x21, 0x4a,
						// The path "a".
						0x29, 0x21, 0x01, 0x61,
						// The bytes "hi", under a 64-bit length.
						0x47, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
						0x68, 0x69,
						// The runs, 54 bytes: one of them, 51 bytes.
						0x33, 0x21, 0x36,
							0x33, 0x21, 0x33,
								// At rendered offset zero.
								0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
								// Showing the content r1:2+0..2.
								0x33, 0x21, 0x27,
									0x33, 0x21, 0x12,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
										0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
									0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
									0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
						// No flags, which is a list of no bytes.
						0x33, 0x20,
		];
		assert_eq!(res!(snap.encode()), want, "the snapshot format has changed");
		assert_eq!(res!(Snapshot::decode(want)), snap);
		Ok(())
	}
}
