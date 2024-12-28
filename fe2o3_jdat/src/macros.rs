#[macro_export]
/// Create a `Dat` with automatic `Kind` detection using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
///
/// let dat = dat!( 1u8 );
/// ```
macro_rules! dat {
    ( $v:expr ) => {
        Dat::from($v)
    };
}

#[macro_export]
/// Create an annotated `Dat` with automatic `Kind` detection using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
///
/// let dat = abox!( 1u8, "A type1 comment" );
/// ```
macro_rules! abox {
    ( $v:expr, $($arg:tt)*) => {
        Dat::ABox(oxedize_fe2o3_jdat::note::NoteConfig::default(), Box::new(Dat::from($v)), format!($($arg)*))
    };
}

#[macro_export]
/// Create a `Dat` with automatic `Kind` detection using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
///
/// let dat = best_dat!( 1u8 );
/// ```
macro_rules! best_dat {
    ( $v:expr ) => {
        Dat::best_from($v)
    };
}

#[macro_export]
/// Create a `List` of `Dat`s using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
///
/// let list = listdat![ 1u8, 2u16, "value", -3i32, ];
/// ```
macro_rules! listdat {
    { $( $x:expr ),* $(,)* } => {
        {
            let mut vec = Vec::new();
            $(
                vec.push(Dat::from($x));
            )*
            Dat::List(vec)
        }
    };
}

#[macro_export]
/// Create a `List` of `Dat`s using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
///
/// let list = best_listdat![ 1u8, 2u16, "value", -3i32, ];
/// ```
macro_rules! best_listdat {
    { $( $x:expr ),* $(,)* } => {
        {
            let mut vec = Vec::new();
            $(
                vec.push(Dat::best_from($x));
            )*
            Dat::List(vec)
        }
    };
}

#[macro_export]
/// Create a `Dat::Tup2` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup2dat;
///
/// let expected = Dat::Tup2(Box::new([ dat!(1u8), dat!(2u8) ]));
/// let list2 = tup2dat![ 1u8, 2u8, ];
/// assert_eq!(list2, expected);
/// ```
macro_rules! tup2dat {

    ( $e1:expr, $e2:expr $(,)? ) => {
        Dat::Tup2(Box::new([
            Dat::from($e1),
            Dat::from($e2),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup2` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup2dat;
///
/// let expected = Dat::Tup2(Box::new([ dat!(1u8), dat!(2u8) ]));
/// let list2 = tup2dat![ 1u8, 2u8, ];
/// assert_eq!(list2, expected);
/// ```
macro_rules! best_tup2dat {

    ( $e1:expr, $e2:expr $(,)? ) => {
        Dat::Tup2(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup3` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup3dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup3(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
/// ]));
/// let list3 = tup3dat![ 1u8, 2u8, 3u8, ];
/// assert_eq!(list3, expected);
/// ```
macro_rules! tup3dat {

    ( $e1:expr, $e2:expr, $e3:expr $(,)? ) => {
        Dat::Tup3(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup3` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup3dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup3(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
/// ]));
/// let list3 = tup3dat![ 1u8, 2u8, 3u8, ];
/// assert_eq!(list3, expected);
/// ```
macro_rules! best_tup3dat {

    ( $e1:expr, $e2:expr, $e3:expr $(,)? ) => {
        Dat::Tup3(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup4` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup4dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup4(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
/// ]));
/// let list4 = tup4dat![ 1u8, 2u8, 3u8, 4u8, ];
/// assert_eq!(list4, expected);
/// ```
macro_rules! tup4dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr $(,)? ) => {
        Dat::Tup4(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
            Dat::from($e4),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup4` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup4dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup4(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
/// ]));
/// let list4 = tup4dat![ 1u8, 2u8, 3u8, 4u8, ];
/// assert_eq!(list4, expected);
/// ```
macro_rules! best_tup4dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr $(,)? ) => {
        Dat::Tup4(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
            Dat::best_from($e4),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup5` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup5dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup5(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
/// ]));
/// let list5 = tup5dat![ 1u8, 2u8, 3u8, 4u8, 5u8, ];
/// assert_eq!(list5, expected);
/// ```
macro_rules! tup5dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr $(,)? ) => {
        Dat::Tup5(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
            Dat::from($e4),
            Dat::from($e5),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup5` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup5dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup5(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
/// ]));
/// let list5 = tup5dat![ 1u8, 2u8, 3u8, 4u8, 5u8, ];
/// assert_eq!(list5, expected);
/// ```
macro_rules! best_tup5dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr $(,)? ) => {
        Dat::Tup5(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
            Dat::best_from($e4),
            Dat::best_from($e5),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup6` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup6dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup6(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
/// ]));
/// let list6 = tup6dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, ];
/// assert_eq!(list6, expected);
/// ```
macro_rules! tup6dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr $(,)? ) => {
        Dat::Tup6(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
            Dat::from($e4),
            Dat::from($e5),
            Dat::from($e6),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup6` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup6dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup6(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
/// ]));
/// let list6 = tup6dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, ];
/// assert_eq!(list6, expected);
/// ```
macro_rules! best_tup6dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr $(,)? ) => {
        Dat::Tup6(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
            Dat::best_from($e4),
            Dat::best_from($e5),
            Dat::best_from($e6),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup7` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup7dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup7(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
/// ]));
/// let list7 = tup7dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, ];
/// assert_eq!(list7, expected);
/// ```
macro_rules! tup7dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr $(,)? ) => {
        Dat::Tup7(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
            Dat::from($e4),
            Dat::from($e5),
            Dat::from($e6),
            Dat::from($e7),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup7` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup7dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup7(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
/// ]));
/// let list7 = tup7dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, ];
/// assert_eq!(list7, expected);
/// ```
macro_rules! best_tup7dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr $(,)? ) => {
        Dat::Tup7(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
            Dat::best_from($e4),
            Dat::best_from($e5),
            Dat::best_from($e6),
            Dat::best_from($e7),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup8` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup8dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup8(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
///     dat!(8u8),
/// ]));
/// let list8 = tup8dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, ];
/// assert_eq!(list8, expected);
/// ```
macro_rules! tup8dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr, $e8:expr $(,)? ) => {
        Dat::Tup8(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
            Dat::from($e4),
            Dat::from($e5),
            Dat::from($e6),
            Dat::from($e7),
            Dat::from($e8),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup8` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup8dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup8(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
///     dat!(8u8),
/// ]));
/// let list8 = tup8dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, ];
/// assert_eq!(list8, expected);
/// ```
macro_rules! best_tup8dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr, $e8:expr $(,)? ) => {
        Dat::Tup8(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
            Dat::best_from($e4),
            Dat::best_from($e5),
            Dat::best_from($e6),
            Dat::best_from($e7),
            Dat::best_from($e8),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup9` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup9dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup9(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
///     dat!(8u8),
///     dat!(9u8),
/// ]));
/// let list9 = tup9dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, ];
/// assert_eq!(list9, expected);
/// ```
macro_rules! tup9dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr, $e8:expr, $e9:expr $(,)? ) => {
        Dat::Tup9(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
            Dat::from($e4),
            Dat::from($e5),
            Dat::from($e6),
            Dat::from($e7),
            Dat::from($e8),
            Dat::from($e9),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup9` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup9dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup9(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
///     dat!(8u8),
///     dat!(9u8),
/// ]));
/// let list9 = tup9dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, ];
/// assert_eq!(list9, expected);
/// ```
macro_rules! best_tup9dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr, $e8:expr, $e9:expr $(,)? ) => {
        Dat::Tup9(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
            Dat::best_from($e4),
            Dat::best_from($e5),
            Dat::best_from($e6),
            Dat::best_from($e7),
            Dat::best_from($e8),
            Dat::best_from($e9),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup10` using `std::convert::From` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup10dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup10(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
///     dat!(8u8),
///     dat!(9u8),
///     dat!(10u8),
/// ]));
/// let list10 = tup10dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, 10u8, ];
/// assert_eq!(list10, expected);
/// ```
macro_rules! tup10dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr, $e8:expr, $e9:expr, $e10:expr $(,)? ) => {
        Dat::Tup10(Box::new([
            Dat::from($e1),
            Dat::from($e2),
            Dat::from($e3),
            Dat::from($e4),
            Dat::from($e5),
            Dat::from($e6),
            Dat::from($e7),
            Dat::from($e8),
            Dat::from($e9),
            Dat::from($e10),
        ]))
    }
}

#[macro_export]
/// Create a `Dat::Tup10` using `oxedize_fe2o3_core::conv::BestFrom` conversion.
///
/// ```
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::tup10dat;
/// use oxedize_fe2o3_jdat::Dat;
///
/// let expected = Dat::Tup10(Box::new([
///     dat!(1u8),
///     dat!(2u8),
///     dat!(3u8),
///     dat!(4u8),
///     dat!(5u8),
///     dat!(6u8),
///     dat!(7u8),
///     dat!(8u8),
///     dat!(9u8),
///     dat!(10u8),
/// ]));
/// let list10 = tup10dat![ 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, 10u8, ];
/// assert_eq!(list10, expected);
/// ```
macro_rules! best_tup10dat {

    ( $e1:expr, $e2:expr, $e3:expr, $e4:expr, $e5:expr,
      $e6:expr, $e7:expr, $e8:expr, $e9:expr, $e10:expr $(,)? ) => {
        Dat::Tup10(Box::new([
            Dat::best_from($e1),
            Dat::best_from($e2),
            Dat::best_from($e3),
            Dat::best_from($e4),
            Dat::best_from($e5),
            Dat::best_from($e6),
            Dat::best_from($e7),
            Dat::best_from($e8),
            Dat::best_from($e9),
            Dat::best_from($e10),
        ]))
    }
}

#[macro_export]
/// Create a `Map` of `Dat`s.
///
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*; // create_dat_map must be brought into scope
///
/// fn main() {
///     let map = mapdat![
///        1u8 => 2u16,
///        "key" => -3i32,
///     ];
/// }
/// ```
macro_rules! mapdat {
    { $( $k:expr => $v:expr ),* $(,)* } => {
        {
            let mut pairs = Vec::new();
            $(
                pairs.push((Dat::from($k), Dat::from($v)));
            )*
            create_dat_map(pairs)
        }
    };
}

#[macro_export]
/// Similar to `mapdat` except an order-preserving `Dat::OrdMap` is returned.
macro_rules! omapdat {
    { $( $k:expr => $v:expr ),* $(,)* } => {
        {
            let mut pairs = Vec::new();
            $(
                pairs.push((Dat::from($k), Dat::from($v)));
            )*
            create_dat_ordmap(pairs)
        }
    };
}

#[macro_export]
macro_rules! best_mapdat {
    { $( $k:expr => $v:expr ),* $(,)* } => {
        {
            //use crate::create_dat_map;
            let mut pairs = Vec::new();
            $(
                pairs.push((Dat::best_from($k), Dat::best_from($v)));
            )*
            create_dat_map(pairs)
        }
    };
}

#[macro_export]
macro_rules! best_omapdat {
    { $( $k:expr => $v:expr ),* $(,)* } => {
        {
            //use crate::create_dat_map;
            let mut pairs = Vec::new();
            $(
                pairs.push((Dat::best_from($k), Dat::best_from($v)));
            )*
            create_dat_ordmap(pairs)
        }
    };
}

#[macro_export]
/// Extracts associated data from a `Dat`.  The motivating purpose was to extract the
/// associated `Vec<u8>` encapsulated using `Dat::bytdat`.  This uses flexible, but still
/// fixed, encoding for the length, yielding a either a `Dat::BU8` through to a
/// `Dat::BU64`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_dat! can return Err
///     let v1 = vec![1u8, 2, 3, 4];
///     let d2 = Dat::bytdat(v1.clone());
///     let v2 = try_extract_dat!(d2, BU8, BU16, BU32, BU64);
///     assert_eq!(v1.len(), v2.len());
///     for (i, v) in v1.into_iter().enumerate() {
///         assert_eq!(v, v2[i]);
///     }
///     Ok(())
/// }
/// ```
macro_rules! try_extract_dat {
    { $src:expr, $($var:ident),* $(,)* } => {
        match $src {
            $(
            Dat::$var(v) => v,
            )*
            unknown => return Err(err!(
                "Expected one of a {} found {:?}.",
                stringify!($(Dat::$var,)*), unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated data from a `Dat`, including a specified cast to a type alias for
/// multiple source variants.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_dat_as;
///
/// type AnIntType = u32;
///
/// fn main() -> Outcome<()> { // needed because try_extract_dat! can return Err
///     let n1 = 42u8 as AnIntType;
///     let d1 = dat!(n1);
///     let n2 = try_extract_dat_as!(d1, AnIntType, U16, U32, U64);
///     // An alternative here is to use:
///     //let n2 = try_extract_dat!(d1, U32) as AnIntType;
///     // but rather than a single change of `AnIntType` to a `u64`, we would also need to change
///     // this `try_extract_dat!` call.
///     assert_eq!(n1, n2);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_dat_as {
    { $src:expr, $trg:ty, $( $var:ident ),* $(,)* } => {
        match $src {
            $(
            Dat::$var(v) => v as $trg,
            )*
            unknown => return Err(err!(
                "Expected a {} found {:?}.",
                stringify!($(Dat::$var,)*), unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `Vec` from a `Dat::List`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_listdat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_listdat! can return Err
///     let list = listdat![42u8, "hello"];
///     let v = try_extract_listdat!(list, 2);
///     assert_eq!(v, vec![dat!(42u8), dat!("hello")]);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_listdat {
    { $src:expr, $n:expr } => {
        match $src {
            Dat::List(v) => {
                if v.len() == $n {
                    v
                } else {
                    return Err(err!(
                        "Number of Dat::List items should be {}, found {}.",
                        $n, v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::List, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 2]` from a `Dat::Tup2`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup2dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup2dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///     ];
///     let list = Dat::Tup2(Box::new(array.clone()));
///     let v = try_extract_tup2dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup2dat {
    { $src:expr } => {
        match $src {
            Dat::Tup2(v) => {
                if v.len() == 2 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup2 items should be 2, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup2, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 3]` from a `Dat::Tup3`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup3dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup3dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///     ];
///     let list = Dat::Tup3(Box::new(array.clone()));
///     let v = try_extract_tup3dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup3dat {
    { $src:expr } => {
        match $src {
            Dat::Tup3(v) => {
                if v.len() == 3 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup3 items should be 3, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup3, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 4]` from a `Dat::Tup4`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup4dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup4dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///         dat!("four"),
///     ];
///     let list = Dat::Tup4(Box::new(array.clone()));
///     let v = try_extract_tup4dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup4dat {
    { $src:expr } => {
        match $src {
            Dat::Tup4(v) => {
                if v.len() == 4 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup4 items should be 4, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup4, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 5]` from a `Dat::Tup5`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup5dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup5dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///         dat!("four"),
///         dat!("five"),
///     ];
///     let list = Dat::Tup5(Box::new(array.clone()));
///     let v = try_extract_tup5dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup5dat {
    { $src:expr } => {
        match $src {
            Dat::Tup5(v) => {
                if v.len() == 5 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup5 items should be 5, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup5, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 6]` from a `Dat::Tup6`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup6dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup6dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///         dat!("four"),
///         dat!("five"),
///         dat!("six"),
///     ];
///     let list = Dat::Tup6(Box::new(array.clone()));
///     let v = try_extract_tup6dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup6dat {
    { $src:expr } => {
        match $src {
            Dat::Tup6(v) => {
                if v.len() == 6 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup6 items should be 6, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup6, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 7]` from a `Dat::Tup7`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup7dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup7dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///         dat!("four"),
///         dat!("five"),
///         dat!("six"),
///         dat!("seven"),
///     ];
///     let list = Dat::Tup7(Box::new(array.clone()));
///     let v = try_extract_tup7dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup7dat {
    { $src:expr } => {
        match $src {
            Dat::Tup7(v) => {
                if v.len() == 7 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup7 items should be 7, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup7, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 8]` from a `Dat::Tup8`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup8dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup8dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///         dat!("four"),
///         dat!("five"),
///         dat!("six"),
///         dat!("seven"),
///         dat!("eight"),
///     ];
///     let list = Dat::Tup8(Box::new(array.clone()));
///     let v = try_extract_tup8dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup8dat {
    { $src:expr } => {
        match $src {
            Dat::Tup8(v) => {
                if v.len() == 8 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup8 items should be 8, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup8, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 9]` from a `Dat::Tup9`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup9dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup9dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///         dat!("four"),
///         dat!("five"),
///         dat!("six"),
///         dat!("seven"),
///         dat!("eight"),
///         dat!("nine"),
///     ];
///     let list = Dat::Tup9(Box::new(array.clone()));
///     let v = try_extract_tup9dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup9dat {
    { $src:expr } => {
        match $src {
            Dat::Tup9(v) => {
                if v.len() == 9 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup9 items should be 9, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup9, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Extracts associated `[Dat; 10]` from a `Dat::Tup10`.
/// ```
/// use oxedize_fe2o3_core::prelude::*;
/// use oxedize_fe2o3_jdat::prelude::*;
/// use oxedize_fe2o3_jdat::try_extract_tup10dat;
///
/// fn main() -> Outcome<()> { // needed because try_extract_tup10dat! can return Err
///     let array = [
///         dat!("one"),
///         dat!("two"),
///         dat!("three"),
///         dat!("four"),
///         dat!("five"),
///         dat!("six"),
///         dat!("seven"),
///         dat!("eight"),
///         dat!("nine"),
///         dat!("ten"),
///     ];
///     let list = Dat::Tup10(Box::new(array.clone()));
///     let v = try_extract_tup10dat!(list);
///     assert_eq!(v, array);
///     Ok(())
/// }
/// ```
macro_rules! try_extract_tup10dat {
    { $src:expr } => {
        match $src {
            Dat::Tup10(v) => {
                if v.len() == 10 {
                    *v
                } else {
                    return Err(err!(
                        "Number of Dat::Tup10 items should be 10, found {}.", v.len();
                    Daticle, Input, Invalid));
                }
            },
            unknown => return Err(err!(
                "Expected a Dat::Tup10, found {:?}.",
                unknown;
            Daticle, Input, Invalid)),
        }
    };
}

#[macro_export]
/// Implement infallible, consuming `From` conversion to `Dat`, as well as fallible cloning `ToDat`
/// and consuming `FromDat`.
macro_rules! to_from_dat {
    { $t:ty, $v:ident } => {
        
        impl From<$t> for Dat {
            fn from(x: $t) -> Self { Dat::$v(x) }
        }
        impl ToDat for $t {
            fn to_dat(&self) -> Outcome<Dat> { Ok(Dat::from(self.clone())) }
        }
        impl FromDat for $t {
            fn from_dat(dat: Dat) -> Outcome<Self> { Ok(try_extract_dat!(dat, $v)) }
        }
        
    };
}

#[macro_export]
/// Implement [`std::convert::From`] conversions to [`Dat`] variants using min-sizing.
macro_rules! best_from_int_to_dat {
    { $t:ty } => {
        
        impl BestFrom<$t> for Dat {
            fn best_from(x: $t) -> Self {
                DatInt::from(x).min_size().as_dat()
            }
        }
        
    };
}

#[macro_export]
/// Implement [`std::convert::From`] boxed conversions to [`Dat`] variants.
macro_rules! to_from_dat_boxed {
    { $t:ty, $v:ident } => {
        
        impl From<$t> for Dat {
            fn from(x: $t) -> Self {
                Dat::$v(Box::new(x))
            }
        }
        
    };
}

#[macro_export]
/// Provide a simple cloning getter for variants of an enum, primarily for the
/// [`FromDatMap`] and [`ToDatMap`] derive macros.
macro_rules! enum_getter {
    { $method:ident, $typ:ty, $variant:ident } => {
        
        pub fn $method(&self) -> Option<$typ> {
            if let Self::$variant(v) = self {
                Some(v.clone())
            } else {
                None
            }
        }
        
    };
}

#[macro_export]
/// For decoding homogenous byte tuples.
macro_rules! binary_decode_byte_tuple {
    { $variant:ident, $typ:ty, $n:literal, $buf:expr } => {
        
        {
            const N: usize = $n;
            const L: usize = std::mem::size_of::<$typ>();
            if $buf.len() > N * L { 
                let mut a: [$typ; N] = [0; N];
                let mut i: usize = 1;
                for j in 0..N {
                    a[j] = <$typ>::from_be_bytes(
                        res!(<[u8; L]>::try_from(&$buf[i..i+L]), Decode, Bytes)
                    );
                    i += L;
                }
                return Ok((Dat::$variant(a), i));
            } else {
                return Err(<Dat as FromBytes>::too_few(
                    $buf.len(), N * L + 1, &Dat::code_name($buf[0]), file!(), line!()));
            }
        }
    };
    { $variant:ident, $wrap:ident, $typ:ty, $n:literal, $buf:expr } => {
        
        {
            const N: usize = $n;
            const L: usize = std::mem::size_of::<$typ>();
            if $buf.len() > N * L { 
                let mut a: [$typ; N] = [0; N];
                let mut i: usize = 1;
                for j in 0..N {
                    a[j] = <$typ>::from_be_bytes(
                        res!(<[u8; L]>::try_from(&$buf[i..i+L]), Decode, Bytes)
                    );
                    i += L;
                }
                return Ok((Dat::$variant($wrap(a)), i));
            } else {
                return Err(<Dat as FromBytes>::too_few(
                    $buf.len(), N * L + 1, &Dat::code_name($buf[0]), file!(), line!()));
            }
        }
    };
}

#[macro_export]
/// For reading homogenous byte tuples.
macro_rules! binary_read_byte_tuple {
    { $variant:ident, $typ:ty, $n:literal, $r:expr, $csum:expr } => {
        
        {
            const N: usize = $n;
            const L: usize = std::mem::size_of::<$typ>();
            let mut a: [$typ; N] = [0; N];
            let mut i: usize = 1;
            for j in 0..N {
                let mut n = [0; L];
                res!($r.read_exact(&mut n));
                if let Some(csum) = $csum { res!(csum.update(&n)); }
                a[j] = <$typ>::from_be_bytes(
                    res!(<[u8; L]>::try_from(&n[..]), Decode, Bytes)
                );
                i += L;
            }
            return Ok((Dat::$variant(a), i));
        }
        
    };
    { $variant:ident, $wrap:ident, $typ:ty, $n:literal, $r:expr, $csum:expr } => {
        
        {
            const N: usize = $n;
            const L: usize = std::mem::size_of::<$typ>();
            let mut a: [$typ; N] = [0; N];
            let mut i: usize = 1;
            for j in 0..N {
                let mut n = [0; L];
                res!($r.read_exact(&mut n));
                if let Some(csum) = $csum { res!(csum.update(&n)); }
                a[j] = <$typ>::from_be_bytes(
                    res!(<[u8; L]>::try_from(&n[..]), Decode, Bytes)
                );
                i += L;
            }
            return Ok((Dat::$variant($wrap(a)), i));
        }
        
    };
}

#[macro_export]
/// For loading homogenous byte tuples.
macro_rules! binary_load_byte_tuple {
    { $variant:ident, $typ:ty, $n:literal, $r:expr, $byts:expr } => {
        
        {
            const N: usize = $n;
            const L: usize = std::mem::size_of::<$typ>();
            for _ in 0..N {
                let mut a = [0; L];
                res!($r.read_exact(&mut a));
                $byts.extend_from_slice(&a[..]);
            }
            return Ok(())
        }
        
    };
}

#[macro_export]
/// For counting homogenous byte tuples.
macro_rules! binary_count_byte_tuple {
    { $typ:ty, $n:literal, $rs:expr, $count:expr } => {
        
        {
            const N: usize = $n;
            const L: i64 = std::mem::size_of::<$typ>() as i64;
            for _ in 0..N {
                res!($rs.seek(SeekFrom::Current(L)));
                *$count += (L as usize);
            }
            return Ok(())
        }
        
    };
}

#[macro_export]
/// For unit testing of binary encoding and decoding for homogenous byte tuples.
macro_rules! test_binary_encode_decode_byte_tuple {
    { $arr:expr } => {
        
        let d1 = Dat::from($arr);
        let mut buf = Vec::new();
        buf = res!(d1.to_bytes(buf));
        let (d2, n) = res!(Dat::from_bytes(&buf));
        req!(Some(n), d1.byte_len());
        req!(d1, d2);

    };
}

#[macro_export]
/// For string decoding an homogenous byte or integer tuple.
macro_rules! string_decode_heterogenous_tuple {
    { $target:ident, $len:literal, $list:expr, $state:expr } => {
        
        return Ok(Dat::$target(Box::new(
            match $list.try_into() {
                Ok(a) => a,
                Err(_) => return Err(err!(
                    "Length of {} string is {}.", $state.kind_outer, $len;
                Input, Invalid)),
            }
        )))

    };
}

#[macro_export]
/// For string decoding an homogenous byte or integer tuple.
macro_rules! string_decode_int_tuple {
    { $target:ident, $kind:ident, $typ:ty, $len:literal, $list:expr, $state:expr } => {
        
        {
            if $list.len() != $len {
                return Err(err!(
                    "Length of {} string is {}.", $state.kind_outer, $len;
                Input, Invalid));
            }
            let mut a: [$typ; $len] = [0; $len];
            let mut i: usize = 0;
            for d in $list {
                match d {
                    Dat::$kind(n) => a[i] = n,
                    _ => return Err(err!(
                        "Expecting a daticle of kind {:?}, found {:?}.",
                        Kind::$kind, d;
                    Input, Mismatch, Unexpected, Bug)),
                }
                i += 1;
            }
            return Ok(Dat::$target(a));
        }

    };
}

#[macro_export]
/// For unit testing of string encoding and decoding of homogenous byte tuples.
macro_rules! test_string_encode_decode_homogenous_tuple {
    { $cfg:ident, $arr:expr } => {
        
        let cfg_enc = EncoderConfig::<(), ()>::$cfg(None);
        let cfg_dec = DecoderConfig::<(), ()>::$cfg(None);
        let d1 = Dat::from($arr);
        let d1_str = res!(d1.encode_string_with_config(&cfg_enc));
        let d2 = res!(Dat::decode_string_with_config(d1_str, &cfg_dec));
        req!(d1, d2);

    };
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use oxedize_fe2o3_core::prelude::*;

    #[test]
    fn test_dat() {
        let v1 = dat!(42u8);
        assert_eq!(v1, Dat::from(42u8));
    }

    #[test]
    fn test_listdat_01() {
        let v1 = listdat!["hello", "world", 42u8, listdat![ 1i16, 2i16, 256u16 ]];
        let v2 = Dat::List(vec![
            dat!("hello"),
            dat!("world"),
            dat!(42u8),
            Dat::List(vec![
                dat!(1i16),
                dat!(2i16),
                dat!(256u16),
            ]),
        ]);
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_listdat_02() {
        let v1 = listdat![1u16];
        let v2 = Dat::List(vec![dat!(1u16)]);
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_try_extract_dat_00() -> Outcome<()> {
        let n = 42u8;
        let d = dat!(n);
        let e = try_extract_dat!(d, U8);
        assert_eq!(n, e);
        Ok(())
    }

    #[test]
    fn test_mapdat_01() -> Outcome<()> {
        let v1: Dat = mapdat!{
            listdat![1u16] => listdat![1u16, 2u16, 3u16],
            listdat![2u16] => listdat![1u16, 2u16, 3u16],
        };
        //assert_eq!(v1, v2);
        println!("map = {:?}",v1);
        Ok(())
    }

    #[test]
    fn test_mapdat_02() -> Outcome<()> {
        let v1 = mapdat!{
            "Meaning of life" => 42u8,
            "key" => "value",
            "map" => mapdat!{
                1i16 => Dat::I32(2),
                3u8 => 4i8,
            },
        };
        let mut m1 = DaticleMap::new();
        m1.insert(dat!("Meaning of life"), dat!(42u8)); 
        m1.insert(dat!("key"), dat!("value")); 
        let mut m2 = DaticleMap::new();
        m2.insert(dat!(1i16), dat!(2));
        m2.insert(dat!(3u8), dat!(4i8));
        m1.insert(dat!("map"), Dat::Map(m2));
        let v2 = Dat::Map(m1);
        assert_eq!(v1, v2);
        Ok(())
    }
}
