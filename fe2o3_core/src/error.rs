//! # Error Handling in Hematite
//! 
//! Hematite's error handling system balances performance with robust error management through 
//! a layered approach to error propagation and contextualisation.
//!
//! ## Core Types
//!
//! - `Outcome<V>`: The main result type, an alias for `strd::Result::Result<V, Error<error::ErrTag>>`
//! - `Error<T>`: An error type that can represent local errors, upstream errors, or error collections
//! - `ErrMsg`: Contains an error message and associated tags
//! - `ErrTag`: An enumeration of standard error categories
//!
//! ## Error Handling Macros
//!
//! Hematite provides three error handling macros with different capabilities:
//!
//! ### `ok!` - Basic Error Propagation
//! ```rust
//! // Equivalent to the ? operator but with prefix syntax.
//! let file = ok!(File::create("data.txt"));
//! ```
//! - Lightweight error propagation using standard `From` trait
//! - A prefix alias for the `?` operator suffix
//! - Ideal for performance-critical paths
//!
//! ### `res!` - Contextual Error Propagation
//! ```rust
//! // Adds context and tags to errors.
//! let file = res!(File::create("data.txt"), IO, File);
//! ```
//! - Short for "result"
//! - Adds file/line context and error tags
//! - Thread-safe error sharing via `Arc`
//! - Small overhead from context capture
//! - Suitable for general application code
//!
//! ### `catch!` - Panic-Catching Error Propagation
//! ```rust
//! // Converts both errors and panics into Outcome
//! let value = catch!(compute_result(), Numeric, Overflow);
//! ```
//! - Catches unwinding panics and converts them to errors
//! - Adds context and tags like `ok!`
//! - Higher overhead due to unwinding tables
//! - Most suitable for top-level error boundaries
//!
//! ## Error Tags
//!
//! Error tags provide multi-dimensional error classification:
//!
//! ```rust
//! // Multiple tags can be attached
//! let result = res!(operation(), IO, Network, Timeout);
//!
//! // Tags can be matched efficiently
//! if err.tags().contains(&ErrTag::Timeout) {
//!     // Handle timeout case
//! }
//! ```
//!
//! ## Performance Considerations
//!
//! - `ok!`: Minimal overhead, uses standard error conversion
//! - `res!`: Small overhead from `Arc` and context capture
//! - `catch!`: Larger overhead from unwinding support
//!   - Requires unwinding tables in binary
//!   - Extra stack frame management
//!   - Cannot be nested due to closure limitations
//!
//! ## Design Philosophy
//!
//! Hematite's error handling aims to:
//! - Provide flexible error handling tools for different needs
//! - Make error handling strategies explicit at call sites
//! - Allow users to choose appropriate performance/feature tradeoffs
//! - Maintain rich context for debugging
//! - Convert panics to manageable errors where beneficial
//!
//! The system balances comprehensive error handling against performance, allowing developers to
//! choose the appropriate level of error management for their specific needs.
//!
//! The default macro used in Hematite is `res!` for enhanced error reporting.  Foundational code
//! should migrate to `ok!` when improved performance is needed, while `catch!` should be used for
//! top-level application code.
//!
use crate::GenTag;

use oxedize_fe2o3_stds::chars::Term;

use std::{
    fmt,
    io,
    num,
    string,
    sync::Arc,
};
use std::convert::From;


#[allow(non_camel_case_types)] 
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrTag {
    Async,
    Borrow,
    Bug, // The fault is with the developer, not the user.
    Bytes, // TODO deprecate
    Binary,
    Channel,
    Checksum,
    Configuration,
    Conflict,
    Conversion,
    Counter,
    Create,
    Data,
    Daticle,
    Decode,
    Decrypt,
    Divisibility,
    Duplicate,
    Encode,
    Encrypt,
    Excessive,
    Exists,
    Fatal,
    File,
    Format,
    Identifier,
    Index,
    Init,
    Input,
    Integer,
    Interrupted,
    Invalid,
    IO,
    Key,
    LimitReached,
    Lock,
    Mismatch,
    Missing,
    Name,
    Network,
    NoImpl,
    NotFound,
    Numeric,
    Order,
    Output,
    Overflow,
    Panic,
    Path,
    Poisoned,
    Range,
    Read,
    Seek,
    Size,
    Slice,
    String,
    Suggestion,
    System,
    Test,
    Timeout,
    Thread,
    TooBig,
    TooSmall,
    Wire,
    Write,
    Unauthorised,
    Underflow,
    Unexpected,
    Unimplemented,
    Unknown,
    Unreachable,
    Upstream,
    UTF8,
    Value,
    Version,
    ZeroDenominator,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ErrMsg<T: GenTag> {
    pub msg:    String,
    pub tags:   &'static [T],
}

impl<T: GenTag> fmt::Display for ErrMsg<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone)]
pub enum Error<T: GenTag> {
    Local(ErrMsg<T>),
    Other(ErrMsg<T>),
    Upstream(Arc<dyn std::error::Error + Send + Sync>, ErrMsg<T>),
    Collection(Vec<Box<Self>>),
}

impl<T: GenTag> Error<T> where Error<T>: std::error::Error {

    pub fn tags(&self) -> Vec<T> {
        match self {
            Error::Local(ErrMsg { tags: t, ..}) |
            Error::Other(ErrMsg { tags: t, ..}) => t.to_vec(),
            Error::Upstream(_, ErrMsg { tags: t, ..}) => t.to_vec(),
            Error::Collection(boxerrs) => {
                let mut t = Vec::new();
                for e in boxerrs {
                    for tag in (*e).tags() {
                        t.push(tag.clone())
                    }
                }
                t
            },
        }
    }

    pub fn tags_display(tags: Vec<T>) -> String {
        let mut result = String::new();
        if tags.len() > 0 {
            result.push('[');
            let mut c = 0;
            for tag in tags {
                if c > 0 {
                    result.push(' ');
                }
                result.push_str(&tag.to_string());
                c += 1;
            }
            result.push(']');
        }
        result
    }

    fn fmt_debug_local(
        f: &mut fmt::Formatter<'_>,
        m: &str,
        t: &'static [T],
    )
        -> fmt::Result
    {
        write!(f, "LocalErr{{{}{}}}",
            Self::tags_display(t.to_vec()),
            if m.len() > 0 {
                if t.len() > 0 {
                    fmt!(" \"{}\"", m)
                } else {
                    fmt!("\"{}\"", m)
                }
            } else {
                String::new()
            },
        )
    }

    fn fmt_debug_upstream_specific(
        f: &mut fmt::Formatter<'_>,
        e: &Self,
        m: &str,
        t: &'static [T],
    )
        -> fmt::Result
    {
        write!(f, "UpstreamErr{{{}{}}}\n{}",
            Self::tags_display(t.to_vec()),
            if m.len() > 0 {
                if t.len() > 0 {
                    fmt!(" \"{}\"", m)
                } else {
                    fmt!("\"{}\"", m)
                }
            } else {
                String::new()
            },
            e,
        )
    }

    fn fmt_debug_upstream_general(
        f:      &mut fmt::Formatter<'_>,
        arc_e:  &Arc<dyn std::error::Error + Send + Sync>,
        m:      &str,
        t:      &'static [T],
    )
        -> fmt::Result
    {
        write!(f, "UpstreamErr{{{}{}}}",
            Error::tags_display(t.to_vec()),
            if m.len() > 0 {
                if t.len() > 0 {
                    fmt!(" \"{}\" \"{:?} {}\"",
                        m,
                        arc_e,
                        arc_e,
                    )
                } else {
                    fmt!("\"{}\" \"{:?} {}\"",
                        m,
                        arc_e,
                        arc_e,
                    )
                }
            } else {
                String::new()
            },
        )
    }

    fn fmt_display_local(
        f: &mut fmt::Formatter<'_>,
        m: &str,
        t: &'static [T],
    )
        -> fmt::Result
    {
        write!(f,
            //1 2           3 4 5 6 7 8 91011  12
            "{}{}LocalErr{{{}{}{}{}{}{}{}{}{}}}{}",
            Term::SET_BRIGHT_FORE_RED,      // 1
            Term::BOLD,                     // 2
            Term::RESET,                    // 3
            Term::FORE_MAGENTA,             // 4
            Error::tags_display(t.to_vec()),         // 5
            Term::RESET,                    // 6
            Term::SET_BRIGHT_FORE_YELLOW,   // 7
            if m.len() > 0 {
                if t.len() > 0 {
                    fmt!(" \"{}\"", m)   // 8
                } else {
                    fmt!("\"{}\"", m)    // 8
                }
            } else {
                String::new()               // 8
            },
            Term::RESET,                    // 9
            Term::SET_BRIGHT_FORE_RED,      // 10
            Term::BOLD,                     // 11
            Term::RESET,                    // 12
        )
    }

    fn fmt_display_upstream_specific(
        f: &mut fmt::Formatter<'_>,
        e: &Self,
        m: &str,
        t: &'static [T],
    )
        -> fmt::Result
    {
        write!(f,
            //1 2              3 4 5 6 7 8 91011  12  13
            "{}{}UpstreamErr{{{}{}{}{}{}{}{}{}{}}}{}\n{}",
            Term::SET_BRIGHT_FORE_RED,      // 1
            Term::BOLD,                     // 2
            Term::RESET,                    // 3
            Term::FORE_MAGENTA,             // 4
            Error::tags_display(t.to_vec()),         // 5
            Term::RESET,                    // 6
            Term::SET_BRIGHT_FORE_CYAN,     // 7
            if m.len() > 0 {
                if t.len() > 0 {
                    fmt!(" \"{}\"", m)   // 8
                } else {
                    fmt!("\"{}\"", m)    // 8
                }
            } else {
                String::new()               // 8
            },
            Term::RESET,                    // 9
            Term::SET_BRIGHT_FORE_RED,      // 10
            Term::BOLD,                     // 11
            Term::RESET,                    // 12
            e,                              // 13
        )
    }

    fn fmt_display_upstream_general(
        f:      &mut fmt::Formatter<'_>,
        arc_e:  &Arc<dyn std::error::Error + Send + Sync>,
        m:      &str,
        t:      &'static [T],
    )
        -> fmt::Result
    {
        write!(f,
            //1 2              3 4 5 6 7 8 91011  12
            "{}{}UpstreamErr{{{}{}{}{}{}{}{}{}{}}}{}",
            Term::SET_BRIGHT_FORE_RED,      // 1
            Term::BOLD,                     // 2
            Term::RESET,                    // 3
            Term::FORE_MAGENTA,             // 4
            Error::tags_display(t.to_vec()),         // 5
            Term::RESET,                    // 6
            Term::SET_BRIGHT_FORE_CYAN,     // 7
            if m.len() > 0 {
                if t.len() > 0 {
                    fmt!(" \"{}\"{}{} {}\"{:?} {}\"{}",   // 8
                        m,
                        Term::RESET,
                        Term::SET_BRIGHT_FORE_BLACK,
                        Term::BACK_YELLOW,
                        arc_e,
                        arc_e,
                        Term::RESET,
                    )
                } else {
                    fmt!("\"{}\"{}{} {}\"{:?} {}\"{}",    // 8
                        m,
                        Term::RESET,
                        Term::SET_BRIGHT_FORE_BLACK,
                        Term::BACK_YELLOW,
                        arc_e,
                        arc_e,
                        Term::RESET,
                    )
                }
            } else {
                String::new()                       // 8
            },
            Term::RESET,                    // 9
            Term::SET_BRIGHT_FORE_RED,      // 10
            Term::BOLD,                     // 11
            Term::RESET,                    // 12
        )
    }
}

/// Plain, without ANSI terminal colour codes.
impl<T: GenTag> fmt::Debug for Error<T> where Error<T>: std::error::Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Local(ErrMsg {msg:m, tags: t}) |
            Error::Other(ErrMsg {msg:m, tags: t}) => Self::fmt_debug_local(f, m, t),
            Error::Upstream(arc_e, ErrMsg{msg: m, tags: t}) => match arc_e.downcast_ref::<Error<T>>() {
                Some(e) => Self::fmt_debug_upstream_specific(f, e, m, t),
                None => Self::fmt_debug_upstream_general(f, arc_e, m, t),
            },
            Error::Collection(boxerrs) => {
                writeln!(f, "Collection of {} errors:", boxerrs.len())?;
                for (i, boxerr) in boxerrs.iter().enumerate() {
                    writeln!(f, "{:04}: {:?}", i, *boxerr)?;
                }
                Ok(())
            },
        }
    }
}

/// For console use.
impl<T: GenTag> fmt::Display for Error<T> where Error<T>: std::error::Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Local(ErrMsg {tags: t, msg: m}) |
            Error::Other(ErrMsg {tags: t, msg: m}) => Self::fmt_display_local(f, m, t),
            Error::Upstream(arc_e, ErrMsg{tags: t, msg: m}) => match arc_e.downcast_ref::<Error<T>>() {
                Some(e) => Self::fmt_display_upstream_specific(f, e, m, t),
                None => Self::fmt_display_upstream_general(f, arc_e, m, t),
            },
            Error::Collection(boxerrs) => {
                writeln!(f, "Collection of {} errors:", boxerrs.len())?;
                for (i, boxerr) in boxerrs.iter().enumerate() {
                    writeln!(f, "{:04}: {}", i, *boxerr)?;
                }
                Ok(())
            },
        }
    }
}

// Automate conversions for standard library errors.
//
impl std::error::Error for Error<ErrTag> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Upstream(arc_e, _) => Some(arc_e.as_ref()),
            _ => None,
        }
    }
}

impl From<fmt::Error> for Error<ErrTag> {
    fn from(e: fmt::Error) -> Self {
        Error::Upstream(Arc::new(e), ErrMsg {
            tags: &[ErrTag::Format],
            msg: String::new(),
        })
    }
}

impl From<io::Error> for Error<ErrTag> {
    fn from(e: io::Error) -> Self {
        Error::Upstream(Arc::new(e), ErrMsg {
            tags: &[ErrTag::IO],
            msg: String::new(),
        })
    }
}

impl From<string::FromUtf8Error> for Error<ErrTag> {
    fn from(e: string::FromUtf8Error) -> Self {
        Error::Upstream(Arc::new(e), ErrMsg {
            tags: &[ErrTag::Decode, ErrTag::UTF8, ErrTag::String],
            msg: String::new(),
        })
    }
}

impl From<std::str::Utf8Error> for Error<ErrTag> {
    fn from(e: std::str::Utf8Error) -> Self {
        Error::Upstream(Arc::new(e), ErrMsg {
            tags: &[ErrTag::Decode, ErrTag::UTF8, ErrTag::String],
            msg: String::new(),
        })
    }
}

impl From<num::ParseIntError> for Error<ErrTag> {
    fn from(e: num::ParseIntError) -> Self {
        Error::Upstream(Arc::new(e), ErrMsg {
            tags: &[ErrTag::Decode, ErrTag::Integer, ErrTag::String],
            msg: String::new(),
        })
    }
}

impl From<std::array::TryFromSliceError> for Error<ErrTag> {
    fn from(e: std::array::TryFromSliceError) -> Self {
        Error::Upstream(Arc::new(e), ErrMsg {
            tags: &[ErrTag::Conversion, ErrTag::Slice],
            msg: String::new(),
        })
    }
}

impl From<std::time::SystemTimeError> for Error<ErrTag> {
    fn from(e: std::time::SystemTimeError) -> Self {
        Error::Upstream(Arc::new(e), ErrMsg {
            tags: &[ErrTag::Conversion, ErrTag::Slice],
            msg: String::new(),
        })
    }
}

impl<T> From<std::sync::PoisonError<T>> for Error<ErrTag> {
    fn from(_e: std::sync::PoisonError<T>) -> Self {
        Error::Local(ErrMsg {
            tags: &[ErrTag::Poisoned],
            msg: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errmsg,
        Outcome,
    };
    use std::{
        fmt::write,
        str::FromStr,
    };

    fn return_fmt_error() -> Outcome<i32> {
        let mut output = String::new();
        write(&mut output, format_args!("Hello {}!", "world"))?;
        Ok(42)
    }

    #[test]
    fn test_errctx() -> Outcome<()> {
        let m = errmsg!("The meaning of life is {}", 42);
        println!("ErrMsg = {}", m);
        let m = errmsg!();
        println!("ErrMsg = {}", m);
        let n = 41;
        let e = Error::Local(ErrMsg {
            tags: &[ErrTag::Invalid],
            msg: errmsg!("The meaning of life is not quite {}", n),
        });
        println!("This is a test of an Error: {}", e);

        Ok(())
    }

    #[test]
    fn test_errprop_00() -> Outcome<()> {
        let res0 = Outcome::Ok(());
        let res1 = res!(res0);
        msg!("{:?}", res1);
        Ok(())
    }

    #[test]
    fn test_err_00() -> Outcome<()> {
        let e0 = err!(fmt!("A test {}", 42), String, Invalid);
        msg!("{:?}", e0);
        let e1 = Error::Local(ErrMsg { tags: &[ErrTag::IO, ErrTag::Invalid], msg: errmsg!("A test 42") });
        let e2 = Error::Upstream(Arc::new(e1), ErrMsg { tags: &[ErrTag::IO, ErrTag::File],  msg: errmsg!() });
        let e3 = Error::Upstream(Arc::new(e2), ErrMsg { tags: &[ErrTag::Conversion], msg: errmsg!() });
        msg!("\n{:?}", e3);
        Ok(())
    }

    #[test]
    fn test_err_01() -> Outcome<()> {
        match u8::from_str("-1") {
            Err(e0) => {
                msg!("{}", e0);
                let e1 = err!(e0, errmsg!("A test"),
                    Decode, String, Invalid, Input);
                msg!("{}", e1);
                let e2 = err!(e1, errmsg!("Another level"), Bug);
                msg!("{}", e2);
            },
            Ok(_) => (),
        }

        Ok(())
    }
}

impl GenTag for ErrTag {}

impl fmt::Display for ErrTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Default for ErrTag {
    fn default() -> Self {
        Self::Unknown
    }
}
