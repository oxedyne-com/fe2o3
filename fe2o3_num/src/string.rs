use crate::{
    float::{
        Float32,
        Float64,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fmt,
    str::{
        self,
        FromStr,
    },
};

use num_bigint::{
    BigInt,
    Sign,
};
use bigdecimal::BigDecimal;

impl FromStr for Float32 {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.parse::<f32>() {
            Ok(f) => Ok(Float32(f)),
            Err(e) => Err(err!(e,
                "While parsing '{}' as an f32", s;
            String, Input, Decode, Numeric)),
        }
    }
}

impl fmt::Display for Float32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Float64 {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.parse::<f64>() {
            Ok(f) => Ok(Float64(f)),
            Err(e) => Err(err!(e,
                "While parsing '{}' as an f64", s;
            String, Input, Decode, Numeric)),
        }
    }
}

impl fmt::Display for Float64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn string_to_digit_vec(input: &str) -> Vec<u8> {
    input.chars().map(|c| c.to_digit(10).unwrap() as u8).collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NumberType {
    SignedInt,
    UnsignedInt,
    FloatingPoint,
    Hexadecimal,
    Octal,
    Base64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// An intermediate representation, simplifying subsequent conversion of the string parts to a
/// number.
///
/// ```ignore
/// -1234.567e-34
/// +---+---+ +-+
///     |      |
///     |      +--- exponent (an integer)
///     +----- significand made of an integer and fraction part
/// ```
pub struct NumberString {
    src:        String, // Source string
    radix:      u32,    // The number base, e.g. 10, 2, 8, 16
    signeg:     bool,   // Significand sign
    sigint:     String, // Significand integer part (absolute value)
    digits:     Vec<u8>,// Significand integer part as bytes
    sigfrac:    String, // Significand fraction part
    exp:        String, // Exponent part (absolute value)
    expneg:     bool,   // Exponent sign
    ftz:        usize,  // Number of trailing zeros in significand fraction part
}

impl Default for NumberString {
    fn default() -> Self {
        Self {
            src:        String::new(),
            radix:      10,
            signeg:     false,
            sigint:     String::new(),
            digits:     Vec::new(),
            sigfrac:    String::new(),
            exp:        String::new(),
            expneg:     false,
            ftz:        0,
        }
    }
}

impl NumberString {

    const CHAR_ZERO: u8 = b"0"[0] as u8;
    const CHAR_A_LOWER: u8 = b"a"[0] as u8;

    pub fn new<S: Into<String>>(s: S) -> Outcome<Self> {
        Self::validate(s)
    }

    /// Does the number contain a radix or decimal point?
    pub fn has_point(&self) -> bool { self.sigfrac.len() > 0 }

    pub fn source(&self) -> &str { &self.src }
    pub fn radix(&self) -> u32 { self.radix }
    pub fn is_negative(&self) -> bool { self.signeg }
    pub fn abs_integer_str(&self) -> &str { &self.sigint }
    pub fn int_string(&self) -> String {
        if self.signeg {
            fmt!("-{}", self.sigint)
        } else {
            self.sigint.clone()
        }
    }
    pub fn fraction_str(&self) -> &str { &self.sigfrac }
    pub fn exponent_str(&self) -> &str { &self.exp }
    pub fn exponent_negative(&self) -> bool { self.expneg }
    /// When reading digits in the fractional part of a number, we can't know how many
    /// trailing zeros there are until we reach the end.  So during validation, we simply keep a
    /// count.
    pub fn fraction_trailing_zeros(&self) -> usize { self.ftz }
    pub fn is_zero(&self) -> bool {
        self.sigint.len() == 0 && !self.has_exp()
    }

    /// Does the number contain an exponent?
    pub fn has_exp(&self) -> bool { self.exp.len() > 0 }

    fn str_to_digits(input: &str) -> Outcome<Vec<u8>> {
        let mut digits = Vec::new();
        let mut i: usize = 0;
        for s in input.chars() {
            i += 1;
            digits.push(match s {
                '0'..='9' => (s as u8) - Self::CHAR_ZERO,
                'a'..='z' => (s as u8) - Self::CHAR_A_LOWER + 10,
                _ => return Err(err!(
                    "Character '{}' at position {} is not recognised as a \
                    potential numerical digit.  Use characters '0'..'9' or \
                    'a'..'z'.", s, i; 
                String, Decode, Invalid, Input)),
            });
        }
        Ok(digits)
    }

    pub fn count_trailing_zeros(s: &str) -> usize {
        let mut result = 0usize;
        for c in s.chars().rev() {
            if c == '0' {
                result += 1;
            } else {
                break;
            }
        }
        result
    }

    pub fn sign_bigint(&self) -> Sign {
        if self.signeg {
            Sign::Minus
        } else {
            Sign::Plus
        }
    }

    /// Convert the significand integer part to a [`BigInt`].  This accomodates the common radix
    /// values 10, 2, 8 and 16.
    pub fn as_bigint(&self) -> Outcome<BigInt> {
        let digits = res!(Self::str_to_digits(&self.sigint));
        match BigInt::from_radix_be(
            self.sign_bigint(),
            &digits,
            self.radix,
        ) {
            Some(n) => Ok(n),
            None => Err(err!(
                "Could not interpret the number {:?} as a BigInt.", self;
            Conversion, Integer)),
        }
    }

    pub fn as_bigdecimal(&self) -> Outcome<BigDecimal> {
        let mut exp = 0i64;
        if self.exp.len() > 0 {
            match <i64>::from_str_radix(&self.exp, self.radix) {
                Ok(n) => exp = n,
                Err(e) => return Err(err!(e,
                    "While trying to convert exp in {:?} to i64", self;
                String, Input, Decode, Numeric)),
            }
        }
        let flen = self.sigfrac.len() - self.ftz;
        let mut digits: Vec<u8> =
            self.sigint
            .as_bytes()
            .iter()
            .map(|x| (*x as u8) - Self::CHAR_ZERO)
            .collect();
        digits.append(
            &mut self.sigfrac
            .as_bytes()
            .iter()
            .take(flen)
            .map(|x| (*x as u8) - Self::CHAR_ZERO)
            .collect::<Vec<u8>>()
        );
        
        Ok(
            BigDecimal::new(
                match BigInt::from_radix_be(
                    self.sign_bigint(),
                    &digits,
                    self.radix,
                ) {
                    Some(n) => n,
                    None => return Err(err!(
                        "The conversion of a NumberString with significand \
                        '{}.{}' to a digit vector {:?} using sign {:?} and radix {} \
                        resulted in a None BigInt",
                        self.sigint,
                        self.sigfrac,
                        &digits,
                        self.sign_bigint(),
                        self.radix;
                    String, Encode, Numeric)),
                },
                (flen as i64)-exp,
            )
        )
    }

    /// A commonly used error string template for the [`Self::validate`] associated function.
    fn errmsg(
        c:      char,
        pos:    usize,
        s:      &str,
        reason: String,
    ) -> String {
        fmt!(
            "The digit '{}' was detected at position {} in the string '{}' {}",
            c, pos, s.to_string(), reason,
        )
    }

    /// Takes a string and validates it as an integer of the given radix, or else as a number in
    /// decimal, or scientific notation, if the radix is 10, in a single pass.  Extra
    /// leading zeros in the significand integer and the exponent are ignored for base 10.
    #[allow(unused_assignments)] // for exp_digit_count
    pub fn validate<S: Into<String>>(s: S) -> Outcome<Self> {
        let s = s.into().trim().to_lowercase();
        let s_ref = &s;
        // Defaults
        let mut radix: u32 = 10; // default
        // Initialisations
        let mut sigint                  = String::new();
        let mut sigfrac                 = String::new();
        let mut exp                     = String::new();
        let mut frac_trailing_zeros     = 0usize;
        let mut sigint_digit_count: usize   = 0;
        let mut exp_digit_count: usize      = 0;

        let mut flags = ValidationFlags::default();

        let mut iter = s.chars();

        if s_ref.starts_with("-") {
            flags.signeg = true;
            flags.sig_sign_detected = true;
            iter.next();
        } else if s_ref.starts_with("+") {
            flags.signeg = false;
            flags.sig_sign_detected = true;
            iter.next();
        }

        let mut chars_to_skip: usize = 0;

        if iter.as_str().starts_with("0x") {
            radix = 16;
            flags.radix_detected = true;
            flags.sig_capture_active = true;
            chars_to_skip = 2;
        } else if iter.as_str().starts_with("0o") {
            radix = 8;
            flags.radix_detected = true;
            flags.sig_capture_active = true;
            chars_to_skip = 2;
        } else if iter.as_str().starts_with("0b") {
            radix = 2;
            flags.radix_detected = true;
            flags.sig_capture_active = true;
            chars_to_skip = 2;
        }

        let mut iter2 = iter.clone().enumerate().skip(chars_to_skip);
        while let Some((i, c)) = iter2.next() {
            //trace!("(i,c)=({},'{}') {:?}",i,c,flags);
            match c {
                '0' ..= '9' => {
                    if flags.sig_capture_active {
                        if flags.sigfrac_capture_active {
                            sigfrac.push(c);
                            if c == '0' {
                                frac_trailing_zeros += 1;
                            } else {
                                frac_trailing_zeros = 0;
                            }
                        } else {
                            if c == '0' {
                                flags.prev_char_zero = true;
                                // Capture the character in the significand integer string
                                // if a non-zero character has already been detected, or if
                                // the radix is anything other than 10.
                                if flags.sig_nonzero_lead_detected || radix != 10 {
                                    sigint.push(c);
                                    sigint_digit_count += 1;
                                }
                            } else {
                                flags.prev_char_zero = false;
                                if radix == 2 && c != '1' {
                                    // Binary numbers can only contain 0 or 1.
                                    return Err(err!("{}", Self::errmsg(c, i, &s,
                                        fmt!("but is not valid in base {}", radix));
                                    String, Encode, Numeric, Mismatch));
                                }
                                if radix == 8 && (c == '8' || c == '9') {
                                    // Octal numbers can only contain 0..7.
                                    return Err(err!("{}", Self::errmsg(c, i, &s,
                                        fmt!("but is not valid in base {}", radix));
                                    String, Encode, Numeric, Mismatch));
                                }
                                sigint.push(c);
                                sigint_digit_count += 1;
                                flags.sig_nonzero_lead_detected = true;
                            }
                        }
                    } else {
                        if c == '0' {
                            if flags.exp_nonzero_lead_detected {
                                exp.push(c);
                                exp_digit_count += 1;
                            }
                        } else {
                            exp.push(c);
                            exp_digit_count += 1;
                            flags.exp_nonzero_lead_detected = true;
                        }
                    }
                    continue;
                },
                'a' ..= 'f' | 'A' ..= 'F' => {
                    if flags.sig_capture_active {
                        if (c == 'e' || c == 'E') && radix == 10 {
                            if flags.exp_detected {
                                return Err(err!("{}", Self::errmsg(c, i, &s,
                                    fmt!("but an exp symbol had already been detected"));
                                String, Encode, Numeric, Mismatch));
                            } else {
                                flags.exp_detected = true;
                                flags.sig_capture_active = false;
                                flags.prev_char_zero = false;
                                continue;
                            }
                        }

                        if radix == 16 {
                            sigint.push(c.to_ascii_lowercase());
                            sigint_digit_count += 1;
                        } else {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("which is expected to represent a base {} \
                                    number, but is only valid in a base 16 integer", radix));
                            String, Encode, Numeric, Mismatch));
                        }
                        flags.prev_char_zero = false;
                        continue;
                    } else {
                        return Err(err!("{}", Self::errmsg(c, i, &s,
                            fmt!("which is expected to represent a base {} \
                                number, but is only valid in a base 16 integer", radix));
                        String, Encode, Numeric, Mismatch));
                    }
                },
                '+' | '-' => {
                    if flags.sig_capture_active {
                        if sigint_digit_count > 0 {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("but the sign should be the first character"));
                            String, Encode, Numeric, Mismatch));
                        } else if flags.sig_sign_detected {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("but a sign had already been detected"));
                            String, Encode, Numeric, Mismatch));
                        } else if radix != 10 {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("with expected base {}, but signs are generally only used \
                                    with base 10, other bases use non-signed negation", radix));
                            String, Encode, Numeric, Mismatch));
                        } else {
                            flags.sig_sign_detected = true;
                            if c == '-' {
                                flags.signeg = true;
                            }
                            flags.prev_char_zero = false;
                            continue;
                        }
                    } else {
                        if exp_digit_count > 0 {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("but the sign should be the first character of the exponent"));
                            String, Encode, Numeric, Mismatch));
                        } else if flags.exp_sign_detected {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("but a sign had already been detected in the exponent"));
                            String, Encode, Numeric, Mismatch));
                        } else {
                            flags.exp_sign_detected = true;
                            if c == '-' {
                                flags.expneg = true;
                            }
                            flags.prev_char_zero = false;
                            continue;
                        }
                    }
                },
                '.' => {
                    if flags.sig_capture_active {
                        if flags.point_detected {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("but a decimal point had already been detected"));
                            String, Encode, Numeric, Mismatch));
                        } else if radix != 10 {
                            return Err(err!("{}", Self::errmsg(c, i, &s,
                                fmt!("which appears to be base {}, and does not generally support \
                                    decimal notation", radix));
                            String, Encode, Numeric, Mismatch));
                        } else {
                            flags.point_detected = true;
                            flags.sigfrac_capture_active = true;
                            if !flags.sig_nonzero_lead_detected {
                                sigint.push('0');
                                sigint_digit_count += 1;
                            }
                            flags.prev_char_zero = false;
                            continue;
                        }
                    } else {
                        return Err(err!("{}", Self::errmsg(c, i, &s,
                            fmt!("but this implementation does not support fractional exps"));
                        String, Encode, Numeric, NoImpl));
                    }
                },
                '_' => { // valid space characters
                    flags.prev_char_zero = false;
                },
                _ => {
                    return Err(err!("{}", Self::errmsg(c, i, &s,
                        fmt!("but is not valid for a number"));
                    String, Encode, Numeric, Invalid));
                },
            }
        }

        if sigint_digit_count == 0 {
            if flags.prev_char_zero {
                sigint.push('0');
                sigint_digit_count += 1;
            }
        }

        //if sigint_digit_count < 1 && !flags.sig_nonzero_lead_detected {
        //    return Err(err!(
        //        "The string '{}' does not represent a \
        //        number", s,
        //    ), String, Decode, Input, Numeric))
        //}

        if flags.exp_detected && exp_digit_count == 0 {
            exp.push('0');
            exp_digit_count += 1;
        }

        let digits = res!(Self::str_to_digits(&sigint));

        Ok(NumberString {
            src: s,
            radix,
            signeg: flags.signeg,
            sigint,
            digits,
            sigfrac,
            exp,
            expneg: flags.expneg,
            ftz: frac_trailing_zeros,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ValidationFlags {
    radix_detected:             bool,
    sig_sign_detected:          bool, // significand sign detected
    signeg:                     bool,
    sig_nonzero_lead_detected:  bool, // applies to significand integer
    sig_capture_active:         bool, // capturing significand
    point_detected:             bool, // radix (i.e. decimal) point '.'
    sigfrac_capture_active:     bool, // capturing significand fraction part
    exp_detected:               bool, // exponent detected
    exp_sign_detected:          bool, // exponent sign detected
    exp_nonzero_lead_detected:  bool, // applies to exponent
    expneg:                     bool,
    prev_char_zero:             bool, // a way to flag if the previous character was zero
}

impl Default for ValidationFlags {
    fn default() -> Self {
        Self {
            radix_detected:             false,
            sig_sign_detected:          false,
            signeg:                     false,
            sig_nonzero_lead_detected:  false,
            sig_capture_active:         true,
            point_detected:             false,
            sigfrac_capture_active:     false,
            exp_detected:               false,
            exp_sign_detected:          false,
            exp_nonzero_lead_detected:  false,
            expneg:                     false,
            prev_char_zero:             false,
        }
    }
}

/// Trait for formatting numbers with thousands separators.
pub trait ThousandsSeparator {
    /// Formats the number with the specified thousands separator character.
    /// 
    /// # Arguments
    /// * `sep` - The character to use as thousands separator (e.g., ',', ' ', '_').
    /// 
    /// # Example
    /// ```
    /// use oxedyne_fe2o3_num::string::ThousandsSeparator;
    /// 
    /// assert_eq!(1234567.with_sep(','), "1,234,567");
    /// assert_eq!((-9876543).with_sep(' '), "-9 876 543");
    /// ```
    fn with_sep(&self, sep: char) -> String;
    
    /// Formats the number with separators and specified decimal precision.
    /// 
    /// # Arguments
    /// * `sep` - The character to use as thousands separator (e.g., ',', ' ', '_').
    /// * `decimals` - Number of decimal places to show (0 means no decimal point).
    /// 
    /// # Example
    /// ```
    /// use oxedyne_fe2o3_num::string::ThousandsSeparator;
    /// 
    /// assert_eq!(1234567.with_sep_dp(',', 2), "1,234,567.00");
    /// assert_eq!(1234.567.with_sep_dp(' ', 1), "1 234.6");
    /// assert_eq!(1234.with_sep_dp(',', 0), "1,234");
    /// ```
    fn with_sep_dp(&self, sep: char, decimals: usize) -> String;
    
    /// Formats the number with comma thousands separators.
    /// 
    /// # Example
    /// ```
    /// use oxedyne_fe2o3_num::string::ThousandsSeparator;
    /// 
    /// assert_eq!(1234567.with_commas(), "1,234,567");
    /// assert_eq!((-9876543).with_commas(), "-9,876,543");
    /// ```
    fn with_commas(&self) -> String {
        self.with_sep(',')
    }
    
    /// Formats the number with comma thousands separators and specified decimal precision.
    /// 
    /// # Arguments
    /// * `decimals` - Number of decimal places to show (0 means no decimal point).
    /// 
    /// # Example
    /// ```
    /// use oxedyne_fe2o3_num::string::ThousandsSeparator;
    /// 
    /// assert_eq!(1234567.with_commas_dp(2), "1,234,567.00");
    /// assert_eq!(1234.567.with_commas_dp(1), "1,234.6");
    /// assert_eq!(1234.with_commas_dp(0), "1,234");
    /// ```
    fn with_commas_dp(&self, decimals: usize) -> String {
        self.with_sep_dp(',', decimals)
    }
}

/// Helper function to add separators to a string of digits.
fn add_separators_to_digits(digits: &str, sep: char) -> String {
    if digits.len() <= 3 {
        return digits.to_string();
    }
    
    let mut result = String::with_capacity(digits.len() + (digits.len() - 1) / 3);
    let chars: Vec<char> = digits.chars().collect();
    
    for (i, &ch) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(sep);
        }
        result.push(ch);
    }
    
    result
}

macro_rules! impl_thousands_separator_for_int {
    ($($t:ty),*) => {
        $(
            impl ThousandsSeparator for $t {
                fn with_sep(&self, sep: char) -> String {
                    let s = self.to_string();
                    if let Some(stripped) = s.strip_prefix('-') {
                        format!("-{}", add_separators_to_digits(stripped, sep))
                    } else {
                        add_separators_to_digits(&s, sep)
                    }
                }
                
                fn with_sep_dp(&self, sep: char, decimals: usize) -> String {
                    let integer_part = self.with_sep(sep);
                    if decimals == 0 {
                        integer_part
                    } else {
                        format!("{}.{}", integer_part, "0".repeat(decimals))
                    }
                }
            }
        )*
    };
}

macro_rules! impl_thousands_separator_for_float {
    ($($t:ty),*) => {
        $(
            impl ThousandsSeparator for $t {
                fn with_sep(&self, sep: char) -> String {
                    let s = self.to_string();
                    let (sign, rest) = if let Some(stripped) = s.strip_prefix('-') {
                        ("-", stripped)
                    } else {
                        ("", s.as_str())
                    };
                    
                    if let Some(dot_pos) = rest.find('.') {
                        let (integer_part, fractional_part) = rest.split_at(dot_pos);
                        format!("{}{}{}", sign, add_separators_to_digits(integer_part, sep), fractional_part)
                    } else {
                        format!("{}{}", sign, add_separators_to_digits(rest, sep))
                    }
                }
                
                fn with_sep_dp(&self, sep: char, decimals: usize) -> String {
                    let formatted = if decimals == 0 {
                        format!("{:.0}", self)
                    } else {
                        format!("{:.1$}", self, decimals)
                    };
                    
                    let (sign, rest) = if let Some(stripped) = formatted.strip_prefix('-') {
                        ("-", stripped)
                    } else {
                        ("", formatted.as_str())
                    };
                    
                    if decimals == 0 {
                        format!("{}{}", sign, add_separators_to_digits(rest, sep))
                    } else if let Some(dot_pos) = rest.find('.') {
                        let (integer_part, fractional_part) = rest.split_at(dot_pos);
                        format!("{}{}{}", sign, add_separators_to_digits(integer_part, sep), fractional_part)
                    } else {
                        format!("{}{}", sign, add_separators_to_digits(rest, sep))
                    }
                }
            }
        )*
    };
}

// Implement for all standard integer types
impl_thousands_separator_for_int!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

// Implement for all standard floating-point types
impl_thousands_separator_for_float!(f32, f64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_00() -> Outcome<()> {
        let s = String::from("");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The empty string '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_empty_01() -> Outcome<()> {
        let s = String::from("-");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The empty string '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_integers_base_10_000() -> Outcome<()> {
        for i in 1..10001 {
            let s0 = i.to_string();
            let digits = string_to_digit_vec(&s0);
            let ns = res!(NumberString::validate(&s0));
            let expected = NumberString {
                src:    s0.clone(),
                sigint: s0,
                digits: digits,
                ..Default::default()
            };
            assert_eq!(ns, expected);
        }
        Ok(())
    }

    #[test]
    fn test_integers_base_10_010() -> Outcome<()> {
        for i in -10001..0i32 {
            let s0 = i.to_string();
            let s00 = i.abs().to_string();
            let digits = string_to_digit_vec(&s00);
            let ns = res!(NumberString::validate(&s0));
            let expected = NumberString {
                src:    s0,
                signeg: true,
                sigint: s00,
                digits: digits,
                ..Default::default()
            };
            assert_eq!(ns, expected);
        }
        Ok(())
    }

    #[test]
    fn test_leading_zeros_000() -> Outcome<()> {
        for i in 1..10001 {
            let s0 = i.to_string();
            let mut s1 = s0.clone();
            s1.insert_str(0, "0000");
            let digits = string_to_digit_vec(&s0);
            let ns = res!(NumberString::validate(&s1));
            let expected = NumberString {
                src:    s1,
                sigint: s0,
                digits: digits,
                ..Default::default()
            };
            assert_eq!(ns, expected);
        }
        Ok(())
    }

    #[test]
    fn test_leading_zeros_010() -> Outcome<()> {
        let s = String::from("0");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            sigint: String::from("0"),
            digits: vec![0],
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_signs_000() -> Outcome<()> {
        let s = String::from("-0");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            signeg: true,
            sigint: String::from("0"),
            digits: vec![0],
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_signs_010() -> Outcome<()> {
        let s = String::from("+0");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            sigint: String::from("0"),
            digits: vec![0],
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_signs_020() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("-");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The string '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_signs_030() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("+");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The string '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_signs_040() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("++10");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The extra sign in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_signs_050() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("--10");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The extra sign in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_signs_060() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("-+10");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The extra sign in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_signs_070() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("-1+0");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The extra sign in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_signs_080() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("-1000+");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The extra sign in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_with_space_000() -> Outcome<()> {
        // Space should trigger error
        let s = String::from("1 234 567");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The first space in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_binary_000() -> Outcome<()> {
        let s = String::from("0b1010");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:        s,
            radix:      2,
            sigint:     String::from("1010"),
            digits:     vec![1,0,1,0],
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_binary_005() -> Outcome<()> {
        let s = String::from("0b_10_10");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:        s,
            radix:      2,
            sigint:     String::from("1010"),
            digits:     vec![1,0,1,0],
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_binary_010() -> Outcome<()> {
        // Invalid binary digit
        let s = String::from("0b2");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The non-binary digit 2 in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_octal_000() -> Outcome<()> {
        let s = String::from("0o1234567");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            radix:  8,
            sigint: String::from("1234567"),
            digits: (1..8).collect(),
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_octal_010() -> Outcome<()> {
        // Invalid octal digit
        let s = String::from("0o8");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The non-octal digit 8 in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_hex_000() -> Outcome<()> {
        let s = String::from("0x123456789abcdef");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            radix:  16,
            sigint: String::from("123456789abcdef"),
            digits: (1..16).collect(),
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_hex_010() -> Outcome<()> {
        // Invalid hex digit
        let s = String::from("0xg");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The non-hex digit g in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_hex_020() -> Outcome<()> {
        // Invalid hex digit
        let s = String::from("0xo");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The non-hex digit o in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_hex_030() -> Outcome<()> {
        // Invalid hex digit
        let s = String::from("0x__o");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The non-hex digit o in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_hex_040() -> Outcome<()> {
        // Invalid hex digit
        let s = String::from("0xx");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The non-hex digit x in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_hex_050() -> Outcome<()> {
        // Invalid hex digit
        let s = String::from("0x___x");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The non-hex digit x in '{}' should have triggered an error but the result was \
                ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_hex_060() -> Outcome<()> {
        // Invalid hex digit
        let s = String::from("afe123");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The hex number '{}' should have triggered an error because of the absence of \
                a '0x' prefix but the result was ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_hex_100() -> Outcome<()> {
        let s1 = String::from("0xf0");
        let ns1 = NumberString::validate(&s1);
        test!("{:?}", ns1);
        Ok(())
    }

    #[test]
    fn test_decimals_000() -> Outcome<()> {
        for i in 1..101 {
            for j in 1..101 {
                let si = i.to_string();
                let sf = j.to_string();
                let mut s = sf.clone();
                s.insert(0, '.');
                s.insert_str(0, &si);
                let digits = string_to_digit_vec(&si);
                let ns = res!(NumberString::validate(&s));
                let ftz = NumberString::count_trailing_zeros(&s);
                let expected = NumberString {
                    src:    s,
                    sigint: si,
                    digits: digits,
                    sigfrac:sf,
                    exp:    String::from(""),
                    ftz,
                    ..Default::default()
                };
                assert_eq!(ns, expected);
            }
        }
        Ok(())
    }

    #[test]
    fn test_decimals_010() -> Outcome<()> {
        for i in -101..0i32 {
            for j in 1..101 {
                let si = i.to_string();
                let si0 = i.abs().to_string();
                let sf = j.to_string();
                let mut s = sf.clone();
                s.insert(0, '.');
                s.insert_str(0, &si);
                let digits = string_to_digit_vec(&si0);
                let ns = res!(NumberString::validate(&s));
                let ftz = NumberString::count_trailing_zeros(&s);
                let expected = NumberString {
                    src:    s,
                    signeg: i < 0,
                    sigint: si0,
                    digits: digits,
                    sigfrac:sf,
                    ftz,
                    ..Default::default()
                };
                assert_eq!(ns, expected);
            }
        }
        Ok(())
    }

    #[test]
    fn test_decimals_020() -> Outcome<()> {
        let s = String::from("0.0");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            sigint: String::from("0"),
            digits: vec![0],
            sigfrac:String::from("0"),
            ftz:    1,
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_decimals_030() -> Outcome<()> {
        let s = String::from("+0.0");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            sigint: String::from("0"),
            digits: vec![0],
            sigfrac:String::from("0"),
            ftz:    1,
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_decimals_040() -> Outcome<()> {
        let s = String::from("000.0000");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            sigint: String::from("0"),
            digits: vec![0],
            sigfrac:String::from("0000"),
            ftz:    4,
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_sci_000() -> Outcome<()> {
        for i in 1..11 {
            for j in 1..11 {
                for k in -10..11i32 {
                    let si = i.to_string();
                    let sf = j.to_string();
                    let e = k.to_string();
                    let e0 = k.abs().to_string();
                    let mut s = e.clone();
                    s.insert(0, 'e');
                    s.insert_str(0, &sf);
                    s.insert(0, '.');
                    s.insert_str(0, &si);
                    let digits = string_to_digit_vec(&si);
                    let ns = res!(NumberString::validate(&s));
                    let expected = NumberString {
                        src:    s,
                        sigint: si,
                        digits: digits,
                        sigfrac:sf.clone(),
                        exp:    e0,
                        expneg: k < 0,
                        ftz:    NumberString::count_trailing_zeros(&sf),
                        ..Default::default()
                    };
                    assert_eq!(ns, expected);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_sci_010() -> Outcome<()> {
        for i in 1..11 {
            for j in 1..11 {
                for sign in "-+".chars() {
                    for k in 0..11 {
                        let si = i.to_string();
                        let sf = j.to_string();
                        let e0 = k.to_string();
                        let mut e = e0.clone();
                        e.insert_str(0, "000");
                        e.insert(0, sign);
                        let mut s = e.clone();
                        s.insert(0, 'e');
                        s.insert_str(0, &sf);
                        s.insert(0, '.');
                        s.insert_str(0, &si);
                        let digits = string_to_digit_vec(&si);
                        let ns = res!(NumberString::validate(&s));
                        let expected = NumberString {
                            src:    s,
                            sigint: si,
                            digits: digits,
                            sigfrac:sf.clone(),
                            exp:    e0,
                            expneg: sign == '-',
                            ftz:    NumberString::count_trailing_zeros(&sf),
                            ..Default::default()
                        };
                        assert_eq!(ns, expected);
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_sci_020() -> Outcome<()> {
        let s = String::from("-0120.03450e+003");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            signeg: true,
            sigint: String::from("120"),
            digits: vec![1,2,0],
            sigfrac:String::from("03450"),
            exp:    String::from("3"),
            ftz:    1,
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_sci_050() -> Outcome<()> {
        let s = String::from("0.0e0");
        let ns = res!(NumberString::validate(&s));
        let expected = NumberString {
            src:    s,
            sigint: String::from("0"),
            digits: vec![0],
            sigfrac:String::from("0"),
            exp:    String::from("0"),
            ftz:    1,
            ..Default::default()
        };
        assert_eq!(ns, expected);
        Ok(())
    }

    #[test]
    fn test_sci_100() -> Outcome<()> {
        let s = String::from("0.0f0");
        match NumberString::validate(&s) {
            Ok(ns) => Err(err!(
                "The invalid exp designator f in '{}' should have triggered an error but \
                the result was ns = {:?}", s, ns;
            String, Decode)),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn test_string_to_bigdecimal_000() -> Outcome<()> {
        let s = String::from("0");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(s, fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_010() -> Outcome<()> {
        let s = String::from("1234");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(s, fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_020() -> Outcome<()> {
        let s = String::from("1234.5678");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(s, fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_030() -> Outcome<()> {
        let s = String::from("-1234.5678e5");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(String::from("-123456780"), fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_040() -> Outcome<()> {
        let s = String::from("-0.000001234");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(s, fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_140() -> Outcome<()> {
        let s = String::from("0b0");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(String::from("0"), fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_150() -> Outcome<()> {
        let s = String::from("0b1010");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(String::from("10"), fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_160() -> Outcome<()> {
        let s = String::from("0o30071");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(String::from("12345"), fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_170() -> Outcome<()> {
        let s = String::from("0x3039");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(String::from("12345"), fmt!("{}", d));
        Ok(())
    }

    #[test]
    fn test_string_to_bigdecimal_180() -> Outcome<()> {
        // Use of a negative sign ok for all radices.
        let s = String::from("-0x3039");
        let ns = res!(NumberString::validate(&s));
        let d = res!(ns.as_bigdecimal());
        assert_eq!(String::from("-12345"), fmt!("{}", d));
        Ok(())
    }
}
