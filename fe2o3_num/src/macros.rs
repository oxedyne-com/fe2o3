#[macro_export]
/// Create an integer of arbitrary size using primitive integers.  Can return an error.
///
/// ```
/// use oxedyne_fe2o3_core::prelude::*; // To handle the possible error.
/// use oxedyne_fe2o3_num::prelude::*;
///
/// fn main() -> Outcome<()> {
///     let n = res!(aint!(fmt!("{}0", u128::MAX)));
///     Ok(())
/// }
/// ```
macro_rules! aint {
    ( $v:expr ) => {
        num_bigint::BigInt::from_str(&($v.to_string()))
    };
}

//#[macro_export]
///// Create an integer of arbitrary size from a str.
/////
///// ```
///// use oxedyne_fe2o3_core::prelude::*;
///// use oxedyne_fe2o3_num::aintstr;
///// use std::str::FromStr;
///// use num_bigint::BigInt;
/////
///// fn main() -> Outcome<()> {
/////     let n = res!(aintstr!("-42"));
/////     Ok(())
///// }
///// ```
//macro_rules! aintstr {
//    ( $v:expr ) => {
//        {
//            use std::str::FromStr;
//            match num_bigint::BigInt::from_str($v) {
//                Err(e) => Err(err!(e, errmsg!(
//                    "While interpreting '{}' as a BigInt", $v,
//                ), ErrTag::String, ErrTag::Input, ErrTag::Decode, ErrTag::Numeric)),
//                Ok(v) => Ok(v),
//            }
//        }
//    };
//}

#[macro_export]
/// Create an arbitrary decimal via string conversion. Can return an error.
///
/// ```
/// use oxedyne_fe2o3_core::prelude::*; // To handle the possible error.
/// use oxedyne_fe2o3_num::prelude::*;
///
/// fn main() -> Outcome<()> {
///     let n = res!(adec!(fmt!("{}0", f64::MAX)));
///     Ok(())
/// }
/// ```
macro_rules! adec {
    ( $v:expr ) => {
        bigdecimal::BigDecimal::from_str(&($v.to_string()))
    };
}
