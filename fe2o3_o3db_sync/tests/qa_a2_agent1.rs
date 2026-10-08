#![cfg(feature = "dist")]
//! Post-landing QA of distributed Ozone A2, agent 1 (identity, anti-entropy, convergence).
//! Every test here fails on 96d0c015 and names the defect it shows.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_o3db_sync::{
	dist::{
		config::{
			Consistency,
			DistOzoneConfig,
			TableConfig,
		},
		engine::DistOzone,
		record::{
			Record,
			RecordId,
			content_hash,
		},
		resolve::{
			Convergence,
			LastVersionWins,
			ReadView,
			ResolveCtx,
			Resolver,
			Verdict,
			check_convergence,
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

use std::time::Duration;


fn node(b: u8) -> NodeId {
	let mut bytes = [0u8; 32];
	bytes[31] = b;
	NodeId::from_bytes(bytes)
}

fn rid2(i: u16) -> RecordId {
	let mut bytes = [0u8; 32];
	bytes[0] = (i >> 8) as u8;
	bytes[1] = i as u8;
	RecordId::from_bytes(bytes)
}

fn pair() -> Outcome<(DistOzone<MemoryStorage>, DistOzone<MemoryStorage>)> {
	let mk = |local: u8, remote: u8| -> Outcome<DistOzoneConfig> {
		let oam = res!(OamConfig::new(2, 2));
		let table = res!(TableConfig::new("identity", Consistency::Eventual, Duration::from_secs(30), 256));
		DistOzoneConfig::new(node(local), vec![node(remote)], oam, vec![table])
	};
	Ok((
		res!(DistOzone::new(res!(mk(1, 2)), MemoryStorage::new())),
		res!(DistOzone::new(res!(mk(2, 1)), MemoryStorage::new())),
	))
}

// One exchange as a real transport runs it: every envelope is encoded and decoded, and one the
// codec refuses to write is lost, as it would be on the wire.
fn wire_exchange(d: &DistOzone<MemoryStorage>, l: &DistOzone<MemoryStorage>, l_id: u8) -> Outcome<usize> {
	let mut lost = 0;
	let req = res!(d.build_anti_entropy_request("identity", node(l_id)));
	let req = res!(Envelope::decode(&res!(req.encode())));
	let out = res!(l.handle_envelope_at(req, 1_000));
	for reply in out.outbound {
		let bytes = match reply.encode() {
			Ok(b)	=> b,
			Err(_)	=> { lost += 1; continue; },
		};
		let back = res!(d.handle_envelope_at(res!(Envelope::decode(&bytes)), 1_000));
		for push in back.outbound {
			match push.encode() {
				Ok(b)	=> { res!(l.handle_envelope_at(res!(Envelope::decode(&b)), 1_000)); },
				Err(_)	=> lost += 1,
			}
		}
	}
	Ok(lost)
}

// F1 (S1). A peer that rejoins a table holding more than MAX_ENVELOPE_BYTES never converges: its
// empty sketch overloads, the bulk reply is the listener's whole table, and the codec refuses to
// write it, on every exchange, for ever. 300 records of 64 KiB is 19.7 MB.
#[test]
fn qa1_rejoin_over_16mib_table_converges() -> Outcome<()> {
	let (d, l) = res!(pair());
	for i in 0..300u16 {
		res!(l.storage().put(&Record::new(rid2(i), "identity", vec![(i & 0xff) as u8; 64 * 1024])));
	}
	let mut lost = 0;
	for _ in 0..3 {
		lost += res!(wire_exchange(&d, &l, 2));
	}
	let held = res!(d.storage().digests("identity")).len();
	assert_eq!(held, 300, "the rejoining peer holds {} of 300 after 3 exchanges ({} envelopes the codec refused)", held, lost);
	Ok(())
}

// F1 (S1), without overload. A difference the sketch decodes (100 entries, under the ~170 that
// 256 cells hold) of 200 KiB records is a 20 MB reply, refused alike, so it never repairs.
#[test]
fn qa1_decoded_difference_over_16mib_converges() -> Outcome<()> {
	let (d, l) = res!(pair());
	for i in 0..100u16 {
		res!(l.storage().put(&Record::new(rid2(i), "identity", vec![(i & 0xff) as u8; 200 * 1024])));
	}
	let mut lost = 0;
	for _ in 0..3 {
		lost += res!(wire_exchange(&d, &l, 2));
	}
	let held = res!(d.storage().digests("identity")).len();
	assert_eq!(held, 100, "the dialler holds {} of 100 after 3 exchanges ({} envelopes refused)", held, lost);
	Ok(())
}

// F2 (S2). A sketch whose header claims a key length near usize::MAX panics the listener inside
// Iblt::from_bytes (an unchecked `key_len + value_len`, then `vec![0; num_cells * key_len]`)
// before the engine's config check can refuse it. The codec passes the sketch as opaque bytes.
#[test]
fn qa1_hostile_sketch_header_is_an_err_not_a_panic() -> Outcome<()> {
	let (_, l) = res!(pair());
	let mut sketch = Vec::with_capacity(40 + 76);
	sketch.extend_from_slice(&1u64.to_le_bytes());					// num_cells
	sketch.extend_from_slice(&1u64.to_le_bytes());					// num_hashes
	sketch.extend_from_slice(&(1u64 << 63).to_le_bytes());			// key_len
	sketch.extend_from_slice(&((1u64 << 63) + 64).to_le_bytes());	// value_len; the sum wraps to 64
	sketch.extend_from_slice(&0u64.to_le_bytes());					// seed
	sketch.extend_from_slice(&[0u8; 76]);							// one cell of the wrapped width
	let env = Envelope::new(node(1), node(2), MsgKind::AntiEntropyDigest {
		table:	"identity".to_string(),
		sketch,
		after:	None,
	});
	// The envelope survives the codec, so a peer can send it.
	let env = res!(Envelope::decode(&res!(env.encode())));
	let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l.handle_envelope_at(env, 1_000)));
	match got {
		Err(_)		=> panic!("a hostile sketch header panicked the listener"),
		Ok(Ok(_))	=> panic!("a hostile sketch header was accepted"),
		Ok(Err(_))	=> {},
	}
	Ok(())
}

/// Defers a value stamped after the clock; once the clock passes, the first value an id
/// receives wins, which diverges by arrival order.
struct FirstWinsAfter;

impl Resolver for FirstWinsAfter {
	fn resolve<V: ReadView>(
		&self,
		ctx:		&ResolveCtx,
		_view:		&V,
		_table:		&str,
		_id:		&RecordId,
		held:		Option<&[u8]>,
		incoming:	&[u8],
	)
		-> Outcome<Verdict>
	{
		if let Some(stamp) = LastVersionWins::version(incoming) {
			if stamp > ctx.now_ms {
				return Ok(Verdict::Defer);
			}
		}
		Ok(match held {
			Some(_)	=> Verdict::Keep,
			None	=> Verdict::Take(incoming.to_vec()),
		})
	}
}

// F3 (S2). check_convergence calls a run quiet at the first full round that stores nothing, even
// while local puts it deferred are still pending and the skewed clocks are still moving. So a
// time-deferred record is never offered past its time, and a rule that diverges once the time
// comes passes, while the same rule with a nearer time fails.
#[test]
fn qa1_check_reports_divergence_after_a_deferred_time() -> Outcome<()> {
	const NOW: u64 = 1_000;
	let run = |stamp: u64| -> Outcome<()> {
		let x = RecordId::from_bytes([9u8; 32]);
		let records = vec![
			Record::new(x, "identity", LastVersionWins::value(stamp, &[1])),
			Record::new(x, "identity", LastVersionWins::value(stamp, &[2])),
		];
		let cfg = Convergence::new(3, 0..16, NOW, 16).with_skew_ms(400);
		check_convergence(&cfg, &["identity"], &records, |_| FirstWinsAfter, |_, _, _| Ok(Vec::new()))
	};
	// Control: inside the clocks' first step, the divergence is reported.
	assert!(run(NOW + 200).is_err(), "control: the near stamp did not fail");
	// Five steps out, within max_rounds = 16: the same rule passes.
	assert!(run(NOW + 2_000).is_err(), "a rule that diverges after its deferred time passed the check");
	Ok(())
}

// F4 (S3). When the dialler takes the listener's value, it pushes that value straight back,
// although the listener holds those very bytes, so each such repair crosses the wire twice. The
// bulk path already skips this case.
#[test]
fn qa1_push_does_not_echo_the_listeners_own_bytes() -> Outcome<()> {
	let (d, l) = res!(pair());
	let x = RecordId::from_bytes([7u8; 32]);
	let old = LastVersionWins::value(1, b"old");
	let new = LastVersionWins::value(2, b"new");
	res!(d.storage().put(&Record::new(x, "identity", old)));
	res!(l.storage().put(&Record::new(x, "identity", new.clone())));
	let req = res!(d.build_anti_entropy_request("identity", node(2)));
	let mut out = res!(l.handle_envelope_at(req, 1_000));
	let back = res!(d.handle_envelope_at(out.outbound.remove(0), 1_000));
	for env in &back.outbound {
		if let MsgKind::AntiEntropyPush { records, .. } = &env.body {
			for r in records {
				assert!(content_hash(&r.value) != content_hash(&new),
					"the push sends the listener the bytes it already holds at {:?}", r.id);
			}
		}
	}
	Ok(())
}
