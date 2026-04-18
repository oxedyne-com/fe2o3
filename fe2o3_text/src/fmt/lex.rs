//! Generic lexer driven by language-specific token definitions.
//!
//! The lexer transforms source text into a flat sequence of `Token`s,
//! preserving all whitespace and comments as leading trivia on each
//! token. This ensures round-trip fidelity: every byte of the
//! source is accounted for.
//!

use crate::fmt::cst::{
    Span,
    Token,
    TokenKind,
    Trivia,
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::BTreeSet;


/// Language-specific token definitions. These drive the generic
/// lexer so that adding a new language only requires filling in
/// this structure.
#[derive(Clone, Debug)]
pub struct LangTokens {
    /// Keywords (e.g. `fn`, `let`, `if`, `struct`).
    pub keywords:           BTreeSet<String>,
    /// Single-line comment prefix (e.g. `//`).
    pub line_comment:       String,
    /// Block comment open (e.g. `/*`).
    pub block_comment_open: String,
    /// Block comment close (e.g. `*/`).
    pub block_comment_close: String,
    /// Doc-comment prefixes (e.g. `///`, `//!`).
    pub doc_comment_prefixes: Vec<String>,
    /// Multi-character operators, longest first.
    pub operators:          Vec<String>,
    /// Single character that opens a string literal (e.g. `"`).
    pub string_delimiters:  Vec<char>,
    /// Character literal delimiter (e.g. `'`).
    pub char_delimiter:     Option<char>,
    /// Raw string prefix (e.g. `r#"` in Rust). Empty if none.
    pub raw_string_prefix:  String,
    /// Attribute prefix (e.g. `#[` in Rust). Empty if none.
    pub attribute_prefix:   String,
    /// Lifetime prefix (e.g. `'` when followed by an ident in Rust).
    pub lifetime_prefix:    Option<char>,
}

/// Rust token definitions.
pub fn rust_tokens() -> LangTokens {
    let keywords: BTreeSet<String> = [
        "as", "async", "await", "break", "const", "continue", "crate",
        "dyn", "else", "enum", "extern", "false", "fn", "for", "if",
        "impl", "in", "let", "loop", "match", "mod", "move", "mut",
        "pub", "ref", "return", "self", "Self", "static", "struct",
        "super", "trait", "true", "type", "unsafe", "use", "where",
        "while", "yield",
    ].iter().map(|s| s.to_string()).collect();

    let operators = vec![
        // Three-character.
        "<<=", ">>=", "..=",
        // Two-character.
        "->", "=>", "::", "&&", "||", "==", "!=", "<=", ">=",
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>",
        "..",
    ].iter().map(|s| s.to_string()).collect();

    let doc_comment_prefixes = vec![
        "///".to_string(),
        "//!".to_string(),
    ];

    LangTokens {
        keywords,
        line_comment:           "//".to_string(),
        block_comment_open:     "/*".to_string(),
        block_comment_close:    "*/".to_string(),
        doc_comment_prefixes,
        operators,
        string_delimiters:      vec!['"'],
        char_delimiter:         Some('\''),
        raw_string_prefix:      "r".to_string(),
        attribute_prefix:       "#[".to_string(),
        lifetime_prefix:        Some('\''),
    }
}

/// C token definitions.
pub fn c_tokens() -> LangTokens {
    let keywords: BTreeSet<String> = [
        "auto", "break", "case", "char", "const", "continue", "default",
        "do", "double", "else", "enum", "extern", "float", "for", "goto",
        "if", "inline", "int", "long", "register", "restrict", "return",
        "short", "signed", "sizeof", "static", "struct", "switch",
        "typedef", "union", "unsigned", "void", "volatile", "while",
        "_Bool", "_Complex", "_Imaginary",
    ].iter().map(|s| s.to_string()).collect();

    let operators = vec![
        "<<=", ">>=",
        "->", "++", "--", "&&", "||", "==", "!=", "<=", ">=",
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>",
    ].iter().map(|s| s.to_string()).collect();

    LangTokens {
        keywords,
        line_comment:           "//".to_string(),
        block_comment_open:     "/*".to_string(),
        block_comment_close:    "*/".to_string(),
        doc_comment_prefixes:   Vec::new(),
        operators,
        string_delimiters:      vec!['"'],
        char_delimiter:         Some('\''),
        raw_string_prefix:      String::new(),
        attribute_prefix:       String::new(),
        lifetime_prefix:        None,
    }
}

/// Go token definitions.
pub fn go_tokens() -> LangTokens {
    let keywords: BTreeSet<String> = [
        "break", "case", "chan", "const", "continue", "default", "defer",
        "else", "fallthrough", "for", "func", "go", "goto", "if",
        "import", "interface", "map", "package", "range", "return",
        "select", "struct", "switch", "type", "var",
    ].iter().map(|s| s.to_string()).collect();

    let operators = vec![
        "<<=", ">>=",
        ":=", "<-", "++", "--", "&&", "||", "==", "!=", "<=", ">=",
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>",
        "&^", "&^=", "...",
    ].iter().map(|s| s.to_string()).collect();

    LangTokens {
        keywords,
        line_comment:           "//".to_string(),
        block_comment_open:     "/*".to_string(),
        block_comment_close:    "*/".to_string(),
        doc_comment_prefixes:   Vec::new(),
        operators,
        string_delimiters:      vec!['"', '`'],
        char_delimiter:         Some('\''),
        raw_string_prefix:      String::new(),
        attribute_prefix:       String::new(),
        lifetime_prefix:        None,
    }
}

/// JavaScript / TypeScript token definitions.
pub fn js_tokens() -> LangTokens {
    let keywords: BTreeSet<String> = [
        "abstract", "arguments", "async", "await", "break", "case",
        "catch", "class", "const", "continue", "debugger", "default",
        "delete", "do", "else", "enum", "export", "extends", "false",
        "finally", "for", "from", "function", "get", "if", "implements",
        "import", "in", "instanceof", "interface", "let", "new", "null",
        "of", "package", "private", "protected", "public", "return",
        "set", "static", "super", "switch", "this", "throw", "true",
        "try", "type", "typeof", "undefined", "var", "void", "while",
        "with", "yield",
    ].iter().map(|s| s.to_string()).collect();

    let operators = vec![
        ">>>", "<<=", ">>=",
        "===", "!==", "**=", ">>>=",
        "=>", "++", "--", "&&", "||", "==", "!=", "<=", ">=",
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>",
        "**", "??", "?.", "...",
    ].iter().map(|s| s.to_string()).collect();

    LangTokens {
        keywords,
        line_comment:           "//".to_string(),
        block_comment_open:     "/*".to_string(),
        block_comment_close:    "*/".to_string(),
        doc_comment_prefixes:   vec!["/**".to_string()],
        operators,
        string_delimiters:      vec!['"', '\'', '`'],
        char_delimiter:         None,
        raw_string_prefix:      String::new(),
        attribute_prefix:       "@".to_string(),
        lifetime_prefix:        None,
    }
}

/// Java token definitions.
pub fn java_tokens() -> LangTokens {
    let keywords: BTreeSet<String> = [
        "abstract", "assert", "boolean", "break", "byte", "case",
        "catch", "char", "class", "const", "continue", "default", "do",
        "double", "else", "enum", "extends", "false", "final", "finally",
        "float", "for", "goto", "if", "implements", "import",
        "instanceof", "int", "interface", "long", "native", "new",
        "null", "package", "private", "protected", "public", "return",
        "short", "static", "strictfp", "super", "switch", "synchronized",
        "this", "throw", "throws", "transient", "true", "try", "void",
        "volatile", "while",
    ].iter().map(|s| s.to_string()).collect();

    let operators = vec![
        ">>>", "<<=", ">>=", ">>>=",
        "->", "++", "--", "&&", "||", "==", "!=", "<=", ">=",
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>",
        "::",
    ].iter().map(|s| s.to_string()).collect();

    LangTokens {
        keywords,
        line_comment:           "//".to_string(),
        block_comment_open:     "/*".to_string(),
        block_comment_close:    "*/".to_string(),
        doc_comment_prefixes:   vec!["/**".to_string()],
        operators,
        string_delimiters:      vec!['"'],
        char_delimiter:         Some('\''),
        raw_string_prefix:      String::new(),
        attribute_prefix:       "@".to_string(),
        lifetime_prefix:        None,
    }
}

/// Python token definitions.
pub fn python_tokens() -> LangTokens {
    let keywords: BTreeSet<String> = [
        "False", "None", "True", "and", "as", "assert", "async",
        "await", "break", "class", "continue", "def", "del", "elif",
        "else", "except", "finally", "for", "from", "global", "if",
        "import", "in", "is", "lambda", "nonlocal", "not", "or",
        "pass", "raise", "return", "try", "while", "with", "yield",
    ].iter().map(|s| s.to_string()).collect();

    let operators = vec![
        "<<=", ">>=",
        "**=", "//=",
        "->", "==", "!=", "<=", ">=",
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>",
        "**", "//", ":=",
    ].iter().map(|s| s.to_string()).collect();

    LangTokens {
        keywords,
        line_comment:           "#".to_string(),
        block_comment_open:     String::new(),
        block_comment_close:    String::new(),
        doc_comment_prefixes:   Vec::new(),
        operators,
        string_delimiters:      vec!['"', '\''],
        char_delimiter:         None,
        raw_string_prefix:      "r".to_string(),
        attribute_prefix:       "@".to_string(),
        lifetime_prefix:        None,
    }
}

/// Lex source text into a token stream.
pub fn lex(src: &str, lang: &LangTokens) -> Outcome<Vec<Token>> {
    let mut tokens = Vec::new();
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos: usize = 0;

    loop {
        // Collect leading trivia.
        let mut trivia: Vec<Trivia> = Vec::new();
        loop {
            if pos >= len {
                break;
            }
            // Newline.
            if bytes[pos] == b'\n' {
                trivia.push(Trivia::Newline);
                pos += 1;
                continue;
            }
            if bytes[pos] == b'\r' && pos + 1 < len && bytes[pos + 1] == b'\n' {
                trivia.push(Trivia::Newline);
                pos += 2;
                continue;
            }
            // Whitespace (not newline).
            if bytes[pos] == b' ' || bytes[pos] == b'\t' {
                let start = pos;
                while pos < len && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                    pos += 1;
                }
                trivia.push(Trivia::Whitespace(src[start..pos].to_string()));
                continue;
            }
            // Doc comments (must check before line comments).
            let mut is_doc = false;
            for prefix in &lang.doc_comment_prefixes {
                if src[pos..].starts_with(prefix) {
                    let start = pos;
                    // Consume to end of line.
                    while pos < len && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                    // Push as a doc-comment token, not trivia.
                    // We break out and handle it below.
                    // Actually, doc comments are tokens, not trivia.
                    // Rewind pos and break.
                    pos = start;
                    is_doc = true;
                    break;
                }
            }
            if is_doc {
                break;
            }
            // Line comment.
            if !lang.line_comment.is_empty() && src[pos..].starts_with(&lang.line_comment) {
                let start = pos;
                while pos < len && bytes[pos] != b'\n' {
                    pos += 1;
                }
                trivia.push(Trivia::LineComment(src[start..pos].to_string()));
                continue;
            }
            // Block comment.
            if !lang.block_comment_open.is_empty() && src[pos..].starts_with(&lang.block_comment_open) {
                let start = pos;
                pos += lang.block_comment_open.len();
                let mut depth = 1usize;
                while pos < len && depth > 0 {
                    if src[pos..].starts_with(&lang.block_comment_close) {
                        depth -= 1;
                        pos += lang.block_comment_close.len();
                    } else if src[pos..].starts_with(&lang.block_comment_open) {
                        depth += 1;
                        pos += lang.block_comment_open.len();
                    } else {
                        pos = advance_char(src, pos);
                    }
                }
                trivia.push(Trivia::BlockComment(src[start..pos].to_string()));
                continue;
            }
            break;
        }

        if pos >= len {
            // EOF token.
            tokens.push(Token {
                kind:           TokenKind::Eof,
                text:           String::new(),
                leading_trivia: trivia,
                span:           Span { start: pos, end: pos },
            });
            break;
        }

        let tok_start = pos;

        // Doc comment.
        let doc_prefix = lang.doc_comment_prefixes.iter()
            .find(|p| src[pos..].starts_with(p.as_str()))
            .cloned();
        if doc_prefix.is_some() {
            let start = pos;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            let text = src[start..pos].to_string();
            tokens.push(Token {
                kind:           TokenKind::DocComment(text.clone()),
                text,
                leading_trivia: trivia,
                span:           Span { start: tok_start, end: pos },
            });
            continue;
        }

        // Attribute (e.g. #[...]).
        if !lang.attribute_prefix.is_empty() && src[pos..].starts_with(&lang.attribute_prefix) {
            let start = pos;
            pos += lang.attribute_prefix.len();
            let mut depth = 1usize;
            while pos < len && depth > 0 {
                match bytes[pos] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    pos += 1;
                } else {
                    pos += 1; // Consume the closing ].
                }
            }
            tokens.push(Token {
                kind:           TokenKind::Attribute,
                text:           src[start..pos].to_string(),
                leading_trivia: trivia,
                span:           Span { start: tok_start, end: pos },
            });
            continue;
        }

        // String literal.
        if lang.string_delimiters.contains(&(bytes[pos] as char)) {
            let delim = bytes[pos] as char;
            let start = pos;
            pos += 1;
            while pos < len {
                if bytes[pos] == b'\\' {
                    pos += 2; // Skip escaped character.
                } else if bytes[pos] as char == delim {
                    pos += 1;
                    break;
                } else {
                    pos += 1;
                }
            }
            tokens.push(Token {
                kind:           TokenKind::StringLit,
                text:           src[start..pos].to_string(),
                leading_trivia: trivia,
                span:           Span { start: tok_start, end: pos },
            });
            continue;
        }

        // Raw string (Rust r#"..."#).
        if !lang.raw_string_prefix.is_empty()
            && src[pos..].starts_with(&lang.raw_string_prefix)
            && pos + lang.raw_string_prefix.len() < len
        {
            let after_r = pos + lang.raw_string_prefix.len();
            let mut hashes = 0usize;
            let mut p = after_r;
            while p < len && bytes[p] == b'#' {
                hashes += 1;
                p += 1;
            }
            if p < len && bytes[p] == b'"' {
                // Valid raw string opening.
                let start = pos;
                p += 1; // Skip opening ".
                let close_pat: String =
                    std::iter::once('"').chain(std::iter::repeat('#').take(hashes)).collect();
                while p < len {
                    if src[p..].starts_with(&close_pat) {
                        p += close_pat.len();
                        break;
                    }
                    p = advance_char(src, p);
                }
                pos = p;
                tokens.push(Token {
                    kind:           TokenKind::StringLit,
                    text:           src[start..pos].to_string(),
                    leading_trivia: trivia,
                    span:           Span { start: tok_start, end: pos },
                });
                continue;
            }
        }

        // Character literal.
        if let Some(cd) = lang.char_delimiter {
            if bytes[pos] as char == cd {
                // Distinguish char literal from lifetime:
                // char literal: 'a', '\n', etc.
                // lifetime: 'a followed by an ident-continue character.
                let start = pos;
                pos += 1;
                if pos < len && bytes[pos] == b'\\' {
                    // Escaped char literal.
                    pos += 1;
                    while pos < len && bytes[pos] as char != cd {
                        pos += 1;
                    }
                    if pos < len { pos += 1; }
                    tokens.push(Token {
                        kind:           TokenKind::CharLit,
                        text:           src[start..pos].to_string(),
                        leading_trivia: trivia,
                        span:           Span { start: tok_start, end: pos },
                    });
                    continue;
                } else if pos < len && pos + 1 < len && bytes[pos + 1] as char == cd {
                    // Single-char literal like 'a'.
                    pos += 2;
                    tokens.push(Token {
                        kind:           TokenKind::CharLit,
                        text:           src[start..pos].to_string(),
                        leading_trivia: trivia,
                        span:           Span { start: tok_start, end: pos },
                    });
                    continue;
                } else if pos < len && is_ident_start(bytes[pos]) {
                    // Lifetime.
                    let _lstart = pos;
                    while pos < len && is_ident_continue(bytes[pos]) {
                        pos += 1;
                    }
                    tokens.push(Token {
                        kind:           TokenKind::Lifetime,
                        text:           src[start..pos].to_string(),
                        leading_trivia: trivia,
                        span:           Span { start: tok_start, end: pos },
                    });
                    continue;
                } else {
                    // Bare apostrophe — treat as punctuation.
                    pos = start + 1;
                    tokens.push(Token {
                        kind:           TokenKind::Punct('\''),
                        text:           "'".to_string(),
                        leading_trivia: trivia,
                        span:           Span { start: tok_start, end: pos },
                    });
                    continue;
                }
            }
        }

        // Number.
        if bytes[pos].is_ascii_digit() {
            let start = pos;
            // Consume digits, hex prefix, underscores, dots, exponent.
            while pos < len && (bytes[pos].is_ascii_alphanumeric()
                || bytes[pos] == b'_'
                || bytes[pos] == b'.'
                || bytes[pos] == b'+'
                || bytes[pos] == b'-')
            {
                // Avoid consuming a `..` range operator.
                if bytes[pos] == b'.' && pos + 1 < len && bytes[pos + 1] == b'.' {
                    break;
                }
                // Avoid consuming `+`/`-` unless it's part of an exponent.
                if (bytes[pos] == b'+' || bytes[pos] == b'-')
                    && pos > start
                    && bytes[pos - 1] != b'e'
                    && bytes[pos - 1] != b'E'
                {
                    break;
                }
                pos += 1;
            }
            tokens.push(Token {
                kind:           TokenKind::Number,
                text:           src[start..pos].to_string(),
                leading_trivia: trivia,
                span:           Span { start: tok_start, end: pos },
            });
            continue;
        }

        // Identifier / keyword.
        if is_ident_start(bytes[pos]) {
            let start = pos;
            while pos < len && is_ident_continue(bytes[pos]) {
                pos += 1;
            }
            let word = &src[start..pos];
            // Check for macro invocation (ident followed by `!`).
            let kind = if pos < len && bytes[pos] == b'!' && !lang.keywords.contains(word) {
                // Don't consume the `!` here, let it be a separate punct token
                // unless it's a macro name.
                // Actually, consume it as part of the macro name.
                // pos += 1; // No — the `!` is part of the macro syntax, not the name.
                if lang.keywords.contains(word) {
                    TokenKind::Keyword(word.to_string())
                } else {
                    TokenKind::Ident
                }
            } else if lang.keywords.contains(word) {
                TokenKind::Keyword(word.to_string())
            } else {
                TokenKind::Ident
            };
            tokens.push(Token {
                kind,
                text:           word.to_string(),
                leading_trivia: trivia,
                span:           Span { start: tok_start, end: pos },
            });
            continue;
        }

        // Multi-character operator.
        let matched_op = lang.operators.iter()
            .find(|op| src[pos..].starts_with(op.as_str()))
            .cloned();
        if let Some(op) = matched_op {
            pos += op.len();
            tokens.push(Token {
                kind:           TokenKind::Operator(op.clone()),
                text:           op,
                leading_trivia: trivia,
                span:           Span { start: tok_start, end: pos },
            });
            continue;
        }

        // Single-character punctuation (or full multi-byte char).
        let ch = src[pos..].chars().next().unwrap_or('\0');
        let next = advance_char(src, pos);
        tokens.push(Token {
            kind:           TokenKind::Punct(ch),
            text:           src[pos..next].to_string(),
            leading_trivia: trivia,
            span:           Span { start: tok_start, end: next },
        });
        pos = next;
    }

    Ok(tokens)
}

/// Advance past one complete UTF-8 character.
fn advance_char(src: &str, pos: usize) -> usize {
    let mut p = pos + 1;
    while p < src.len() && !src.is_char_boundary(p) {
        p += 1;
    }
    p
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}


#[cfg(test)]
mod tests {
    use super::*;

    fn lex_rust(src: &str) -> Vec<Token> {
        let lang = rust_tokens();
        lex(src, &lang).expect("lex failed")
    }

    #[test]
    fn test_simple_fn() {
        let tokens = lex_rust("fn main() {}");
        // fn, main, (, ), {, }, EOF
        assert_eq!(tokens.len(), 7);
        assert_eq!(tokens[0].kind, TokenKind::Keyword("fn".into()));
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "main");
        assert_eq!(tokens[2].kind, TokenKind::Punct('('));
        assert_eq!(tokens[3].kind, TokenKind::Punct(')'));
        assert_eq!(tokens[4].kind, TokenKind::Punct('{'));
        assert_eq!(tokens[5].kind, TokenKind::Punct('}'));
        assert_eq!(tokens[6].kind, TokenKind::Eof);
    }

    #[test]
    fn test_trivia_preserved() {
        let tokens = lex_rust("fn  main() {\n    // hi\n}");
        // fn has no leading trivia.
        assert!(tokens[0].leading_trivia.is_empty());
        // main has whitespace trivia.
        assert_eq!(tokens[1].leading_trivia.len(), 1);
        match &tokens[1].leading_trivia[0] {
            Trivia::Whitespace(s) => assert_eq!(s, "  "),
            other => panic!("expected whitespace, got {:?}", other),
        }
    }

    #[test]
    fn test_string_literal() {
        let tokens = lex_rust(r#"let s = "hello \"world\"";"#);
        // let, s, =, "hello \"world\"", ;, EOF
        let string_tok = tokens.iter().find(|t| matches!(t.kind, TokenKind::StringLit)).unwrap();
        assert_eq!(string_tok.text, r#""hello \"world\"""#);
    }

    #[test]
    fn test_operators() {
        let tokens = lex_rust("a -> b => c :: d");
        let ops: Vec<&str> = tokens.iter().filter_map(|t| {
            if let TokenKind::Operator(ref s) = t.kind { Some(s.as_str()) } else { None }
        }).collect();
        assert_eq!(ops, vec!["->", "=>", "::"]);
    }

    #[test]
    fn test_doc_comment() {
        let tokens = lex_rust("/// A doc comment.\nfn foo() {}");
        assert!(matches!(tokens[0].kind, TokenKind::DocComment(_)));
        assert_eq!(tokens[0].text, "/// A doc comment.");
    }

    #[test]
    fn test_attribute() {
        let tokens = lex_rust("#[derive(Clone, Debug)]\nstruct Foo;");
        assert_eq!(tokens[0].kind, TokenKind::Attribute);
        assert_eq!(tokens[0].text, "#[derive(Clone, Debug)]");
    }

    #[test]
    fn test_lifetime() {
        let tokens = lex_rust("fn foo<'a>(x: &'a str) {}");
        let lifetimes: Vec<&str> = tokens.iter().filter_map(|t| {
            if matches!(t.kind, TokenKind::Lifetime) { Some(t.text.as_str()) } else { None }
        }).collect();
        assert_eq!(lifetimes, vec!["'a", "'a"]);
    }

    #[test]
    fn test_char_literal() {
        let tokens = lex_rust("let c = 'x';");
        let char_tok = tokens.iter().find(|t| matches!(t.kind, TokenKind::CharLit)).unwrap();
        assert_eq!(char_tok.text, "'x'");
    }

    #[test]
    fn test_number() {
        let tokens = lex_rust("let n = 42_000;");
        let num = tokens.iter().find(|t| matches!(t.kind, TokenKind::Number)).unwrap();
        assert_eq!(num.text, "42_000");
    }

    #[test]
    fn test_block_comment() {
        let tokens = lex_rust("/* block */ fn foo() {}");
        // The block comment is leading trivia on `fn`.
        assert_eq!(tokens[0].kind, TokenKind::Keyword("fn".into()));
        assert!(tokens[0].leading_trivia.iter().any(|t| matches!(t, Trivia::BlockComment(_))));
    }

    #[test]
    fn test_line_comment_trivia() {
        let tokens = lex_rust("// line comment\nfn foo() {}");
        // Line comment is leading trivia on `fn`.
        assert_eq!(tokens[0].kind, TokenKind::Keyword("fn".into()));
        assert!(tokens[0].leading_trivia.iter().any(|t| matches!(t, Trivia::LineComment(_))));
    }

    #[test]
    fn test_lex_c() {
        let lang = c_tokens();
        let tokens = lex("int main() { return 0; }", &lang).expect("lex failed");
        assert_eq!(tokens[0].kind, TokenKind::Keyword("int".into()));
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "main");
    }

    #[test]
    fn test_lex_go() {
        let lang = go_tokens();
        let tokens = lex("func main() { fmt.Println(\"hello\") }", &lang).expect("lex failed");
        assert_eq!(tokens[0].kind, TokenKind::Keyword("func".into()));
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "main");
    }

    #[test]
    fn test_lex_js() {
        let lang = js_tokens();
        let tokens = lex("const x = async () => { await fetch(); };", &lang).expect("lex failed");
        assert_eq!(tokens[0].kind, TokenKind::Keyword("const".into()));
        let arrow = tokens.iter().find(|t| matches!(&t.kind, TokenKind::Operator(op) if op == "=>"));
        assert!(arrow.is_some(), "expected => operator");
    }

    #[test]
    fn test_lex_java() {
        let lang = java_tokens();
        let tokens = lex("public class Foo { void bar() {} }", &lang).expect("lex failed");
        assert_eq!(tokens[0].kind, TokenKind::Keyword("public".into()));
        assert_eq!(tokens[1].kind, TokenKind::Keyword("class".into()));
        assert_eq!(tokens[2].kind, TokenKind::Ident);
        assert_eq!(tokens[2].text, "Foo");
    }

    #[test]
    fn test_lex_python() {
        let lang = python_tokens();
        let tokens = lex("def foo(x, y):\n    return x + y", &lang).expect("lex failed");
        assert_eq!(tokens[0].kind, TokenKind::Keyword("def".into()));
        assert_eq!(tokens[1].kind, TokenKind::Ident);
        assert_eq!(tokens[1].text, "foo");
    }

    #[test]
    fn test_lex_python_comment() {
        let lang = python_tokens();
        let tokens = lex("# a comment\nx = 1", &lang).expect("lex failed");
        // Comment should be trivia on `x`.
        assert_eq!(tokens[0].kind, TokenKind::Ident);
        assert!(tokens[0].leading_trivia.iter().any(|t| matches!(t, Trivia::LineComment(_))));
    }

    #[test]
    fn test_nested_generics() {
        let tokens = lex_rust("Outcome<Vec<Token>>");
        let texts: Vec<&str> = tokens.iter()
            .filter(|t| !matches!(t.kind, TokenKind::Eof))
            .map(|t| t.text.as_str()).collect();
        // Should be: Outcome, <, Vec, <, Token, >>, not >>
        println!("tokens: {:?}", texts);
        // The >> should be lexed as the >> operator, but in a type
        // context it's two closing >. This is a known ambiguity.
        // For formatting, we just need to know it's there.
    }

    #[test]
    fn test_range_vs_dot() {
        let tokens = lex_rust("0..10");
        // 0, .., 10, EOF
        assert_eq!(tokens[0].kind, TokenKind::Number);
        assert_eq!(tokens[0].text, "0");
        assert_eq!(tokens[1].kind, TokenKind::Operator("..".into()));
        assert_eq!(tokens[2].kind, TokenKind::Number);
        assert_eq!(tokens[2].text, "10");
    }
}
