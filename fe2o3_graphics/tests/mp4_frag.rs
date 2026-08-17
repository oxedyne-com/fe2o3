//! Does a film written as fragments keep its sound against its picture?
//!
//! A whole-file MP4 states its timing once, in a sample table the writer builds
//! with every sample in hand. A fragmented one states it over and over: the
//! `moov` at the front carries no samples at all, and each `moof` after it says
//! where its run begins on each track's timeline and how long every sample in it
//! lasts. Nothing in the file cross-checks a fragment against the one before it,
//! and no decoder ever complains about the arithmetic, so a writer that loses a
//! tick a fragment on one track produces a film that opens, reports the right
//! shape, decodes every frame -- and walks its sound away from its picture. That
//! is the characteristic failure of a bad remux and the reason this test exists.
//!
//! So the film is written by calling `head` once and `next` several times, the
//! parts are concatenated, and a **different program** is asked five questions
//! in order, each a separate named failure:
//!
//! 1. **Shape.** Two streams; the picture h264 at the source's size; the sound
//!    aac; and the packet counts equal to what was handed to the writer.
//! 2. **Decode.** `ffmpeg -v error` says nothing at all. A container written
//!    wrongly still opens and still counts right, and only a decode settles
//!    whether the bytes in the `mdat` were the frames the `moof` described.
//! 3. **When the pictures are shown.** The written times are read back and held
//!    against the source's, allowing exactly ONE constant difference across all
//!    of them: the whole track may be delayed -- that is how a negative
//!    composition offset is avoided -- but the intervals may not change.
//! 4. **When the sounds are heard.** The same check on `a:0`, and the one that
//!    matters most, because sound drifting away from picture is invisible to
//!    checks 1 and 2 and is exactly what a viewer notices first.
//! 5. **The two tracks against each other.** Both may be delayed; they may not
//!    be delayed by *different* amounts, which is a film whose sound is out and
//!    which every check above passes happily.
//!
//! # Proving the checks
//!
//! A check is only proved by a break it catches that everything before it
//! misses. A break that dies at the decode has said nothing about the timing
//! comparison that would have run after it. So each of these is chosen to
//! survive every earlier question and fail exactly one. They are applied by
//! `BREAK_FRAG=<name>` to the samples handed to the writer, leaving what the
//! test expects untouched, and an unknown name is an error rather than a quiet
//! nothing.
//!
//! - `picture-one-tick` **proves check 3.** One picture sample's composition
//!   offset, in the middle of a fragment, is raised by a single tick. The
//!   sample count does not move, so check 1 passes. A millisecond on a picture
//!   shown forty milliseconds from its neighbours reorders nothing and leaves
//!   every offset non-negative, so ffmpeg decodes it without a word and check 2
//!   passes. The picture times then need two constants where one is allowed,
//!   and check 3 fires. The sound is untouched and the moved sample is not a
//!   fragment's first, so checks 4 and 5 would have seen nothing.
//! - `sound-one-tick` **proves check 4.** The same single tick, on one sound
//!   sample's composition offset in the middle of a fragment. Counts, decode
//!   and the whole picture track are as they were, and the moved sample is not
//!   a fragment's first, so check 5 is left with nothing to find either.
//! - `sound-one-tick-dur` is the fallback for check 4 where a writer drops
//!   composition offsets on a sound track -- which is itself a defect worth
//!   knowing about. It raises one sound sample's *duration* by a tick, which
//!   moves every sample after it and therefore disturbs check 5 at the later
//!   fragments as well. Check 4 still fires first, but this break proves less
//!   than the one above and is here only so a stuck run has a way forward.
//! - `sound-delayed` **proves check 5.** Every sound sample's composition
//!   offset is raised by [`DELAY`] milliseconds. The sound track is then
//!   internally perfect: the same intervals, one constant away from the source,
//!   which is precisely what check 4 permits. Counts and decode are untouched.
//!   Only check 5, which holds the two tracks against one another, can see that
//!   the sound now sits a fifth of a second from where the picture puts it.
//!
//! One caveat, for whoever changes the writer: these comparisons read the
//! packet times ffprobe reports, and those move with an edit list. The writer
//! today writes none, so a track's times are its media times and checks 3 to 5
//! are exact. If a compensating `elst` is ever added to one track and not the
//! other, check 5 will fail on a film that is in fact correct, and it must be
//! told about the compensation rather than loosened.
//!
//! Point `MKV_CORPUS` at a directory of films. Output goes under
//! `~/.cache/ochre-remux-probe`, **never `/tmp`**, which is a tmpfs here and is
//! charged to the memory budget of whoever writes to it.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_graphics::{
	matroska::{Clusters, Matroska, TrackKind},
	mp4::{Codec, Fragments, Media, Sample, Stream, composition_offsets},
};

use std::{
	env,
	fs::{self, File},
	io::Read,
	path::{Path, PathBuf},
	process::Command,
};

const HEAD: usize = 1024 * 1024;	// read before the clusters, to find the tracks
const WINDOW: usize = 256 * 1024;	// the window the frame reader is held to
const FRAMES: usize = 300;		// pictures repackaged, deciding how long the written film is

// How many fragments the film is cut into: more than a handful, because one
// fragment carrying everything would exercise none of the bookkeeping that runs
// between them, which is the whole subject.
const FRAGS: usize = 6;
const MIN_FRAGS: usize = 4;	// fewer and fragmentation is not being tested, so the film is skipped

// The timescale both tracks are written in, in ticks a second. Milliseconds,
// matching the unit Matroska stamps its frames in, so no time is rescaled and no
// rounding is introduced anywhere between the source and the comparison. A sound
// track is more usually written on its sampling rate, and a remuxer in earnest
// should be; here the point is to compare times exactly, and a rate of 44100
// does not divide a millisecond.
const SCALE: u32 = 1000;

// How far sound-delayed moves the sound, in ticks -- glaring to a viewer and
// invisible to every check but the last.
const DELAY: i32 = 200;

#[test]
fn a_fragmented_film_carries_both_streams() -> Outcome<()> {
	let dir = match env::var("MKV_CORPUS") {
		Ok(d) => PathBuf::from(d),
		Err(_) => {
			println!("MKV_CORPUS is not set, so nothing was fragmented and this \
				test proves nothing.");
			return Ok(());
		},
	};
	if Command::new("ffprobe").arg("-version").output().is_err() {
		println!("ffprobe is not installed, so there is no oracle and this test \
			proves nothing.");
		return Ok(());
	}
	if Command::new("ffmpeg").arg("-version").output().is_err() {
		println!("ffmpeg is not installed, so nothing can be decoded and this \
			test proves nothing.");
		return Ok(());
	}

	let out = match env::var("HOME") {
		Ok(h) => PathBuf::from(h).join(".cache/ochre-remux-probe"),
		Err(_) => return Err(err!("HOME is not set, so there is nowhere to \
			write the film."; Invalid, Init)),
	};
	res!(fs::create_dir_all(&out));

	let mut films = Vec::new();
	res!(gather(&dir, &mut films));
	films.sort();

	let want = num_from_env("MKV_FRAMES", FRAMES);
	let frags = num_from_env("MKV_FRAGS", FRAGS).max(MIN_FRAGS);
	let mut done = 0usize;
	let mut skipped = 0usize;
	// What was actually held against something, so that a run which compared
	// nothing cannot read as a run that passed.
	let mut compared = 0usize;
	let mut joins = 0usize;

	for film in films.iter() {
		if done >= num_from_env("MKV_FILES", 2) {
			break;
		}
		let made = match repackage(film, want, frags) {
			Ok(Some(v)) => v,
			// A film without both an h264 picture and an aac sound is not what
			// this test is about, and is skipped by name rather than failed.
			Ok(None) => { skipped += 1; continue },
			Err(e) => {
				println!("{}: could not be fragmented: {}", film.display(), e);
				skipped += 1;
				continue;
			},
		};
		let path = out.join(format!("frag-{}.mp4", done));
		res!(fs::write(&path, &made.bytes));

		// 1. Does another program agree it is a film of the right shape?
		let streams = res!(stream_count(&path));
		if streams != 2 {
			return Err(err!(
				"{}: two streams were written and ffprobe finds {} in the film.",
				film.display(), streams; Invalid, Mismatch));
		}
		let pic = res!(probe_fields(&path, "v:0",
			"stream=codec_name,width,height,nb_read_packets"));
		let name = res!(pick(&pic, "codec_name"));
		if name != "h264" {
			return Err(err!(
				"{}: the written film's picture says it holds {} rather than \
				h264.", film.display(), name; Invalid, Mismatch));
		}
		let w = res!(number(&pic, "width"));
		let h = res!(number(&pic, "height"));
		if w != made.w as i64 || h != made.h as i64 {
			return Err(err!(
				"{}: the written film is {} by {} and the source is {} by {}.",
				film.display(), w, h, made.w, made.h; Invalid, Mismatch));
		}
		let n = res!(number(&pic, "nb_read_packets"));
		if n != made.vid.len() as i64 {
			return Err(err!(
				"{}: {} pictures were written over {} fragments and ffprobe \
				counts {}.",
				film.display(), made.vid.len(), made.starts.len(), n;
				Invalid, Mismatch));
		}
		let snd = res!(probe_fields(&path, "a:0",
			"stream=codec_name,channels,nb_read_packets"));
		let name = res!(pick(&snd, "codec_name"));
		if name != "aac" {
			return Err(err!(
				"{}: the written film's sound says it holds {} rather than aac.",
				film.display(), name; Invalid, Mismatch));
		}
		let chans = res!(number(&snd, "channels"));
		if chans != made.chans as i64 {
			return Err(err!(
				"{}: {} channels of sound were written and ffprobe finds {}.",
				film.display(), made.chans, chans; Invalid, Mismatch));
		}
		let n = res!(number(&snd, "nb_read_packets"));
		if n != made.aud.len() as i64 {
			return Err(err!(
				"{}: {} sounds were written over {} fragments and ffprobe counts \
				{}.", film.display(), made.aud.len(), made.starts.len(), n;
				Invalid, Mismatch));
		}

		// 2. Does it actually decode, both streams of it, without a murmur?
		res!(decodes_silently(&path, film));

		// 3. Are the pictures shown when the source said they were?
		let back = res!(times_of(&path, "v:0", film));
		if back.len() != made.vid.len() {
			return Err(err!(
				"{}: {} picture times were written and {} read back.",
				film.display(), made.vid.len(), back.len(); Invalid, Mismatch));
		}
		let vshift = back[0] - made.vid[0];
		for i in 0..back.len() {
			if back[i] - made.vid[i] != vshift {
				return Err(err!(
					"{}: picture {} is shown at {} and was written at {}, a \
					difference of {} against the film's {}. The pictures are not \
					where they were put.",
					film.display(), i, back[i], made.vid[i],
					back[i] - made.vid[i], vshift; Invalid, Mismatch));
			}
			compared += 1;
		}

		// 4. Is the sound heard when the source said it was?
		//
		// The reason the test exists. A sound track's timing is bookkeeping and
		// nothing else -- there is no reordering in it and no offset any decoder
		// would object to -- so an error here passes every question above and
		// arrives as a film whose voices do not match its mouths.
		let backa = res!(times_of(&path, "a:0", film));
		if backa.len() != made.aud.len() {
			return Err(err!(
				"{}: {} sound times were written and {} read back.",
				film.display(), made.aud.len(), backa.len(); Invalid, Mismatch));
		}
		let ashift = backa[0] - made.aud[0];
		for j in 0..backa.len() {
			if backa[j] - made.aud[j] != ashift {
				return Err(err!(
					"{}: sound {} is heard at {} and was written at {}, a \
					difference of {} against the track's {}. The sound has \
					drifted from where it was put, by {} ticks so far.",
					film.display(), j, backa[j], made.aud[j],
					backa[j] - made.aud[j], ashift,
					(backa[j] - made.aud[j]) - ashift; Invalid, Mismatch));
			}
			compared += 1;
		}

		// 5. Do the two tracks stay in step with each other?
		//
		// Checks 3 and 4 each allow the track they look at to be delayed as a
		// whole, and separately they are right to: a delay is how a negative
		// offset is avoided. Together they let both tracks be delayed by
		// DIFFERENT amounts, which is a film whose sound is out and which
		// nothing above can see. So the two delays are held against each other,
		// at every fragment, so that a failure names where the tracks parted.
		for (k, (vi, ai)) in made.starts.iter().enumerate() {
			let gap = (back[*vi] - made.vid[*vi]) - (backa[*ai] - made.aud[*ai]);
			if gap != 0 {
				return Err(err!(
					"{}: at fragment {} the sound sits {} ticks from where the \
					picture puts it. The picture is delayed {} ticks and the \
					sound {}, and a film's two tracks must be delayed by the \
					same amount or not at all.",
					film.display(), k, gap, back[*vi] - made.vid[*vi],
					backa[*ai] - made.aud[*ai]; Invalid, Mismatch));
			}
			joins += 1;
		}

		println!("{}: {} pictures and {} sounds over {} fragments, all decode, \
			both tracks where they were put (picture delayed {} ticks, sound \
			starting {} ticks after it)",
			film.display(), made.vid.len(), made.aud.len(), made.starts.len(),
			made.shift, made.lead);
		done += 1;
	}

	println!("{} films found; {} fragmented, {} skipped; {} times compared and \
		{} fragment joins checked",
		films.len(), done, skipped, compared, joins);
	if done == 0 {
		return Err(err!(
			"{} films were found and not one held both an h264 picture and an \
			aac sound that could be fragmented, so this run proves nothing.",
			films.len(); Invalid, Mismatch));
	}
	if compared == 0 || joins == 0 {
		return Err(err!(
			"{} films were fragmented and no time was compared, so this run \
			proves nothing.", done; Invalid, Mismatch));
	}
	Ok(())
}

// --------------------------------------------------------------- the writing

/// A fragmented film written out, and everything the checks hold it against.
struct Made {
	bytes:	Vec<u8>,
	w:		u16,
	h:		u16,
	chans:	u16,			// channels of sound, as the source states them
	vid:	Vec<i64>,		// picture times written, in decode order, on its own timeline
	aud:	Vec<i64>,		// the same for the sound, on the sound track's own timeline
	starts:	Vec<(usize, usize)>,	// each fragment's first picture and first sound sample
	shift:	i64,			// how far the picture track is delayed by its offsets
	// How far the first sound sample sits after the first picture sample on the
	// source's clock. The one misalignment this test introduces itself, and it is
	// under a frame of sound: each track begins at its own decode time nought, and
	// the sound frame nearest the first picture is rarely on the same instant. It
	// is carried here because the expectations are built with it and check 5
	// therefore holds regardless of it.
	lead:	i64,
}

/// Reads a film's picture and sound frames and writes them as a fragmented MP4,
/// decoding nothing.
///
/// `None` where the film does not hold both an h264 picture and an aac sound
/// with the configuration records a repackaging needs, which is not a defect in
/// the writer and is skipped rather than failed.
fn repackage(path: &Path, want: usize, frags: usize) -> Outcome<Option<Made>> {
	let mut head = vec![0u8; HEAD];
	let mut file = res!(File::open(path));
	let n = res!(file.read(&mut head));
	head.truncate(n);
	let mkv = res!(Matroska::read(&head));

	// The picture. Its configuration record moves across verbatim -- this is the
	// whole trick -- because the `avcC` an MP4 sample entry wants and the
	// `CodecPrivate` a Matroska track entry carries are the same bytes.
	let vt = match mkv.tracks().iter().find(|t| t.kind() == Some(TrackKind::Video)) {
		Some(t) => t,
		None => return Ok(None),
	};
	if vt.codec() != "V_MPEG4/ISO/AVC" || vt.private().is_empty() {
		return Ok(None);
	}
	let (w, h) = vt.size();
	if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
		return Ok(None);
	}

	// The sound. The same move: `CodecPrivate` for `A_AAC` is the
	// `AudioSpecificConfig` an `esds` states, and a film that carries none of it
	// would need the configuration derived, which is not this test's subject.
	let at = match mkv.tracks().iter().find(|t| {
		t.kind() == Some(TrackKind::Audio) && t.codec().starts_with("A_AAC")
	}) {
		Some(t) => t,
		None => return Ok(None),
	};
	if at.private().is_empty() {
		return Ok(None);
	}
	let chans = at.channels();
	let rate = at.rate();
	if chans == 0 || chans > 8 || !rate.is_finite() || rate < 8000.0 || rate > 192_000.0 {
		return Ok(None);
	}
	let chans = chans as u16;
	let rate = rate.round() as u32;

	// One pass over the clusters, taking both streams as they interleave. The
	// sound is taken only while the picture is still wanted, so the two spans
	// end together rather than the sound running on past the last picture.
	let vnum = vt.number();
	let anum = at.number();
	let step = if vt.frame_nanos() > 0 {
		(vt.frame_nanos() / 1_000_000).max(1) as u32
	} else {
		40
	};
	let mut file = res!(File::open(path));
	let mut cl = Clusters::new(&mkv);
	let mut buf: Vec<u8> = Vec::new();
	let mut vraw: Vec<(Vec<u8>, bool, i64)> = Vec::new();
	let mut araw: Vec<(Vec<u8>, i64)> = Vec::new();
	let mut eof = false;

	while vraw.len() < want {
		while buf.len() < WINDOW && !eof {
			let mut chunk = vec![0u8; WINDOW];
			let got = res!(file.read(&mut chunk));
			if got == 0 {
				eof = true;
				break;
			}
			chunk.truncate(got);
			buf.extend_from_slice(&chunk);
		}
		if buf.is_empty() {
			break;
		}
		let fed = res!(cl.feed(&buf, &mut |frame| {
			if vraw.len() < want {
				if frame.track == vnum {
					vraw.push((frame.data.to_vec(), frame.key, frame.time));
				} else if frame.track == anum {
					araw.push((frame.data.to_vec(), frame.time));
				}
			}
			Ok(())
		}));
		buf.drain(..fed.used);
		if fed.used == 0 {
			if eof {
				break;
			}
			if fed.want > buf.len() {
				let mut chunk = vec![0u8; fed.want - buf.len()];
				let got = res!(file.read(&mut chunk));
				if got == 0 {
					break;
				}
				chunk.truncate(got);
				buf.extend_from_slice(&chunk);
			}
		}
	}

	// A film must begin at a sync sample; anything before the first one cannot
	// be decoded by a reader starting here and is dropped rather than written.
	let first = match vraw.iter().position(|(_, k, _)| *k) {
		Some(i) => i,
		None => return Ok(None),
	};
	let vkept: Vec<(Vec<u8>, bool, i64)> = vraw.drain(..).skip(first).collect();
	if vkept.len() < frags * 2 {
		return Ok(None);
	}
	// The instant the film starts, which both tracks are measured from.
	let t0 = vkept[0].2;
	let last = match vkept.last() {
		Some((_, _, t)) => *t,
		None => return Ok(None),
	};
	let akept: Vec<(Vec<u8>, i64)> = araw.drain(..)
		.filter(|(_, t)| *t >= t0 && *t <= last)
		.collect();
	if akept.len() < frags * 2 {
		return Ok(None);
	}

	// The picture arrives in decode order carrying the times it is SHOWN, and
	// MP4 wants the times it is decoded plus the difference. Every duration here
	// is the same, so the decoding times are `i * step` and the offsets are what
	// is left -- which for a film with B-pictures is not nought, and which
	// delays the whole track by one constant to keep them all non-negative.
	let vrel: Vec<i64> = vkept.iter().map(|(_, _, t)| *t - t0).collect();
	let vdurs: Vec<u32> = vec![step; vkept.len()];
	let voffs = res!(composition_offsets(&vrel, &vdurs));
	let mut vid = Vec::with_capacity(vkept.len());
	for i in 0..vkept.len() {
		vid.push((i as i64) * step as i64 + voffs[i] as i64);
	}
	let shift = vid[0] - vrel[0];

	// The sound is not reordered, so its offsets are nought and its durations
	// are simply the gaps between the times the source states. Taking them from
	// the source rather than from the sampling rate is deliberate: 1024 samples
	// at 44100 is not a whole number of milliseconds, and a duration rounded
	// once a frame is exactly the slow drift this test is looking for.
	let arel: Vec<i64> = akept.iter().map(|(_, t)| *t - t0).collect();
	let lead = arel[0];
	let mut adurs: Vec<u32> = Vec::with_capacity(arel.len());
	for j in 0..arel.len() {
		let d = if j + 1 < arel.len() {
			arel[j + 1] - arel[j]
		} else {
			match adurs.last() {
				Some(d) => *d as i64,
				None => 0,
			}
		};
		// A stream whose stamps do not advance is one this test cannot reason
		// about, and guessing at it would prove nothing.
		if d <= 0 || d > u32::MAX as i64 {
			return Ok(None);
		}
		adurs.push(d as u32);
	}
	let aud: Vec<i64> = arel.iter().map(|t| *t - lead).collect();

	// Where the fragments are cut. A fragment beginning at a sync sample is what
	// makes one seekable, so the cuts are put on sync samples where the film has
	// enough of them and spread evenly where it does not.
	let starts = res!(cuts(&vkept, &arel, step, frags));
	if starts.len() < MIN_FRAGS {
		return Ok(None);
	}

	// The samples, built whole and then broken up, so that a break is applied to
	// one film rather than to one fragment.
	let mut pic: Vec<Sample> = Vec::with_capacity(vkept.len());
	for (i, (data, key, _)) in vkept.into_iter().enumerate() {
		let s = if key { Sample::key(data, step) } else { Sample::delta(data, step) };
		pic.push(s.shown_after(voffs[i]));
	}
	let mut snd: Vec<Sample> = Vec::with_capacity(akept.len());
	for (j, (data, _)) in akept.into_iter().enumerate() {
		snd.push(Sample::key(data, adurs[j]));
	}
	res!(apply_break(&mut pic, &mut snd, &starts));

	let streams = vec![
		Stream {
			media:		Media::Picture { w: w as u16, h: h as u16 },
			timescale:	SCALE,
			codec:		Codec::Avc(vt.private().to_vec()),
			// Both tracks begin at nought here: the times handed to the writer
			// are already rebased on the first kept frame of each.
			start:		0,
		},
		Stream {
			media:		Media::Sound { channels: chans, rate },
			timescale:	SCALE,
			codec:		Codec::Aac(at.private().to_vec()),
			start:		0,
		},
	];
	let mut film = res!(Fragments::new(streams));
	let mut bytes = res!(film.head());
	for k in 0..starts.len() {
		let (vi, ai) = starts[k];
		let (vj, aj) = match starts.get(k + 1) {
			Some((v, a)) => (*v, *a),
			None => (pic.len() + vi, snd.len() + ai),
		};
		// The samples are drained from the front, so what is left always begins
		// at this fragment and the counts are differences of the boundaries.
		let vrun: Vec<Sample> = pic.drain(..vj - vi).collect();
		let arun: Vec<Sample> = snd.drain(..aj - ai).collect();
		req!(vrun.is_empty(), false, "Fragment {} carries no picture.", k);
		req!(arun.is_empty(), false, "Fragment {} carries no sound.", k);
		let part = res!(film.next(vec![(0, vrun), (1, arun)]));
		bytes.extend_from_slice(&part);
	}
	req!(pic.len(), 0, "Pictures were left over after the last fragment.");
	req!(snd.len(), 0, "Sounds were left over after the last fragment.");

	Ok(Some(Made {
		bytes,
		w: w as u16,
		h: h as u16,
		chans,
		vid,
		aud,
		starts,
		shift,
		lead,
	}))
}

/// Where the fragments begin: for each, the index of its first picture sample
/// and of its first sound sample.
///
/// The sound follows the picture rather than being cut on its own count, so that
/// a fragment holds the sound of the pictures in it -- which is what a reader
/// playing one fragment at a time needs, and what makes the two runs in a `moof`
/// describe the same stretch of film.
fn cuts(
	pics:	&[(Vec<u8>, bool, i64)],
	arel:	&[i64],
	step:	u32,
	frags:	usize,
)
	-> Outcome<Vec<(usize, usize)>>
{
	let n = pics.len();
	let target = (n + frags - 1) / frags;
	if target == 0 {
		return Err(err!("A film of {} pictures cannot be cut into {} fragments.",
			n, frags; Invalid, Input, Range));
	}
	let mut vstarts = vec![0usize];
	for (i, (_, key, _)) in pics.iter().enumerate() {
		if !*key {
			continue;
		}
		let prev = match vstarts.last() {
			Some(v) => *v,
			None => 0,
		};
		// The last fragment is left something to carry.
		if i >= prev + target && n - i >= 2 {
			vstarts.push(i);
		}
	}
	// A film whose sync samples are too far apart is still worth fragmenting; a
	// fragment that does not begin at one is legal and merely not seekable.
	if vstarts.len() < MIN_FRAGS {
		vstarts = (0..frags).map(|k| k * n / frags).collect();
		vstarts.dedup();
	}

	let mut out = Vec::with_capacity(vstarts.len());
	for (k, i) in vstarts.iter().enumerate() {
		if k == 0 {
			out.push((0usize, 0usize));
			continue;
		}
		// The fragment's picture begins at this decode time on the film's clock,
		// and the sound it carries is the sound from there on.
		let at = (*i as i64) * step as i64;
		let j = match arel.iter().position(|t| *t >= at) {
			Some(j) => j,
			None => arel.len(),
		};
		let prev = match out.last() {
			Some((_, a)) => *a,
			None => 0,
		};
		// A fragment with no sound in it would leave check 5 nothing to read at
		// that join, so the cut is dropped rather than written empty.
		if j <= prev || j >= arel.len() {
			continue;
		}
		out.push((*i, j));
	}
	Ok(out)
}

/// Applies the break named by `BREAK_FRAG` to the samples about to be written.
///
/// The expectations the checks hold the film against are built before this runs
/// and are not touched by it, so a break moves the film and not the yardstick.
/// An unrecognised name is an error: a mistyped break that quietly did nothing
/// would look exactly like a check that passed.
fn apply_break(
	pic:	&mut [Sample],
	snd:	&mut [Sample],
	starts:	&[(usize, usize)],
)
	-> Outcome<()>
{
	let name = match env::var("BREAK_FRAG") {
		Ok(v) => v,
		Err(_) => return Ok(()),
	};
	if name.trim().is_empty() {
		return Ok(());
	}
	// Well inside the second fragment, so that no break sits on a boundary that
	// a later check reads and none of them are proved by the wrong question.
	let (vi, ai) = match starts.get(1) {
		Some(v) => *v,
		None => return Err(err!(
			"BREAK_FRAG={} needs a film of more than one fragment.", name;
			Invalid, Input)),
	};
	let v = vi + 3;
	let a = ai + 3;
	if v + 1 >= pic.len() || a + 1 >= snd.len() {
		return Err(err!(
			"BREAK_FRAG={} needs a longer film than this one.", name;
			Invalid, Input, Range));
	}
	match name.as_str() {
		"picture-one-tick"		=> pic[v].off += 1,
		"sound-one-tick"		=> snd[a].off += 1,
		"sound-one-tick-dur"	=> snd[a].dur += 1,
		"sound-delayed"			=> {
			for s in snd.iter_mut() {
				s.off += DELAY;
			}
		},
		other => return Err(err!(
			"BREAK_FRAG={} names no break this test knows. The breaks are \
			picture-one-tick, sound-one-tick, sound-one-tick-dur and \
			sound-delayed.", other; Invalid, Input)),
	}
	println!("BREAK_FRAG={} was applied, so this run is expected to FAIL.", name);
	Ok(())
}

// ----------------------------------------------------------- asking ffprobe

/// The `key=value` fields ffprobe reports for one stream of a film.
///
/// Read as keys rather than as a bare comma-separated line, because the order of
/// the columns is ffprobe's business and a silent reordering would compare the
/// width against the height.
fn probe_fields(path: &Path, select: &str, entries: &str)
	-> Outcome<Vec<(String, String)>>
{
	let out = res!(Command::new("ffprobe")
		.args([
			"-v", "error",
			"-select_streams", select,
			"-count_packets",
			"-show_entries", entries,
			"-of", "default=noprint_wrappers=1",
		])
		.arg(path)
		.output());
	if !out.status.success() {
		return Err(err!(
			"ffprobe would not read stream {} of {}: {}",
			select, path.display(), String::from_utf8_lossy(&out.stderr).trim();
			Invalid, Mismatch));
	}
	let text = String::from_utf8_lossy(&out.stdout);
	let mut fields = Vec::new();
	for line in text.lines() {
		if let Some(at) = line.find('=') {
			fields.push((
				line[..at].trim().to_string(),
				line[at + 1..].trim().to_string(),
			));
		}
	}
	if fields.is_empty() {
		return Err(err!(
			"ffprobe found no stream {} in {} at all.", select, path.display();
			Invalid, Mismatch));
	}
	Ok(fields)
}

/// One field of what ffprobe said.
fn pick(fields: &[(String, String)], key: &str) -> Outcome<String> {
	for (k, v) in fields {
		if k == key {
			return Ok(v.clone());
		}
	}
	Err(err!("ffprobe said nothing about `{}`.", key; Missing))
}

/// One field of what ffprobe said, as a number.
fn number(fields: &[(String, String)], key: &str) -> Outcome<i64> {
	let v = res!(pick(fields, key));
	match v.parse::<i64>() {
		Ok(n) => Ok(n),
		Err(_) => Err(err!(
			"ffprobe gives `{}` as {}, which is not a number.", key, v;
			Invalid, Mismatch)),
	}
}

fn stream_count(path: &Path) -> Outcome<usize> {
	let out = res!(Command::new("ffprobe")
		.args([
			"-v", "error",
			"-show_entries", "stream=index",
			"-of", "csv=p=0",
		])
		.arg(path)
		.output());
	if !out.status.success() {
		return Err(err!(
			"ffprobe would not read {}: {}",
			path.display(), String::from_utf8_lossy(&out.stderr).trim();
			Invalid, Mismatch));
	}
	let text = String::from_utf8_lossy(&out.stdout);
	Ok(text.lines().filter(|l| !l.trim().is_empty()).count())
}

/// Decodes the whole film and refuses any word ffmpeg has to say about it.
///
/// The stronger half of the shape check: a container whose fragments are wrong
/// still opens and still reports a plausible packet count, and only a decode
/// says whether the bytes handed to the decoder were the frames. Anything at all
/// on stderr is a failure -- a warning about a broken `trun` is a broken `trun`
/// -- and it is quoted so the fault can be read rather than guessed at.
fn decodes_silently(path: &Path, film: &Path) -> Outcome<()> {
	let out = res!(Command::new("ffmpeg")
		.args(["-v", "error", "-i"])
		.arg(path)
		.args(["-f", "null", "-"])
		.output());
	let errs = String::from_utf8_lossy(&out.stderr);
	if !errs.trim().is_empty() {
		return Err(err!(
			"{}: ffmpeg complained while decoding the fragmented film: {}",
			film.display(), errs.trim(); Invalid, Mismatch));
	}
	if !out.status.success() {
		return Err(err!(
			"{}: ffmpeg gave up on the fragmented film without saying why.",
			film.display(); Invalid, Mismatch));
	}
	Ok(())
}

/// The presentation times of one stream of a written film, in decode order.
fn times_of(path: &Path, select: &str, film: &Path) -> Outcome<Vec<i64>> {
	let out = res!(Command::new("ffprobe")
		.args([
			"-v", "error",
			"-select_streams", select,
			"-show_entries", "packet=pts",
			"-of", "csv=p=0",
		])
		.arg(path)
		.output());
	if !out.status.success() {
		return Err(err!(
			"{}: ffprobe would not read the times of stream {}.",
			film.display(), select; Invalid, Mismatch));
	}
	let text = String::from_utf8_lossy(&out.stdout);
	let mut times = Vec::new();
	for line in text.lines() {
		let v = line.trim();
		if v.is_empty() {
			continue;
		}
		match v.parse::<i64>() {
			Ok(t) => times.push(t),
			// `N/A` here is a packet the reader could not place, which is the
			// fault this test is about and not a reason to read past it.
			Err(_) => return Err(err!(
				"{}: stream {} has a packet whose time reads `{}`.",
				film.display(), select, v; Invalid, Mismatch)),
		}
	}
	if times.is_empty() {
		return Err(err!(
			"{}: stream {} of the written film holds no packet times.",
			film.display(), select; Invalid, Mismatch));
	}
	Ok(times)
}

// --------------------------------------------------------------- the corpus

fn gather(dir: &Path, out: &mut Vec<PathBuf>) -> Outcome<()> {
	let entries = match fs::read_dir(dir) {
		Ok(e) => e,
		Err(_) => return Ok(()),
	};
	for entry in entries.flatten() {
		let path = entry.path();
		let meta = match fs::metadata(&path) {
			Ok(m) => m,
			Err(_) => continue,
		};
		if meta.is_dir() {
			res!(gather(&path, out));
		} else if meta.len() > 0
			&& path.extension().map(|e| e.eq_ignore_ascii_case("mkv")).unwrap_or(false)
		{
			out.push(path);
		}
	}
	Ok(())
}

fn num_from_env(key: &str, or: usize) -> usize {
	match env::var(key) {
		Ok(v) => v.parse::<usize>().unwrap_or(or),
		Err(_) => or,
	}
}
