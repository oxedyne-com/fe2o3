//! Phase 3: a true native desktop window that opens a `.prl` and renders its pages with the CPU
//! rasteriser, no webview.
//!
//! The window is [`winit`]'s cross-platform surface (x11/wayland, macOS, Windows) and the page pixels are
//! blitted to it with [`softbuffer`]'s raw framebuffer -- so the very [`Pixmap`](crate::raster::RasterPage)
//! the PNG path draws is what reaches the screen, at pixel parity. Pages stack vertically and fit the
//! window width; the DPI each page rasters at is chosen from the window's physical width, so a HiDPI
//! display (winit reports its scale factor, and the physical inner size already carries it) rasters
//! sharper rather than larger. Navigation is wheel, PageUp/PageDown, arrows and Home/End; a resize
//! re-fits the width and re-rasters.
//!
//! A document that carries an outline gets a contents sidebar on the left, the native counterpart of the
//! web reader's contents rail: the heading tree from [`PearlDoc::outline`], each branch folded or
//! unfolded by its caret, a click on a title jumping to that heading, and the heading the view is in
//! marked. The toggle in the top-left corner, or the `t` key, shows and hides it. Titles are set in the
//! embedded Libertinus faces through the same shaper and outline filler the pages use.
//!
//! This module is behind the default-off `gui` feature, the sole gate on the winit and softbuffer
//! dependencies. House rule: the winit callback model cannot return an error, so a failure inside a
//! handler is stored on the app and the loop is asked to exit; [`open`] inspects it after the loop and
//! surfaces it as an [`Outcome`]. No `unwrap`, `?` or `unsafe` in this crate's own code.

use crate::contents::Contents;
use crate::raster::{
	self,
	RasterPage,
};

use oxedyne_fe2o3_austenite::emit::pearl::{
	OutlineEntry,
	PearlDoc,
};
use oxedyne_fe2o3_austenite::fonts;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_font::{
	face::Role,
	font::Font,
	set::FontSet,
	shape::Dir,
};
use oxedyne_fe2o3_graphics::colour::Rgba;
use oxedyne_fe2o3_graphics::path::{
	Bounds,
	Path,
	PathBuilder,
	Pt,
};
use oxedyne_fe2o3_graphics::pixmap::Pixmap;
use oxedyne_fe2o3_graphics::transform::Transform;

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::application::ApplicationHandler;
use winit::event::{
	ElementState,
	KeyEvent,
	MouseButton,
	MouseScrollDelta,
	WindowEvent,
};
use winit::event_loop::{
	ActiveEventLoop,
	ControlFlow,
	EventLoop,
};
use winit::keyboard::{
	Key,
	NamedKey,
};
use winit::window::{
	Window,
	WindowId,
};

// A step for an arrow key and a wheel line, in logical-ish physical pixels; a page step is most of the
// viewport height, keeping a sliver of overlap for continuity.
const LINE_STEP:	f64 = 64.0;
const PAGE_FRACTION:	f64 = 0.9;
// The grey gutter between and around pages, in device pixels at 1x; it rides the scale factor.
const GUTTER:		f64 = 16.0;
// The window's background and page gutter, as a 0x00RRGGBB softbuffer word.
const GUTTER_RGB:	u32 = 0x00303338;
const GUTTER_INK:	Rgba = Rgba::opaque(0x30, 0x33, 0x38);	// the same, for painting into a pixmap

// Contents sidebar geometry, in logical pixels; each rides the scale factor.
const SIDE_W:		f64 = 280.0;	// open width, capped at SIDE_MAX of the window
const SIDE_MAX:		f64 = 0.6;
const HEAD_H:		f64 = 44.0;		// header band holding the toggle and the label
const ROW_H:		f64 = 26.0;
const INDENT:		f64 = 14.0;		// per nesting level
const CARET_W:		f64 = 16.0;
const PAD:			f64 = 8.0;
const BTN:			f64 = 28.0;		// the toggle, square, at (PAD, PAD) whether the sidebar is open or not
const NUM_GAP:		f64 = 6.0;		// between a heading's number and its title
const JUMP_CLEAR:	f64 = 24.0;		// clearance left above a heading jumped to

// Contents sidebar palette, after the web reader's rail.
const SIDE_BG:		Rgba = Rgba::opaque(0xfa, 0xfa, 0xfa);
const SIDE_RULE:	Rgba = Rgba::opaque(0xe5, 0xe7, 0xeb);
const TEXT_MAIN:	Rgba = Rgba::opaque(0x11, 0x18, 0x27);
const TEXT_DEEP:	Rgba = Rgba::opaque(0x37, 0x41, 0x51);	// level 3 and below
const TEXT_NUM:		Rgba = Rgba::opaque(0x9c, 0xa3, 0xaf);
const TEXT_HEAD:	Rgba = Rgba::opaque(0x6b, 0x72, 0x80);
const TEXT_DEAD:	Rgba = Rgba::opaque(0xb9, 0x1c, 0x1c);	// a heading the ledger never placed
const CUR_BG:		Rgba = Rgba::opaque(0xdb, 0xea, 0xfe);
const CUR_TEXT:		Rgba = Rgba::opaque(0x1d, 0x4e, 0xd8);
const CARET_INK:	Rgba = Rgba::opaque(0x9c, 0xa3, 0xaf);
const BTN_ON:		Rgba = Rgba::opaque(0x25, 0x63, 0xeb);
const BTN_OFF:		Rgba = Rgba::opaque(0xff, 0xff, 0xff);
const BTN_EDGE:		Rgba = Rgba::opaque(0xd1, 0xd5, 0xdb);
const BTN_BARS:		Rgba = Rgba::opaque(0x37, 0x41, 0x51);

/// Opens `doc` in a native window titled `title`, returning when the window closes. Blocks the calling
/// thread for the lifetime of the window, as a desktop reader's main loop does.
pub fn open(doc: PearlDoc, title: String) -> Outcome<()> {
	let page_count = res!(doc.page_count());
	let mut sizes = Vec::with_capacity(page_count);
	for i in 0..page_count {
		sizes.push(res!(doc.page_size(i)));
	}

	let contents = Contents::new(res!(doc.outline()));
	// The sidebar's faces are parsed only for a document that has a sidebar to set.
	let faces = if contents.is_empty() { None } else { Some(res!(fonts::libertinus())) };

	// An optional headless capture: render the first frame at a given scroll offset, save it as a PNG and
	// exit. This is how the reader is screenshotted under `xvfb-run`, and it doubles as a smoke test that
	// the window opens, rasters a frame and presents it cleanly. Off unless `PEARLITE_CAPTURE` is set.
	// Three companions shape that frame: `PEARLITE_CAPTURE_CONTENTS=closed` starts with the sidebar
	// hidden, `PEARLITE_CAPTURE_FOLD=i,j` folds those outline entries, and `PEARLITE_CAPTURE_JUMP=n`
	// jumps to outline entry `n` exactly as a click on its row does.
	let capture = std::env::var("PEARLITE_CAPTURE").ok().map(|path| Capture {
		path,
		scroll: std::env::var("PEARLITE_CAPTURE_SCROLL").ok()
			.and_then(|s| s.parse::<f64>().ok())
			.unwrap_or(0.0),
	});
	let (toc_open, jump, folds) = match capture {
		Some(_)	=> (
			std::env::var("PEARLITE_CAPTURE_CONTENTS").map(|v| v != "closed").unwrap_or(true),
			std::env::var("PEARLITE_CAPTURE_JUMP").ok().and_then(|s| s.parse::<usize>().ok()),
			std::env::var("PEARLITE_CAPTURE_FOLD").ok()
				.map(|s| s.split(',').filter_map(|t| t.trim().parse::<usize>().ok()).collect())
				.unwrap_or_else(Vec::new),
		),
		None	=> (true, None, Vec::new()),
	};

	let event_loop = res!(EventLoop::new(), IO, System, Init);
	event_loop.set_control_flow(ControlFlow::Wait);

	let mut app = Reader {
		doc,
		title,
		sizes,
		cache:			(0..page_count).map(|_| None).collect(),
		cache_width:	0,
		scroll:			capture.as_ref().map(|c| c.scroll).unwrap_or(0.0),
		window:			None,
		surface:		None,
		capture,
		captured:		false,
		result:			Ok(()),
		contents,
		faces,
		toc_open,
		toc_scroll:		0.0,
		toc_version:	0,
		sidebar:		None,
		layout:			Layout::default(),
		cursor:			(0.0, 0.0),
		jump,
	};
	for idx in folds {
		if app.contents.toggle(idx) {
			app.toc_version += 1;
		}
	}

	res!(event_loop.run_app(&mut app), IO, System);
	app.result
}

/// A pending headless capture: where to write the PNG, and the scroll offset to render it at.
struct Capture {
	path:	String,
	scroll:	f64,
}

/// What the last frame was laid out at, for hit-testing a click against what is on screen.
#[derive(Clone, Copy, Debug, Default)]
struct Layout {
	side:	usize,	// sidebar width, device pixels, zero when hidden
	scale:	f64,
}

/// Everything the sidebar's pixels depend on; the cached sidebar is redrawn only when this changes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SideKey {
	w:			usize,
	h:			usize,
	scale:		f64,
	scroll:		f64,
	current:	Option<usize>,
	version:	u64,
}

type ReaderSurface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

struct Reader {
	doc:			PearlDoc,
	title:			String,
	sizes:			Vec<(usize, usize)>,		// each page's media box, whole points
	cache:			Vec<Option<RasterPage>>,	// rasterised pages at `cache_width`
	cache_width:	usize,						// the page-area width the cache was rendered for
	scroll:			f64,						// vertical scroll, device pixels from the top of the stack
	window:			Option<Rc<Window>>,
	surface:		Option<ReaderSurface>,
	capture:		Option<Capture>,
	captured:		bool,
	result:			Outcome<()>,
	contents:		Contents,					// the outline tree, empty for a document without one
	faces:			Option<FontSet>,			// the sidebar's faces, loaded only when there is a sidebar
	toc_open:		bool,
	toc_scroll:		f64,						// sidebar list scroll, device pixels
	toc_version:	u64,						// bumped on every fold, so the cached sidebar is redrawn
	sidebar:		Option<(SideKey, Pixmap)>,
	layout:			Layout,
	cursor:			(f64, f64),					// last pointer position, device pixels
	jump:			Option<usize>,				// an outline entry to jump to at the next frame
}

impl Reader {
	/// Records the first failure and asks the loop to exit; later failures are dropped, the first being
	/// the one worth reporting.
	fn fail(&mut self, event_loop: &ActiveEventLoop, e: Error<ErrTag>) {
		if self.result.is_ok() {
			self.result = Err(e);
		}
		event_loop.exit();
	}

	fn redraw(&self) {
		if let Some(w) = &self.window {
			w.request_redraw();
		}
	}

	/// The device height, in pixels, a page occupies when fitted to width `w`.
	fn page_device_height(&self, idx: usize, w: usize) -> usize {
		let (pw, ph) = self.sizes[idx];
		if pw == 0 {
			return 0;
		}
		let s = (w as f32) / (pw as f32);
		(((ph as f32) * s).ceil() as usize).max(1)
	}

	/// The DPI a page rasters at to fit width `w`.
	fn page_dpi(&self, idx: usize, w: usize) -> f32 {
		let (pw, _) = self.sizes[idx];
		if pw == 0 {
			return raster::DEFAULT_DPI;
		}
		raster::DEFAULT_DPI * (w as f32) / (pw as f32)
	}

	/// The total height of the page stack at width `w`, including a gutter above every page and below the
	/// last.
	fn stack_height(&self, w: usize, gutter: f64) -> f64 {
		let mut total = gutter;
		for idx in 0..self.sizes.len() {
			total += self.page_device_height(idx, w) as f64 + gutter;
		}
		total
	}

	/// Where a heading sits in the page stack at width `w`, in device pixels from the stack's top: its
	/// page's top plus its own y scaled as the page is. `None` for a heading the ledger never placed, or
	/// one naming a page the document does not have.
	fn entry_y(&self, e: &OutlineEntry, w: usize, gutter: f64) -> Option<f64> {
		let idx = match e.page {
			Some(p) if p >= 1 && (p as usize) <= self.sizes.len()	=> (p as usize) - 1,
			_														=> return None,
		};
		let mut top = gutter;
		for i in 0..idx {
			top += self.page_device_height(i, w) as f64 + gutter;
		}
		let pw = self.sizes[idx].0;
		let s = if pw == 0 { 0.0 } else { (w as f64) / (pw as f64) };
		Some(top + e.y.map(|y| y.to_pt()).unwrap_or(0.0) * s)
	}

	/// The sidebar's width in a window `w` device pixels wide: nothing without an outline or while
	/// hidden.
	fn sidebar_width(&self, w: usize, scale: f64) -> usize {
		if self.contents.is_empty() || !self.toc_open {
			return 0;
		}
		((SIDE_W * scale).round() as usize).min(((w as f64) * SIDE_MAX) as usize)
	}

	/// Renders and presents one frame. Clears the cache when the page area's width changed, clamps the
	/// scrolls, paints the gutter, blits every visible page, then lays the sidebar and toggle over them.
	fn draw(&mut self, event_loop: &ActiveEventLoop) {
		let window = match &self.window {
			Some(w)	=> w.clone(),
			None	=> return,
		};
		let size = window.inner_size();
		let (w, h) = (size.width as usize, size.height as usize);
		if w == 0 || h == 0 {
			return;
		}
		let scale = window.scale_factor();
		let gutter = GUTTER * scale;
		let side = self.sidebar_width(w, scale);
		let area = w - side;	// the page column's width
		if area == 0 {
			return;
		}
		self.layout = Layout { side, scale };

		if area != self.cache_width {
			for slot in self.cache.iter_mut() {
				*slot = None;
			}
			self.cache_width = area;
		}

		// A jump asked for before the geometry was known lands now.
		if let Some(idx) = self.jump.take() {
			if let Some(y) = self.contents.entries().get(idx).and_then(|e| self.entry_y(e, area, gutter)) {
				self.scroll = y - JUMP_CLEAR * scale;
			}
		}

		// Clamp the scroll now the geometry is known.
		let total = self.stack_height(area, gutter);
		let max_scroll = (total - h as f64).max(0.0);
		self.scroll = self.scroll.min(max_scroll).max(0.0);

		// Rasterise every visible page up front, so the borrow of the surface buffer below holds nothing
		// else of `self` mutably.
		let mut placements: Vec<(usize, i64)> = Vec::new();	// (page index, top y on screen)
		let mut y = gutter;
		for idx in 0..self.sizes.len() {
			let ph = self.page_device_height(idx, area) as f64;
			let top = y - self.scroll;
			if top + ph >= 0.0 && top < h as f64 {
				if self.cache[idx].is_none() {
					let dpi = self.page_dpi(idx, area);
					match raster::render_page_to_pixmap(&self.doc, idx, dpi) {
						Ok(page)	=> self.cache[idx] = Some(page),
						Err(e)		=> { self.fail(event_loop, e); return; },
					}
				}
				placements.push((idx, top.round() as i64));
			}
			y += ph + gutter;
		}

		// The sidebar, redrawn only when something it shows has changed, and the toggle floating over the
		// pages while the sidebar is hidden.
		let mut toggle: Option<Pixmap> = None;
		if !self.contents.is_empty() {
			let outcome = if side > 0 {
				self.refresh_sidebar(side, h, scale, area, gutter)
			} else {
				self.sidebar = None;
				self.floating_toggle(scale).map(|pm| { toggle = Some(pm); })
			};
			if let Err(e) = outcome {
				self.fail(event_loop, e);
				return;
			}
		}
		let mut overlays: Vec<(&Pixmap, usize, usize)> = Vec::new();	// (pixels, left, top)
		if let Some((_, pm)) = &self.sidebar {
			if side > 0 {
				overlays.push((pm, 0, 0));
			}
		}
		let at = (PAD * scale).round() as usize;
		if let Some(pm) = &toggle {
			overlays.push((pm, at, at));
		}

		// Disjoint field borrows: the surface, the page cache, the capture and its flag are separate fields,
		// so the framebuffer work happens in an associated function that never re-borrows all of `self` --
		// which is what lets an error surface through the return rather than a `self.fail` call inside the
		// live buffer borrow.
		let outcome = Self::present_frame(
			self.surface.as_mut(),
			&self.cache,
			&self.capture,
			&mut self.captured,
			w, h,
			side,
			&placements,
			&overlays,
		);
		match outcome {
			Ok(true)	=> event_loop.exit(),
			Ok(false)	=> {},
			Err(e)		=> self.fail(event_loop, e),
		}
	}

	/// The toggle drawn on its own, to float over the pages while the sidebar is hidden.
	fn floating_toggle(&self, scale: f64) -> Outcome<Pixmap> {
		let bs = (BTN * scale).round().max(1.0) as usize;
		let mut pm = res!(Pixmap::filled(bs, bs, GUTTER_INK));
		res!(paint_toggle(&mut pm, 0.0, 0.0, bs as f32, false));
		Ok(pm)
	}

	/// Redraws the cached sidebar when its key has changed: the window, the list scroll, the current
	/// heading or a fold.
	fn refresh_sidebar(
		&mut self,
		side:	usize,
		h:		usize,
		scale:	f64,
		area:	usize,
		gutter:	f64,
	)
		-> Outcome<()>
	{
		// Clamp the list scroll to the rows there are.
		let rows = self.contents.visible().len() as f64;
		let content = (HEAD_H + rows * ROW_H + PAD) * scale;
		self.toc_scroll = self.toc_scroll.min((content - h as f64).max(0.0)).max(0.0);

		let at = self.scroll + JUMP_CLEAR * scale + 1.0;
		let current = self.contents.current(|e| self.entry_y(e, area, gutter), at);
		let key = SideKey {
			w:			side,
			h,
			scale,
			scroll:		self.toc_scroll,
			current,
			version:	self.toc_version,
		};
		if let Some((k, _)) = &self.sidebar {
			if *k == key {
				return Ok(());
			}
		}
		let pm = res!(self.render_sidebar(&key));
		self.sidebar = Some((key, pm));
		Ok(())
	}

	/// Paints the sidebar: the visible rows of the contents tree, the current heading's row marked, then
	/// the header band with the toggle and its label laid over any row scrolled beneath it.
	fn render_sidebar(&self, key: &SideKey) -> Outcome<Pixmap> {
		let faces = res!(self.faces.as_ref().ok_or_else(|| err!(
			"The contents sidebar was drawn without its faces loaded."; Bug, Missing)));
		let s		= key.scale as f32;
		let w		= key.w as f32;
		let h		= key.h as f32;
		let rule	= s.max(1.0).round();
		let head	= HEAD_H as f32 * s;
		let row_h	= ROW_H as f32 * s;
		let pad		= PAD as f32 * s;
		let mut pm	= res!(Pixmap::filled(key.w, key.h, SIDE_BG));
		let list	= Bounds::new(0.0, head, w - rule, h);

		let mark = key.current.map(|c| self.contents.shown_for(c));
		for (r, &idx) in self.contents.visible().iter().enumerate() {
			let top = head + (r as f32) * row_h - key.scroll as f32;
			if top + row_h < head || top > h {
				continue;
			}
			let e = &self.contents.entries()[idx];
			let is_mark = mark == Some(idx);
			if is_mark {
				res!(pm.fill_bounds(Bounds::new(pad * 0.5, top, w - rule - pad * 0.5, top + row_h),
					CUR_BG, Some(list)));
			}
			let x0 = pad + (self.contents.depth(idx) as f32) * INDENT as f32 * s;
			let mid = top + row_h * 0.5;
			if self.contents.has_children(idx) {
				res!(paint_caret(&mut pm, x0 + CARET_W as f32 * s * 0.5, mid, s,
					self.contents.is_open(idx), list));
			}
			let (role, size, ink) = match e.level {
				0 | 1	=> (Role::Bold, 13.5, TEXT_MAIN),
				2		=> (Role::Body, 13.0, TEXT_MAIN),
				_		=> (Role::Body, 12.5, TEXT_DEEP),
			};
			let ink = if e.page.is_none() { TEXT_DEAD } else if is_mark { CUR_TEXT } else { ink };
			let font = faces.get(role);
			let size = size * s;
			let m = res!(font.metrics(size));
			let base = mid + (m.ascent - m.descent) * 0.5;
			let right = w - rule - pad;
			let mut x = x0 + CARET_W as f32 * s;
			if !e.number.is_empty() {
				let num = faces.get(Role::Body);
				x += res!(paint_text(&mut pm, num, size, &e.number, x, base, right - x, TEXT_NUM, list));
				x += NUM_GAP as f32 * s;
			}
			res!(paint_text(&mut pm, font, size, &e.title, x, base, right - x, ink, list));
		}

		// The header band, over whatever row scrolled up beneath it.
		let whole = Bounds::new(0.0, 0.0, w, h);
		res!(pm.fill_bounds(Bounds::new(0.0, 0.0, w, head), SIDE_BG, None));
		res!(pm.fill_bounds(Bounds::new(0.0, head - rule, w, head), SIDE_RULE, None));
		res!(pm.fill_bounds(Bounds::new(w - rule, 0.0, w, h), SIDE_RULE, None));
		let b = pad;
		let bs = BTN as f32 * s;
		res!(paint_toggle(&mut pm, b, b, bs, true));
		let label = faces.get(Role::Bold);
		let lsize = 11.0 * s;
		let lm = res!(label.metrics(lsize));
		let lbase = b + bs * 0.5 + (lm.ascent - lm.descent) * 0.5;
		let lx = b + bs + 10.0 * s;
		res!(paint_text(&mut pm, label, lsize, "CONTENTS", lx, lbase, w - rule - pad - lx, TEXT_HEAD, whole));
		Ok(pm)
	}

	/// Resizes the surface, paints the gutter, blits the visible pages into the column right of the
	/// sidebar, lays the overlays on top, optionally captures the frame, and presents it. Returns `true`
	/// when the loop should exit (a headless capture is done). Takes its inputs as separate field borrows
	/// so no `&mut self` overlaps the live framebuffer borrow.
	fn present_frame(
		surface:	Option<&mut ReaderSurface>,
		cache:		&[Option<RasterPage>],
		capture:	&Option<Capture>,
		captured:	&mut bool,
		w:			usize,
		h:			usize,
		left:		usize,
		placements:	&[(usize, i64)],
		overlays:	&[(&Pixmap, usize, usize)],
	) -> Outcome<bool> {
		let surface = match surface {
			Some(s)	=> s,
			None	=> return Ok(false),
		};
		let nz_w = res!(NonZeroU32::new(w as u32).ok_or_else(|| err!(
			"A frame cannot have zero width."; Bug, Invalid)));
		let nz_h = res!(NonZeroU32::new(h as u32).ok_or_else(|| err!(
			"A frame cannot have zero height."; Bug, Invalid)));
		if let Err(e) = surface.resize(nz_w, nz_h) {
			return Err(err!("Resizing the framebuffer to {}x{} failed: {}.", w, h, e; IO, System));
		}
		let mut buffer = match surface.buffer_mut() {
			Ok(b)	=> b,
			Err(e)	=> return Err(err!("Acquiring the framebuffer failed: {}.", e; IO, System)),
		};

		for px in buffer.iter_mut() {
			*px = GUTTER_RGB;
		}

		for (idx, top) in placements {
			if let Some(page) = &cache[*idx] {
				blit(&mut buffer, w, h, page.pixmap.data(), page.width_px, page.height_px,
					left, *top);
			}
		}
		for (pm, x, y) in overlays {
			blit(&mut buffer, w, h, pm.data(), pm.width(), pm.height(), *x, *y as i64);
		}

		// A headless capture takes the finished frame before `present` consumes the buffer.
		if let Some(cap) = capture {
			if !*captured {
				res!(save_frame(&buffer, w, h, &cap.path));
				*captured = true;
			}
		}

		if let Err(e) = buffer.present() {
			return Err(err!("Presenting the framebuffer failed: {}.", e; IO, System));
		}

		Ok(capture.is_some() && *captured)
	}

	/// Applies a scroll delta and asks for a redraw.
	fn scroll_by(&mut self, delta: f64) {
		self.scroll += delta;
		self.redraw();
	}

	/// Jumps to the top or the bottom of the stack.
	fn scroll_to(&mut self, top: bool) {
		self.scroll = if top { 0.0 } else { f64::MAX };
		self.redraw();
	}

	/// A page step: most of the viewport height, in the given direction (`+1` down, `-1` up).
	fn page_step(&mut self, dir: f64) {
		let step = self.window.as_ref()
			.map(|w| w.inner_size().height as f64 * PAGE_FRACTION)
			.unwrap_or(LINE_STEP);
		self.scroll_by(dir * step);
	}

	/// Shows or hides the contents sidebar. The page column changes width, so the pages re-fit.
	fn toggle_contents(&mut self) {
		if self.contents.is_empty() {
			return;
		}
		self.toc_open = !self.toc_open;
		self.redraw();
	}

	/// A left click: the toggle, a caret folding its branch, or a title jumping to its heading.
	fn click(&mut self) {
		if self.contents.is_empty() {
			return;
		}
		let Layout { side, scale, .. } = self.layout;
		let (cx, cy) = self.cursor;
		let b = PAD * scale;
		let bs = BTN * scale;
		if cx >= b && cx < b + bs && cy >= b && cy < b + bs {
			self.toggle_contents();
			return;
		}
		let head = HEAD_H * scale;
		if side == 0 || cx >= side as f64 || cy < head {
			return;
		}
		let row = ((cy - head + self.toc_scroll) / (ROW_H * scale)).floor();
		if row < 0.0 {
			return;
		}
		let idx = match self.contents.visible().get(row as usize) {
			Some(&i)	=> i,
			None		=> return,
		};
		let x0 = PAD * scale + (self.contents.depth(idx) as f64) * INDENT * scale;
		if self.contents.has_children(idx) && cx >= x0 && cx < x0 + CARET_W * scale {
			if self.contents.toggle(idx) {
				self.toc_version += 1;
			}
		} else if self.contents.entries()[idx].page.is_some() {
			self.jump = Some(idx);
		}
		self.redraw();
	}
}

impl ApplicationHandler for Reader {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.window.is_some() {
			return;	// already have a window; a second resume is not a second window
		}
		// Wide enough for the sidebar beside a full page when there is an outline.
		let width = if self.contents.is_empty() { 900.0 } else { 900.0 + SIDE_W };
		let attrs = Window::default_attributes()
			.with_title(self.title.clone())
			.with_inner_size(winit::dpi::LogicalSize::new(width, 1120.0));
		let window = match event_loop.create_window(attrs) {
			Ok(w)	=> Rc::new(w),
			Err(e)	=> { self.fail(event_loop, err!("Creating the reader window failed: {}.", e; IO, System, Init)); return; },
		};
		let context = match softbuffer::Context::new(window.clone()) {
			Ok(c)	=> c,
			Err(e)	=> { self.fail(event_loop, err!("Creating the softbuffer context failed: {}.", e; IO, System, Init)); return; },
		};
		let surface = match softbuffer::Surface::new(&context, window.clone()) {
			Ok(s)	=> s,
			Err(e)	=> { self.fail(event_loop, err!("Creating the framebuffer surface failed: {}.", e; IO, System, Init)); return; },
		};
		window.request_redraw();
		self.window = Some(window);
		self.surface = Some(surface);
	}

	fn window_event(
		&mut self,
		event_loop:	&ActiveEventLoop,
		_id:		WindowId,
		event:		WindowEvent,
	) {
		match event {
			WindowEvent::CloseRequested => {
				event_loop.exit();
			},
			WindowEvent::Resized(_) => {
				self.redraw();
			},
			WindowEvent::ScaleFactorChanged { .. } => {
				// The physical inner size already carries the new factor; a redraw re-fits and re-rasters.
				self.cache_width = 0;	// force a re-raster at the new physical width
				self.redraw();
			},
			WindowEvent::RedrawRequested => {
				self.draw(event_loop);
			},
			WindowEvent::CursorMoved { position, .. } => {
				self.cursor = (position.x, position.y);
			},
			WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
				self.click();
			},
			WindowEvent::MouseWheel { delta, .. } => {
				let dy = match delta {
					MouseScrollDelta::LineDelta(_, y)	=> (y as f64) * LINE_STEP,
					MouseScrollDelta::PixelDelta(p)		=> p.y,
				};
				// Wheel up (positive) scrolls the content up, toward the top: a smaller offset. Over the
				// sidebar it scrolls the contents list instead of the pages.
				if self.layout.side > 0 && self.cursor.0 < self.layout.side as f64 {
					self.toc_scroll -= dy;
					self.redraw();
				} else {
					self.scroll_by(-dy);
				}
			},
			WindowEvent::KeyboardInput {
				event: KeyEvent { logical_key, state: ElementState::Pressed, .. },
				..
			} => {
				match logical_key {
					Key::Named(NamedKey::PageDown)		=> self.page_step(1.0),
					Key::Named(NamedKey::PageUp)		=> self.page_step(-1.0),
					Key::Named(NamedKey::ArrowDown)		=> self.scroll_by(LINE_STEP),
					Key::Named(NamedKey::ArrowUp)		=> self.scroll_by(-LINE_STEP),
					Key::Named(NamedKey::Space)			=> self.page_step(1.0),
					Key::Named(NamedKey::Home)			=> self.scroll_to(true),
					Key::Named(NamedKey::End)			=> self.scroll_to(false),
					Key::Named(NamedKey::Escape)		=> event_loop.exit(),
					Key::Character(c) if c == "t"		=> self.toggle_contents(),
					_									=> {},
				}
			},
			_ => {},
		}
	}
}

/// Sets `text` in `font` at `size` device pixels on the baseline `base` from `x`, clipped to `clip`,
/// ending in an ellipsis when it would run past `max_w`. Returns the width it took.
fn paint_text(
	pm:		&mut Pixmap,
	font:	&Font,
	size:	f32,
	text:	&str,
	x:		f32,
	base:	f32,
	max_w:	f32,
	ink:	Rgba,
	clip:	Bounds,
)
	-> Outcome<f32>
{
	if max_w <= 0.0 {
		return Ok(0.0);
	}
	let run = res!(font.shape(text, size, Dir::Ltr));
	if run.advance <= max_w {
		for g in &run.glyphs {
			res!(paint_glyph(pm, font, g.face, g.id, size, x + g.x, base - g.y, ink, clip));
		}
		return Ok(run.advance);
	}
	let ell = res!(font.shape("\u{2026}", size, Dir::Ltr));
	let limit = max_w - ell.advance;
	let mut pen = 0.0;
	for g in run.glyphs.iter().take_while(|g| g.x + g.adv <= limit) {
		res!(paint_glyph(pm, font, g.face, g.id, size, x + g.x, base - g.y, ink, clip));
		pen = g.x + g.adv;
	}
	for g in &ell.glyphs {
		res!(paint_glyph(pm, font, g.face, g.id, size, x + pen + g.x, base - g.y, ink, clip));
	}
	Ok(pen + ell.advance)
}

/// Fills one glyph's outline with its origin at (`x`, `y`). The outline is font-frame, y up; the pixmap
/// is y down, so it is flipped first, exactly as the SVG emitter places a glyph.
fn paint_glyph(
	pm:		&mut Pixmap,
	font:	&Font,
	face:	u8,
	id:		u32,
	size:	f32,
	x:		f32,
	y:		f32,
	ink:	Rgba,
	clip:	Bounds,
)
	-> Outcome<()>
{
	let path = res!(font.outline(face, id, size));
	if path.is_empty() {
		return Ok(());	// a space: an advance and no ink
	}
	let t = Transform::scale(1.0, -1.0).then(&Transform::translate(x, y));
	pm.fill_path(&path, &t, ink, Some(clip))
}

/// A branch's caret centred on (`cx`, `cy`): pointing down while open, right while folded.
fn paint_caret(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, open: bool, clip: Bounds) -> Outcome<()> {
	let (a, b, c) = if open {
		(Pt::new(cx - 4.0 * s, cy - 2.0 * s), Pt::new(cx + 4.0 * s, cy - 2.0 * s), Pt::new(cx, cy + 3.0 * s))
	} else {
		(Pt::new(cx - 2.0 * s, cy - 4.0 * s), Pt::new(cx - 2.0 * s, cy + 4.0 * s), Pt::new(cx + 3.0 * s, cy))
	};
	let mut pb = PathBuilder::new();
	pb.move_to(a);
	pb.line_to(b);
	pb.line_to(c);
	pb.close();
	let path: Path = res!(pb.finish());
	pm.fill_path(&path, &Transform::IDENTITY, CARET_INK, Some(clip))
}

/// The contents toggle, a `bs`-square button at (`x`, `y`) bearing three bars: filled blue while the
/// sidebar is open, as the web reader's pressed toggle is, and white with an edge while it is hidden.
fn paint_toggle(pm: &mut Pixmap, x: f32, y: f32, bs: f32, on: bool) -> Outcome<()> {
	let r = bs * 0.18;
	let edge = (bs / 28.0).max(1.0);
	let (face, bars) = if on { (BTN_ON, BTN_OFF) } else { (BTN_OFF, BTN_BARS) };
	if !on {
		let outer = res!(Path::round_rect(Bounds::new(x, y, x + bs, y + bs), r));
		res!(pm.fill_path(&outer, &Transform::IDENTITY, BTN_EDGE, None));
	}
	let inner = if on {
		Bounds::new(x, y, x + bs, y + bs)
	} else {
		Bounds::new(x + edge, y + edge, x + bs - edge, y + bs - edge)
	};
	let body = res!(Path::round_rect(inner, (r - edge).max(0.0)));
	res!(pm.fill_path(&body, &Transform::IDENTITY, face, None));
	let (bw, bh) = (bs * 0.5, (bs / 14.0).max(1.0));
	let bx = x + (bs - bw) * 0.5;
	for k in [-1.0f32, 0.0, 1.0] {
		let cy = y + bs * 0.5 + k * bs * 0.18;
		res!(pm.fill_bounds(Bounds::new(bx, cy - bh * 0.5, bx + bw, cy + bh * 0.5), bars, None));
	}
	Ok(())
}

/// Blits RGBA pixels `src_w` by `src_h` into the framebuffer at (`left`, `top`), clipping to the window.
fn blit(
	buffer:	&mut [u32],
	w:		usize,
	h:		usize,
	data:	&[u8],
	src_w:	usize,
	src_h:	usize,
	left:	usize,
	top:	i64,
) {
	if left >= w {
		return;
	}
	let cols = src_w.min(w - left);
	for py in 0..src_h {
		let sy = top + py as i64;
		if sy < 0 {
			continue;
		}
		let sy = sy as usize;
		if sy >= h {
			break;
		}
		let row = sy * w + left;
		let src_row = py * src_w * 4;
		for px in 0..cols {
			let si = src_row + px * 4;
			// Every source here is opaque -- a page on its white ground, the sidebar on its own -- so alpha
			// is ignored: straight RGB into 0x00RRGGBB.
			let r = data[si] as u32;
			let g = data[si + 1] as u32;
			let b = data[si + 2] as u32;
			buffer[row + px] = (r << 16) | (g << 8) | b;
		}
	}
}

/// Encodes the finished framebuffer to a PNG at `path`, for the headless capture path.
fn save_frame(buffer: &[u32], w: usize, h: usize, path: &str) -> Outcome<()> {
	let mut rgba = Vec::with_capacity(w * h * 4);
	for px in buffer {
		rgba.push(((px >> 16) & 0xff) as u8);
		rgba.push(((px >> 8) & 0xff) as u8);
		rgba.push((px & 0xff) as u8);
		rgba.push(255);
	}
	let pm = res!(Pixmap::from_data(w, h, rgba));
	let png = res!(pm.to_png());
	res!(std::fs::write(path, &png), IO, File, Write);
	Ok(())
}
