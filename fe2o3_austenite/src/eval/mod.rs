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

use crate::diag::{
	Diagnostic,
	DiagnosticKind,
};
use crate::eval::intro::{
	Introspector,
	ReadLog,
};
use crate::eval::locate::{
	Location,
	Locator,
	Place,
};
use crate::eval::styles::StyleChain;
use crate::eval::value::Module;
use crate::flow::text::FontStore;
use crate::syntax::{
	FileId,
	Source,
	Span,
};
use crate::timings::{
	Phase,
	Timings,
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
		let text = match vfs::read_to_string(path) {
			Ok(t)	=> t,
			// A file that is there but is not text is told from one that cannot be read, since the remedy differs.
			Err(e) if vfs::is_not_utf8(&e)	=> return Err(err!(
				"The source {} is not valid UTF-8 text.", path.display(); IO, File, Decode, UTF8)),
			Err(e)	=> return Err(err!("Could not read source {}: {}", path.display(), e; IO, File, Read)),
		};
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
	pub body:		Option<bool>,		// whether the pass's realised body set content; none until it is realised
	pub timings:	Option<Timings>,	// the per-phase clock of `--timings`; none costs a test of the option
}

/// The kind of an error a native function raised with `err!` and no diagnostic of its own: the kind its
/// tags give, with an input fault, a bad argument or value, read as a type error.
pub(crate) fn native_kind(e: &Error<ErrTag>) -> DiagnosticKind {
	match DiagnosticKind::from_error_tags(e) {
		DiagnosticKind::Internal if e.tags().contains(&ErrTag::Input)	=> DiagnosticKind::Type,
		kind															=> kind,
	}
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
			body:		None,
			timings:	None,
		}
	}

	/// Runs `f` as phase `p` of the timing record. With no recorder it is a call of `f`.
	pub fn timed<R, F: FnOnce(&mut Engine) -> R>(&mut self, p: Phase, f: F) -> R {
		if let Some(t) = self.timings.as_mut() {
			t.enter(p);
		}
		let out = f(self);
		if let Some(t) = self.timings.as_mut() {
			t.leave();
		}
		out
	}

	/// Records an error diagnostic at `span` and returns the error to propagate.
	pub fn error<S: Into<String>>(&mut self, kind: DiagnosticKind, span: Span, message: S) -> Error<ErrTag> {
		let message = message.into();
		let e = err!("{}", message; Input, Invalid);
		self.diags.push(Diagnostic::error(kind, span, message));
		e
	}

	/// As [`error`](Self::error), with a hint line.
	pub fn error_hint<S: Into<String>, H: Into<String>>(
		&mut self,
		kind:		DiagnosticKind,
		span:		Span,
		message:	S,
		hint:		H,
	)
		-> Error<ErrTag>
	{
		let message = message.into();
		let e = err!("{}", message; Input, Invalid);
		self.diags.push(Diagnostic::error(kind, span, message).with_hint(hint));
		e
	}

	/// Records a warning of `kind`. A warning of a kind that [`refuses_strict`](DiagnosticKind::refuses_strict)
	/// fails a strict compile, so one for a construct passed over or set otherwise than Typst sets it is
	/// `Unsupported`.
	///
	/// A warning identical to one already recorded is dropped, as Typst's sink drops it, so a set rule met
	/// again, or a paragraph laid out in every pass, warns once.
	pub fn warn<S: Into<String>>(&mut self, kind: DiagnosticKind, span: Span, message: S) {
		let d = Diagnostic::warning(kind, span, message);
		if !self.diags.contains(&d) {
			self.diags.push(d);
		}
	}

	/// Typst's `Engine::delay`: the failure of a show rule is recorded and the element it was showing shows as
	/// nothing, so the compile goes on. The error stays among the diagnostics, and fails the compile only if it is
	/// still there after the final pass: an earlier pass may fail for want of a label or a counter that a later
	/// one has, and the fixpoint drops each pass's diagnostics before the next. `mark` is the length of
	/// `diags` before the show ran, so an error raised without a diagnostic of its own is given one at `span`.
	pub fn delay<T: Default>(&mut self, mark: usize, span: Span, result: Outcome<T>) -> T {
		match result {
			Ok(v)	=> v,
			Err(e)	=> {
				self.adopt(mark, span, e);
				T::default()
			},
		}
	}

	/// Enters a closure call; past [`MAX_CALL_DEPTH`] it is an error, not a stack overflow.
	pub fn enter_call(&mut self, span: Span) -> Outcome<()> {
		if self.depth >= MAX_CALL_DEPTH {
			return Err(self.error(DiagnosticKind::Limit, span, "maximum function call depth exceeded"));
		}
		self.depth += 1;
		Ok(())
	}

	pub fn exit_call(&mut self) { self.depth = self.depth.saturating_sub(1); }

	/// Runs `f` with realisations counted afresh at `place`, Typst's `Locator::relayout`: content laid out
	/// again at the place it was laid out at is realised to the same locations, whatever was realised in
	/// between. `None` leaves the realisation where it stands. The locator in force comes back when `f` ends,
	/// by whichever route.
	pub fn within<R, F: FnOnce(&mut Engine) -> R>(&mut self, place: Option<Place>, f: F) -> R {
		match place {
			None		=> f(self),
			Some(p)		=> {
				let outer = std::mem::replace(&mut self.locator, Locator::new(p));
				let out = f(self);
				self.locator = outer;
				out
			},
		}
	}

	/// Spends one loop iteration; an exhausted budget is a diagnostic, so a runaway loop terminates.
	pub fn burn(&mut self, span: Span) -> Outcome<()> {
		if self.fuel == 0 {
			return Err(self.error(DiagnosticKind::Limit, span, "loop seems to be infinite"));
		}
		self.fuel -= 1;
		Ok(())
	}

	pub fn has_errors(&self) -> bool { self.diags.iter().any(|d| d.is_error()) }
}

/// Evaluates a loaded source into a module. The body is U2's [`eval::eval_module`]. The source goes on
/// the import route while it runs, so a file it imports that imports it back is a cycle, as in Typst.
pub fn eval_source(engine: &mut Engine, id: FileId) -> Outcome<Module> {
	let path = match engine.world.source(id) {
		Some(s)	=> s.path.clone(),
		None	=> return Err(err!("No source with id {} is loaded.", id.0; Missing, Input)),
	};
	engine.world.route.push(path);
	let out = eval::eval_module(engine, id);
	engine.world.route.pop();
	out
}
