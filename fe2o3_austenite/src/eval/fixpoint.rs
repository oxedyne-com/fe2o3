// U8 owns this file: the outer fixpoint. Evaluate once; then up to `MAX_PASSES` times realise against the
// previous introspector, flow, run the driver, and rebuild the introspector, stopping when no recorded
// context read changed. A fifth pass still moving is a warning and its output stands, as in Typst.

use crate::eval::func::unimplemented;
use crate::eval::intro::Introspector;
use crate::eval::value::Module;
use crate::eval::Engine;
use crate::flow::PageRun;
use crate::ledger::Ledger;
use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

pub const MAX_PASSES: u32 = 5;

/// The laid-out document: what `compile::assemble` (U9) hands to decoration and the emitters.
#[derive(Clone, Debug)]
pub struct Laid {
	pub pages:		Vec<Page>,
	pub runs:		Vec<PageRun>,	// per-run geometry and furniture, for decoration
	pub ledger:		Ledger,
	pub intro:		Arc<Introspector>,
	pub passes:		u32,
	pub converged:	bool,
}

pub fn run(_engine: &mut Engine, _module: &Module) -> Outcome<Laid> {
	Err(unimplemented("fixpoint", "run"))
}
