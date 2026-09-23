// U10 owns this file: `import`, `include` and path resolution. Relative paths resolve against the
// importing file's directory, a leading `/` against `World::root`, `@ns/name:ver` through `package.rs`.
// A module is evaluated once and cached by canonical path; re-entering one on `World::route` is a cycle.

use crate::eval::content::Content;
use crate::eval::func::unimplemented;
use crate::eval::value::Module;
use crate::eval::Engine;
use crate::syntax::{
	FileId,
	Span,
};

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;
use std::sync::Arc;

/// The vfs path a spec written in `from` names.
pub fn resolve_path(_engine: &mut Engine, spec: &str, _from: FileId, _span: Span) -> Outcome<PathBuf> {
	Err(unimplemented("import", spec))
}

/// `import spec`: the evaluated module, from the cache when already evaluated.
pub fn import_module(_engine: &mut Engine, spec: &str, _from: FileId, _span: Span) -> Outcome<Arc<Module>> {
	Err(unimplemented("import", spec))
}

/// `include spec`: the module's content.
pub fn include(_engine: &mut Engine, spec: &str, _from: FileId, _span: Span) -> Outcome<Content> {
	Err(unimplemented("include", spec))
}
