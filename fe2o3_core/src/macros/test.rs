#[macro_export]
/// A simple test harness.  If the given string matches any of the tags, the test is run.
///
macro_rules! test_it {
    ($filter:expr, $name:literal, $($tag:literal),* $(,)? { $($code:tt)* }) => {
        match $filter {
            $($tag)|* | $name => {
                test!("'{}' test...", $name);
                let _outcome = res! ( Outcome::Ok( { $($code)* } ) );
            },
            _ => (),
        }
    };
}

#[macro_export]
/// A non-panicking `assert_eq` that returns an `Err` when left and right, compared via
/// `PartialEq`, do not match.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// fn main() -> Outcome<()> {
///     let v1 = 42u8;
///     let v2 = 42u8;
///     req!(v1, v2); // Will return with an Outcome::Err if not equal
///     Ok(())
/// }
///
///```
macro_rules! req {
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left != $right {
            return Err(Error::Local(ErrMsg {
                tags: &[ErrTag::Test, ErrTag::Mismatch],
                msg: errmsg!(
                    "Left value {:?} does not match right value {:?}: {}",
                    $left, $right, fmt!($($arg)*),
                ),
            }));
        }
    };
    ($left:expr, $right:expr $(,)?) => {
        if $left != $right {
            return Err(Error::Local(ErrMsg {
                tags: &[ErrTag::Test, ErrTag::Mismatch],
                msg: errmsg!("Left value {:?} does not match right value {:?}", $left, $right),
            }));
        }
    }
}
