//! Hand a file we wrote to demuxers that know nothing about us, and check they find the track we
//! put in it.
//!
//! The unit tests in `mp4.rs` assert on the boxes the writer emitted -- their sizes, the run-length
//! coding of the time table, the offsets in the chunk table. That is a fair check of the arithmetic
//! and no check at all of whether the file plays: a sample entry with the wrong four-character
//! code, a duration in the wrong timescale, or a chunk offset measured from the start of `mdat`
//! rather than from the start of the file all produce perfectly well-formed boxes, and every one of
//! those tests still passes. The writer and its unit tests share a hand, so they share any
//! misreading of what the fields mean.
//!
//! # Where the samples come from
//!
//! Nothing here encodes anything, because the crate cannot. FFmpeg encodes a short clip to a raw
//! H.264 elementary stream; the test splits it into NAL units, groups them into access units,
//! builds the decoder configuration record out of the parameter sets it finds, and hands the whole
//! lot to the writer. So the samples are somebody else's, produced by a tool that has no idea this
//! crate exists, and no fixture is checked into the tree.
//!
//! That also makes the strongest check available cheap: the same coded pictures are decoded twice,
//! once out of FFmpeg's own elementary stream and once out of our container, and the raw planes
//! must be identical byte for byte. If the container mistimes, misorders, truncates or misplaces a
//! single sample, the two decodes differ.
//!
//! # The oracles
//!
//! - **FFprobe**, which reports the codec, the dimensions, the timescale, the sample count and the
//!   durations from the boxes alone.
//! - **FFmpeg**, decoding to raw planes, compared against the decode of the source stream.
//! - **ExifTool**, a wholly separate implementation in Perl, which walks the box tree itself.
//! - **GStreamer's `gst-discoverer-1.0`**, whose `qtdemux` shares no code with libavformat.
//! - **A box walker in Python**, which re-derives every sample's position from the tables and checks
//!   that the bytes at that position are the ones that were pushed.
//!
//! `mp4box` and `mediainfo` are not installed on this machine and `pymp4` is not importable, so
//! neither is used; ExifTool and GStreamer stand in their place, and both are independent of
//! FFmpeg. A missing tool is a failure here rather than a skip: an oracle that quietly does not run
//! is an oracle that is not there.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::mp4::{
	Codec,
	Sample,
	Track,
};

use std::{
	fs,
	path::PathBuf,
	process::Command,
};

// The clip FFmpeg is asked for: a test pattern, small enough to decode in a moment and large
// enough that a plane comparison means something.
const W: u16 = 64;
const H: u16 = 48;
const FPS: u32 = 10;
const FRAMES: usize = 10;

// The track's timescale, in ticks a second. Ninety thousand is the MPEG transport clock, and is
// chosen here because it is nothing like the frame rate: a writer that quietly used one where it
// meant the other would be caught by it.
const TIMESCALE: u32 = 90_000;
const TICKS: u32 = TIMESCALE / FPS;	// one sample's duration in that timescale

fn tmp(name: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

/// Fails unless the named program answers a version query, since an oracle that does not run is not
/// an oracle.
fn require(prog: &str) -> Outcome<()> {
	// Three spellings, because FFmpeg takes one, Python and ExifTool another, and GStreamer's
	// discoverer has no version flag at all and only answers a request for help.
	for flag in ["-version", "--version", "--help"] {
		let ok = Command::new(prog)
			.arg(flag)
			.output()
			.map(|o| o.status.success())
			.unwrap_or(false);
		if ok {
			return Ok(());
		}
	}
	Err(err!(
		"'{}' is not on the path, and the MP4 writer has no oracle without it. Install it rather \
		than skipping the test.", prog;
	Missing, Configuration))
}

/// Asks FFmpeg for a raw H.264 elementary stream, and gives its bytes.
///
/// No B-pictures, so decode order and display order agree and the container needs no composition
/// offset table; a keyframe every five frames, so the track has both sync and delta samples and the
/// sync sample table is exercised.
fn source(tag: &str) -> Outcome<(PathBuf, Vec<u8>)> {
	res!(require("ffmpeg"));
	let path = tmp(&fmt!("mp4_oracle_{}.264", tag));
	let out = res!(Command::new("ffmpeg")
		.args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
		.arg(fmt!("testsrc=size={}x{}:rate={}:duration=2", W, H, FPS))
		.args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
		.args(["-g", "5", "-bf", "0", "-frames:v"])
		.arg(fmt!("{}", FRAMES))
		.args(["-f", "h264"])
		.arg(&path)
		.output());
	if !out.status.success() {
		return Err(err!(
			"FFmpeg would not encode the source clip: {}", String::from_utf8_lossy(&out.stderr);
		Invalid, Input));
	}
	let buf = res!(fs::read(&path));
	Ok((path, buf))
}

/// Splits an Annex B elementary stream into its NAL units.
///
/// The units are separated by a three- or four-byte start code, and a unit may be followed by
/// padding zeroes which belong to neither it nor the next start code. Those are trimmed, which is
/// safe because the trailing bits of every NAL end in a set stop bit, so its last meaningful byte
/// is never zero.
fn nal_units(buf: &[u8]) -> Vec<&[u8]> {
	let mut starts = Vec::new();
	let mut i = 0usize;
	while i + 3 <= buf.len() {
		if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
			starts.push(i + 3);
			i += 3;
		} else {
			i += 1;
		}
	}
	let mut out = Vec::with_capacity(starts.len());
	for (k, s) in starts.iter().enumerate() {
		let mut e = if k + 1 < starts.len() { starts[k + 1] - 3 } else { buf.len() };
		while e > *s && buf[e - 1] == 0 {
			e -= 1;
		}
		out.push(&buf[*s..e]);
	}
	out
}

/// The parameter sets and the access units of an elementary stream.
///
/// An access unit is one coded picture. With one slice a picture and no field coding, every video
/// coding layer NAL unit -- types 1 to 5 -- begins one, and the non-video units before it belong to
/// it. Parameter sets and access unit delimiters are dropped from the samples, since the container
/// carries the parameter sets out of band in its decoder configuration record.
fn demux(stream: &[u8]) -> Outcome<(Vec<u8>, Vec<u8>, Vec<(Vec<u8>, bool)>)> {
	let mut sps: Option<Vec<u8>> = None;
	let mut pps: Option<Vec<u8>> = None;
	let mut units: Vec<(Vec<u8>, bool)> = Vec::new();
	let mut pending: Vec<Vec<u8>> = Vec::new();
	for nal in nal_units(stream) {
		if nal.is_empty() {
			continue;
		}
		let kind = nal[0] & 0x1F;
		match kind {
			7 => if sps.is_none() { sps = Some(nal.to_vec()); },
			8 => if pps.is_none() { pps = Some(nal.to_vec()); },
			9 => {},
			1..=5 => {
				let mut au = Vec::new();
				for p in pending.drain(..) {
					au.extend_from_slice(&(p.len() as u32).to_be_bytes());
					au.extend_from_slice(&p);
				}
				au.extend_from_slice(&(nal.len() as u32).to_be_bytes());
				au.extend_from_slice(nal);
				units.push((au, kind == 5));
			},
			_ => pending.push(nal.to_vec()),
		}
	}
	let sps = match sps {
		Some(v)	=> v,
		None	=> return Err(err!(
			"The encoded stream carries no sequence parameter set."; Invalid, Input, Missing)),
	};
	let pps = match pps {
		Some(v)	=> v,
		None	=> return Err(err!(
			"The encoded stream carries no picture parameter set."; Invalid, Input, Missing)),
	};
	Ok((sps, pps, units))
}

/// An `AVCDecoderConfigurationRecord` around one sequence and one picture parameter set, with a
/// four-byte NAL length, as ISO/IEC 14496-15 §5.3.3.1 lays it out.
fn avcc(sps: &[u8], pps: &[u8]) -> Vec<u8> {
	let mut rec = vec![1, sps[1], sps[2], sps[3], 0xFF, 0xE1];
	rec.extend_from_slice(&(sps.len() as u16).to_be_bytes());
	rec.extend_from_slice(sps);
	rec.push(1);
	rec.extend_from_slice(&(pps.len() as u16).to_be_bytes());
	rec.extend_from_slice(pps);
	rec
}

/// Writes the MP4 and gives its path, the path of the elementary stream it came from, and the
/// sample sizes and sync flags that went into it, so a reader's report can be checked against what
/// was pushed rather than against the file.
///
/// The tag names the pair of files, because the tests run at the same time and two of them writing
/// one path is a race that reads a half-written file.
fn fixture(tag: &str) -> Outcome<(PathBuf, PathBuf, Vec<usize>, Vec<bool>)> {
	let (src, stream) = res!(source(tag));
	let (sps, pps, units) = res!(demux(&stream));
	if units.len() != FRAMES {
		return Err(err!(
			"FFmpeg was asked for {} pictures and its stream demuxes to {} access units.",
			FRAMES, units.len();
		Invalid, Input, Mismatch));
	}

	// The dimensions given here are the ones the command line asked FFmpeg for, so the writer
	// accepting them is a check of its reading of the sequence parameter set against a third party.
	let mut track = res!(Track::new(W, H, TIMESCALE, Codec::Avc(avcc(&sps, &pps))));
	let mut sizes = Vec::with_capacity(units.len());
	let mut syncs = Vec::with_capacity(units.len());
	for (data, sync) in units {
		sizes.push(data.len());
		syncs.push(sync);
		// No composition offset: the source is encoded with no B-pictures, so decode order and
		// display order agree and each sample is shown at its decoding time.
		res!(track.push(Sample { data, dur: TICKS, sync, off: 0 }));
	}
	let buf = res!(track.finish());
	let path = tmp(&fmt!("mp4_oracle_{}.mp4", tag));
	res!(fs::write(&path, &buf));
	Ok((path, src, sizes, syncs))
}

/// The value FFprobe printed against a key in its JSON, as text with any quotes stripped.
///
/// A whole JSON parser is more than this needs: FFprobe's output is one key and one scalar a line,
/// and the keys wanted here appear once.
fn field(json: &str, key: &str) -> Outcome<String> {
	let needle = fmt!("\"{}\":", key);
	let at = match json.find(&needle) {
		Some(i)	=> i + needle.len(),
		None	=> return Err(err!(
			"FFprobe reported no '{}'.", key; Missing, Test)),
	};
	let rest = &json[at..];
	// To the end of the line rather than to the next comma: a value may hold commas of its own,
	// and `format_name` is a comma-separated list of every format FFmpeg's demuxer answers to.
	let end = rest.find('\n').unwrap_or(rest.len());
	Ok(rest[..end].trim().trim_end_matches(',').trim().trim_matches('"').to_string())
}

#[test]
fn test_ffprobe_reads_the_track_00() -> Outcome<()> {
	res!(require("ffprobe"));
	let (path, _, sizes, _) = res!(fixture("ffprobe"));

	let out = res!(Command::new("ffprobe")
		.args(["-v", "quiet", "-print_format", "json", "-show_streams", "-show_format"])
		.arg(&path)
		.output());
	if !out.status.success() {
		return Err(err!(
			"FFprobe refused the file: {}", String::from_utf8_lossy(&out.stderr);
		Invalid, Input));
	}
	let json = String::from_utf8_lossy(&out.stdout).to_string();

	// What the track is.
	req!(res!(field(&json, "codec_name")), "h264".to_string());
	req!(res!(field(&json, "codec_type")), "video".to_string());
	req!(res!(field(&json, "codec_tag_string")), "avc1".to_string());
	req!(res!(field(&json, "nb_streams")), "1".to_string());

	// How big its pictures are, which came out of the sample entry.
	req!(res!(field(&json, "width")), fmt!("{}", W));
	req!(res!(field(&json, "height")), fmt!("{}", H));
	req!(res!(field(&json, "coded_width")), fmt!("{}", W));

	// How it is timed. The time base is the media header's timescale; the total is the sum of the
	// sample durations in the decoding time table; the frame rate is what the two imply.
	req!(res!(field(&json, "time_base")), fmt!("1/{}", TIMESCALE));
	req!(res!(field(&json, "duration_ts")), fmt!("{}", TICKS as usize * FRAMES));
	req!(res!(field(&json, "r_frame_rate")), fmt!("{}/1", FPS));
	req!(res!(field(&json, "nb_frames")), fmt!("{}", FRAMES));

	// The container as a whole: the brand list of the file type box, and the movie header's
	// duration, which is the same span restated in milliseconds.
	let brands = res!(field(&json, "format_name"));
	req!(brands.contains("mp4"), true, "FFprobe does not call the file an MP4: {}", brands);
	let secs: f64 = match res!(field(&json, "duration")).parse::<f64>() {
		Ok(v)	=> v,
		Err(_)	=> return Err(err!(
			"FFprobe's duration is not a number."; Test, Invalid)),
	};
	let want = FRAMES as f64 / FPS as f64;
	let close = (secs - want).abs() < 0.002;
	req!(close, true, "FFprobe reports {} seconds where {} were written.", secs, want);

	// The size of the file is the size of the index plus the samples, and FFprobe read it.
	let bytes: usize = match res!(field(&json, "size")).parse::<usize>() {
		Ok(v)	=> v,
		Err(_)	=> return Err(err!("FFprobe's size is not a number."; Test, Invalid)),
	};
	let media: usize = sizes.iter().sum();
	let indexed = bytes > media;
	req!(indexed, true, "The file is {} bytes and its media alone is {}.", bytes, media);

	println!("FFprobe: h264 {}x{}, {} frames, {}/{} ticks, {} s, {} bytes.",
		W, H, FRAMES, TICKS as usize * FRAMES, TIMESCALE, secs, bytes);
	Ok(())
}

#[test]
fn test_ffmpeg_decodes_the_same_pixels_01() -> Outcome<()> {
	res!(require("ffmpeg"));
	let (path, src, _, _) = res!(fixture("decode"));

	// The same decoder, over the same coded pictures, out of two containers -- one FFmpeg's own
	// elementary stream and one ours. The planes are taken raw, in the decoder's native format, so
	// nothing between the decoder and the comparison can resample, retime or convert.
	let decode = |input: &PathBuf| -> Outcome<Vec<u8>> {
		let out = res!(Command::new("ffmpeg")
			.args(["-loglevel", "error", "-i"])
			.arg(input)
			.args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
			.output());
		if !out.status.success() {
			return Err(err!(
				"FFmpeg would not decode {}: {}",
				input.display(), String::from_utf8_lossy(&out.stderr);
			Invalid, Input));
		}
		Ok(out.stdout)
	};

	let want = res!(decode(&src));
	let got = res!(decode(&path));

	// A 4:2:0 frame is one luma sample and half a chroma sample a pixel.
	let frame = W as usize * H as usize * 3 / 2;
	req!(want.len(), frame * FRAMES, "The source stream did not decode to {} frames.", FRAMES);
	req!(got.len(), frame * FRAMES, "Our file decoded to {} frames.", got.len() / frame);

	if let Some(i) = want.iter().zip(got.iter()).position(|(a, b)| a != b) {
		return Err(err!(
			"The decode of our file differs from the decode of the source stream at byte {}, \
			which is frame {}: {} where {} was encoded.",
			i, i / frame, got[i], want[i];
		Invalid, Input, Mismatch));
	}
	println!("FFmpeg decodes {} frames of {}x{} identically out of both containers.",
		FRAMES, W, H);
	Ok(())
}

#[test]
fn test_exiftool_walks_the_boxes_02() -> Outcome<()> {
	res!(require("exiftool"));
	let (path, _, _, _) = res!(fixture("exiftool"));
	let out = res!(Command::new("exiftool").arg("-s").arg("-G").arg(&path).output());
	if !out.status.success() {
		return Err(err!(
			"ExifTool refused the file: {}", String::from_utf8_lossy(&out.stderr);
		Invalid, Input));
	}
	let text = String::from_utf8_lossy(&out.stdout).to_string();

	// ExifTool is a separate implementation in Perl which walks the box tree itself, so the fields
	// below were read out of the boxes rather than out of any FFmpeg parse of them.
	let wants = [
		("MajorBrand", "MP4 Base Media v1 [IS0 14496-12:2003]"),
		("HandlerType", "Video Track"),
		("CompressorID", "avc1"),
		("ImageWidth", "64"),
		("ImageHeight", "48"),
		("MediaTimeScale", "90000"),
		("MediaDuration", "1.00 s"),
		("VideoFrameRate", "10"),
		("Duration", "1.00 s"),
	];
	for (key, want) in wants {
		let line = text.lines()
			.find(|l| l.split_whitespace().nth(1) == Some(key))
			.map(|l| l.to_string());
		let line = match line {
			Some(l)	=> l,
			None	=> return Err(err!(
				"ExifTool reported no '{}'. It read:\n{}", key, text; Missing, Test)),
		};
		let got = match line.split_once(": ") {
			Some((_, v))	=> v.trim().to_string(),
			None		=> return Err(err!(
				"ExifTool's line for '{}' has no value: {}", key, line; Test)),
		};
		req!(got, want.to_string(), "ExifTool disagrees about '{}'.", key);
	}
	println!("ExifTool agrees on the brand, the handler, the codec, the size and the timing.");
	Ok(())
}

#[test]
fn test_gstreamer_demuxes_the_track_03() -> Outcome<()> {
	res!(require("gst-discoverer-1.0"));
	let (path, _, _, _) = res!(fixture("gst"));
	let out = res!(Command::new("gst-discoverer-1.0").arg(&path).output());
	let text = fmt!("{}{}",
		String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
	if !out.status.success() {
		return Err(err!(
			"GStreamer refused the file: {}", text; Invalid, Input));
	}

	// `qtdemux` shares no code with libavformat, so this is a second demuxer's reading of the same
	// box tree rather than a second run of the first.
	for want in ["H.264", "Width: 64", "Height: 48", "0:00:01.000000000"] {
		req!(text.contains(want), true,
			"GStreamer's report does not carry '{}'. It read:\n{}", want, text);
	}
	req!(text.contains("Missing plugins"), false, "GStreamer could not open the track:\n{}", text);
	println!("GStreamer's qtdemux finds an H.264 track of 64x48 lasting one second.");
	Ok(())
}

#[test]
fn test_a_box_walker_finds_every_sample_where_the_tables_say_04() -> Outcome<()> {
	res!(require("python3"));
	let (path, _, sizes, syncs) = res!(fixture("walker"));

	// A walker that knows only the specification: it descends the box tree, reads the sample size,
	// sample to chunk, chunk offset, decoding time and sync sample tables, and derives from them
	// where each sample begins and how long it lasts. Nothing in it came from the writer.
	let walk = r#"
import struct, sys

buf = open(sys.argv[1], 'rb').read()

def children(b, at, end):
    out = []
    while at + 8 <= end:
        size = struct.unpack('>I', b[at:at+4])[0]
        kind = b[at+4:at+8].decode('latin-1')
        if size == 1:
            size = struct.unpack('>Q', b[at+8:at+16])[0]
            body = at + 16
        else:
            body = at + 8
        if size < 8 or at + size > end:
            raise SystemExit('box %s at %d claims %d bytes' % (kind, at, size))
        out.append((kind, at, body, at + size))
        at += size
    if at != end:
        raise SystemExit('boxes do not tile: stopped at %d of %d' % (at, end))
    return out

CONTAINERS = {'moov', 'trak', 'mdia', 'minf', 'stbl'}
index = {}
def descend(at, end, trail):
    for kind, start, body, stop in children(buf, at, end):
        index.setdefault(trail + '/' + kind, (body, stop))
        if kind in CONTAINERS:
            descend(body, stop, trail + '/' + kind)

top = children(buf, 0, len(buf))
print('TOP', ','.join(k for k, _, _, _ in top))
descend(0, len(buf), '')

def body(pathname):
    if pathname not in index:
        raise SystemExit('no box at ' + pathname)
    return index[pathname]

# Sample sizes.
b0, b1 = body('/moov/trak/mdia/minf/stbl/stsz')
common, count = struct.unpack('>II', buf[b0+4:b0+12])
if common != 0:
    raise SystemExit('the sample size table declares a common size')
sizes = list(struct.unpack('>%dI' % count, buf[b0+12:b0+12+4*count]))

# Sample to chunk.
b0, b1 = body('/moov/trak/mdia/minf/stbl/stsc')
n = struct.unpack('>I', buf[b0+4:b0+8])[0]
stsc = [struct.unpack('>III', buf[b0+8+i*12:b0+20+i*12]) for i in range(n)]

# Chunk offsets.
if '/moov/trak/mdia/minf/stbl/stco' in index:
    b0, b1 = body('/moov/trak/mdia/minf/stbl/stco')
    nc = struct.unpack('>I', buf[b0+4:b0+8])[0]
    offs = list(struct.unpack('>%dI' % nc, buf[b0+8:b0+8+4*nc]))
else:
    b0, b1 = body('/moov/trak/mdia/minf/stbl/co64')
    nc = struct.unpack('>I', buf[b0+4:b0+8])[0]
    offs = list(struct.unpack('>%dQ' % nc, buf[b0+8:b0+8+8*nc]))

# Decoding times, expanded from their runs.
b0, b1 = body('/moov/trak/mdia/minf/stbl/stts')
n = struct.unpack('>I', buf[b0+4:b0+8])[0]
runs = [struct.unpack('>II', buf[b0+8+i*8:b0+16+i*8]) for i in range(n)]
durs = []
for c, d in runs:
    durs.extend([d] * c)

# Sync samples, or every sample where the table is absent.
key = '/moov/trak/mdia/minf/stbl/stss'
if key in index:
    b0, b1 = body(key)
    n = struct.unpack('>I', buf[b0+4:b0+8])[0]
    sync = list(struct.unpack('>%dI' % n, buf[b0+8:b0+8+4*n]))
else:
    sync = list(range(1, count + 1))

# Which chunk each sample is in, and therefore where it begins.
starts = []
s = 0
for i, (first, per, _desc) in enumerate(stsc):
    last = stsc[i+1][0] - 1 if i + 1 < len(stsc) else len(offs)
    for c in range(first, last + 1):
        at = offs[c-1]
        for _ in range(per):
            if s >= count:
                break
            starts.append(at)
            at += sizes[s]
            s += 1
if s != count:
    raise SystemExit('the chunk tables place %d of %d samples' % (s, count))

# Each sample's first four bytes are its first NAL unit's length, which must fit the sample.
for i, (at, size) in enumerate(zip(starts, sizes)):
    if at + size > len(buf):
        raise SystemExit('sample %d runs past the end of the file' % i)
    nal = struct.unpack('>I', buf[at:at+4])[0]
    if nal + 4 > size:
        raise SystemExit('sample %d holds a NAL of %d bytes in %d' % (i, nal, size))

mdat = [t for t in top if t[0] == 'mdat'][0]
if starts[0] != mdat[2]:
    raise SystemExit('the first sample is at %d and mdat begins at %d' % (starts[0], mdat[2]))
if starts[-1] + sizes[-1] != mdat[3]:
    raise SystemExit('the last sample ends at %d and mdat ends at %d'
        % (starts[-1] + sizes[-1], mdat[3]))

print('SIZES', ','.join(str(v) for v in sizes))
print('DURS', ','.join(str(v) for v in durs))
print('SYNC', ','.join(str(v) for v in sync))
print('OK')
"#;
	let out = res!(Command::new("python3").arg("-c").arg(walk).arg(&path).output());
	let text = String::from_utf8_lossy(&out.stdout).to_string();
	if !out.status.success() {
		return Err(err!(
			"The box walker rejected the file: {}{}",
			text, String::from_utf8_lossy(&out.stderr);
		Invalid, Input));
	}

	let line = |k: &str| -> Outcome<String> {
		match text.lines().find(|l| l.starts_with(k)) {
			Some(l)	=> Ok(l[k.len()..].trim().to_string()),
			None	=> Err(err!("The box walker printed no '{}' line: {}", k, text; Test)),
		}
	};

	req!(res!(line("TOP")), "ftyp,moov,mdat".to_string());
	req!(res!(line("SIZES")),
		sizes.iter().map(|n| fmt!("{}", n)).collect::<Vec<_>>().join(","));
	req!(res!(line("DURS")),
		(0..FRAMES).map(|_| fmt!("{}", TICKS)).collect::<Vec<_>>().join(","));
	req!(res!(line("SYNC")),
		syncs.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| fmt!("{}", i + 1))
			.collect::<Vec<_>>().join(","));
	req!(text.contains("OK"), true, "The box walker did not finish: {}", text);

	println!("The box walker places all {} samples from the tables alone.", FRAMES);
	Ok(())
}
