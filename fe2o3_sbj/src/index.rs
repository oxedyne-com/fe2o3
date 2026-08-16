//! The optional trailing index: node id to byte offset. Derived, outside the hash. See §1.4.
//!
//! BDAT puts a byte length in front of every compound, so a tree can be walked without being
//! decoded. The walk here reads only what it must: the fixed header of each node, the declared
//! payload length of each map and list, and the map key that names a node's children. Everything
//! else is skipped whole with `Dat::count_bytes`, which seeks over a daticle rather than building
//! one. Nothing on the way is turned into a `Dat`.
//!
//! The index is derived data living outside the hash, so it is never trusted. What an index offset
//! points at is checked before it is used: `node_at` confines a lookup to the tree region and
//! refuses anything that does not decode as a node, and `check` compares a whole index against the
//! tree it claims to describe, which costs one walk and no decoding.

use crate::{
	canon,
	kinds::KEY_CHILDREN,
	limit,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	bdat::DecodeLimits,
	usr::UsrKindId,
};

use std::{
	collections::BTreeMap,
	io::Cursor,
};

/// The daticle nesting depth an index reaches: the map itself, then the scalar under each key.
///
/// An index is flat, so anything deeper is not an index, and a reader that decodes an index it did
/// not build says so rather than descending into it.
pub const INDEX_DAT_DEPTH: usize = 2;

/// The limits an index region is decoded under.
///
/// An index names at most one offset per node, so a tree at the size limit of §5 indexes to well
/// under that limit again, and the byte bound is the tree's. The bound matters because the index
/// region trails the hashed part of the file: its length is whatever the file says it is, and
/// nothing has vouched for a byte of it.
fn decode_limits() -> DecodeLimits {
	DecodeLimits::new(INDEX_DAT_DEPTH, limit::TREE_BYTES)
}

/// Builds an index over an encoded tree region.
pub fn build(tree_bytes: &[u8]) -> Outcome<Vec<u8>> {
	let offs = res!(offsets(tree_bytes));
	let mut map = DaticleMap::new();
	for (id, off) in &offs {
		map.insert(Dat::C64(*id), Dat::C64(*off));
	}
	Ok(res!(Dat::Map(map).to_bytes(Vec::new())))
}

/// Parses an index, returning node id to byte offset.
///
/// The bytes lie outside the hash and outside the signature, so they are decoded under the limits
/// of [`decode_limits`] rather than on trust.
pub fn parse(buf: &[u8]) -> Outcome<BTreeMap<u64, u64>> {
	let (dat, n) = res!(Dat::from_bytes_limited(buf, &decode_limits()));
	if n != buf.len() {
		return Err(err!(
			"The index is {} bytes but its map ends at byte {}, leaving {} trailing bytes.",
			buf.len(), n, buf.len() - n;
		Invalid, Input, Decode));
	}
	let map = match dat {
		Dat::Map(map) => map,
		d => return Err(err!(
			"An index must be a Dat::Map of node id to byte offset, found a {:?}.", d.kind();
		Invalid, Input, Decode)),
	};
	let mut out = BTreeMap::new();
	for (k, v) in map {
		let id = match k {
			Dat::C64(id) => id,
			k => return Err(err!(
				"An index key must be a c64 node id, found a {:?}.", k.kind();
			Invalid, Input, Decode)),
		};
		let off = match v {
			Dat::C64(off) => off,
			v => return Err(err!(
				"The index entry for node {} must be a c64 byte offset, found a {:?}.",
				id, v.kind();
			Invalid, Input, Decode)),
		};
		out.insert(id, off);
	}
	Ok(out)
}

/// Walks an encoded tree region, returning the true node id to byte offset map.
///
/// Ids are assigned in a depth-first, pre-order walk, counting from 0 at the root (§4.3), and an
/// offset is measured from the start of the tree region.
pub fn offsets(tree_bytes: &[u8]) -> Outcome<BTreeMap<u64, u64>> {
	let mut out = BTreeMap::new();
	let mut next: u64 = 0;
	let end = res!(walk_node(tree_bytes, 0, 0, &mut next, &mut out));
	if end != tree_bytes.len() {
		return Err(err!(
			"The tree region is {} bytes but its root node ends at byte {}, leaving {} \
			trailing bytes.", tree_bytes.len(), end, tree_bytes.len() - end;
		Invalid, Input, Decode));
	}
	Ok(out)
}

/// Decodes the node an untrusted index offset points at, refusing anything that is not one.
///
/// The offset is confined to the tree region and the daticle at it must decode as a node, so a
/// corrupt offset yields an error rather than a read beyond the region or a panic. The guarantee is
/// confinement, not identity: to establish that an index names the nodes it claims to, check it
/// against the tree with `check`.
pub fn node_at(
	tree_bytes:	&[u8],
	off:		u64,
)
	-> Outcome<Dat>
{
	let start = try_into!(usize, off);
	if start >= tree_bytes.len() {
		return Err(err!(
			"An index offset of byte {} lies at or past the end of the {} byte tree region.",
			start, tree_bytes.len();
		Invalid, Input, Index));
	}
	if tree_bytes[start] != Dat::USR_CODE {
		return Err(err!(
			"An index offset of byte {} does not begin a node: a node starts with the usr code \
			{}, but the byte there is {}.", start, Dat::USR_CODE, tree_bytes[start];
		Invalid, Input, Index));
	}
	// The daticle at the offset is decoded under the tree's own limits, since an offset that lands
	// inside a node, or on bytes an index built elsewhere invented, is not to be descended into
	// without a bound on how far it goes.
	let (dat, _len) = res!(Dat::from_bytes_limited(
		&tree_bytes[start..],
		&canon::decode_limits(),
	));
	match dat {
		Dat::Usr(..) => Ok(dat),
		d => Err(err!(
			"An index offset of byte {} decodes to a {:?} rather than a node.",
			start, d.kind();
		Invalid, Input, Index)),
	}
}

/// Checks an index against the tree region it claims to describe, naming the first entry that lies.
///
/// An index may name fewer nodes than the tree holds, since a reader that misses an entry can
/// always walk instead, but every entry it does carry must land on the node it names.
pub fn check(
	tree_bytes:	&[u8],
	index:		&BTreeMap<u64, u64>,
)
	-> Outcome<()>
{
	let truth = res!(offsets(tree_bytes));
	for (id, off) in index {
		match truth.get(id) {
			None => return Err(err!(
				"The index names node {}, but the tree holds {} nodes.", id, truth.len();
			Invalid, Input, Index)),
			Some(real) if real != off => return Err(err!(
				"The index puts node {} at byte {}, but it starts at byte {}.", id, off, real;
			Invalid, Input, Index)),
			Some(_) => (),
		}
	}
	Ok(())
}

/// Walks one node, recording its id and offset, and returns the offset just past it.
fn walk_node(
	buf:	&[u8],
	off:	usize,
	depth:	usize,
	next:	&mut u64,
	out:	&mut BTreeMap<u64, u64>,
)
	-> Outcome<usize>
{
	if depth > limit::DEPTH {
		return Err(err!(
			"The node at byte {} nests deeper than the limit of {}.", off, limit::DEPTH;
		Invalid, Input, Excessive));
	}
	if off >= buf.len() {
		return Err(err!(
			"Expected a node at byte {}, but the tree region is only {} bytes.",
			off, buf.len();
		Invalid, Input, Decode));
	}
	if buf[off] != Dat::USR_CODE {
		return Err(err!(
			"Expected a node at byte {}: a node is a usr daticle, code {}, but the byte there \
			is {}.", off, Dat::USR_CODE, buf[off];
		Invalid, Input, Decode));
	}
	let id = *next;
	*next += 1;
	if *next as usize > limit::NODES {
		return Err(err!(
			"The tree holds more nodes than the limit of {}.", limit::NODES;
		Invalid, Input, Excessive));
	}
	out.insert(id, try_into!(u64, off));
	// A node's header is the usr code, the u16 kind code, then the marker saying whether a
	// payload follows.
	let mark = off + 1 + UsrKindId::CODE_BYTE_LEN;
	if mark >= buf.len() {
		return Err(err!(
			"Node {} at byte {} is truncated: its header runs past the end of the {} byte tree \
			region.", id, off, buf.len();
		Invalid, Input, Decode));
	}
	match buf[mark] {
		Dat::OPT_NONE_CODE => Ok(mark + 1),
		Dat::OPT_SOME_CODE => {
			let p = mark + 1;
			if p >= buf.len() {
				return Err(err!(
					"Node {} at byte {} declares a payload, but the tree region ends at byte \
					{}.", id, off, buf.len();
				Invalid, Input, Decode));
			}
			match buf[p] {
				Dat::MAP_CODE => walk_map(buf, p, id, depth, next, out),
				// Any other payload, such as the string a text node carries, holds no children
				// and is skipped whole.
				_ => {
					let len = res!(skip(buf, p));
					Ok(p + len)
				},
			}
		},
		code => Err(err!(
			"Node {} at byte {} carries the byte {} where a usr payload marker, {} or {}, \
			belongs.", id, off, code, Dat::OPT_NONE_CODE, Dat::OPT_SOME_CODE;
		Invalid, Input, Decode)),
	}
}

/// Walks a node's payload map, descending into the children it names, and returns its end offset.
fn walk_map(
	buf:	&[u8],
	off:	usize,
	id:	u64,
	depth:	usize,
	next:	&mut u64,
	out:	&mut BTreeMap<u64, u64>,
)
	-> Outcome<usize>
{
	let (plen, n) = res!(read_len(buf, off + 1));
	let start = off + 1 + n;
	let end = res!(region_end(start, plen, buf.len(), id, "payload map"));
	let mut q = start;
	while q < end {
		let klen = res!(skip(buf, q));
		let kend = q + klen;
		if kend > end {
			return Err(err!(
				"A key of the payload map of node {} runs past the {} bytes the map declares.",
				id, plen;
			Invalid, Input, Decode));
		}
		q = if is_children_key(&buf[q..kend]) {
			res!(walk_children(buf, kend, end, id, depth, next, out))
		} else {
			let vlen = res!(skip(buf, kend));
			let vend = kend + vlen;
			if vend > end {
				return Err(err!(
					"A value of the payload map of node {} runs past the {} bytes the map \
					declares.", id, plen;
				Invalid, Input, Decode));
			}
			vend
		};
	}
	if q != end {
		return Err(err!(
			"The entries of the payload map of node {} end at byte {}, not at byte {} where the \
			{} bytes it declares run out.", id, q, end, plen;
		Invalid, Input, Decode));
	}
	Ok(end)
}

/// Walks the list of children a node carries, and returns the offset just past the list.
fn walk_children(
	buf:	&[u8],
	off:	usize,
	end:	usize,
	id:	u64,
	depth:	usize,
	next:	&mut u64,
	out:	&mut BTreeMap<u64, u64>,
)
	-> Outcome<usize>
{
	if off >= buf.len() {
		return Err(err!(
			"Node {} names its children at byte {}, past the end of the {} byte tree region.",
			id, off, buf.len();
		Invalid, Input, Decode));
	}
	if buf[off] != Dat::LIST_CODE {
		return Err(err!(
			"The children of node {} must be a list, code {}, but the byte at {} is {}.",
			id, Dat::LIST_CODE, off, buf[off];
		Invalid, Input, Decode));
	}
	let (plen, n) = res!(read_len(buf, off + 1));
	let start = off + 1 + n;
	let stop = res!(region_end(start, plen, end, id, "children list"));
	let mut q = start;
	while q < stop {
		q = res!(walk_node(buf, q, depth + 1, next, out));
	}
	if q != stop {
		return Err(err!(
			"The children of node {} end at byte {}, not at byte {} where the {} bytes the list \
			declares run out.", id, q, stop, plen;
		Invalid, Input, Decode));
	}
	Ok(stop)
}

/// Skips the daticle at an offset, returning its length in bytes without decoding it.
fn skip(
	buf:	&[u8],
	off:	usize,
)
	-> Outcome<usize>
{
	if off >= buf.len() {
		return Err(err!(
			"Expected a daticle at byte {}, but the tree region is only {} bytes.",
			off, buf.len();
		Invalid, Input, Decode));
	}
	let mut cursor = Cursor::new(&buf[off..]);
	let len = res!(Dat::count_bytes(&mut cursor));
	let left = buf.len() - off;
	if len == 0 || len > left {
		return Err(err!(
			"The daticle at byte {} counts {} bytes, which overruns the {} bytes left in the \
			tree region.", off, len, left;
		Invalid, Input, Decode));
	}
	Ok(len)
}

/// Reads the c64 byte length that BDAT puts in front of a compound.
fn read_len(
	buf:	&[u8],
	off:	usize,
)
	-> Outcome<(usize, usize)>
{
	if off >= buf.len() {
		return Err(err!(
			"Expected a c64 length at byte {}, but the tree region is only {} bytes.",
			off, buf.len();
		Invalid, Input, Decode));
	}
	if buf[off] < Dat::C64_CODE_START || buf[off] > Dat::C64_CODE_END {
		return Err(err!(
			"Expected a c64 length at byte {}, whose code must lie between {} and {}, but the \
			byte there is {}.", off, Dat::C64_CODE_START, Dat::C64_CODE_END, buf[off];
		Invalid, Input, Decode));
	}
	let (len, n) = res!(Dat::read_c64(&buf[off..]));
	Ok((try_into!(usize, len), n))
}

/// Returns the end of a declared region, refusing one that runs past the bytes available.
fn region_end(
	start:	usize,
	plen:	usize,
	avail:	usize,
	id:	u64,
	what:	&str,
)
	-> Outcome<usize>
{
	match start.checked_add(plen) {
		Some(end) if end <= avail => Ok(end),
		_ => Err(err!(
			"The {} of node {} declares {} bytes from byte {}, running past the {} bytes \
			available.", what, id, plen, start, avail;
		Invalid, Input, Decode)),
	}
}

/// Whether an encoded map key is the one under which a node carries its children.
fn is_children_key(byts: &[u8]) -> bool {
	// A key encodes as the str code, a one byte c64 length, then the key itself.
	let key = KEY_CHILDREN.as_bytes();
	byts.len() == 3 + key.len()
		&& byts[0] == Dat::STR_CODE
		&& byts[1] == Dat::C64_CODE_START + 1
		&& byts[2] as usize == key.len()
		&& &byts[3..] == key
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::kinds::NodeKind;

	/// Wraps a payload as a node of the given kind.
	fn node(kind: NodeKind, payload: Dat) -> Dat {
		Dat::Usr(
			UsrKindId::new(kind.code(), Some(kind.label()), None),
			Some(Box::new(payload)),
		)
	}

	/// A text node, whose payload is a bare string.
	fn text(s: &str) -> Dat {
		node(NodeKind::Text, Dat::Str(s.to_string()))
	}

	/// A node whose payload is a map of the given fields plus a list of children.
	fn parent(kind: NodeKind, fields: Vec<(&str, Dat)>, children: Vec<Dat>) -> Dat {
		let mut map = DaticleMap::new();
		for (k, v) in fields {
			map.insert(dat!(k), v);
		}
		map.insert(dat!(KEY_CHILDREN), Dat::List(children));
		node(kind, Dat::Map(map))
	}

	/// A small tree exercising every shape the walk must cope with: a map payload, a bare string
	/// payload, nesting, and siblings.
	fn tree() -> Dat {
		parent(NodeKind::Doc, vec![
			("title",	dat!("Style without a cascade")),
			("lang",	dat!("en")),
		], vec![
			parent(NodeKind::Heading, vec![("level", Dat::U8(2))], vec![
				text("Style without a cascade"),
			]),
			parent(NodeKind::Para, vec![], vec![
				text("A paragraph with "),
				parent(NodeKind::Emph, vec![("strong", Dat::Bool(true))], vec![
					text("emphasis"),
				]),
			]),
			parent(NodeKind::List, vec![("ordered", Dat::Bool(false))], vec![
				parent(NodeKind::Item, vec![], vec![
					parent(NodeKind::Para, vec![], vec![
						text("An item."),
					]),
				]),
			]),
		])
	}

	/// The kinds of the tree above, in depth-first pre-order: the node each id names.
	fn expected() -> Vec<NodeKind> {
		vec![
			NodeKind::Doc,
			NodeKind::Heading,
			NodeKind::Text,
			NodeKind::Para,
			NodeKind::Text,
			NodeKind::Emph,
			NodeKind::Text,
			NodeKind::List,
			NodeKind::Item,
			NodeKind::Para,
			NodeKind::Text,
		]
	}

	/// The encoded tree region.
	fn encoded() -> Outcome<Vec<u8>> {
		Ok(res!(tree().to_bytes(Vec::new())))
	}

	/// The kind code of a decoded node.
	fn code_of(dat: &Dat) -> Outcome<u16> {
		match dat {
			Dat::Usr(ukid, _) => Ok(ukid.code()),
			d => Err(err!("Expected a node, found a {:?}.", d.kind(); Invalid, Input)),
		}
	}

	#[test]
	fn test_offsets_land_on_node_boundaries() -> Outcome<()> {
		let byts = res!(encoded());
		let offs = res!(offsets(&byts));
		let want = expected();
		assert_eq!(offs.len(), want.len());
		// Every offset must decode to the node its id names.
		for (i, kind) in want.iter().enumerate() {
			let id = i as u64;
			let off = match offs.get(&id) {
				Some(off) => *off,
				None => return Err(err!("No index entry for node {}.", id; Invalid, Input)),
			};
			let dat = res!(node_at(&byts, off));
			assert_eq!(res!(code_of(&dat)), kind.code(), "node {} at byte {}", id, off);
		}
		// The root sits at byte 0 and offsets rise with the pre-order.
		assert_eq!(offs.get(&0), Some(&0u64));
		let mut last = 0u64;
		for (id, off) in &offs {
			if *id > 0 {
				assert!(*off > last, "node {} at byte {} does not follow byte {}", id, off, last);
			}
			last = *off;
		}
		Ok(())
	}

	#[test]
	fn test_round_trip_build_parse() -> Outcome<()> {
		let byts = res!(encoded());
		let idx = res!(build(&byts));
		let parsed = res!(parse(&idx));
		assert_eq!(parsed, res!(offsets(&byts)));
		res!(check(&byts, &parsed));
		Ok(())
	}

	#[test]
	fn test_text_payload_is_skipped_not_descended() -> Outcome<()> {
		// A text node's payload is a bare string, so it must contribute exactly one id and no
		// children, even though the string it carries could be mistaken for anything.
		let byts = res!(node(NodeKind::Text, dat!("children")).to_bytes(Vec::new()));
		let offs = res!(offsets(&byts));
		assert_eq!(offs.len(), 1);
		assert_eq!(offs.get(&0), Some(&0u64));
		Ok(())
	}

	#[test]
	fn test_corrupt_offset_past_end_is_detected() -> Outcome<()> {
		let byts = res!(encoded());
		let past = byts.len() as u64;
		assert!(node_at(&byts, past).is_err());
		assert!(node_at(&byts, u64::MAX).is_err());
		let mut idx = res!(offsets(&byts));
		idx.insert(2, past);
		assert!(check(&byts, &idx).is_err());
		Ok(())
	}

	#[test]
	fn test_corrupt_offset_inside_a_node_is_detected() -> Outcome<()> {
		let byts = res!(encoded());
		let truth = res!(offsets(&byts));
		let real = match truth.get(&2) {
			Some(off) => *off,
			None => return Err(err!("No index entry for node 2."; Invalid, Input)),
		};
		// One byte into a node lands on its kind code, not on a node.
		assert!(node_at(&byts, real + 1).is_err());
		let mut idx = truth.clone();
		idx.insert(2, real + 1);
		assert!(check(&byts, &idx).is_err());
		// An id the tree does not hold is refused too.
		let mut idx = truth.clone();
		idx.insert(999, 0);
		assert!(check(&byts, &idx).is_err());
		Ok(())
	}

	#[test]
	fn test_no_offset_off_a_boundary_survives() -> Outcome<()> {
		let byts = res!(encoded());
		let truth = res!(offsets(&byts));
		let bounds: Vec<u64> = truth.values().cloned().collect();
		// Whatever a corrupt index says, a reader is confined to the tree region and every
		// offset that is not a node boundary is refused when it is used.
		for off in 0..(byts.len() as u64 + 64) {
			let boundary = bounds.contains(&off);
			assert_eq!(node_at(&byts, off).is_ok(), boundary,
				"byte {} accepted as a node, boundary = {}", off, boundary);
			let mut idx = BTreeMap::new();
			idx.insert(0u64, off);
			assert_eq!(check(&byts, &idx).is_ok(), off == 0, "byte {} accepted for node 0", off);
		}
		Ok(())
	}

	#[test]
	fn test_truncated_and_padded_trees_are_refused() -> Outcome<()> {
		let byts = res!(encoded());
		// A tree region one byte short of the root node it holds.
		assert!(offsets(&byts[..byts.len() - 1]).is_err());
		// A tree region one byte longer than the root node it holds.
		let mut padded = byts.clone();
		padded.push(0);
		assert!(offsets(&padded).is_err());
		// Nothing at all.
		assert!(offsets(&[]).is_err());
		Ok(())
	}

	#[test]
	fn test_index_nesting_is_bounded() -> Outcome<()> {
		// The index region trails the hashed part of a file, so its length and its contents are
		// whatever the last hand to touch the file made them. A hundred thousand nested lists cost
		// a few hundred kilobytes to write and a stack to read, so the decoder is bounded, and an
		// index is flat.
		let mut buf = Vec::new();
		for _ in 0..100_000 {
			let mut lvl = vec![Dat::LIST_CODE];
			lvl = res!(Dat::C64(buf.len() as u64).to_bytes(lvl));
			lvl.extend_from_slice(&buf);
			buf = lvl;
		}
		match parse(&buf) {
			Ok(_) => Err(err!(
				"A deeply nested index region was accepted.";
			Test, Invalid)),
			Err(e) => {
				let msg = fmt!("{}", e);
				assert!(msg.contains("nesting depth"),
					"The rejection should name the depth limit, but says: {}", msg);
				Ok(())
			},
		}
	}

	#[test]
	fn test_index_bytes_are_a_map_of_c64_to_c64() -> Outcome<()> {
		let byts = res!(encoded());
		let idx = res!(build(&byts));
		let (dat, n) = res!(Dat::from_bytes(&idx));
		assert_eq!(n, idx.len());
		match dat {
			Dat::Map(map) => {
				assert_eq!(map.len(), expected().len());
				for (k, v) in &map {
					assert!(matches!(k, Dat::C64(_)), "index key is not a c64: {:?}", k.kind());
					assert!(matches!(v, Dat::C64(_)), "index value is not a c64: {:?}", v.kind());
				}
			},
			d => return Err(err!("The index is a {:?}, not a map.", d.kind(); Invalid, Input)),
		}
		// Garbage is not an index.
		assert!(parse(&[0xff, 0xff, 0xff]).is_err());
		// A well formed daticle that is not a map is not an index either.
		let not_a_map = res!(dat!("nope").to_bytes(Vec::new()));
		assert!(parse(&not_a_map).is_err());
		// Trailing bytes after the map are not an index either.
		let mut padded = idx.clone();
		padded.push(0);
		assert!(parse(&padded).is_err());
		Ok(())
	}
}
