//! Semantic-preservation regression for the annealer.
//!
//! A formatter may only change trivia (whitespace, comment layout);
//! it must never alter the significant token stream. This test lexes
//! every `.rs` file in the workspace, formats it, lexes the output,
//! and asserts the two significant-token sequences are identical.
//!
//! This is the strongest correctness bar for a formatter: if the
//! token stream survives, the code cannot have changed meaning.

use oxedyne_fe2o3_text::fmt::{
    format_rust,
    lex::{
        lex,
        rust_tokens,
    },
    cst::{
        Token,
        TokenKind,
        Trivia,
    },
    spec::FormatSpec,
};

/// A token reduced to its semantically-significant identity.
///
/// Plain comments are not tokens -- the lexer attaches them to the
/// following token as `Trivia` -- so they are lifted into this stream
/// as `Comment` entries. Without that, a formatter could delete every
/// `//` line in the file and this test would still pass.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sig {
    /// A significant token.
    Tok {
        kind: TokenKind,
        text: String,
    },
    /// A plain `//` or `/* */` comment, right-trimmed.
    Comment(String),
}

impl Sig {
    /// The text, for divergence reporting.
    fn text(&self) -> &str {
        match self {
            Self::Tok { text, .. }  => text,
            Self::Comment(s)        => s,
        }
    }
}

/// Lift a token's leading comments into the comparable stream.
///
/// Whitespace and newlines are skipped: they are exactly what a
/// formatter exists to change. Comment text is right-trimmed, since
/// trailing spaces inside a comment carry no meaning.
fn push_comments(out: &mut Vec<Sig>, tok: &Token) {
    for t in &tok.leading_trivia {
        match t {
            Trivia::LineComment(s) | Trivia::BlockComment(s) => {
                out.push(Sig::Comment(s.trim_end().to_string()));
            }
            Trivia::Whitespace(_) | Trivia::Newline => {}
        }
    }
}

/// Reduce a token stream to a semantically-comparable sequence.
///
/// Two normalisations remove changes that a formatter is entitled to
/// make without altering meaning, so that what remains is genuine
/// corruption:
///
/// - **Comment trailing whitespace** — a doc comment's trailing spaces
///   carry no meaning, so comment text is right-trimmed.
/// - **Trailing commas** — a comma immediately before a closing
///   delimiter (`)`, `]`, `}`) is insignificant in Rust, so it is
///   dropped from both sequences.
fn sig(tokens: &[Token]) -> Vec<Sig> {
    // First pass: drop EOF, right-trim comment text, and split
    // angle-bracket-run operators (`>>`, `<<`) into individual
    // brackets. The latter makes `Foo<Bar<T> >` and `Foo<Bar<T>>`
    // compare equal: removing the space between two closing generics
    // is a legitimate, meaning-preserving formatting change that only
    // shifts a token boundary (`>` `>` becomes the single `>>`).
    let mut raw: Vec<Sig> = Vec::with_capacity(tokens.len());
    for t in tokens.iter() {
        // Comments precede the token they are attached to, including
        // the EOF token -- a comment at the end of a file hangs off
        // EOF and would otherwise be dropped here.
        push_comments(&mut raw, t);
        if matches!(t.kind, TokenKind::Eof) {
            continue;
        }
        match &t.kind {
            TokenKind::DocComment(s) => raw.push(Sig::Tok {
                kind: TokenKind::DocComment(s.trim_end().to_string()),
                text: t.text.trim_end().to_string(),
            }),
            TokenKind::Operator(op)
                if !op.is_empty()
                    && (op.bytes().all(|b| b == b'>') || op.bytes().all(|b| b == b'<')) =>
            {
                let ch = op.as_bytes()[0] as char;
                for _ in 0..op.len() {
                    raw.push(Sig::Tok { kind: TokenKind::Punct(ch), text: ch.to_string() });
                }
            }
            // A number token with a trailing dot (e.g. `0.`) arises when
            // the formatter puts a space after a tuple-index dot
            // (`self.0.buf` rendered as `self. 0. buf`): the `0.` then
            // lexes as a float. Split it back into the digits plus a `.`
            // Punct so it matches the source's `0` `.` sequence. This
            // is a *split*, not a strip, so a genuine float-to-integer
            // change (`0.` becoming `0`) would still be caught.
            TokenKind::Number if t.text.len() > 1 && t.text.ends_with('.') => {
                raw.push(Sig::Tok {
                    kind: TokenKind::Number,
                    text: t.text[..t.text.len() - 1].to_string(),
                });
                raw.push(Sig::Tok { kind: TokenKind::Punct('.'), text: ".".to_string() });
            }
            _ => raw.push(Sig::Tok { kind: t.kind.clone(), text: t.text.clone() }),
        }
    }

    raw
}

/// The significant tokens alone, with insignificant trailing commas
/// removed.
///
/// The comma drop runs here, on the comment-free sequence, so that a
/// comment sitting between a comma and its closing delimiter cannot
/// change whether that comma is judged insignificant.
fn code_sig(tokens: &[Token]) -> Vec<Sig> {
    let raw: Vec<Sig> = sig(tokens)
        .into_iter()
        .filter(|s| !matches!(s, Sig::Comment(_)))
        .collect();
    let mut out: Vec<Sig> = Vec::with_capacity(raw.len());
    for i in 0..raw.len() {
        if raw[i] == (Sig::Tok { kind: TokenKind::Punct(','), text: ",".to_string() }) {
            if let Some(Sig::Tok { kind, .. }) = raw.get(i + 1) {
                if matches!(
                    kind,
                    TokenKind::Punct(')') | TokenKind::Punct(']') | TokenKind::Punct('}')
                ) {
                    continue;
                }
            }
        }
        out.push(raw[i].clone());
    }
    out
}

/// The comments alone, in order.
fn comment_sig(tokens: &[Token]) -> Vec<Sig> {
    sig(tokens)
        .into_iter()
        .filter(|s| matches!(s, Sig::Comment(_)))
        .collect()
}

/// Report a single file's semantic divergence, if any.
struct Divergence {
    file:   String,
    index:  usize,
    before: Option<Sig>,
    after:  Option<Sig>,
    context: String,
}

#[test]
fn test_workspace_semantic_preservation() {
    use std::fs;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let spec = FormatSpec::fe2o3();
    let lang = rust_tokens();

    let mut checked = 0usize;
    let mut lex_errs = 0usize;
    let mut fmt_errs = 0usize;
    let mut code_diffs = 0usize;
    let mut comment_diffs = 0usize;
    let mut diffs: Vec<Divergence> = Vec::new();

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
                // `.claude` holds agent worktrees: whole duplicate
                // copies of this tree, which would be checked again
                // under a second name.
                if name == "target" || name == ".git" || name == ".claude" {
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
            if source.trim().is_empty() {
                continue;
            }
            let rel = path.strip_prefix(&root).unwrap_or(&path)
                .display().to_string();

            let src_toks = match lex(&source, &lang) {
                Ok(t) => t,
                Err(_) => { lex_errs += 1; continue; }
            };
            let formatted = match format_rust(&source, &spec) {
                Ok(s) => s,
                Err(_) => { fmt_errs += 1; continue; }
            };
            let out_toks = match lex(&formatted, &lang) {
                Ok(t) => t,
                Err(_) => { lex_errs += 1; continue; }
            };

            let a_code = code_sig(&src_toks);
            let b_code = code_sig(&out_toks);
            let a_com  = comment_sig(&src_toks);
            let b_com  = comment_sig(&out_toks);
            checked += 1;

            // Two separate guarantees, because they carry different
            // weight. Losing a comment, or reordering one, is
            // corruption. Moving one between the end of a line and the
            // line above is layout, which is what a formatter is for.
            if a_code != b_code {
                code_diffs += 1;
                println!("  CODE DIVERGE {}", rel);
            }
            if a_com != b_com {
                comment_diffs += 1;
                println!(
                    "  COMMENT DIVERGE {} ({} before, {} after)",
                    rel, a_com.len(), b_com.len(),
                );
            }
            let (a, b) = (a_code, b_code);
            if a != b {
                // Locate the first divergence.
                let mut idx = 0;
                while idx < a.len() && idx < b.len() && a[idx] == b[idx] {
                    idx += 1;
                }
                // A little context: the three significant tokens before.
                let lo = idx.saturating_sub(3);
                let ctx: Vec<String> = a[lo..idx]
                    .iter()
                    .map(|s| s.text().to_string())
                    .collect();
                diffs.push(Divergence {
                    file:    rel,
                    index:   idx,
                    before:  a.get(idx).cloned(),
                    after:   b.get(idx).cloned(),
                    context: ctx.join(" "),
                });
            }
        }
    }

    println!(
        "Semantic check: {} files, {} code-divergences, {} comment-divergences, \
{} layout moves, {} lex-errs, {} refused.",
        checked, code_diffs, comment_diffs,
        diffs.len().saturating_sub(code_diffs + comment_diffs),
        lex_errs, fmt_errs,
    );
    for d in diffs.iter().take(20) {
        println!(
            "  DIVERGE {} @tok {} after [{}]:\n     source: {:?}\n     output: {:?}",
            d.file, d.index, d.context, d.before, d.after,
        );
    }

    // A formatter may move a comment between the end of a line and the
    // line above it -- that is layout. It may never lose one, duplicate
    // one, reorder them, or change a single significant token. Those
    // are the two assertions worth making, and the reason the earlier
    // token-only check passed while comments were being deleted.
    assert_eq!(
        code_diffs, 0,
        "{} file(s) had their significant token stream altered",
        code_diffs,
    );
    assert_eq!(
        comment_diffs, 0,
        "{} file(s) had a comment lost, added or reordered",
        comment_diffs,
    );
}
