//! The ICC reader, the matrix path and the built-in spaces, against LittleCMS.
//!
//! Every expected number below came from `transicc` (LittleCMS 2.17, lcms2-utils) run once over the
//! swatch set by `tools/icc/fixtures.sh`, which is how to make them again. LittleCMS shares nothing
//! with this reader, so agreement is agreement with an independent implementation. The numbers are
//! printed to four places, in percent for XYZ and in the 0..255 scale for grey.
//!
//! The Ghostscript profiles are AGPL and are read from their system path only, by tests marked
//! `#[ignore]`; none is copied into the tree. Run them with `--ignored`. FOGRA39L Coated is bundled
//! (CC0) and its tests always run.
//!
//! The profiles refused are built here, byte by byte, so that what is wrong with each is on the page.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::icc::{
	builtin,
	matrix::{
		Grey,
		RgbMatrix,
		rgb_to_grey,
	},
	read::{
		Class,
		Curve,
		Lut,
		Profile,
		Sig,
		Space,
	},
};

use std::{
	fs,
	path::PathBuf,
};

const GS: &str = "/usr/share/color/icc/ghostscript";
const COLORD: &str = "/usr/share/color/icc/colord";

const CUBE_GREY: &[f64] = &[
	0.0000,
	10.2685,
	30.1323,
	49.7276,
	69.6342,
	53.9027,
	56.2335,
	63.0817,
	73.9222,
	87.9027,
	109.6109,
	110.6342,
	113.8638,
	119.5564,
	127.8677,
	164.4514,
	165.0700,
	167.0311,
	170.5914,
	175.9961,
	220.1634,
	220.5875,
	221.9377,
	224.4086,
	228.2179,
	27.7004,
	32.0895,
	43.3268,
	58.4747,
	75.8716,
	61.9844,
	64.0000,
	70.0000,
	79.7665,
	92.7160,
	113.3269,
	114.3152,
	117.4125,
	122.9066,
	130.9455,
	166.7004,
	167.3113,
	169.2412,
	172.7432,
	178.0545,
	221.7121,
	222.1323,
	223.4630,
	225.9183,
	229.6926,
	61.8949,
	63.9105,
	69.9222,
	79.7004,
	92.6615,
	82.1907,
	83.6498,
	88.1206,
	95.7626,
	106.4358,
	124.3502,
	125.2296,
	128.0000,
	132.9455,
	140.2763,
	173.6770,
	174.2529,
	176.0817,
	179.4124,
	184.4825,
	226.5798,
	226.9844,
	228.2879,
	230.6693,
	234.3463,
	95.5798,
	96.7977,
	100.5798,
	107.1595,
	116.5759,
	108.9144,
	109.9494,
	113.2023,
	118.9339,
	127.2879,
	142.0428,
	142.7860,
	145.1362,
	149.3813,
	155.7471,
	185.7276,
	186.2568,
	187.9339,
	190.9961,
	195.6848,
	235.2568,
	235.6420,
	236.8794,
	239.1517,
	242.6615,
	129.8054,
	130.6303,
	133.2646,
	137.9767,
	144.9883,
	139.2607,
	140.0272,
	142.4397,
	146.7821,
	153.2840,
	165.1907,
	165.8054,
	167.7626,
	171.2957,
	176.6731,
	202.8171,
	203.2879,
	204.7899,
	207.5292,
	211.7471,
	248.0817,
	248.4436,
	249.5953,
	251.7198,
	255.0000,
];
const RAMP_GREY: &[f64] = &[
	0.0000,
	14.9961,
	30.0000,
	45.0000,
	60.0000,
	74.9961,
	90.0000,
	105.0000,
	120.0039,
	135.0000,
	150.0000,
	165.0000,
	180.0000,
	195.0000,
	210.0000,
	225.0000,
	240.0000,
	255.0000,
];
const CORNER_XYZ: &[[f64; 3]] = &[
	[0.0000, 0.0000, 0.0000],
	[14.3046, 6.0610, 71.3913],
	[38.5113, 71.6905, 9.7067],
	[52.8159, 77.7515, 81.0980],
	[43.6041, 22.2485, 1.3920],
	[57.9087, 28.3095, 72.7833],
	[82.1154, 93.9390, 11.0987],
	[96.4200, 100.0000, 82.4900],
	[20.8133, 21.5861, 17.8063],
];
const RAMP_XYZ: &[[f64; 3]] = &[
	[0.0000, 0.0000, 0.0000],
	[0.4605, 0.4776, 0.3940],
	[1.2521, 1.2985, 1.0712],
	[2.5306, 2.6246, 2.1650],
	[4.3564, 4.5182, 3.7271],
	[6.7840, 7.0359, 5.8039],
	[9.8575, 10.2235, 8.4334],
	[13.6211, 14.1268, 11.6532],
	[18.1099, 18.7823, 15.4935],
	[23.3609, 24.2283, 19.9859],
	[29.4078, 30.4997, 25.1592],
	[36.2787, 37.6257, 31.0374],
	[44.0073, 45.6413, 37.6495],
	[52.6187, 54.5724, 45.0167],
	[62.1408, 64.4480, 53.1632],
	[72.5986, 75.2941, 62.1101],
	[84.0172, 87.1366, 71.8790],
	[96.4200, 100.0000, 82.4900],
];
const CUBE_GREY_GS: &[f64] = &[
	0.0000,
	10.2685,
	30.1323,
	49.7276,
	69.6342,
	53.9027,
	56.2335,
	63.0817,
	73.9222,
	87.9027,
	109.6070,
	110.6342,
	113.8560,
	119.5525,
	127.8599,
	164.4514,
	165.0700,
	167.0272,
	170.5875,
	175.9922,
	220.1595,
	220.5837,
	221.9339,
	224.4047,
	228.2140,
	27.7004,
	32.0895,
	43.3268,
	58.4747,
	75.8716,
	61.9844,
	64.0000,
	70.0000,
	79.7665,
	92.7160,
	113.3230,
	114.3113,
	117.4086,
	122.8988,
	130.9416,
	166.7004,
	167.3113,
	169.2374,
	172.7393,
	178.0545,
	221.7082,
	222.1245,
	223.4591,
	225.9105,
	229.6887,
	61.8949,
	63.9105,
	69.9222,
	79.7004,
	92.6615,
	82.1907,
	83.6498,
	88.1128,
	95.7549,
	106.4358,
	124.3463,
	125.2257,
	128.0000,
	132.9455,
	140.2763,
	173.6731,
	174.2490,
	176.0817,
	179.4124,
	184.4786,
	226.5759,
	226.9805,
	228.2802,
	230.6654,
	234.3424,
	95.5875,
	96.7977,
	100.5798,
	107.1595,
	116.5798,
	108.9144,
	109.9494,
	113.2023,
	118.9339,
	127.2879,
	142.0428,
	142.7821,
	145.1362,
	149.3813,
	155.7471,
	185.7276,
	186.2529,
	187.9339,
	190.9961,
	195.6809,
	235.2529,
	235.6381,
	236.8755,
	239.1479,
	242.6576,
	129.8054,
	130.6303,
	133.2646,
	137.9767,
	144.9883,
	139.2607,
	140.0272,
	142.4397,
	146.7821,
	153.2840,
	165.1907,
	165.8015,
	167.7587,
	171.2957,
	176.6731,
	202.8171,
	203.2840,
	204.7860,
	207.5292,
	211.7471,
	248.0778,
	248.4397,
	249.5914,
	251.7160,
	254.9961,
];
const CORNER_XYZ_GS: &[[f64; 3]] = &[
	[0.0000, 0.0000, 0.0000],
	[14.3066, 6.0608, 71.4096],
	[38.5147, 71.6873, 9.7076],
	[52.8213, 77.7481, 81.1173],
	[43.6066, 22.2488, 1.3916],
	[57.9132, 28.3096, 72.8012],
	[82.1213, 93.9362, 11.0992],
	[96.4279, 99.9969, 82.5088],
	[20.8144, 21.5848, 17.8099],
];
const FOGRA_LAB: &[[f64; 3]] = &[
	[100.0000, -0.0000, -0.0000],
	[17.4249, 0.0117, 0.6094],
	[93.7837, -4.6914, 97.8867],
	[17.0757, -3.1328, 12.5977],
	[50.9252, 77.4102, -1.7813],
	[11.5748, 14.5000, 1.9375],
	[49.8851, 71.3164, 50.9961],
	[12.5169, 8.9570, 8.0313],
	[58.1189, -39.7109, -50.4766],
	[11.5303, -8.6914, -10.0391],
	[52.9289, -67.7461, 29.1055],
	[12.4908, -13.3438, 4.1250],
	[25.6771, 22.5508, -46.9023],
	[8.9185, 5.9688, -5.5781],
	[24.7151, 0.1016, 0.6797],
	[9.8192, -0.0664, 2.6328],
];
const GS_CMYK_LAB: &[[f64; 3]] = &[
	[100.0000, -0.0000, -0.0000],
	[22.3529, 1.0703, 0.0586],
	[95.0812, -6.2969, 90.3516],
	[20.4856, -2.2578, 11.7109],
	[53.9537, 76.1406, -6.5625],
	[13.9767, 15.8789, -0.3906],
	[53.6045, 69.8125, 45.1953],
	[15.0398, 11.5742, 6.3789],
	[63.6106, -41.3945, -48.3359],
	[16.7754, -8.0195, -9.9844],
	[58.8848, -67.5430, 27.1289],
	[16.8413, -13.4687, 4.3789],
	[30.9191, 19.9883, -48.3633],
	[10.6265, 7.7031, -9.0977],
	[29.0119, 0.4844, -1.3555],
	[11.7724, 0.7656, 0.3281],
];
const GS_LAB_CMYK: &[[f64; 4]] = &[
	[100.0000, 94.1176, 0.0000, 10.5882],
	[74.9020, 67.8431, 65.4902, 90.1961],
	[87.8431, 98.4314, 0.0000, 14.5098],
	[56.4706, 80.3922, 62.3529, 76.0784],
	[100.0000, 0.0000, 0.0000, 0.0000],
	[62.3529, 0.0000, 100.0000, 0.0000],
	[6.6667, 34.9020, 0.0000, 0.0000],
	[0.0000, 100.0000, 81.1765, 0.0000],
];

// The first nine tags of FOGRA39L Coated are those every later reader needs; the rest are listed
// so that a change to the tag table is seen.
const FOGRA_TAGS: &[&[u8; 4]] = &[
	b"desc", b"cprt", b"wtpt", b"bkpt", b"A2B1", b"B2A1", b"A2B0", b"A2B2", b"B2A0", b"B2A2",
	b"gamt", b"arts", b"meta", b"dscm", b"dmnd", b"dmdd",
];

fn fogra_bytes() -> Outcome<Vec<u8>> {
	let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/icc/FOGRA39L_coated.icc");
	Ok(res!(fs::read(&p), IO, File))
}

fn system(dir: &str, name: &str) -> Outcome<Profile> {
	let p = PathBuf::from(dir).join(name);
	let b = res!(fs::read(&p), IO, File);
	Profile::read(name, &b)
}

fn sigs(p: &Profile) -> Vec<Sig> {
	p.tags.iter().map(|e| e.sig).collect()
}

fn near(got: f64, want: f64, tol: f64, what: &str) {
	assert!((got - want).abs() <= tol, "{}: got {}, want {} (tolerance {})", what, got, want, tol);
}

// Linear interpolation of a 16-bit table at x in 0..1, as a lookup table's curves are applied.
fn table(t: &[u16], x: f64) -> f64 {
	let at = x.clamp(0.0, 1.0) * (t.len() - 1) as f64;
	let i = (at.floor() as usize).min(t.len() - 2);
	let f = at - i as f64;
	(t[i] as f64 + (t[i + 1] as f64 - t[i] as f64) * f) / 65535.0
}

// A lattice node's values after the table's output curves, as fractions of full scale. The
// swatches sit on the lattice's corners, where the input curves return 0 and 1, which is asserted.
fn corner(lut: &Lut, idx: &[usize]) -> Outcome<Vec<f64>> {
	let top = lut.max();
	for c in 0..lut.nin {
		let t = &lut.ins[c * lut.ine..(c + 1) * lut.ine];
		assert_eq!((t[0] as f64, t[lut.ine - 1] as f64), (0.0, top), "input curve {}", c);
	}
	let node = res!(lut.node(idx));
	let mut out = Vec::new();
	for c in 0..lut.nout {
		let t = &lut.outs[c * lut.oute..(c + 1) * lut.oute];
		let t16: Vec<u16> = if lut.bits == 8 {
			t.iter().map(|v| v * 257).collect()
		} else {
			t.to_vec()
		};
		out.push(table(&t16, node[c] as f64 / top));
	}
	Ok(out)
}

// The index of corner `bits` (the first input slowest, as the fixtures count) of a lattice.
fn corner_idx(n: usize, grid: usize, k: usize) -> Vec<usize> {
	(0..n).map(|i| if (k >> (n - 1 - i)) & 1 == 1 { grid - 1 } else { 0 }).collect()
}

// A profile built byte by byte: the header, a tag table, and the tags laid end to end.
fn build(version: u32, class: &[u8; 4], space: &[u8; 4], pcs: &[u8; 4], tags: &[(Sig, Vec<u8>)]) -> Vec<u8> {
	let table_len = 4 + 12 * tags.len();
	let mut body = Vec::new();
	let mut dir = Vec::new();
	for (sig, data) in tags {
		let off = 128 + table_len + body.len();
		dir.extend_from_slice(sig);
		dir.extend_from_slice(&(off as u32).to_be_bytes());
		dir.extend_from_slice(&(data.len() as u32).to_be_bytes());
		body.extend_from_slice(data);
		while body.len() % 4 != 0 {
			body.push(0);
		}
	}
	let size = 128 + table_len + body.len();
	let mut b = vec![0u8; 128];
	b[0..4].copy_from_slice(&(size as u32).to_be_bytes());
	b[8..12].copy_from_slice(&version.to_be_bytes());
	b[12..16].copy_from_slice(class);
	b[16..20].copy_from_slice(space);
	b[20..24].copy_from_slice(pcs);
	b[36..40].copy_from_slice(b"acsp");
	for (i, v) in [0.964_202_88f64, 1.0, 0.824_905_4].iter().enumerate() {
		b[68 + 4 * i..72 + 4 * i].copy_from_slice(&((v * 65536.0).round() as i32).to_be_bytes());
	}
	b.extend_from_slice(&(tags.len() as u32).to_be_bytes());
	b.extend_from_slice(&dir);
	b.extend_from_slice(&body);
	b
}

fn s15(v: f64) -> [u8; 4] {
	((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_tag(v: [f64; 3]) -> Vec<u8> {
	let mut t = b"XYZ \0\0\0\0".to_vec();
	for x in v {
		t.extend_from_slice(&s15(x));
	}
	t
}

// The sRGB curve as the version 4 parametric curve of function type 3.
fn para_srgb() -> Vec<u8> {
	let mut t = b"para\0\0\0\0\0\x03\0\0".to_vec();
	for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
		t.extend_from_slice(&s15(v));
	}
	t
}

// A text description in the version 4 form, one English record.
fn mluc(text: &str) -> Vec<u8> {
	let units: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
	let mut t = b"mluc\0\0\0\0".to_vec();
	t.extend_from_slice(&1u32.to_be_bytes());
	t.extend_from_slice(&12u32.to_be_bytes());
	t.extend_from_slice(b"enUS");
	t.extend_from_slice(&(units.len() as u32).to_be_bytes());
	t.extend_from_slice(&28u32.to_be_bytes());
	t.extend_from_slice(&units);
	t
}

// A matrix profile of sRGB, version 4, the colorants those of sRGB adapted to D50.
fn srgb_v4() -> Vec<u8> {
	let cols = [[0.43607, 0.22249, 0.01392], [0.38515, 0.71687, 0.09708], [0.14307, 0.06061, 0.71410]];
	build(0x0420_0000, b"mntr", b"RGB ", b"XYZ ", &[
		(*b"desc", mluc("Hand-built sRGB")),
		(*b"rXYZ", xyz_tag(cols[0])),
		(*b"gXYZ", xyz_tag(cols[1])),
		(*b"bXYZ", xyz_tag(cols[2])),
		(*b"rTRC", para_srgb()),
		(*b"gTRC", para_srgb()),
		(*b"bTRC", para_srgb()),
	])
}

fn cube() -> Vec<[f64; 3]> {
	let mut v = Vec::new();
	for r in [0.0, 64.0, 128.0, 191.0, 255.0] {
		for g in [0.0, 64.0, 128.0, 191.0, 255.0] {
			for b in [0.0, 64.0, 128.0, 191.0, 255.0] {
				v.push([r / 255.0, g / 255.0, b / 255.0]);
			}
		}
	}
	v
}

fn corners() -> Vec<[f64; 3]> {
	let mut v = Vec::new();
	for r in [0.0, 1.0] {
		for g in [0.0, 1.0] {
			for b in [0.0, 1.0] {
				v.push([r, g, b]);
			}
		}
	}
	v.push([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0]);
	v
}

// RGB to grey within one level of eight bits.
fn check_grey(src: &RgbMatrix, dst: &Grey, cube_want: &[f64], ramp_want: &[f64], tag: &str) {
	let c = cube();
	assert_eq!(c.len(), cube_want.len());
	for (i, rgb) in c.iter().enumerate() {
		near(rgb_to_grey(src, dst, *rgb) * 255.0, cube_want[i], 1.0, &fmt!("{} cube {:?}", tag, rgb));
	}
	assert_eq!(ramp_want.len(), 18);
	for (i, want) in ramp_want.iter().enumerate() {
		let v = (i * 15) as f64 / 255.0;
		near(rgb_to_grey(src, dst, [v, v, v]) * 255.0, *want, 1.0, &fmt!("{} ramp {}", tag, i));
	}
}

fn check_xyz(src: &RgbMatrix, want: &[[f64; 3]], tol: f64, tag: &str) {
	let c = corners();
	assert_eq!(c.len(), want.len());
	for (i, rgb) in c.iter().enumerate() {
		let got = src.xyz(*rgb);
		for k in 0..3 {
			near(got[k] * 100.0, want[i][k], tol, &fmt!("{} corner {} xyz[{}]", tag, i, k));
		}
	}
}

#[test]
fn test_fogra39l_header_tags_and_description_read_as_lcms_reports_them() -> Outcome<()> {
	let p = res!(Profile::read("FOGRA39L_coated.icc", &res!(fogra_bytes())));
	assert_eq!((p.head.major, p.head.minor), (2, 2));
	assert_eq!(p.head.class, Class::Output);
	assert_eq!((p.head.space, p.head.pcs), (Space::Cmyk, Space::Lab));
	assert_eq!(p.head.intent, 1);
	assert_eq!(p.head.size as usize, res!(fogra_bytes()).len());
	near(p.head.illum[0], 0.9642, 1e-4, "illuminant X");
	assert_eq!(sigs(&p), FOGRA_TAGS.iter().map(|s| **s).collect::<Vec<Sig>>());
	assert_eq!(p.desc.as_deref(), Some("FOGRA39L Coated"));
	let w = res!(p.wtpt.ok_or_else(|| err!("No wtpt."; Missing)));
	// The media white point of a printing condition is the paper, not the D50 of the connection space.
	near(w[0], 0.8448, 1e-4, "white point X");
	near(w[1], 0.8763, 1e-4, "white point Y");
	near(w[2], 0.7462, 1e-4, "white point Z");
	Ok(())
}

#[test]
fn test_fogra39l_lookup_tables_have_the_shape_lcms_walks() -> Outcome<()> {
	let p = res!(Profile::read("FOGRA39L_coated.icc", &res!(fogra_bytes())));
	for i in 0..3 {
		let t = res!(p.a2b[i].as_ref().ok_or_else(|| err!("No A2B{}.", i; Missing)));
		assert_eq!((t.bits, t.nin, t.nout, t.grid, t.ine, t.oute), (16, 4, 3, 9, 1024, 1024));
		assert_eq!((t.ins.len(), t.clut.len(), t.outs.len()), (4 * 1024, 9 * 9 * 9 * 9 * 3, 3 * 1024));
	}
	for i in 0..3 {
		let t = res!(p.b2a[i].as_ref().ok_or_else(|| err!("No B2A{}.", i; Missing)));
		assert_eq!((t.bits, t.nin, t.nout, t.grid, t.ine, t.oute), (16, 3, 4, 17, 1024, 1024));
		assert_eq!(t.clut.len(), 17 * 17 * 17 * 4);
	}
	// No matrix path in a printer's profile.
	assert!(p.cols.iter().all(|c| c.is_none()) && p.ktrc.is_none());
	Ok(())
}

#[test]
fn test_fogra39l_corner_nodes_give_the_lab_lcms_gives() -> Outcome<()> {
	let p = res!(Profile::read("FOGRA39L_coated.icc", &res!(fogra_bytes())));
	let t = res!(p.a2b[1].as_ref().ok_or_else(|| err!("No A2B1."; Missing)));
	assert_eq!(FOGRA_LAB.len(), 16);
	for (k, want) in FOGRA_LAB.iter().enumerate() {
		let out = res!(corner(t, &corner_idx(4, t.grid, k)));
		let v = [out[0] * 65535.0, out[1] * 65535.0, out[2] * 65535.0];
		let lab = t.lab([v[0].round() as u16, v[1].round() as u16, v[2].round() as u16]);
		for c in 0..3 {
			near(lab[c], want[c], 0.01, &fmt!("FOGRA corner {} Lab[{}]", k, c));
		}
	}
	Ok(())
}

#[test]
fn test_a_truncated_profile_is_refused_by_name() -> Outcome<()> {
	let whole = res!(fogra_bytes());
	// Cut short with its length field left alone: the header says more than is there.
	let e = match Profile::read("cut.icc", &whole[..60_000]) {
		Ok(_) => return Err(err!("A profile cut to 60000 bytes was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("cut.icc") && e.contains("truncated") && e.contains("122152"), "{}", e);
	// Cut short with the length field made to agree: the tag table runs off the end, and says which tag.
	let mut short = whole[..60_000].to_vec();
	short[0..4].copy_from_slice(&60_000u32.to_be_bytes());
	let e = match Profile::read("cut.icc", &short) {
		Ok(_) => return Err(err!("A profile cut to 60000 bytes was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("'B2A1'") && e.contains("cut.icc") && e.contains("truncated"), "{}", e);
	// Not even a header.
	let e = match Profile::read("tiny.icc", &whole[..100]) {
		Ok(_) => return Err(err!("A profile of 100 bytes was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("tiny.icc") && e.contains("header"), "{}", e);
	// A tag cut short inside its own declared size: a lut16 whose table is shorter than its header says.
	let mut p = whole.clone();
	let at = 132 + 12 * 4; // A2B1 is the fifth entry; its size is the last word of the entry
	let size = u32::from_be_bytes([p[at + 8], p[at + 9], p[at + 10], p[at + 11]]);
	p[at + 8..at + 12].copy_from_slice(&(size - 4000).to_be_bytes());
	let e = match Profile::read("short_tag.icc", &p) {
		Ok(_) => return Err(err!("A table with its tail removed was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("'A2B1'") && e.contains("short_tag.icc") && e.contains("truncated"), "{}", e);
	Ok(())
}

#[test]
fn test_a_version_4_mab_table_is_refused_by_name() -> Outcome<()> {
	let mut mab = b"mAB \0\0\0\0".to_vec();
	mab.extend_from_slice(&[3, 3, 0, 0]);
	mab.resize(32, 0);
	let b = build(0x0420_0000, b"prtr", b"RGB ", b"Lab ", &[(*b"A2B0", mab)]);
	let e = match Profile::read("v4_mab.icc", &b) {
		Ok(_) => return Err(err!("A profile with an mAB table was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("'A2B0'") && e.contains("'mAB '") && e.contains("v4_mab.icc"), "{}", e);
	assert!(e.contains("No other profile is substituted"), "{}", e);
	// The same profile with no table in the tag position reads, so it is the table that is refused.
	let ok = build(0x0420_0000, b"prtr", b"RGB ", b"Lab ", &[(*b"cprt", b"text\0\0\0\0x\0".to_vec())]);
	let p = res!(Profile::read("v4_plain.icc", &ok));
	assert!(p.a2b.iter().all(|t| t.is_none()));
	// mBA is refused alike.
	let mut mba = b"mBA \0\0\0\0".to_vec();
	mba.resize(32, 0);
	let b = build(0x0420_0000, b"prtr", b"RGB ", b"Lab ", &[(*b"B2A1", mba)]);
	let e = match Profile::read("v4_mba.icc", &b) {
		Ok(_) => return Err(err!("A profile with an mBA table was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("'B2A1'") && e.contains("'mBA '"), "{}", e);
	Ok(())
}

#[test]
fn test_a_profile_not_made_for_the_reader_is_refused() -> Outcome<()> {
	// Not an ICC profile at all.
	let mut b = srgb_v4();
	b[36..40].copy_from_slice(b"nope");
	assert!(Profile::read("noticc.icc", &b).is_err());
	// A version this reader does not read.
	let b = build(0x0300_0000, b"mntr", b"RGB ", b"XYZ ", &[]);
	let e = match Profile::read("v3.icc", &b) {
		Ok(_) => return Err(err!("A version 3 profile was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("v3.icc") && e.contains("version 3"), "{}", e);
	// A lookup table whose channels do not fit its profile: an RGB profile holding a four-input table.
	let mut lut = b"mft2\0\0\0\0".to_vec();
	lut.extend_from_slice(&[4, 3, 2, 0]);
	for v in [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
		lut.extend_from_slice(&s15(v));
	}
	lut.extend_from_slice(&[0, 2, 0, 2]);
	lut.resize(52 + 2 * (4 * 2 + 16 * 3 + 3 * 2), 0);
	let b = build(0x0220_0000, b"prtr", b"RGB ", b"Lab ", &[(*b"A2B0", lut)]);
	let e = match Profile::read("mismatch.icc", &b) {
		Ok(_) => return Err(err!("A table of the wrong shape was read."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("'A2B0'") && e.contains("mismatch.icc"), "{}", e);
	Ok(())
}

#[test]
fn test_a_grey_profile_with_a_lab_connection_space_is_refused_by_name() -> Outcome<()> {
	// A kTRC of no entries is the identity; it is the connection space that the grey path cannot take.
	let curv = b"curv\0\0\0\0\0\0\0\0".to_vec();
	let b = build(0x0220_0000, b"mntr", b"GRAY", b"Lab ", &[(*b"kTRC", curv)]);
	let p = res!(Profile::read("labgrey.icc", &b));
	let e = match Grey::read(&p) {
		Ok(_) => return Err(err!("A grey profile with a Lab connection space was read as a grey curve."; Test, Bug)),
		Err(e) => fmt!("{}", e),
	};
	assert!(e.contains("labgrey.icc") && e.contains("Gray") && e.contains("Lab"), "{}", e);
	Ok(())
}

#[test]
fn test_a_hand_built_version_4_srgb_profile_matches_the_built_in_space() -> Outcome<()> {
	let p = res!(Profile::read("hand_srgb.icc", &srgb_v4()));
	assert_eq!(p.desc.as_deref(), Some("Hand-built sRGB"));
	assert_eq!((p.head.major, p.head.minor), (4, 2));
	assert!(matches!(p.trcs[0], Some(Curve::Para { func: 3, .. })));
	let m = res!(RgbMatrix::read(&p));
	let b = res!(builtin::srgb());
	for rgb in cube() {
		let (x, y) = (m.xyz(rgb), b.xyz(rgb));
		for k in 0..3 {
			near(x[k], y[k], 2e-4, &fmt!("hand-built vs built-in {:?} [{}]", rgb, k));
		}
	}
	Ok(())
}

#[test]
fn test_rgb_to_xyz_through_the_built_in_srgb_matches_lcms() -> Outcome<()> {
	check_xyz(&res!(builtin::srgb()), CORNER_XYZ, 5e-3, "built-in sRGB");
	Ok(())
}

#[test]
fn test_rgb_to_grey_through_the_built_in_spaces_is_within_one_level_of_lcms() -> Outcome<()> {
	check_grey(&res!(builtin::srgb()), &builtin::sgray(), CUBE_GREY, RAMP_GREY, "built-in");
	Ok(())
}

#[test]
fn test_grey_level_to_y_through_the_built_in_sgray_matches_lcms() {
	let g = builtin::sgray();
	for (i, want) in RAMP_XYZ.iter().enumerate() {
		near(g.y((i * 15) as f64 / 255.0) * 100.0, want[1], 0.05, &fmt!("sGray Y at {}", i * 15));
	}
}

#[test]
fn test_the_bundled_bytes_are_the_fogra39l_file() -> Outcome<()> {
	// Present only with the feature on; the build without it carries none of the bytes.
	#[cfg(feature = "fogra39")]
	{
		assert_eq!(oxedyne_fe2o3_graphics::icc::FOGRA39L, &res!(fogra_bytes())[..]);
		let p = res!(oxedyne_fe2o3_graphics::icc::fogra39l());
		assert_eq!(p.desc.as_deref(), Some("FOGRA39L Coated"));
	}
	Ok(())
}

#[test]
#[ignore = "reads Ghostscript's AGPL profiles from /usr/share/color/icc/ghostscript"]
fn test_ghostscript_default_rgb_reads_and_converts_as_lcms_does() -> Outcome<()> {
	let p = res!(system(GS, "default_rgb.icc"));
	assert_eq!((p.head.major, p.head.minor), (2, 1));
	assert_eq!(p.head.class, Class::Display);
	assert_eq!((p.head.space, p.head.pcs), (Space::Rgb, Space::Xyz));
	assert_eq!(sigs(&p), [b"desc", b"cprt", b"wtpt", b"bkpt", b"rXYZ", b"gXYZ", b"bXYZ", b"rTRC", b"gTRC", b"bTRC"].iter().map(|s| **s).collect::<Vec<Sig>>());
	assert_eq!(p.desc.as_deref(), Some("Artifex Software sRGB ICC Profile"));
	assert!(matches!(&p.trcs[0], Some(Curve::Table(t)) if t.len() == 1024));
	let m = res!(RgbMatrix::read(&p));
	check_xyz(&m, CORNER_XYZ_GS, 5e-3, "default_rgb");
	let g = res!(Grey::read(&res!(system(GS, "default_gray.icc"))));
	check_grey(&m, &g, CUBE_GREY_GS, RAMP_GREY, "default_rgb to default_gray");
	Ok(())
}

#[test]
#[ignore = "reads Ghostscript's AGPL profiles from /usr/share/color/icc/ghostscript"]
fn test_ghostscript_default_gray_reads_as_lcms_reports_it() -> Outcome<()> {
	let p = res!(system(GS, "default_gray.icc"));
	assert_eq!(p.head.class, Class::Display);
	assert_eq!((p.head.space, p.head.pcs), (Space::Gray, Space::Xyz));
	assert_eq!(sigs(&p), [b"desc", b"cprt", b"wtpt", b"bkpt", b"kTRC"].iter().map(|s| **s).collect::<Vec<Sig>>());
	assert_eq!(p.desc.as_deref(), Some("Artifex Software sGray ICC Profile"));
	let g = res!(Grey::read(&p));
	for (i, want) in RAMP_XYZ.iter().enumerate() {
		near(g.y((i * 15) as f64 / 255.0) * 100.0, want[1], 5e-3, &fmt!("default_gray Y at {}", i * 15));
	}
	Ok(())
}

#[test]
#[ignore = "reads Ghostscript's AGPL profiles from /usr/share/color/icc/ghostscript"]
fn test_ghostscript_default_cmyk_reads_both_lut_widths_as_lcms_does() -> Outcome<()> {
	let p = res!(system(GS, "default_cmyk.icc"));
	assert_eq!((p.head.major, p.head.minor), (2, 1));
	assert_eq!(p.head.class, Class::Output);
	assert_eq!((p.head.space, p.head.pcs), (Space::Cmyk, Space::Lab));
	assert_eq!(sigs(&p), [b"desc", b"cprt", b"wtpt", b"A2B0", b"B2A0", b"A2B1", b"B2A1", b"A2B2", b"B2A2"].iter().map(|s| **s).collect::<Vec<Sig>>());
	assert_eq!(p.desc.as_deref(), Some("Artifex CMYK SWOP Profile"));
	// A2B0 is a lut16, B2A0 a lut8. The three intents share one table, so the relative colorimetric
	// fixtures (no black point compensation) are the nodes of A2B0 and B2A0 too.
	for i in 1..3 {
		let (a0, ai) = (res!(p.a2b[0].as_ref().ok_or_else(|| err!("No A2B0."; Missing))), res!(p.a2b[i].as_ref().ok_or_else(|| err!("No A2B{}.", i; Missing))));
		assert_eq!((&a0.clut, &a0.ins, &a0.outs), (&ai.clut, &ai.ins, &ai.outs));
	}
	let a = res!(p.a2b[0].as_ref().ok_or_else(|| err!("No A2B0."; Missing)));
	assert_eq!((a.bits, a.nin, a.nout, a.grid, a.ine, a.oute), (16, 4, 3, 9, 256, 2));
	let b = res!(p.b2a[0].as_ref().ok_or_else(|| err!("No B2A0."; Missing)));
	assert_eq!((b.bits, b.nin, b.nout, b.grid, b.ine, b.oute), (8, 3, 4, 33, 256, 256));
	assert_eq!(b.clut.len(), 33 * 33 * 33 * 4);
	for (k, want) in GS_CMYK_LAB.iter().enumerate() {
		let out = res!(corner(a, &corner_idx(4, a.grid, k)));
		let v = [out[0] * 65535.0, out[1] * 65535.0, out[2] * 65535.0];
		let lab = a.lab([v[0].round() as u16, v[1].round() as u16, v[2].round() as u16]);
		for c in 0..3 {
			near(lab[c], want[c], 0.01, &fmt!("default_cmyk A2B0 corner {} Lab[{}]", k, c));
		}
	}
	for (k, want) in GS_LAB_CMYK.iter().enumerate() {
		let out = res!(corner(b, &corner_idx(3, b.grid, k)));
		for c in 0..4 {
			near(out[c] * 100.0, want[c], 0.05, &fmt!("default_cmyk B2A0 corner {} [{}]", k, c));
		}
	}
	Ok(())
}

#[test]
#[ignore = "reads colord's profiles from /usr/share/color/icc/colord"]
fn test_colord_srgb_v4_reads_its_parametric_curves() -> Outcome<()> {
	let p = res!(system(COLORD, "sRGB.icc"));
	assert_eq!(p.head.major, 4);
	assert!(p.chad.is_some() && p.wtpt.is_some());
	assert!(matches!(p.trcs[1], Some(Curve::Para { .. })));
	assert!(p.desc.is_some());
	let m = res!(RgbMatrix::read(&p));
	let b = res!(builtin::srgb());
	for rgb in cube() {
		let (x, y) = (m.xyz(rgb), b.xyz(rgb));
		for k in 0..3 {
			near(x[k], y[k], 2e-3, &fmt!("colord sRGB vs built-in {:?} [{}]", rgb, k));
		}
	}
	Ok(())
}
