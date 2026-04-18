//! Layout renderer.
//!
//! Converts a `Doc` into a formatted `String` by making optimal
//! line-breaking decisions. Based on Lindig's strict version of
//! Wadler's algorithm, extended with alignment support.
//!

use crate::fmt::doc::Doc;

use oxedyne_fe2o3_core::prelude::*;


/// Rendering mode for the current group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    /// Try to fit everything on one line.
    Flat,
    /// Break at every `Line`.
    Break,
}

/// A command on the rendering work stack.
#[derive(Clone, Debug)]
struct Cmd {
    indent: usize,
    mode:   Mode,
    doc:    Doc,
}

/// Render a `Doc` to a formatted string.
///
/// # Arguments
/// * `width` — maximum line width (soft limit; `Text` nodes that
///   exceed it are never broken).
/// * `indent_str` — the string used for one level of indentation
///   (e.g. `"    "` for four spaces).
/// * `doc` — the layout document to render.
pub fn render(
    width:      usize,
    indent_str: &str,
    doc:        &Doc,
) -> String {
    let indent_w = indent_str.len();
    let mut out = String::with_capacity(1024);
    let mut col: usize = 0; // Current column position.
    // Work stack (processed back to front).
    let mut stack: Vec<Cmd> = vec![Cmd {
        indent: 0,
        mode:   Mode::Break,
        doc:    doc.clone(),
    }];

    while let Some(cmd) = stack.pop() {
        match cmd.doc {
            Doc::Empty => {}

            Doc::Text(ref s) => {
                out.push_str(s);
                col += s.len();
            }

            Doc::Line => {
                match cmd.mode {
                    Mode::Flat => {
                        // In flat mode, a Line becomes a single space.
                        out.push(' ');
                        col += 1;
                    }
                    Mode::Break => {
                        out.push('\n');
                        let spaces = cmd.indent;
                        emit_indent(&mut out, spaces, indent_str, indent_w);
                        col = spaces;
                    }
                }
            }

            Doc::HardLine => {
                out.push('\n');
                let spaces = cmd.indent;
                emit_indent(&mut out, spaces, indent_str, indent_w);
                col = spaces;
            }

            Doc::Nest(n, ref inner) => {
                stack.push(Cmd {
                    indent: cmd.indent + (n as usize),
                    mode:   cmd.mode,
                    doc:    (**inner).clone(),
                });
            }

            Doc::Concat(ref docs) => {
                // Push in reverse so the first doc is processed first.
                for d in docs.iter().rev() {
                    stack.push(Cmd {
                        indent: cmd.indent,
                        mode:   cmd.mode,
                        doc:    d.clone(),
                    });
                }
            }

            Doc::Group(ref inner) => {
                // Try flat mode: measure whether the group fits.
                let flat_len = measure_flat(&inner, width.saturating_sub(col));
                if flat_len.is_some() {
                    stack.push(Cmd {
                        indent: cmd.indent,
                        mode:   Mode::Flat,
                        doc:    (**inner).clone(),
                    });
                } else {
                    stack.push(Cmd {
                        indent: cmd.indent,
                        mode:   Mode::Break,
                        doc:    (**inner).clone(),
                    });
                }
            }

            Doc::Align(ref inner) => {
                // Align sets the indent to the current column.
                stack.push(Cmd {
                    indent: col,
                    mode:   cmd.mode,
                    doc:    (**inner).clone(),
                });
            }

            Doc::IfBreak { ref flat, ref broken } => {
                match cmd.mode {
                    Mode::Flat => {
                        stack.push(Cmd {
                            indent: cmd.indent,
                            mode:   cmd.mode,
                            doc:    (**flat).clone(),
                        });
                    }
                    Mode::Break => {
                        stack.push(Cmd {
                            indent: cmd.indent,
                            mode:   cmd.mode,
                            doc:    (**broken).clone(),
                        });
                    }
                }
            }
        }
    }

    // Strip trailing whitespace from each line.
    strip_trailing_whitespace(&mut out);
    out
}

/// Measure the flat-mode width of a document. Returns `Some(width)`
/// if it fits within `remaining`, or `None` if it would overflow.
fn measure_flat(doc: &Doc, remaining: usize) -> Option<usize> {
    let mut rem = remaining as isize;
    let mut stack = vec![doc];

    while let Some(d) = stack.pop() {
        if rem < 0 {
            return None;
        }
        match d {
            Doc::Empty => {}
            Doc::Text(s) => rem -= s.len() as isize,
            Doc::Line => rem -= 1, // Space in flat mode.
            Doc::HardLine => return None, // Cannot flatten.
            Doc::Nest(_, inner) => stack.push(inner),
            Doc::Group(inner) => stack.push(inner),
            Doc::Align(inner) => stack.push(inner),
            Doc::Concat(docs) => {
                for d in docs.iter().rev() {
                    stack.push(d);
                }
            }
            Doc::IfBreak { flat, .. } => stack.push(flat),
        }
    }

    if rem >= 0 { Some((remaining as isize - rem) as usize) } else { None }
}

/// Emit indentation using the indent string.
fn emit_indent(
    out:        &mut String,
    columns:    usize,
    indent_str: &str,
    indent_w:   usize,
) {
    if indent_w == 0 {
        return;
    }
    let full = columns / indent_w;
    let frac = columns % indent_w;
    for _ in 0..full {
        out.push_str(indent_str);
    }
    for _ in 0..frac {
        out.push(' ');
    }
}

/// Remove trailing whitespace from every line in the buffer.
fn strip_trailing_whitespace(s: &mut String) {
    let bytes = s.as_bytes();
    let mut result = String::with_capacity(bytes.len());
    for line in s.split('\n') {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(line.trim_end());
    }
    *s = result;
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmt::doc::*;

    #[test]
    fn test_simple_text() {
        let d = text("hello");
        let out = render(80, "    ", &d);
        assert_eq!(out, "hello");
    }

    #[test]
    fn test_concat() {
        let d = concat(vec![text("hello"), text(" "), text("world")]);
        let out = render(80, "    ", &d);
        assert_eq!(out, "hello world");
    }

    #[test]
    fn test_group_fits() {
        // Group that fits on one line.
        let d = group(concat(vec![text("a"), line(), text("b"), line(), text("c")]));
        let out = render(80, "    ", &d);
        assert_eq!(out, "a b c");
    }

    #[test]
    fn test_group_breaks() {
        // Group that does not fit on one line.
        let d = group(concat(vec![text("aaaa"), line(), text("bbbb"), line(), text("cccc")]));
        let out = render(10, "    ", &d);
        assert_eq!(out, "aaaa\nbbbb\ncccc");
    }

    #[test]
    fn test_nest() {
        let d = group(concat(vec![
            text("fn foo("),
            nest(4, concat(vec![line(), text("x: u32,"), line(), text("y: u32,")])),
            line(),
            text(")"),
        ]));
        // Too wide for 20 columns.
        let out = render(20, "    ", &d);
        assert_eq!(out, "fn foo(\n    x: u32,\n    y: u32,\n)");
    }

    #[test]
    fn test_nest_fits() {
        let d = group(concat(vec![
            text("fn foo("),
            nest(4, concat(vec![line(), text("x: u32")])),
            line(),
            text(")"),
        ]));
        let out = render(80, "    ", &d);
        assert_eq!(out, "fn foo( x: u32 )");
    }

    #[test]
    fn test_hardline() {
        let d = concat(vec![text("a"), hardline(), text("b")]);
        let out = render(80, "    ", &d);
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn test_if_break() {
        let comma = if_break(empty(), text(","));
        let d = group(concat(vec![
            text("foo("),
            nest(4, concat(vec![line(), text("a"), comma.clone()])),
            line(),
            text(")"),
        ]));
        // Fits: no trailing comma.
        let flat = render(80, "    ", &d);
        assert_eq!(flat, "foo( a )");
        // Breaks: trailing comma.
        let broken = render(5, "    ", &d);
        assert_eq!(broken, "foo(\n    a,\n)");
    }

    #[test]
    fn test_align() {
        let d = concat(vec![
            text("let x = "),
            align(concat(vec![text("foo"), line(), text("bar"), line(), text("baz")])),
        ]);
        // When broken, continuation aligns to column after "let x = ".
        let out = render(15, "    ", &d);
        assert_eq!(out, "let x = foo\n        bar\n        baz");
    }

    #[test]
    fn test_fn_signature_rust_style() {
        // Simulates:
        //   fn generate_random_string(len: usize, charset: &str) -> String {
        // or when broken:
        //   fn generate_random_string(
        //       len: usize,
        //       charset: &str,
        //   ) -> String {
        let params = join(concat(vec![text(","), line()]), vec![
            text("len: usize"),
            text("charset: &str"),
        ]);
        let d = group(concat(vec![
            text("fn generate_random_string("),
            nest(4, concat(vec![line(), params, trailing_comma()])),
            line(),
            text(") -> String {"),
        ]));

        // Fits on one line.
        let flat = render(100, "    ", &d);
        assert_eq!(flat, "fn generate_random_string( len: usize, charset: &str ) -> String {");

        // Broken.
        let broken = render(40, "    ", &d);
        assert_eq!(broken, "fn generate_random_string(\n    len: usize,\n    charset: &str,\n) -> String {");
    }

    #[test]
    fn test_struct_fields() {
        // Struct with fields.
        let fields = join(concat(vec![text(","), hardline()]), vec![
            text("date: CalendarDate"),
            text("time: ClockTime"),
        ]);
        let d = concat(vec![
            text("pub struct CalClock {"),
            nest(4, concat(vec![hardline(), fields, text(",")])),
            hardline(),
            text("}"),
        ]);
        let out = render(80, "    ", &d);
        assert_eq!(out, "pub struct CalClock {\n    date: CalendarDate,\n    time: ClockTime,\n}");
    }

    #[test]
    fn test_nested_groups() {
        // Outer group broken, inner group fits.
        let inner = group(concat(vec![text("a"), line(), text("b")]));
        let d = group(concat(vec![
            text("outer("),
            nest(4, concat(vec![line(), inner, text(","), line(), text("c")])),
            line(),
            text(")"),
        ]));
        // Width 14: outer group breaks but inner group "a b" still fits.
        let out = render(14, "    ", &d);
        assert_eq!(out, "outer(\n    a b,\n    c\n)");
    }

    #[test]
    fn test_strip_trailing_whitespace() {
        let mut s = String::from("hello   \nworld  \n  ");
        strip_trailing_whitespace(&mut s);
        assert_eq!(s, "hello\nworld\n");
    }
}
