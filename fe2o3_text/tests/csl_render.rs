//! B2a foundations: the case folder, numbers and the output tree. Renderer tests against Typst
//! 0.15.1 follow in tests/data/csl/expect.
use oxedyne_fe2o3_text::csl::{
	case::Case,
	item::{
		ChunkKind,
		Chunked,
		NumVal,
		Numeric,
	},
	node::{
		self,
		Formatting,
		Node,
	},
};

#[test]
fn test_title_case_as_typst() {
	// From Typst 0.15.1 with chicago-author-date (tests/data/csl/expect/chicago-author-date.html).
	assert_eq!(Case::Title.transform("Example Site: the home page"), "Example Site: The Home Page");
	assert_eq!(Case::Title.transform("on the art of war"), "On the Art of War");
	assert_eq!(Case::Title.transform("an NASA report"), "An NASA Report");
	assert_eq!(Case::Sentence.transform("The Study of Paris"), "The study of Paris");
	assert_eq!(Case::FirstUpper.transform("hello world"), "Hello world");
	assert_eq!(Case::AllUpper.transform("hello world"), "Hello World");
	assert_eq!(Case::Uppercase.transform("abc"), "ABC");
}

#[test]
fn test_numeric_parse() {
	assert_eq!(Numeric::parse("12").map(|n| n.to_string()), Some("12".to_string()));
	assert_eq!(Numeric::parse("12-15").map(|n| n.to_string()), Some("12–15".to_string()));
	assert_eq!(Numeric::parse("2nd").map(|n| (n.will_transform(), n.to_string())), Some((false, "2nd".to_string())));
	assert_eq!(Numeric::parse("1, 3 & 5").map(|n| n.is_plural(false)), Some(true));
	assert!(Numeric::parse("twelve").is_none());
	assert!(matches!(NumVal::from_str("Supplement A"), NumVal::Str(_)));
}

#[test]
fn test_chunked_and_tree() {
	let mut c = Chunked::normal("A Study of ");
	c.push(ChunkKind::Verbatim, "DNA");
	c.push(ChunkKind::Math, "x^2");
	assert_eq!(c.to_string(), "A Study of DNA$x^2$");
	let f = Formatting::default();
	let tree = node::simplify(vec![
		Node::Text("a".into(), f),
		Node::Text("b".into(), f),
		Node::Elem(node::Elem { children: vec![Node::Text("c".into(), f)], display: None, meta: None }),
	]);
	assert_eq!(tree.len(), 2);
	assert_eq!(node::plain(&tree), "abc");
}
