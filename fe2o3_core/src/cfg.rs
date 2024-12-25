use crate::file::Loadable;

use std::path::PathBuf;

/// Allows configuration to be specified either with `Config` or a file.
#[derive(Debug)]
pub enum ConfigInit<C: Loadable> {
    Data(C),
    Path(PathBuf),
}

