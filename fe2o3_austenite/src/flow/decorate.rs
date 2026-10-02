// U9 owns this file: page furniture. Header, footer, background and foreground are evaluated once per
// page per pass, after layout, with `here()` at the page; they sit outside the body flow, so this never
// feeds back into pagination. The decoration now in `doc.rs` moves here at U9.

use crate::eval::func::unimplemented;
use crate::eval::value::Value;
use crate::eval::Engine;
use crate::flow::RunSetup;
use crate::page::Page;

use oxedyne_fe2o3_core::prelude::*;

/// Decorates a placed page. A page with no header, footer, background, foreground or page numbering has no
/// furniture and stands as placed; one with any is not implemented yet, which fails the compile rather than
/// leaving the furniture out.
pub fn decorate_page(_engine: &mut Engine, _page: &mut Page, setup: &RunSetup) -> Outcome<()> {
	let furnished = setup.header.is_some() || setup.footer.is_some() || setup.background.is_some()
		|| setup.foreground.is_some() || !matches!(setup.numbering, Value::None);
	if furnished {
		return Err(unimplemented("flow", "decorate (page furniture)"));
	}
	Ok(())
}
