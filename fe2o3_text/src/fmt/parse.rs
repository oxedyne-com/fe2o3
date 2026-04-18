//! Structural parser for Rust.
//!
//! Converts a flat token stream into a concrete syntax tree by
//! recognising brackets, keywords, and their nesting structure.
//! This is a lightweight "structural" parser — it tracks bracket
//! depth and keyword patterns rather than implementing a full
//! grammar. This is enough for formatting decisions.
//!

use crate::fmt::cst::{
    CstChild,
    CstNode,
    ChildRole,
    NodeKind,
    Span,
    Token,
    TokenKind,
};

use oxedyne_fe2o3_core::prelude::*;


/// Parse a Rust token stream into a CST.
pub fn parse_rust(tokens: Vec<Token>) -> Outcome<CstNode> {
    let span = file_span(&tokens);
    let mut parser = Parser::new(tokens);
    let children = parser.parse_items();
    Ok(CstNode {
        kind: NodeKind::SourceFile,
        children,
        span,
    })
}

/// Parser state.
struct Parser {
    tokens: Vec<Token>,
    pos:    usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    /// Peek at the current token without consuming.
    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    /// Whether we have reached EOF.
    fn at_eof(&self) -> bool {
        self.pos >= self.tokens.len()
            || matches!(self.peek().kind, TokenKind::Eof)
    }

    /// Consume the current token and return it.
    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.pos].clone();
        self.pos += 1;
        tok
    }

    /// Check if the current token is a specific keyword.
    fn at_keyword(&self, kw: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Keyword(k) if k == kw)
    }

    /// Check if the current token is a specific punctuation character.
    fn at_punct(&self, ch: char) -> bool {
        matches!(&self.peek().kind, TokenKind::Punct(c) if *c == ch)
    }

    /// Peek ahead by n tokens.
    fn peek_ahead(&self, n: usize) -> &Token {
        let idx = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[idx]
    }

    // ── Top-level parsing ────────────────────────────────────

    /// Parse a sequence of top-level items.
    fn parse_items(&mut self) -> Vec<CstChild> {
        let mut children = Vec::new();
        while !self.at_eof() {
            children.extend(self.parse_item());
        }
        children
    }

    /// Parse a single item. Returns one or more CstChild entries
    /// (an item may be preceded by attributes/doc comments).
    fn parse_item(&mut self) -> Vec<CstChild> {
        // Collect doc comments and attributes.
        let mut parts: Vec<CstChild> = Vec::new();

        // Leading doc comments.
        while matches!(self.peek().kind, TokenKind::DocComment(_)) {
            parts.push(CstChild::Token(self.bump()));
        }

        // Attributes.
        while matches!(self.peek().kind, TokenKind::Attribute) {
            parts.push(CstChild::Token(self.bump()));
        }

        // Detect item kind by keyword.
        let kind = match &self.peek().kind {
            TokenKind::Keyword(k) => match k.as_str() {
                "fn"            => Some(NodeKind::FnDef),
                "pub"           => return self.parse_pub_item(parts),
                "struct"        => Some(NodeKind::StructDef),
                "enum"          => Some(NodeKind::EnumDef),
                "trait"         => Some(NodeKind::TraitDef),
                "impl"          => Some(NodeKind::ImplBlock),
                "use"           => Some(NodeKind::UseDecl),
                "mod"           => Some(NodeKind::ModDecl),
                "type"          => Some(NodeKind::TypeAlias),
                "const" | "static" => Some(NodeKind::ConstItem),
                "let"           => Some(NodeKind::LetBinding),
                _               => None,
            },
            _ => None,
        };

        match kind {
            Some(NodeKind::FnDef) => {
                let node = self.parse_fn_def(&mut parts);
                parts.push(CstChild::Node { role: ChildRole::Misc, node });
                parts
            }
            Some(NodeKind::StructDef) => {
                let node = self.parse_struct_def(&mut parts);
                parts.push(CstChild::Node { role: ChildRole::Misc, node });
                parts
            }
            Some(NodeKind::EnumDef) => {
                let node = self.parse_enum_def(&mut parts);
                parts.push(CstChild::Node { role: ChildRole::Misc, node });
                parts
            }
            Some(NodeKind::ImplBlock) => {
                let node = self.parse_impl_block(&mut parts);
                parts.push(CstChild::Node { role: ChildRole::Misc, node });
                parts
            }
            Some(NodeKind::UseDecl) => {
                let node = self.parse_use_decl();
                parts.push(CstChild::Node { role: ChildRole::Misc, node });
                parts
            }
            Some(nk) => {
                // Generic: consume to next semicolon or braced block.
                let node = self.parse_generic_item(nk);
                parts.push(CstChild::Node { role: ChildRole::Misc, node });
                parts
            }
            None => {
                // Unknown token — emit verbatim.
                parts.push(CstChild::Token(self.bump()));
                parts
            }
        }
    }

    /// Handle `pub` visibility qualifier then delegate.
    fn parse_pub_item(&mut self, mut parts: Vec<CstChild>) -> Vec<CstChild> {
        // Consume `pub`.
        parts.push(CstChild::Token(self.bump()));
        // Consume optional `(crate)` / `(super)` / `(in path)`.
        if self.at_punct('(') {
            parts.push(CstChild::Token(self.bump())); // (
            while !self.at_eof() && !self.at_punct(')') {
                parts.push(CstChild::Token(self.bump()));
            }
            if self.at_punct(')') {
                parts.push(CstChild::Token(self.bump())); // )
            }
        }
        // Now delegate to parse_item for the actual item.
        parts.extend(self.parse_item());
        parts
    }

    // ── Function definition ──────────────────────────────────

    fn parse_fn_def(&mut self, _attrs: &mut Vec<CstChild>) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        // `fn` keyword.
        children.push(CstChild::Token(self.bump()));

        // Name.
        if !self.at_eof() && matches!(self.peek().kind, TokenKind::Ident) {
            children.push(CstChild::Token(self.bump()));
        }

        // Optional generic params <...>.
        if self.at_punct('<') {
            let generics = self.parse_angle_bracketed();
            children.push(CstChild::Node {
                role: ChildRole::Generics,
                node: generics,
            });
        }

        // Parameter list (...).
        if self.at_punct('(') {
            let params = self.parse_paren_list(NodeKind::ParamList);
            children.push(CstChild::Node {
                role: ChildRole::Params,
                node: params,
            });
        }

        // Return type: -> Type.
        if matches!(self.peek().kind, TokenKind::Operator(ref op) if op == "->") {
            let mut ret_children = Vec::new();
            ret_children.push(CstChild::Token(self.bump())); // ->
            // Consume type tokens until `{`, `where`, or `;`.
            while !self.at_eof() {
                if self.at_punct('{') || self.at_punct(';')
                    || self.at_keyword("where")
                {
                    break;
                }
                ret_children.push(CstChild::Token(self.bump()));
            }
            children.push(CstChild::Node {
                role: ChildRole::ReturnType,
                node: CstNode {
                    kind: NodeKind::TypeExpr,
                    children: ret_children,
                    span: Span::default(),
                },
            });
        }

        // Where clause.
        if self.at_keyword("where") {
            let wh = self.parse_where_clause();
            children.push(CstChild::Node {
                role: ChildRole::Where,
                node: wh,
            });
        }

        // Body { ... } or semicolon.
        if self.at_punct('{') {
            let body = self.parse_braced_block();
            children.push(CstChild::Node {
                role: ChildRole::Body,
                node: body,
            });
        } else if self.at_punct(';') {
            children.push(CstChild::Token(self.bump()));
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::FnDef,
            children,
            span: Span { start, end },
        }
    }

    // ── Struct definition ────────────────────────────────────

    fn parse_struct_def(&mut self, _attrs: &mut Vec<CstChild>) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        // `struct` keyword.
        children.push(CstChild::Token(self.bump()));

        // Name.
        if !self.at_eof() && matches!(self.peek().kind, TokenKind::Ident) {
            children.push(CstChild::Token(self.bump()));
        }

        // Optional generics.
        if self.at_punct('<') {
            let generics = self.parse_angle_bracketed();
            children.push(CstChild::Node {
                role: ChildRole::Generics,
                node: generics,
            });
        }

        // Where clause.
        if self.at_keyword("where") {
            let wh = self.parse_where_clause();
            children.push(CstChild::Node {
                role: ChildRole::Where,
                node: wh,
            });
        }

        // Body { ... } or tuple struct (...) or unit struct ;.
        if self.at_punct('{') {
            let body = self.parse_braced_block();
            children.push(CstChild::Node {
                role: ChildRole::Body,
                node: body,
            });
        } else if self.at_punct('(') {
            let body = self.parse_paren_list(NodeKind::ParamList);
            children.push(CstChild::Node {
                role: ChildRole::Body,
                node: body,
            });
            if self.at_punct(';') {
                children.push(CstChild::Token(self.bump()));
            }
        } else if self.at_punct(';') {
            children.push(CstChild::Token(self.bump()));
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::StructDef,
            children,
            span: Span { start, end },
        }
    }

    // ── Enum definition ──────────────────────────────────────

    fn parse_enum_def(&mut self, _attrs: &mut Vec<CstChild>) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        children.push(CstChild::Token(self.bump())); // enum

        if !self.at_eof() && matches!(self.peek().kind, TokenKind::Ident) {
            children.push(CstChild::Token(self.bump())); // name
        }

        if self.at_punct('<') {
            let generics = self.parse_angle_bracketed();
            children.push(CstChild::Node {
                role: ChildRole::Generics,
                node: generics,
            });
        }

        if self.at_keyword("where") {
            let wh = self.parse_where_clause();
            children.push(CstChild::Node {
                role: ChildRole::Where,
                node: wh,
            });
        }

        if self.at_punct('{') {
            let body = self.parse_braced_block();
            children.push(CstChild::Node {
                role: ChildRole::Body,
                node: body,
            });
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::EnumDef,
            children,
            span: Span { start, end },
        }
    }

    // ── Impl block ───────────────────────────────────────────

    fn parse_impl_block(&mut self, _attrs: &mut Vec<CstChild>) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        children.push(CstChild::Token(self.bump())); // impl

        // Consume tokens until `{`.
        while !self.at_eof() && !self.at_punct('{') {
            if self.at_keyword("where") {
                let wh = self.parse_where_clause();
                children.push(CstChild::Node {
                    role: ChildRole::Where,
                    node: wh,
                });
            } else {
                children.push(CstChild::Token(self.bump()));
            }
        }

        if self.at_punct('{') {
            let body = self.parse_item_block();
            children.push(CstChild::Node {
                role: ChildRole::Body,
                node: body,
            });
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::ImplBlock,
            children,
            span: Span { start, end },
        }
    }

    // ── Use declaration ──────────────────────────────────────

    fn parse_use_decl(&mut self) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        children.push(CstChild::Token(self.bump())); // use

        // Consume until semicolon, handling nested braces.
        let mut depth = 0usize;
        while !self.at_eof() {
            if self.at_punct('{') { depth += 1; }
            if self.at_punct('}') {
                if depth > 0 { depth -= 1; }
            }
            let is_semi = self.at_punct(';') && depth == 0;
            children.push(CstChild::Token(self.bump()));
            if is_semi { break; }
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::UseDecl,
            children,
            span: Span { start, end },
        }
    }

    // ── Generic item (consumes to `;` or `{}`) ───────────────

    fn parse_generic_item(&mut self, kind: NodeKind) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        while !self.at_eof() {
            if self.at_punct('{') {
                let body = self.parse_braced_block();
                children.push(CstChild::Node {
                    role: ChildRole::Body,
                    node: body,
                });
                break;
            }
            if self.at_punct(';') {
                children.push(CstChild::Token(self.bump()));
                break;
            }
            children.push(CstChild::Token(self.bump()));
        }

        let end = self.prev_end();
        CstNode { kind, children, span: Span { start, end } }
    }

    // ── Bracketed helpers ────────────────────────────────────

    /// Parse a braced block that contains items (fn, type, const,
    /// etc.). Used for impl and trait blocks. Recursively parses
    /// each item so they get structured CST nodes.
    fn parse_item_block(&mut self) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        children.push(CstChild::Token(self.bump())); // {

        // Parse items until `}`.
        while !self.at_eof() && !self.at_punct('}') {
            let item_children = self.parse_item();
            children.extend(item_children);
        }

        if self.at_punct('}') {
            children.push(CstChild::Token(self.bump())); // }
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::Block,
            children,
            span: Span { start, end },
        }
    }

    /// Parse a braced block `{ ... }`, preserving all contents.
    fn parse_braced_block(&mut self) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        // Opening brace.
        children.push(CstChild::Token(self.bump())); // {

        let mut depth = 1usize;
        while !self.at_eof() && depth > 0 {
            if self.at_punct('{') { depth += 1; }
            if self.at_punct('}') { depth -= 1; }
            if depth == 0 {
                children.push(CstChild::Token(self.bump())); // closing }
                break;
            }
            children.push(CstChild::Token(self.bump()));
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::Block,
            children,
            span: Span { start, end },
        }
    }

    /// Parse a parenthesised list `( ... )`.
    fn parse_paren_list(&mut self, kind: NodeKind) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        children.push(CstChild::Token(self.bump())); // (

        let mut depth = 1usize;
        while !self.at_eof() && depth > 0 {
            if self.at_punct('(') { depth += 1; }
            if self.at_punct(')') { depth -= 1; }
            if depth == 0 {
                children.push(CstChild::Token(self.bump())); // closing )
                break;
            }
            children.push(CstChild::Token(self.bump()));
        }

        let end = self.prev_end();
        CstNode { kind, children, span: Span { start, end } }
    }

    /// Parse angle-bracketed generics `< ... >`.
    fn parse_angle_bracketed(&mut self) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        children.push(CstChild::Token(self.bump())); // <

        let mut depth = 1usize;
        while !self.at_eof() && depth > 0 {
            if self.at_punct('<') { depth += 1; }
            if self.at_punct('>') { depth -= 1; }
            if depth == 0 {
                children.push(CstChild::Token(self.bump())); // closing >
                break;
            }
            children.push(CstChild::Token(self.bump()));
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::GenericParams,
            children,
            span: Span { start, end },
        }
    }

    /// Parse a where clause: `where P1, P2, ... `.
    /// Consumed until `{` or `;` is encountered.
    fn parse_where_clause(&mut self) -> CstNode {
        let start = self.peek().span.start;
        let mut children = Vec::new();

        children.push(CstChild::Token(self.bump())); // where

        while !self.at_eof() {
            if self.at_punct('{') || self.at_punct(';') {
                break;
            }
            children.push(CstChild::Token(self.bump()));
        }

        let end = self.prev_end();
        CstNode {
            kind: NodeKind::WhereClause,
            children,
            span: Span { start, end },
        }
    }

    /// End position of the previously consumed token.
    fn prev_end(&self) -> usize {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span.end
        } else {
            0
        }
    }
}

/// Compute the span covering all tokens.
fn file_span(tokens: &[Token]) -> Span {
    if tokens.is_empty() {
        return Span::default();
    }
    let start = tokens.first().map(|t| t.span.start).unwrap_or(0);
    let end = tokens.last().map(|t| t.span.end).unwrap_or(0);
    Span { start, end }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmt::lex;

    fn parse(src: &str) -> CstNode {
        let lang = lex::rust_tokens();
        let tokens = lex::lex(src, &lang).expect("lex failed");
        parse_rust(tokens).expect("parse failed")
    }

    fn count_nodes(node: &CstNode) -> usize {
        let mut n = 1;
        for child in &node.children {
            if let CstChild::Node { node: ref inner, .. } = child {
                n += count_nodes(inner);
            }
        }
        n
    }

    #[test]
    fn test_parse_fn_def() {
        let cst = parse("fn foo(x: u32) -> bool { true }");
        assert_eq!(cst.kind, NodeKind::SourceFile);
        // Should have a FnDef child node.
        let fn_node = cst.children.iter().find_map(|c| {
            if let CstChild::Node { node, .. } = c {
                if node.kind == NodeKind::FnDef { return Some(node); }
            }
            None
        });
        assert!(fn_node.is_some(), "expected FnDef node");
        let fn_node = fn_node.expect("checked");
        // Should have Params and ReturnType and Body children.
        let has_params = fn_node.children.iter().any(|c| matches!(c,
            CstChild::Node { role: ChildRole::Params, .. }));
        let has_ret = fn_node.children.iter().any(|c| matches!(c,
            CstChild::Node { role: ChildRole::ReturnType, .. }));
        let has_body = fn_node.children.iter().any(|c| matches!(c,
            CstChild::Node { role: ChildRole::Body, .. }));
        assert!(has_params, "expected Params");
        assert!(has_ret, "expected ReturnType");
        assert!(has_body, "expected Body");
    }

    #[test]
    fn test_parse_struct() {
        let cst = parse("pub struct Foo { x: u32, y: u64, }");
        let struct_node = cst.children.iter().find_map(|c| {
            if let CstChild::Node { node, .. } = c {
                if node.kind == NodeKind::StructDef { return Some(node); }
            }
            None
        });
        assert!(struct_node.is_some(), "expected StructDef node");
    }

    #[test]
    fn test_parse_impl() {
        let cst = parse("impl Foo { fn bar(&self) {} }");
        let impl_node = cst.children.iter().find_map(|c| {
            if let CstChild::Node { node, .. } = c {
                if node.kind == NodeKind::ImplBlock { return Some(node); }
            }
            None
        });
        assert!(impl_node.is_some(), "expected ImplBlock node");
    }

    #[test]
    fn test_parse_use() {
        let cst = parse("use std::collections::HashMap;");
        let use_node = cst.children.iter().find_map(|c| {
            if let CstChild::Node { node, .. } = c {
                if node.kind == NodeKind::UseDecl { return Some(node); }
            }
            None
        });
        assert!(use_node.is_some(), "expected UseDecl node");
    }

    #[test]
    fn test_parse_fn_with_where() {
        let cst = parse("fn foo<T>(x: T) -> T where T: Clone { x.clone() }");
        let fn_node = cst.children.iter().find_map(|c| {
            if let CstChild::Node { node, .. } = c {
                if node.kind == NodeKind::FnDef { return Some(node); }
            }
            None
        });
        assert!(fn_node.is_some());
        let fn_node = fn_node.expect("checked");
        let has_where = fn_node.children.iter().any(|c| matches!(c,
            CstChild::Node { role: ChildRole::Where, .. }));
        assert!(has_where, "expected Where clause");
    }

    #[test]
    fn test_parse_multiple_items() {
        let cst = parse("use std::fmt;\n\nfn main() {}\n\nstruct Foo;");
        let node_count = count_nodes(&cst);
        assert!(node_count >= 4, "expected at least 4 nodes, got {}", node_count);
    }
}
