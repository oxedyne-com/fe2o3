// U10 owns this file. Packages are read from `/@<namespace>/<name>/<version>/` in the vfs, which the host
// fills (owner decision 2); the engine never fetches. A package absent from the vfs is a hard error.

use crate::eval::func::unimplemented;
use crate::eval::World;

use oxedyne_fe2o3_core::prelude::*;

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageSpec {
	pub namespace:	String,		// `preview`, `local`
	pub name:		String,
	pub version:	(u32, u32, u32),
}

impl PackageSpec {
	/// The package's root directory in the vfs.
	pub fn dir(&self) -> PathBuf {
		PathBuf::from(fmt!("/@{}/{}/{}.{}.{}",
			self.namespace, self.name, self.version.0, self.version.1, self.version.2))
	}
}

/// Parses `@namespace/name:1.2.3`.
pub fn parse_spec(spec: &str) -> Outcome<PackageSpec> {
	Err(unimplemented("package", spec))
}

/// The entrypoint file named by the package's `typst.toml`.
pub fn entrypoint(_world: &World, spec: &PackageSpec) -> Outcome<PathBuf> {
	Err(unimplemented("package", &spec.name))
}
