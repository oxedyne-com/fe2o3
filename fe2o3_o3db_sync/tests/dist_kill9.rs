#![cfg(feature = "dist")]
//! A put the engine returned for survives `kill -9` of the process (A2 QA agent 2).
//!
//! The one-zone configuration that `DistOzone::over_o3db` is used with in production: one zone,
//! a barrier on every write and a cache far smaller than what is stored. The test re-runs its
//! own binary as a child that puts records in a loop and prints each id once `put_at` has
//! returned for it. The parent reads those lines, kills the child with SIGKILL in mid-stream,
//! reopens the root, and every acknowledged record must read back byte for byte. Three rounds
//! reopen a root that the previous round crashed. The one write the child was inside when it
//! died may be there or not, but if it is there it must be whole.
//!
//! Each run takes about half a minute at most.
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
			RecordId,
		},
		resolve::LastVersionWins,
		storage::Storage,
	},
	kademlia::id::NodeId,
	oam::config::OamConfig,
	test::setup,
};

use std::{
	collections::BTreeMap,
	io::{
		BufRead,
		BufReader,
	},
	os::unix::process::ExitStatusExt,
	path::PathBuf,
	process::{
		Command,
		Stdio,
	},
	time::Duration,
};


type Eng = DistOzone<
	O3dbStorage<{ setup::UID_LEN }, setup::Uid, EncryptionScheme, HashScheme, HashScheme, ChecksumScheme>,
	LastVersionWins,
>;

const ENV_ROOT:		&str = "DIST_KILL9_ROOT";
const ENV_START:	&str = "DIST_KILL9_START";
const ENV_FRESH:	&str = "DIST_KILL9_FRESH";
const ACK:			&str = "ACK ";

fn rid(n: u32) -> RecordId {
	let mut bytes = [0xABu8; 32];
	bytes[..4].copy_from_slice(&n.to_be_bytes());
	RecordId::from_bytes(bytes)
}

fn number(id: &RecordId) -> u32 {
	let b = id.as_bytes();
	u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

// 200 to 1,500 bytes, some past 256 so that a width is exercised, and different for each n.
fn value(n: u32) -> Vec<u8> {
	let len = 200 + (n % 97) as usize * 13;
	(0..len).map(|i| (i as u32).wrapping_mul(n).wrapping_add(n >> 3) as u8).collect()
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

// One zone, a barrier on every write, and a cache far smaller than what is stored. The saved
// configuration wins on a reopen, so only a fresh root reads it.
fn one_zone() -> Outcome<OzoneConfig> {
	let overrides = res!(mapdat!{
		1u16 => mapdat!{ "dir" => "", "max_size" => 104_857_600u64 },
	}.get_map().ok_or_else(|| err!("The zone override map is not a map."; Test, Invalid)));
	Ok(OzoneConfig {
		num_zones:				1,
		sync_on_write:			true,
		cache_size_limit_bytes:	4_096,
		zone_overrides:			overrides,
		..OzoneConfig::default()
	})
}

fn open(root: &PathBuf, fresh: bool) -> Outcome<Eng> {
	let oam = res!(OamConfig::new(1, 1));
	let dist = res!(DistOzoneConfig::new(
		NodeId::from_bytes([1u8; 32]),
		Vec::new(),
		oam,
		vec![res!(TableConfig::eventual("identity"))],
	));
	let cfg = if fresh { Some(res!(one_zone())) } else { None };
	Eng::over_o3db(root.clone(), cfg, dist, res!(schemes()), setup::Uid::default(), LastVersionWins)
}

fn root_dir(name: &str) -> PathBuf {
	PathBuf::from(fmt!("./test_db_dist_kill9_{}", name))
}

// The child. It does nothing unless the parent set its environment, so an ordinary run of this
// binary passes it at once.
#[test]
fn kill9_child() -> Outcome<()> {
	let root = match std::env::var(ENV_ROOT) {
		Ok(r)	=> PathBuf::from(r),
		Err(_)	=> return Ok(()),
	};
	let start: u32 = res!(res!(std::env::var(ENV_START)).parse::<u32>());
	let fresh = std::env::var(ENV_FRESH).is_ok();
	// A child the parent never kills ends itself.
	std::thread::spawn(|| {
		std::thread::sleep(Duration::from_secs(150));
		std::process::exit(3);
	});
	let e = res!(open(&root, fresh));
	for n in start.. {
		let out = res!(e.put_at(Record::new(rid(n), "identity", value(n)), 1_000));
		assert!(out.local_persisted);
		// Only now has the put returned, so only now is the id acknowledged.
		println!("{}{}", ACK, n);
	}
	Ok(())
}

#[test]
fn kill9_acknowledged_puts_survive() -> Outcome<()> {
	let root = root_dir("rounds");
	if root.exists() {
		res!(std::fs::remove_dir_all(&root));
	}
	res!(std::fs::create_dir_all(&root));
	let root = res!(root.canonicalize());
	let exe = res!(std::env::current_exe());

	let mut acked: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
	let rounds: [(u32, usize); 3] = [(1, 25), (1_000_000, 50), (2_000_000, 90)];
	for (round, (start, kill_after)) in rounds.iter().enumerate() {
		let mut child = res!(Command::new(&exe)
			.args(["kill9_child", "--exact", "--nocapture", "--test-threads=1"])
			.env(ENV_ROOT, &root)
			.env(ENV_START, start.to_string())
			.envs(if round == 0 { Some((ENV_FRESH, "1")) } else { None })
			.stdout(Stdio::piped())
			.stderr(Stdio::inherit())
			.spawn());
		let out = res!(child.stdout.take().ok_or_else(|| err!("The child has no stdout."; Test, Missing)));
		let mut lines = BufReader::new(out);
		let mut this_round = 0usize;
		let mut line = String::new();
		let mut killed = false;
		loop {
			line.clear();
			let got = res!(lines.read_line(&mut line));
			if got == 0 {
				break;
			}
			// A line cut by the kill has no newline and is not an acknowledgement.
			if !line.ends_with('\n') {
				continue;
			}
			if let Some(n) = line.trim_end().strip_prefix(ACK) {
				let n: u32 = res!(n.parse::<u32>());
				acked.insert(n, value(n));
				this_round += 1;
				if this_round == *kill_after && !killed {
					// SIGKILL, in mid-stream: the child is already inside its next put.
					res!(child.kill());
					killed = true;
				}
			}
		}
		let status = res!(child.wait());
		assert!(killed, "round {}: the child ended after {} acknowledgements, before the kill ({:?})",
			round, this_round, status);
		assert_eq!(status.signal(), Some(9), "round {}: the child was not ended by SIGKILL", round);
		assert!(this_round >= *kill_after);

		// Reopen what the crash left, and read everything acknowledged so far.
		let e = res!(open(&root, false));
		for (n, want) in &acked {
			let got = res!(e.storage().get("identity", &rid(*n)));
			let got = res!(got.ok_or_else(|| err!("round {}: acknowledged record {} is missing.", round, n; Test, Missing)));
			assert_eq!(&got.value, want, "round {}: record {} came back at other bytes", round, n);
		}
		// A write the child was inside when it died may be present, whole, or absent.
		let digests = res!(e.storage().digests("identity"));
		for d in &digests {
			let n = number(&d.id);
			if acked.contains_key(&n) {
				continue;
			}
			if let Some(r) = res!(e.storage().get("identity", &d.id)) {
				assert_eq!(r.value, value(n), "round {}: unacknowledged record {} is torn", round, n);
			}
		}
		assert!(digests.len() >= acked.len());
		assert!(digests.len() <= acked.len() + round + 1,
			"round {}: {} records stored, {} acknowledged", round, digests.len(), acked.len());
		res!(e.close());
	}
	Ok(())
}
