//! A protocol-oriented syntax system for unified command handling across REPL and network interfaces.
//! 
//! This crate provides tools for defining and processing commands in a structured way, whether they 
//! originate from a text-based REPL or network messages. The core abstraction is the [`Syntax`] type,
//! which defines available commands, their arguments, and expected values.
//!
//! Rather than using callbacks or trait implementations, command handling is done through explicit 
//! pattern matching, giving developers direct control over the command processing flow. This approach 
//! favours simplicity and transparency over abstraction.
//!
//! # Example
//! ```rust
//! use oxedyne_fe2o3_syntax::{Syntax, SyntaxRef};
//! use oxedyne_fe2o3_jdat::kind::Kind;
//! 
//! // Create a new syntax
//! let mut syntax = Syntax::new("example")
//!     .expected_vals(vec![(Kind::Str, "Protocol name".to_string())]);
//!
//! // Add commands with arguments
//! let cmd = Cmd::new("connect")?
//!     .expected_vals(vec![(Kind::Str, "Host to connect to".to_string())])
//!     .help("Connect to a remote host");
//! 
//! syntax = syntax.add_cmd(cmd)?;
//!
//! // Process commands through direct matching
//! match msg.get_cmd("connect") {
//!     Some(cmd) => {
//!         if let Some(vals) = cmd.get_vals() {
//!             // Handle connect command...
//!         }
//!     }
//!     None => {
//!         // Command not found...
//!     }
//! }
//! ```
//! 
//! # Details
//!
//! A `Syntax` represents rules for communication in the Presentation Layer of the [OSI
//! Model](https://en.wikipedia.org/wiki/OSI_model).  This generalises to a command line interface.
//! Messages are composed of one or more pre-defined commands.  There can be a variable number of
//! arguments associated with the message and with each command.  There can be a fixed number of
//! values ([Daticle](oxedyne_fe2o3_jdat::daticle::Daticle) of pre-defined `Kind`) for the message
//! and for each argument and command.
//!
//! Valid examples:
//! ```ignore
//! {invoc} v                              | 1 message val (required)
//! {invoc} v a v                          | 1 message val followed by 1 message arg and val
//! {invoc} a v a v v v
//! {invoc} a v a a c v v a v v a c v a a  | multiple commands
//! ```
//!
//! where
//!
//! ```ignore
//! {invoc} = invocation command (e.g. the program pathname when using a shell)
//! c = command
//! a = argument
//! v = value
//! ```
//!
//! An argument comes in three possible versions, its prefixless name, or prefixed with one or two
//! hyphens.  An argument without a value is an option (or "switch").  Values are decoded as
//! `Daticles` which protect single and double quotes by default, and allow type specification,
//! e.g. `(i16|-42)`.  Arguments are optional unless specified otherwise.  Note that because values
//! are daticles, you can use compound daticles like `Kind::MAP` and `Kind::LIST` to embed a
//! variable number of values.  
//!
//! `Syntax` attempts to unify:
//! - command line text interfaces (CLI or TUI) including one-time invocation with argument
//!     passing, and interactive read-evaluate-print loops (REPLs), and
//! - over-the-wire (OTW) text and binary messages.
//! Multiple commands in a single message are permitted.  A session begins when a user logs in, and
//! session state is maintained via a mapping of `Daticle`s to `Daticle`s.
//!
//! The API facilitates the use of a Builder Pattern, e.g.
//! ```ignore
//! let syntax = res!(res!(res!(Syntax::new("repl")
//!     .with_default_help_cmd())
//!     .version("1")
//!     .about("Demonstration REPL")
//!     .add_cmd(
//!         res!(Cmd::new("pwd"))
//!         .help("Print path of current/working directory")
//!     ))
//!     .add_cmd(
//!         res!(res!(Cmd::new("cd"))
//!         .help("Change directory")
//!         .add_arg(res!(res!(res!(Arg::new("dir"))
//!             .hyph1("p"))
//!             .hyph2("path"))
//!             .required(true)
//!             .expected_vals(vec![Kind::Str])
//!             .help("Directory path")
//!         ))
//!     ));
//! ```
//! Since syntax definition is a once-off process, you may like to use `catch!` rather than `res!`
//! to catch a wide class of panics, but since the closure-based `catch!` doesn't nest well, the
//! definitions just need to be split up:
//! ```ignore
//! let mut p = Syntax::from(SyntaxConfig {
//!     name:   fmt!("repl"),
//!     ver:    Some(fmt!("1")),
//!     about:  Some(fmt!("Demonstration REPL")),
//!     ..Default::default()
//! });
//!
//! p = catch!(p.with_default_help_cmd());
//!
//! let mut c = Cmd::from(CmdConfig {
//!     name:   fmt!("cd"),
//!     help:   Some(fmt!("Change directory")),
//!     ..Default::default()
//! });
//! let a = Arg::from(ArgConfig {
//!     name:   fmt!("dir"),
//!     hyph1:  fmt!("p"),
//!     hyph2:  Some(fmt!("path")),
//!     reqd:   true,
//!     evals:  vec![Kind::Str],
//!     help:   Some(fmt!("Directory path")),
//!     ..Default::default()
//! });
//! c = catch!(c.add_arg(a));
//! p = catch!(p.add_cmd(c));
//!
//! let mut c = Cmd::from(CmdConfig {
//!     name:   fmt!("pwd"),
//!     help:   Some(fmt!("Print path of current/working directory")),
//!     ..Default::default()
//! });
//! p = catch!(p.add_cmd(c));
//! ```
//! The code defines `Arg` and `Cmd` which form parts of the static `Syntax`, while `Msg`
//! represents a message decoded using the syntax.  A message can contain values, arguments (with
//! possible values), and commands, the latter represented by `MsgCmd`, which itself can contain
//! values and arguments (with possibe values).
//!
#![forbid(unsafe_code)]
pub mod apps;
pub mod arg;
pub mod cmd;
pub mod core;
pub mod help;
pub mod key;
pub mod msg;
pub mod opt;

pub use core::{
    Syntax,
    SyntaxRef,
};
