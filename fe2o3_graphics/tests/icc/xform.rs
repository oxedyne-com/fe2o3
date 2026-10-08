//! Transforms between profiles, held against LittleCMS 2.17 (`transicc`) to one level in 255 on
//! every channel of every swatch, for each of the four intents with black-point compensation on and
//! off. The numbers are in `xform_data.rs`, made by `tools/icc/fixtures.sh xform`.
//!
//! FOGRA39L Coated is the bundled CMYK profile and its tests always run. Ghostscript's profiles are
//! read from their system path by the `#[ignore]`d tests, which are run with `--ignored`.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use super::{
	GS,
	fogra_bytes,
	system,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::icc::{
	lab,
	read::{
		Curve,
		Profile,
	},
	transform::{
		Dev,
		Intent,
		Transform,
		grey_to_k,
	},
};

#[path = "xform_data.rs"]
#[allow(non_upper_case_globals)]
mod data;

const ONE: f64 = 1.0 / 255.0;

// The intents and settings of compensation, in the order the fixture script makes them.
macro_rules! seven {
	($a:expr, $b:expr, $c:expr, $d:expr, $e:expr, $f:expr, $g:expr) => {
		[
			("T0", Intent::Perceptual, false, $a),
			("T0B", Intent::Perceptual, true, $b),
			("T1", Intent::Relative, false, $c),
			("T1B", Intent::Relative, true, $d),
			("T2", Intent::Saturation, false, $e),
			("T2B", Intent::Saturation, true, $f),
			("T3", Intent::Absolute, false, $g),
		]
	};
}

// The swatches of the fixture script: a 3-step cube, red slowest, eight greys and eight colours.
fn swatches() -> Vec<[f64; 3]> {
	let mut v = Vec::new();
	for r in [0, 128, 255] {
		for g in [0, 128, 255] {
			for b in [0, 128, 255] {
				v.push([r, g, b]);
			}
		}
	}
	for g in [16, 48, 80, 112, 160, 192, 224, 240] {
		v.push([g, g, g]);
	}
	v.extend([
		[224, 172, 105], [91, 155, 213], [96, 153, 60], [110, 20, 30],
		[240, 200, 220], [128, 128, 0], [0, 128, 128], [101, 67, 33],
	]);
	v.iter().map(|c| [c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0]).collect()
}

fn cmyks() -> Vec<[f64; 4]> {
	let mut v: Vec<[f64; 4]> = [0.0, 25.0, 50.0, 75.0, 100.0].iter().map(|k| [0.0, 0.0, 0.0, *k]).collect();
	v.extend([
		[100.0, 0.0, 0.0, 0.0], [0.0, 100.0, 0.0, 0.0], [0.0, 0.0, 100.0, 0.0], [100.0, 100.0, 100.0, 0.0],
		[100.0, 100.0, 100.0, 100.0], [50.0, 50.0, 50.0, 0.0], [0.0, 50.0, 100.0, 20.0],
	]);
	v.iter().map(|c| [c[0] / 100.0, c[1] / 100.0, c[2] / 100.0, c[3] / 100.0]).collect()
}

fn greys() -> Vec<f64> {
	[0.0, 32.0, 64.0, 96.0, 128.0, 160.0, 192.0, 224.0, 255.0].iter().map(|g| g / 255.0).collect()
}

fn fogra() -> Outcome<Dev> {
	let b = res!(fogra_bytes());
	Dev::from_profile(&res!(Profile::read("FOGRA39L_coated.icc", &b)))
}

fn gs(name: &str) -> Outcome<Dev> {
	Dev::from_profile(&res!(system(GS, name)))
}

fn report(tag: &str, worst: f64, at: usize) {
	eprintln!("{}: worst channel error {:.5} ({:.2} of 255) at swatch {}", tag, worst, worst * 255.0, at);
	assert!(worst <= ONE, "{}: a channel is {:.2} levels from LittleCMS at swatch {}", tag, worst * 255.0, at);
}

fn rgb_to_cmyk_set(dst: &Dev, name: &str, sets: &[(&str, Intent, bool, &[[f64; 4]])]) -> Outcome<()> {
	let src = res!(Dev::srgb());
	let ins = swatches();
	for (tag, intent, bpc, want) in sets {
		assert_eq!(want.len(), ins.len(), "{} {}: the fixture and the swatches differ in length", name, tag);
		let t = res!(Transform::new(&src, dst, *intent, *bpc));
		let (mut worst, mut at) = (0.0f64, 0);
		for (i, (c, w)) in ins.iter().zip(*want).enumerate() {
			let got = res!(t.rgb_to_cmyk(*c));
			for j in 0..4 {
				let e = (got[j] as f64 - w[j] / 100.0).abs();
				if e > worst {
					(worst, at) = (e, i);
					eprintln!("{} {}: swatch {} {:?} gives {:?}, LittleCMS {:?}", name, tag, i, c, got, w);
				}
			}
		}
		report(&format!("{} {}", name, tag), worst, at);
	}
	Ok(())
}

fn grey_set<F: Fn(&Transform, usize) -> Outcome<f32>>(
	src: &Dev,
	dst: &Dev,
	name: &str,
	n: usize,
	sets: &[(&str, Intent, bool, &[f64])],
	f: F,
) -> Outcome<()> {
	for (tag, intent, bpc, want) in sets {
		assert_eq!(want.len(), n, "{} {}: the fixture and the swatches differ in length", name, tag);
		let t = res!(Transform::new(src, dst, *intent, *bpc));
		let (mut worst, mut at) = (0.0f64, 0);
		for i in 0..n {
			let e = (res!(f(&t, i)) as f64 - want[i] / 255.0).abs();
			if e > worst {
				(worst, at) = (e, i);
			}
		}
		report(&format!("{} {}", name, tag), worst, at);
	}
	Ok(())
}

#[test]
fn test_fogra39l_rgb_to_cmyk_is_within_one_level_of_lcms_for_every_intent_and_compensation() -> Outcome<()> {
	let sets = seven!(
		data::FOGRA_T0, data::FOGRA_T0B, data::FOGRA_T1, data::FOGRA_T1B,
		data::FOGRA_T2, data::FOGRA_T2B, data::FOGRA_T3
	);
	rgb_to_cmyk_set(&res!(fogra()), "FOGRA39L", &sets)
}

#[test]
fn test_the_perceptual_black_with_compensation_is_the_black_lcms_gives() -> Outcome<()> {
	// sRGB black through FOGRA39L, perceptual, compensated and not; the first swatch of each set.
	let (src, dst) = (res!(Dev::srgb()), res!(fogra()));
	let t = res!(Transform::new(&src, &dst, Intent::Perceptual, true));
	let k = res!(t.rgb_to_cmyk([0.0; 3]));
	let want = data::FOGRA_T0B[0];
	for j in 0..4 {
		near(k[j] as f64, want[j] / 100.0, ONE, "compensated black");
	}
	let plain = res!(Transform::new(&src, &dst, Intent::Perceptual, false));
	let p = res!(plain.rgb_to_cmyk([0.0; 3]));
	let want = data::FOGRA_T0[0];
	for j in 0..4 {
		near(p[j] as f64, want[j] / 100.0, ONE, "uncompensated black");
	}
	// The two differ, so the fixture can tell compensation from none.
	assert!((0..4).any(|j| (k[j] - p[j]).abs() as f64 > 2.0 * ONE), "{:?} and {:?} should differ", k, p);
	Ok(())
}

fn near(got: f64, want: f64, tol: f64, what: &str) {
	assert!((got - want).abs() <= tol, "{}: got {}, want {} (tolerance {})", what, got, want, tol);
}

// Destination XYZ and source L* of the three intents, as `cmsDetectDestinationBlackPoint` and
// `cmsDetectBlackPoint` give them (called from python through ctypes).
fn black_points(p: &Profile, dst: [[f64; 3]; 2], dst_l: [f64; 2], src_l: [f64; 2]) {
	let (i0, i1) = (Intent::Perceptual, Intent::Relative);
	for (intent, xyz, l) in [(i0, dst[0], dst_l[0]), (i1, dst[1], dst_l[1]), (Intent::Saturation, dst[0], dst_l[0])] {
		let got = lab::cmyk_black_dst(p, intent);
		for j in 0..3 {
			near(got[j], xyz[j], 2e-5, &format!("{:?} destination black XYZ[{}]", intent, j));
		}
		near(lab::xyz_to_lab(got)[0], l, 0.03, &format!("{:?} destination black L*", intent));
	}
	for (intent, l) in [(i0, src_l[0]), (i1, src_l[1]), (Intent::Saturation, src_l[0])] {
		near(lab::xyz_to_lab(lab::cmyk_black_src(p, intent))[0], l, 0.03, &format!("{:?} source black L*", intent));
	}
}

#[test]
fn test_fogra39l_black_points_are_those_lcms_detects() -> Outcome<()> {
	let b = res!(fogra_bytes());
	let p = res!(Profile::read("FOGRA39L_coated.icc", &b));
	black_points(&p, [[0.010406, 0.010792, 0.008902], [0.010303, 0.010685, 0.008814]], [9.6346, 9.5496], [9.8192, 9.5496]);
	Ok(())
}

#[test]
#[ignore = "reads Ghostscript's AGPL profiles from /usr/share/color/icc/ghostscript"]
fn test_ghostscript_default_cmyk_black_points_are_those_lcms_detects() -> Outcome<()> {
	let p = res!(system(GS, "default_cmyk.icc"));
	black_points(&p, [[0.020524, 0.021286, 0.017559], [0.021208, 0.021995, 0.018144]], [16.1482, 16.5012], [11.7724, 16.5012]);
	Ok(())
}

#[test]
#[ignore = "reads Ghostscript's AGPL profiles from /usr/share/color/icc/ghostscript"]
fn test_ghostscript_swop_rgb_to_cmyk_is_within_one_level_of_lcms() -> Outcome<()> {
	let sets = seven!(
		data::SWOP_T0, data::SWOP_T0B, data::SWOP_T1, data::SWOP_T1B,
		data::SWOP_T2, data::SWOP_T2B, data::SWOP_T3
	);
	rgb_to_cmyk_set(&res!(gs("default_cmyk.icc")), "SWOP", &sets)
}

#[test]
#[ignore = "reads Ghostscript's AGPL profiles from /usr/share/color/icc/ghostscript"]
fn test_ghostscript_rgb_cmyk_and_grey_conversions_to_grey_and_from_grey_are_within_one_level_of_lcms() -> Outcome<()> {
	let (srgb, grey, cmyk) = (res!(Dev::srgb()), res!(gs("default_gray.icc")), res!(gs("default_cmyk.icc")));
	let fogra = res!(fogra());
	let rgb = swatches();
	let sets = seven!(
		data::RGB_GREY_T0, data::RGB_GREY_T0B, data::RGB_GREY_T1, data::RGB_GREY_T1B,
		data::RGB_GREY_T2, data::RGB_GREY_T2B, data::RGB_GREY_T3
	);
	res!(grey_set(&srgb, &grey, "RGB to grey", rgb.len(), &sets, |t, i| t.rgb_to_grey(rgb[i])));
	let ink = cmyks();
	let sets = seven!(
		data::CMYK_GREY_T0, data::CMYK_GREY_T0B, data::CMYK_GREY_T1, data::CMYK_GREY_T1B,
		data::CMYK_GREY_T2, data::CMYK_GREY_T2B, data::CMYK_GREY_T3
	);
	res!(grey_set(&cmyk, &grey, "CMYK to grey", ink.len(), &sets, |t, i| t.cmyk_to_grey(ink[i])));
	// Grey to FOGRA39L, the rich path: four values each, so held to the same level here.
	let lv = greys();
	let sets = seven!(
		data::GREY_FOGRA_T0, data::GREY_FOGRA_T0B, data::GREY_FOGRA_T1, data::GREY_FOGRA_T1B,
		data::GREY_FOGRA_T2, data::GREY_FOGRA_T2B, data::GREY_FOGRA_T3
	);
	for (tag, intent, bpc, want) in sets {
		let t = res!(Transform::new(&grey, &fogra, intent, bpc));
		let (mut worst, mut at) = (0.0f64, 0);
		for (i, g) in lv.iter().enumerate() {
			let got = res!(t.grey_to_cmyk(*g));
			for j in 0..4 {
				let e = (got[j] as f64 - want[i][j] / 100.0).abs();
				if e > worst {
					(worst, at) = (e, i);
				}
			}
		}
		report(&format!("grey to FOGRA39L {}", tag), worst, at);
	}
	Ok(())
}

#[test]
fn test_k_alone_is_one_minus_the_level() {
	assert_eq!(grey_to_k(0.0), [0.0, 0.0, 0.0, 1.0]);
	assert_eq!(grey_to_k(1.0), [0.0; 4]);
	assert_eq!(grey_to_k(0.25)[3], 0.75);
	assert_eq!(grey_to_k(3.0)[3], 0.0);
}

#[test]
fn test_the_grid_is_within_two_levels_of_the_exact_path() -> Outcome<()> {
	let (src, dst) = (res!(Dev::srgb()), res!(fogra()));
	let mut seed = 0x2545_f491_4f6c_dd1du64;
	let mut next = move || {
		seed ^= seed << 13;
		seed ^= seed >> 7;
		seed ^= seed << 17;
		(seed >> 11) as f64 / (1u64 << 53) as f64
	};
	let mut pts: Vec<[f64; 3]> = swatches();
	for r in 0..17 {
		for g in 0..17 {
			for b in 0..17 {
				pts.push([r as f64 / 16.0, g as f64 / 16.0, b as f64 / 16.0]);
			}
		}
	}
	for _ in 0..3000 {
		pts.push([next(), next(), next()]);
	}
	for (intent, bpc) in [(Intent::Perceptual, true), (Intent::Relative, false), (Intent::Absolute, false)] {
		let t = res!(Transform::new(&src, &dst, intent, bpc));
		let grid = res!(t.grid());
		assert!(std::ptr::eq(grid, res!(t.grid())), "the grid is built once");
		let (mut worst, mut at) = (0.0f64, [0.0; 3]);
		for p in &pts {
			let (e, x) = (grid.eval(*p), t.eval([p[0], p[1], p[2], 0.0]));
			for j in 0..4 {
				let d = (e[j] as f64 - x[j]).abs();
				if d > worst {
					(worst, at) = (d, *p);
				}
			}
		}
		eprintln!("grid {:?} {}: worst error {:.5} ({:.2} of 255) at {:?}", intent, bpc, worst, worst * 255.0, at);
		assert!(worst <= 2.0 * ONE, "the grid is {:.2} levels from the exact path at {:?}", worst * 255.0, at);
		// The bulk converter is the same arithmetic.
		let px = [0u8, 0, 0, 255, 255, 255, 12, 200, 99];
		let out = res!(grid.convert8(&px));
		assert_eq!(out.len(), 3);
		assert_eq!(out[2], grid.eval([12.0 / 255.0, 200.0 / 255.0, 99.0 / 255.0]));
		assert!(grid.convert8(&px[..8]).is_err());
	}
	Ok(())
}

#[test]
fn test_a_transform_is_identified_by_its_profiles_intent_and_compensation() -> Outcome<()> {
	let (srgb, fogra) = (res!(Dev::srgb()), res!(fogra()));
	let id = |s: &Dev, d: &Dev, i, b| Transform::new(s, d, i, b).map(|t| t.id());
	let base = res!(id(&srgb, &fogra, Intent::Relative, false));
	assert_eq!(base, res!(id(&srgb, &fogra, Intent::Relative, false)), "the same request, the same identity");
	assert_ne!(base, res!(id(&srgb, &fogra, Intent::Relative, true)));
	assert_ne!(base, res!(id(&srgb, &fogra, Intent::Perceptual, false)));
	assert_ne!(base, res!(id(&srgb, &fogra, Intent::Saturation, false)));
	assert_ne!(base, res!(id(&srgb, &fogra, Intent::Absolute, false)));
	assert_ne!(base, res!(id(&srgb, &Dev::sgray(), Intent::Relative, false)));
	assert_ne!(base, res!(id(&Dev::sgray(), &fogra, Intent::Relative, false)));
	// Absolute colorimetric is never compensated, so asking for it changes nothing.
	assert_eq!(
		res!(id(&srgb, &fogra, Intent::Absolute, false)),
		res!(id(&srgb, &fogra, Intent::Absolute, true)),
	);
	// A version 4 destination compensates perceptual whether asked or not.
	let (g, s) = (Dev::sgray(), &srgb);
	assert_eq!(res!(id(s, &g, Intent::Perceptual, false)), res!(id(s, &g, Intent::Perceptual, true)));
	assert!(res!(Transform::new(s, &g, Intent::Perceptual, false)).bpc());
	assert!(!res!(Transform::new(s, &fogra, Intent::Perceptual, false)).bpc());
	Ok(())
}

#[test]
fn test_a_transform_that_is_not_made_is_refused_by_name() -> Outcome<()> {
	let (srgb, fogra) = (res!(Dev::srgb()), res!(fogra()));
	let e = format!("{}", Transform::new(&fogra, &srgb, Intent::Relative, false).unwrap_err());
	assert!(e.contains("sRGB"), "{}", e);
	let t = res!(Transform::new(&srgb, &fogra, Intent::Relative, false));
	let e = format!("{}", t.cmyk_to_grey([0.0; 4]).unwrap_err());
	assert!(e.contains("sRGB") && e.contains("FOGRA39L"), "{}", e);
	assert!(t.grey_to_cmyk(0.5).is_err());
	assert!(res!(Transform::new(&Dev::sgray(), &fogra, Intent::Relative, false)).rgb_to_cmyk([0.0; 3]).is_err());
	Ok(())
}

// The reference inverse: bisection, as the curve did before it had closed forms.
fn bisect(c: &Curve, y: f64) -> f64 {
	let (mut lo, mut hi) = (0.0f64, 1.0f64);
	for _ in 0..60 {
		let mid = 0.5 * (lo + hi);
		if c.eval(mid) < y { lo = mid; } else { hi = mid; }
	}
	0.5 * (lo + hi)
}

#[test]
fn test_a_curves_inverse_agrees_with_bisection() {
	let table: Vec<u16> = (0..256).map(|i| ((i as f64 / 255.0).powf(1.8) * 65535.0).round() as u16).collect();
	let para = |func: u16, p: [f64; 7]| Curve::Para { func, p };
	// (curve, strictly rising)
	let curves = [
		(Curve::Identity, true),
		(Curve::Gamma(2.2), true),
		(Curve::Gamma(1.0 / 2.2), true),
		(Curve::Table(table), true),
		(Curve::Table(vec![0, 0, 100, 40000, 40000, 65535]), false),
		(para(0, [2.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]), true),
		(para(1, [2.0, 0.9, 0.1, 0.0, 0.0, 0.0, 0.0]), false),
		(para(2, [2.0, 0.9, 0.1, 0.05, 0.0, 0.0, 0.0]), false),
		(para(3, [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045, 0.0, 0.0]), true),
		(para(4, [2.2, 0.95, 0.05, 0.1, 0.1, 0.02, 0.0]), false),
	];
	for (k, (c, rising)) in curves.iter().enumerate() {
		for i in 0..=200 {
			let y = i as f64 / 200.0;
			let (x, r) = (c.inverse(y), bisect(c, y));
			assert!((0.0..=1.0).contains(&x), "curve {} y {}: inverse {} is outside 0..1", k, y, x);
			let (a, b) = (c.eval(x), c.eval(r));
			assert!((a - b).abs() < 1e-6, "curve {} y {}: eval(inverse) {} against bisection's {}", k, y, a, b);
			if *rising {
				assert!((x - r).abs() < 1e-6, "curve {} y {}: inverse {} against bisection's {}", k, y, x, r);
			}
		}
	}
}
