//! `lineset::union_lines` and `lineset::missing_lines`: a file as a multiset of lines.
//!
//! Written for the Daimond page log (10 Oct 2026). On 7 Oct two devices each appended one row
//! to the same diet log while apart, and the merge kept one side's. The first case is that
//! shape, with synthetic rows.

use oxedyne_fe2o3_text::lineset::{
	missing_lines,
	union_lines,
	Joined,
};

use std::collections::BTreeMap;


// The rows of a buffer as a multiset.
fn content(buf: &[u8]) -> BTreeMap<Vec<u8>, usize> {
	let mut map = BTreeMap::new();
	for line in buf.split(|b| *b == b'\n') {
		if !line.is_empty() {
			*map.entry(line.to_vec()).or_insert(0) += 1;
		}
	}
	map
}

fn max_of(a: &BTreeMap<Vec<u8>, usize>, b: &BTreeMap<Vec<u8>, usize>) -> BTreeMap<Vec<u8>, usize> {
	let mut out = a.clone();
	for (k, n) in b {
		let e = out.entry(k.clone()).or_insert(0);
		*e = (*e).max(*n);
	}
	out
}

fn join(arriving: &str, here: &str) -> Joined {
	union_lines(arriving.as_bytes(), here.as_bytes())
}

fn text(j: &Joined) -> String {
	String::from_utf8_lossy(&j.bytes).into_owned()
}

struct XorShift(u64);

impl XorShift {
	fn next(&mut self) -> u64 {
		let mut x = self.0;
		x ^= x << 13;
		x ^= x >> 7;
		x ^= x << 17;
		self.0 = x;
		x
	}
	fn below(&mut self, n: u64) -> u64 {
		self.next() % n
	}
}

// A small vocabulary, so rows collide and repeat often.
const WORDS: [&[u8]; 7] = [b"a", b"b", b"c", b"a\r", b"a ", b"\xff\xfe", b"{\"id\":\"q\"}"];

fn random_file(rng: &mut XorShift) -> Vec<u8> {
	let mut out = Vec::new();
	let n = rng.below(7);
	for _ in 0..n {
		match rng.below(9) {
			0		=> out.push(b'\n'), // A blank line
			_		=> {
				out.extend_from_slice(WORDS[rng.below(WORDS.len() as u64) as usize]);
				out.push(b'\n');
			}
		}
	}
	if !out.is_empty() && rng.below(4) == 0 {
		out.pop(); // No final newline, or a torn last row
	}
	out
}

#[test]
fn seven_oct_both_kept() {
	let shared = concat!(
		"{\"id\":\"t0brekfst001\",\"at\":\"2026-10-07T07:40:12+08:00\",\"day\":\"2026-10-07\",\"src\":\"scale\",\"f\":{\"food\":\"toast\",\"qty\":1},\"w\":1000000000001}\n",
		"{\"id\":\"t0coffee0002\",\"at\":\"2026-10-07T08:02:47+08:00\",\"day\":\"2026-10-07\",\"src\":\"chat\",\"f\":{\"food\":\"coffee\",\"qty\":1},\"w\":1000000000002}\n",
	);
	let oats = "{\"id\":\"t0oats000003\",\"at\":\"2026-10-07T11:06:05+08:00\",\"day\":\"2026-10-07\",\"src\":\"scale\",\"f\":{\"food\":\"oats\",\"qty\":1},\"w\":1000000000003}\n";
	let eggs = "{\"id\":\"t0eggs000004\",\"at\":\"2026-10-07T11:15:55+08:00\",\"day\":\"2026-10-07\",\"src\":\"scale\",\"f\":{\"food\":\"egg\",\"qty\":2},\"w\":1000000000004}\n";
	let arriving = format!("{}{}", shared, oats);
	let here = format!("{}{}", shared, eggs);
	let j = join(&arriving, &here);
	assert_eq!(text(&j), format!("{}{}{}", shared, oats, eggs));
	assert_eq!((j.here_adds, j.arriving_adds), (1, 1));
	// From the other side, both rows are still kept.
	let k = join(&here, &arriving);
	assert_eq!(text(&k), format!("{}{}{}", shared, eggs, oats));
	assert_eq!((k.here_adds, k.arriving_adds), (1, 1));
}

#[test]
fn duplicates_by_occurrence() {
	let j = join("x\nx\n", "x\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("x\nx\n", 0, 1));
	let j = join("x\n", "x\nx\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("x\nx\n", 1, 0));
	let j = join("x\n", "x\nx\nx\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("x\nx\nx\n", 2, 0));
	let j = join("x\nx\nx\n", "x\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("x\nx\nx\n", 0, 2));
	let j = join("x\nx\n", "x\nx\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("x\nx\n", 0, 0));
}

#[test]
fn settles_bytes() {
	for (arriving, here) in [
		("b\na\nc\n",		"a\nc\n"),
		("b\na\nc",			"c\na\n"), // Order and the final newline do not matter
		("\n\na\n\nb\n",	"b\n\n"),
		("a\nb\n",			""),
		("a\r\nb \n",		"b \na\r\n"),
	] {
		let j = join(arriving, here);
		assert_eq!(j.bytes, arriving.as_bytes(), "{:?} against {:?}", arriving, here);
		assert_eq!(j.here_adds, 0);
	}
}

#[test]
fn content_commutative_associative() {
	let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
	for _ in 0..20_000 {
		let a = random_file(&mut rng);
		let b = random_file(&mut rng);
		let c = random_file(&mut rng);
		let (ca, cb, cc) = (content(&a), content(&b), content(&c));

		let ab = union_lines(&a, &b);
		let ba = union_lines(&b, &a);
		let want = max_of(&ca, &cb);
		assert_eq!(content(&ab.bytes), want, "a={:?} b={:?}", a, b);
		assert_eq!(content(&ba.bytes), want, "a={:?} b={:?}", a, b);
		let only_b: usize = cb.iter().map(|(k, n)| n.saturating_sub(*ca.get(k).unwrap_or(&0))).sum();
		let only_a: usize = ca.iter().map(|(k, n)| n.saturating_sub(*cb.get(k).unwrap_or(&0))).sum();
		assert_eq!((ab.here_adds, ab.arriving_adds), (only_b, only_a));

		let left = union_lines(&ab.bytes, &c);
		let bc = union_lines(&b, &c);
		let right = union_lines(&a, &bc.bytes);
		let want3 = max_of(&want, &cc);
		assert_eq!(content(&left.bytes), want3);
		assert_eq!(content(&right.bytes), want3);
	}
}

#[test]
fn idempotent_bytes() {
	let mut rng = XorShift(0x0123_4567_89AB_CDEF);
	for _ in 0..5_000 {
		let a = random_file(&mut rng);
		let h = random_file(&mut rng);
		let aa = union_lines(&a, &a);
		assert_eq!(aa.bytes, a);
		assert_eq!((aa.here_adds, aa.arriving_adds), (0, 0));
		let once = union_lines(&a, &h);
		let twice = union_lines(&once.bytes, &h);
		assert_eq!(twice.bytes, once.bytes, "a={:?} h={:?}", a, h);
		assert_eq!(twice.here_adds, 0);
	}
}

#[test]
fn three_device_gossip_converges() {
	let mut rng = XorShift(0xDEAD_BEEF_CAFE_F00D);
	for _ in 0..2_000 {
		let mut dev = [random_file(&mut rng), random_file(&mut rng), random_file(&mut rng)];
		let want = max_of(&max_of(&content(&dev[0]), &content(&dev[1])), &content(&dev[2]));
		let mut rounds = 0;
		loop {
			let mut changed = false;
			for i in 0..3 {
				for j in 0..3 {
					if i != j {
						let next = union_lines(&dev[j], &dev[i]).bytes; // i pulls from j
						if next != dev[i] {
							dev[i] = next;
							changed = true;
						}
					}
				}
			}
			rounds += 1;
			if !changed {
				break;
			}
			assert!(rounds < 6, "no fixpoint after {} rounds: {:?}", rounds, dev);
		}
		assert_eq!(dev[0], dev[1]);
		assert_eq!(dev[1], dev[2]);
		assert_eq!(content(&dev[0]), want);
	}
}

#[test]
fn r4_order() {
	for (arriving, here, want) in [
		// After the nearest preceding shared row, past arriving's own rows after it.
		("s1\na1\ns2\n",	"s1\nh1\ns2\n",			"s1\na1\nh1\ns2\n"),
		// No shared row before: after the arriving-only rows that open arriving.
		("a0\ns1\n",		"h0\ns1\n",				"a0\nh0\ns1\n"),
		("s1\n",			"h0\ns1\n",				"h0\ns1\n"),
		// The anchor is arriving's last row.
		("s1\na1\n",		"s1\nh1\n",				"s1\na1\nh1\n"),
		// Several here rows in one place keep here's order.
		("s1\ns2\n",		"s1\nh1\nh2\ns2\nh3\n",	"s1\nh1\nh2\ns2\nh3\n"),
		// The anchor is found in arriving wherever it sits there.
		("s2\ns1\n",		"s1\nh1\ns2\n",			"s2\ns1\nh1\n"),
		// The anchor is a row by occurrence: here's second x is arriving's second x.
		("x\na\nx\n",		"x\nx\nh\n",			"x\na\nx\nh\n"),
		// Blank lines after the run stay after the spliced rows.
		("s1\n\ns2\n",		"s1\nh\ns2\n",			"s1\nh\n\ns2\n"),
	] {
		let j = join(arriving, here);
		assert_eq!(text(&j), want, "{:?} against {:?}", arriving, here);
	}
}

#[test]
fn byte_exact() {
	let j = join("a\r\n", "a\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("a\r\na\n", 1, 1));
	let j = join("a \n", "a\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("a \na\n", 1, 1));
	let j = union_lines(b"\xff\xfe\n", b"\xff\n");
	assert_eq!(j.bytes, b"\xff\xfe\n\xff\n".to_vec());
	let j = union_lines(b"\xff\n", b"\xff\n");
	assert_eq!((j.bytes, j.here_adds), (b"\xff\n".to_vec(), 0));
}

#[test]
fn newline_edges() {
	let j = join("", "");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("", 0, 0));
	let j = join("", "x");
	assert_eq!((text(&j).as_str(), j.here_adds), ("x\n", 1));
	let j = join("x", "");
	assert_eq!((text(&j).as_str(), j.arriving_adds), ("x", 1));
	// Arriving lacks a final newline, so one is added before the spliced row.
	let j = join("a", "b\n");
	assert_eq!(text(&j), "a\nb\n");
	// Blank lines are never rows: never spliced, and arriving's kept as they are.
	let j = join("a\n", "\n\na\n\n");
	assert_eq!((text(&j).as_str(), j.here_adds), ("a\n", 0));
	let j = join("\n\n", "x\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("x\n\n\n", 1, 0));
	let j = join("a\n\n\n", "a\nb\n");
	assert_eq!(text(&j), "a\nb\n\n\n");
}

#[test]
fn torn_fragment_is_a_row() {
	// A fragment equal to a whole row is that row.
	let j = join("a\nb", "a\nb\n");
	assert_eq!((text(&j).as_str(), j.here_adds), ("a\nb", 0));
	// A fragment unequal to any row is a row of its own, on either side.
	let j = join("a\n", "a\nbro");
	assert_eq!((text(&j).as_str(), j.here_adds), ("a\nbro\n", 1));
	let j = join("a\nbro", "a\nbrown\n");
	assert_eq!((text(&j).as_str(), j.here_adds, j.arriving_adds), ("a\nbro\nbrown\n", 1, 1));
}

#[test]
fn missing_lines_by_occurrence() {
	let lost = missing_lines(b"x\ny\nx\nx\n", b"x\n");
	assert_eq!(lost, vec![b"y".to_vec(), b"x".to_vec(), b"x".to_vec()]);
	assert!(missing_lines(b"x\nx\n", b"x\nx\nx\n").is_empty());
	assert!(missing_lines(b"\n\n\n", b"").is_empty()); // Blank lines are not rows
	assert_eq!(missing_lines(b"a\r\n", b"a\n"), vec![b"a\r".to_vec()]);
	assert_eq!(missing_lines(b"a\nfrag", b"a\n"), vec![b"frag".to_vec()]);
	assert!(missing_lines(b"", b"a\n").is_empty());
}
