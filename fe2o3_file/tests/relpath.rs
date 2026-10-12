//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::relpath::{
	Fault,
	RelPath,
};

use oxedyne_fe2o3_core::{
	prelude::*,
	test::test_it,
};


pub fn test_relpath(filter: &'static str) -> Outcome<()> {

	res!(test_it(filter, &["Every non-canonical spelling is refused, never trimmed 000", "all", "relpath"], || {
		let cases: &[(&str, Fault)] = &[
			("",				Fault::Empty),
			(".",				Fault::Dot),
			("..",				Fault::DotDot),
			("./a",				Fault::Dot),
			("a/./b",			Fault::Dot),
			("a/.",				Fault::Dot),
			("../a",			Fault::DotDot),
			("a/../b",			Fault::DotDot),
			("a/..",			Fault::DotDot),
			("/a",				Fault::Absolute),
			("/",				Fault::Absolute),
			("a\\b",			Fault::Backslash),
			("..\\x",			Fault::Backslash),
			("a//b",			Fault::EmptySegment),
			("a/",				Fault::EmptySegment),
			("a/b/",			Fault::EmptySegment),
			(" a",				Fault::EdgeSpace),
			("a ",				Fault::EdgeSpace),
			("a/ b",			Fault::EdgeSpace),
			("a/b /c",			Fault::EdgeSpace),
			("\u{a0}a",			Fault::EdgeSpace),	// no-break space
			("a\u{3000}",		Fault::EdgeSpace),	// ideographic space
			(" ",				Fault::EdgeSpace),
			("a\0b",			Fault::Control),
			("\0",				Fault::Control),
			("a\u{7}",			Fault::Control),
			("a\nb",			Fault::Control),
			("a\u{9f}b",		Fault::Control),
			// Daimond HIST-SAFE U2 post-deploy QA, F1 and F2, verbatim.
			(" sp",				Fault::EdgeSpace),
			(" secret",			Fault::EdgeSpace),
			("secret ",			Fault::EdgeSpace),
			("\tsecret",		Fault::Control),
			("chatc/../secret",	Fault::DotDot),
			("/secret",			Fault::Absolute),
			("secret/",			Fault::EmptySegment),
			("./secret",		Fault::Dot),
			("code/../secret",	Fault::DotDot),
			("../secret",		Fault::DotDot),
			("code/..",			Fault::DotDot),
			("code//..//secret",Fault::EmptySegment),
			// CAPTURE-FENCE QA B, F1, verbatim.
			(" ../x.md",		Fault::EdgeSpace),
			(".. /x.md",		Fault::EdgeSpace),
			("..\t/x.md",		Fault::Control),
		];
		for (s, want) in cases {
			let got = RelPath::fault(s);
			if got != Some(*want) {
				return Err(err!("RelPath::fault({:?}) = {:?}, expected {:?}.", s, got, want; Test));
			}
			match RelPath::parse(s) {
				Ok(p)	=> return Err(err!("RelPath::parse({:?}) accepted {:?}.", s, p; Test)),
				Err(e)	=> {
					let msg = fmt!("{}", e);
					if !msg.contains(&fmt!("{:?}", s)) || !msg.contains(&fmt!("{}", want)) {
						return Err(err!("The refusal of {:?} does not name the value and the reason: {}", s, msg; Test));
					}
				}
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["Legal names are kept exactly as written 000", "all", "relpath"], || {
		let ok = [
			"a", "sp", "a/b/c.md", "a..b.md", "v1.2.3.md", ".../x.md", "..hidden.md", "x...md",
			"..../e.md", ".hidden", "a b/c d.md", "café/naïve.txt", "%2e%2e/x", "\u{2024}\u{2024}/x",
			"diamonds/0da1/shots/self.png",
		];
		for s in ok {
			let p = res!(RelPath::parse(s));
			if p.as_str() != s {
				return Err(err!("RelPath::parse({:?}) changed it to {:?}.", s, p; Test));
			}
		}
		Ok(())
	}));

	res!(test_it(filter, &["Segments, name, parent, join and is_within 000", "all", "relpath"], || {
		let p = res!(RelPath::parse("a/b/c.md"));
		let segs: Vec<&str> = p.segments().collect();
		assert_eq!(segs, vec!["a", "b", "c.md"]);
		assert_eq!(p.name(), "c.md");
		assert_eq!(p.parent().map(|q| q.into_string()), Some(fmt!("a/b")));
		assert_eq!(res!(RelPath::parse("a")).parent(), None);
		let a  = res!(RelPath::parse("a"));
		let ab = res!(RelPath::parse("ab"));
		assert!(p.is_within(&a));
		assert!(a.is_within(&a));
		assert!(!ab.is_within(&a));
		assert!(!a.is_within(&p));
		let j = a.join(&res!(RelPath::parse("b/c.md")));
		assert_eq!(j, p);
		assert!(RelPath::is_canonical(j.as_str()));
		Ok(())
	}));

	Ok(())
}
