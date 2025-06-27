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

#[macro_export]
/// Captures and propagates a `Mutex` poisoning error when locking.
///
///```
/// use oxedyne_fe2o3_core::prelude::*;
/// use std::sync::{Arc, Mutex};
/// 
/// fn main() -> Outcome<()> {
///     let n = Arc::new(Mutex::new(42));
///     let guard = lock_mutex!(n);
///     assert_eq!(*guard, 42);
///     Ok(())
/// }
///
///```
macro_rules! lock_mutex {
    ($locked:expr, $($arg:tt)*) => {
        match $locked.lock() {
            Err(_) => {
                return Err(Error::Local(ErrMsg {
                    tags: &[ErrTag::Lock, ErrTag::Poisoned],
                    msg: errmsg!($($arg)*),
                }));
            },
            Ok(v) => v,
        }
    };
    ($locked:expr) => {
        match $locked.lock() {
            Err(_) => {
                return Err(Error::Local(ErrMsg {
                    tags: &[ErrTag::Lock, ErrTag::Poisoned],
                    msg: errmsg!("Mutex lock failed: poisoned lock"),
                }));
            },
            Ok(v) => v,
        }
    }
}

#[macro_export]
/// Handles `Mutex` locks in thread contexts where we can't return `Outcome`.
/// Prints an error message and returns early on poisoned mutex.
///
///```ignore
/// // Use in thread contexts
/// use oxedyne_fe2o3_core::prelude::*;
/// use std::sync::{Arc, Mutex};
/// use std::thread;
/// 
/// let data = Arc::new(Mutex::new(42));
/// let data_clone = Arc::clone(&data);
/// 
/// thread::spawn(move || {
///     let guard = lock_mutex_thread!(data_clone, "updating data");
///     // Use guard...
/// });
///```
macro_rules! lock_mutex_thread {
    ($locked:expr, $context:expr) => {
        match $locked.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Thread: poisoned mutex in {}", $context);
                return;
            }
        }
    };
}

