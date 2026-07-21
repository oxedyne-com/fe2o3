//! The `data:` URL, as a browser hands one over.
//!
//! A page that lets someone choose a picture reads the file in the browser and gets back a `data:`
//! URL: the media type, then the bytes, base64 in every practical case. Sending that string in an
//! ordinary form field is how a small upload reaches a server without a multipart parser on either
//! side, and it is the whole of what this module is for -- a picture, an icon, a signature, not a
//! video.
//!
//! The size ceiling is the caller's and is checked against the decoded length, since base64 inflates
//! by a third and a caller means the bytes it will store, not the string it was sent.

use oxedyne_fe2o3_core::prelude::*;


/// What a `data:` URL carries: what the bytes are, and the bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataUrl {
	/// The media type, e.g. `image/png`. `text/plain` where the URL named none, as the syntax says.
	pub media_type:	String,
	/// The payload, decoded.
	pub bytes:	Vec<u8>,
}

impl DataUrl {

	/// Whether the payload calls itself an image of one of the types a browser draws everywhere.
	///
	/// A caller storing a picture should ask this and refuse the rest: the media type is a claim by
	/// whoever sent it, so it decides what will be served back with the bytes, and serving arbitrary
	/// content under a type of its own choosing is how a picture becomes a page.
	pub fn is_web_image(&self) -> bool {
		matches!(self.media_type.as_str(),
			"image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/svg+xml")
	}
}

/// Reads a `data:` URL, refusing one whose payload exceeds `max_bytes`.
///
/// The shape is `data:[<media type>][;base64],<payload>`. A URL that is not base64 is percent-encoded
/// text, which is read too -- it costs a few lines and the syntax allows it. The ceiling is on the
/// decoded bytes, and a string too long to be under it is refused before it is decoded, so an
/// oversized payload is never held in memory twice.
pub fn parse(s: &str, max_bytes: usize) -> Outcome<DataUrl> {
	let rest = match s.strip_prefix("data:") {
		Some(r)	=> r,
		None	=> return Err(err!("A data URL begins with 'data:'."; Invalid, Input)),
	};
	let (meta, payload) = match rest.split_once(',') {
		Some(p)	=> p,
		None	=> return Err(err!(
			"A data URL separates its media type from its payload with a comma.";
			Invalid, Input, Missing)),
	};
	let b64 = meta.to_lowercase().ends_with(";base64");
	let media_type = {
		// Everything before the first parameter is the type; a charset or another parameter says
		// nothing about bytes this reader hands on, so it is dropped rather than kept unread.
		let head = if b64 { &meta[..meta.len() - ";base64".len()] } else { meta };
		let t = head.split(';').next().unwrap_or("").trim().to_lowercase();
		if t.is_empty() {
			// The syntax's own default for a URL that named no type.
			fmt!("text/plain")
		} else {
			t
		}
	};

	// Four base64 characters carry three bytes, so the decoded length is known from the string's and
	// an oversized payload is turned away before it is decoded.
	let claimed = if b64 { payload.len() / 4 * 3 } else { payload.len() };
	if claimed > max_bytes {
		return Err(err!(
			"A data URL of about {} bytes was sent, over the ceiling of {}.", claimed, max_bytes;
			Invalid, Input, Size, TooBig));
	}

	let bytes = if b64 {
		match base64::decode(payload.trim()) {
			Ok(v)	=> v,
			Err(e)	=> return Err(err!(e,
				"The payload of a data URL is not base64.";
				Invalid, Input, Decode)),
		}
	} else {
		res!(percent_decode(payload))
	};
	if bytes.len() > max_bytes {
		return Err(err!(
			"A data URL of {} bytes was sent, over the ceiling of {}.", bytes.len(), max_bytes;
			Invalid, Input, Size, TooBig));
	}
	Ok(DataUrl { media_type, bytes })
}

/// Percent-decoding, over bytes rather than characters, since what is decoded need not be text.
fn percent_decode(s: &str) -> Outcome<Vec<u8>> {
	let src = s.as_bytes();
	let mut out = Vec::with_capacity(src.len());
	let mut i = 0;
	while i < src.len() {
		match src[i] {
			b'%' => {
				if i + 2 >= src.len() {
					return Err(err!(
						"A percent in a data URL is followed by two hexadecimal digits.";
						Invalid, Input, Decode));
				}
				let hex = match std::str::from_utf8(&src[i + 1..i + 3]) {
					Ok(h)	=> h,
					Err(_)	=> return Err(err!(
						"A percent escape in a data URL is not text."; Invalid, Input, Decode)),
				};
				match u8::from_str_radix(hex, 16) {
					Ok(b)	=> out.push(b),
					Err(e)	=> return Err(err!(e,
						"'{}' is not a pair of hexadecimal digits.", hex;
						Invalid, Input, Decode)),
				}
				i += 3;
			}
			b => {
				out.push(b);
				i += 1;
			}
		}
	}
	Ok(out)
}


#[cfg(test)]
mod tests {
	use super::*;

	/// The ordinary case: what a browser hands back for a small picture.
	#[test]
	fn test_a_base64_picture_reads_00() -> Outcome<()> {
		// A one-pixel GIF, which is the smallest real image there is.
		let u = res!(parse("data:image/gif;base64,R0lGODlhAQABAAAAACw=", 1024));
		assert_eq!(u.media_type, "image/gif");
		assert_eq!(u.bytes.len(), 14);
		assert!(u.is_web_image());
		Ok(())
	}

	/// A URL naming no type is text, as the syntax says, and a parameter beside the type is dropped.
	#[test]
	fn test_a_typeless_url_is_text_01() -> Outcome<()> {
		let u = res!(parse("data:,hello%20there", 1024));
		assert_eq!(u.media_type, "text/plain");
		assert_eq!(String::from_utf8_lossy(&u.bytes), "hello there");
		let p = res!(parse("data:text/plain;charset=utf-8;base64,aGk=", 1024));
		assert_eq!(p.media_type, "text/plain");
		assert_eq!(String::from_utf8_lossy(&p.bytes), "hi");
		assert!(!p.is_web_image());
		Ok(())
	}

	/// A payload over the ceiling is refused, and refused on its decoded length rather than its
	/// string's, so the third that base64 adds does not count against the caller.
	#[test]
	fn test_a_payload_over_the_ceiling_is_refused_02() -> Outcome<()> {
		// Sixty bytes, which base64 writes in eighty characters.
		let big = base64::encode(vec![7u8; 60]);
		assert!(parse(&fmt!("data:image/png;base64,{}", big), 50).is_err(),
			"an oversized payload was taken");
		let ok = res!(parse(&fmt!("data:image/png;base64,{}", big), 64));
		assert_eq!(ok.bytes.len(), 60);
		Ok(())
	}

	/// What is not a data URL says so, rather than being read as an empty one.
	#[test]
	fn test_what_is_not_a_data_url_is_refused_03() -> Outcome<()> {
		assert!(parse("https://example.com/a.png", 1024).is_err(), "a plain URL was taken");
		assert!(parse("data:image/png;base64", 1024).is_err(), "a URL with no comma was taken");
		assert!(parse("data:image/png;base64,!!!not base64!!!", 1024).is_err(),
			"a bad payload was taken");
		Ok(())
	}
}
