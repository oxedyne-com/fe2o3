// U9 owns this file: page furniture. Header, footer, background and foreground are evaluated once per
// page per pass, after layout, with `here()` at the page; they sit outside the body flow, so this never
// feeds back into pagination. The decoration now in `doc.rs` moves here at U9.

use crate::eval::func::unimplemented;
use crate::eval::Engine;
use crate::flow::RunSetup;
use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;

pub fn decorate_page(_engine: &mut Engine, _page: &mut Page, _setup: &RunSetup) -> Outcome<()> {
	Err(unimplemented("flow", "decorate"))
}
