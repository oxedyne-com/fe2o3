//! A rendered state at a frontier, as bytes.
//!
//! A snapshot is what the repository looked like when the operations named by a
//! frontier had been applied and no others. It exists so that a reader does not
//! have to replay a history from its first operation to see the present: it
//! takes the snapshot, and then only the segments holding operations the
//! frontier does not already cover.
//!
//! What is stored is a render and not a state machine. Per file: its identity,
//! its path, the bytes, the provenance of those bytes, and what the renderer
//! noticed. The provenance is there because without it the bytes are dumb -- a
//! frontend that means to author a content-anchored splice against what it is
//! showing needs to know what the byte under the cursor is called, and that is
//! exactly what a run says.
//!
//! # A file is named by identity
//!
//! A snapshot keys its files by the identity of the operation that created each,
//! not by path. A path is metadata a rename may change, and two live files may
//! share one -- two branches that independently created the same path minted two
//! files, both of which exist -- so a snapshot keyed by path could not encode a
//! state the repository can genuinely be in. The path is carried as bytes,
//! because a path is not required to be UTF-8.
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
///
/// Version 1 keyed a file by its path and spelled that path as a string. Version
/// 2 keys by identity and spells the path as bytes; nothing was ever written in
/// version 1 that needs to be read again, so the old form is gone rather than
/// carried.
pub const VERSION: u8 = 2;


/// One file, as it stood.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileState {
	/// The file's identity, which is the identity of the operation that created
	/// it and is what a snapshot keys by.
	pub file:	OpId,
	/// Where the file sat, as bytes: metadata, and not a name for it.
	pub path:	Vec<u8>,
	/// The rendered bytes.
	pub bytes:	Vec<u8>,
	/// What those bytes are made of, in render order and coalesced.
	pub runs:	Vec<Run>,
	/// What the renderer noticed while producing them.
	pub flags:	Vec<Flag>,
}

impl FileState {
	/// Takes one file's render as its state.
	pub fn of(rendered: &Rendered) -> Self {
		Self {
			file:	rendered.file(),
			path:	rendered.path().to_vec(),
			bytes:	rendered.bytes().to_vec(),
			runs:	rendered.runs().to_vec(),
			flags:	rendered.flags().to_vec(),
		}
	}

	/// Returns the path as a string, with anything that is not valid UTF-8
	/// replaced. For messages; the bytes themselves are the record.
	pub fn path_lossy(&self) -> String {
		String::from_utf8_lossy(&self.path).into_owned()
	}

	/// Serialises the state to a [`Dat`]. The shape is
	/// `[file, path, bytes, [run, ...], [flag, ...]]`.
	///
	/// Both the path and the bytes are a [`Dat::BU64`]: a file is routinely
	/// longer than the 255 bytes a [`Dat::BU8`] length field can express, and a
	/// truncated length there would corrupt silently.
	pub fn to_dat(&self) -> Dat {
		Dat::List(vec![
			self.file.to_dat(),
			Dat::BU64(self.path.clone()),
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
			Dat::List(v) if v.len() == 5 => v,
			_ => return Err(err!(
				"A FileState expects a 5-element Dat::List, got {:?}.", dat;
			Decode, Input, Mismatch)),
		};
		let file = res!(OpId::from_dat(&v[0]));
		let path = match &v[1] {
			Dat::BU64(b) => b.clone(),
			other => return Err(err!(
				"The path of the file {} expects Dat::BU64, got {:?}.", file, other;
			Decode, Input, Mismatch)),
		};
		let bytes = match &v[2] {
			Dat::BU64(b) => b.clone(),
			other => return Err(err!(
				"The rendered bytes of the file {} expect Dat::BU64, got {:?}.",
				file, other;
			Decode, Input, Mismatch)),
		};
		let listed = match &v[3] {
			Dat::List(l) => l,
			other => return Err(err!(
				"The runs of the file {} expect Dat::List, got {:?}.", file, other;
			Decode, Input, Mismatch)),
		};
		let mut runs = Vec::with_capacity(listed.len());
		for item in listed {
			runs.push(res!(Run::from_dat(item)));
		}
		let listed = match &v[4] {
			Dat::List(l) => l,
			other => return Err(err!(
				"The flags of the file {} expect Dat::List, got {:?}.", file, other;
			Decode, Input, Mismatch)),
		};
		let mut flags = Vec::with_capacity(listed.len());
		for item in listed {
			flags.push(res!(Flag::from_dat(item)));
		}
		Ok(Self { file, path, bytes, runs, flags })
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
	/// The files, in ascending order of identity.
	files:		Vec<FileState>,
}

impl Snapshot {

	/// Constructs a snapshot, sorting both lists.
	///
	/// Fails where two states name one file, since which of them the file was is
	/// then unanswerable. Two states at one *path* are legal and no longer a
	/// contradiction: they are two files, and that is a state the repository can
	/// genuinely be in. Repeated frontier entries are dropped, a frontier being a
	/// set.
	pub fn new(frontier: Vec<OpId>, files: Vec<FileState>)
		-> Outcome<Self>
	{
		let mut frontier = frontier;
		frontier.sort();
		frontier.dedup();
		let mut files = files;
		files.sort_by(|a, b| a.file.cmp(&b.file));
		for pair in files.windows(2) {
			if pair[0].file == pair[1].file {
				return Err(err!(
					"A snapshot names the file {} twice.", pair[0].file;
				Invalid, Input, Duplicate));
			}
		}
		Ok(Self { frontier, files })
	}

	/// Returns the frontier the state was rendered at.
	pub fn frontier(&self) -> &[OpId] {
		&self.frontier
	}

	/// Returns the files, in ascending order of identity.
	pub fn files(&self) -> &[FileState] {
		&self.files
	}

	/// Returns the state of one file, if the snapshot holds it.
	pub fn file(&self, file: OpId)
		-> Option<&FileState>
	{
		self.files
			.binary_search_by(|f| f.file.cmp(&file))
			.ok()
			.and_then(|i| self.files.get(i))
	}

	/// Returns the states at a path, in ascending order of identity.
	///
	/// More than one is legal, and is what two branches creating one path leaves
	/// behind; which of them a working copy writes under that name is a policy the
	/// caller owns.
	pub fn at_path(&self, path: &[u8]) -> Vec<&FileState> {
		self.files.iter().filter(|f| f.path == path).collect()
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
				if state.file <= last.file {
					return Err(err!(
						"A Snapshot lists the file {} after {}; files are encoded \
						ascending by identity and without repetition.",
						state.file, last.file;
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
		Anchor,
		ContentRange,
		ReplicaId,
	};
	use crate::op::{
		Header,
		Op,
	};
	use crate::seq::render::Repo;
	use crate::seq::slot::Origin;
	use crate::seq::Sequence;

	/// An operation identifier.
	fn oid(replica: u64, counter: u64) -> OpId {
		OpId::new(ReplicaId::new(replica), counter)
	}

	/// A file state with nothing in it but an identity and a path.
	fn bare(file: OpId, path: &[u8]) -> FileState {
		FileState {
			file,
			path:	path.to_vec(),
			bytes:	Vec::new(),
			runs:	Vec::new(),
			flags:	Vec::new(),
		}
	}

	/// A rendered repository, made the way a frontend would make one: create a
	/// file, seed it, move a line, and edit inside the moved line concurrently.
	fn rendered()
		-> Outcome<(Repo, OpId, Vec<OpId>)>
	{
		let create = oid(1, 1);
		let seed_id = oid(1, 2);
		let mut seq = Sequence::new();
		res!(seq.apply(Header::root(create), Op::FileCreate {
			path: b"shopping.txt".to_vec(),
		}));
		res!(seq.apply(res!(Header::new(seed_id, vec![create])), Op::Splice {
			left:	Some(Anchor::origin(create)),
			right:	None,
			remove:	Vec::new(),
			insert:	b"- Eggs\n- Milk\n- Cheese\n".to_vec(),
		}));
		let repo = res!(seq.render());
		let view = match repo.file(create) {
			Some(f) => f.clone(),
			None => return Err(err!("The file went missing."; Test, Missing)),
		};
		let mv = res!(view.move_range(7, 7, 0));
		res!(seq.apply(res!(Header::new(oid(2, 3), vec![seed_id])), mv));
		let ed = res!(view.splice(9, 1, b"Soy m".to_vec()));
		res!(seq.apply(res!(Header::new(oid(3, 3), vec![seed_id])), ed));
		let out = res!(seq.render());
		Ok((out, create, vec![oid(2, 3), oid(3, 3)]))
	}

	/// A snapshot of a rendered file survives the round trip with its identity,
	/// its path, its bytes, its provenance and its flags.
	#[test]
	fn a_rendered_file_round_trips() -> Outcome<()> {
		let (repo, file, frontier) = res!(rendered());
		let state = match repo.file(file) {
			Some(f) => FileState::of(f),
			None => return Err(err!("The render lost its only file."; Test, Missing)),
		};
		let snap = res!(Snapshot::new(frontier.clone(), vec![state]));
		let back = res!(Snapshot::decode(&res!(snap.encode())));
		assert_eq!(back, snap);
		assert_eq!(back.frontier(), &frontier[..]);
		let got = match back.file(file) {
			Some(f) => f,
			None => return Err(err!("The snapshot lost its only file."; Test, Missing)),
		};
		assert_eq!(got.path, b"shopping.txt");
		assert_eq!(String::from_utf8_lossy(&got.bytes), "- Soy milk\n- Eggs\n- Cheese\n");
		assert_eq!(got.path_lossy(), "shopping.txt");
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
			Flag::CrossedFile {
				op:		oid(5, 1),
				sub:	7,
				origin:	Origin::Left,
				from:	oid(1, 1),
				to:		oid(2, 1),
			},
			Flag::MovedIntoDeleted { op: oid(5, 2), file: oid(2, 1) },
			Flag::Orphaned { op: oid(5, 3), sub: 11 },
		];
		for flag in &flags {
			assert_eq!(*flag, res!(Flag::from_dat(&flag.to_dat())), "flag {}", flag.name());
		}
		let snap = res!(Snapshot::new(vec![oid(1, 1)], vec![FileState {
			flags: flags.clone(),
			..bare(oid(1, 1), b"f")
		}]));
		let back = res!(Snapshot::decode(&res!(snap.encode())));
		assert_eq!(back.files()[0].flags, flags);
		Ok(())
	}

	/// A snapshot of several files keeps them apart and in order of identity,
	/// whatever order they were given in.
	#[test]
	fn files_are_sorted_by_identity_and_kept_apart() -> Outcome<()> {
		let files = vec![
			FileState { bytes: b"zed".to_vec(), ..bare(oid(9, 1), b"z.txt") },
			FileState { bytes: b"aye".to_vec(), ..bare(oid(1, 1), b"a.txt") },
			FileState { bytes: b"em".to_vec(), ..bare(oid(2, 4), b"m.txt") },
		];
		let snap = res!(Snapshot::new(vec![oid(9, 1), oid(1, 1), oid(9, 1)], files));
		assert_eq!(snap.frontier(), &[oid(1, 1), oid(9, 1)], "sorted, and a set");
		let ids: Vec<OpId> = snap.files().iter().map(|f| f.file).collect();
		assert_eq!(ids, vec![oid(1, 1), oid(2, 4), oid(9, 1)]);
		assert_eq!(snap.len(), 3);
		match snap.file(oid(2, 4)) {
			Some(f)	=> assert_eq!(f.bytes, b"em"),
			None	=> return Err(err!("A file went missing."; Test, Missing)),
		}
		assert!(snap.file(oid(7, 7)).is_none());
		assert_eq!(res!(Snapshot::decode(&res!(snap.encode()))), snap);
		Ok(())
	}

	/// One identity may not name two states, and one path may.
	#[test]
	fn identity_is_unique_and_a_path_is_not() -> Outcome<()> {
		let twice = vec![
			FileState { bytes: b"one".to_vec(), ..bare(oid(1, 1), b"f") },
			FileState { bytes: b"two".to_vec(), ..bare(oid(1, 1), b"g") },
		];
		assert!(Snapshot::new(Vec::new(), twice).is_err());
		// Two files at one path are two files, which is a state the repository
		// can genuinely be in, so the snapshot encodes it rather than refusing it.
		let shared = vec![
			FileState { bytes: b"one".to_vec(), ..bare(oid(1, 1), b"notes.md") },
			FileState { bytes: b"two".to_vec(), ..bare(oid(2, 1), b"notes.md") },
		];
		let snap = res!(Snapshot::new(Vec::new(), shared));
		assert_eq!(snap.at_path(b"notes.md").len(), 2);
		assert_eq!(res!(Snapshot::decode(&res!(snap.encode()))), snap);
		Ok(())
	}

	/// A path that is not UTF-8 survives, which the string-keyed form could not
	/// even express.
	#[test]
	fn a_path_that_is_not_utf8_survives() -> Outcome<()> {
		let path: Vec<u8> = vec![0xff, 0xfe, b'/', 0x80, b'x'];
		let snap = res!(Snapshot::new(Vec::new(), vec![bare(oid(1, 1), &path)]));
		let back = res!(Snapshot::decode(&res!(snap.encode())));
		assert_eq!(back.files()[0].path, path);
		assert_eq!(back.at_path(&path).len(), 1);
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
	/// all of them, and so does a path.
	#[test]
	fn a_long_file_keeps_its_length() -> Outcome<()> {
		for len in [255usize, 256, 70_000] {
			let snap = res!(Snapshot::new(Vec::new(), vec![FileState {
				file:	oid(1, 1),
				path:	vec![b'p'; len],
				bytes:	vec![0x5a; len],
				runs:	vec![Run {
					at:			0,
					content:	res!(ContentRange::new(oid(1, 1), 0, len as u64)),
				}],
				flags:	Vec::new(),
			}]));
			let back = res!(Snapshot::decode(&res!(snap.encode())));
			assert_eq!(back.files()[0].bytes.len(), len);
			assert_eq!(back.files()[0].path.len(), len);
		}
		Ok(())
	}

	/// Truncating a snapshot anywhere is a typed error, never a panic and never a
	/// half-read state.
	#[test]
	fn truncation_at_every_offset_is_clean() -> Outcome<()> {
		let (repo, file, frontier) = res!(rendered());
		let state = match repo.file(file) {
			Some(f) => FileState::of(f),
			None => return Err(err!("The render lost its only file."; Test, Missing)),
		};
		let snap = res!(Snapshot::new(frontier, vec![state]));
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
		let snap = res!(Snapshot::new(Vec::new(), vec![bare(oid(1, 1), b"f")]));
		let mut wrong = res!(snap.encode());
		wrong[MAGIC.len()] = VERSION + 1;
		let e = match Snapshot::decode(&wrong) {
			Ok(_) => return Err(err!("An unknown version was accepted."; Test)),
			Err(e) => e,
		};
		assert!(fmt!("{}", e).contains("version"), "message was {}", e);
		// The version this reader knows is not the one the old form used, so a
		// snapshot written before file identity is refused rather than misread.
		let mut old = res!(snap.encode());
		old[MAGIC.len()] = 1;
		assert!(Snapshot::decode(&old).is_err());
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
		let a = bare(oid(1, 1), b"a");
		let b = bare(oid(2, 1), b"b");
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
		// A file named by something that is not an identifier.
		assert!(FileState::from_dat(&Dat::List(vec![
			Dat::U64(1),
			Dat::BU64(Vec::new()),
			Dat::BU64(Vec::new()),
			Dat::List(vec![]),
			Dat::List(vec![]),
		])).is_err());
		// A path that is a string rather than bytes, which is what the old form
		// spelled.
		assert!(FileState::from_dat(&Dat::List(vec![
			oid(1, 1).to_dat(),
			Dat::Str(fmt!("f")),
			Dat::BU64(Vec::new()),
			Dat::List(vec![]),
			Dat::List(vec![]),
		])).is_err());
		// And bytes that are not bytes.
		assert!(FileState::from_dat(&Dat::List(vec![
			oid(1, 1).to_dat(),
			Dat::BU64(Vec::new()),
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
			let mut ids: Vec<OpId> = Vec::new();
			for _ in 0..next() % 5 {
				let file = oid((next() % 9) as u64, (next() % 20) as u64 + 1);
				if ids.contains(&file) {
					continue;
				}
				ids.push(file);
				// Two files may share a path, so the path is drawn freely.
				let path: Vec<u8> = fmt!("f{}", next() % 4).into_bytes();
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
					flags.push(match next() % 7 {
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
						3 => Flag::CrossedFile {
							op,
							sub:	(next() % 100) as u64,
							origin:	if next() % 2 == 0 { Origin::Left } else { Origin::Right },
							from:	oid((next() % 5) as u64, (next() % 50) as u64),
							to:		oid((next() % 5) as u64, (next() % 50) as u64),
						},
						4 => Flag::MovedIntoDeleted {
							op,
							file: oid((next() % 5) as u64, (next() % 50) as u64),
						},
						5 => Flag::Orphaned { op, sub: (next() % 100) as u64 },
						_ => Flag::Overlap {
							ops:	vec![op, oid((next() % 5) as u64, (next() % 50) as u64)],
							region:	res!(ContentRange::new(op, 1, (next() % 30) as u64 + 1)),
						},
					});
				}
				files.push(FileState {
					file,
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
	/// here would agree with itself while it happened. The values below were
	/// rewritten by hand when the format changed for file identity, each line
	/// annotated with the field it spells.
	#[test]
	fn the_snapshot_bytes_are_frozen() -> Outcome<()> {
		let snap = res!(Snapshot::new(vec![oid(1, 2)], vec![FileState {
			file:	oid(1, 2),
			path:	b"a".to_vec(),
			bytes:	b"hi".to_vec(),
			runs:	vec![Run {
				at:			0,
				content:	res!(ContentRange::new(oid(1, 2), 0, 2)),
			}],
			flags:	Vec::new(),
		}]));
		let want: &[u8] = &[
			// The magic and the version, which is 2 since file identity.
			0x4f, 0x52, 0x45, 0x53, 0x4e, 0x50,
			0x02,
			// A daticle list of 131 bytes: the frontier (24), then the files (107).
			0x33, 0x21, 0x83,
				// The frontier, 21 bytes: one identifier, r1:2.
				0x33, 0x21, 0x15,
					0x33, 0x21, 0x12,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
						0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
				// The files, 104 bytes: one of them.
				0x33, 0x21, 0x68,
					// The file, 101 bytes: 21 of identity, 10 of path, 11 of
					// bytes, 57 of runs and 2 of flags.
					0x33, 0x21, 0x65,
						// Its identity, r1:2, which is what a snapshot keys by.
						0x33, 0x21, 0x12,
							0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
							0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
						// The path "a", as bytes under a 64-bit length.
						0x47, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
						0x61,
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
