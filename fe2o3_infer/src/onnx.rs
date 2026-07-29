//! A reader for the subset of the ONNX wire format a small convolutional
//! network uses.
//!
//! ONNX is protocol buffers, and protocol buffers can be walked without a
//! schema: every field carries its number and its wire type. This module reads
//! only the fields it needs -- nodes, their attributes, and the initialisers
//! that hold the weights -- and ignores the rest, so it is a few hundred lines
//! rather than a generated library.
//!
//! Nothing here interprets the graph. [`crate::graph`] does that.

use oxedyne_fe2o3_core::prelude::*;

/// Protocol buffer wire types this reader understands.
const WIRE_VARINT:	u8 = 0;
const WIRE_I64:		u8 = 1;
const WIRE_LEN:		u8 = 2;
const WIRE_I32:		u8 = 5;

/// ONNX tensor element types this reader understands.
const DT_FLOAT:	i64 = 1;
const DT_INT64:	i64 = 7;

/// A cursor over a protocol buffer message.
struct Reader<'a> {
	/// The bytes of one message.
	buf:	&'a [u8],
	/// Read position within `buf`.
	pos:	usize,
}

/// One field of a protocol buffer message, as read off the wire.
enum Field<'a> {
	/// A base-128 integer.
	Varint(u64),
	/// A length-delimited payload: a string, a submessage or packed values.
	Bytes(&'a [u8]),
	/// A fixed thirty-two bit value.
	Fixed32([u8; 4]),
	/// A fixed sixty-four bit value, which no field this reader wants uses.
	Fixed64,
}

impl<'a> Reader<'a> {
	/// Starts a cursor over a message.
	fn new(buf: &'a [u8]) -> Self {
		Self { buf, pos: 0 }
	}

	/// Whether every byte has been consumed.
	fn done(&self) -> bool {
		self.pos >= self.buf.len()
	}

	/// Reads one base-128 integer.
	fn varint(&mut self) -> Outcome<u64> {
		let mut r = 0u64;
		let mut shift = 0u32;
		loop {
			if self.pos >= self.buf.len() {
				return Err(err!("A base-128 integer runs past the end of the message.";
					Invalid, Input, Decode));
			}
			let b = self.buf[self.pos];
			self.pos += 1;
			if shift >= 64 {
				return Err(err!("A base-128 integer is wider than sixty-four bits.";
					Invalid, Input, Decode));
			}
			r |= ((b & 0x7f) as u64) << shift;
			if b & 0x80 == 0 {
				return Ok(r);
			}
			shift += 7;
		}
	}

	/// Reads the next field, answering its number and payload.
	fn next(&mut self) -> Outcome<(u64, Field<'a>)> {
		let key = res!(self.varint());
		let num = key >> 3;
		let wire = (key & 7) as u8;
		let f = match wire {
			WIRE_VARINT => Field::Varint(res!(self.varint())),
			WIRE_LEN => {
				let n = res!(self.varint()) as usize;
				let end = match self.pos.checked_add(n) {
					Some(e) if e <= self.buf.len() => e,
					_ => return Err(err!(
						"A length-delimited field of {} bytes at {} runs past the end of a \
						message of {} bytes.", n, self.pos, self.buf.len();
					Invalid, Input, Decode)),
				};
				let s = &self.buf[self.pos..end];
				self.pos = end;
				Field::Bytes(s)
			},
			WIRE_I32 => {
				if self.pos + 4 > self.buf.len() {
					return Err(err!("A thirty-two bit field runs past the end of the message.";
						Invalid, Input, Decode));
				}
				let mut a = [0u8; 4];
				a.copy_from_slice(&self.buf[self.pos..self.pos + 4]);
				self.pos += 4;
				Field::Fixed32(a)
			},
			WIRE_I64 => {
				if self.pos + 8 > self.buf.len() {
					return Err(err!("A sixty-four bit field runs past the end of the message.";
						Invalid, Input, Decode));
				}
				self.pos += 8;
				Field::Fixed64
			},
			other => return Err(err!(
				"Wire type {} is not one this reader knows.", other; Invalid, Input, Decode)),
		};
		Ok((num, f))
	}
}

/// Reads a payload as a UTF-8 string.
fn as_str(b: &[u8]) -> Outcome<String> {
	match core::str::from_utf8(b) {
		Ok(s) => Ok(s.to_string()),
		Err(e) => Err(err!(e, "A name in the model is not valid UTF-8."; Invalid, Input, Decode)),
	}
}

/// Reads a payload as packed base-128 integers.
fn packed_varints(b: &[u8]) -> Outcome<Vec<i64>> {
	let mut r = Reader::new(b);
	let mut out = Vec::new();
	while !r.done() {
		out.push(res!(r.varint()) as i64);
	}
	Ok(out)
}

/// Reads a payload as little-endian `f32` values.
fn le_f32(b: &[u8]) -> Outcome<Vec<f32>> {
	if b.len() % 4 != 0 {
		return Err(err!(
			"A block of {} bytes does not divide into four-byte floats.", b.len();
		Invalid, Input, Decode));
	}
	let mut out = Vec::with_capacity(b.len() / 4);
	for c in b.chunks_exact(4) {
		out.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
	}
	Ok(out)
}

/// Reads a payload as little-endian `i64` values.
fn le_i64(b: &[u8]) -> Outcome<Vec<i64>> {
	if b.len() % 8 != 0 {
		return Err(err!(
			"A block of {} bytes does not divide into eight-byte integers.", b.len();
		Invalid, Input, Decode));
	}
	let mut out = Vec::with_capacity(b.len() / 8);
	for c in b.chunks_exact(8) {
		let mut a = [0u8; 8];
		a.copy_from_slice(c);
		out.push(i64::from_le_bytes(a));
	}
	Ok(out)
}

/// The value of one node attribute.
#[derive(Clone, Debug)]
pub enum Attr {
	/// A single integer.
	Int(i64),
	/// A single float.
	Float(f32),
	/// A single string.
	Str(String),
	/// A list of integers.
	Ints(Vec<i64>),
	/// A list of floats.
	Floats(Vec<f32>),
}

impl Attr {
	/// Reads the attribute as a single integer.
	pub fn int(&self) -> Outcome<i64> {
		match self {
			Self::Int(v) => Ok(*v),
			other => Err(err!("An integer attribute was expected, found {:?}.", other;
				Invalid, Input, Mismatch)),
		}
	}

	/// Reads the attribute as a single float.
	pub fn float(&self) -> Outcome<f32> {
		match self {
			Self::Float(v) => Ok(*v),
			other => Err(err!("A float attribute was expected, found {:?}.", other;
				Invalid, Input, Mismatch)),
		}
	}

	/// Reads the attribute as a list of integers.
	pub fn ints(&self) -> Outcome<&[i64]> {
		match self {
			Self::Ints(v) => Ok(v),
			other => Err(err!("A list of integers was expected, found {:?}.", other;
				Invalid, Input, Mismatch)),
		}
	}

	/// Reads the attribute as a string.
	pub fn text(&self) -> Outcome<&str> {
		match self {
			Self::Str(v) => Ok(v),
			other => Err(err!("A string attribute was expected, found {:?}.", other;
				Invalid, Input, Mismatch)),
		}
	}
}

/// An initialiser, in whichever element type the model stored it.
#[derive(Clone, Debug)]
pub enum Init {
	/// Thirty-two bit floats -- weights, biases, scales.
	F32 {
		/// Extent along each axis.
		dims:	Vec<usize>,
		/// Values, row-major.
		data:	Vec<f32>,
	},
	/// Sixty-four bit integers -- shapes, axes, permutations.
	I64 {
		/// Extent along each axis.
		dims:	Vec<usize>,
		/// Values, row-major.
		data:	Vec<i64>,
	},
}

impl Init {
	/// Extent along each axis.
	pub fn dims(&self) -> &[usize] {
		match self {
			Self::F32 { dims, .. } => dims,
			Self::I64 { dims, .. } => dims,
		}
	}

	/// Reads the initialiser as floats.
	pub fn floats(&self) -> Outcome<&[f32]> {
		match self {
			Self::F32 { data, .. } => Ok(data),
			Self::I64 { dims, .. } => Err(err!(
				"A float initialiser was expected, found integers of shape {:?}.", dims;
			Invalid, Input, Mismatch)),
		}
	}

	/// Reads the initialiser as integers.
	pub fn ints(&self) -> Outcome<&[i64]> {
		match self {
			Self::I64 { data, .. } => Ok(data),
			Self::F32 { dims, .. } => Err(err!(
				"An integer initialiser was expected, found floats of shape {:?}.", dims;
			Invalid, Input, Mismatch)),
		}
	}
}

/// One node of the graph, as the model spelled it.
#[derive(Clone, Debug, Default)]
pub struct Node {
	/// The operator name, such as `Conv`.
	pub op:			String,
	/// The node's own name, which may be empty.
	pub name:		String,
	/// Names of the tensors it consumes.
	pub inputs:		Vec<String>,
	/// Names of the tensors it produces.
	pub outputs:	Vec<String>,
	/// Attributes, in the order the model listed them.
	pub attrs:		Vec<(String, Attr)>,
}

impl Node {
	/// Finds an attribute by name.
	pub fn attr(&self, name: &str) -> Option<&Attr> {
		self.attrs.iter().find(|(n, _)| n == name).map(|(_, a)| a)
	}

	/// Finds an attribute by name, failing if it is absent.
	pub fn need(&self, name: &str) -> Outcome<&Attr> {
		match self.attr(name) {
			Some(a) => Ok(a),
			None => Err(err!(
				"The {} node has no {} attribute.", self.op, name; Invalid, Input, Missing)),
		}
	}
}

/// A model, read but not yet interpreted.
#[derive(Clone, Debug, Default)]
pub struct Model {
	/// Nodes, in the order the model listed them, which ONNX requires to be
	/// topological.
	pub nodes:		Vec<Node>,
	/// Initialisers, by name.
	pub inits:		Vec<(String, Init)>,
	/// Declared graph inputs. A model exported from some frameworks lists its
	/// weights here as well, each shadowed by an initialiser of the same name.
	pub inputs:		Vec<String>,
	/// Declared graph outputs, in order.
	pub outputs:	Vec<String>,
}

impl Model {
	/// Finds an initialiser by name.
	pub fn init(&self, name: &str) -> Option<&Init> {
		self.inits.iter().find(|(n, _)| n == name).map(|(_, i)| i)
	}

	/// Reads a model from the bytes of an `.onnx` file.
	pub fn read(bytes: &[u8]) -> Outcome<Self> {
		let mut r = Reader::new(bytes);
		let mut graph = None;
		while !r.done() {
			let (num, f) = res!(r.next());
			if num == 7 {
				if let Field::Bytes(b) = f {
					graph = Some(b);
				}
			}
		}
		let g = match graph {
			Some(g) => g,
			None => return Err(err!("The model carries no graph."; Invalid, Input, Missing)),
		};
		Self::read_graph(g)
	}

	/// Reads a `GraphProto`.
	fn read_graph(buf: &[u8]) -> Outcome<Self> {
		let mut m = Self::default();
		let mut r = Reader::new(buf);
		while !r.done() {
			let (num, f) = res!(r.next());
			match (num, f) {
				(1, Field::Bytes(b)) => m.nodes.push(res!(read_node(b))),
				(5, Field::Bytes(b)) => {
					let (name, init) = res!(read_tensor(b));
					m.inits.push((name, init));
				},
				(11, Field::Bytes(b)) => m.inputs.push(res!(read_value_info(b))),
				(12, Field::Bytes(b)) => m.outputs.push(res!(read_value_info(b))),
				_ => {},
			}
		}
		Ok(m)
	}
}

/// Reads a `NodeProto`.
fn read_node(buf: &[u8]) -> Outcome<Node> {
	let mut n = Node::default();
	let mut r = Reader::new(buf);
	while !r.done() {
		let (num, f) = res!(r.next());
		match (num, f) {
			(1, Field::Bytes(b)) => n.inputs.push(res!(as_str(b))),
			(2, Field::Bytes(b)) => n.outputs.push(res!(as_str(b))),
			(3, Field::Bytes(b)) => n.name = res!(as_str(b)),
			(4, Field::Bytes(b)) => n.op = res!(as_str(b)),
			(5, Field::Bytes(b)) => {
				if let Some(a) = res!(read_attr(b)) {
					n.attrs.push(a);
				}
			},
			_ => {},
		}
	}
	Ok(n)
}

/// Reads an `AttributeProto`, answering `None` for a kind this reader does not
/// carry -- a subgraph, for instance, which no model here uses.
fn read_attr(buf: &[u8]) -> Outcome<Option<(String, Attr)>> {
	let mut name = String::new();
	let mut typ = 0i64;
	let mut i = 0i64;
	let mut fl = 0f32;
	let mut s = String::new();
	let mut ints: Vec<i64> = Vec::new();
	let mut floats: Vec<f32> = Vec::new();
	let mut r = Reader::new(buf);
	while !r.done() {
		let (num, f) = res!(r.next());
		match (num, f) {
			(1, Field::Bytes(b)) => name = res!(as_str(b)),
			(2, Field::Fixed32(a)) => fl = f32::from_le_bytes(a),
			(3, Field::Varint(v)) => i = v as i64,
			(4, Field::Bytes(b)) => s = res!(as_str(b)),
			(7, Field::Bytes(b)) => floats = res!(le_f32(b)),
			(7, Field::Fixed32(a)) => floats.push(f32::from_le_bytes(a)),
			(8, Field::Bytes(b)) => ints = res!(packed_varints(b)),
			(8, Field::Varint(v)) => ints.push(v as i64),
			(20, Field::Varint(v)) => typ = v as i64,
			_ => {},
		}
	}
	// AttributeType: 1 FLOAT, 2 INT, 3 STRING, 6 FLOATS, 7 INTS.
	let a = match typ {
		1 => Attr::Float(fl),
		2 => Attr::Int(i),
		3 => Attr::Str(s),
		6 => Attr::Floats(floats),
		7 => Attr::Ints(ints),
		_ => return Ok(None),
	};
	Ok(Some((name, a)))
}

/// Reads a `TensorProto`, answering its name and values.
fn read_tensor(buf: &[u8]) -> Outcome<(String, Init)> {
	let mut dims: Vec<usize> = Vec::new();
	let mut dtype = 0i64;
	let mut name = String::new();
	let mut raw: Option<&[u8]> = None;
	let mut floats: Vec<f32> = Vec::new();
	let mut ints: Vec<i64> = Vec::new();
	let mut r = Reader::new(buf);
	while !r.done() {
		let (num, f) = res!(r.next());
		match (num, f) {
			(1, Field::Varint(v)) => dims.push(v as usize),
			(1, Field::Bytes(b)) => {
				for v in res!(packed_varints(b)) {
					dims.push(v as usize);
				}
			},
			(2, Field::Varint(v)) => dtype = v as i64,
			(4, Field::Bytes(b)) => floats = res!(le_f32(b)),
			(7, Field::Bytes(b)) => ints = res!(packed_varints(b)),
			(8, Field::Bytes(b)) => name = res!(as_str(b)),
			(9, Field::Bytes(b)) => raw = Some(b),
			_ => {},
		}
	}
	let want = dims.iter().product::<usize>();
	let init = match dtype {
		DT_FLOAT => {
			let data = match raw {
				Some(b) => res!(le_f32(b)),
				None => floats,
			};
			if data.len() != want {
				return Err(err!(
					"The initialiser {} declares shape {:?}, which wants {} floats, but \
					carries {}.", name, dims, want, data.len();
				Invalid, Input, Mismatch));
			}
			Init::F32 { dims, data }
		},
		DT_INT64 => {
			let data = match raw {
				Some(b) => res!(le_i64(b)),
				None => ints,
			};
			if data.len() != want {
				return Err(err!(
					"The initialiser {} declares shape {:?}, which wants {} integers, but \
					carries {}.", name, dims, want, data.len();
				Invalid, Input, Mismatch));
			}
			Init::I64 { dims, data }
		},
		other => return Err(err!(
			"The initialiser {} has element type {}, which this reader does not carry.",
			name, other;
		Invalid, Input, Unimplemented)),
	};
	Ok((name, init))
}

/// Reads the name out of a `ValueInfoProto`.
fn read_value_info(buf: &[u8]) -> Outcome<String> {
	let mut r = Reader::new(buf);
	while !r.done() {
		let (num, f) = res!(r.next());
		if let (1, Field::Bytes(b)) = (num, f) {
			return as_str(b);
		}
	}
	Ok(String::new())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_truncated_message_is_an_error_not_a_panic() -> Outcome<()> {
		// A length-delimited field claiming more bytes than remain.
		let bytes = [0x0a, 0x40, 0x01, 0x02];
		let mut r = Reader::new(&bytes);
		req!(r.next().is_err(), true);
		Ok(())
	}

	#[test]
	fn an_empty_model_has_no_graph() -> Outcome<()> {
		req!(Model::read(&[]).is_err(), true);
		Ok(())
	}

	#[test]
	fn packed_dimensions_read_back() -> Outcome<()> {
		let v = res!(packed_varints(&[0x01, 0x02, 0x80, 0x02]));
		req!(v, vec![1i64, 2, 256]);
		Ok(())
	}
}
