// U3 owns this file. Typst's `decimal`: a 96-bit integer mantissa, a scale of 0 to 28 decimal places and
// a sign, with the arithmetic of the `rust_decimal` crate Typst 0.15 uses (itself a port of .NET's
// `System.Decimal`), so a result carries the digits, the scale and even the sign of zero that Typst's
// does. The algorithms follow rust_decimal 1.39: an exact result reduced to 96 bits by round half to
// even over every discarded digit, division extended a chunk of digits at a time and then stripped of
// trailing zeros, and a float read through rust_decimal's base-2 walk.

use oxedyne_fe2o3_core::prelude::*;

use std::cmp::Ordering;

const MAX_SCALE:	u32		= 28;
const MAX_M:		u128	= (1u128 << 96) - 1;
const LIMBS:		usize	= 8;	// a 256-bit working integer, room for any product or aligned sum

// Wide unsigned arithmetic, little-endian 32-bit limbs.
type W = [u32; LIMBS];

fn w_from(x: u128) -> W {
	let mut w = [0u32; LIMBS];
	for (i, l) in w.iter_mut().enumerate().take(4) {
		*l = (x >> (32 * i)) as u32;
	}
	w
}

fn w_fits96(w: &W) -> bool { w.iter().skip(3).all(|l| *l == 0) }

fn w_to_u128(w: &W) -> u128 {
	w.iter().take(4).enumerate().fold(0u128, |a, (i, l)| a | ((*l as u128) << (32 * i)))
}

// Multiplies in place by a small factor; true when it overflowed 256 bits.
fn w_mul_small(w: &mut W, f: u32) -> bool {
	let mut carry = 0u64;
	for l in w.iter_mut() {
		let t = (*l as u64) * (f as u64) + carry;
		*l = t as u32;
		carry = t >> 32;
	}
	carry != 0
}

// Divides in place by a small divisor, returning the remainder.
fn w_div_small(w: &mut W, d: u32) -> u32 {
	let mut rem = 0u64;
	for l in w.iter_mut().rev() {
		let t = (rem << 32) | (*l as u64);
		*l = (t / d as u64) as u32;
		rem = t % d as u64;
	}
	rem as u32
}

fn w_mul(a: u128, b: u128) -> W {
	let (x, y) = (w_from(a), w_from(b));
	let mut out = [0u32; LIMBS];
	for i in 0..4 {
		let mut carry = 0u64;
		for j in 0..4 {
			let k = i + j;
			let t = (x[i] as u64) * (y[j] as u64) + (out[k] as u64) + carry;
			out[k] = t as u32;
			carry = t >> 32;
		}
		let mut k = i + 4;
		while carry != 0 && k < LIMBS {
			let t = (out[k] as u64) + carry;
			out[k] = t as u32;
			carry = t >> 32;
			k += 1;
		}
	}
	out
}

fn w_add(a: &W, b: &W) -> W {
	let mut out = [0u32; LIMBS];
	let mut carry = 0u64;
	for i in 0..LIMBS {
		let t = (a[i] as u64) + (b[i] as u64) + carry;
		out[i] = t as u32;
		carry = t >> 32;
	}
	out
}

// `a - b` for `a >= b`.
fn w_sub(a: &W, b: &W) -> W {
	let mut out = [0u32; LIMBS];
	let mut borrow = 0i64;
	for i in 0..LIMBS {
		let mut t = (a[i] as i64) - (b[i] as i64) - borrow;
		borrow = 0;
		if t < 0 {
			t += 1i64 << 32;
			borrow = 1;
		}
		out[i] = t as u32;
	}
	out
}

fn w_cmp(a: &W, b: &W) -> Ordering {
	for i in (0..LIMBS).rev() {
		match a[i].cmp(&b[i]) {
			Ordering::Equal	=> continue,
			o				=> return o,
		}
	}
	Ordering::Equal
}

// Multiplies by ten `k` times; true on overflow.
fn w_scale_up(w: &mut W, k: u32) -> bool {
	for _ in 0..k {
		if w_mul_small(w, 10) {
			return true;
		}
	}
	false
}

/// A Typst decimal. Equality and ordering are by value (`1.0 == 1.00`); the scale survives for display.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decimal {
	m:		u128,	// below 2^96
	scale:	u32,	// 0 to 28 digits after the point
	neg:	bool,
}

impl PartialEq for Decimal {
	fn eq(&self, other: &Self) -> bool { self.cmp(other) == Ordering::Equal }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for Decimal {
	fn cmp(&self, other: &Self) -> Ordering {
		let (za, zb) = (self.is_zero(), other.is_zero());
		if za && zb {
			return Ordering::Equal;
		}
		let (na, nb) = (self.neg && !za, other.neg && !zb);
		if na != nb {
			return if na { Ordering::Less } else { Ordering::Greater };
		}
		let (a, b) = aligned(self, other);
		let mag = match (a, b) {
			(Some(a), Some(b))	=> w_cmp(&a, &b),
			_					=> Ordering::Equal,
		};
		if na { mag.reverse() } else { mag }
	}
}

// Both mantissas at the larger scale.
fn aligned(a: &Decimal, b: &Decimal) -> (Option<W>, Option<W>) {
	let s = a.scale.max(b.scale);
	let mut x = w_from(a.m);
	let mut y = w_from(b.m);
	let ox = w_scale_up(&mut x, s - a.scale);
	let oy = w_scale_up(&mut y, s - b.scale);
	(if ox { None } else { Some(x) }, if oy { None } else { Some(y) })
}

impl From<i64> for Decimal {
	fn from(i: i64) -> Self { Decimal { m: i.unsigned_abs() as u128, scale: 0, neg: i < 0 } }
}

impl Decimal {
	pub const ZERO:	Decimal = Decimal { m: 0, scale: 0, neg: false };
	pub const ONE:	Decimal = Decimal { m: 1, scale: 0, neg: false };

	pub fn is_zero(&self) -> bool { self.m == 0 }

	/// Is the sign bit set? A negative zero is, as in rust_decimal.
	pub fn is_negative(&self) -> bool { self.neg }

	pub fn scale(&self) -> u32 { self.scale }

	pub fn neg(self) -> Self { Decimal { neg: !self.neg, ..self } }

	pub fn abs(self) -> Self { Decimal { neg: false, ..self } }

	/// Does the value have no fractional part?
	pub fn is_integer(&self) -> bool {
		let mut m = self.m;
		for _ in 0..self.scale {
			if m % 10 != 0 {
				return false;
			}
			m /= 10;
		}
		true
	}

	/// rust_decimal's `from_str_exact`: an optional sign, digits with `_` separators after the first
	/// digit, one optional point, no exponent; more than 28 decimal places or 96 bits is an error.
	pub fn parse(s: &str) -> Option<Self> {
		let b = s.as_bytes();
		// rust_decimal takes its 96-bit path from 18 bytes on, where 28 decimal places is the limit.
		let big = b.len() >= 18;
		let mut i = 0;
		let mut neg = false;
		match b.first() {
			Some(b'-')	=> { neg = true; i = 1; }
			Some(b'+')	=> i = 1,
			_			=> (),
		}
		let (mut m, mut scale, mut point, mut has) = (0u128, 0u32, false, false);
		while i < b.len() {
			let c = b[i];
			match c {
				b'0'..=b'9' => {
					let next = m * 10 + (c - b'0') as u128;
					if next > MAX_M {
						return None;
					}
					m = next;
					has = true;
					if point {
						scale += 1;
						if big && scale >= MAX_SCALE && i + 1 < b.len() {
							return None;
						}
					}
				}
				b'.' if !point	=> point = true,
				b'_' if has		=> (),
				_				=> return None,
			}
			i += 1;
		}
		if !has || scale > MAX_SCALE {
			return None;
		}
		Some(Decimal { m, scale, neg })
	}

	/// rust_decimal's `from_f64_retain`: the float's binary value walked into base ten, keeping every
	/// digit that fits. `None` for NaN, an infinity or a magnitude past 96 bits.
	pub fn from_f64(x: f64) -> Option<Self> {
		if !x.is_finite() {
			return None;
		}
		let raw = x.to_bits();
		let positive = (raw >> 63) == 0;
		let biased = ((raw >> 52) & 0x7ff) as i32;
		let frac = (raw & 0x000f_ffff_ffff_ffff) as u128;
		if biased == 0 && frac == 0 {
			return Some(Decimal { m: 0, scale: 0, neg: !positive });
		}
		let mut exp2 = biased - 1023;
		let mut bits = frac;
		if biased == 0 {
			exp2 += 1;
		} else {
			bits |= 1u128 << 52;
		}
		exp2 -= 52;
		let mut exp5 = -exp2;
		let mut exp10 = exp2;
		while exp5 > 0 {
			if bits & 1 == 0 {
				exp10 += 1;
				exp5 -= 1;
				bits >>= 1;
			} else {
				exp5 -= 1;
				let t = bits * 5;
				if t <= MAX_M {
					bits = t;
				} else {
					exp10 += 1;
					bits >>= 1;
				}
			}
		}
		while exp5 < 0 {
			if bits & (1u128 << 95) == 0 {
				exp10 -= 1;
				exp5 += 1;
				bits <<= 1;
			} else if exp10 * 2 > -exp5 {
				return None;
			} else {
				exp5 += 1;
				bits /= 5;
			}
		}
		while exp10 > 0 {
			let t = bits * 10;
			if t > MAX_M {
				return None;
			}
			bits = t;
			exp10 -= 1;
		}
		while exp10 < -(MAX_SCALE as i32) {
			let rem = bits % 10;
			bits /= 10;
			exp10 += 1;
			if bits == 0 {
				exp10 = 0;
			} else if rem >= 5 {
				bits += 1;
			}
		}
		Some(Decimal { m: bits, scale: (-exp10) as u32, neg: !positive })
	}

	/// rust_decimal's conversion to `f64`: the integral and fractional parts summed, then rounded to
	/// the scale's decimal places.
	pub fn to_f64(&self) -> f64 {
		let v = if self.scale == 0 {
			self.m as f64
		} else {
			let p = 10u128.pow(self.scale);
			let int = (self.m / p) as f64;
			let frac = ((self.m % p) as f64) / (p as f64);
			if frac == 0.0 {
				int
			} else {
				let r = 10f64.powi(self.scale as i32);
				((int + frac) * r).round() / r
			}
		};
		if self.neg { -v } else { v }
	}

	/// The integer part, when it fits in an `i64`.
	pub fn to_i64(&self) -> Option<i64> {
		let t = self.trunc();
		if t.neg {
			if t.m <= (i64::MAX as u128) + 1 { Some((t.m as i128).wrapping_neg() as i64) } else { None }
		} else if t.m <= i64::MAX as u128 {
			Some(t.m as i64)
		} else {
			None
		}
	}

	pub fn trunc(&self) -> Self {
		let mut m = self.m;
		for _ in 0..self.scale {
			m /= 10;
		}
		Decimal { m, scale: 0, neg: self.neg }
	}

	pub fn fract(&self) -> Self {
		match self.checked_sub(self.trunc()) {
			Some(f)	=> f,
			None	=> Decimal::ZERO,
		}
	}

	pub fn floor(&self) -> Self {
		if self.scale == 0 {
			return *self;
		}
		let t = self.trunc();
		if self.neg && !self.fract().is_zero() {
			t.checked_sub(Decimal::ONE).unwrap_or(t)
		} else {
			t
		}
	}

	pub fn ceil(&self) -> Self {
		if self.scale == 0 {
			return *self;
		}
		let t = self.trunc();
		if !self.neg && !self.fract().is_zero() {
			t.checked_add(Decimal::ONE).unwrap_or(t)
		} else {
			t
		}
	}

	/// Rounds half away from zero to `dp` decimal places; a value with fewer places is unchanged.
	pub fn round_dp(&self, dp: u32) -> Self {
		if self.scale <= dp {
			return *self;
		}
		if self.is_zero() {
			return Decimal { m: 0, scale: dp, neg: self.neg };
		}
		let cut = 10u128.pow(self.scale - dp);
		let mut q = self.m / cut;
		if (self.m % cut) * 2 >= cut {
			q += 1;
		}
		Decimal { m: q, scale: dp, neg: self.neg }
	}

	/// Typst's `round(digits)`: half away from zero, negative digits rounding to tens, hundreds and so
	/// on. `None` when multiplying back overflows.
	pub fn round(&self, digits: i64) -> Option<Self> {
		if digits >= 0 {
			let dp = if digits > u32::MAX as i64 { u32::MAX } else { digits as u32 };
			return Some(self.round_dp(dp));
		}
		let d = digits.unsigned_abs();
		let new_scale = (self.scale as u64) + d;
		if new_scale > MAX_SCALE as u64 || d > 28 {
			return Some(Decimal { m: 0, scale: 0, neg: self.neg });
		}
		let shifted = Decimal { m: self.m, scale: new_scale as u32, neg: self.neg }.round_dp(0);
		let ten = Decimal { m: 10u128.pow(d as u32), scale: 0, neg: false };
		shifted.checked_mul(ten)
	}

	pub fn checked_add(self, other: Self) -> Option<Self> { add_sub(self, other, false) }

	pub fn checked_sub(self, other: Self) -> Option<Self> { add_sub(self, other, true) }

	pub fn checked_mul(self, other: Self) -> Option<Self> {
		if self.is_zero() || other.is_zero() {
			return Some(Decimal::ZERO);
		}
		let scale = self.scale + other.scale;
		let neg = self.neg ^ other.neg;
		// Two 32-bit mantissas: past 47 places the product rounds to a plain zero.
		if self.m >> 32 == 0 && other.m >> 32 == 0 && scale > MAX_SCALE + 19 {
			return Some(Decimal::ZERO);
		}
		reduce(w_mul(self.m, other.m), scale, neg)
	}

	pub fn checked_div(self, other: Self) -> Option<Self> {
		if other.is_zero() {
			return None;
		}
		if self.is_zero() {
			return Some(Decimal::ZERO);
		}
		div(self, other)
	}

	/// The remainder with the dividend's sign, at the larger of the two scales.
	pub fn checked_rem(self, other: Self) -> Option<Self> {
		if other.is_zero() {
			return None;
		}
		if self.is_zero() {
			return Some(Decimal::ZERO);
		}
		let (a, b) = match aligned(&self, &other) {
			(Some(a), Some(b))	=> (a, b),
			_					=> return None,
		};
		match w_cmp(&a, &b) {
			Ordering::Equal	=> return Some(Decimal::ZERO),
			Ordering::Less	=> return Some(self),
			Ordering::Greater	=> (),
		}
		let b = w_to_u128(&b);
		// `a` may exceed 96 bits after alignment; the remainder does not.
		let mut rem = 0u128;
		for l in a.iter().rev() {
			rem = ((rem << 32) | (*l as u128)) % b;
		}
		Some(Decimal { m: rem, scale: self.scale.max(other.scale), neg: self.neg })
	}

	pub fn checked_div_euclid(self, other: Self) -> Option<Self> {
		let q = match self.checked_div(other) {
			Some(q)	=> q.trunc(),
			None	=> return None,
		};
		match self.checked_rem(other) {
			Some(r) if r.neg && !r.is_zero() => if !other.neg {
				q.checked_sub(Decimal::ONE)
			} else {
				q.checked_add(Decimal::ONE)
			},
			_ => Some(q),
		}
	}

	pub fn checked_rem_euclid(self, other: Self) -> Option<Self> {
		let r = match self.checked_rem(other) {
			Some(r)	=> r,
			None	=> return None,
		};
		if r.neg && !r.is_zero() { r.checked_add(other.abs()) } else { Some(r) }
	}

	/// rust_decimal's `checked_powi`: squaring and multiplying, then `1 / x^n` for a negative power.
	pub fn checked_powi(self, exp: i64) -> Option<Self> {
		if exp >= 0 {
			return self.checked_powu(exp as u64);
		}
		match self.checked_powu(exp.unsigned_abs()) {
			Some(p)	=> Decimal::ONE.checked_div(p),
			None	=> None,
		}
	}

	fn checked_powu(self, exp: u64) -> Option<Self> {
		if exp == 0 {
			return Some(Decimal::ONE);
		}
		if self.is_zero() {
			return Some(Decimal::ZERO);
		}
		if self == Decimal::ONE {
			return Some(Decimal::ONE);
		}
		match exp {
			1	=> Some(self),
			2	=> self.checked_mul(self),
			_	=> {
				let mut product = Decimal::ONE;
				let mut mask = exp;
				let mut power = self;
				for n in 0..(64 - exp.leading_zeros()) {
					if n > 0 {
						power = match power.checked_mul(power) {
							Some(p)	=> p,
							None	=> return None,
						};
						mask >>= 1;
					}
					if mask & 1 > 0 {
						product = match product.checked_mul(power) {
							Some(p)	=> p,
							None	=> return None,
						};
					}
				}
				Some(product.normalize())
			}
		}
	}

	/// Trailing zeros removed; zero becomes a plain positive zero.
	pub fn normalize(self) -> Self {
		if self.is_zero() {
			return Decimal::ZERO;
		}
		let (mut m, mut scale) = (self.m, self.scale);
		while scale > 0 && m % 10 == 0 {
			m /= 10;
			scale -= 1;
		}
		Decimal { m, scale, neg: self.neg }
	}

	/// The digits with the scale's places, unsigned: `rust_decimal`'s `Display` without its sign.
	pub fn digits(&self) -> String {
		let raw = self.m.to_string();
		let s = self.scale as usize;
		if s == 0 {
			return raw;
		}
		let padded = if raw.len() <= s { fmt!("{}{}", "0".repeat(s + 1 - raw.len()), raw) } else { raw };
		let (int, frac) = padded.split_at(padded.len() - s);
		fmt!("{}.{}", int, frac)
	}

	/// As `repr` quotes it: an ASCII minus, and none on a zero, which rust_decimal prints unsigned.
	pub fn text(&self) -> String {
		if self.neg && !self.is_zero() { fmt!("-{}", self.digits()) } else { self.digits() }
	}

	/// As a document shows it: a typographic minus.
	pub fn display(&self) -> String {
		if self.neg && !self.is_zero() { fmt!("\u{2212}{}", self.digits()) } else { self.digits() }
	}
}

// Reduces an exact mantissa at `scale` to 96 bits and at most 28 places, rounding half to even over
// every discarded digit, as rust_decimal's `rescale`. If rounding carries past 96 bits, one more digit
// goes, rounded afresh.
fn reduce(w: W, scale: u32, neg: bool) -> Option<Decimal> {
	let mut w = w;
	let mut scale = scale as i64;
	let (mut last, mut sticky) = (0u32, false);
	let mut cut = 0u32;
	while !w_fits96(&w) || scale > MAX_SCALE as i64 {
		if scale == 0 {
			return None;
		}
		if cut > 0 {
			sticky |= last != 0;
		}
		last = w_div_small(&mut w, 10);
		cut += 1;
		scale -= 1;
	}
	let mut m = w_to_u128(&w);
	if cut > 0 && (last > 5 || (last == 5 && (sticky || m & 1 == 1))) {
		m += 1;
		if m > MAX_M {
			if scale == 0 {
				return None;
			}
			let d = (m % 10) as u32;
			m /= 10;
			scale -= 1;
			if d > 5 || (d == 5 && m & 1 == 1) {
				m += 1;
			}
		}
	}
	Some(Decimal { m, scale: scale as u32, neg })
}

// Addition and subtraction exactly as rust_decimal signs them, including the sign of a zero result.
fn add_sub(a: Decimal, b: Decimal, subtract: bool) -> Option<Decimal> {
	if a.is_zero() {
		let mut r = b;
		if subtract && !b.is_zero() {
			r.neg = !b.neg;
		}
		return Some(r);
	}
	if b.is_zero() {
		return Some(a);
	}
	let sub = subtract ^ (a.neg != b.neg);
	let (x, y) = match aligned(&a, &b) {
		(Some(x), Some(y))	=> (x, y),
		_					=> return None,
	};
	let scale = a.scale.max(b.scale);
	if !sub {
		return reduce(w_add(&x, &y), scale, a.neg);
	}
	// The magnitude is |x - y|; the sign follows the larger operand. An exact zero keeps the sign
	// rust_decimal's paths give it: the first operand's, except where the first had the larger scale
	// and did not fit its 32-bit fast path, when the aligned operands were swapped.
	match w_cmp(&x, &y) {
		Ordering::Greater	=> reduce(w_sub(&x, &y), scale, a.neg),
		Ordering::Less		=> reduce(w_sub(&y, &x), scale, !a.neg),
		Ordering::Equal		=> {
			let small = a.m >> 32 == 0 && b.m >> 32 == 0 && fast_rescale(&a, &b);
			let neg = if !small && a.scale > b.scale { sub ^ a.neg } else { a.neg };
			Some(Decimal { m: 0, scale, neg })
		}
	}
}

// Whether rust_decimal's 32-bit fast path handles the pair: the rescaled operand must stay in 32 bits.
fn fast_rescale(a: &Decimal, b: &Decimal) -> bool {
	let (small, by) = if a.scale > b.scale { (b.m, a.scale - b.scale) } else { (a.m, b.scale - a.scale) };
	if by > 9 {
		return false;
	}
	small * 10u128.pow(by) <= u32::MAX as u128
}

// The largest power of ten, up to nine, the quotient can take without passing 96 bits, per
// rust_decimal's `find_scale`, which judges from the top 32 bits and so is conservative.
fn find_scale(q: u128, scale: i32) -> Option<u32> {
	const POV: [u128; 8] = [
		MAX_M / 10, MAX_M / 100, MAX_M / 1_000, MAX_M / 10_000, MAX_M / 100_000, MAX_M / 1_000_000,
		MAX_M / 10_000_000, MAX_M / 100_000_000,
	];
	let hi = (q >> 64) as u32;
	let low64 = q as u64;
	if hi > 429_496_729 {
		return if scale < 0 { None } else { Some(0) };
	}
	let hi_of = |i: usize| (POV[i] >> 64) as u32;
	let low_of = |i: usize| POV[i] as u64;
	if scale > MAX_SCALE as i32 - 9 {
		let x = (MAX_SCALE as i32 - scale) as usize;
		if x >= 1 && hi < hi_of(x - 1) {
			return if x as i32 + scale < 0 { None } else { Some(x as u32) };
		}
	} else if hi < 4 || (hi == 4 && low64 <= 5_441_186_219_426_131_129) {
		return Some(9);
	}
	let mut x: usize = if hi > 42_949 {
		if hi > 4_294_967 {
			if hi > 42_949_672 { 1 } else { 2 }
		} else if hi > 429_496 {
			3
		} else {
			4
		}
	} else if hi > 429 {
		if hi > 4_294 { 5 } else { 6 }
	} else if hi > 42 {
		7
	} else {
		8
	};
	if hi == hi_of(x - 1) && low64 > low_of(x - 1) {
		x -= 1;
	}
	if x as i32 + scale < 0 { None } else { Some(x as u32) }
}

// A carry past 96 bits after adding a quotient digit: back one place, rounding half to even with the
// remainder as a sticky digit.
fn unscale_from_overflow(q: u128, scale: i32, sticky: bool) -> Option<(u128, i32)> {
	let scale = scale - 1;
	if scale < 0 {
		return None;
	}
	let d = (q % 10) as u32;
	let mut m = q / 10;
	if d > 5 || (d == 5 && (sticky || m & 1 == 1)) {
		m += 1;
	}
	Some((m, scale))
}

// rust_decimal's division: the integer quotient, extended by up to nine digits at a time while the
// remainder is not zero and there is room, rounded half to even at the end, and, when any remainder
// was met, stripped of trailing zeros.
fn div(a: Decimal, b: Decimal) -> Option<Decimal> {
	let neg = a.neg ^ b.neg;
	let mut scale = a.scale as i32 - b.scale as i32;
	let d = b.m;
	let mut q = a.m / d;
	let mut r = a.m % d;
	let mut unscale = false;
	loop {
		let mut power = 0u32;
		if r == 0 {
			if scale >= 0 {
				break;
			}
			power = 9.min((-scale) as u32);
		} else {
			unscale = true;
			let full = if scale == MAX_SCALE as i32 {
				true
			} else {
				match find_scale(q, scale) {
					Some(s)	=> {
						power = s;
						s == 0
					}
					None	=> return None,
				}
			};
			if full {
				let twice = r * 2;
				if twice > d || (twice == d && q & 1 == 1) {
					q += 1;
					if q > MAX_M {
						match unscale_from_overflow(q, scale, true) {
							Some((m, s))	=> { q = m; scale = s; }
							None			=> return None,
						}
					}
				}
				break;
			}
			let p = 10u128.pow(power);
			scale += power as i32;
			q = match q.checked_mul(p) {
				Some(x) if x <= MAX_M	=> x,
				_						=> return None,
			};
			let rs = r * p;
			q += rs / d;
			r = rs % d;
			if q > MAX_M {
				match unscale_from_overflow(q, scale, r != 0) {
					Some((m, s))	=> { q = m; scale = s; }
					None			=> return None,
				}
				break;
			}
			continue;
		}
		let p = 10u128.pow(power);
		scale += power as i32;
		q = match q.checked_mul(p) {
			Some(x) if x <= MAX_M	=> x,
			_						=> return None,
		};
	}
	// rust_decimal's `unscale`: eight zeros at a time only while the low word is clear, then at most
	// one strip each of four, two and one, so a long run of zeros can survive.
	if unscale {
		while (q as u32) == 0 && scale >= 8 && q % 100_000_000 == 0 {
			q /= 100_000_000;
			scale -= 8;
		}
		for (mask, k, p) in [(0xfu128, 4, 10_000u128), (0x3, 2, 100), (0x1, 1, 10)] {
			if q & mask == 0 && scale >= k && q % p == 0 {
				q /= p;
				scale -= k;
			}
		}
	}
	if scale < 0 {
		return None;
	}
	Some(Decimal { m: q, scale: scale as u32, neg })
}
