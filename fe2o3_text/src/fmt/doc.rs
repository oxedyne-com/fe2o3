//! Layout algebra for code formatting.
//!
//! Based on Wadler's "A Prettier Printer" (2003) with extensions for
//! column alignment. The algebra has a small number of constructors
//! that can express any formatting pattern:
//!
//! - `Text` — literal text, never broken.
//! - `Line` — a potential line break (rendered as a space when the
//!   enclosing `Group` fits on one line, a newline otherwise).
//! - `HardLine` — an unconditional line break.
//! - `Nest` — increase indentation for the nested document.
//! - `Group` — try to fit the contents on one line; break if too wide.
//! - `Concat` — sequential composition.
//! - `Align` — align continuation lines to the current column.
//! - `IfBreak` — choose between two documents depending on whether
//!   the enclosing group was broken.
//!

/// A layout document. Constructed with the free functions below,
/// then rendered to a string by the renderer.
#[derive(Clone, Debug)]
pub enum Doc {
    /// Nothing.
    Empty,
    /// Literal text (must not contain newlines).
    Text(String),
    /// A potential line break. Rendered as a single space when the
    /// enclosing `Group` fits, or a newline + indentation otherwise.
    Line,
    /// An unconditional line break.
    HardLine,
    /// Increase indentation by `n` spaces for the nested document.
    Nest(u16, Box<Doc>),
    /// Try to fit everything on one line. If it doesn't fit within
    /// the remaining width, break at every `Line` inside.
    Group(Box<Doc>),
    /// Sequential composition.
    Concat(Vec<Doc>),
    /// Align continuation lines to the current column position.
    Align(Box<Doc>),
    /// Emit `flat` when the enclosing group is flat (fits on one
    /// line), `broken` when the group was broken across lines.
    IfBreak {
        flat:   Box<Doc>,
        broken: Box<Doc>,
    },
}

// ── Constructors ─────────────────────────────────────────────────

/// Empty document.
pub fn empty() -> Doc { Doc::Empty }

/// Literal text (must not contain newlines).
pub fn text<S: Into<String>>(s: S) -> Doc { Doc::Text(s.into()) }

/// A potential line break (space when flat, newline when broken).
pub fn line() -> Doc { Doc::Line }

/// An unconditional line break.
pub fn hardline() -> Doc { Doc::HardLine }

/// A soft line break: nothing when the group is flat, a newline
/// when the group breaks. Use inside brackets: `(softline body softline)`.
pub fn softline() -> Doc {
    Doc::IfBreak {
        flat:   Box::new(Doc::Empty),
        broken: Box::new(Doc::Line),
    }
}

/// Increase indentation for the nested document.
pub fn nest(indent: u16, doc: Doc) -> Doc {
    Doc::Nest(indent, Box::new(doc))
}

/// Try to fit the document on one line.
pub fn group(doc: Doc) -> Doc {
    Doc::Group(Box::new(doc))
}

/// Align continuation lines to the current column.
pub fn align(doc: Doc) -> Doc {
    Doc::Align(Box::new(doc))
}

/// Emit different documents depending on whether the enclosing
/// group was broken.
pub fn if_break(flat: Doc, broken: Doc) -> Doc {
    Doc::IfBreak {
        flat:   Box::new(flat),
        broken: Box::new(broken),
    }
}

/// Concatenate a sequence of documents.
pub fn concat(docs: Vec<Doc>) -> Doc {
    // Flatten nested concats and remove empties.
    let mut flat = Vec::new();
    for d in docs {
        match d {
            Doc::Empty => {}
            Doc::Concat(inner) => flat.extend(inner),
            other => flat.push(other),
        }
    }
    match flat.len() {
        0 => Doc::Empty,
        1 => flat.into_iter().next().unwrap_or(Doc::Empty),
        _ => Doc::Concat(flat),
    }
}

/// Concatenate two documents.
pub fn cat(a: Doc, b: Doc) -> Doc {
    concat(vec![a, b])
}

// ── Convenience combinators ──────────────────────────────────────

/// Join documents with a separator between each pair.
pub fn join(sep: Doc, docs: Vec<Doc>) -> Doc {
    let mut parts = Vec::with_capacity(docs.len() * 2);
    let mut first = true;
    for d in docs {
        if !first {
            parts.push(sep.clone());
        }
        parts.push(d);
        first = false;
    }
    concat(parts)
}

/// Join documents with `line()` between each pair (soft breaks).
pub fn join_lines(docs: Vec<Doc>) -> Doc {
    join(line(), docs)
}

/// Text followed by a space.
pub fn texts<S: Into<String>>(s: S) -> Doc {
    cat(text(s), text(" "))
}

/// Surround a document with left and right text, indenting the body.
/// Typically used for bracketed constructs: `surround("(", ")", 4, body)`.
pub fn surround(
    left:   &str,
    right:  &str,
    indent: u16,
    body:   Doc,
) -> Doc {
    group(concat(vec![
        text(left),
        nest(indent, concat(vec![line(), body])),
        line(),
        text(right),
    ]))
}

/// Like `surround` but with a hardline before the closing bracket,
/// producing the "tall" layout even when the group fits.
pub fn surround_hard(
    left:   &str,
    right:  &str,
    indent: u16,
    body:   Doc,
) -> Doc {
    concat(vec![
        text(left),
        nest(indent, concat(vec![hardline(), body])),
        hardline(),
        text(right),
    ])
}

/// A trailing comma: present when broken, absent when flat.
pub fn trailing_comma() -> Doc {
    if_break(empty(), text(","))
}
