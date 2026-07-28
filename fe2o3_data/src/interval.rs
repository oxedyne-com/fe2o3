//! An ordered map from disjoint half-open integer intervals to values.
//!
//! An [`IntervalMap`] partitions a subset of the `u64` line into non-overlapping
//! half-open intervals `[start, end)`, each carrying one value. The map is the
//! natural shape for interval bookkeeping: recording which stretches of an index
//! space are spoken for, and by what.
//!
//! Insertion is last-writer-wins. A new interval displaces whatever occupied the
//! ground it covers, splitting the intervals it partly overlaps so that only the
//! uncovered remainders survive. The invariant that no two intervals overlap is
//! therefore maintained by construction rather than checked afterwards.
//!
//! Adjacent intervals carrying equal values are coalesced, so the map never
//! holds two neighbours that a reader could not tell apart. Representation is
//! thus canonical for a given covering: two maps built by different sequences of
//! insertions compare equal whenever they describe the same covering.

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeMap;
use std::ops::Range;


/// An ordered map from disjoint half-open `u64` intervals to values.
///
/// Entries are keyed by interval start and ordered by it. No two intervals in
/// the map overlap, and no two adjacent intervals carry equal values.
///
/// # Type Parameters
///
/// * `V` - The value carried by an interval. `Clone` is needed to split an
///   interval in two, and `PartialEq` to decide whether neighbours coalesce.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntervalMap<V> {
	/// Start of each interval mapped to its exclusive end and its value.
	map: BTreeMap<u64, (u64, V)>,
}

impl<V> Default for IntervalMap<V> {
	fn default() -> Self {
		Self { map: BTreeMap::new() }
	}
}

impl<V> IntervalMap<V> {

	/// Constructs an empty map.
	pub fn new() -> Self {
		Self { map: BTreeMap::new() }
	}

	/// Returns the number of intervals in the map.
	///
	/// This counts intervals, not the points they cover, and coalescing can
	/// lower it: inserting into a map of two intervals may leave one.
	pub fn len(&self) -> usize {
		self.map.len()
	}

	/// Reports whether the map holds no intervals.
	pub fn is_empty(&self) -> bool {
		self.map.is_empty()
	}

	/// Removes every interval, leaving the map empty.
	pub fn clear(&mut self) {
		self.map.clear();
	}

	/// Returns the value covering `point`, if any.
	///
	/// The interval `[s, e)` covers `point` when `s <= point < e`, so the value
	/// is returned at an interval's start but not at its end.
	pub fn get(&self, point: u64)
		-> Option<&V>
	{
		self.entry(point).map(|(_, val)| val)
	}

	/// Returns the interval covering `point` together with its value.
	pub fn entry(&self, point: u64)
		-> Option<(Range<u64>, &V)>
	{
		match self.map.range(..=point).next_back() {
			Some((start, (end, val))) if *end > point =>
				Some((*start..*end, val)),
			_ => None,
		}
	}

	/// Reports whether any interval covers `point`.
	pub fn contains(&self, point: u64) -> bool {
		self.get(point).is_some()
	}

	/// Iterates over the intervals in ascending order of start.
	pub fn iter(&self)
		-> impl Iterator<Item = (Range<u64>, &V)>
	{
		self.map.iter().map(|(start, (end, val))| (*start..*end, val))
	}

	/// Iterates over the intervals overlapping `range`, in ascending order of
	/// start.
	///
	/// Intervals are yielded as they are stored, so the first and last may reach
	/// beyond `range`; a caller wanting only the covered ground clips them. An
	/// empty or reversed `range` overlaps nothing and yields nothing.
	///
	/// # Examples
	///
	/// ```
	/// use oxedyne_fe2o3_data::interval::IntervalMap;
	///
	/// let mut map: IntervalMap<char> = IntervalMap::new();
	/// assert!(map.insert(0..10, 'a').is_ok());
	/// assert!(map.insert(20..30, 'b').is_ok());
	///
	/// let hit: Vec<char> = map.overlapping(5..25).map(|(_, v)| *v).collect();
	/// assert_eq!(hit, vec!['a', 'b']);
	/// assert_eq!(map.overlapping(10..20).count(), 0);
	/// ```
	pub fn overlapping(&self, range: Range<u64>)
		-> impl Iterator<Item = (Range<u64>, &V)>
	{
		let (start, end) = (range.start, range.end);
		let live = end > start;
		// At most one interval begins before the range and reaches into it; the
		// rest begin inside it.
		let head = match self.map.range(..start).next_back() {
			Some((s, (e, val))) if live && *e > start	=> Some((*s..*e, val)),
			_						=> None,
		};
		let hi = if live { end } else { start };
		head.into_iter()
			.chain(self.map.range(start..hi).map(|(s, (e, val))| (*s..*e, val)))
	}
}

impl<V: Clone + PartialEq> IntervalMap<V> {

	/// Inserts `val` over `range`, overwriting whatever it covers.
	///
	/// The contract is last-writer-wins over the covered ground, and nothing
	/// else. Every point in `range` afterwards maps to `val`; every point
	/// outside it keeps the value it had. An existing interval that `range`
	/// covers entirely is removed; one that `range` covers in part is split, and
	/// only the uncovered remainder survives, carrying a clone of the original
	/// value.
	///
	/// Once the ground is taken, the new interval is coalesced with either
	/// neighbour that abuts it and carries an equal value, so the map stays
	/// canonical.
	///
	/// Fails when the range is empty, that is, when `end <= start`. An empty
	/// range would name no ground and so could not be an intelligible claim on
	/// any.
	///
	/// # Examples
	///
	/// ```
	/// use oxedyne_fe2o3_data::interval::IntervalMap;
	///
	/// let mut map: IntervalMap<char> = IntervalMap::new();
	/// assert!(map.insert(0..100, 'a').is_ok());
	/// assert!(map.insert(40..60, 'b').is_ok());
	///
	/// // The later write holds the middle, and the first survives either side.
	/// assert_eq!(map.get(39), Some(&'a'));
	/// assert_eq!(map.get(40), Some(&'b'));
	/// assert_eq!(map.get(60), Some(&'a'));
	/// assert_eq!(map.len(), 3);
	/// ```
	pub fn insert(&mut self, range: Range<u64>, val: V)
		-> Outcome<()>
	{
		let (start, end) = (range.start, range.end);
		if end <= start {
			return Err(err!(
				"The interval [{}, {}) is empty; a range must satisfy start < end.",
				start, end;
			Invalid, Input, Range));
		}
		self.vacate(start, end);
		self.map.insert(start, (end, val));
		self.coalesce(start);
		Ok(())
	}

	/// Removes every interval, and part-interval, lying within `range`.
	///
	/// Points inside `range` are afterwards covered by nothing; points outside
	/// it are untouched, an interval straddling an edge being split rather than
	/// dropped. Fails on an empty range, as [`IntervalMap::insert`] does.
	pub fn remove(&mut self, range: Range<u64>)
		-> Outcome<()>
	{
		let (start, end) = (range.start, range.end);
		if end <= start {
			return Err(err!(
				"The interval [{}, {}) is empty; a range must satisfy start < end.",
				start, end;
			Invalid, Input, Range));
		}
		self.vacate(start, end);
		Ok(())
	}

	/// Clears `[start, end)`, splitting any interval that straddles an edge.
	///
	/// Leaves the map free of any coverage inside the range, and unchanged
	/// outside it.
	fn vacate(&mut self, start: u64, end: u64) {
		// The starts of every interval overlapping the range. At most one
		// begins before the range and reaches into it; the rest begin inside.
		let mut hits = Vec::new();
		if let Some((s, (e, _))) = self.map.range(..start).next_back() {
			if *e > start {
				hits.push(*s);
			}
		}
		for (s, _) in self.map.range(start..end) {
			hits.push(*s);
		}
		for s in hits {
			// Take the interval out, then put back whatever falls outside the
			// range. A hit straddling both edges yields two remainders.
			if let Some((e, val)) = self.map.remove(&s) {
				if s < start {
					self.map.insert(s, (start, val.clone()));
				}
				if e > end {
					self.map.insert(end, (e, val));
				}
			}
		}
	}

	/// Merges the interval starting at `start` with any abutting equal
	/// neighbour.
	///
	/// Does nothing if no interval starts there.
	fn coalesce(&mut self, start: u64) {
		let (end, val) = match self.map.get(&start) {
			Some((e, v))	=> (*e, v.clone()),
			None			=> return,
		};
		let mut lo = start;
		let mut hi = end;
		// A left neighbour abuts when its end is this interval's start.
		if let Some((s, (e, v))) = self.map.range(..start).next_back() {
			if *e == start && *v == val {
				lo = *s;
			}
		}
		// A right neighbour abuts when it is keyed at this interval's end.
		if let Some((e, v)) = self.map.get(&end) {
			if *v == val {
				hi = *e;
			}
		}
		if lo != start {
			self.map.remove(&lo);
		}
		if hi != end {
			self.map.remove(&end);
		}
		self.map.remove(&start);
		self.map.insert(lo, (hi, val));
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ TESTS                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
	use super::*;

	/// Collects the map into a comparable form, so a test can state the whole
	/// expected covering in one line.
	fn dump(map: &IntervalMap<char>) -> Vec<(u64, u64, char)> {
		map.iter().map(|(r, v)| (r.start, r.end, *v)).collect()
	}

	#[test]
	fn test_a_new_map_is_empty_00() {
		let map: IntervalMap<char> = IntervalMap::new();
		assert!(map.is_empty());
		assert_eq!(map.len(), 0);
		assert_eq!(map.get(0), None);
		assert_eq!(map.get(u64::MAX), None);
	}

	#[test]
	fn test_an_empty_range_is_refused_00() {
		let mut map: IntervalMap<char> = IntervalMap::new();
		assert!(map.insert(5..5, 'a').is_err(),
			"a zero-width range names no ground");
		assert!(map.insert(9..5, 'a').is_err(),
			"a reversed range names no ground");
		assert!(map.remove(5..5).is_err());
		assert!(map.is_empty(),
			"a refused insertion must leave the map untouched");
	}

	#[test]
	fn test_a_point_query_respects_both_boundaries_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(10..20, 'a').is_ok());
		assert_eq!(map.get(9), None,	"the point before the start is outside");
		assert_eq!(map.get(10), Some(&'a'), "the start is inside");
		assert_eq!(map.get(15), Some(&'a'));
		assert_eq!(map.get(19), Some(&'a'), "the last covered point is inside");
		assert_eq!(map.get(20), None,	"the end is exclusive");
		assert_eq!(map.get(21), None);
		assert!(map.contains(10));
		assert!(!map.contains(20));
	}

	#[test]
	fn test_entry_reports_the_covering_interval_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(10..20, 'a').is_ok());
		assert_eq!(map.entry(14), Some((10..20, &'a')));
		assert_eq!(map.entry(20), None);
	}

	#[test]
	fn test_disjoint_insertions_are_kept_apart_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(20..30, 'b').is_ok());
		assert!(map.insert(10..20, 'c').is_ok());
		assert_eq!(dump(&map), vec![(0, 10, 'a'), (10, 20, 'c'), (20, 30, 'b')]);
		assert_eq!(map.get(15), Some(&'c'));
	}

	#[test]
	fn test_an_overwrite_wholly_inside_splits_in_three_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..100, 'a').is_ok());
		assert!(map.insert(40..60, 'b').is_ok());
		assert_eq!(dump(&map), vec![(0, 40, 'a'), (40, 60, 'b'), (60, 100, 'a')]);
		assert_eq!(map.get(39), Some(&'a'));
		assert_eq!(map.get(40), Some(&'b'));
		assert_eq!(map.get(59), Some(&'b'));
		assert_eq!(map.get(60), Some(&'a'));
	}

	#[test]
	fn test_an_overwrite_spanning_an_interval_replaces_it_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(40..60, 'a').is_ok());
		assert!(map.insert(0..100, 'b').is_ok());
		assert_eq!(dump(&map), vec![(0, 100, 'b')]);
		assert_eq!(map.get(50), Some(&'b'));
	}

	#[test]
	fn test_an_overwrite_on_the_left_edge_trims_the_start_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(10..20, 'a').is_ok());
		assert!(map.insert(5..15, 'b').is_ok());
		assert_eq!(dump(&map), vec![(5, 15, 'b'), (15, 20, 'a')]);
		assert_eq!(map.get(14), Some(&'b'));
		assert_eq!(map.get(15), Some(&'a'));
	}

	#[test]
	fn test_an_overwrite_on_the_right_edge_trims_the_end_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(10..20, 'a').is_ok());
		assert!(map.insert(15..25, 'b').is_ok());
		assert_eq!(dump(&map), vec![(10, 15, 'a'), (15, 25, 'b')]);
		assert_eq!(map.get(14), Some(&'a'));
		assert_eq!(map.get(15), Some(&'b'));
		assert_eq!(map.get(24), Some(&'b'));
		assert_eq!(map.get(25), None);
	}

	#[test]
	fn test_an_exact_overwrite_replaces_the_value_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(10..20, 'a').is_ok());
		assert!(map.insert(10..20, 'b').is_ok());
		assert_eq!(dump(&map), vec![(10, 20, 'b')]);
		assert_eq!(map.len(), 1);
	}

	#[test]
	fn test_an_overwrite_across_several_intervals_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(10..20, 'b').is_ok());
		assert!(map.insert(20..30, 'c').is_ok());
		assert!(map.insert(30..40, 'd').is_ok());
		// Straddles the first, swallows the middle two, straddles the last.
		assert!(map.insert(5..35, 'z').is_ok());
		assert_eq!(dump(&map), vec![(0, 5, 'a'), (5, 35, 'z'), (35, 40, 'd')]);
		assert_eq!(map.get(4), Some(&'a'));
		assert_eq!(map.get(5), Some(&'z'));
		assert_eq!(map.get(34), Some(&'z'));
		assert_eq!(map.get(35), Some(&'d'));
	}

	#[test]
	fn test_an_overwrite_covering_everything_leaves_one_interval_00() {
		let mut map = IntervalMap::new();
		for i in 0..10u64 {
			assert!(map.insert(i * 10..(i + 1) * 10, 'a').is_ok());
		}
		// Ten equal-valued neighbours have already coalesced into one.
		assert_eq!(dump(&map), vec![(0, 100, 'a')]);
		assert!(map.insert(0..100, 'b').is_ok());
		assert_eq!(dump(&map), vec![(0, 100, 'b')]);
	}

	#[test]
	fn test_an_abutting_equal_neighbour_coalesces_on_the_left_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(10..20, 'a').is_ok());
		assert_eq!(dump(&map), vec![(0, 20, 'a')]);
		assert_eq!(map.len(), 1);
	}

	#[test]
	fn test_an_abutting_equal_neighbour_coalesces_on_the_right_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(10..20, 'a').is_ok());
		assert!(map.insert(0..10, 'a').is_ok());
		assert_eq!(dump(&map), vec![(0, 20, 'a')]);
	}

	#[test]
	fn test_an_insertion_between_two_equals_coalesces_both_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(20..30, 'a').is_ok());
		assert_eq!(map.len(), 2, "the two are not yet adjacent");
		assert!(map.insert(10..20, 'a').is_ok());
		assert_eq!(dump(&map), vec![(0, 30, 'a')]);
	}

	#[test]
	fn test_an_unequal_neighbour_does_not_coalesce_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(10..20, 'b').is_ok());
		assert!(map.insert(20..30, 'a').is_ok());
		assert_eq!(dump(&map), vec![(0, 10, 'a'), (10, 20, 'b'), (20, 30, 'a')]);
	}

	#[test]
	fn test_a_split_remainder_coalesces_with_the_new_value_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..100, 'a').is_ok());
		// The write agrees with what it partly covers, so nothing is divided.
		assert!(map.insert(40..60, 'a').is_ok());
		assert_eq!(dump(&map), vec![(0, 100, 'a')]);
		// And a write past the end simply extends it.
		assert!(map.insert(90..120, 'a').is_ok());
		assert_eq!(dump(&map), vec![(0, 120, 'a')]);
	}

	#[test]
	fn test_the_representation_is_canonical_00() {
		let mut a = IntervalMap::new();
		assert!(a.insert(0..30, 'x').is_ok());
		let mut b = IntervalMap::new();
		assert!(b.insert(0..10, 'x').is_ok());
		assert!(b.insert(10..20, 'x').is_ok());
		assert!(b.insert(20..30, 'x').is_ok());
		assert_eq!(a, b,
			"the same covering reached two ways must compare equal");
	}

	#[test]
	fn test_remove_clears_the_range_and_splits_the_edges_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..100, 'a').is_ok());
		assert!(map.remove(40..60).is_ok());
		assert_eq!(dump(&map), vec![(0, 40, 'a'), (60, 100, 'a')]);
		assert_eq!(map.get(39), Some(&'a'));
		assert_eq!(map.get(40), None);
		assert_eq!(map.get(59), None);
		assert_eq!(map.get(60), Some(&'a'));
	}

	#[test]
	fn test_clear_empties_the_map_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(20..30, 'b').is_ok());
		map.clear();
		assert!(map.is_empty());
		assert_eq!(map.get(5), None);
	}

	#[test]
	fn test_iteration_is_in_ascending_order_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(50..60, 'c').is_ok());
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(20..30, 'b').is_ok());
		let starts: Vec<u64> = map.iter().map(|(r, _)| r.start).collect();
		assert_eq!(starts, vec![0, 20, 50]);
	}

	/// Checks the map against an independent model: a plain array of points,
	/// one slot per position, which cannot get splitting or coalescing wrong
	/// because it does neither. Any disagreement is the map's.
	#[test]
	fn test_the_map_agrees_with_a_point_by_point_model_00() {
		const N: u64 = 64;
		let mut map: IntervalMap<char> = IntervalMap::new();
		let mut model = [None::<char>; N as usize];
		// A small linear congruential generator, so the sequence is fixed and a
		// failure can be reproduced.
		let mut seed = 0x2545_F491_4F6C_DD1Du64;
		let mut next = || {
			seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			seed >> 33
		};
		for step in 0..2_000 {
			let a = next() % N;
			let b = next() % N;
			let (lo, hi) = if a <= b { (a, b + 1) } else { (b, a + 1) };
			let hi = hi.min(N);
			// Three values only, so coalescing is exercised often.
			let val = [b'x', b'y', b'z'][(next() % 3) as usize] as char;
			let removing = next() % 4 == 0;
			if removing {
				assert!(map.remove(lo..hi).is_ok(), "step {}", step);
				for p in lo..hi {
					model[p as usize] = None;
				}
			} else {
				assert!(map.insert(lo..hi, val).is_ok(), "step {}", step);
				for p in lo..hi {
					model[p as usize] = Some(val);
				}
			}
			for p in 0..N {
				assert_eq!(map.get(p).copied(), model[p as usize],
					"step {}, point {}", step, p);
			}
			// No two intervals may overlap, abut with equal values, or be empty.
			let mut prev: Option<(u64, u64, char)> = None;
			for (r, v) in map.iter() {
				assert!(r.start < r.end, "step {}: an empty interval survived", step);
				if let Some((_, pe, pv)) = prev {
					assert!(pe <= r.start, "step {}: intervals overlap", step);
					assert!(pe < r.start || pv != *v,
						"step {}: equal neighbours were left uncoalesced", step);
				}
				prev = Some((r.start, r.end, *v));
			}
		}
	}

	#[test]
	fn test_overlapping_yields_only_what_it_touches_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..10, 'a').is_ok());
		assert!(map.insert(20..30, 'b').is_ok());
		assert!(map.insert(40..50, 'c').is_ok());
		let hit = |r: Range<u64>| -> Vec<(u64, u64, char)> {
			map.overlapping(r).map(|(i, v)| (i.start, i.end, *v)).collect()
		};
		assert_eq!(hit(0..50), vec![(0, 10, 'a'), (20, 30, 'b'), (40, 50, 'c')]);
		assert_eq!(hit(10..20), vec![],
			"the gap between two intervals overlaps neither");
		assert_eq!(hit(5..25), vec![(0, 10, 'a'), (20, 30, 'b')],
			"an interval reaching into the range from the left is included whole");
		assert_eq!(hit(9..10), vec![(0, 10, 'a')]);
		assert_eq!(hit(10..11), vec![], "the end of an interval is exclusive");
		assert_eq!(hit(29..31), vec![(20, 30, 'b')]);
		assert_eq!(hit(30..31), vec![]);
	}

	#[test]
	fn test_overlapping_refuses_to_be_confused_by_an_empty_range_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..100, 'a').is_ok());
		assert_eq!(map.overlapping(50..50).count(), 0,
			"a zero-width range covers no ground");
		assert_eq!(map.overlapping(60..50).count(), 0,
			"a reversed range covers no ground");
	}

	#[test]
	fn test_the_extremes_of_the_line_are_covered_00() {
		let mut map = IntervalMap::new();
		assert!(map.insert(0..1, 'a').is_ok());
		assert!(map.insert(u64::MAX - 1..u64::MAX, 'b').is_ok());
		assert_eq!(map.get(0), Some(&'a'));
		assert_eq!(map.get(1), None);
		assert_eq!(map.get(u64::MAX - 1), Some(&'b'));
		assert_eq!(map.get(u64::MAX), None,
			"the end is exclusive even at the top of the range");
	}
}
