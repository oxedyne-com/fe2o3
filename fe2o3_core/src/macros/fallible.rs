#[macro_export]
/// Shortcut for attempt to convert type.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     let n = 42u32;
///     let n1 = try_into!(usize, n);
///     let n2 = res!(TryInto::<usize>::try_into(n));
///     assert_eq!(n1, n2);
///     Ok(())
/// }
///
///```
macro_rules! try_into {
    ($typ:tt, $expr:expr) => {
        match TryInto::<$typ>::try_into($expr) {
            Ok(v) => v,
            Err(e) => return Err(err!(e,
                "Could not convert {:?} into {:?}.", $expr, std::any::type_name::<$typ>();
            Bug, Conversion)),
        }
    }
}

#[macro_export]
/// Shortcut for attempt to add integers that can overflow.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     //let sum = try_add!(255u8, 1); // Will return an overflow error
///     let sum = try_add!(254u8, 1); // No error, returns value
///     Ok(())
/// }
///
///```
macro_rules! try_add {
    ($n1:expr, $n2:expr $(,)?) => {
        match $n1.checked_add($n2) {
            Some(result) => result,
            None => return Err(err!(
                "Attempt to add {} and {} (type {}) resulted in integer overflow.",
                $n1, $n2, fmt_typ!($n1);
            Integer, Overflow)),
        }
    }
}

#[macro_export]
/// Shortcut for attempt to subtract integers that can underflow.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     //let sum = try_sub!(1u8, 2); // Will return an overflow error
///     let sum = try_sub!(2u8, 1); // No error, returns value
///     Ok(())
/// }
///
///```
macro_rules! try_sub {
    ($n1:expr, $n2:expr $(,)?) => {
        match $n1.checked_sub($n2) {
            Some(result) => result,
            None => return Err(err!(
                "Attempt to subtract {} from {} (type {}) resulted in integer underflow.",
                $n2, $n1, fmt_typ!($n1);
            Integer, Underflow)),
        }
    }
}

#[macro_export]
/// Shortcut for attempt to multiply integers that can overflow.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     //let prod = try_mul!(17u8, 17); // Will return an overflow error
///     let prod = try_mul!(15u8, 15); // No error, returns value
///     Ok(())
/// }
///
///```
macro_rules! try_mul {
    ($n1:expr, $n2:expr $(,)?) => {
        match $n1.checked_mul($n2) {
            Some(result) => result,
            None => return Err(err!(
                "Attempt to multiply {} and {} (type {}) resulted in integer overflow.",
                $n1, $n2, fmt_typ!($n1);
            Integer, Overflow)),
        }
    }
}

#[macro_export]
/// Shortcut for attempt to divide integers, checking for division by zero.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     //let div = try_div!(17u8, 0); // Will return an error
///     let div = try_div!(15u8, 4); // No error, returns value
///     Ok(())
/// }
///
///```
macro_rules! try_div {
    ($n1:expr, $n2:expr $(,)?) => {
        match $n1.checked_div($n2) {
            Some(result) => result,
            None => return Err(err!(
                "Attempt to divide {} by {} (type {}).",
                $n1, $n2, fmt_typ!($n1);
            Integer, ZeroDenominator)),
        }
    }
}

#[macro_export]
/// Shortcut for attempt to find the remainder after division, checking for division by zero.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     //let rem = try_rem!(17u8, 0); // Will return an error
///     let rem = try_rem!(15u8, 4); // No error, returns value
///     Ok(())
/// }
///
///```
macro_rules! try_rem {
    ($n1:expr, $n2:expr $(,)?) => {
        match $n1.checked_rem($n2) {
            Some(result) => result,
            None => return Err(err!(
                "Attempt to find the remainder when {} is divided by {} (type {}).",
                $n1, $n2, fmt_typ!($n1);
            Integer, ZeroDenominator)),
        }
    }
}

#[macro_export]
/// Shortcut for check on whether the given expression fits within range.  Intended for numbers but
/// works for anything that is `std::cmp::PartialOrd`.  The user is responsible for ensuring valid
/// specification or inference of types.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     //let n = try_range!(3u8, 5u8, 17u8); // Will return an error
///     let n = try_range!(6u8, 5u8, 17u8); // No error, returns 6u8
///     Ok(())
/// }
///
///```
macro_rules! try_range {
    ($n:expr, $min:expr, $max:expr $(,)?) => {
        match (&$n, &$min, &$max) {
            (n, min, max) if std::cmp::PartialOrd::lt(n, min) || std::cmp::PartialOrd::gt(n, max) => {
                return Err(err!(
                    "{} is outside range [{}, {}].", $n, $min, $max;
                Numeric, Range));
            }
            _ => $n,
        }
    };
}

/// Implement some basic traits for native integers.
#[macro_export]
macro_rules! impls_for_native_integer {
    ($t:ty, $n:literal) => {
        
        impl oxedize_fe2o3_core::byte::FromBytes for $t {
            fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
                const BYTE_LEN: usize = std::mem::size_of::<$t>();
                if buf.len() < BYTE_LEN {
                    return Err(err!(
                        "Not enough bytes to decode, require at least {} \
                        for a {}, slice is of length {}.",
                        BYTE_LEN, std::any::type_name::<Self>(), buf.len();
                    Bytes, Invalid, Input, Decode, Missing));
                }
                let n = <$t>::from_be_bytes(res!(
                    <[u8; BYTE_LEN]>::try_from(&buf[0..BYTE_LEN]),
                    Decode, Bytes, Integer,
                ));
                Ok((n, BYTE_LEN))
            }
        }

        impl oxedize_fe2o3_core::byte::FromByteArray for $t {
            fn from_byte_array<const L: usize>(buf: [u8; L]) -> Outcome<Self> {
                const BYTE_LEN: usize = std::mem::size_of::<$t>();
                if L < BYTE_LEN {
                    return Err(err!(
                        "Not enough bytes to decode, require at least {} \
                        for a {}, array is of length {}.",
                        BYTE_LEN, std::any::type_name::<Self>(), L;
                    Bytes, Invalid, Input, Decode, Missing));
                }
                Ok(<$t>::from_be_bytes(res!(
                    <[u8; BYTE_LEN]>::try_from(&buf[0..BYTE_LEN]),
                    Decode, Bytes, Integer,
                )))
            }
        }

        impl oxedize_fe2o3_core::byte::ToBytes for $t {
            fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
                buf.extend_from_slice(&self.to_be_bytes());
                Ok(buf)
            }
        }

        impl oxedize_fe2o3_core::byte::ToByteArray<$n> for $t {
            fn to_byte_array(&self) -> [u8; $n] {
                self.to_be_bytes()
            }
        }

        impl oxedize_fe2o3_core::string::ToHexString for $t {}
    }
}
