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
//!
//! A syntax is built from configuration structs, a message is parsed against
//! it, and what the message said is read by matching rather than by dispatch.
//!
//! ```ignore
//! use oxedyne_fe2o3_syntax::{
//!     cmd::{Cmd, CmdConfig},
//!     core::{Syntax, SyntaxConfig, SyntaxRef},
//!     msg::Msg,
//!     val::Val,
//! };
//! use oxedyne_fe2o3_jdat::prelude::*;
//! use oxedyne_fe2o3_core::prelude::*;
//!
//! // A syntax, with one command that takes one string value.
//! let mut syntax = Syntax::from(SyntaxConfig {
//!     name:   fmt!("example"),
//!     ..Default::default()
//! });
//! let cmd = Cmd::from(CmdConfig {
//!     name:   fmt!("connect"),
//!     vals:   vec![Val::text("host").help("Host to connect to")],
//!     help:   Some(fmt!("Connect to a remote host")),
//!     ..Default::default()
//! });
//! syntax = res!(syntax.add_cmd(cmd));
//! let syntax = SyntaxRef::new(syntax);
//!
//! // A message read against it.
//! let msg = res!(Msg::new(syntax).from_str("connect example.com", None));
//! match msg.get_cmd("connect") {
//!     Some(cmd) => match cmd.get_vals() {
//!         Some(_vals) => (), // Handle the connect command.
//!         None => (),        // It named no host.
//!     },
//!     None => (),            // The message said something else.
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
//! hyphens; either hyphenated form may be absent.  An argument without a value is an option (or
//! "switch").  Values from a REPL line or the wire are decoded as `Daticles` which protect single
//! and double quotes by default, and allow type specification, e.g. `(i16|-42)`.  Arguments are
//! optional unless specified otherwise.  Because values are daticles, you can use compound
//! daticles like `Kind::MAP` and `Kind::LIST` to embed a variable number of values.
//!
//! # Process command lines
//!
//! A syntax can also read a process's own arguments through [`argv::parse`], which answers
//! `help`, `--help` and `--version` itself.  Set `SyntaxConfig::one_cmd` so that the command line
//! names exactly one command and its values may follow its options.  There, a word the shell has
//! already unquoted is taken as it stands when a string is expected, a value may be optional or
//! repeated ([`val::Arity`]), and a command may take the words after `--` as its `rest`.
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
//!             .expected_vals(vec![(Kind::Str, "Directory path")])
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
//!     hyph1:  Some(fmt!("p")),
//!     hyph2:  Some(fmt!("path")),
//!     reqd:   true,
//!     vals:   vec![Val::text("dir")],
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
pub mod argv;
pub mod cmd;
pub mod core;
pub mod help;
pub mod key;
pub mod msg;
pub mod opt;
pub mod val;

pub use core::{
    Syntax,
    SyntaxRef,
};
