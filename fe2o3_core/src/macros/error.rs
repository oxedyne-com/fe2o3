#[macro_export]
/// Create context for an Error.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
///
/// let n = 41;
/// let result0: Outcome<()> = Err(Error::Local(ErrMsg {
///     tags: &[ErrTag::Invalid, ErrTag::Input],
///     msg: errmsg!("The meaning of life is not {}", n),
/// }));
///```
macro_rules! errmsg {
    () => (
        format!("{}:{}", file!(), line!())
    );
    ($($arg:tt)*) => (
        format!("{}:{}: {}", file!(), line!(), format!($($arg)*))
    )
}

#[macro_export]
/// Create an Error with context info and tags.
///
/// Format: err!(message; tags)
/// Where message can be:
/// - A string literal
/// - A format string with arguments  
/// And compulsory tags are comma-separated ErrTag identifiers
///
/// # Examples
///
/// ## Local Errors
/// ```ignore
/// use oxedize_fe2o3_core::prelude::*;
/// 
/// // Simple message
/// let e1 = err!("Just text"; Input);
///
/// // Multiple tags
/// let e2 = err!("Simple message"; Input, Invalid);
///
/// // Format string with arguments
/// let value = 42;
/// let e3 = err!("Value is {}", value; Input);
///
/// // Multiple arguments and tags
/// let (val1, val2) = (1, 2);
/// let e4 = err!("Values are {} and {}", val1, val2; Input, Invalid);
/// ```
///
/// ## Upstream Errors 
/// ```ignore
/// use oxedize_fe2o3_core::prelude::*;
/// use std::fs;
///
/// let io_error = fs::read_to_string("missing.txt").unwrap_err();
/// 
/// // Simple message
/// let e1 = err!(io_error, "Failed to read file"; IO, File);
///
/// // With format args
/// let filename = "config.txt";
/// let e2 = err!(io_error, "Failed to read {}", filename; IO, File);
/// ```
macro_rules! err {
    // Local error with simple message and tags
    ($msg:expr; $($tag:ident),+) => {
        Error::Local(ErrMsg {
            msg: format!("{}:{}: {}", file!(), line!(), $msg),
            tags: &[$(ErrTag::$tag),+],
        })
    };

    // Local error with format string, args and tags
    ($fmt:literal, $($arg:expr),+; $($tag:ident),+) => {
        Error::Local(ErrMsg {
            msg: format!("{}:{}: {}", file!(), line!(), format!($fmt, $($arg),+)),
            tags: &[$(ErrTag::$tag),+],
        })
    };

    // Upstream error with simple message and tags
    ($err:expr, $msg:expr; $($tag:ident),+) => {
        Error::Upstream(std::sync::Arc::new($err), ErrMsg {
            msg: format!("{}:{}: {}", file!(), line!(), $msg),
            tags: &[$(ErrTag::$tag),+],
        })
    };

    // Upstream error with format string, args and tags
    ($err:expr, $fmt:literal, $($arg:expr),+; $($tag:ident),+) => {
        Error::Upstream(std::sync::Arc::new($err), ErrMsg {
            msg: format!("{}:{}: {}", file!(), line!(), format!($fmt, $($arg),+)),
            tags: &[$(ErrTag::$tag),+],
        })
    };
}

#[macro_export]
/// A prefix alternative to the `?` operator for error propagation.
///
/// This macro provides identical functionality to the `?` operator but uses prefix notation.
/// It converts errors using the standard `From` trait and propagates them to the caller.
///
/// # Examples
///
/// Basic usage:
/// ```ignore
/// use fe2o3_core::prelude::*;
/// use std::fs::File;
/// 
/// fn read_file() -> std::io::Result<()> {
///     let file = ok!(File::create("data.txt"));
///     Ok(())
/// }
/// ```
///
/// With different error types:
/// ```ignore
/// use fe2o3_core::prelude::*;
/// use std::error::Error;
/// 
/// fn process_data() -> Result<i32, Box<dyn Error>> {
///     // Both errors will be converted to Box<dyn Error>
///     let file = ok!(std::fs::read_to_string("numbers.txt"));
///     let number = ok!(file.parse::<i32>());
///     Ok(number)
/// }
/// ```
///
/// # Performance
/// This macro has the same performance characteristics as the `?` operator,
/// as it expands to identical code using the `From` trait for error conversion.
///
/// # Note
/// Unlike `res!` and `catch!`, this macro does not add any context or catch panics.
/// Use this macro in performance-critical code paths where standard error
/// propagation is sufficient.
macro_rules! ok {
    ($expr:expr) => {
        ($expr)?
    };
}

#[macro_export]
/// Propagates errors and adds context through error tags while maintaining the error chain.
///
/// Similar to `ok!`, but wraps both Rust errors and std error trait objects to add context.
/// Use this for general application code where error context is valuable.
///
/// # Examples
///
/// Basic usage with tags:
/// ```ignore
/// use fe2o3_core::prelude::*;
/// 
/// fn process_data() -> Outcome<()> {
///     // Adds IO and Parse tags to any error
///     let data = res!(read_file(), IO, Parse);
///     Ok(())
/// }
/// ```
///
/// Chaining errors (not nested):
/// ```ignore
/// let intermediate = res!(first_operation(), IO);
/// let result = res!(second_operation(intermediate), Processing);
/// ```
///
/// # Note
/// - Cannot be nested recursively due to return type limitations
/// - Adds some overhead from Arc and context capture
/// - For performance-critical code paths, consider using `ok!` instead
macro_rules! res {
    ($res:expr, $($etvars:ident),* $(,)?) => {
        match $res {
            Ok(v) => v,
            Err(e) => {
                return Err(Error::Upstream(std::sync::Arc::new(e), ErrMsg {
                    tags: &[ $(ErrTag::$etvars),* ],
                    msg: errmsg!(),
                }));
            },
        }
    };
    ($res:expr, $($enum:ident::$etvars:ident),* $(,)?) => {
        match $res {
            Ok(v) => v,
            Err(e) => {
                return Err(Error::Upstream(std::sync::Arc::new(e), ErrMsg {
                    tags: &[ $($enum::$etvars),* ],
                    msg: errmsg!(),
                }));
            },
        }
    };
    ($res:expr) => {
        match $res {
            Ok(v) => v,
            Err(e) => {
                return Err(Error::Upstream(std::sync::Arc::new(e), ErrMsg {
                    tags: &[],
                    msg: errmsg!(),
                }));
            },
        }
    }
}

#[macro_export]
/// Propagates errors while catching unwinding panics and adding context.
///
/// Most comprehensive error handling macro - converts both errors and unwinding panics
/// into `Outcome::Err` while maintaining context. Use this at application boundaries
/// where panic recovery is important.
///
/// # Examples
///
/// Basic usage:
/// ```ignore
/// use fe2o3_core::prelude::*;
///
/// fn handle_request() -> Outcome<Response> {
///     // Will catch panics and convert them to errors
///     let result = catch!(process_request(), Request, Processing);
///     Ok(Response::new(result))
/// }
/// ```
///
/// # Panics
/// Catches most unwinding panics including:
/// - Array bounds violations
/// - Integer overflow in debug builds
/// - Unwrap/expect failures
/// - Division by zero
///
/// Does not catch:
/// - Stack overflows
/// - Memory allocation failures
/// - Panics in destructors
/// - FFI panics marked `#[no_unwind]`
/// - Any panics when compiled with `panic=abort`
///
/// # Performance
/// Has significant overhead due to:
/// - Unwinding tables in binary
/// - Stack frame management
/// - Register state tracking
///
/// Use only at key boundaries where panic recovery justifies the cost.
macro_rules! catch {
    ($res:expr, $($etvars:ident),* $(,)?) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $res
        })) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(Error::Upstream(std::sync::Arc::new(e), ErrMsg {
                tags: &[ $(ErrTag::$etvars),* ],
                msg: errmsg!(),
            })),
            Err(cause) => {
                let msg = if let Some(s) = cause.downcast_ref::<&str>() {
                    s
                } else if let Some(s) = cause.downcast_ref::<String>() {
                    s.as_str()
                } else if let Some(box_any) = cause.downcast_ref::<Box<dyn std::any::Any + Send + Sync>>() {
                    if let Some(string) = box_any.downcast_ref::<String>() {
                        string.as_str()
                    } else {
                        "A panic occurred, but the message is not a string."
                    }
                } else {
                    "A panic occurred, but the message could not be extracted."
                };
                return Err(Error::Local(ErrMsg {
                    tags: &[ ErrTag::Panic, $(ErrTag::$etvars),* ],
                    msg: errmsg!("A panic occurred: {}", msg),
                }));
            },
        }
    };
    ($res:expr) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $res
        })) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                return Err(Error::Upstream(std::sync::Arc::new(e), ErrMsg {
                    tags: &[],
                    msg: errmsg!(),
                }));
            },
            Err(cause) => {
                let msg = if let Some(s) = cause.downcast_ref::<&str>() {
                    s
                } else if let Some(s) = cause.downcast_ref::<String>() {
                    s.as_str()
                } else if let Some(box_any) = cause.downcast_ref::<Box<dyn std::any::Any + Send + Sync>>() {
                    if let Some(string) = box_any.downcast_ref::<String>() {
                        string.as_str()
                    } else {
                        "A panic occurred, but the message is not a string."
                    }
                } else {
                    "A panic occurred, but the message could not be extracted."
                };
                return Err(Error::Local(ErrMsg {
                    tags: &[ ErrTag::Panic ],
                    msg: errmsg!("A panic occurred: {}", msg),
                }));
            },
        }
    }
}

#[macro_export]
/// While `catch!` can handle any error type that implements `std::error::Error`, this macro deals
/// with cases like `anyhow::Error`, which do not.  It can be difficult or impossible to get the
/// error out as a `std::error::Error` so we just use the `String`.
macro_rules! catch_other {
    ($res:expr, $($etvars:ident),* $(,)?) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $res
        })) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(Error::Other(
                ErrMsg {
                    tags: &[ErrTag::Upstream],
                    msg: e.to_string(),
                }
            )),
            Err(cause) => {
                let msg = if let Some(s) = cause.downcast_ref::<&str>() {
                    s
                } else if let Some(s) = cause.downcast_ref::<String>() {
                    s.as_str()
                } else if let Some(box_any) = cause.downcast_ref::<Box<dyn std::any::Any + Send + Sync>>() {
                    if let Some(string) = box_any.downcast_ref::<String>() {
                        string.as_str()
                    } else {
                        "A panic occurred, but the message is not a string."
                    }
                } else {
                    "A panic occurred, but the message could not be extracted."
                };
                return Err(Error::Local(ErrMsg {
                    tags: &[ ErrTag::Panic, $(ErrTag::$etvars),* ],
                    msg: errmsg!("A panic occurred: {}", msg),
                }));
            },
        }
    };
    ($res:expr) => {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            $res
        })) {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(Error::Other(
                ErrMsg {
                    tags: &[ErrTag::Upstream],
                    msg: e.to_string(),
                }
            )),
            Err(cause) => {
                let msg = if let Some(s) = cause.downcast_ref::<&str>() {
                    s
                } else if let Some(s) = cause.downcast_ref::<String>() {
                    s.as_str()
                } else if let Some(box_any) = cause.downcast_ref::<Box<dyn std::any::Any + Send + Sync>>() {
                    if let Some(string) = box_any.downcast_ref::<String>() {
                        string.as_str()
                    } else {
                        "A panic occurred, but the message is not a string."
                    }
                } else {
                    "A panic occurred, but the message could not be extracted."
                };
                return Err(Error::Local(ErrMsg {
                    tags: &[ ErrTag::Panic ],
                    msg: errmsg!("A panic occurred: {}", msg),
                }));
            },
        }
    }
}
