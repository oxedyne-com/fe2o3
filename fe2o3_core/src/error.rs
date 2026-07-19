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

use crate::term::Term;

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
    Permanent, // A failure that retrying will not cure; the operation must not be attempted again.
    Poisoned,
    Range,
    Read,
    Sealed, // A key-protected resource is locked; it must be unsealed first.
    Security,
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

/// A message with the source location the `errmsg!` macro puts in front of it taken off again.
///
/// The macro writes `file.rs:123: the words`, or `file.rs:123` alone where it was given no words, so
/// what is left after the location is what somebody meant to say. A message that begins with no
/// location is returned whole, since it was written by something else and is not ours to trim.
fn without_location(msg: &str) -> &str {
    match msg.split_once(": ") {
        Some((head, tail)) if is_location(head) => tail,
        // The whole message may be a location and nothing else, which is what a frame that merely
        // wraps another error looks like. It has nothing to say, so it says nothing.
        _ => if is_location(msg) { "" } else { msg },
    }
}

/// Whether a string is a `file:line` of the shape `errmsg!` writes.
///
/// The test is that what follows the last colon is a line number. A message of somebody's own that
/// happened to hold a colon has words after it, not digits, and is left alone.
fn is_location(s: &str) -> bool {
    match s.rsplit_once(':') {
        Some((file, line)) => {
            !file.is_empty()
                && !line.is_empty()
                && line.chars().all(|c| c.is_ascii_digit())
        },
        None => false,
    }
}

/// Adds a message to the list, unless it has nothing in it.
fn push_words(out: &mut Vec<String>, msg: &str) {
    let msg = msg.trim();
    if !msg.is_empty() {
        out.push(msg.to_string());
    }
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

    /// The words the error carries, outermost first, with nothing in them but words.
    ///
    /// The `err!` macro writes the file and the line in front of every message it is given, and an
    /// error that wraps another keeps a frame for each. That is what a developer wants and it is not
    /// what anybody else wants, so this gives the messages alone: no source locations, no frame
    /// names, no tags, and nothing from a frame that carried no words of its own.
    pub fn msgs(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.gather(&mut out);
        out
    }

    /// Walks the error and its causes, appending the words each carries.
    fn gather(&self, out: &mut Vec<String>) {
        match self {
            Error::Local(ErrMsg { msg: m, .. }) |
            Error::Other(ErrMsg { msg: m, .. }) => push_words(out, without_location(m)),
            Error::Upstream(arc_e, ErrMsg { msg: m, .. }) => {
                push_words(out, without_location(m));
                match arc_e.downcast_ref::<Error<T>>() {
                    // One of ours, so its words are reachable and are gathered in turn.
                    Some(e) => e.gather(out),
                    // Somebody else's, so all we have is what it says of itself. `Display` is the
                    // right form here: a foreign error has no ANSI colour of ours in it.
                    None => push_words(out, &fmt!("{}", arc_e)),
                }
            },
            Error::Collection(boxerrs) => {
                for boxerr in boxerrs {
                    boxerr.gather(out);
                }
            },
        }
    }

    /// The error as a person should read it: what went wrong, in the words the code chose.
    ///
    /// `Debug` is the developer's form and names the file and the line of every frame; `Display` is
    /// the console's and carries ANSI colour. Neither belongs in a browser, a log field, or anything
    /// a user will read, and this is the form for those. The outermost words come first, since they
    /// are the context, and the innermost last, since they are the detail.
    pub fn plain(&self) -> String {
        let msgs = self.msgs();
        if msgs.is_empty() {
            // An error with no words at all still has its tags, and they beat saying nothing.
            let tags = Self::tags_display(self.tags());
            if tags.is_empty() {
                return "An error carrying neither a message nor a tag.".to_string();
            }
            return tags;
        }
        msgs.join(" ")
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

    /// The upstream error is one of ours, so it is printed as one: with `Debug`, which is plain.
    ///
    /// Printing it with `Display` would recurse into the console form, and every frame below the
    /// first would carry ANSI colour codes. That is invisible in a terminal, where they are what is
    /// wanted, and it is why it went unnoticed. Anywhere else -- a browser, a log file, a JSON
    /// field -- the escapes are rubbish in the middle of the message.
    fn fmt_debug_upstream_specific(
        f: &mut fmt::Formatter<'_>,
        e: &Self,
        m: &str,
        t: &'static [T],
    )
        -> fmt::Result
    {
        write!(f, "UpstreamErr{{{}{}}}\n{:?}",
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
                ok!(writeln!(f, "Collection of {} errors:", boxerrs.len()));
                for (i, boxerr) in boxerrs.iter().enumerate() {
                    ok!(writeln!(f, "{:04}: {:?}", i, *boxerr));
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
                ok!(writeln!(f, "Collection of {} errors:", boxerrs.len()));
                for (i, boxerr) in boxerrs.iter().enumerate() {
                    ok!(writeln!(f, "{:04}: {}", i, *boxerr));
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
        ok!(write(&mut output, format_args!("Hello {}!", "world")));
        Ok(42)
    }

    /// The `Debug` form must be plain to the bottom of the chain, not merely at the top.
    ///
    /// `Display` is for a console and colours itself with ANSI escapes. `Debug` is what a caller
    /// reaches for when the message is going somewhere that is not a console -- a browser, a log
    /// file, a JSON field -- and an escape code in the middle of it is rubbish. A nested error is
    /// still one of ours, so it must be printed as one.
    #[test]
    fn test_debug_carries_no_terminal_escapes() -> Outcome<()> {
        // Three deep: a local error, wrapped, wrapped again.
        fn innermost() -> Outcome<()> {
            Err(err!("The tree region does not hash to what was signed."; Invalid, Input))
        }
        fn middle() -> Outcome<()> {
            res!(innermost());
            Ok(())
        }
        fn outer() -> Outcome<()> {
            res!(middle());
            Ok(())
        }
        let e = match outer() {
            Ok(()) => return Err(err!("The error was supposed to propagate."; Bug)),
            Err(e) => e,
        };

        let debug = fmt!("{:?}", e);
        assert!(
            !debug.contains('\u{1b}'),
            "The Debug form carries an ANSI escape, so it is not plain: {:?}", debug,
        );
        assert!(
            debug.contains("does not hash to what was signed"),
            "and the innermost message must still be in it: {}", debug,
        );

        // The Display form is for a console, and may colour itself all it likes.
        Ok(())
    }

    /// `plain` is what a person reads, so it must hold the words and nothing else.
    ///
    /// `Debug` names the file and the line of every frame, which is right for a developer and wrong
    /// for a browser: a reader told a document was refused wants to know why, not which line of
    /// which file noticed. The words are all that crosses.
    #[test]
    fn test_plain_is_words_and_nothing_else() -> Outcome<()> {
        // The shape of a real rejection: a detailed innermost error, wrapped by frames that add
        // context, and wrapped again by one that adds none.
        fn innermost() -> Outcome<()> {
            Err(err!(
                "The 1895 byte tree region hashes to 1507362a, but the envelope declares \
                387a4f57."; Invalid, Input, Mismatch))
        }
        fn middle() -> Outcome<()> {
            match innermost() {
                Ok(()) => Ok(()),
                Err(e) => Err(err!(e, "The document could not be read."; Invalid)),
            }
        }
        fn outer() -> Outcome<()> {
            res!(middle());	// Adds a frame carrying no words of its own.
            Ok(())
        }
        let e = match outer() {
            Ok(()) => return Err(err!("The error was supposed to propagate."; Bug)),
            Err(e) => e,
        };

        let plain = e.plain();

        // What a developer needs, and a reader does not.
        assert!(!plain.contains(".rs:"), "plain names a source file: {}", plain);
        assert!(!plain.contains("LocalErr"), "plain names a frame: {}", plain);
        assert!(!plain.contains("UpstreamErr"), "plain names a frame: {}", plain);
        assert!(!plain.contains('['), "plain carries its tags: {}", plain);
        assert!(!plain.contains('\u{1b}'), "plain carries an ANSI escape: {}", plain);

        // What a reader needs, and every word of it.
        assert!(
            plain.contains("The document could not be read."),
            "the context is missing: {}", plain,
        );
        assert!(
            plain.contains("hashes to 1507362a, but the envelope declares 387a4f57."),
            "the detail is missing: {}", plain,
        );
        // Context first, detail after: the reader is told what failed before being told how.
        let ctx = plain.find("could not be read");
        let det = plain.find("hashes to");
        assert!(ctx < det, "the detail came before the context: {}", plain);

        // The frame that carried no words of its own contributed none.
        assert_eq!(e.msgs().len(), 2, "a wordless frame put something in: {:?}", e.msgs());
        Ok(())
    }

    #[test]
    fn test_a_location_is_told_from_a_message_that_merely_holds_a_colon() {
        assert_eq!(without_location("src/doc.rs:95: The tree is short."), "The tree is short.");
        assert_eq!(without_location("src/doc.rs:95"), "");
        // A message of somebody else's, which owes the macro nothing.
        assert_eq!(without_location("error: the file is missing"), "error: the file is missing");
        assert_eq!(without_location("Note: 3 of 4 failed"), "Note: 3 of 4 failed");
        assert_eq!(without_location("no colon here"), "no colon here");
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
        let e0 = err!(fmt!("A test {}", 42); String, Invalid);
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
                let e1 = err!(e0, errmsg!("A test");
                    Decode, String, Invalid, Input);
                msg!("{}", e1);
                let e2 = err!(e1, errmsg!("Another level"); Bug);
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
