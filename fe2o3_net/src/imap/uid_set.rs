//! A set of IMAP UIDs held as sorted, disjoint, non-adjacent ranges, so that the 10^5 UIDs of a
//! large mailbox cost a few bytes rather than a few hundred kilobytes, and a command line carrying
//! them stays short enough for a server to accept.
//!
//! Its text form is the RFC 3501 `sequence-set` (`1:3,7,9:*`), which is both what goes on the wire
//! and what a server sends back in `COPYUID`, `APPENDUID` and `VANISHED`. A range ending in `*`
//! is held as ending at `u32::MAX`, which is what `*` means for a set no larger than the mailbox;
//! a caller asking for `n:*` must still drop any answer below `n`, because a server answers
//! `n:*` past its last UID with that last message (RFC 3501 6.4.8).
//!
//! Tokio-free, so the sync engine can hold one in a browser.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;


/// The largest UID; `*` in a set.
pub const UID_STAR: u32 = u32::MAX;

#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct UidSet {
    ranges: Vec<(u32, u32)>,    // inclusive, sorted, disjoint and never touching
}

impl UidSet {

    pub fn new() -> Self {
        Self::default()
    }

    /// Every UID from `first` on, `first:*` on the wire.
    pub fn from_open(first: u32) -> Self {
        let mut s = Self::new();
        s.insert_range(first, UID_STAR);
        s
    }

    pub fn from_range(first: u32, last: u32) -> Self {
        let mut s = Self::new();
        s.insert_range(first, last);
        s
    }

    /// Parses a `sequence-set`: comma-separated numbers and `a:b` ranges, either end of which may
    /// be `*`, in either order. Zero is not a UID and is refused.
    pub fn parse(text: &str) -> Outcome<Self> {
        let mut s = Self::new();
        for (a, b) in res!(parse_items(text)) {
            s.insert_range(a, b);
        }
        Ok(s)
    }

    pub fn insert(&mut self, uid: u32) {
        self.insert_range(uid, uid);
    }

    /// Either order; a range touching or overlapping others is merged with them.
    pub fn insert_range(&mut self, a: u32, b: u32) {
        let (mut lo, mut hi) = if a <= b { (a, b) } else { (b, a) };
        // The first range that could touch [lo, hi]: its end is at least lo - 1.
        let start = self.ranges.partition_point(|&(_, e)| e.saturating_add(1) < lo);
        let mut end = start;
        while end < self.ranges.len() && self.ranges[end].0 <= hi.saturating_add(1) {
            lo = lo.min(self.ranges[end].0);
            hi = hi.max(self.ranges[end].1);
            end += 1;
        }
        self.ranges.splice(start..end, std::iter::once((lo, hi)));
    }

    pub fn remove(&mut self, uid: u32) {
        let i = self.ranges.partition_point(|&(_, e)| e < uid);
        if i >= self.ranges.len() || self.ranges[i].0 > uid {
            return;
        }
        let (lo, hi) = self.ranges[i];
        let mut parts = Vec::with_capacity(2);
        if lo < uid { parts.push((lo, uid - 1)); }
        if uid < hi { parts.push((uid + 1, hi)); }
        self.ranges.splice(i..i + 1, parts);
    }

    pub fn contains(&self, uid: u32) -> bool {
        let i = self.ranges.partition_point(|&(_, e)| e < uid);
        i < self.ranges.len() && self.ranges[i].0 <= uid
    }

    /// How many UIDs, counting an open range up to `u32::MAX`.
    pub fn len(&self) -> u64 {
        self.ranges.iter().map(|&(a, b)| (b - a) as u64 + 1).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn ranges(&self) -> &[(u32, u32)] {
        &self.ranges
    }

    pub fn first(&self) -> Option<u32> {
        self.ranges.first().map(|r| r.0)
    }

    pub fn last(&self) -> Option<u32> {
        self.ranges.last().map(|r| r.1)
    }

    /// Ascending. An open range runs to `u32::MAX`, so bound it before iterating.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.ranges.iter().flat_map(|&(a, b)| a..=b)
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut out = self.clone();
        for &(a, b) in &other.ranges {
            out.insert_range(a, b);
        }
        out
    }

    /// What is in `self` and not in `other`: the held UIDs a server no longer reports, say.
    pub fn difference(&self, other: &Self) -> Self {
        let mut out = Vec::new();
        let mut j = 0usize;
        for &(a, b) in &self.ranges {
            let mut lo = a;
            while j < other.ranges.len() && other.ranges[j].1 < lo {
                j += 1;
            }
            let mut k = j;
            let mut done = false;
            while k < other.ranges.len() && other.ranges[k].0 <= b {
                let (oa, ob) = other.ranges[k];
                if oa > lo {
                    out.push((lo, oa - 1));
                }
                if ob >= b {
                    done = true;
                    break;
                }
                lo = ob + 1;
                k += 1;
            }
            if !done {
                out.push((lo, b));
            }
        }
        Self { ranges: out }
    }

    /// What is in both.
    pub fn intersection(&self, other: &Self) -> Self {
        let mut out = Vec::new();
        let (mut i, mut j) = (0usize, 0usize);
        while i < self.ranges.len() && j < other.ranges.len() {
            let (a, b) = self.ranges[i];
            let (c, d) = other.ranges[j];
            let lo = a.max(c);
            let hi = b.min(d);
            if lo <= hi {
                out.push((lo, hi));
            }
            if b < d { i += 1; } else { j += 1; }
        }
        Self { ranges: out }
    }
}

impl fmt::Display for UidSet {
    /// The wire form; an empty set writes nothing, which no command accepts, so a caller sends no
    /// command for one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let end = |n: u32| if n == UID_STAR { fmt!("*") } else { fmt!("{}", n) };
        let mut out = String::new();
        for (i, &(a, b)) in self.ranges.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if a == b {
                out.push_str(&end(a));
            } else {
                out.push_str(&fmt!("{}:{}", a, end(b)));
            }
        }
        f.write_str(&out)
    }
}

impl From<&[u32]> for UidSet {
    fn from(uids: &[u32]) -> Self {
        uids.iter().copied().collect()
    }
}

impl<const N: usize> From<[u32; N]> for UidSet {
    fn from(uids: [u32; N]) -> Self {
        uids.into_iter().collect()
    }
}

impl From<&Vec<u32>> for UidSet {
    fn from(uids: &Vec<u32>) -> Self {
        uids.iter().copied().collect()
    }
}

impl FromIterator<u32> for UidSet {
    fn from_iter<I: IntoIterator<Item = u32>>(it: I) -> Self {
        let mut v: Vec<u32> = it.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for u in v {
            match ranges.last_mut() {
                Some(r) if r.1.checked_add(1) == Some(u) => r.1 = u,
                _                                         => ranges.push((u, u)),
            }
        }
        Self { ranges }
    }
}

/// The UIDs of a `sequence-set` in the order written, each range expanded in its written
/// direction. This is how `COPYUID` pairs a source UID with its destination: by position. A `*`
/// has no number to expand to and is refused, and so is a list longer than `max`.
pub fn uid_list(text: &str, max: usize) -> Outcome<Vec<u32>> {
    let mut out = Vec::new();
    for (a, b) in res!(parse_items_ordered(text)) {
        if a == UID_STAR || b == UID_STAR {
            return Err(err!("The UID list '{}' holds a '*', which names no one UID.", text;
                Invalid, Input, Decode));
        }
        let n = (a.max(b) - a.min(b)) as usize + 1;
        if out.len() + n > max {
            return Err(err!("The UID list '{}' holds more than {} UIDs.", text, max;
                Invalid, Input, Excessive));
        }
        if a <= b {
            out.extend(a..=b);
        } else {
            out.extend((b..=a).rev());
        }
    }
    Ok(out)
}

/// Each item as `(first, last)`, normalised to ascending.
fn parse_items(text: &str) -> Outcome<Vec<(u32, u32)>> {
    Ok(res!(parse_items_ordered(text)).into_iter()
        .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
        .collect())
}

/// Each item as `(first, last)` exactly as written.
fn parse_items_ordered(text: &str) -> Outcome<Vec<(u32, u32)>> {
    let text = text.trim();
    if text.is_empty() {
        return Err(err!("An empty UID set."; Invalid, Input, Decode));
    }
    let mut out = Vec::new();
    for item in text.split(',') {
        let (a, b) = match item.split_once(':') {
            Some((a, b)) => (res!(one_uid(a, text)), res!(one_uid(b, text))),
            None         => { let n = res!(one_uid(item, text)); (n, n) },
        };
        out.push((a, b));
    }
    Ok(out)
}

fn one_uid(s: &str, whole: &str) -> Outcome<u32> {
    if s == "*" {
        return Ok(UID_STAR);
    }
    match s.parse::<u32>() {
        Ok(0)  => Err(err!("Zero is not a UID, in '{}'.", whole; Invalid, Input, Decode)),
        Ok(n)  => Ok(n),
        Err(_) => Err(err!("'{}' is not a UID, in '{}'.", s, whole; Invalid, Input, Decode)),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str) -> Outcome<UidSet> {
        UidSet::parse(text)
    }

    #[test]
    fn test_runs_collapse_and_print_as_the_wire_form_00() -> Outcome<()> {
        let s: UidSet = [10u32, 1, 2, 3, 7, 9, 3].into_iter().collect();
        req!("1:3,7,9:10", s.to_string().as_str());
        req!(6, s.len());
        req!("5:*", UidSet::from_open(5).to_string().as_str());
        req!("", UidSet::new().to_string().as_str());
        Ok(())
    }

    #[test]
    fn test_parse_merges_reversed_overlapping_and_touching_ranges_00() -> Outcome<()> {
        let s = res!(set("9:7,1,2:4,5,20:*"));
        req!(vec![(1, 5), (7, 9), (20, UID_STAR)], s.ranges().to_vec());
        req!(true, UidSet::parse("0").is_err(), "zero was taken for a UID");
        req!(true, UidSet::parse("1,,2").is_err());
        req!(true, UidSet::parse("").is_err());
        Ok(())
    }

    #[test]
    fn test_insert_and_remove_keep_the_ranges_canonical_00() -> Outcome<()> {
        let mut s = res!(set("1:3,7:9"));
        s.insert(5);
        req!("1:3,5,7:9", s.to_string().as_str());
        s.insert(4);
        s.insert(6);
        req!("1:9", s.to_string().as_str());
        s.remove(5);
        s.remove(1);
        s.remove(9);
        s.remove(42);
        req!("2:4,6:8", s.to_string().as_str());
        req!(true, s.contains(3) && !s.contains(5) && !s.contains(9));
        s.insert_range(UID_STAR, 1);
        req!("1:*", s.to_string().as_str());
        Ok(())
    }

    #[test]
    fn test_difference_is_what_vanished_00() -> Outcome<()> {
        let held = res!(set("1:10,20:30"));
        let now  = res!(set("2:4,6,10:21,25"));
        req!("1,5,7:9,22:24,26:30", held.difference(&now).to_string().as_str());
        req!("", held.difference(&held).to_string().as_str());
        req!(held.to_string(), held.difference(&UidSet::new()).to_string());
        req!("2:4,6,10,20:21,25", held.intersection(&now).to_string().as_str());
        req!("1:30", held.union(&res!(set("11:19"))).to_string().as_str());
        Ok(())
    }

    #[test]
    fn test_a_uid_list_keeps_its_written_order_00() -> Outcome<()> {
        req!(vec![304, 319, 320, 5, 4], res!(uid_list("304,319:320,5:4", 100)));
        req!(true, uid_list("1:*", 100).is_err(), "a star was expanded");
        req!(true, uid_list("1:1000", 100).is_err(), "the bound was not kept");
        Ok(())
    }
}
