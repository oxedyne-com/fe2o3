// U10 owns this file. Packages are read from `/@<namespace>/<name>/<version>/` in the vfs, which the host
// fills (owner decision 2); the engine never fetches. A package absent from the vfs is a hard error.

use crate::eval::func::unimplemented;
use crate::eval::World;
use crate::syntax::lexer::is_ident;

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

/// Parses `@namespace/name:1.2.3` as Typst does, with Typst's messages for each malformed part.
pub fn parse_spec(spec: &str) -> Outcome<PackageSpec> {
	let rest = match spec.strip_prefix('@') {
		Some(r)	=> r,
		None	=> return Err(bad("package specification must start with '@'".to_string())),
	};
	let (namespace, rest) = rest.split_once('/').unwrap_or((rest, ""));
	if namespace.is_empty() {
		return Err(bad("package specification is missing namespace".to_string()));
	}
	if !is_ident(namespace) {
		return Err(bad(fmt!("`{}` is not a valid package namespace", namespace)));
	}
	let (name, version) = rest.split_once(':').unwrap_or((rest, ""));
	if name.is_empty() {
		return Err(bad("package specification is missing name".to_string()));
	}
	if !is_ident(name) {
		return Err(bad(fmt!("`{}` is not a valid package name", name)));
	}
	if version.is_empty() {
		return Err(bad("package specification is missing version".to_string()));
	}
	let mut parts = version.split('.');
	let mut next = |which: &str| -> Outcome<u32> {
		match parts.next().filter(|p| !p.is_empty()) {
			Some(p)	=> p.parse::<u32>().map_err(|_| bad(fmt!("`{}` is not a valid {} version", p, which))),
			None	=> Err(bad(fmt!("version number is missing {} version", which))),
		}
	};
	let major = res!(next("major"));
	let minor = res!(next("minor"));
	let patch = res!(next("patch"));
	if let Some(extra) = parts.next() {
		return Err(bad(fmt!("version number has unexpected fourth component: `{}`", extra)));
	}
	Ok(PackageSpec {
		namespace:	namespace.to_string(),
		name:		name.to_string(),
		version:	(major, minor, patch),
	})
}

fn bad(msg: String) -> Error<ErrTag> {
	err!("{}", msg; Input, Invalid)
}

/// The entrypoint file named by the package's `typst.toml`.
pub fn entrypoint(_world: &World, spec: &PackageSpec) -> Outcome<PathBuf> {
	Err(unimplemented("package", &spec.name))
}
