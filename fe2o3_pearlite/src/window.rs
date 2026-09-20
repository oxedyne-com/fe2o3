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
//! This module is behind the default-off `gui` feature, the sole gate on the winit and softbuffer
//! dependencies. House rule: the winit callback model cannot return an error, so a failure inside a
//! handler is stored on the app and the loop is asked to exit; [`open`] inspects it after the loop and
//! surfaces it as an [`Outcome`]. No `unwrap`, `?` or `unsafe` in this crate's own code.

use crate::raster::{
	self,
	RasterPage,
};

use oxedyne_fe2o3_austenite::emit::pearl::PearlDoc;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::pixmap::Pixmap;

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::application::ApplicationHandler;
use winit::event::{
	ElementState,
	KeyEvent,
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

/// Opens `doc` in a native window titled `title`, returning when the window closes. Blocks the calling
/// thread for the lifetime of the window, as a desktop reader's main loop does.
pub fn open(doc: PearlDoc, title: String) -> Outcome<()> {
	let page_count = res!(doc.page_count());
	let mut sizes = Vec::with_capacity(page_count);
	for i in 0..page_count {
		sizes.push(res!(doc.page_size(i)));
	}

	// An optional headless capture: render the first frame at a given scroll offset, save it as a PNG and
	// exit. This is how the reader is screenshotted under `xvfb-run`, and it doubles as a smoke test that
	// the window opens, rasters a frame and presents it cleanly. Off unless `PEARLITE_CAPTURE` is set.
	let capture = std::env::var("PEARLITE_CAPTURE").ok().map(|path| Capture {
		path,
		scroll: std::env::var("PEARLITE_CAPTURE_SCROLL").ok()
			.and_then(|s| s.parse::<f64>().ok())
			.unwrap_or(0.0),
	});

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
	};

	res!(event_loop.run_app(&mut app), IO, System);
	app.result
}

/// A pending headless capture: where to write the PNG, and the scroll offset to render it at.
struct Capture {
	path:	String,
	scroll:	f64,
}

type ReaderSurface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

struct Reader {
	doc:			PearlDoc,
	title:			String,
	sizes:			Vec<(usize, usize)>,		// each page's media box, whole points
	cache:			Vec<Option<RasterPage>>,	// rasterised pages at `cache_width`
	cache_width:	usize,						// the window width the cache was rendered for
	scroll:			f64,						// vertical scroll, device pixels from the top of the stack
	window:			Option<Rc<Window>>,
	surface:		Option<ReaderSurface>,
	capture:		Option<Capture>,
	captured:		bool,
	result:			Outcome<()>,
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

	/// Renders and presents one frame. Clears the cache when the width changed, clamps the scroll to the
	/// stack, paints the gutter, then blits every visible page.
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

		if w != self.cache_width {
			for slot in self.cache.iter_mut() {
				*slot = None;
			}
			self.cache_width = w;
		}

		// Clamp the scroll now the geometry is known.
		let total = self.stack_height(w, gutter);
		let max_scroll = (total - h as f64).max(0.0);
		if self.scroll > max_scroll {
			self.scroll = max_scroll;
		}
		if self.scroll < 0.0 {
			self.scroll = 0.0;
		}

		// Rasterise every visible page up front, so the borrow of the surface buffer below holds nothing
		// else of `self` mutably.
		let mut placements: Vec<(usize, i64)> = Vec::new();	// (page index, top y on screen)
		let mut y = gutter;
		for idx in 0..self.sizes.len() {
			let ph = self.page_device_height(idx, w) as f64;
			let top = y - self.scroll;
			if top + ph >= 0.0 && top < h as f64 {
				if self.cache[idx].is_none() {
					let dpi = self.page_dpi(idx, w);
					match raster::render_page_to_pixmap(&self.doc, idx, dpi) {
						Ok(page)	=> self.cache[idx] = Some(page),
						Err(e)		=> { self.fail(event_loop, e); return; },
					}
				}
				placements.push((idx, top.round() as i64));
			}
			y += ph + gutter;
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
			&placements,
		);
		match outcome {
			Ok(true)	=> event_loop.exit(),
			Ok(false)	=> {},
			Err(e)		=> self.fail(event_loop, e),
		}
	}

	/// Resizes the surface, paints the gutter, blits the visible pages, optionally captures the frame, and
	/// presents it. Returns `true` when the loop should exit (a headless capture is done). Takes its inputs
	/// as separate field borrows so no `&mut self` overlaps the live framebuffer borrow.
	fn present_frame(
		surface:	Option<&mut ReaderSurface>,
		cache:		&[Option<RasterPage>],
		capture:	&Option<Capture>,
		captured:	&mut bool,
		w:			usize,
		h:			usize,
		placements:	&[(usize, i64)],
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
				blit(&mut buffer, w, h, page, *top);
			}
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
		if let Some(w) = &self.window {
			w.request_redraw();
		}
	}

	/// Jumps to the top or the bottom of the stack.
	fn scroll_to(&mut self, top: bool) {
		self.scroll = if top { 0.0 } else { f64::MAX };
		if let Some(w) = &self.window {
			w.request_redraw();
		}
	}

	/// A page step: most of the viewport height, in the given direction (`+1` down, `-1` up).
	fn page_step(&mut self, dir: f64) {
		let step = self.window.as_ref()
			.map(|w| w.inner_size().height as f64 * PAGE_FRACTION)
			.unwrap_or(LINE_STEP);
		self.scroll_by(dir * step);
	}
}

impl ApplicationHandler for Reader {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.window.is_some() {
			return;	// already have a window; a second resume is not a second window
		}
		let attrs = Window::default_attributes()
			.with_title(self.title.clone())
			.with_inner_size(winit::dpi::LogicalSize::new(900.0, 1120.0));
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
				if let Some(w) = &self.window {
					w.request_redraw();
				}
			},
			WindowEvent::ScaleFactorChanged { .. } => {
				// The physical inner size already carries the new factor; a redraw re-fits and re-rasters.
				self.cache_width = 0;	// force a re-raster at the new physical width
				if let Some(w) = &self.window {
					w.request_redraw();
				}
			},
			WindowEvent::RedrawRequested => {
				self.draw(event_loop);
			},
			WindowEvent::MouseWheel { delta, .. } => {
				let dy = match delta {
					MouseScrollDelta::LineDelta(_, y)	=> (y as f64) * LINE_STEP,
					MouseScrollDelta::PixelDelta(p)		=> p.y,
				};
				// Wheel up (positive) scrolls the content up, toward the top: a smaller offset.
				self.scroll_by(-dy);
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
					_									=> {},
				}
			},
			_ => {},
		}
	}
}

/// Blits a rasterised page into the framebuffer at screen row `top`, clipping to the window. The page is
/// left-aligned; when it is narrower than the window the gutter shows through on the right.
fn blit(buffer: &mut [u32], w: usize, h: usize, page: &RasterPage, top: i64) {
	let pw = page.width_px.min(w);
	let data = page.pixmap.data();
	for py in 0..page.height_px {
		let sy = top + py as i64;
		if sy < 0 {
			continue;
		}
		let sy = sy as usize;
		if sy >= h {
			break;
		}
		let row = sy * w;
		let src_row = py * page.width_px * 4;
		for px in 0..pw {
			let si = src_row + px * 4;
			// The pixmap ground is opaque white, so alpha is ignored: straight RGB into 0x00RRGGBB.
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
