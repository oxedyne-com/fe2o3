//! `pearlite-reader` -- the reader as a desktop application with no console. On Windows it is linked
//! for the GUI subsystem, so a double-click, or a `.prl` opened from Explorer, shows the window and
//! nothing else; the console `pearlite` binary keeps the subcommands. A document path, when the
//! desktop passes one, opens directly; otherwise the open-file dialog asks for one.

#![windows_subsystem = "windows"]

use oxedyne_fe2o3_pearlite::launch;

use oxedyne_fe2o3_core::prelude::*;

fn main() -> Outcome<()> {
	launch::run(std::env::args_os().nth(1).map(std::path::PathBuf::from))
}
