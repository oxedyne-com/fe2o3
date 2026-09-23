// U3 owns this file. `numbering(pattern, ..nums)`: the one implementation of Typst numbering patterns
// (`"1.a)"`, `"I"`, `"*"`); heading, figure, list, footnote and page numbering (U5, U8) all call `apply`.
// The twenty-three numeral systems and their tables were read off the `typst` 0.15.1 oracle: bijective
// alphabets for Latin, kana and Hangul, greedy additive tables for Roman, Greek and Hebrew, positional
// digit sets, circled forms, and Chinese with ten-thousand grouping.

use crate::eval::args::Args;
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::lib::foundations::{
	mismatch,
	need,
};
use crate::eval::scope::Scope;
use crate::eval::value::Value;
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum NumberingFn {
		Numbering		=> "numbering",
	}
}

pub fn define(scope: &mut Scope) {
	scope.define("numbering", Value::Func(Func::Native(NativeFunc::Numbering(NumberingFn::Numbering))));
}

pub fn call(_f: NumberingFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	let numbering = res!(need(engine, &mut args, "numbering"));
	let rest = res!(args.all::<Value>());
	let mut nums = Vec::with_capacity(rest.len());
	for v in rest {
		match v {
			Value::Int(i) if i >= 0	=> nums.push(i as u64),
			Value::Int(_)			=> return Err(engine.error(span, "number must be at least zero")),
			other					=> return Err(mismatch(engine, span, "integer", &other)),
		}
	}
	res!(crate::eval::lib::foundations::finish(engine, args));
	apply_at(engine, span, &numbering, &nums)
}

/// Formats `nums` with a pattern string (`"1.1"`) or calls a numbering function.
pub fn apply(engine: &mut Engine, numbering: &Value, nums: &[u64]) -> Outcome<Value> {
	apply_at(engine, Span::detached(), numbering, nums)
}

fn apply_at(engine: &mut Engine, span: Span, numbering: &Value, nums: &[u64]) -> Outcome<Value> {
	match numbering {
		Value::Str(s) => {
			let pat = match Pattern::parse(s) {
				Some(p)	=> p,
				None	=> return Err(engine.error(span, "invalid numbering pattern")),
			};
			for (i, n) in nums.iter().enumerate() {
				if *n == 0 {
					let kind = pat.kind_at(i);
					if let Some(name) = kind.zeroless_name() {
						engine.warn(span, fmt!("the numeral system `{}` cannot represent zero", name));
					}
				}
			}
			Ok(Value::str(pat.apply(nums)))
		}
		Value::Func(f) => {
			let mut a = Args::new(span);
			for n in nums {
				a.push(span, Value::Int(*n as i64));
			}
			engine.call_func(f, a)
		}
		other => Err(mismatch(engine, span, "string or function", other)),
	}
}

/// A parsed pattern: each counting symbol with the text before it, and the text after the last one.
#[derive(Clone, Debug)]
pub struct Pattern {
	pub pieces:		Vec<(String, Kind)>,
	pub suffix:		String,
	pub trimmed:	bool,	// drop the first prefix and the suffix, as heading references do
}

impl Pattern {
	pub fn parse(s: &str) -> Option<Self> {
		let mut pieces = Vec::new();
		let mut handled = 0;
		for (i, c) in s.char_indices() {
			if let Some(k) = Kind::from_char(c) {
				pieces.push((s[handled..i].to_string(), k));
				handled = i + c.len_utf8();
			}
		}
		if pieces.is_empty() {
			return None;
		}
		Some(Self { pieces, suffix: s[handled..].to_string(), trimmed: false })
	}

	fn kind_at(&self, i: usize) -> Kind {
		match self.pieces.get(i).or(self.pieces.last()) {
			Some((_, k))	=> *k,
			None			=> Kind::Arabic,
		}
	}

	pub fn apply(&self, nums: &[u64]) -> String {
		let mut out = String::new();
		let mut it = nums.iter();
		for (i, ((prefix, kind), n)) in self.pieces.iter().zip(&mut it).enumerate() {
			if i > 0 || !self.trimmed {
				out.push_str(prefix);
			}
			out.push_str(&kind.apply(*n));
		}
		if let Some((prefix, kind)) = self.pieces.last() {
			for n in it {
				if prefix.is_empty() {
					out.push_str(&self.suffix);
				} else {
					out.push_str(prefix);
				}
				out.push_str(&kind.apply(*n));
			}
		}
		if !self.trimmed {
			out.push_str(&self.suffix);
		}
		out
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	Arabic,
	LowerLatin,
	UpperLatin,
	LowerRoman,
	UpperRoman,
	LowerGreek,
	UpperGreek,
	Symbol,
	Hebrew,
	LowerChinese,
	UpperChinese,
	HiraganaAiueo,
	HiraganaIroha,
	KatakanaAiueo,
	KatakanaIroha,
	KoreanJamo,
	KoreanSyllable,
	EasternArabic,
	Persian,
	Devanagari,
	Bengali,
	Circled,
	DoubleCircled,
}

const HIRAGANA_AIUEO: &str = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん";
const HIRAGANA_IROHA: &str = "いろはにほへとちりぬるをわかよたれそつねならむうゐのおくやまけふこえてあさきゆめみしゑひもせす";
const KATAKANA_AIUEO: &str = "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン";
const KATAKANA_IROHA: &str = "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス";
const KOREAN_JAMO: &str = "ㄱㄴㄷㄹㅁㅂㅅㅇㅈㅊㅋㅌㅍㅎ";
const KOREAN_SYLLABLE: &str = "가나다라마바사아자차카타파하";
const SYMBOLS: [&str; 6] = ["*", "†", "‡", "§", "¶", "‖"];

const ROMAN: [(&str, u64); 20] = [
	("M̅", 1_000_000), ("D̅", 500_000), ("C̅", 100_000), ("L̅", 50_000), ("X̅", 10_000), ("V̅", 5_000),
	("I̅V̅", 4_000), ("M", 1000), ("CM", 900), ("D", 500), ("CD", 400), ("C", 100), ("XC", 90),
	("L", 50), ("XL", 40), ("X", 10), ("IX", 9), ("V", 5), ("IV", 4), ("I", 1),
];

const GREEK_LOWER: [(&str, u64); 36] = [
	("͵θ", 9000), ("͵η", 8000), ("͵ζ", 7000), ("͵ϛ", 6000), ("͵ε", 5000), ("͵δ", 4000), ("͵γ", 3000),
	("͵β", 2000), ("͵α", 1000), ("ϡ", 900), ("ω", 800), ("ψ", 700), ("χ", 600), ("φ", 500), ("υ", 400),
	("τ", 300), ("σ", 200), ("ρ", 100), ("ϟ", 90), ("π", 80), ("ο", 70), ("ξ", 60), ("ν", 50), ("μ", 40),
	("λ", 30), ("κ", 20), ("ι", 10), ("θ", 9), ("η", 8), ("ζ", 7), ("στ", 6), ("ε", 5), ("δ", 4), ("γ", 3),
	("β", 2), ("α", 1),
];

const GREEK_UPPER: [(&str, u64); 36] = [
	("͵Θ", 9000), ("͵Η", 8000), ("͵Ζ", 7000), ("͵Ϛ", 6000), ("͵Ε", 5000), ("͵Δ", 4000), ("͵Γ", 3000),
	("͵Β", 2000), ("͵Α", 1000), ("Ϡ", 900), ("Ω", 800), ("Ψ", 700), ("Χ", 600), ("Φ", 500), ("Υ", 400),
	("Τ", 300), ("Σ", 200), ("Ρ", 100), ("Ϟ", 90), ("Π", 80), ("Ο", 70), ("Ξ", 60), ("Ν", 50), ("Μ", 40),
	("Λ", 30), ("Κ", 20), ("Ι", 10), ("Θ", 9), ("Η", 8), ("Ζ", 7), ("ΣΤ", 6), ("Ε", 5), ("Δ", 4), ("Γ", 3),
	("Β", 2), ("Α", 1),
];

const HEBREW: [(&str, u64); 22] = [
	("ת", 400), ("ש", 300), ("ר", 200), ("ק", 100), ("צ", 90), ("פ", 80), ("ע", 70), ("ס", 60), ("נ", 50),
	("מ", 40), ("ל", 30), ("כ", 20), ("י", 10), ("ט", 9), ("ח", 8), ("ז", 7), ("ו", 6), ("ה", 5), ("ד", 4),
	("ג", 3), ("ב", 2), ("א", 1),
];

const CHINESE_LOWER_DIGITS: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
const CHINESE_UPPER_DIGITS: [&str; 10] = ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"];
const CHINESE_LOWER_UNITS: [&str; 3] = ["十", "百", "千"];
const CHINESE_UPPER_UNITS: [&str; 3] = ["拾", "佰", "仟"];
const CHINESE_GROUPS: [&str; 5] = ["", "万", "亿", "兆", "京"];

impl Kind {
	pub fn from_char(c: char) -> Option<Kind> {
		let k = match c {
			'1'	=> Kind::Arabic,
			'a'	=> Kind::LowerLatin,
			'A'	=> Kind::UpperLatin,
			'i'	=> Kind::LowerRoman,
			'I'	=> Kind::UpperRoman,
			'α'	=> Kind::LowerGreek,
			'Α'	=> Kind::UpperGreek,
			'*'	=> Kind::Symbol,
			'א'	=> Kind::Hebrew,
			'一'	=> Kind::LowerChinese,
			'壹'	=> Kind::UpperChinese,
			'あ'	=> Kind::HiraganaAiueo,
			'い'	=> Kind::HiraganaIroha,
			'ア'	=> Kind::KatakanaAiueo,
			'イ'	=> Kind::KatakanaIroha,
			'ㄱ'	=> Kind::KoreanJamo,
			'가'	=> Kind::KoreanSyllable,
			'١'	=> Kind::EasternArabic,
			'۱'	=> Kind::Persian,
			'१'	=> Kind::Devanagari,
			'১'	=> Kind::Bengali,
			'①'	=> Kind::Circled,
			'⓵'	=> Kind::DoubleCircled,
			_	=> return None,
		};
		Some(k)
	}

	/// The system's name in Typst's "cannot represent zero" warning, for the systems that warn.
	fn zeroless_name(self) -> Option<&'static str> {
		match self {
			Kind::LowerLatin	=> Some("latin"),
			Kind::UpperLatin	=> Some("Latin"),
			_					=> None,
		}
	}

	pub fn apply(self, n: u64) -> String {
		match self {
			Kind::Arabic			=> n.to_string(),
			Kind::LowerLatin		=> bijective(n, "abcdefghijklmnopqrstuvwxyz"),
			Kind::UpperLatin		=> bijective(n, "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
			Kind::LowerRoman		=> if n == 0 { "n".to_string() } else { additive(n, &ROMAN).to_lowercase() },
			Kind::UpperRoman		=> if n == 0 { "N".to_string() } else { additive(n, &ROMAN) },
			Kind::LowerGreek		=> if n == 0 { "𐆊".to_string() } else { additive(n, &GREEK_LOWER) },
			Kind::UpperGreek		=> if n == 0 { "𐆊".to_string() } else { additive(n, &GREEK_UPPER) },
			Kind::Symbol			=> if n == 0 {
				"0".to_string()
			} else {
				SYMBOLS[((n - 1) % 6) as usize].repeat(((n - 1) / 6 + 1) as usize)
			},
			Kind::Hebrew			=> hebrew(n),
			Kind::LowerChinese		=> chinese(n, &CHINESE_LOWER_DIGITS, &CHINESE_LOWER_UNITS),
			Kind::UpperChinese		=> chinese(n, &CHINESE_UPPER_DIGITS, &CHINESE_UPPER_UNITS),
			Kind::HiraganaAiueo		=> bijective(n, HIRAGANA_AIUEO),
			Kind::HiraganaIroha		=> bijective(n, HIRAGANA_IROHA),
			Kind::KatakanaAiueo		=> bijective(n, KATAKANA_AIUEO),
			Kind::KatakanaIroha		=> bijective(n, KATAKANA_IROHA),
			Kind::KoreanJamo		=> bijective(n, KOREAN_JAMO),
			Kind::KoreanSyllable	=> bijective(n, KOREAN_SYLLABLE),
			Kind::EasternArabic		=> digits(n, '٠'),
			Kind::Persian			=> digits(n, '۰'),
			Kind::Devanagari		=> digits(n, '०'),
			Kind::Bengali			=> digits(n, '০'),
			Kind::Circled			=> circled(n),
			Kind::DoubleCircled		=> match n {
				1..=10	=> char::from_u32(0x24F5 + n as u32 - 1).map(|c| c.to_string()).unwrap_or_default(),
				_		=> n.to_string(),
			},
		}
	}
}

// Bijective base-k over an alphabet: 1 is the first letter, k + 1 the first letter twice. Zero, which
// such a system cannot write, falls back to "0".
fn bijective(n: u64, alphabet: &str) -> String {
	if n == 0 {
		return "0".to_string();
	}
	let letters: Vec<char> = alphabet.chars().collect();
	let k = letters.len() as u64;
	let mut n = n;
	let mut out = Vec::new();
	while n > 0 {
		n -= 1;
		out.push(letters[(n % k) as usize]);
		n /= k;
	}
	out.iter().rev().collect()
}

fn additive(n: u64, table: &[(&str, u64)]) -> String {
	let mut n = n;
	let mut out = String::new();
	for (sym, v) in table {
		while n >= *v {
			out.push_str(sym);
			n -= v;
		}
	}
	out
}

fn hebrew(n: u64) -> String {
	if n == 0 {
		return "0".to_string();
	}
	// 15 and 16 are written 9 + 6 and 9 + 7, avoiding spellings of the divine name.
	let mut n = n;
	let mut out = String::new();
	for (sym, v) in &HEBREW {
		while n >= *v {
			match n {
				15 => { out.push_str("טו"); return out; }
				16 => { out.push_str("טז"); return out; }
				_  => { out.push_str(sym); n -= v; }
			}
		}
	}
	out
}

fn digits(n: u64, zero: char) -> String {
	n.to_string().chars().map(|d| {
		let k = d.to_digit(10).unwrap_or(0);
		char::from_u32(zero as u32 + k).unwrap_or(d)
	}).collect()
}

fn circled(n: u64) -> String {
	let c = match n {
		0		=> Some('⓪'),
		1..=20	=> char::from_u32(0x2460 + n as u32 - 1),
		21..=35	=> char::from_u32(0x3251 + n as u32 - 21),
		36..=50	=> char::from_u32(0x32B1 + n as u32 - 36),
		_		=> None,
	};
	match c {
		Some(c)	=> c.to_string(),
		None	=> n.to_string(),
	}
}

// Chinese numerals with ten-thousand grouping (万, 亿, 兆, 京): a zero inside or between groups is
// written once as 零, and a leading 一十 is shortened to 十.
fn chinese(n: u64, dig: &[&str; 10], units: &[&str; 3]) -> String {
	if n == 0 {
		return dig[0].to_string();
	}
	let mut groups = Vec::new();
	let mut m = n;
	while m > 0 {
		groups.push((m % 10_000) as u32);
		m /= 10_000;
	}
	let mut out = String::new();
	let mut zero_pending = false;
	let top = groups.len() - 1;
	for gi in (0..groups.len()).rev() {
		let g = groups[gi];
		if g == 0 {
			zero_pending = !out.is_empty();
			continue;
		}
		if !out.is_empty() && (zero_pending || g < 1000) {
			out.push_str(dig[0]);
		}
		zero_pending = false;
		out.push_str(&chinese_group(g, gi == top, dig, units));
		out.push_str(CHINESE_GROUPS.get(gi).copied().unwrap_or(""));
	}
	out
}

fn chinese_group(g: u32, leading: bool, dig: &[&str; 10], units: &[&str; 3]) -> String {
	if leading && (10..20).contains(&g) {
		let mut s = units[0].to_string();
		if g > 10 {
			s.push_str(dig[(g - 10) as usize]);
		}
		return s;
	}
	let ds = [g / 1000, g / 100 % 10, g / 10 % 10, g % 10];
	let mut out = String::new();
	let mut zero = false;
	for (i, d) in ds.iter().enumerate() {
		if *d == 0 {
			zero = !out.is_empty();
			continue;
		}
		if zero {
			out.push_str(dig[0]);
			zero = false;
		}
		out.push_str(dig[*d as usize]);
		if i < 3 {
			out.push_str(units[2 - i]);
		}
	}
	out
}
