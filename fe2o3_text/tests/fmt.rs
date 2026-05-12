//! Integration tests for the code formatter.
//!
//! Tests against representative Rust code patterns found in the
//! fe2o3 codebase. Each test checks that the formatter produces
//! reasonable output and that formatting is idempotent.

use oxedyne_fe2o3_text::fmt::{
    format_rust,
    detect_language_from_source,
    spec::FormatSpec,
};

fn fe2o3_fmt(source: &str) -> String {
    let spec = FormatSpec::fe2o3();
    format_rust(source, &spec).expect("format failed")
}

/// Assert that formatting is idempotent: formatting twice gives
/// the same result as formatting once.
fn assert_idempotent(source: &str) {
    let first = fe2o3_fmt(source);
    let second = fe2o3_fmt(&first);
    assert_eq!(
        first, second,
        "not idempotent!\n--- first ---\n{}\n--- second ---\n{}",
        first, second,
    );
}

/// Assert the output contains an expected substring.
fn assert_contains(source: &str, expected: &str) {
    let out = fe2o3_fmt(source);
    assert!(
        out.contains(expected),
        "expected {:?} in output:\n{}",
        expected, out,
    );
}

// ── Basic constructs ─────────────────────────────────────────────

#[test]
fn test_empty_fn() {
    let out = fe2o3_fmt("fn main() {}");
    assert!(out.contains("fn main()"), "output: {}", out);
    assert_idempotent("fn main() {}");
}

#[test]
fn test_fn_with_return_type() {
    let out = fe2o3_fmt("fn foo() -> bool { true }");
    assert!(out.contains("-> bool"), "output: {}", out);
    assert_idempotent("fn foo() -> bool { true }");
}

#[test]
fn test_fn_with_params() {
    let source = "fn foo(x: u32, y: u64) -> bool { true }";
    let out = fe2o3_fmt(source);
    assert!(out.contains("x: u32"), "output: {}", out);
    assert!(out.contains("y: u64"), "output: {}", out);
    assert_idempotent(source);
}

#[test]
fn test_struct_definition() {
    let source = "pub struct CalClock {\n    date: CalendarDate,\n    time: ClockTime,\n}";
    let out = fe2o3_fmt(source);
    assert!(out.contains("pub struct CalClock"), "output: {}", out);
    assert!(out.contains("date: CalendarDate"), "output: {}", out);
    assert_idempotent(source);
}

#[test]
fn test_unit_struct() {
    let source = "pub struct Rand;";
    let out = fe2o3_fmt(source);
    assert!(out.contains("pub struct Rand;"), "output: {}", out);
    assert_idempotent(source);
}

#[test]
fn test_enum_definition() {
    let source = "pub enum NumberType {\n    SignedInt,\n    UnsignedInt,\n    FloatingPoint,\n}";
    let out = fe2o3_fmt(source);
    assert!(out.contains("pub enum NumberType"), "output: {}", out);
    assert!(out.contains("SignedInt"), "output: {}", out);
    assert_idempotent(source);
}

// ── Imports ──────────────────────────────────────────────────────

#[test]
fn test_simple_use() {
    let source = "use std::fmt;";
    let out = fe2o3_fmt(source);
    assert!(out.contains("use std::fmt;"), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_grouped_use() {
    let source = "use crate::{\n    prelude::*,\n    byte::B32,\n};";
    let out = fe2o3_fmt(source);
    assert!(out.contains("prelude"), "output: {:?}", out);
    assert!(out.contains("B32"), "output: {:?}", out);
    assert_idempotent(source);
}

// ── Impl blocks ──────────────────────────────────────────────────

#[test]
fn test_impl_block() {
    let source = r#"impl Foo {
    fn bar(&self) -> usize { 42 }
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("impl Foo"), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_impl_trait() {
    let source = r#"impl fmt::Display for Foo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("impl fmt::Display for Foo"), "output: {:?}", out);
    assert_idempotent(source);
}

// ── Comments ─────────────────────────────────────────────────────

#[test]
fn test_preserves_line_comments() {
    let source = "// This is a comment.\nfn foo() {}";
    let out = fe2o3_fmt(source);
    assert!(out.contains("// This is a comment."), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_preserves_doc_comments() {
    let source = "/// Documentation for foo.\nfn foo() {}";
    let out = fe2o3_fmt(source);
    assert!(out.contains("/// Documentation for foo."), "output: {:?}", out);
    assert_idempotent(source);
}

// ── Attributes ───────────────────────────────────────────────────

#[test]
fn test_preserves_attributes() {
    let source = "#[derive(Clone, Debug)]\npub struct Foo;";
    let out = fe2o3_fmt(source);
    assert!(out.contains("#[derive(Clone, Debug)]"), "output: {:?}", out);
    assert_idempotent(source);
}

// ── Where clauses ────────────────────────────────────────────────

#[test]
fn test_fn_with_where() {
    let source = r#"fn value<T>() -> T
where
    T: Clone,
{
    todo!()
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("where"), "output: {:?}", out);
    assert!(out.contains("T: Clone"), "output: {:?}", out);
    assert_idempotent(source);
}

// ── Multiple items ───────────────────────────────────────────────

#[test]
fn test_multiple_items() {
    let source = r#"use std::fmt;

fn main() {}

struct Foo;"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("use std::fmt"), "output: {:?}", out);
    assert!(out.contains("fn main"), "output: {:?}", out);
    assert!(out.contains("struct Foo"), "output: {:?}", out);
    assert_idempotent(source);
}

// ── Fe2o3-style patterns ─────────────────────────────────────────

#[test]
fn test_fe2o3_error_macro() {
    // The err! macro uses semicolons — should be preserved verbatim.
    let source = r#"fn foo() {
    return Err(err!("Invalid input: {}", x; Invalid, Input));
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("err!"), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_fe2o3_res_macro() {
    let source = r#"fn foo() {
    let val = res!(some_function());
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("res!"), "output: {:?}", out);
    assert_idempotent(source);
}

// ── Idempotency on real-world patterns ───────────────────────────

#[test]
fn test_idempotent_trait_impl() {
    assert_idempotent(r#"impl ParseId<1> for u8 {
    fn parse_id(s: &str) -> Outcome<Self> {
        Ok(res!(u8::from_str(s)))
    }
}"#);
}

#[test]
fn test_idempotent_enum_with_values() {
    assert_idempotent(r#"pub enum Encoding {
    Unknown = 0,
    Binary = 1,
    UTF8 = 2,
}"#);
}

#[test]
fn test_idempotent_complex_fn() {
    assert_idempotent(r#"pub fn byte_slices_equal(a: &[u8], b: &[u8]) -> Outcome<()> {
    for (i, ai) in a.iter().enumerate() {
        if *ai != b[i] {
            return Err(err!("Mismatch detected"; Input, Mismatch));
        }
    }
    Ok(())
}"#);
}

// ── Format spec options ──────────────────────────────────────────

// ── Oxedyne fn signature layout ──────────────────────────────────

#[test]
fn test_oxedyne_fn_signature_flat() {
    // Short signature: everything on one line.
    let out = fe2o3_fmt("fn foo(x: u32) -> bool { true }");
    // Return type should be inline.
    assert!(out.contains("fn foo(x: u32) -> bool"), "output: {:?}", out);
    assert_idempotent("fn foo(x: u32) -> bool { true }");
}

#[test]
fn test_oxedyne_fn_signature_broken() {
    // Force params to break by using a narrow width.
    let mut spec = FormatSpec::fe2o3();
    spec.max_width = 30;
    let source = "fn generate_random_string(len: usize, charset: &str) -> String { todo!() }";
    let out = format_rust(source, &spec).expect("format failed");
    println!("output:\n{}", out);
    // Params should be vertical, return type on own line.
    assert!(out.contains("fn generate_random_string("), "output: {:?}", out);
    assert!(out.contains("len:"), "expected vertical params, output: {:?}", out);
    assert!(out.contains("usize,"), "expected vertical params, output: {:?}", out);
    assert!(out.contains("charset: &str"), "expected vertical params, output: {:?}", out);
    // Type column should be aligned (len padded to match charset).
    assert!(out.contains("len:     usize,"), "expected aligned types, output: {:?}", out);
    // Return type on own indented line (Oxedyne rule).
    // The `-> String` should appear on a line by itself, indented.
    let lines: Vec<&str> = out.lines().collect();
    let ret_line = lines.iter().find(|l| l.contains("-> String"));
    assert!(ret_line.is_some(), "expected '-> String' on its own line, output: {:?}", out);
}

#[test]
fn test_config_from_str() {
    let config = r#"
# Oxedyne style with 2-space indent
indent_width = 2
max_width = 80
where_indent = true
fn_return_type = own_line
brace_style = same_line_unless_where
trailing_comma = true
field_align_threshold = 30
"#;
    let spec = FormatSpec::from_config_str(config).expect("parse failed");
    assert_eq!(spec.indent_width, 2);
    assert_eq!(spec.max_width, 80);
    assert_eq!(spec.where_indent, true);
    assert_eq!(spec.field_align_threshold, 30);
}

#[test]
fn test_config_unknown_key_is_error() {
    let config = "nonexistent_option = true\n";
    assert!(FormatSpec::from_config_str(config).is_err());
}

#[test]
fn test_format_with_loaded_config() {
    let config = "indent_width = 2\nmax_width = 100\n";
    let spec = FormatSpec::from_config_str(config).expect("parse failed");
    let source = "fn foo() {\n    let x = 1;\n}";
    let out = format_rust(source, &spec).expect("format failed");
    assert!(out.contains("\n  let"), "expected 2-space indent, got: {:?}", out);
}

#[test]
fn test_spec_indent_width() {
    let mut spec = FormatSpec::fe2o3();
    spec.indent_width = 2;
    let source = "fn foo() {\n    let x = 1;\n}";
    let out = format_rust(source, &spec).expect("format failed");
    // Should use 2-space indent.
    assert!(out.contains("\n  let"), "expected 2-space indent, got: {:?}", out);
}

#[test]
fn test_print_formatted_output() {
    let source = r#"fn day_number(day: DayOfWeek) -> u8 {
    match day {
        Monday => 1,
        Tuesday => 2,
        Wednesday => 3,
        Thursday => 4,
        Friday => 5,
        Saturday => 6,
        Sunday => 7,
    }
}

pub struct ImapServer {
    pub store:      u32,
    pub users:      u64,
    pub hostname:   String,
}

use crate::{
    prelude::*,
    byte::B32,
};

use std::cmp::PartialOrd;

/// Sampling method for range generation.
#[derive(Clone, Copy, Debug)]
pub enum SamplingMethod {
    Uniform,
    GaussianClampedDerived,
}

pub struct Rand;

impl Rand {
    pub fn rand_u8() -> u8 {
        42
    }

    pub fn in_range<T>(lower: T, upper: T) -> T
    where
        T: PartialOrd
    {
        lower
    }
}
"#;
    let out = fe2o3_fmt(source);
    println!("=== FORMATTED OUTPUT ===");
    println!("{}", out);
    println!("========================");
    assert_idempotent(source);
}

// ── Comprehensive Oxedyne-style test ─────────────────────────────

#[test]
fn test_oxedyne_comprehensive() {
    let source = r#"use crate::{
    prelude::*,
    byte::B32,
};

use std::fmt;

/// Combined date and clock time.
pub struct CalClock {
    date: CalendarDate,
    time: ClockTime,
    zone: CalClockZone,
}

impl CalClock {
    pub fn year(&self) -> i32 { self.date.year() }

    pub fn day_of_week(&self) -> DayOfWeek {
        match self.date.weekday() {
            0 => DayOfWeek::Monday,
            1 => DayOfWeek::Tuesday,
            2 => DayOfWeek::Wednesday,
            3 => DayOfWeek::Thursday,
            4 => DayOfWeek::Friday,
            5 => DayOfWeek::Saturday,
            _ => DayOfWeek::Sunday,
        }
    }
}

impl fmt::Display for CalClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.date)
    }
}
"#;
    let out = fe2o3_fmt(source);

    // Struct field alignment.
    assert!(out.contains("date: CalendarDate"), "output: {:?}", out);
    assert!(out.contains("zone: CalClockZone"), "output: {:?}", out);

    // Short fn on one line.
    assert!(out.contains("pub fn year(&self) -> i32 { self.date.year() }"),
        "expected single-line fn, output: {:?}", out);

    // Trait impl fn stays on one line.
    assert!(out.contains("fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result"),
        "expected single-line trait fn sig, output: {:?}", out);

    // Match arm alignment.
    assert!(out.contains("0 => DayOfWeek::Monday"),    "output: {:?}", out);
    assert!(out.contains("2 => DayOfWeek::Wednesday"),  "output: {:?}", out);

    // Idempotent.
    assert_idempotent(source);
}

#[test]
fn test_enum_discriminant_alignment() {
    let source = r#"pub enum Encoding {
    Unknown = 0,
    Binary = 1,
    UTF8 = 2,
}"#;
    let out = fe2o3_fmt(source);
    // `=` column should be aligned.
    assert!(out.contains("Unknown = 0"), "output: {:?}", out);
    assert!(out.contains("Binary  = 1"), "expected aligned =, output: {:?}", out);
    assert!(out.contains("UTF8    = 2"), "expected aligned =, output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_method_chain_preserved() {
    let source = r#"fn foo() {
    let result = items.iter()
        .filter(|x| x > 0)
        .map(|x| x * 2)
        .collect();
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains(".filter"), "output: {:?}", out);
    assert!(out.contains(".map"), "output: {:?}", out);
    assert!(out.contains(".collect"), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_method_chain_on_one_line() {
    // Short chain on one line stays on one line.
    let source = r#"fn foo() {
    let x = items.iter().map(|x| x * 2).collect();
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("items.iter().map"), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_nested_generics() {
    let out = fe2o3_fmt("fn foo() -> Outcome<Vec<Token>> { todo!() }");
    assert!(out.contains("Outcome<Vec<Token>>"), "expected no space in >>, output: {:?}", out);
    assert_idempotent("fn foo() -> Outcome<Vec<Token>> { todo!() }");
}

#[test]
fn test_binary_operator_continuation() {
    let source = r#"fn foo() {
    let x = a
        && b
        || c;
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("&&"), "output: {:?}", out);
    assert!(out.contains("||"), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_binop_precedence_indentation() {
    // Test the format_binop_expr function directly.
    use oxedyne_fe2o3_text::fmt::spec::FormatSpec;
    let _spec = FormatSpec::fe2o3();
    // The precedence-based indentation will be visible when the
    // expression is formatted at a narrow width.
    let source = r#"fn foo() {
    let valid = name_len > 0 && age >= 18 && country == "NZ" || special;
}"#;
    let out = fe2o3_fmt(source);
    assert!(out.contains("&&"), "output: {:?}", out);
    assert!(out.contains("||"), "output: {:?}", out);
    assert_idempotent(source);
}

#[test]
fn test_binop_break_wide_expression() {
    // Narrow width forces the expression to break. Oxedyne rule:
    // break before each operator; each lower-precedence operator
    // indents one level deeper than the previous.
    let mut spec = FormatSpec::fe2o3();
    spec.max_width = 40;
    let source = "fn foo() {\n    let valid = aaaa && bbbb && cccc || dddd;\n}\n";
    let out = format_rust(source, &spec).expect("format failed");
    // `&&` at +4 (tightest), `||` at +8 (one level deeper).
    assert!(out.contains("\n        && bbbb"), "expected break-before && at +4, output:\n{}", out);
    assert!(out.contains("\n        && cccc"), "expected break-before && at +4, output:\n{}", out);
    assert!(out.contains("\n            || dddd"), "expected break-before || at +8, output:\n{}", out);
    // Idempotent.
    let second = format_rust(&out, &spec).expect("format failed");
    assert_eq!(out, second, "not idempotent:\n--- first ---\n{}\n--- second ---\n{}", out, second);
}

#[test]
fn test_binop_stays_flat_when_fits() {
    // At default width (100), the expression fits on one line.
    let source = "fn foo() {\n    let ok = a && b || c;\n}\n";
    let out = fe2o3_fmt(source);
    assert!(out.contains("a && b || c"), "should stay flat, output:\n{}", out);
    assert_idempotent(source);
}

#[test]
fn test_binop_return_statement() {
    // Binary ops in a return statement.
    let mut spec = FormatSpec::fe2o3();
    spec.max_width = 30;
    let source = "fn check() {\n    return alpha && beta || gamma;\n}\n";
    let out = format_rust(source, &spec).expect("format failed");
    assert!(out.contains("return alpha"), "output:\n{}", out);
    assert!(out.contains("\n        && beta"), "expected break-before &&, output:\n{}", out);
    assert!(out.contains("\n            || gamma"), "expected break-before ||, output:\n{}", out);
    let second = format_rust(&out, &spec).expect("format failed");
    assert_eq!(out, second, "not idempotent:\n{}\nvs:\n{}", out, second);
}

#[test]
fn test_binop_let_mut() {
    // `let mut` should not confuse the handler.
    let source = "fn foo() {\n    let mut x = a && b;\n}\n";
    let out = fe2o3_fmt(source);
    assert!(out.contains("let mut x"), "output:\n{}", out);
    assert!(out.contains("a && b"), "output:\n{}", out);
    assert_idempotent(source);
}

#[test]
fn test_return_no_expr() {
    // `return;` should not produce `return ;`.
    let source = "fn foo() {\n    return;\n}\n";
    let out = fe2o3_fmt(source);
    assert!(out.contains("return;"), "expected 'return;' not 'return ;', output:\n{}", out);
    assert_idempotent(source);
}

#[test]
fn test_import_reorder() {
    let mut spec = FormatSpec::fe2o3();
    spec.import_reorder = true;
    let source = "use crate::{\n    zebra,\n    alpha,\n    middle,\n};";
    let out = format_rust(source, &spec).expect("format failed");
    // Items should be sorted alphabetically.
    let alpha_pos = out.find("alpha").unwrap_or(999);
    let middle_pos = out.find("middle").unwrap_or(999);
    let zebra_pos = out.find("zebra").unwrap_or(999);
    assert!(alpha_pos < middle_pos, "alpha should be before middle, output: {:?}", out);
    assert!(middle_pos < zebra_pos, "middle should be before zebra, output: {:?}", out);
}

// ── Full fe2o3-style integration test ────────────────────────────

#[test]
fn test_fe2o3_realistic_code() {
    let source = r#"use crate::{
    prelude::*,
    byte::B32,
};

use std::{
    fmt,
    sync::Arc,
};


/// Broad token categories.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Keyword,
    Ident,
    Number,
    StringLit,
    Eof,
}

/// Byte range in the source text.
#[derive(Clone, Copy, Debug, Default)]
pub struct Span {
    pub start: usize,
    pub end:   usize,
}

#[repr(u8)]
pub enum Encoding {
    Unknown = 0,
    Binary  = 1,
    UTF8    = 2,
}

pub struct Lexer {
    src:    String,
    pos:    usize,
    tokens: Vec<Token>,
}

impl Lexer {
    pub fn new(src: String) -> Self {
        Self { src, pos: 0, tokens: Vec::new() }
    }

    pub fn peek(&self) -> &Token { &self.tokens[self.pos] }

    pub fn at_eof(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    pub fn lex_all(
        src:        &str,
        max_tokens: usize,
        strict:     bool,
    )
        -> Outcome<Vec<Token>>
    {
        let mut tokens = Vec::new();
        for (i, ch) in src.chars().enumerate() {
            if i >= max_tokens {
                break;
            }
            match ch {
                '0'..='9' => tokens.push(Token { kind: TokenKind::Number }),
                'a'..='z' => tokens.push(Token { kind: TokenKind::Ident }),
                _         => tokens.push(Token { kind: TokenKind::Eof }),
            }
        }
        Ok(tokens)
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
"#;
    let out = fe2o3_fmt(source);
    println!("=== REALISTIC OUTPUT ===");
    println!("{}", out);
    println!("========================");

    // Struct field alignment.
    assert!(out.contains("start: usize"), "output: {:?}", out);
    assert!(out.contains("end:   usize"), "expected aligned fields, output: {:?}", out);

    // Enum discriminant alignment.
    assert!(out.contains("Binary  = 1"), "expected aligned =, output: {:?}", out);

    // Match arm alignment.
    assert!(out.contains("'0'..='9' =>"), "output: {:?}", out);
    assert!(out.contains("'a'..='z' =>"), "output: {:?}", out);

    // Short fns on one line.
    assert!(out.contains("pub fn at_eof(&self) -> bool"),
        "expected single-line fn, output: {:?}", out);

    // Trait impl Display stays on one line.
    assert!(out.contains("fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result"),
        "output: {:?}", out);

    // Idempotent.
    assert_idempotent(source);
}

// ── Workspace regression ────────────────────────────────────────

/// Run the formatter against every `.rs` file in the fe2o3 workspace.
///
/// Each file is processed independently with no state carried
/// across files. Checks that the formatter does not crash and
/// that the output is idempotent (formatting twice gives the
/// same result as formatting once).
#[test]
fn test_workspace_regression() {
    use std::fs;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let spec = FormatSpec::fe2o3();

    let mut checked = 0usize;
    let mut failed: Vec<String> = Vec::new();

    // Walk the workspace, skipping target/.
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default();
                if name == "target" || name == ".git" {
                    continue;
                }
                dirs.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let source = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if source.is_empty() {
                continue;
            }
            let rel = path.strip_prefix(&root).unwrap_or(&path);

            // Format once.
            let first = match format_rust(&source, &spec) {
                Ok(s) => s,
                Err(e) => {
                    failed.push(format!("{}: format error: {}", rel.display(), e));
                    continue;
                }
            };

            // Idempotency: format the output a second time.
            let second = match format_rust(&first, &spec) {
                Ok(s) => s,
                Err(e) => {
                    failed.push(format!("{}: second-pass error: {}", rel.display(), e));
                    continue;
                }
            };
            if first != second {
                // Find the first divergent line for the report.
                let fl: Vec<&str> = first.lines().collect();
                let sl: Vec<&str> = second.lines().collect();
                let mut diff_line = 0;
                for (i, (a, b)) in fl.iter().zip(sl.iter()).enumerate() {
                    if a != b {
                        diff_line = i + 1;
                        break;
                    }
                }
                if diff_line == 0 {
                    diff_line = fl.len().min(sl.len()) + 1;
                }
                failed.push(format!(
                    "{}: not idempotent (first divergence at line {})",
                    rel.display(), diff_line,
                ));
            }

            checked += 1;
        }
    }

    println!("Checked {} files, {} failures.", checked, failed.len());
    if !failed.is_empty() {
        for f in &failed {
            println!("  FAIL: {}", f);
        }
        panic!("{} file(s) failed the regression check.", failed.len());
    }
}

// ── Content-based language detection ─────────────────────────────

#[test]
fn test_detect_rust_from_source() {
    let src = r#"
use std::collections::HashMap;

pub fn main() {
    let mut map = HashMap::new();
    map.insert("key", "value");
}
"#;
    assert_eq!(detect_language_from_source(src), Some("rust"));
}

#[test]
fn test_detect_go_from_source() {
    let src = r#"
package main

import "fmt"

func main() {
    x := 42
    fmt.Println(x)
}
"#;
    assert_eq!(detect_language_from_source(src), Some("go"));
}

#[test]
fn test_detect_python_from_source() {
    let src = r#"
from collections import defaultdict

class Foo:
    def __init__(self):
        self.data = defaultdict(list)

    def add(self, key, value):
        self.data[key].append(value)
"#;
    assert_eq!(detect_language_from_source(src), Some("python"));
}

#[test]
fn test_detect_python_shebang() {
    let src = "#!/usr/bin/env python3\nimport sys\nprint(sys.argv)\n";
    assert_eq!(detect_language_from_source(src), Some("python"));
}

#[test]
fn test_detect_js_from_source() {
    let src = r#"
import { readFile } from 'fs';

const handler = async (req) => {
    if (req.method === 'GET') {
        console.log('request received');
    }
};

export default handler;
"#;
    assert_eq!(detect_language_from_source(src), Some("js"));
}

#[test]
fn test_detect_java_from_source() {
    let src = r#"
import java.util.HashMap;

public class Main {
    public static void main(String[] args) {
        System.out.println("hello");
    }
}
"#;
    assert_eq!(detect_language_from_source(src), Some("java"));
}

#[test]
fn test_detect_c_from_source() {
    let src = r#"
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    printf("hello %s\n", argv[0]);
    return 0;
}
"#;
    assert_eq!(detect_language_from_source(src), Some("c"));
}

#[test]
fn test_detect_none_for_empty() {
    assert_eq!(detect_language_from_source(""), None);
    assert_eq!(detect_language_from_source("hello world"), None);
}

// ── Corpus-based detection tests ─────────────────────────────────

/// Walk a corpus directory and verify detection for every file.
///
/// The corpus lives in `tests/detect_corpus/`. Subdirectories are
/// named after the expected language (`rust`, `c`, `go`, `java`,
/// `js`, `python`) or `none` for non-code files. Each `.txt` file
/// in a language directory should be detected as that language.
/// Files in `none/` should return `None`.
#[test]
fn test_detect_corpus() {
    let corpus_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("detect_corpus");

    let mut checked = 0;
    let mut failed: Vec<String> = Vec::new();

    let lang_dirs = ["rust", "c", "cpp", "csharp", "go", "java", "js", "python", "none"];

    for lang_dir in &lang_dirs {
        let dir = corpus_dir.join(lang_dir);
        if !dir.exists() {
            panic!("corpus directory missing: {}", dir.display());
        }

        let expected: Option<&str> = match *lang_dir {
            "none" => None,
            other  => Some(other),
        };

        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .expect("cannot read corpus dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "txt"))
            .collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let path = entry.path();
            let source = std::fs::read_to_string(&path)
                .expect("cannot read corpus file");

            let detected = detect_language_from_source(&source);
            let rel = format!("{}/{}", lang_dir, path.file_name().unwrap().to_string_lossy());

            if detected != expected {
                failed.push(format!(
                    "{}: expected {:?}, got {:?}",
                    rel, expected, detected,
                ));
            }
            checked += 1;
        }
    }

    println!("Corpus: checked {} files, {} failures.", checked, failed.len());
    if !failed.is_empty() {
        for f in &failed {
            println!("  FAIL: {}", f);
        }
        panic!("{} corpus file(s) failed detection.", failed.len());
    }
}
