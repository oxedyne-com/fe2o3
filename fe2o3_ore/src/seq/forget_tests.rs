//! Forgetting: an operation loses its content and keeps its shape, and every
//! replica renders the same whether it still holds the bytes or only the stub.
//!
//! The property under test is the one the whole design rests on -- that the
//! render is a function of the operation set, forgets included, and of nothing
//! else. So every case is run under every delivery order, and the case that
//! matters most puts an original on one replica and a repack's stub on another
//! and asks for byte equality.

use crate::id::{
	OpId,
	ReplicaId,
};
use crate::op::{
	Header,
	Op,
	Placing,
	Stub,
};
use crate::seq::tests::{
	case,
	converge,
	seed,
	Replica,
};
use crate::seq::Sequence;

use oxedyne_fe2o3_core::prelude::*;


/// A forget of an operation, as a command would author it: the shape read off
/// the operation itself.
fn forget_of(rep: &mut Replica, targets: &[(OpId, &Op)])
	-> Outcome<(Header, Op)>
{
	let mut of: Vec<Stub> = Vec::new();
	for (id, op) in targets {
		let placing = res!(op.stub_of().ok_or_else(|| err!(
			"{} has nothing to forget", op.name(); Test)));
		of.push(Stub { id: *id, placing });
	}
	of.sort_by_key(|s| s.id);
	rep.author(Op::Forget { of, reason: b"a test".to_vec(), time: 1_755_000_000 })
}

/// A forgotten insertion's bytes are gone from every state, and an edit made
/// inside it afterwards still renders where it was made.
#[test]
fn a_forgotten_insertion_is_buried_and_what_was_anchored_in_it_survives() -> Outcome<()> {
	let mut st = res!(seed(b"one two three\n", 1));
	let rep = &mut st.reps[0];
	// The secret goes in, and a word is then written inside it.
	let secret = res!(rep.insert(4, b"SECRET "));
	let inside = res!(rep.insert(4 + 3, b"[x]"));
	assert_eq!(res!(rep.view()).text_lossy(), "one SEC[x]RET two three\n");
	let forget = res!(forget_of(rep, &[(secret.0.id(), &secret.1)]));
	let mut ops = st.ops.clone();
	ops.extend([secret, inside, forget]);
	// The secret's bytes are gone; the word written inside it stands where the
	// secret stood, since its anchors still name the buried bytes.
	let got = res!(case(st.file, "one [x]two three\n", &ops));
	assert!(!got.bytes().windows(6).any(|w| w == b"SECRET"), "the bytes are gone");
	Ok(())
}

/// A replica holding the original beside the forget and one holding only the
/// stub a repack wrote render byte for byte the same, in every delivery order.
#[test]
fn a_stub_and_an_original_render_the_same() -> Outcome<()> {
	let mut st = res!(seed(b"alpha beta\n", 1));
	let rep = &mut st.reps[0];
	let secret = res!(rep.insert(6, b"KEY=abc "));
	let after = res!(rep.insert(6 + 8, b"gamma "));
	let forget = res!(forget_of(rep, &[(secret.0.id(), &secret.1)]));
	let placing = res!(secret.1.stub_of().ok_or_else(|| err!("no shape"; Test)));

	let mut with_original = st.ops.clone();
	with_original.extend([secret.clone(), after.clone(), forget.clone()]);
	let mut with_stub = st.ops.clone();
	with_stub.extend([
		(secret.0.clone(), Op::Forgotten { placing }),
		after,
		forget,
	]);
	let a = res!(case(st.file, "alpha gamma beta\n", &with_original));
	let b = res!(case(st.file, "alpha gamma beta\n", &with_stub));
	assert_eq!(a.bytes(), b.bytes());
	assert_eq!(a.runs(), b.runs(), "and the same provenance runs");
	Ok(())
}

/// A forgotten file has no path and is not live, its content is gone, and an
/// edit into it afterwards is complete rather than an error.
#[test]
fn a_forgotten_file_is_dead_from_birth() -> Outcome<()> {
	let mut st = res!(seed(b"public\n", 1));
	let rep = &mut st.reps[0];
	let create = res!(rep.author(Op::FileCreate { path: b"keys/prod.pem".to_vec() }));
	let key = create.0.id();
	let fill = {
		let repo = res!(rep.seq.render());
		let f = res!(repo.file(key).ok_or_else(|| err!("no file"; Test)));
		let op = res!(f.splice(0, 0, b"-----BEGIN".to_vec()));
		res!(rep.author(op))
	};
	let forget = res!(forget_of(rep, &[(key, &create.1), (fill.0.id(), &fill.1)]));
	let late = {
		let repo = res!(rep.seq.render());
		let f = res!(repo.file(key).ok_or_else(|| err!("no file after forget"; Test)));
		let op = res!(f.splice(0, 0, b"late".to_vec()));
		res!(rep.author(op))
	};
	let mut ops = st.ops.clone();
	ops.extend([create, fill, forget, late]);
	let repo = res!(converge(&ops));
	let f = res!(repo.file(key).ok_or_else(|| err!("the file is still a file"; Test)));
	assert!(!f.is_live(), "and not a live one");
	assert!(f.path().is_empty(), "with no path to be laid out under");
	assert!(!f.bytes().windows(5).any(|w| w == b"BEGIN"), "its content gone");
	let public = res!(repo.file(st.file).ok_or_else(|| err!("no public file"; Test)));
	assert_eq!(public.text_lossy(), "public\n", "the other file untouched");
	Ok(())
}

/// A forget arriving before the operation it names holds the shape ready, and
/// the original arriving later is held in that shape and not as itself.
#[test]
fn a_forget_arriving_first_still_shapes_what_comes_after() -> Outcome<()> {
	let mut st = res!(seed(b"ab\n", 1));
	let rep = &mut st.reps[0];
	let secret = res!(rep.insert(1, b"XX"));
	let forget = res!(forget_of(rep, &[(secret.0.id(), &secret.1)]));
	let mut seq = Sequence::new();
	for (h, o) in &st.ops {
		res!(seq.apply(h.clone(), o.clone()));
	}
	res!(seq.apply(forget.0.clone(), forget.1.clone()));
	res!(seq.apply(secret.0.clone(), secret.1.clone()));
	match seq.get(&secret.0.id()) {
		Some(Op::Forgotten { .. })	=> (),
		other						=> return Err(err!(
			"the original is held as {:?}, not as its stub", other.map(|o| o.name()); Test)),
	}
	let repo = res!(seq.render());
	let f = res!(repo.file(st.file).ok_or_else(|| err!("no file"; Test)));
	assert_eq!(f.text_lossy(), "ab\n");
	Ok(())
}

/// Two forgets disagreeing about the shape one operation keeps are refused,
/// because a forgotten operation keeps one shape everywhere.
#[test]
fn two_forgets_disagreeing_on_a_shape_are_refused() -> Outcome<()> {
	let mut st = res!(seed(b"ab\n", 2));
	let secret = res!(st.reps[0].insert(1, b"XX"));
	res!(st.reps[1].recv(secret.clone()));
	let honest = res!(forget_of(&mut st.reps[0], &[(secret.0.id(), &secret.1)]));
	// The second replica claims a different shape for the same operation.
	let dishonest = res!(st.reps[1].author(Op::Forget {
		of:		vec![Stub { id: secret.0.id(), placing: Placing::Void }],
		reason:	Vec::new(),
		time:	0,
	}));
	let mut seq = Sequence::new();
	for (h, o) in st.ops.iter().chain([&secret, &honest]) {
		res!(seq.apply(h.clone(), o.clone()));
	}
	assert!(seq.apply(dishonest.0.clone(), dishonest.1.clone()).is_err());
	// And the other way about, through absorb.
	let mut other = Sequence::new();
	for (h, o) in st.ops.iter().chain([&secret, &dishonest]) {
		res!(other.apply(h.clone(), o.clone()));
	}
	assert!(seq.absorb(&other).is_err());
	Ok(())
}

/// What each kind of operation keeps when forgotten, and which have nothing to
/// forget.
#[test]
fn each_kind_keeps_the_shape_the_rule_says() -> Outcome<()> {
	let r = ReplicaId::new(1);
	let file = OpId::new(r, 1);
	for op in crate::op::tests::samples() {
		let kept = op.stub_of();
		match &op {
			Op::FileCreate { .. }	=> assert_eq!(kept, Some(Placing::File), "{}", op.name()),
			Op::Splice { left, right, remove, insert } => assert_eq!(kept, Some(Placing::Splice {
				left: left.clone(), right: right.clone(), remove: remove.clone(),
				len: insert.len() as u64,
			}), "{}", op.name()),
			Op::FileRename { .. } | Op::Mark { .. } | Op::Note { .. } | Op::Proposal { .. }
			| Op::Said { .. } | Op::Amended { .. }
									=> assert_eq!(kept, Some(Placing::Void), "{}", op.name()),
			_						=> assert_eq!(kept, None, "{} has nothing to forget", op.name()),
		}
	}
	let _ = file;
	Ok(())
}
