#[macro_export]
/// Captures and propagates a `RwLock` poisoning error when reading.
///
///```
/// use oxedyne_fe2o3_core::prelude::*;
/// use std::sync::{std::sync::Arc, RwLock};
/// 
/// fn main() -> Outcome<()> {
///     let n = std::sync::Arc::new(RwLock::new(42));
///     let n_ul = lock_read!(n);
///     assert_eq!(*n_ul, 42);
///     Ok(())
/// }
///
///```
macro_rules! lock_read {
    ($locked:expr, $($arg:tt)*) => {
        match $locked.read() {
            Err(_) => {
                return Err(Error::Local(ErrMsg {
                    tags: &[ErrTag::Lock, ErrTag::Poisoned, ErrTag::Read],
                    msg: errmsg!($($arg)*),
                }));
            },
            Ok(v) => v,
        }
    };
    ($locked:expr) => {
        match $locked.read() {
            Err(_) => {
                return Err(Error::Local(ErrMsg {
                    tags: &[ErrTag::Lock, ErrTag::Poisoned, ErrTag::Read],
                    msg: errmsg!("While locking {:?} for reading", $locked),
                }));
            },
            Ok(v) => v,
        }
    }
}

#[macro_export]
/// Captures and propagates a `RwLock` poisoning error when writing.
///
///```
/// use oxedyne_fe2o3_core::prelude::*;
/// use std::sync::{std::sync::Arc, RwLock};
/// 
/// fn main() -> Outcome<()> {
///     let n = std::sync::Arc::new(RwLock::new(41));
///     let mut n_ul = lock_write!(n);
///     *n_ul += 1;
///     assert_eq!(*n_ul, 42);
///     Ok(())
/// }
///
///```
macro_rules! lock_write {
    ($locked:expr, $($arg:tt)*) => {
        match $locked.write() {
            Err(_) => {
                return Err(Error::Local(ErrMsg {
                    tags: &[ErrTag::Lock, ErrTag::Poisoned, ErrTag::Write],
                    msg: errmsg!($($arg)*),
                }));
            },
            Ok(v) => v,
        }
    };
    ($locked:expr) => {
        match $locked.write() {
            Err(_) => {
                return Err(Error::Local(ErrMsg {
                    tags: &[ErrTag::Lock, ErrTag::Poisoned, ErrTag::Write],
                    msg: errmsg!("While locking {:?} for writing", $locked),
                }));
            },
            Ok(v) => v,
        }
    }
}

