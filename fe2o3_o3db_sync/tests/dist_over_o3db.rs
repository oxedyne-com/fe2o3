#![cfg(feature = "dist")]
//! Integration tests for [`DistOzone::over_o3db`] and [`DistOzone::close`].
//!
//! An application opens a distributed engine over an Ozone database in one call and closes it in
//! another, and what a put returned for must survive the pair and a reopen of the same root.
//! Each test works in its own `./test_db_dist_over_o3db_<name>` root, wiped when the test begins.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_crypto::enc::EncryptionScheme;
use oxedyne_fe2o3_hash::{
	csum::ChecksumScheme,
	hash::HashScheme,
};
use oxedyne_fe2o3_jdat::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
	base::cfg::OzoneConfig,
	data::core::RestSchemesInput,
	dist::{
		config::{
			DistOzoneConfig,
			TableConfig,
		},
		engine::DistOzone,
		o3db_storage::O3dbStorage,
		record::{
			Record,
			RecordDigest,
			RecordId,
			content_hash,
		},
		resolve::{
			LastVersionWins,
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
	test::setup,
};

use std::{
	path::PathBuf,
	sync::{
		Arc,
		Mutex,
	},
};


// One test at a time, so that the process's thread count belongs to the test that reads it.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());


type Eng<R> = DistOzone<
	O3dbStorage<{ setup::UID_LEN }, setup::Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
	R,
>;

fn node(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}

fn rid(b: u8) -> RecordId {
	RecordId::from_bytes([b; 32])
}

fn rec(table: &str, id: RecordId, value: &[u8]) -> Record {
	Record::new(id, table, value.to_vec())
}

/// The threads this process runs, which a database that is open adds its bots to.
fn threads() -> Outcome<usize> {
	Ok(res!(std::fs::read_dir("/proc/self/task")).count())
}

/// A root of its own for the test `name`, emptied before use and left behind after it.
fn fresh_root(name: &str) -> Outcome<PathBuf> {
	let dir = PathBuf::from(fmt!("./test_db_dist_over_o3db_{}", name));
	if dir.exists() {
		res!(std::fs::remove_dir_all(&dir));
	}
	res!(std::fs::create_dir_all(&dir));
	Ok(res!(dir.canonicalize()))
}

fn schemes() -> Outcome<RestSchemesInput<EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>> {
	let enckey = [0x55u8; 32];
	let aes_gcm = res!(EncryptionScheme::new_aes_256_gcm_with_key(&enckey[..]));
	Ok(RestSchemesInput::new(
		Some(aes_gcm),
		None::<HashScheme>,
		None::<HashScheme>,
		Some(ChecksumScheme::new_crc32()),
	))
}

/// The stock store: two zones, the stock cache, the default durability barrier.
fn ozone_cfg() -> OzoneConfig {
	OzoneConfig::default()
}

/// One peer holding everything, with these eventual tables.
fn single(tables: &[&str]) -> Outcome<DistOzoneConfig> {
	let oam = res!(OamConfig::new(1, 1));
	let mut tcs = Vec::with_capacity(tables.len());
	for t in tables {
		tcs.push(res!(TableConfig::eventual(*t)));
	}
	DistOzoneConfig::new(node(1), Vec::new(), oam, tcs)
}

/// Two peers that both hold everything, seen from peer `local`.
fn pair(local: u8, remote: u8, table: &str) -> Outcome<DistOzoneConfig> {
	let oam = res!(OamConfig::new(2, 2));
	let tcs = vec![res!(TableConfig::eventual(table))];
	DistOzoneConfig::new(node(local), vec![node(remote)], oam, tcs)
}

fn open<R: Resolver>(
	root:		&PathBuf,
	ozone_cfg:	Option<OzoneConfig>,
	dist_cfg:	DistOzoneConfig,
	resolver:	R,
)
	-> Outcome<Eng<R>>
{
	Eng::<R>::over_o3db(
		root.clone(),
		ozone_cfg,
		dist_cfg,
		res!(schemes()),
		setup::Uid::default(),
		resolver,
	)
}

fn held<S: Storage, R: Resolver>(
	e:		&DistOzone<S, R>,
	table:	&str,
	id:		RecordId,
)
	-> Outcome<Option<Vec<u8>>>
{
	Ok(res!(e.storage().get(table, &id)).map(|r| r.value))
}

fn digests<S: Storage, R: Resolver>(e: &DistOzone<S, R>, table: &str) -> Outcome<Vec<RecordDigest>> {
	e.storage().digests(table)
}

/// Refuses a value that begins with "BAD", and takes any other.
struct RefuseBad;

impl Resolver for RefuseBad {
	fn resolve<V: ReadView>(
		&self,
		_ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		_held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		if incoming.starts_with(b"BAD") {
			return Ok(Verdict::Refuse("marked bad".to_string()));
		}
		Ok(Verdict::Take(incoming.to_vec()))
	}
}

/// What the engine's answer to a peer's anti-entropy digest says: the records it would send, the
/// ids it asks for, and whether it fell back to a bulk reply.
fn answer<S1: Storage, R1: Resolver, S2: Storage, R2: Resolver>(
	asker:		&DistOzone<S1, R1>,
	answerer:	&DistOzone<S2, R2>,
	answerer_id: u8,
	table:		&str,
)
	-> Outcome<(Vec<Record>, Vec<RecordId>, bool)>
{
	let req = res!(asker.build_anti_entropy_request(table, node(answerer_id)));
	let mut got = res!(answerer.handle_envelope_at(req, 1_000));
	assert_eq!(got.outbound.len(), 1);
	let env: Envelope = got.outbound.remove(0);
	match env.body {
		MsgKind::AntiEntropyReply { records, requested_ids, bulk, .. } => Ok((records, requested_ids, bulk)),
		other => Err(err!("Expected an AntiEntropyReply, got {:?}.", other; Test, Mismatch)),
	}
}


#[test]
fn opens_puts_closes_reopens_reads_back() -> Outcome<()> {
	let _one = lock_mutex!(ONE_AT_A_TIME);
	let root = res!(fresh_root("reopen"));
	let dist = || single(&["identity", "escrow"]);
	let idle = res!(threads());
	let e = res!(open(&root, Some(ozone_cfg()), res!(dist()), LastVersionWins));
	let running = res!(threads());
	assert!(running > idle + 10, "an open database should run its bots: {} threads before, {} after", idle, running);

	// Two values that differ only by a trailing zero, in one table, and a longer one in another.
	let puts = [
		("identity",	rid(1),	vec![0xABu8]),
		("identity",	rid(2),	vec![0xABu8, 0x00]),
		("escrow",		rid(1),	b"gamma, longer than one chunk of eight bytes".to_vec()),
	];
	for (table, id, value) in &puts {
		let out = res!(e.put_at(rec(table, *id, value), 1_000));
		assert!(out.local_persisted, "the put of {:?} in {} was not stored", id, table);
	}
	let before_identity = res!(digests(&e, "identity"));
	let before_escrow = res!(digests(&e, "escrow"));
	assert_eq!(before_identity.len(), 2);
	assert_eq!(before_escrow.len(), 1);
	assert_ne!(before_identity[0].content, before_identity[1].content);
	res!(e.close());
	// A close returns once the bots have stopped, so their threads are gone, or going.
	let closed = res!(threads());
	assert!(closed < idle + (running - idle) / 2,
		"close left the bots running: {} threads before the open, {} open, {} after the close", idle, running, closed);

	// The same root, no Ozone configuration given: the one saved there is used.
	let e = res!(open(&root, None, res!(dist()), LastVersionWins));
	for (table, id, value) in &puts {
		assert_eq!(res!(held(&e, table, *id)).as_ref(), Some(value),
			"{} {:?} did not come back after the reopen", table, id);
	}
	assert_eq!(res!(digests(&e, "identity")), before_identity);
	assert_eq!(res!(digests(&e, "escrow")), before_escrow);
	for d in &before_identity {
		let value = if d.id == rid(1) { vec![0xABu8] } else { vec![0xABu8, 0x00] };
		assert_eq!(d.content, content_hash(&value));
	}
	res!(e.close());
	Ok(())
}

#[test]
fn a_dropped_engine_stops_its_bots_and_keeps_what_it_stored() -> Outcome<()> {
	let _one = lock_mutex!(ONE_AT_A_TIME);
	let root = res!(fresh_root("dropped"));
	let idle = res!(threads());
	let e = res!(open(&root, Some(ozone_cfg()), res!(single(&["identity"])), LastVersionWins));
	let running = res!(threads());
	assert!(running > idle + 10, "an open database should run its bots: {} threads before, {} after", idle, running);
	let out = res!(e.put_at(rec("identity", rid(1), b"kept without a close"), 1_000));
	assert!(out.local_persisted, "the put was not stored");

	// Dropped as an error path or a panic unwinds it, with no close.
	drop(e);
	let dropped = res!(threads());
	assert!(dropped < idle + (running - idle) / 2,
		"dropping the engine left the bots running: {} threads before the open, {} open, {} after the drop",
		idle, running, dropped);

	// The bots were stopped in order, so the same root opens again with the record in it.
	let e = res!(open(&root, None, res!(single(&["identity"])), LastVersionWins));
	assert_eq!(res!(held(&e, "identity", rid(1))), Some(b"kept without a close".to_vec()));
	res!(e.close());
	Ok(())
}

#[test]
fn dropping_a_shared_storage_leaves_the_database_to_its_other_holder() -> Outcome<()> {
	let _one = lock_mutex!(ONE_AT_A_TIME);
	let root = res!(fresh_root("dropshared"));
	let e = res!(open(&root, Some(ozone_cfg()), res!(single(&["identity"])), LastVersionWins));
	res!(e.put_at(rec("identity", rid(1), b"still open"), 1_000));
	let extra = Arc::clone(e.storage().db());

	// The engine goes, the database does not: someone else still holds it.
	drop(e);
	let storage = O3dbStorage::new(extra, setup::Uid::default());
	let back = res!(storage.get("identity", &rid(1)));
	assert_eq!(back.map(|r| r.value), Some(b"still open".to_vec()));
	res!(storage.close());
	Ok(())
}

#[test]
fn over_o3db_uses_the_given_resolver() -> Outcome<()> {
	let _one = lock_mutex!(ONE_AT_A_TIME);
	let root = res!(fresh_root("resolver"));
	let e = res!(open(&root, Some(ozone_cfg()), res!(single(&["identity"])), RefuseBad));

	let refused = res!(e.put_at(rec("identity", rid(1), b"BAD value"), 1_000));
	assert!(!refused.local_persisted);
	assert!(matches!(refused.verdict, Some(Verdict::Refuse(_))), "verdict was {:?}", refused.verdict);
	let taken = res!(e.put_at(rec("identity", rid(2), b"good value"), 1_000));
	assert!(taken.local_persisted);
	assert_eq!(res!(held(&e, "identity", rid(1))), None);
	res!(e.close());

	// Reopened under the same resolver, the refused value is still absent and the other present.
	let e = res!(open(&root, None, res!(single(&["identity"])), RefuseBad));
	assert_eq!(res!(held(&e, "identity", rid(1))), None);
	assert_eq!(res!(held(&e, "identity", rid(2))), Some(b"good value".to_vec()));
	assert_eq!(res!(digests(&e, "identity")).len(), 1);
	res!(e.close());
	Ok(())
}

/// What a peer answers to another's anti-entropy digest: the records it would send, the ids it
/// asks for, and whether it fell back to a bulk reply.
type Answer = (Vec<Record>, Vec<RecordId>, bool);

#[test]
fn memory_and_o3db_peers_decode_empty_difference() -> Outcome<()> {
	let _one = lock_mutex!(ONE_AT_A_TIME);
	let root = res!(fresh_root("mixed"));
	// Peer 1 holds in memory. Peer 2 is held by O3db, and a second peer 2 in memory is the
	// reference: whatever the reference pair does, the mixed pair must do alike.
	let mem: DistOzone<MemoryStorage> = res!(DistOzone::new(res!(pair(1, 2, "identity")), MemoryStorage::new()));
	let o3 = res!(open(&root, Some(ozone_cfg()), res!(pair(2, 1, "identity")), LastVersionWins));
	let refr: DistOzone<MemoryStorage> = res!(DistOzone::new(res!(pair(2, 1, "identity")), MemoryStorage::new()));

	let long = b"a longer value than one chunk of eight bytes".to_vec();
	let records = [
		(rid(1),	vec![0xABu8]),
		(rid(2),	vec![0xABu8, 0x00]),
		(rid(3),	long.clone()),
	];
	for (id, value) in &records {
		for e in [&mem, &refr] {
			res!(e.put_at(rec("identity", *id, value), 1_000));
		}
		res!(o3.put_at(rec("identity", *id, value), 1_000));
	}
	assert_eq!(res!(digests(&mem, "identity")), res!(digests(&o3, "identity")));

	// The two answers each way, from the mixed pair and from the reference pair.
	let both = |_: ()| -> Outcome<(Answer, Answer, Answer, Answer)> {
		Ok((
			res!(answer(&mem, &o3, 2, "identity")),
			res!(answer(&o3, &mem, 1, "identity")),
			res!(answer(&mem, &refr, 2, "identity")),
			res!(answer(&refr, &mem, 1, "identity")),
		))
	};

	// The same three records on both sides: nothing to send, nothing to ask for, no bulk fallback.
	let (m_asks, o_asks, m_ref, o_ref) = res!(both(()));
	for (who, a) in [("memory asked, O3db answered", &m_asks), ("O3db asked, memory answered", &o_asks)] {
		assert!(a.0.is_empty() && a.1.is_empty() && !a.2, "{}: {:?}", who, a);
	}
	assert_eq!((&m_asks, &o_asks), (&m_ref, &o_ref));

	// One value revised at O3db and at the reference. The mixed pair answers as the reference
	// pair does, and each way the answer is exactly one record, the answerer's own copy, with the
	// id asked for in return and no bulk fallback.
	let newer = b"a longer value than one chunk of eight bytes, revised".to_vec();
	for e in [&refr] {
		assert!(res!(e.put_at(rec("identity", rid(3), &newer), 2_000)).local_persisted);
	}
	assert!(res!(o3.put_at(rec("identity", rid(3), &newer), 2_000)).local_persisted);
	let (m_asks, o_asks, m_ref, o_ref) = res!(both(()));
	assert_eq!((&m_asks, &o_asks), (&m_ref, &o_ref));
	assert_eq!(m_asks, (vec![rec("identity", rid(3), &newer)], vec![rid(3)], false));
	assert_eq!(o_asks, (vec![rec("identity", rid(3), &long)], vec![rid(3)], false));

	// The memory peer takes the revision, and the pair agrees again.
	assert!(res!(mem.put_at(rec("identity", rid(3), &newer), 2_000)).local_persisted);
	assert_eq!(res!(digests(&mem, "identity")), res!(digests(&o3, "identity")));
	let (m_asks, o_asks, _, _) = res!(both(()));
	for a in [&m_asks, &o_asks] {
		assert!(a.0.is_empty() && a.1.is_empty() && !a.2, "after the revision: {:?}", a);
	}

	// One record only the O3db peer holds: exactly one record one way and one request the other.
	let extra = vec![0x04u8; 40];
	for e in [&refr] {
		res!(e.put_at(rec("identity", rid(4), &extra), 3_000));
	}
	res!(o3.put_at(rec("identity", rid(4), &extra), 3_000));
	let (m_asks, o_asks, m_ref, o_ref) = res!(both(()));
	assert_eq!((&m_asks, &o_asks), (&m_ref, &o_ref));
	assert_eq!(m_asks, (vec![rec("identity", rid(4), &extra)], Vec::new(), false));
	assert_eq!(o_asks, (Vec::new(), vec![rid(4)], false));

	res!(o3.close());
	Ok(())
}

#[test]
fn close_refuses_while_shared() -> Outcome<()> {
	let _one = lock_mutex!(ONE_AT_A_TIME);
	let root = res!(fresh_root("shared"));
	let e = res!(open(&root, Some(ozone_cfg()), res!(single(&["identity"])), LastVersionWins));
	res!(e.put_at(rec("identity", rid(1), b"kept"), 1_000));

	// A second holder of the database.
	let extra = Arc::clone(e.storage().db());
	let refused = match e.close() {
		Ok(()) => return Err(err!("close succeeded with another reference to the database held."; Test, Unexpected)),
		Err(refused) => refused,
	};
	let said = fmt!("{}", refused);
	assert!(said.contains("1 other reference"), "the refusal does not name the holder: {}", said);

	// The refusal left the database open and the record in it.
	let storage = O3dbStorage::new(extra, setup::Uid::default());
	let back = res!(storage.get("identity", &rid(1)));
	assert_eq!(back.map(|r| r.value), Some(b"kept".to_vec()));
	res!(storage.close());
	Ok(())
}

#[test]
fn a_put_returned_before_close_reads_back_after_reopen_on_one_zone() -> Outcome<()> {
	let _one = lock_mutex!(ONE_AT_A_TIME);
	let root = res!(fresh_root("one_zone"));
	// One zone, a barrier on every write, and a cache far smaller than what is stored.
	let overrides = res!(mapdat!{
		1u16 => mapdat!{ "dir" => "", "max_size" => 104_857_600u64 },
	}.get_map().ok_or_else(|| err!("The zone override map is not a map."; Test, Invalid)));
	let cfg = OzoneConfig {
		num_zones:				1,
		sync_on_write:			true,
		cache_size_limit_bytes:	4_096,
		zone_overrides:			overrides,
		..OzoneConfig::default()
	};
	let e = res!(open(&root, Some(cfg), res!(single(&["identity"])), LastVersionWins));
	let mut puts = Vec::new();
	for n in 1..=24u8 {
		let value: Vec<u8> = (0..300u32).map(|i| (i as u8).wrapping_mul(n).wrapping_add(n)).collect();
		let out = res!(e.put_at(rec("identity", rid(n), &value), 1_000));
		assert!(out.local_persisted);
		puts.push((rid(n), value));
	}
	res!(e.close());

	let e = res!(open(&root, None, res!(single(&["identity"])), LastVersionWins));
	for (id, value) in &puts {
		assert_eq!(res!(held(&e, "identity", *id)).as_ref(), Some(value),
			"{:?} did not come back after the reopen", id);
	}
	assert_eq!(res!(digests(&e, "identity")).len(), puts.len());
	res!(e.close());
	Ok(())
}
