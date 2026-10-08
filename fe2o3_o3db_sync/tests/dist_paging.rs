#![cfg(feature = "dist")]
//! Paged anti-entropy over many pages (A2 QA fix round, A1-F1).
//!
//! Every envelope here goes through `Envelope::encode` and `decode`, as a real transport runs
//! it, and must stay under the codec's bound. The tables are tens of 1 MiB records, so a reply or
//! a push is cut at 12 MiB and an exchange moves eleven. A bulk reply carries a cursor, `next`,
//! that the dialler sends back as `after`; a decoded difference and the dialler's push carry it
//! too, so a record a peer keeps refusing cannot hold up the records behind it.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
	dist::{
		codec::MAX_ENVELOPE_BYTES,
		config::{
			Consistency,
			DistOzoneConfig,
			TableConfig,
		},
		engine::DistOzone,
		record::{
			Record,
			RecordId,
		},
		resolve::{
			ReadView,
			ResolveCtx,
			Resolver,
			Verdict,
		},
		storage::{
			MemoryStorage,
			Storage,
		},
		transport::{
			Envelope,
			MsgKind,
		},
	},
	kademlia::id::NodeId,
	oam::config::OamConfig,
};

use std::{
	collections::{
		BTreeSet,
		HashMap,
	},
	time::Duration,
};


const MIB: usize = 1024 * 1024;

fn node(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}

// Ids sort by their index.
fn rid(i: u16) -> RecordId {
	let mut bytes = [0u8; 32];
	bytes[0] = (i >> 8) as u8;
	bytes[1] = i as u8;
	RecordId::from_bytes(bytes)
}

fn idx(id: &RecordId) -> u16 {
	let b = id.as_bytes();
	((b[0] as u16) << 8) | b[1] as u16
}

fn rec(i: u16, len: usize) -> Record {
	Record::new(rid(i), "identity", vec![(i % 251) as u8 + 1; len])
}

// Takes whatever arrives, except an id below the line, which it refuses.
struct Rule {
	below: u16,
}

impl Resolver for Rule {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		id:			&RecordId,
		_held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		if idx(id) < self.below {
			return Ok(Verdict::Refuse("below the line".to_string()));
		}
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

type Peer = DistOzone<MemoryStorage, Rule>;

fn cfg(local: u8, remote: u8, cells: usize) -> Outcome<DistOzoneConfig> {
	let oam = res!(OamConfig::new(2, 2));
	let table = res!(TableConfig::new("identity", Consistency::Eventual, Duration::from_secs(30), cells));
	DistOzoneConfig::new(node(local), vec![node(remote)], oam, vec![table])
}

// Peer 1 (the dialler) and peer 2 (the listener), on a table whose sketch has `cells` cells. Each
// refuses the ids below its own line.
fn pair(cells: usize, d_below: u16, l_below: u16) -> Outcome<(Peer, Peer)> {
	Ok((
		res!(DistOzone::with_resolver(res!(cfg(1, 2, cells)), MemoryStorage::new(), Rule { below: d_below })),
		res!(DistOzone::with_resolver(res!(cfg(2, 1, cells)), MemoryStorage::new(), Rule { below: l_below })),
	))
}

fn held(e: &Peer) -> Outcome<BTreeSet<u16>> {
	Ok(res!(e.storage().digests("identity")).iter().map(|d| idx(&d.id)).collect())
}

fn same_bytes(a: &Peer, b: &Peer) -> Outcome<bool> {
	let mut x: Vec<_> = res!(a.storage().digests("identity")).into_iter().map(|d| (d.id, d.content)).collect();
	let mut y: Vec<_> = res!(b.storage().digests("identity")).into_iter().map(|d| (d.id, d.content)).collect();
	x.sort();
	y.sort();
	Ok(x == y)
}

// What one exchange carried.
struct Leg {
	after:		Option<RecordId>,	// the digest's cursor
	next:		Option<RecordId>,	// where the reply stopped
	bulk:		bool,
	replied:	Vec<u16>,			// the records in the reply
	pushed:		Vec<u16>,			// the records in the dialler's push
	refused:	Vec<u16>,			// the records either side refused
	biggest:	usize,				// the longest envelope written
}

// An envelope as the wire carries it, held to the codec's bound.
fn carry(env: Envelope, biggest: &mut usize) -> Outcome<Envelope> {
	let bytes = res!(env.encode());
	assert!(bytes.len() <= MAX_ENVELOPE_BYTES, "an envelope of {} bytes passed the bound", bytes.len());
	*biggest = (*biggest).max(bytes.len());
	Envelope::decode(&bytes)
}

// One whole exchange, the dialler's digest to the listener, its reply and the dialler's push.
fn wire(d: &Peer, l: &Peer) -> Outcome<Leg> {
	let mut biggest = 0;
	let req = res!(carry(res!(d.build_anti_entropy_request("identity", node(2))), &mut biggest));
	let after = match &req.body {
		MsgKind::AntiEntropyDigest { after, .. }	=> *after,
		other => return Err(err!("Expected an AntiEntropyDigest, got {:?}.", other; Test, Mismatch)),
	};
	let mut got = res!(l.handle_envelope_at(req, 1_000));
	assert_eq!(got.outbound.len(), 1);
	let reply = res!(carry(got.outbound.remove(0), &mut biggest));
	let (next, bulk, replied) = match &reply.body {
		MsgKind::AntiEntropyReply { next, bulk, records, .. } =>
			(*next, *bulk, records.iter().map(|r| idx(&r.id)).collect::<Vec<_>>()),
		other => return Err(err!("Expected an AntiEntropyReply, got {:?}.", other; Test, Mismatch)),
	};
	let mut mine = res!(d.handle_envelope_at(reply, 1_000));
	assert!(mine.failed.is_empty(), "the dialler failed a record: {:?}", mine.failed);
	let mut refused: Vec<u16> = mine.refused.iter().map(|r| idx(&r.1)).collect();
	let mut pushed = Vec::new();
	for p in std::mem::take(&mut mine.outbound) {
		let p = res!(carry(p, &mut biggest));
		match &p.body {
			MsgKind::AntiEntropyPush { records, .. }	=> pushed.extend(records.iter().map(|r| idx(&r.id))),
			other => return Err(err!("Expected an AntiEntropyPush, got {:?}.", other; Test, Mismatch)),
		}
		let landed = res!(l.handle_envelope_at(p, 1_000));
		assert!(landed.failed.is_empty(), "the listener failed a record: {:?}", landed.failed);
		refused.extend(landed.refused.iter().map(|r| idx(&r.1)));
	}
	Ok(Leg { after, next, bulk, replied, pushed, refused, biggest })
}

// A full page of 1 MiB records is eleven, since the twelfth would pass 12 MiB.
const PAGE: usize = 11;

// Exchanges until both peers hold `total` records, at most `cap`. The records are counted and
// not hashed, since the sketch already hashes every value, and this is a debug build.
fn run(d: &Peer, l: &Peer, total: usize, cap: usize) -> Outcome<Vec<Leg>> {
	let mut legs = Vec::new();
	for _ in 0..cap {
		legs.push(res!(wire(d, l)));
		if res!(d.storage().len()) == total && res!(l.storage().len()) == total {
			break;
		}
	}
	assert!(res!(same_bytes(d, l)), "the peers did not converge in {} exchanges", legs.len());
	Ok(legs)
}


// 26 x 1 MiB at the listener, nothing at the dialler, and a 6-cell sketch that stays overloaded
// until the last page. Each exchange moves one page, `next` of one is `after` of the next, and the
// last page, which ends the table, clears the cursor.
#[test]
fn bulk_reply_pages_and_the_cursor_advances() -> Outcome<()> {
	let (d, l) = res!(pair(6, 0, 0));
	for i in 0..26u16 {
		res!(l.storage().put(&rec(i, MIB)));
	}
	let legs = res!(run(&d, &l, 26, 6));
	assert_eq!(res!(held(&d)).len(), 26);
	assert_eq!(legs.len(), 3, "26 records of 1 MiB are three pages of at most {}", PAGE);
	assert_eq!(legs[0].after, None);
	let mut seen = 0;
	let mut last: Option<RecordId> = None;
	for (k, leg) in legs.iter().enumerate() {
		assert!(leg.bulk, "exchange {} was not a bulk reply", k);
		assert!(leg.replied.len() <= PAGE, "exchange {} carried {} records", k, leg.replied.len());
		assert!(leg.biggest <= MAX_ENVELOPE_BYTES);
		if k + 1 < legs.len() {
			assert_eq!(leg.replied.len(), PAGE, "exchange {} did not fill its page", k);
			assert!(leg.biggest > 11 * MIB, "exchange {} wrote only {} bytes", k, leg.biggest);
		}
		if k > 0 {
			assert_eq!(leg.after, last, "exchange {} did not resume where {} stopped", k, k - 1);
		}
		if k + 1 < legs.len() {
			let n = res!(leg.next.ok_or_else(|| err!("exchange {} ended the table early.", k; Test, Missing)));
			if let Some(prev) = last {
				assert!(n > prev, "the cursor did not advance at exchange {}", k);
			}
			last = Some(n);
		} else {
			assert_eq!(leg.next, None, "the last page names a next");
		}
		seen += leg.replied.len();
	}
	assert_eq!(seen, 26);
	// What arrived is what was sent, byte for byte.
	for i in 0..26u16 {
		let got = res!(d.storage().get("identity", &rid(i)));
		let got = res!(got.ok_or_else(|| err!("record {} is missing.", i; Test, Missing)));
		assert_eq!(got.value, rec(i, MIB).value, "record {} came back at other bytes", i);
	}
	Ok(())
}

// The dialler holds records too, so its push is paged to the range of the listener's page. In
// the interleaved case each side holds half of 30 records, and every page both sends and pushes.
#[test]
fn dialler_also_holds_records_interleaved() -> Outcome<()> {
	let (d, l) = res!(pair(6, 0, 0));
	for i in 0..30u16 {
		if i % 2 == 0 {
			res!(d.storage().put(&rec(i, MIB)));
		} else {
			res!(l.storage().put(&rec(i, MIB)));
		}
	}
	let legs = res!(run(&d, &l, 30, 8));
	assert_eq!(res!(held(&d)).len(), 30);
	assert_eq!(res!(held(&l)).len(), 30);
	assert!(legs.len() > 1, "15 MiB each way cannot cross in one exchange");
	assert!(legs.iter().any(|g| !g.pushed.is_empty()), "the dialler never pushed");
	for leg in &legs {
		assert!(leg.pushed.len() <= PAGE && leg.replied.len() <= PAGE);
	}
	Ok(())
}

// The listener's own page is tiny and ends the table, so the dialler's 26 x 1 MiB cannot cross in
// its one push. The push stops at eleven, and the dialler asks again from the last id it sent.
#[test]
fn dialler_push_is_paged_and_resumes_after_its_last_id() -> Outcome<()> {
	let (d, l) = res!(pair(6, 0, 0));
	res!(l.storage().put(&rec(0, 100)));
	res!(l.storage().put(&rec(1_000, 100)));
	for i in 100..126u16 {
		res!(d.storage().put(&rec(i, MIB)));
	}
	let legs = res!(run(&d, &l, 28, 8));
	assert_eq!(res!(held(&l)).len(), 28);
	assert_eq!(legs[0].pushed.len(), PAGE, "the first push is one page");
	assert_eq!(legs[0].pushed, (100..100 + PAGE as u16).collect::<Vec<_>>(), "the push is in id order");
	assert_eq!(legs[1].after, Some(rid(100 + PAGE as u16 - 1)), "the dialler resumes after the last id it pushed");
	assert!(legs.iter().all(|g| g.pushed.len() <= PAGE));
	Ok(())
}

// 26 x 1 MiB at the listener, a sketch that decodes, and a dialler that refuses the first 12 ids.
// A decoded reply is paged and the refused records stay in the difference, so unless the cursor
// moves past them the first page is the same 11 refused records on every exchange and the
// records behind never arrive. The refused are offered again on the next lap.
#[test]
fn refusing_dialler_does_not_block_the_records_behind() -> Outcome<()> {
	let (d, l) = res!(pair(256, 12, 0));
	for i in 0..26u16 {
		res!(l.storage().put(&rec(i, MIB)));
	}
	let mut tries: HashMap<u16, usize> = HashMap::new();
	let mut legs = Vec::new();
	for _ in 0..6 {
		let leg = res!(wire(&d, &l));
		for i in &leg.refused {
			*tries.entry(*i).or_insert(0) += 1;
		}
		legs.push(leg);
	}
	assert!(legs.iter().all(|g| !g.bulk), "the sketch should decode");
	let want: BTreeSet<u16> = (12..26).collect();
	assert_eq!(res!(held(&d)), want, "the records after the refused ones did not all arrive");
	for i in 0..12u16 {
		assert!(tries.get(&i).copied().unwrap_or(0) >= 2, "refused record {} was not offered again", i);
	}
	assert!(legs.iter().all(|g| g.biggest <= MAX_ENVELOPE_BYTES));
	Ok(())
}

// The mirror: the dialler holds 26 x 1 MiB and the listener refuses the first 12. The dialler's
// push is paged, and its cursor must pass the refused records for the rest to land.
#[test]
fn refusing_listener_does_not_block_the_push() -> Outcome<()> {
	let (d, l) = res!(pair(256, 0, 12));
	for i in 0..26u16 {
		res!(d.storage().put(&rec(i, MIB)));
	}
	let mut tries: HashMap<u16, usize> = HashMap::new();
	for _ in 0..6 {
		let leg = res!(wire(&d, &l));
		for i in &leg.refused {
			*tries.entry(*i).or_insert(0) += 1;
		}
	}
	let want: BTreeSet<u16> = (12..26).collect();
	assert_eq!(res!(held(&l)), want, "the records after the refused ones did not all arrive");
	for i in 0..12u16 {
		assert!(tries.get(&i).copied().unwrap_or(0) >= 2, "refused record {} was not offered again", i);
	}
	Ok(())
}
