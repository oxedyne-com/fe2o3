//! Do fonts injected through a project's `fonts` field actually reach the document as usable faces?
//!
//! The wasm surface installs every `project.fonts` pair into the source map ([`crate::vfs`]) and then
//! assembles the document. Before the fix in this lane a font was installed only at the path the consumer
//! named, which the lone-file face resolver -- reading its own `<root>/assets/fonts` directory alone --
//! never saw unless the consumer happened to inject it there. The wasm surface now also places each font at
//! [`book::project_font_path`], the resolver's own location, so a face the document names resolves whatever
//! path the file was injected under. This test drives the same [`compile::assemble`] path natively, with a
//! source map installed as the wasm surface installs one, and exercises the real [`book::project_font_path`]
//! the surface routes through -- so it is the native proof of the browser behaviour.

use oxedyne_fe2o3_austenite::book;
use oxedyne_fe2o3_austenite::compile;
use oxedyne_fe2o3_austenite::fonts;
use oxedyne_fe2o3_austenite::vfs;

use oxedyne_fe2o3_core::prelude::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// A lone document that names `Testface` as its heading display face through the `doc.with` idiom the
/// documentation trees use, so the theme's top heading levels resolve against it.
const SRC: &str = "#import \"template.typ\": *\n#show: doc.with(heading-font: \"Testface\")\n\n= A Heading\n\nBody text that sets in the reading face.\n";

/// The bytes of a real, parseable face, borrowed from the crate's own Libertinus set so the resolver has
/// something that actually loads (a face that will not parse is dropped by the resolver, which would make
/// a negative result ambiguous).
fn face_bytes() -> Vec<u8> {
	let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fonts").join("LibertinusSerif-Regular.otf");
	std::fs::read(&path).expect("the crate's embedded Libertinus face must be present for this test")
}

/// Assembles the lone document with the given font pairs installed into the source map, returning whether
/// the assembled document's face resolver holds `Testface` -- or the assembly error, which a heading face
/// the document names but no injected font supplies now is (the missing-family precheck). When `route` is set, each font is additionally
/// installed at [`book::project_font_path`] -- exactly what the wasm surface's `read_font_pairs` now does --
/// so the test exercises the real routing rather than a copy of it. Mirrors the wasm `assemble_and_run`
/// otherwise: the embedded reading set is the lone-file provider, and the map is cleared afterwards.
fn resolves_injected(font_pairs: &[(&str, Vec<u8>)], route: bool) -> Outcome<Result<bool, String>> {
	let main = PathBuf::from("/main.typ");
	let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
	files.insert(main.clone(), SRC.as_bytes().to_vec());
	for (path, bytes) in font_pairs {
		let given = PathBuf::from(path);
		if route {
			if let Some(routed) = book::project_font_path(&main, &given) {
				files.insert(routed, bytes.clone());
			}
		}
		files.insert(given, bytes.clone());
	}
	res!(vfs::install(files));
	let fonts	= Arc::new(res!(fonts::libertinus()));
	let outcome	= compile::assemble(&main, || Ok(fonts.clone()));
	let _		= vfs::clear();
	Ok(match outcome {
		Ok((assembled, _refusals, _skip))	=> Ok(assembled.faces.resolves("Testface")),
		Err(e)								=> Err(fmt!("{}", e)),
	})
}

/// Pins the whole story in one test (the shared source-map global forbids two at once): the resolver reads
/// only its own directory, so an unrouted font is invisible unless injected exactly there; routing each
/// injected font to [`book::project_font_path`] -- as the wasm surface now does -- makes a face named in the
/// document resolve whatever path the consumer chose to inject it under.
#[test]
fn injected_fonts_resolve_once_routed_to_the_resolver_path() -> Outcome<()> {
	// The bare gap: the same bytes under a naive `/fonts` directory are invisible without routing.
	let unrouted_bare = res!(resolves_injected(&[("/fonts/Testface-Regular.otf", face_bytes())], false));
	// The pre-existing narrow path that worked: injected exactly where the resolver reads.
	let unrouted_at_assets = res!(resolves_injected(&[("/assets/fonts/Testface-Regular.otf", face_bytes())], false));
	// The fix: routing an arbitrarily-pathed injected font makes it resolve.
	let routed_bare = res!(resolves_injected(&[("/fonts/Testface-Regular.otf", face_bytes())], true));
	// And a bare basename with no directory at all also resolves once routed.
	let routed_basename = res!(resolves_injected(&[("Testface-Regular.otf", face_bytes())], true));

	eprintln!(
		"[font-inject] unrouted /fonts -> {:?}, unrouted /assets/fonts -> {:?}, routed /fonts -> {:?}, routed basename -> {:?}",
		unrouted_bare, unrouted_at_assets, routed_bare, routed_basename);

	// Without routing, a font outside /assets/fonts is invisible; the heading face the document names is
	// then missing, which is a hard error naming it rather than a silent fall-back to the body role.
	match &unrouted_bare {
		Err(msg)	=> assert!(msg.contains("Testface"), "the error must name the missing family: {}", msg),
		Ok(r)		=> return Err(err!(
			"An unrouted, invisible font must fail the precheck, but assembly gave {}.", r; Test, Mismatch)),
	}
	assert_eq!(unrouted_at_assets, Ok(true), "a font injected exactly at /assets/fonts/<Name>-Regular.otf always resolved");
	assert_eq!(routed_bare, Ok(true), "routing makes an arbitrarily-pathed injected font resolve -- the fix");
	assert_eq!(routed_basename, Ok(true), "routing makes a bare-basename injected font resolve too");
	Ok(())
}
