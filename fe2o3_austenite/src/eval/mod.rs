//! Contract (U0, 2026-09-23). Every evaluator type other units name is declared by U0 and keeps its
//! name and public shape; the unit that owns a file may add fields, variants and methods but not rename
//! or remove. Three facts a reader cannot derive from the types:
//!
//! * Native functions are a two-level enum, not trait objects: [`func::NativeFunc`] has one variant per
//!   library area, and each area's inner enum and `call` live in that area's own file, so adding a
//!   library function never touches a shared file.
//! * An element's fields are a schema (`&'static [FieldSpec]`) owned by the unit that lays the element
//!   out; `content.rs` dispatches `ElemKind` to that unit's `fields`, `construct` and `show` by
//!   [`content::Family`].
//! * An evaluation error is reported with [`Engine::error`], which records a spanned
//!   [`Diagnostic`](crate::diag::Diagnostic) and returns the `Error` to propagate with `res!`.

// Declares an area's inner native-function enum with each variant's Typst-facing name. Defined before the
// module list so every submodule sees it.
macro_rules! native_fns {
	($(#[$m:meta])* pub enum $name:ident { $($v:ident => $s:literal,)* }) => {
		$(#[$m])*
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
		pub enum $name {
			$($v,)*
		}

		impl $name {
			pub const ALL: &'static [$name] = &[$($name::$v,)*];

			/// The name Typst code calls it by, within its area (`pow` for `calc.pow`).
			pub fn name(&self) -> &'static str {
				match *self {
					$($name::$v => $s,)*
				}
			}
		}
	};
}

pub mod args;
pub mod content;
#[allow(clippy::module_inception)]
pub mod eval;
pub mod fixpoint;
pub mod func;
pub mod import;
pub mod intro;
pub mod lib;
pub mod locate;
pub mod methods;
pub mod ops;
pub mod package;
pub mod realise;
pub mod scope;
pub mod select;
pub mod styles;
pub mod value;

use crate::diag::Diagnostic;
use crate::eval::intro::{
	Introspector,
	ReadLog,
};
use crate::eval::locate::{
	Location,
	Locator,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::Module;
use crate::flow::text::FontStore;
use crate::syntax::{
	FileId,
	Source,
	Span,
};
use crate::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::{
	Path,
	PathBuf,
};
use std::sync::Arc;

pub const MAX_CALL_DEPTH:	usize	= 80;			// Typst's own limit
pub const LOOP_FUEL:		u64		= 10_000_000;	// iterations across all loops before a diagnostic

/// The sources of one compilation and the modules evaluated from them. Loading a path is here;
/// resolving an import spec to a path, caching and cycle detection are `import.rs` (U10).
#[derive(Clone, Debug, Default)]
pub struct World {
	pub root:		PathBuf,						// the project root a leading `/` resolves against
	pub sources:	Vec<Source>,					// indexed by `FileId.0`
	pub modules:	HashMap<PathBuf, Arc<Module>>,	// evaluated once, by canonical path
	pub route:		Vec<PathBuf>,					// modules being evaluated, outermost first, for cycles
}

impl World {
	pub fn new(root: PathBuf) -> Self {
		Self { root, ..Self::default() }
	}

	/// Parses `text` as a new source and returns its id; a path already loaded returns its existing id.
	pub fn add_source(&mut self, path: PathBuf, text: String) -> Outcome<FileId> {
		if let Some(s) = self.sources.iter().find(|s| s.path == path) {
			return Ok(s.id);
		}
		if self.sources.len() >= FileId::DETACHED.0 as usize {
			return Err(err!("Too many source files: {} is the limit.", FileId::DETACHED.0; Excessive));
		}
		let id = FileId(self.sources.len() as u16);
		self.sources.push(Source::new(id, path, text));
		Ok(id)
	}

	/// Reads a path through the vfs (native disc or the wasm source map) and parses it.
	pub fn load(&mut self, path: &Path) -> Outcome<FileId> {
		if let Some(s) = self.sources.iter().find(|s| s.path == path) {
			return Ok(s.id);
		}
		let text = res!(vfs::read_to_string(path).map_err(|e| err!(
			"Could not read source {}: {}", path.display(), e; IO, File, Read)));
		self.add_source(path.to_path_buf(), text)
	}

	pub fn source(&self, id: FileId) -> Option<&Source> { self.sources.get(id.0 as usize) }
}

/// What `here()` and a contextual `text.size` read: set while a `context` body or a show-rule closure
/// runs, empty elsewhere (reading it then is Typst's "can only be used when context is known").
#[derive(Clone, Debug, Default)]
pub struct Context {
	pub location:	Option<Location>,
	pub styles:		Option<StyleChain>,
}

/// Everything evaluation, realisation and flow share for one compilation. Passed as `&mut Engine`;
/// the introspector is replaced between fixpoint passes, everything else persists.
#[derive(Debug)]
pub struct Engine {
	pub world:		World,
	pub intro:		Arc<Introspector>,	// the previous pass's layout facts (empty on pass one)
	pub locator:	Locator,			// hands out stable element locations during realisation
	pub diags:		Vec<Diagnostic>,
	pub depth:		usize,				// current closure call depth
	pub fuel:		u64,				// loop iterations remaining
	pub context:	Context,
	pub reads:		ReadLog,			// introspection reads recorded for the fixpoint (U8)
	pub fonts:		FontStore,			// faces for shaping and `measure` (U6a)
}

impl Engine {
	pub fn new(world: World) -> Self {
		Self {
			world,
			intro:		Arc::new(Introspector::default()),
			locator:	Locator::default(),
			diags:		Vec::new(),
			depth:		0,
			fuel:		LOOP_FUEL,
			context:	Context::default(),
			reads:		ReadLog::default(),
			fonts:		FontStore::default(),
		}
	}

	/// Records an error diagnostic at `span` and returns the error to propagate.
	pub fn error<S: Into<String>>(&mut self, span: Span, message: S) -> Error<ErrTag> {
		let message = message.into();
		let e = err!("{}", message; Input, Invalid);
		self.diags.push(Diagnostic::error(span, message));
		e
	}

	/// As [`error`](Self::error), with a hint line.
	pub fn error_hint<S: Into<String>, H: Into<String>>(&mut self, span: Span, message: S, hint: H) -> Error<ErrTag> {
		let message = message.into();
		let e = err!("{}", message; Input, Invalid);
		self.diags.push(Diagnostic::error(span, message).with_hint(hint));
		e
	}

	pub fn warn<S: Into<String>>(&mut self, span: Span, message: S) {
		self.diags.push(Diagnostic::warning(span, message));
	}

	/// Enters a closure call; past [`MAX_CALL_DEPTH`] it is an error, not a stack overflow.
	pub fn enter_call(&mut self, span: Span) -> Outcome<()> {
		if self.depth >= MAX_CALL_DEPTH {
			return Err(self.error(span, "maximum function call depth exceeded"));
		}
		self.depth += 1;
		Ok(())
	}

	pub fn exit_call(&mut self) { self.depth = self.depth.saturating_sub(1); }

	/// Spends one loop iteration; an exhausted budget is a diagnostic, so a runaway loop terminates.
	pub fn burn(&mut self, span: Span) -> Outcome<()> {
		if self.fuel == 0 {
			return Err(self.error(span, "loop seems to be infinite"));
		}
		self.fuel -= 1;
		Ok(())
	}

	pub fn has_errors(&self) -> bool { self.diags.iter().any(|d| d.is_error()) }
}

/// Evaluates a loaded source into a module. The body is U2's [`eval::eval_module`].
pub fn eval_source(engine: &mut Engine, id: FileId) -> Outcome<Module> {
	eval::eval_module(engine, id)
}
