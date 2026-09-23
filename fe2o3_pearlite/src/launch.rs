//! The reader as a desktop application: a document named on the command line opens straight into the
//! window, and a launch with none -- a double-click, or the applications menu -- asks for one through the
//! platform's own open-file dialog. A launch from the desktop has no terminal to write to, so a failure
//! is shown in a message box as well as on standard error.

use crate::window;

use oxedyne_fe2o3_austenite::emit::pearl::PearlDoc;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{
	Path,
	PathBuf,
};

/// Opens `path` when given, otherwise asks for a document first. Cancelling the dialog is not an error:
/// the launch simply ends. Any failure is reported on screen before it is returned.
pub fn run(path: Option<PathBuf>) -> Outcome<()> {
	let path = match path {
		Some(p)	=> p,
		None	=> match pick_document() {
			Some(p)	=> p,
			None	=> return Ok(()),
		},
	};
	let outcome = open_path(&path);
	if let Err(e) = &outcome {
		report(&fmt!("Pearlite could not open {}.\n\n{}", path.display(), e.plain()));
	}
	outcome
}

/// Shows the native open-file dialog, filtered to `.prl` documents. `None` when the reader cancels, or
/// when the platform has no dialog to show.
pub fn pick_document() -> Option<PathBuf> {
	rfd::FileDialog::new()
		.set_title("Open a Pearlite document")
		.add_filter("Pearlite documents", &["prl"])
		.pick_file()
}

/// Reads a document and opens it in a window titled with its file stem, since the format carries no
/// title of its own.
pub fn open_path(path: &Path) -> Outcome<()> {
	let doc = res!(PearlDoc::read_file(path));
	let title = path
		.file_stem()
		.map(|n| n.to_string_lossy().into_owned())
		.filter(|s| !s.is_empty())
		.unwrap_or_else(|| "Pearlite".to_string());
	window::open(doc, title)
}

/// Shows an error in a message box, and on standard error for a launch that does have a terminal.
pub fn report(msg: &str) {
	eprintln!("pearlite: {}", msg);
	let _ = rfd::MessageDialog::new()
		.set_level(rfd::MessageLevel::Error)
		.set_title("Pearlite")
		.set_description(msg)
		.set_buttons(rfd::MessageButtons::Ok)
		.show();
}
