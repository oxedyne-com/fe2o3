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

#[macro_export]
/// Acquires a `RwLock` read guard, recovering from a poisoned lock.
///
/// Unlike [`lock_read!`], which returns an error when the lock is
/// poisoned, this macro logs a warning and continues with the inner
/// guard via `into_inner()`. It suits long-running services where a
/// single panic must not cascade into a persistent failure of every
/// subsequent operation that touches the lock. The recovered data may
/// be in an inconsistent state, so use this only where continuing to
/// serve is safer than aborting. The optional trailing arguments form
/// the warning message.
macro_rules! lock_read_or_recover {
    ($locked:expr, $($arg:tt)*) => {
        match $locked.read() {
            Ok(guard)     => guard,
            Err(poisoned) => {
                warn!($($arg)*);
                poisoned.into_inner()
            },
        }
    };
    ($locked:expr) => {
        match $locked.read() {
            Ok(guard)     => guard,
            Err(poisoned) => {
                warn!("RwLock poisoned while reading; recovering with possibly inconsistent data.");
                poisoned.into_inner()
            },
        }
    };
}

#[macro_export]
/// Acquires a `RwLock` write guard, recovering from a poisoned lock.
///
/// The write counterpart of [`lock_read_or_recover!`]. It logs a
/// warning and continues with the inner guard rather than returning an
/// error, for long-running services where staying available is
/// preferable to aborting on a poisoned lock. The recovered data may
/// be inconsistent. The optional trailing arguments form the warning
/// message.
macro_rules! lock_write_or_recover {
    ($locked:expr, $($arg:tt)*) => {
        match $locked.write() {
            Ok(guard)     => guard,
            Err(poisoned) => {
                warn!($($arg)*);
                poisoned.into_inner()
            },
        }
    };
    ($locked:expr) => {
        match $locked.write() {
            Ok(guard)     => guard,
            Err(poisoned) => {
                warn!("RwLock poisoned while writing; recovering with possibly inconsistent data.");
                poisoned.into_inner()
            },
        }
    };
}

#[macro_export]
/// Acquires a `Mutex` guard, recovering from a poisoned lock.
///
/// The `Mutex` counterpart of [`lock_write_or_recover!`]. It logs a
/// warning and continues with the inner guard rather than returning an
/// error. The recovered data may be inconsistent. The optional
/// trailing arguments form the warning message.
macro_rules! lock_mutex_or_recover {
    ($locked:expr, $($arg:tt)*) => {
        match $locked.lock() {
            Ok(guard)     => guard,
            Err(poisoned) => {
                warn!($($arg)*);
                poisoned.into_inner()
            },
        }
    };
    ($locked:expr) => {
        match $locked.lock() {
            Ok(guard)     => guard,
            Err(poisoned) => {
                warn!("Mutex poisoned; recovering with possibly inconsistent data.");
                poisoned.into_inner()
            },
        }
    };
}

