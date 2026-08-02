//! The byte stream state machine.
//!
//! A pseudoterminal hands over bytes in whatever sizes the kernel happens to deliver, so the
//! parser is fed slices of arbitrary length and must survive being cut anywhere: in the middle of
//! a UTF-8 character, between the `ESC` and the `[` of a control sequence, halfway through a
//! parameter list. All of the machine's state therefore lives in [`Parser`] and persists between
//! calls to [`Parser::advance`].
//!
//! The structure follows the VT500 parser of Paul Williams, reduced to the states a modern
//! application actually drives. The rule that matters most is the one about the unrecognised: a
//! sequence the machine does not understand is consumed to its end and dropped. It is never
//! allowed to fall through and be printed, which is what produces the familiar screenful of
//! `[38;5;196m` when a terminal gets this wrong.
//!
//! Memory is bounded by construction. Parameters go into a fixed array and further ones are
//! counted but not stored; a string payload longer than [`MAX_STRING_BYTES`] is dropped while the
//! scan for its terminator continues, so a runaway sequence costs time but not space. `CAN`, `SUB`
//! and a fresh `ESC` abandon whatever was in progress.

use oxedyne_fe2o3_core::prelude::*;


/// The largest number of parameters a control sequence may carry.
pub const MAX_PARAMS: usize = 32;

/// The largest value a single parameter may take.
pub const MAX_PARAM_VALUE: u32 = 65535;

/// The largest string payload, in bytes, that an `OSC` or other string sequence may carry before
/// the payload is discarded.
pub const MAX_STRING_BYTES: usize = 8192;

/// The character substituted for a malformed UTF-8 sequence.
pub const REPLACEMENT: char = '\u{FFFD}';

/// The parameters of a control sequence.
///
/// Parameters are separated by `;`. A parameter may itself be subdivided by `:`, which is how the
/// modern form of the colour selection `38:2::r:g:b` is written; [`Params::is_sub`] reports which
/// separator preceded a value.
#[derive(Clone, Copy, Debug)]
pub struct Params {
	/// The values, in order.
	vals:	[u32; MAX_PARAMS],
	/// Whether each value was introduced by `:` rather than `;`.
	subs:	[bool; MAX_PARAMS],
	/// How many values are held.
	len:	usize,
	/// Whether the sequence carried more parameters, or a larger value, than can be represented.
	over:	bool,
}

impl Default for Params {
	fn default() -> Self {
		Self {
			vals:	[0; MAX_PARAMS],
			subs:	[false; MAX_PARAMS],
			len:	0,
			over:	false,
		}
	}
}

impl Params {
	/// How many parameters were given.
	pub fn len(&self) -> usize {
		self.len
	}

	/// Whether no parameter was given at all.
	pub fn is_empty(&self) -> bool {
		self.len == 0
	}

	/// The parameter at `i`, if it was given.
	pub fn get(&self, i: usize) -> Option<u32> {
		if i < self.len {
			Some(self.vals[i])
		} else {
			None
		}
	}

	/// The parameter at `i`, or `dflt` if it was absent or written as an empty field.
	///
	/// An empty field and a zero are the same thing to nearly every sequence, which is why they are
	/// folded together here.
	pub fn get_or(&self, i: usize, dflt: u32) -> u32 {
		match self.get(i) {
			Some(0) | None	=> dflt,
			Some(v)		=> v,
		}
	}

	/// Whether the parameter at `i` was introduced by `:` rather than `;`.
	pub fn is_sub(&self, i: usize) -> bool {
		i < self.len && self.subs[i]
	}

	/// Whether the sequence overflowed the representable parameter space.
	pub fn overflowed(&self) -> bool {
		self.over
	}

	/// Starts a value if none has been started, then folds in a decimal digit.
	fn digit(&mut self, d: u32) {
		if self.len == 0 {
			self.push(false);
		}
		if self.over {
			return;
		}
		let v = self.vals[self.len - 1] * 10 + d;
		if v > MAX_PARAM_VALUE {
			self.over = true;
		} else {
			self.vals[self.len - 1] = v;
		}
	}

	/// Closes the current value and opens the next.
	fn separate(&mut self, sub: bool) {
		if self.len == 0 {
			self.push(false);
		}
		self.push(sub);
	}

	/// Appends an empty value, noting overflow if there is no room for it.
	fn push(&mut self, sub: bool) {
		if self.len >= MAX_PARAMS {
			self.over = true;
			return;
		}
		self.vals[self.len] = 0;
		self.subs[self.len] = sub;
		self.len += 1;
	}
}

/// A C0 control the screen acts on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C0 {
	/// `BEL`, 0x07.
	Bell,
	/// `BS`, 0x08.
	Backspace,
	/// `HT`, 0x09.
	Tab,
	/// `LF`, `VT` or `FF`, 0x0A to 0x0C.
	LineFeed,
	/// `CR`, 0x0D.
	CarriageReturn,
	/// `SO`, 0x0E, which maps G1 over the printable range.
	ShiftOut,
	/// `SI`, 0x0F, which maps G0 over the printable range.
	ShiftIn,
}

/// A control sequence introduced by `CSI`.
#[derive(Clone, Copy, Debug)]
pub struct Csi {
	/// The private parameter marker `<`, `=`, `>` or `?`, if one was given.
	pub private:	Option<u8>,
	/// The intermediate byte in the range 0x20 to 0x2F, if one was given.
	pub inter:	Option<u8>,
	/// The parameters.
	pub params:	Params,
	/// The final byte, which names the sequence.
	pub fin:	u8,
}

/// An escape sequence with no `CSI`.
#[derive(Clone, Copy, Debug)]
pub struct Esc {
	/// The intermediate byte in the range 0x20 to 0x2F, if one was given.
	pub inter:	Option<u8>,
	/// The final byte.
	pub fin:	u8,
}

/// An operating system command.
#[derive(Clone, Debug)]
pub struct Osc {
	/// The leading numeric identifier, or `None` if the command did not begin with one.
	pub ident:	Option<u32>,
	/// Everything after the first `;`.
	pub text:	String,
}

/// One thing the parser has decided the stream is asking for.
#[derive(Clone, Debug)]
pub enum Act {
	/// A character to place on the screen.
	Print(char),
	/// A C0 control.
	Ctrl(C0),
	/// A control sequence.
	Csi(Csi),
	/// An escape sequence.
	Esc(Esc),
	/// An operating system command.
	Osc(Osc),
}

/// Where the machine is in the stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
	/// Ordinary text.
	Ground,
	/// `ESC` has been seen.
	Escape,
	/// `ESC` and an intermediate have been seen.
	EscapeInter,
	/// `CSI` has been seen and nothing follows it yet.
	CsiEntry,
	/// `CSI` parameters are being collected.
	CsiParam,
	/// A `CSI` intermediate has been seen.
	CsiInter,
	/// The sequence is malformed and is being consumed to its end.
	CsiIgnore,
	/// An `OSC` payload is being collected.
	OscString,
	/// An `OSC` payload has grown too long and is being consumed to its end.
	OscIgnore,
	/// An `ESC` has been seen inside a string payload, which may be the start of `ST`.
	StringEsc,
	/// A `DCS`, `SOS`, `PM` or `APC` payload is being consumed to its end.
	StringIgnore,
	/// An `ESC` has been seen inside a payload that is already being ignored.
	StringIgnoreEsc,
}

/// The byte stream state machine.
///
/// Feed it bytes with [`Parser::advance`] and it appends to a caller supplied vector of [`Act`].
/// The caller owns the vector so that it can be reused between feeds and cost no allocation.
#[derive(Clone, Debug)]
pub struct Parser {
	/// Where the machine is.
	state:	State,
	/// The private marker of the sequence being collected.
	private: Option<u8>,
	/// The intermediate of the sequence being collected.
	inter:	Option<u8>,
	/// Whether more than one intermediate was seen, which makes the sequence malformed.
	inter_over: bool,
	/// The parameters of the sequence being collected.
	params:	Params,
	/// The payload of the string sequence being collected.
	string:	Vec<u8>,
	/// The partial UTF-8 character held over from an earlier feed.
	utf8:	[u8; 4],
	/// How many bytes of the partial character are held.
	utf8_len: usize,
	/// How many bytes the partial character needs in total.
	utf8_need: usize,
}

impl Default for Parser {
	fn default() -> Self {
		Self::new()
	}
}

impl Parser {

	/// A parser at the start of a stream.
	pub fn new() -> Self {
		Self {
			state:		State::Ground,
			private:	None,
			inter:		None,
			inter_over:	false,
			params:		Params::default(),
			string:		Vec::new(),
			utf8:		[0; 4],
			utf8_len:	0,
			utf8_need:	0,
		}
	}

	/// Returns the machine to the start of a stream, discarding anything half collected.
	pub fn reset(&mut self) {
		*self = Self::new();
	}

	/// Whether a character or sequence is half collected, waiting on more bytes.
	pub fn is_partial(&self) -> bool {
		self.state != State::Ground || self.utf8_len > 0
	}

	/// Consumes `bytes`, appending what they ask for to `out`.
	pub fn advance(&mut self, bytes: &[u8], out: &mut Vec<Act>) {
		for b in bytes {
			self.byte(*b, out);
		}
	}

	/// Consumes one byte.
	fn byte(&mut self, b: u8, out: &mut Vec<Act>) {
		match self.state {
			State::Ground		=> self.ground(b, out),
			State::Escape		=> self.escape(b, out),
			State::EscapeInter	=> self.escape_inter(b, out),
			State::CsiEntry		=> self.csi_entry(b, out),
			State::CsiParam		=> self.csi_param(b, out),
			State::CsiInter		=> self.csi_inter(b, out),
			State::CsiIgnore	=> self.csi_ignore(b, out),
			State::OscString	=> self.osc_string(b, out),
			State::OscIgnore	=> self.osc_ignore(b, out),
			State::StringEsc	=> self.string_esc(b, out),
			State::StringIgnore	=> self.string_ignore(b, out),
			State::StringIgnoreEsc	=> self.string_ignore_esc(b, out),
		}
	}

	// ┌─────────────────────────────┐
	// │ GROUND                      │
	// └─────────────────────────────┘

	/// Ordinary text, where UTF-8 is decoded and C0 controls are acted on.
	fn ground(&mut self, b: u8, out: &mut Vec<Act>) {
		if self.utf8_len > 0 {
			// A character is part collected. Only a continuation byte may follow.
			if b & 0xC0 == 0x80 {
				self.utf8[self.utf8_len] = b;
				self.utf8_len += 1;
				if self.utf8_len == self.utf8_need {
					self.emit_utf8(out);
				}
				return;
			}
			// Anything else truncates the character.
			out.push(Act::Print(REPLACEMENT));
			self.utf8_len = 0;
			self.utf8_need = 0;
			// Fall through and reconsider this byte as a fresh one.
		}
		if b < 0x20 || b == 0x7F {
			self.control(b, out);
			return;
		}
		if b < 0x80 {
			out.push(Act::Print(b as char));
			return;
		}
		// A UTF-8 lead byte, or a stray continuation byte.
		let need = if b & 0xE0 == 0xC0 {
			2
		} else if b & 0xF0 == 0xE0 {
			3
		} else if b & 0xF8 == 0xF0 {
			4
		} else {
			out.push(Act::Print(REPLACEMENT));
			return;
		};
		self.utf8[0] = b;
		self.utf8_len = 1;
		self.utf8_need = need;
	}

	/// Turns the held bytes into a character, or into a replacement if they are not valid.
	fn emit_utf8(&mut self, out: &mut Vec<Act>) {
		let c = match std::str::from_utf8(&self.utf8[..self.utf8_len]) {
			Ok(s)	=> match s.chars().next() {
				Some(c)	=> c,
				None	=> REPLACEMENT,
			},
			Err(_)	=> REPLACEMENT,
		};
		out.push(Act::Print(c));
		self.utf8_len = 0;
		self.utf8_need = 0;
	}

	/// Acts on a C0 control byte, wherever in the stream it appears.
	fn control(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x07	=> out.push(Act::Ctrl(C0::Bell)),
			0x08	=> out.push(Act::Ctrl(C0::Backspace)),
			0x09	=> out.push(Act::Ctrl(C0::Tab)),
			0x0A | 0x0B | 0x0C	=> out.push(Act::Ctrl(C0::LineFeed)),
			0x0D	=> out.push(Act::Ctrl(C0::CarriageReturn)),
			0x0E	=> out.push(Act::Ctrl(C0::ShiftOut)),
			0x0F	=> out.push(Act::Ctrl(C0::ShiftIn)),
			0x18 | 0x1A	=> self.abandon(),
			0x1B	=> self.begin_escape(),
			// Everything else is consumed without effect.
			_	=> {}
		}
	}

	/// Drops whatever sequence is in progress and returns to ordinary text.
	fn abandon(&mut self) {
		self.state = State::Ground;
		self.private = None;
		self.inter = None;
		self.inter_over = false;
		self.params = Params::default();
		self.string.clear();
	}

	/// Starts a fresh escape sequence, dropping anything already in progress.
	fn begin_escape(&mut self) {
		self.abandon();
		self.state = State::Escape;
	}

	// ┌─────────────────────────────┐
	// │ ESCAPE                      │
	// └─────────────────────────────┘

	/// Immediately after `ESC`.
	fn escape(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x00..=0x17 | 0x19 | 0x1C..=0x1F	=> self.control(b, out),
			0x18 | 0x1A	=> self.abandon(),
			0x1B		=> self.begin_escape(),
			0x20..=0x2F	=> {
				self.inter = Some(b);
				self.state = State::EscapeInter;
			}
			0x5B		=> {
				// CSI.
				self.state = State::CsiEntry;
			}
			0x5D		=> {
				// OSC.
				self.string.clear();
				self.state = State::OscString;
			}
			0x50 | 0x58 | 0x5E | 0x5F	=> {
				// DCS, SOS, PM and APC, none of which the screen model acts on.
				self.state = State::StringIgnore;
			}
			_		=> {
				out.push(Act::Esc(Esc { inter: None, fin: b }));
				self.state = State::Ground;
			}
		}
	}

	/// After `ESC` and one intermediate, as in `ESC # 8`.
	fn escape_inter(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x00..=0x17 | 0x19 | 0x1C..=0x1F	=> self.control(b, out),
			0x18 | 0x1A	=> self.abandon(),
			0x1B		=> self.begin_escape(),
			// A second intermediate is collected but not stored; the sequence is still consumed.
			0x20..=0x2F	=> self.inter_over = true,
			_		=> {
				if !self.inter_over {
					out.push(Act::Esc(Esc { inter: self.inter, fin: b }));
				}
				self.abandon();
			}
		}
	}

	// ┌─────────────────────────────┐
	// │ CSI                         │
	// └─────────────────────────────┘

	/// Immediately after `CSI`, where a private marker may still appear.
	fn csi_entry(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x00..=0x17 | 0x19 | 0x1C..=0x1F	=> self.control(b, out),
			0x18 | 0x1A	=> self.abandon(),
			0x1B		=> self.begin_escape(),
			0x3C..=0x3F	=> {
				self.private = Some(b);
				self.state = State::CsiParam;
			}
			0x30..=0x39	=> {
				self.params.digit((b - 0x30) as u32);
				self.state = State::CsiParam;
			}
			0x3A		=> {
				self.params.separate(true);
				self.state = State::CsiParam;
			}
			0x3B		=> {
				self.params.separate(false);
				self.state = State::CsiParam;
			}
			0x20..=0x2F	=> {
				self.inter = Some(b);
				self.state = State::CsiInter;
			}
			0x40..=0x7E	=> self.dispatch_csi(b, out),
			_		=> self.state = State::CsiIgnore,
		}
	}

	/// Collecting `CSI` parameters.
	fn csi_param(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x00..=0x17 | 0x19 | 0x1C..=0x1F	=> self.control(b, out),
			0x18 | 0x1A	=> self.abandon(),
			0x1B		=> self.begin_escape(),
			0x30..=0x39	=> self.params.digit((b - 0x30) as u32),
			0x3A		=> self.params.separate(true),
			0x3B		=> self.params.separate(false),
			// A private marker after the parameters have started is malformed.
			0x3C..=0x3F	=> self.state = State::CsiIgnore,
			0x20..=0x2F	=> {
				self.inter = Some(b);
				self.state = State::CsiInter;
			}
			0x40..=0x7E	=> self.dispatch_csi(b, out),
			_		=> self.state = State::CsiIgnore,
		}
	}

	/// After a `CSI` intermediate, where no further parameter may appear.
	fn csi_inter(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x00..=0x17 | 0x19 | 0x1C..=0x1F	=> self.control(b, out),
			0x18 | 0x1A	=> self.abandon(),
			0x1B		=> self.begin_escape(),
			0x20..=0x2F	=> self.inter_over = true,
			0x30..=0x3F	=> self.state = State::CsiIgnore,
			0x40..=0x7E	=> self.dispatch_csi(b, out),
			_		=> self.state = State::CsiIgnore,
		}
	}

	/// Consuming a malformed `CSI` to its end.
	fn csi_ignore(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x00..=0x17 | 0x19 | 0x1C..=0x1F	=> self.control(b, out),
			0x18 | 0x1A	=> self.abandon(),
			0x1B		=> self.begin_escape(),
			0x40..=0x7E	=> self.abandon(),
			_		=> {}
		}
	}

	/// Hands a completed `CSI` to the caller, unless it was malformed.
	///
	/// A sequence whose parameters overflowed is dropped rather than acted on with clamped values.
	/// A terminal that clamps `CSI 99999999999 ; 1 H` moves the cursor somewhere the sender never
	/// asked for; dropping it leaves the screen alone, which is what tmux and xterm do.
	fn dispatch_csi(&mut self, fin: u8, out: &mut Vec<Act>) {
		if !self.params.overflowed() && !self.inter_over {
			out.push(Act::Csi(Csi {
				private:	self.private,
				inter:		self.inter,
				params:		self.params,
				fin,
			}));
		}
		self.abandon();
	}

	// ┌─────────────────────────────┐
	// │ STRING PAYLOADS             │
	// └─────────────────────────────┘

	/// Collecting an `OSC` payload.
	fn osc_string(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x07	=> {
				self.dispatch_osc(out);
				self.state = State::Ground;
			}
			0x1B	=> self.state = State::StringEsc,
			0x18 | 0x1A	=> self.abandon(),
			// Other C0 controls end the payload without dispatching it and are then acted on,
			// which is how a stream that lost its terminator recovers at the next line break
			// instead of swallowing everything after it.
			0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1C..=0x1F	=> {
				self.abandon();
				self.control(b, out);
			}
			_	=> {
				if self.string.len() >= MAX_STRING_BYTES {
					// Too long to be a command anyone meant. Keep scanning for the
					// terminator, but store nothing more.
					self.string.clear();
					self.state = State::OscIgnore;
				} else {
					self.string.push(b);
				}
			}
		}
	}

	/// Consuming an over long `OSC` to its end.
	fn osc_ignore(&mut self, b: u8, out: &mut Vec<Act>) {
		match b {
			0x07	=> self.abandon(),
			0x1B	=> self.state = State::StringIgnoreEsc,
			0x18 | 0x1A	=> self.abandon(),
			0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1C..=0x1F	=> {
				self.abandon();
				self.control(b, out);
			}
			_	=> {}
		}
	}

	/// An `ESC` inside an `OSC` payload, which is `ST` if a backslash follows.
	fn string_esc(&mut self, b: u8, out: &mut Vec<Act>) {
		if b == 0x5C {
			self.dispatch_osc(out);
			self.state = State::Ground;
		} else {
			// Not a terminator, so the payload is over and a new escape sequence has begun.
			self.dispatch_osc(out);
			self.begin_escape();
			self.escape(b, out);
		}
	}

	/// Consuming a `DCS`, `SOS`, `PM` or `APC` payload to its end.
	fn string_ignore(&mut self, b: u8, _out: &mut Vec<Act>) {
		match b {
			0x1B	=> self.state = State::StringIgnoreEsc,
			0x18 | 0x1A	=> self.abandon(),
			_	=> {}
		}
	}

	/// An `ESC` inside a payload that is being ignored.
	fn string_ignore_esc(&mut self, b: u8, out: &mut Vec<Act>) {
		if b == 0x5C {
			self.abandon();
		} else {
			self.begin_escape();
			self.escape(b, out);
		}
	}

	/// Splits a collected `OSC` payload into its identifier and its text, and hands it over.
	fn dispatch_osc(&mut self, out: &mut Vec<Act>) {
		let raw = String::from_utf8_lossy(&self.string).into_owned();
		self.string.clear();
		let (ident, text) = match raw.find(';') {
			Some(i)	=> {
				let head = &raw[..i];
				let tail = &raw[i + 1..];
				(head.parse::<u32>().ok(), tail.to_string())
			}
			None	=> (raw.parse::<u32>().ok(), fmt!("")),
		};
		out.push(Act::Osc(Osc { ident, text }));
	}
}
