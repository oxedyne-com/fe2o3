//! A library implementing Jason's Data And Type (JDAT) scheme, a typed superset of JSON providing
//! both text and binary serialisation.
//!
//! JDAT extends JSON by adding:
//! - Optional type annotations with a rich set of built-in types
//! - Support for arbitrary-precision numbers and compact binary encoding
//! - User-defined type extensions through a flexible system
//! - Comments and trailing commas for improved readability
//! - Any type as map keys, not just strings
//!
//! The library provides:
//! - Complete text and binary serialisation/deserialisation
//! - Derive macros for automatic implementation of conversion traits
//! - Full compatibility with existing JSON data
//! - Optimised binary encoding for network transfers
//! - Support for streaming and incremental parsing
//!
//! # Examples
//!
//! The following example demonstrates core concepts including daticles (type-annotated values like 
//! `(u8|42)`), kindicles (type annotations like `u8|`), and the different text encodings available:
//!
//! ```rust
//! use oxedize_fe2o3_jdat::{
//!     prelude::*,
//!     string::{
//!         dec::DecoderConfig,
//!         enc::EncoderConfig,
//!     },
//! };
//! use oxedize_fe2o3_core::prelude::*;
//! use std::collections::BTreeMap;
//!
//! fn main() -> Outcome<()> {
//!     // Create a sample data structure
//!     let data = mapdat!{
//!         "name" => "Alice",
//!         "age" => 21u8,
//!         "scores" => listdat![95u8, 87u8, 92u8],
//!         dat!(42u8) => "Answer",  // Non-string key, not possible in JSON
//!     };
//!
//!     // Standard JSON format (no type annotations)
//!     let json_cfg = EncoderConfig::<(), ()>::json(None);
//!     println!("JSON format:");
//!     println!("{}", res!(data.encode_string_with_config(&json_cfg)));
//!     // Output:
//!     // {
//!     //     "name": "Alice",
//!     //     "age": 21,
//!     //     "scores": [95, 87, 92],
//!     //     "42": "Answer"
//!     // }
//!
//!     // Display format (most common types inferred)
//!     println!("\nDisplay format (KindScope::Most):");
//!     println!("{}", data);
//!     // Output:
//!     // {
//!     //     "name": "Alice",
//!     //     "age": (u8|21),
//!     //     "scores": [95, 87, 92],
//!     //     (u8|42): "Answer"
//!     // }
//!
//!     // Debug format (all types shown)
//!     println!("\nDebug format (KindScope::Everything):");
//!     println!("{:?}", data);
//!     // Output:
//!     // (map|{
//!     //     (str|"name"): (str|"Alice"),
//!     //     (str|"age"): (u8|21),
//!     //     (str|"scores"): (list|[(u8|95), (u8|87), (u8|92)]),
//!     //     (u8|42): (str|"Answer")
//!     // })
//!
//!     // Demonstrate manual daticle creation and binary conversion
//!     let d1 = Dat::U8(42);                // Manual construction
//!     let d2 = dat!(42);                   // Macro construction (sized to u8)
//!     let k = d1.kind();                   // Get the kind (Kind::U8)
//!     
//!     // Convert to bytes
//!     let mut buf = Vec::new();
//!     buf = res!(d1.to_bytes(buf));
//!     
//!     // Parse a daticle from text
//!     let d3 = res!(Dat::decode_string("(i8|-42)"));
//!     
//!     // Parse a recursive daticle
//!     let d4 = res!(Dat::decode_string("(map|{ \"age\": (u8|21)})"));
//!
//!     Ok(())
//! }
//! ```
//!
//! The key innovation in Jdat is the daticle format, which consists of:
//! - An optional kindicle (type annotation) in parentheses, e.g. `(u8|`
//! - A value that matches the type, e.g. `42)`
//! - Together forming `(u8|42)`
//!
//! Types can be inferred where unambiguous, and kindicles can be omitted for common types like
//! strings and maps in most contexts. The level of type annotation is controlled by the
//! `KindScope` setting, allowing for formats ranging from JSON-compatible to fully typed.
//!
#![forbid(unsafe_code)]

#[macro_use]
pub mod macros;

pub mod binary;
pub mod cfg;
pub mod chunk;
pub mod constant;
pub mod conv;
pub mod daticle;
pub mod file;
pub mod id;
pub mod int;
pub mod kind;
pub mod map;
pub mod note;
pub mod prelude;
pub mod string;
pub mod usr;
pub mod version;

use oxedize_fe2o3_core::prelude::*;

pub use oxedize_fe2o3_core::conv::BestFrom;

pub use dat_map::{
    FromDatMap,
    ToDatMap,
};

pub use crate::{
    daticle::Dat,
    kind::Kind,
};
