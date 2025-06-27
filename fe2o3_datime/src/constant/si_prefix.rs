use oxedyne_fe2o3_core::prelude::*;

/// SI (Système International) unit prefixes.
/// 
/// Refer to https://en.wikipedia.org/wiki/Metric_prefix
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SIPrefix {
	Yotta,
	Zetta,
	Exa,
	Peta,
	Tera,
	Giga,
	Mega,
	Kilo,
	Hecto,
	Deca,
	Deci,
	Centi,
	Milli,
	Micro,
	Nano,
	Pico,
	Femto,
	Atto,
	Zepto,
	Yocto,
}

impl SIPrefix {
	/// Returns the symbol for this prefix.
	pub fn to_symbol(&self) -> &'static str {
		match self {
			Self::Yotta => "Y",
			Self::Zetta => "Z",
			Self::Exa => "E",
			Self::Peta => "P",
			Self::Tera => "T",
			Self::Giga => "G",
			Self::Mega => "M",
			Self::Kilo => "k",
			Self::Hecto => "h",
			Self::Deca => "da",
			Self::Deci => "d",
			Self::Centi => "c",
			Self::Milli => "m",
			Self::Micro => "u",
			Self::Nano => "n",
			Self::Pico => "p",
			Self::Femto => "f",
			Self::Atto => "a",
			Self::Zepto => "z",
			Self::Yocto => "y",
		}
	}
	
	/// Returns the power of 10 for this prefix.
	pub fn to_log10(&self) -> i8 {
		match self {
			Self::Yotta => 24,
			Self::Zetta => 21,
			Self::Exa => 18,
			Self::Peta => 15,
			Self::Tera => 12,
			Self::Giga => 9,
			Self::Mega => 6,
			Self::Kilo => 3,
			Self::Hecto => 2,
			Self::Deca => 1,
			Self::Deci => -1,
			Self::Centi => -2,
			Self::Milli => -3,
			Self::Micro => -6,
			Self::Nano => -9,
			Self::Pico => -12,
			Self::Femto => -15,
			Self::Atto => -18,
			Self::Zepto => -21,
			Self::Yocto => -24,
		}
	}
	
	/// Returns the short name (American English).
	pub fn to_short_name(&self) -> &'static str {
		match self {
			Self::Yotta => "septillion",
			Self::Zetta => "sextillion",
			Self::Exa => "quintillion",
			Self::Peta => "quadrillion",
			Self::Tera => "trillion",
			Self::Giga => "billion",
			Self::Mega => "million",
			Self::Kilo => "thousand",
			Self::Hecto => "hundred",
			Self::Deca => "ten",
			Self::Deci => "tenth",
			Self::Centi => "hundredth",
			Self::Milli => "thousandth",
			Self::Micro => "millionth",
			Self::Nano => "billionth",
			Self::Pico => "trillionth",
			Self::Femto => "quadrillionth",
			Self::Atto => "quintillionth",
			Self::Zepto => "sextillionth",
			Self::Yocto => "septillionth",
		}
	}
	
	/// Returns the long name (European English).
	pub fn to_long_name(&self) -> &'static str {
		match self {
			Self::Yotta => "quadrillion",
			Self::Zetta => "trilliard",
			Self::Exa => "trillion",
			Self::Peta => "billiard",
			Self::Tera => "billion",
			Self::Giga => "milliard",
			Self::Mega => "million",
			Self::Kilo => "thousand",
			Self::Hecto => "hundred",
			Self::Deca => "ten",
			Self::Deci => "tenth",
			Self::Centi => "hundredth",
			Self::Milli => "thousandth",
			Self::Micro => "millionth",
			Self::Nano => "milliardth",
			Self::Pico => "billionth",
			Self::Femto => "billiardth",
			Self::Atto => "trillionth",
			Self::Zepto => "trilliardth",
			Self::Yocto => "quadrillionth",
		}
	}
	
	/// Get prefix by symbol.
	pub fn get_using_symbol(symbol: &str) -> Option<Self> {
		let symbol = symbol.trim();
		match symbol {
			"Y" => Some(Self::Yotta),
			"Z" => Some(Self::Zetta),
			"E" => Some(Self::Exa),
			"P" => Some(Self::Peta),
			"T" => Some(Self::Tera),
			"G" => Some(Self::Giga),
			"M" => Some(Self::Mega),
			"k" => Some(Self::Kilo),
			"h" => Some(Self::Hecto),
			"da" => Some(Self::Deca),
			"d" => Some(Self::Deci),
			"c" => Some(Self::Centi),
			"m" => Some(Self::Milli),
			"u" => Some(Self::Micro),
			"n" => Some(Self::Nano),
			"p" => Some(Self::Pico),
			"f" => Some(Self::Femto),
			"a" => Some(Self::Atto),
			"z" => Some(Self::Zepto),
			"y" => Some(Self::Yocto),
			_ => None,
		}
	}
	
	/// Get prefix by power of 10.
	pub fn get_using_log10(log10: i8) -> Option<Self> {
		match log10 {
			24 => Some(Self::Yotta),
			21 => Some(Self::Zetta),
			18 => Some(Self::Exa),
			15 => Some(Self::Peta),
			12 => Some(Self::Tera),
			9 => Some(Self::Giga),
			6 => Some(Self::Mega),
			3 => Some(Self::Kilo),
			2 => Some(Self::Hecto),
			1 => Some(Self::Deca),
			-1 => Some(Self::Deci),
			-2 => Some(Self::Centi),
			-3 => Some(Self::Milli),
			-6 => Some(Self::Micro),
			-9 => Some(Self::Nano),
			-12 => Some(Self::Pico),
			-15 => Some(Self::Femto),
			-18 => Some(Self::Atto),
			-21 => Some(Self::Zepto),
			-24 => Some(Self::Yocto),
			_ => None,
		}
	}
	
	/// Get prefix by short name.
	pub fn get_using_short_name(name: &str) -> Option<Self> {
		let name = name.trim().to_lowercase();
		match name.as_str() {
			"septillion" => Some(Self::Yotta),
			"sextillion" => Some(Self::Zetta),
			"quintillion" => Some(Self::Exa),
			"quadrillion" => Some(Self::Peta),
			"trillion" => Some(Self::Tera),
			"billion" => Some(Self::Giga),
			"million" => Some(Self::Mega),
			"thousand" => Some(Self::Kilo),
			"hundred" => Some(Self::Hecto),
			"ten" => Some(Self::Deca),
			"tenth" => Some(Self::Deci),
			"hundredth" => Some(Self::Centi),
			"thousandth" => Some(Self::Milli),
			"millionth" => Some(Self::Micro),
			"billionth" => Some(Self::Nano),
			"trillionth" => Some(Self::Pico),
			"quadrillionth" => Some(Self::Femto),
			"quintillionth" => Some(Self::Atto),
			"sextillionth" => Some(Self::Zepto),
			"septillionth" => Some(Self::Yocto),
			_ => None,
		}
	}
	
	/// Get prefix by long name.
	pub fn get_using_long_name(name: &str) -> Option<Self> {
		let name = name.trim().to_lowercase();
		match name.as_str() {
			"quadrillion" => Some(Self::Yotta),
			"trilliard" => Some(Self::Zetta),
			"trillion" => Some(Self::Exa),
			"billiard" => Some(Self::Peta),
			"billion" => Some(Self::Tera),
			"milliard" => Some(Self::Giga),
			"million" => Some(Self::Mega),
			"thousand" => Some(Self::Kilo),
			"hundred" => Some(Self::Hecto),
			"ten" => Some(Self::Deca),
			"tenth" => Some(Self::Deci),
			"hundredth" => Some(Self::Centi),
			"thousandth" => Some(Self::Milli),
			"millionth" => Some(Self::Micro),
			"milliardth" => Some(Self::Nano),
			"billionth" => Some(Self::Pico),
			"billiardth" => Some(Self::Femto),
			"trillionth" => Some(Self::Atto),
			"trilliardth" => Some(Self::Zepto),
			"quadrillionth" => Some(Self::Yocto),
			_ => None,
		}
	}
}