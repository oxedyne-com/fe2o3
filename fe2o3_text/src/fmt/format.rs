//! CST to Doc transformation.
//!
//! Walks the concrete syntax tree produced by the parser and converts
//! each node into a layout document (`Doc`) according to the format
//! specification. The layout algebra then handles line-breaking
//! decisions during rendering.
//!

use crate::fmt::{
    cst::{
        CstChild,
        CstNode,
        ChildRole,
        NodeKind,
        Token,
        TokenKind,
        Trivia,
    },
    doc::{self, Doc},
    lex,
    parse,
    render,
    spec::{
        BinopBreak,
        BraceStyle,
        FormatSpec,
        ReturnTypePlacement,
    },
};

use oxedyne_fe2o3_core::prelude::*;


/// Format Rust source code.
pub fn format_rust(source: &str, spec: &FormatSpec) -> Outcome<String> {
    let lang = lex::rust_tokens();
    let tokens = res!(lex::lex(source, &lang));
    let cst = res!(parse::parse_rust(tokens));
    let document = cst_to_doc(&cst, spec);
    let indent_str = spec.indent_str();
    let formatted = render::render(spec.max_width, &indent_str, &document);
    Ok(formatted)
}

/// Convert a CST node to a Doc.
fn cst_to_doc(node: &CstNode, spec: &FormatSpec) -> Doc {
    match node.kind {
        NodeKind::SourceFile    => format_source_file(node, spec),
        NodeKind::FnDef         => format_fn_def(node, spec),
        NodeKind::StructDef     => format_struct_def(node, spec),
        NodeKind::EnumDef       => format_enum_def(node, spec),
        NodeKind::ImplBlock     => format_impl_block(node, spec),
        NodeKind::UseDecl       => format_use_decl(node, spec),
        NodeKind::Block         => format_block(node, spec),
        NodeKind::ParamList     => format_paren_list(node, spec),
        NodeKind::GenericParams => format_verbatim_children(node, spec),
        NodeKind::WhereClause   => format_where_clause(node, spec),
        NodeKind::TypeExpr      => format_verbatim_children(node, spec),
        _                       => format_verbatim_children(node, spec),
    }
}

// ── Source file ──────────────────────────────────────────────────

/// Format the top-level source file.
///
/// Iterates over the CST children, dispatching structured nodes
/// (fn, struct, impl, ...) to their specialised formatters and
/// handling stray tokens (doc comments, attributes, `pub`) with
/// trivia-aware spacing.
fn format_source_file(node: &CstNode, spec: &FormatSpec) -> Doc {
    let mut tokens: Vec<Token> = Vec::new();
    collect_tokens(node, &mut tokens);
    tokens_to_formatted_doc(&tokens, spec)
}

/// Format a sequence of CstChild entries. Stray tokens get
/// trivia-based spacing; structured nodes are dispatched to
/// their specialised formatters via `cst_to_doc`.
fn format_child_sequence(children: &[CstChild], spec: &FormatSpec) -> Doc {
    let mut docs = Vec::new();

    for (i, child) in children.iter().enumerate() {
        match child {
            CstChild::Token(tok) => {
                if matches!(tok.kind, TokenKind::Eof) {
                    continue;
                }
                docs.push(token_verbatim(tok));
            }
            CstChild::Node { node: inner, .. } => {
                // If previous entry was a stray token (e.g. `pub`),
                // always add a space before the structured node.
                if i > 0 {
                    if let CstChild::Token(_) = &children[i - 1] {
                        docs.push(doc::text(" "));
                    }
                }
                docs.push(cst_to_doc(inner, spec));
            }
        }
    }
    doc::concat(docs)
}

/// Core formatting logic: convert a token stream to a Doc with
/// proper spacing, newlines, and brace-based indentation.
///
/// Recursively processes `{ ... }` blocks, wrapping their contents
/// in `Nest` so the renderer applies the correct indentation.
fn tokens_to_formatted_doc(tokens: &[Token], spec: &FormatSpec) -> Doc {
    let mut tokens = tokens.to_vec();
    // Apply import reordering before formatting.
    if spec.import_reorder {
        apply_import_reorder(&mut tokens);
    }
    let mut pos = 0;
    let result = format_token_range(&tokens, &mut pos, spec, false);
    doc::concat(result)
}

/// Find `use ... { items }` patterns in the token stream and
/// reorder the items alphabetically.
fn apply_import_reorder(tokens: &mut Vec<Token>) {
    // Find each `use` keyword and look for a subsequent `{ ... }`.
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i].kind, TokenKind::Keyword(ref k) if k == "use") {
            // Find the `{` and `}` for this use statement.
            let mut j = i + 1;
            while j < tokens.len() && !matches!(tokens[j].kind, TokenKind::Punct('{') | TokenKind::Punct(';')) {
                j += 1;
            }
            if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Punct('{')) {
                // Extract the range from `use` to the `;` after `}`.
                let open = j;
                let mut depth = 1usize;
                let mut k = open + 1;
                while k < tokens.len() && depth > 0 {
                    match tokens[k].kind {
                        TokenKind::Punct('{') => depth += 1,
                        TokenKind::Punct('}') => depth -= 1,
                        _ => {}
                    }
                    k += 1;
                }
                let close = k - 1; // position of `}`
                // Extract and reorder just this use block's tokens.
                let mut use_tokens: Vec<Token> = tokens[i..=close].to_vec();
                // Find `;` after `}`.
                if close + 1 < tokens.len() && matches!(tokens[close + 1].kind, TokenKind::Punct(';')) {
                    use_tokens.push(tokens[close + 1].clone());
                }
                reorder_use_items(&mut use_tokens);
                // Replace in the main token list.
                let end = if close + 1 < tokens.len() && matches!(tokens[close + 1].kind, TokenKind::Punct(';')) {
                    close + 2
                } else {
                    close + 1
                };
                tokens.splice(i..end, use_tokens);
            }
        }
        i += 1;
    }
}

/// Format tokens from `pos` to the end (or the matching `}`).
/// When `inside_brace` is true, we stop at the matching `}`.
fn format_token_range(
    tokens:         &[Token],
    pos:            &mut usize,
    spec:           &FormatSpec,
    inside_brace:   bool,
) -> Vec<Doc> {
    let indent = spec.indent_width;
    let mut docs = Vec::new();

    while *pos < tokens.len() {
        let i = *pos;
        let tok = &tokens[i];

        // Emit leading trivia (newlines, comments).
        let mut had_newline = false;
        let mut newline_count = 0usize;
        for t in &tok.leading_trivia {
            match t {
                Trivia::Newline => {
                    had_newline = true;
                    newline_count += 1;
                }
                Trivia::LineComment(c) => {
                    flush_newlines(&mut docs, &mut newline_count, &mut had_newline, spec);
                    docs.push(doc::text(c.as_str()));
                }
                Trivia::BlockComment(c) => {
                    flush_newlines(&mut docs, &mut newline_count, &mut had_newline, spec);
                    docs.push(doc::text(c.as_str()));
                }
                Trivia::Whitespace(_) => {
                    if !had_newline && i > 0 {
                        docs.push(doc::text(" "));
                    }
                }
            }
        }
        flush_newlines(&mut docs, &mut newline_count, &mut had_newline, spec);

        // Space before this token.
        if !had_newline && i > 0 && !suppress_space_before(tok) {
            let prev = &tokens[i - 1];
            if !suppress_space_after(prev) && tok.leading_trivia.is_empty() {
                docs.push(doc::text(" "));
            }
        }

        match tok.kind {
            // ── fn signature: Oxedyne layout ─────────────────
            TokenKind::Keyword(ref kw) if kw == "fn" => {
                let fn_doc = format_fn_from_tokens(tokens, pos, spec);
                docs.push(fn_doc);
            }

            // ── match expression ─────────────────────────────
            TokenKind::Keyword(ref kw) if kw == "match" => {
                docs.push(doc::text("match"));
                *pos += 1;
                // Consume the scrutinee (tokens until `{`).
                while *pos < tokens.len() && !matches!(tokens[*pos].kind, TokenKind::Punct('{')) {
                    let mt = &tokens[*pos];
                    if !suppress_space_before(mt) {
                        docs.push(doc::text(" "));
                    }
                    docs.push(doc::text(mt.text.as_str()));
                    *pos += 1;
                }
                // The `{` will be handled by the brace handler on
                // the next iteration — but we want match-arm-aware
                // formatting. Consume the block ourselves.
                if *pos < tokens.len() && matches!(tokens[*pos].kind, TokenKind::Punct('{')) {
                    docs.push(doc::text(" {"));
                    *pos += 1;
                    // Collect arm tokens.
                    let mut arm_tokens: Vec<Token> = Vec::new();
                    let mut depth = 1usize;
                    while *pos < tokens.len() && depth > 0 {
                        let at = &tokens[*pos];
                        match at.kind {
                            TokenKind::Punct('{') => { depth += 1; arm_tokens.push(at.clone()); }
                            TokenKind::Punct('}') => {
                                depth -= 1;
                                if depth > 0 { arm_tokens.push(at.clone()); }
                            }
                            _ => { arm_tokens.push(at.clone()); }
                        }
                        *pos += 1;
                    }
                    let arms_doc = format_match_arms(&arm_tokens, spec);
                    docs.push(doc::nest(indent, doc::concat(vec![
                        doc::hardline(),
                        arms_doc,
                    ])));
                    docs.push(doc::hardline());
                    docs.push(doc::text("}"));
                }
            }

            TokenKind::Punct('{') => {
                // Check if this is a struct/enum field block by
                // looking at the preceding keyword.
                let is_field_block = is_preceded_by_struct_or_enum(tokens, i);

                docs.push(doc::text("{"));
                *pos += 1;

                if is_field_block && spec.field_align_threshold > 0 {
                    // Collect the raw tokens inside the block.
                    let mut field_tokens: Vec<Token> = Vec::new();
                    let mut depth = 1usize;
                    while *pos < tokens.len() && depth > 0 {
                        let bt = &tokens[*pos];
                        match bt.kind {
                            TokenKind::Punct('{') => { depth += 1; field_tokens.push(bt.clone()); }
                            TokenKind::Punct('}') => {
                                depth -= 1;
                                if depth > 0 { field_tokens.push(bt.clone()); }
                            }
                            _ => { field_tokens.push(bt.clone()); }
                        }
                        *pos += 1;
                    }
                    // Format as aligned fields.
                    let field_doc = format_aligned_fields(&field_tokens, spec);
                    docs.push(doc::nest(indent, doc::concat(vec![
                        doc::hardline(),
                        field_doc,
                    ])));
                    docs.push(doc::hardline());
                    docs.push(doc::text("}"));
                } else {
                    // Generic block: recurse.
                    let inner = format_token_range(tokens, pos, spec, true);
                    let (content, closing) = split_off_closing_brace(inner);
                    if !content.is_empty() {
                        docs.push(doc::nest(indent, doc::concat(content)));
                    }
                    docs.extend(closing);
                }
            }
            TokenKind::Punct('}') if inside_brace => {
                docs.push(doc::text("}"));
                *pos += 1;
                return docs;
            }
            // `where` keyword: optionally indent the predicates.
            TokenKind::Keyword(ref kw) if kw == "where" && spec.where_indent => {
                docs.push(doc::text("where"));
                *pos += 1;
                // Collect tokens until `{` or `;`.
                let mut pred_docs = Vec::new();
                while *pos < tokens.len() {
                    let pt = &tokens[*pos];
                    if matches!(pt.kind, TokenKind::Punct('{') | TokenKind::Punct(';')) {
                        break;
                    }
                    // Emit trivia + token with smart spacing.
                    let mut ph = false;
                    let mut pn = 0usize;
                    for t in &pt.leading_trivia {
                        match t {
                            Trivia::Newline => { ph = true; pn += 1; }
                            Trivia::LineComment(c) => {
                                flush_newlines(&mut pred_docs, &mut pn, &mut ph, spec);
                                pred_docs.push(doc::text(c.as_str()));
                            }
                            Trivia::BlockComment(c) => {
                                flush_newlines(&mut pred_docs, &mut pn, &mut ph, spec);
                                pred_docs.push(doc::text(c.as_str()));
                            }
                            Trivia::Whitespace(_) => {
                                if !ph {
                                    pred_docs.push(doc::text(" "));
                                }
                            }
                        }
                    }
                    flush_newlines(&mut pred_docs, &mut pn, &mut ph, spec);
                    if !ph && !pred_docs.is_empty() && !suppress_space_before(pt) {
                        let prev_tok = &tokens[*pos - 1];
                        if !suppress_space_after(prev_tok) && pt.leading_trivia.is_empty() {
                            pred_docs.push(doc::text(" "));
                        }
                    }
                    if !pt.text.is_empty() {
                        pred_docs.push(doc::text(pt.text.as_str()));
                    }
                    *pos += 1;
                }
                if !pred_docs.is_empty() {
                    docs.push(doc::nest(indent, doc::concat(pred_docs)));
                }
            }
            // ── let / return: binop-aware statement ────
            // Skip `let` inside `if let` / `while let` patterns.
            TokenKind::Keyword(ref kw) if (kw == "let" || kw == "return")
                && !(kw == "let" && i > 0 && matches!(
                    tokens[i - 1].kind,
                    TokenKind::Keyword(ref pk) if pk == "if" || pk == "while"
                )) =>
            {
                docs.push(doc::text(tok.text.as_str()));
                *pos += 1;
                // Collect remaining tokens through `;`.
                let start = *pos;
                let mut end = *pos;
                let mut depth = 0i32;
                while end < tokens.len() {
                    match &tokens[end].kind {
                        TokenKind::Punct('(') | TokenKind::Punct('[')
                            | TokenKind::Punct('{') => depth += 1,
                        TokenKind::Punct(')') | TokenKind::Punct(']') => depth -= 1,
                        TokenKind::Punct('}') => {
                            depth -= 1;
                            if depth < 0 { break; }
                        }
                        TokenKind::Punct(';') if depth == 0 => {
                            end += 1;
                            break;
                        }
                        _ => {}
                    }
                    end += 1;
                }
                if start < end {
                    let mut stmt_rest: Vec<Token> = tokens[start..end].to_vec();
                    // First token's trivia already handled by the outer loop.
                    if let Some(first) = stmt_rest.first_mut() {
                        first.leading_trivia.clear();
                    }
                    // Space between keyword and first token (unless `;`).
                    if !matches!(stmt_rest[0].kind, TokenKind::Punct(';')) {
                        docs.push(doc::text(" "));
                    }
                    if stmt_rest.iter().any(|t| is_unambiguous_binary_op(t)) {
                        docs.push(format_binop_expr(&stmt_rest, spec));
                    } else {
                        // No chain breaks — method chain wiring is separate.
                        docs.push(doc::concat(smart_spaced_tokens_inner(&stmt_rest, false)));
                    }
                }
                *pos = end;
            }
            TokenKind::Eof => {
                *pos += 1;
                break;
            }
            _ => {
                if !tok.text.is_empty() {
                    docs.push(doc::text(tok.text.as_str()));
                }
                *pos += 1;
            }
        }
    }
    docs
}

/// Split a doc list into (content, closing). The closing part is
/// the trailing HardLine + `}` that should sit outside the Nest.
fn split_off_closing_brace(mut docs: Vec<Doc>) -> (Vec<Doc>, Vec<Doc>) {
    // Walk backwards to find the `}` text and any preceding hardlines.
    let mut closing = Vec::new();
    // Pop the `}`.
    if let Some(last) = docs.pop() {
        closing.push(last);
    }
    // Pop any hardlines immediately before the `}`.
    while let Some(d) = docs.last() {
        if matches!(d, Doc::HardLine) {
            closing.push(docs.pop().expect("checked"));
        } else {
            break;
        }
    }
    closing.reverse();
    (docs, closing)
}

/// Format an `fn` signature directly from the token stream.
///
/// Reads tokens starting at `fn` through to `{` or `;`, and produces
/// the Oxedyne layout:
///   - Flat:   `fn foo(x: u32, y: u64) -> bool`
///   - Broken: `fn foo(\n    x: u32,\n    y: u64,\n)\n    -> bool`
///
/// Does NOT consume the `{` or `;` — leaves that for the caller.
fn format_fn_from_tokens(
    tokens: &[Token],
    pos:    &mut usize,
    spec:   &FormatSpec,
) -> Doc {
    let indent = spec.indent_width;

    // Phase 1: collect signature tokens into segments.
    let mut pre_paren: Vec<Token> = Vec::new();  // fn name<T>
    let mut params: Vec<Vec<Token>> = Vec::new(); // param groups
    let mut ret_tokens: Vec<Token> = Vec::new();  // -> Type
    let mut where_tokens: Vec<Token> = Vec::new();// where T: Foo
    let mut has_parens = false;

    // Consume `fn`. Strip its leading trivia since the caller
    // already emitted it.
    let mut fn_tok = tokens[*pos].clone();
    fn_tok.leading_trivia.clear();
    pre_paren.push(fn_tok);
    *pos += 1;

    // Consume name and optional generics (until `(`).
    while *pos < tokens.len() {
        let t = &tokens[*pos];
        if matches!(t.kind, TokenKind::Punct('(')) {
            has_parens = true;
            break;
        }
        if matches!(t.kind, TokenKind::Punct('{') | TokenKind::Punct(';')) {
            break;
        }
        pre_paren.push(t.clone());
        *pos += 1;
    }

    // Consume parameter list `(...)`.
    if has_parens && *pos < tokens.len() {
        *pos += 1; // skip `(`
        let mut depth = 1usize;
        let mut current_param: Vec<Token> = Vec::new();
        while *pos < tokens.len() && depth > 0 {
            let t = &tokens[*pos];
            match t.kind {
                TokenKind::Punct('(') => { depth += 1; current_param.push(t.clone()); }
                TokenKind::Punct(')') => {
                    depth -= 1;
                    if depth == 0 {
                        if !current_param.is_empty() {
                            params.push(current_param);
                            current_param = Vec::new();
                        }
                    } else {
                        current_param.push(t.clone());
                    }
                }
                TokenKind::Punct(',') if depth == 1 => {
                    if !current_param.is_empty() {
                        params.push(current_param);
                        current_param = Vec::new();
                    }
                }
                _ => { current_param.push(t.clone()); }
            }
            *pos += 1;
        }
    }

    // Consume return type `-> Type` and/or where clause.
    let mut in_where = false;
    while *pos < tokens.len() {
        let t = &tokens[*pos];
        if matches!(t.kind, TokenKind::Punct('{') | TokenKind::Punct(';')) {
            break;
        }
        if matches!(t.kind, TokenKind::Keyword(ref k) if k == "where") {
            in_where = true;
        }
        if in_where {
            where_tokens.push(t.clone());
        } else {
            ret_tokens.push(t.clone());
        }
        *pos += 1;
    }

    // Phase 2: build the Doc.
    let mut sig = Vec::new();

    // fn name<T>
    sig.push(doc::concat(smart_spaced_tokens(&pre_paren)));

    // Parameters + return type wrapped in ONE group.
    // The group's flat/broken state controls both the param layout
    // AND the return type placement (Oxedyne rule).
    let mut grouped = Vec::new();

    if has_parens {
        if params.is_empty() {
            grouped.push(doc::text("()"));
        } else {
            // Build param docs. When vertical and there are 2+
            // params, align the type column.
            let align = params.len() >= 2 && spec.field_align_threshold > 0;
            let param_docs = format_param_list(&params, align, spec);

            let sep = doc::concat(vec![doc::text(","), doc::line()]);
            let body = doc::join(sep, param_docs);
            let trailing = if spec.trailing_comma {
                doc::trailing_comma()
            } else {
                doc::empty()
            };
            grouped.push(doc::text("("));
            grouped.push(doc::nest(indent, doc::concat(vec![
                doc::softline(),
                body,
                trailing,
            ])));
            grouped.push(doc::softline());
            grouped.push(doc::text(")"));
        }
    }

    // Return type: inline when flat, own line when broken.
    if !ret_tokens.is_empty() {
        let ret_doc = doc::concat(smart_spaced_tokens(&ret_tokens));
        match spec.fn_return_type {
            ReturnTypePlacement::OwnLine => {
                grouped.push(doc::if_break(
                    // Flat: ` -> Type`
                    doc::concat(vec![doc::text(" "), ret_doc.clone()]),
                    // Broken: `\n    -> Type`
                    doc::nest(indent, doc::concat(vec![
                        doc::hardline(),
                        ret_doc,
                    ])),
                ));
            }
            ReturnTypePlacement::SameLine => {
                grouped.push(doc::text(" "));
                grouped.push(ret_doc);
            }
        }
    }

    // Where clause — part of the grouped section so that its
    // presence forces a break (which triggers the Oxedyne layout).
    if !where_tokens.is_empty() {
        grouped.push(doc::hardline());
        if spec.where_indent {
            grouped.push(doc::text("where"));
            let pred_tokens: Vec<Token> = where_tokens.into_iter().skip(1).collect();
            if !pred_tokens.is_empty() {
                grouped.push(doc::nest(indent, doc::concat(vec![
                    doc::hardline(),
                    doc::concat(smart_spaced_tokens(&pred_tokens)),
                ])));
            }
        } else {
            grouped.push(doc::concat(smart_spaced_tokens(&where_tokens)));
        }
    }

    // Brace + body.
    // Oxedyne rule: when the signature is flat, body stays inline
    //   fn foo(x: u32) -> bool { expr }
    // When broken, brace on own line, body indented:
    //   fn foo(
    //       x: u32,
    //   )
    //       -> bool
    //   {
    //       expr
    //   }
    if *pos < tokens.len() && matches!(tokens[*pos].kind, TokenKind::Punct('{')) {
        // Consume `{` and collect body tokens until matching `}`.
        *pos += 1;
        let mut body_tokens: Vec<Token> = Vec::new();
        let mut depth = 1usize;
        while *pos < tokens.len() && depth > 0 {
            let bt = &tokens[*pos];
            match bt.kind {
                TokenKind::Punct('{') => { depth += 1; body_tokens.push(bt.clone()); }
                TokenKind::Punct('}') => {
                    depth -= 1;
                    if depth > 0 { body_tokens.push(bt.clone()); }
                }
                _ => { body_tokens.push(bt.clone()); }
            }
            *pos += 1;
        }

        // Decide: single-expression body (can go on one line) or
        // multi-statement body (always vertical).
        let is_short = is_short_body(&body_tokens);

        if is_short && spec.fn_single_line {
            // Short body: use smart_spaced_tokens (no trivia hardlines)
            // so the group can choose flat.
            let body_doc = doc::concat(smart_spaced_tokens(&body_tokens));

            grouped.push(doc::if_break(
                doc::text(" {"),
                doc::concat(vec![doc::hardline(), doc::text("{")]),
            ));
            grouped.push(doc::if_break(
                doc::concat(vec![doc::text(" "), body_doc.clone(), doc::text(" ")]),
                doc::concat(vec![
                    doc::nest(indent, doc::concat(vec![doc::hardline(), body_doc])),
                    doc::hardline(),
                ]),
            ));
            grouped.push(doc::text("}"));
        } else {
            // Multi-statement body: always vertical, outside the group.
            sig.push(doc::group(doc::concat(grouped)));
            sig.push(doc::text(" {"));
            // Strip leading newlines/whitespace from the first body
            // token to avoid doubling the newline after `{`. Only
            // strip the prefix before the first comment or content,
            // not newlines between consecutive comment lines.
            if let Some(first) = body_tokens.first_mut() {
                let mut past_lead = false;
                first.leading_trivia.retain(|t| {
                    if past_lead { return true; }
                    match t {
                        Trivia::Newline | Trivia::Whitespace(_) => false,
                        _ => { past_lead = true; true }
                    }
                });
            }
            let body_doc = tokens_to_formatted_doc(&body_tokens, spec);
            sig.push(doc::nest(indent, doc::concat(vec![
                doc::hardline(),
                body_doc,
            ])));
            sig.push(doc::hardline());
            sig.push(doc::text("}"));
            return doc::concat(sig);
        }
    } else if *pos < tokens.len() && matches!(tokens[*pos].kind, TokenKind::Punct(';')) {
        grouped.push(doc::text(";"));
        *pos += 1;
    }

    sig.push(doc::group(doc::concat(grouped)));
    doc::concat(sig)
}

/// Flush pending newlines as hardlines, capped to max_blank_lines.
fn flush_newlines(
    docs:           &mut Vec<Doc>,
    newline_count:  &mut usize,
    had_newline:    &mut bool,
    spec:           &FormatSpec,
) {
    if *had_newline {
        let blanks = (*newline_count).min(spec.max_blank_lines + 1);
        for _ in 0..blanks {
            docs.push(doc::hardline());
        }
        *newline_count = 0;
        *had_newline = false;
    }
}

// ── Function definition ─────────────────────────────────────────

fn format_fn_def(node: &CstNode, spec: &FormatSpec) -> Doc {
    let mut docs = Vec::new();
    let indent = spec.indent_width;

    // Collect the parts by role.
    let mut sig_tokens: Vec<Doc> = Vec::new();  // fn, async, name
    let mut params_node: Option<&CstNode> = None;
    let mut ret_node: Option<&CstNode> = None;
    let mut where_node: Option<&CstNode> = None;
    let mut body_node: Option<&CstNode> = None;
    let mut semi = false;

    for child in &node.children {
        match child {
            CstChild::Token(tok) => {
                if tok.kind == TokenKind::Punct(';') {
                    semi = true;
                } else {
                    sig_tokens.push(token_text(tok));
                }
            }
            CstChild::Node { role, node: inner } => match role {
                ChildRole::Params       => params_node = Some(inner),
                ChildRole::ReturnType   => ret_node = Some(inner),
                ChildRole::Where        => where_node = Some(inner),
                ChildRole::Body         => body_node = Some(inner),
                ChildRole::Generics     => {
                    sig_tokens.push(cst_to_doc(inner, spec));
                }
                _ => {
                    sig_tokens.push(cst_to_doc(inner, spec));
                }
            },
        }
    }

    // Build the signature as a group. The group controls whether
    // params go vertical. The body is OUTSIDE the group so it
    // doesn't force the signature to break.

    let mut sig = Vec::new();

    // fn name
    sig.push(doc::concat(intersperse_space(sig_tokens)));

    // Parameters.
    if let Some(params) = params_node {
        sig.push(format_fn_params(params, spec));
    }

    // Return type.
    // Oxedyne rule: when the group breaks (params go vertical),
    // the return type goes on its own indented line.
    if let Some(ret) = ret_node {
        let ret_doc = format_return_type(ret, spec);
        match spec.fn_return_type {
            ReturnTypePlacement::OwnLine => {
                sig.push(doc::if_break(
                    doc::concat(vec![doc::text(" "), ret_doc.clone()]),
                    doc::concat(vec![
                        doc::hardline(),
                        doc::nest(indent, ret_doc),
                    ]),
                ));
            }
            ReturnTypePlacement::SameLine => {
                sig.push(doc::text(" "));
                sig.push(ret_doc);
            }
        }
    }

    // Where clause.
    if let Some(wh) = where_node {
        sig.push(doc::hardline());
        sig.push(format_where_clause(wh, spec));
    }

    // The signature group — the renderer decides flat vs broken.
    docs.push(doc::group(doc::concat(sig)));

    // Body or semicolon (outside the group).
    if let Some(body) = body_node {
        let has_where = where_node.is_some();
        // Brace placement: use the opening-brace helper.
        // For the Oxedyne style, the brace goes on its own line
        // when the signature broke, but this is handled by the
        // where-clause rule (SameLineUnlessWhere).
        docs.push(format_opening_brace(spec, has_where));

        let body_doc = format_block_contents(body, spec);
        docs.push(doc::nest(indent, doc::concat(vec![
            doc::hardline(),
            body_doc,
        ])));
        docs.push(doc::hardline());
        docs.push(doc::text("}"));
    } else if semi {
        docs.push(doc::text(";"));
    }

    doc::concat(docs)
}

/// Format a function's parameter list.
fn format_fn_params(params: &CstNode, spec: &FormatSpec) -> Doc {
    // Extract tokens between ( and ).
    let inner = extract_inner_tokens(params);
    if inner.is_empty() {
        return doc::text("()");
    }

    // Split into comma-separated items.
    let items = split_by_comma(&inner);
    let item_docs: Vec<Doc> = items.into_iter()
        .map(|toks| tokens_to_doc(&toks, spec))
        .collect();

    format_bracketed_list("(", ")", spec.indent_width, &item_docs, spec)
}

/// Format a return type (-> Type).
fn format_return_type(ret: &CstNode, spec: &FormatSpec) -> Doc {
    format_verbatim_children(ret, spec)
}

// ── Struct definition ────────────────────────────────────────────

fn format_struct_def(node: &CstNode, spec: &FormatSpec) -> Doc {
    let mut docs = Vec::new();

    let mut header_tokens: Vec<Doc> = Vec::new();
    let mut body_node: Option<&CstNode> = None;
    let mut where_node: Option<&CstNode> = None;
    let mut has_semi = false;

    for child in &node.children {
        match child {
            CstChild::Token(tok) if tok.kind == TokenKind::Punct(';') => {
                has_semi = true;
            }
            CstChild::Token(tok) => {
                header_tokens.push(token_text(tok));
            }
            CstChild::Node { role: ChildRole::Body, node: inner } => {
                body_node = Some(inner);
            }
            CstChild::Node { role: ChildRole::Where, node: inner } => {
                where_node = Some(inner);
            }
            CstChild::Node { node: inner, .. } => {
                header_tokens.push(cst_to_doc(inner, spec));
            }
        }
    }

    docs.push(doc::concat(intersperse_space(header_tokens)));

    if let Some(wh) = where_node {
        docs.push(doc::hardline());
        docs.push(format_where_clause(wh, spec));
    }

    if let Some(body) = body_node {
        if body.kind == NodeKind::Block {
            let has_where = where_node.is_some();
            docs.push(format_opening_brace(spec, has_where));
            let body_doc = format_block_contents(body, spec);
            docs.push(doc::nest(spec.indent_width, doc::concat(vec![
                doc::hardline(),
                body_doc,
            ])));
            docs.push(doc::hardline());
            docs.push(doc::text("}"));
        } else {
            docs.push(cst_to_doc(body, spec));
        }
    }

    if has_semi {
        docs.push(doc::text(";"));
    }

    doc::concat(docs)
}

// ── Enum definition ──────────────────────────────────────────────

fn format_enum_def(node: &CstNode, spec: &FormatSpec) -> Doc {
    // Reuse struct_def logic — same structure.
    format_struct_def(node, spec)
}

// ── Impl block ───────────────────────────────────────────────────

fn format_impl_block(node: &CstNode, spec: &FormatSpec) -> Doc {
    let mut docs = Vec::new();
    let mut header_tokens: Vec<Doc> = Vec::new();
    let mut body_node: Option<&CstNode> = None;
    let mut where_node: Option<&CstNode> = None;

    for child in &node.children {
        match child {
            CstChild::Token(tok) => {
                header_tokens.push(token_text(tok));
            }
            CstChild::Node { role: ChildRole::Body, node: inner } => {
                body_node = Some(inner);
            }
            CstChild::Node { role: ChildRole::Where, node: inner } => {
                where_node = Some(inner);
            }
            CstChild::Node { node: inner, .. } => {
                header_tokens.push(cst_to_doc(inner, spec));
            }
        }
    }

    docs.push(doc::concat(intersperse_space(header_tokens)));

    if let Some(wh) = where_node {
        docs.push(doc::hardline());
        docs.push(format_where_clause(wh, spec));
    }

    if let Some(body) = body_node {
        let has_where = where_node.is_some();
        docs.push(format_opening_brace(spec, has_where));
        let body_doc = format_block_contents(body, spec);
        if !is_empty_block(body) {
            docs.push(doc::nest(spec.indent_width, doc::concat(vec![
                doc::hardline(),
                body_doc,
            ])));
            docs.push(doc::hardline());
        }
        docs.push(doc::text("}"));
    }

    doc::concat(docs)
}

// ── Use declaration ──────────────────────────────────────────────

fn format_use_decl(node: &CstNode, spec: &FormatSpec) -> Doc {
    let mut tokens: Vec<Token> = Vec::new();
    collect_tokens(node, &mut tokens);

    if spec.import_reorder {
        reorder_use_items(&mut tokens);
    }

    tokens_to_formatted_doc(&tokens, spec)
}

/// Reorder items inside `use { ... }` blocks alphabetically.
/// Finds the `{` and `}` in the token stream, extracts the
/// comma-separated items between them, sorts by text, and
/// reassembles.
fn reorder_use_items(tokens: &mut Vec<Token>) {
    // Find the `{` and `}` positions.
    let open = tokens.iter().position(|t| matches!(t.kind, TokenKind::Punct('{')));
    let close = tokens.iter().rposition(|t| matches!(t.kind, TokenKind::Punct('}')));
    let (open_idx, close_idx) = match (open, close) {
        (Some(o), Some(c)) if c > o + 1 => (o, c),
        _ => return, // No braced block or empty.
    };

    // Extract inner tokens and split by comma.
    let inner: Vec<Token> = tokens[open_idx + 1..close_idx].to_vec();
    let mut items = split_by_comma(&inner);
    if items.len() < 2 {
        return; // Nothing to sort.
    }

    // Sort by the concatenated text of each item's tokens.
    items.sort_by(|a, b| {
        let a_text: String = a.iter().map(|t| t.text.as_str()).collect();
        let b_text: String = b.iter().map(|t| t.text.as_str()).collect();
        a_text.cmp(&b_text)
    });

    // Reassemble: preserve the trivia of the first original inner
    // token on the first sorted item.
    let first_trivia = if !inner.is_empty() {
        inner[0].leading_trivia.clone()
    } else {
        Vec::new()
    };

    // Build new inner token list from sorted items with commas.
    let mut new_inner: Vec<Token> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        for (j, tok) in item.iter().enumerate() {
            let mut tok = tok.clone();
            // Give the first token of the first item the original
            // leading trivia (newline + whitespace for indentation).
            if i == 0 && j == 0 {
                tok.leading_trivia = first_trivia.clone();
            }
            new_inner.push(tok);
        }
        if i < items.len() - 1 {
            // Add comma between items.
            new_inner.push(Token {
                kind: TokenKind::Punct(','),
                text: ",".to_string(),
                leading_trivia: Vec::new(),
                span: crate::fmt::cst::Span::default(),
            });
        }
    }
    // Trailing comma.
    new_inner.push(Token {
        kind: TokenKind::Punct(','),
        text: ",".to_string(),
        leading_trivia: Vec::new(),
        span: crate::fmt::cst::Span::default(),
    });

    // Replace the inner tokens.
    let mut result: Vec<Token> = Vec::new();
    result.extend_from_slice(&tokens[..open_idx + 1]); // up to and including `{`
    result.extend(new_inner);
    result.extend_from_slice(&tokens[close_idx..]); // `}` and beyond
    *tokens = result;
}

// ── Where clause ─────────────────────────────────────────────────

fn format_where_clause(node: &CstNode, spec: &FormatSpec) -> Doc {
    format_verbatim_children(node, spec)
}

// ── Block ────────────────────────────────────────────────────────

fn format_block(node: &CstNode, spec: &FormatSpec) -> Doc {
    let mut docs = vec![doc::text("{")];
    let body_doc = format_block_contents(node, spec);
    if !is_empty_block(node) {
        docs.push(doc::nest(spec.indent_width, doc::concat(vec![
            doc::hardline(),
            body_doc,
        ])));
        docs.push(doc::hardline());
    }
    docs.push(doc::text("}"));
    doc::concat(docs)
}

/// Format the contents of a block (without the outer braces).
/// Uses the recursive token formatter so nested braces get
/// proper indentation. Only strips the outermost `{` and `}`;
/// inner braces are preserved.
fn format_block_contents(node: &CstNode, spec: &FormatSpec) -> Doc {
    let mut all_tokens: Vec<Token> = Vec::new();
    collect_tokens(node, &mut all_tokens);
    // Strip only the first `{` and last `}`.
    if let Some(first) = all_tokens.first() {
        if matches!(first.kind, TokenKind::Punct('{')) {
            all_tokens.remove(0);
        }
    }
    if let Some(last) = all_tokens.last() {
        if matches!(last.kind, TokenKind::Punct('}')) {
            all_tokens.pop();
        }
    }
    tokens_to_formatted_doc(&all_tokens, spec)
}

// ── Parenthesised list ───────────────────────────────────────────

fn format_paren_list(node: &CstNode, spec: &FormatSpec) -> Doc {
    let inner = extract_inner_tokens(node);
    if inner.is_empty() {
        return doc::text("()");
    }
    let items = split_by_comma(&inner);
    let item_docs: Vec<Doc> = items.into_iter()
        .map(|toks| tokens_to_doc(&toks, spec))
        .collect();
    format_bracketed_list("(", ")", spec.indent_width, &item_docs, spec)
}

// ── Helpers ──────────────────────────────────────────────────────

/// Format all children with smart spacing between tokens.
fn format_verbatim_children(node: &CstNode, _spec: &FormatSpec) -> Doc {
    // Collect all leaf tokens.
    let mut tokens: Vec<Token> = Vec::new();
    collect_tokens(node, &mut tokens);
    doc::concat(smart_spaced_tokens(&tokens))
}

/// Recursively collect all leaf tokens from a CST node.
fn collect_tokens(node: &CstNode, out: &mut Vec<Token>) {
    for child in &node.children {
        match child {
            CstChild::Token(tok) => out.push(tok.clone()),
            CstChild::Node { node: inner, .. } => collect_tokens(inner, out),
        }
    }
}

/// Emit just the token text (no trivia-based whitespace).
/// Comments from trivia are preserved; whitespace/newlines are
/// discarded because the Doc controls all spacing.
fn token_text(tok: &Token) -> Doc {
    let mut docs = Vec::new();

    // Preserve comments from trivia.
    for t in &tok.leading_trivia {
        match t {
            Trivia::LineComment(c) => {
                docs.push(doc::hardline());
                docs.push(doc::text(c.as_str()));
                docs.push(doc::hardline());
            }
            Trivia::BlockComment(c) => {
                docs.push(doc::text(" "));
                docs.push(doc::text(c.as_str()));
            }
            // Whitespace and newlines are handled by the Doc,
            // not by trivia.
            Trivia::Whitespace(_) | Trivia::Newline => {}
        }
    }

    if !tok.text.is_empty() {
        docs.push(doc::text(tok.text.as_str()));
    }

    doc::concat(docs)
}

/// Emit a token with trivia for top-level / verbatim contexts
/// where we want to preserve the original whitespace structure.
fn token_verbatim(tok: &Token) -> Doc {
    let mut docs = Vec::new();

    for t in &tok.leading_trivia {
        match t {
            Trivia::Whitespace(s) => docs.push(doc::text(s.as_str())),
            Trivia::Newline       => docs.push(doc::hardline()),
            Trivia::LineComment(c)  => docs.push(doc::text(c.as_str())),
            Trivia::BlockComment(c) => docs.push(doc::text(c.as_str())),
        }
    }

    if !tok.text.is_empty() {
        docs.push(doc::text(tok.text.as_str()));
    }

    doc::concat(docs)
}

/// Opening brace, respecting brace style.
fn format_opening_brace(spec: &FormatSpec, has_where: bool) -> Doc {
    match spec.brace_style {
        BraceStyle::SameLine => doc::text(" {"),
        BraceStyle::NextLine => doc::concat(vec![doc::hardline(), doc::text("{")]),
        BraceStyle::SameLineUnlessWhere => {
            if has_where {
                doc::concat(vec![doc::hardline(), doc::text("{")])
            } else {
                doc::text(" {")
            }
        }
    }
}

/// Format a bracketed list with group/nest for potential breaking.
fn format_bracketed_list(
    open:       &str,
    close:      &str,
    indent:     u16,
    items:      &[Doc],
    spec:       &FormatSpec,
) -> Doc {
    if items.is_empty() {
        return doc::concat(vec![doc::text(open), doc::text(close)]);
    }

    let sep = doc::concat(vec![doc::text(","), doc::line()]);
    let body = doc::join(sep, items.to_vec());

    let trailing = if spec.trailing_comma {
        doc::trailing_comma()
    } else {
        doc::empty()
    };

    doc::group(doc::concat(vec![
        doc::text(open),
        doc::nest(indent, doc::concat(vec![
            doc::line(),
            body,
            trailing,
        ])),
        doc::line(),
        doc::text(close),
    ]))
}

/// Extract the inner tokens of a bracketed node (skip the first
/// and last tokens, which are the brackets).
fn extract_inner_tokens(node: &CstNode) -> Vec<Token> {
    let children = &node.children;
    if children.len() <= 2 {
        return Vec::new();
    }
    let mut toks = Vec::new();
    for child in &children[1..children.len()-1] {
        if let CstChild::Token(tok) = child {
            toks.push(tok.clone());
        }
    }
    toks
}

/// Split a token sequence at commas.
fn split_by_comma(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut groups: Vec<Vec<Token>> = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    for tok in tokens {
        if tok.kind == TokenKind::Punct(',') {
            if !current.is_empty() {
                groups.push(current);
                current = Vec::new();
            }
        } else {
            current.push(tok.clone());
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// Convert a sequence of tokens to a Doc with smart spacing.
fn tokens_to_doc(tokens: &[Token], _spec: &FormatSpec) -> Doc {
    doc::concat(smart_spaced_tokens(tokens))
}

/// Insert a space between consecutive Docs.
fn intersperse_space(docs: Vec<Doc>) -> Vec<Doc> {
    let mut result = Vec::with_capacity(docs.len() * 2);
    let mut first = true;
    for d in docs {
        if !first {
            result.push(doc::text(" "));
        }
        result.push(d);
        first = false;
    }
    result
}

/// Insert spaces between tokens with language-aware rules.
/// No space before `:`, `;`, `,`, `.`, `)`, `]`, `>`.
/// No space after `(`, `[`, `<`, `.`, `::`, `&`, `*`.
/// Binary operators get a `line()` before or after (soft break
/// point) so long expressions can wrap at operator boundaries.
fn smart_spaced_tokens(tokens: &[Token]) -> Vec<Doc> {
    smart_spaced_tokens_inner(tokens, true)
}

fn smart_spaced_tokens_inner(tokens: &[Token], chain_breaks: bool) -> Vec<Doc> {
    let mut docs = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        if i > 0 {
            let prev = &tokens[i - 1];
            // Method chain break point: after `)` before `.`.
            if chain_breaks
                && matches!(prev.kind, TokenKind::Punct(')'))
                && matches!(tok.kind, TokenKind::Punct('.'))
            {
                docs.push(doc::line());
                docs.push(token_text(tok));
                continue;
            }
            let need_space = !suppress_space_after(prev) && !suppress_space_before(tok);
            if need_space {
                docs.push(doc::text(" "));
            }
        }
        docs.push(token_text(tok));
    }
    docs
}

/// Precedence level of a binary operator. Higher means tighter
/// binding. Returns 0 for non-operators.
///
/// Oxedyne indentation rule: when a line breaks at an operator,
/// each lower-precedence operator indents one level deeper. So
/// `&&` at level 4 sits at the base, `||` at level 3 indents one
/// level relative to `&&`.
fn binop_precedence(tok: &Token) -> u8 {
    match &tok.kind {
        TokenKind::Operator(op) => match op.as_str() {
            "||"                            => 3,
            "&&"                            => 4,
            "==" | "!=" | "<=" | ">="      => 5,
            "|"                             => 6,
            "^"                             => 7,
            "<<" | ">>"                     => 9,
            "+=" | "-=" | "*=" | "/="
                | "%=" | "&=" | "|=" | "^="
                | "<<=" | ">>="             => 1,
            _                               => 0,
        },
        TokenKind::Punct(c) => match c {
            '|'     => 6,
            '^'     => 7,
            '&'     => 8,
            '+' | '-' => 10,
            '*' | '/' | '%' => 11,
            _       => 0,
        },
        _ => 0,
    }
}

/// Check whether a token is a binary operator.
fn is_binary_op(tok: &Token) -> bool {
    binop_precedence(tok) > 0
}

/// Check whether a token is an unambiguously binary operator.
///
/// Only matches multi-character Operator tokens (`&&`, `||`, `==`,
/// `+=`, etc.), not single-character Punct that could be unary
/// (`&`, `*`, `+`, `-`).
fn is_unambiguous_binary_op(tok: &Token) -> bool {
    matches!(&tok.kind, TokenKind::Operator(_)) && binop_precedence(tok) > 0
}

/// Format a token sequence containing binary operators with
/// precedence-based indentation.
///
/// Oxedyne rule: each lower-precedence operator indents one level
/// deeper than the previous. For example:
///
/// ```text
/// let valid = name.len() > 0
///     && age >= 18
///     && country == "NZ"
///         || special_override;
/// ```
///
/// `&&` (precedence 4) is at indent+1, `||` (precedence 3) is at
/// indent+2 because it's a lower-precedence level.
fn format_binop_expr(tokens: &[Token], spec: &FormatSpec) -> Doc {
    let indent = spec.indent_width;

    // Find the distinct precedence levels used (sorted descending).
    let mut prec_levels: Vec<u8> = tokens.iter()
        .map(|t| binop_precedence(t))
        .filter(|p| *p > 0)
        .collect();
    prec_levels.sort();
    prec_levels.dedup();
    prec_levels.reverse(); // Highest first.

    if prec_levels.is_empty() {
        // No operators — emit normally.
        return doc::concat(smart_spaced_tokens(tokens));
    }

    // Map each precedence level to an indent depth.
    // Highest precedence = 1 indent, next = 2 indents, etc.
    let prec_to_depth = |p: u8| -> u16 {
        let idx = prec_levels.iter().position(|&x| x == p).unwrap_or(0);
        (idx as u16 + 1) * indent
    };

    // Build the doc: break before each operator (BinopBreak::Before),
    // nesting to the operator's depth.
    let mut docs = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        let prec = binop_precedence(tok);
        if prec > 0 && i > 0 {
            // Binary operator — insert break point.
            let depth = prec_to_depth(prec);
            match spec.binop_break {
                BinopBreak::Before => {
                    // Break, then indent, then operator, then space.
                    docs.push(doc::nest(depth, doc::concat(vec![
                        doc::line(),
                        doc::text(tok.text.as_str()),
                    ])));
                }
                BinopBreak::After => {
                    // Space, then operator, then break + indent.
                    docs.push(doc::text(" "));
                    docs.push(doc::text(tok.text.as_str()));
                    docs.push(doc::nest(depth, doc::line()));
                }
            }
        } else {
            if i > 0 {
                let prev = &tokens[i - 1];
                let need_space = !suppress_space_after(prev) && !suppress_space_before(tok);
                if need_space {
                    docs.push(doc::text(" "));
                }
            }
            docs.push(token_text(tok));
        }
        i += 1;
    }

    doc::group(doc::concat(docs))
}

/// Tokens after which no trailing space should be added.
fn suppress_space_after(tok: &Token) -> bool {
    match &tok.kind {
        TokenKind::Punct('(') | TokenKind::Punct('[') | TokenKind::Punct('<')
            | TokenKind::Punct('.') | TokenKind::Punct('&') | TokenKind::Punct('*')
            | TokenKind::Punct('!') | TokenKind::Punct('#')
            => true,
        TokenKind::Operator(op)
            if op == "::" || op == ".."  || op == "..="
            => true,
        _ => false,
    }
}

/// Tokens before which no leading space should be added.
fn suppress_space_before(tok: &Token) -> bool {
    match &tok.kind {
        TokenKind::Punct(':') | TokenKind::Punct(';') | TokenKind::Punct(',')
            | TokenKind::Punct('.') | TokenKind::Punct(')') | TokenKind::Punct(']')
            | TokenKind::Punct('>') | TokenKind::Punct('!')
            | TokenKind::Punct('(') | TokenKind::Punct('[')
            | TokenKind::Punct('<')
            => true,
        TokenKind::Operator(op)
            if op == "::" || op == ".." || op == "..="
            // Nested generics: `>>` closing two type params.
            || op == ">>"
            => true,
        _ => false,
    }
}

/// Check whether a block is empty (only contains `{` and `}`).
fn is_empty_block(node: &CstNode) -> bool {
    node.children.iter().all(|c| matches!(c,
        CstChild::Token(tok) if matches!(tok.kind,
            TokenKind::Punct('{') | TokenKind::Punct('}') | TokenKind::Eof)
    ))
}

/// Check whether a body token sequence is short enough to potentially
/// go on one line. A body is short if it has no semicolons (except
/// possibly at the end) and has few tokens.
/// Format a parameter list with optional type-column alignment.
///
/// When `align` is true, the type column is aligned across all
/// parameters by padding shorter names with spaces. The alignment
/// only appears in the broken (vertical) layout; the flat layout
/// uses normal spacing.
///
///   Flat:   `(len: usize, charset: &str)`
///   Broken: `(\n    len:     usize,\n    charset: &str,\n)`
fn format_param_list(
    params: &[Vec<Token>],
    align:  bool,
    _spec:  &FormatSpec,
) -> Vec<Doc> {
    if !align {
        return params.iter()
            .map(|toks| doc::concat(smart_spaced_tokens(toks)))
            .collect();
    }

    // Split each param at `:` into name-tokens and type-tokens.
    let mut parsed: Vec<(Vec<Token>, Vec<Token>)> = Vec::new();
    for param_toks in params {
        let colon_pos = param_toks.iter().position(|t| matches!(t.kind, TokenKind::Punct(':')));
        match colon_pos {
            Some(cp) => {
                let name_toks = param_toks[..cp].to_vec();
                let type_toks = param_toks[cp + 1..].to_vec();
                parsed.push((name_toks, type_toks));
            }
            None => {
                // No colon (e.g. `self`) — no alignment for this param.
                parsed.push((param_toks.clone(), Vec::new()));
            }
        }
    }

    // Compute max name width (text length of name tokens + spaces).
    let name_widths: Vec<usize> = parsed.iter().map(|(name_toks, _)| {
        name_toks.iter().map(|t| t.text.len()).sum::<usize>()
            + name_toks.len().saturating_sub(1) // spaces between tokens
    }).collect();
    let max_name = name_widths.iter().copied().max().unwrap_or(0);

    // Build docs: flat uses normal spacing, broken uses padded names.
    parsed.iter().zip(name_widths.iter()).map(|((name_toks, type_toks), &name_w)| {
        if type_toks.is_empty() {
            // No colon (self, etc.) — just emit as-is.
            return doc::concat(smart_spaced_tokens(name_toks));
        }

        let name_doc = doc::concat(smart_spaced_tokens(name_toks));
        let type_doc = doc::concat(smart_spaced_tokens(type_toks));

        // Flat: `name: type`
        let flat = doc::concat(vec![
            name_doc.clone(),
            doc::text(": "),
            type_doc.clone(),
        ]);

        // Broken: `name:   type` (padded to align type column).
        let pad_len = max_name - name_w;
        let pad = " ".repeat(pad_len + 1); // +1 for the space after `:`
        let broken = doc::concat(vec![
            name_doc,
            doc::text(":"),
            doc::text(pad.as_str()),
            type_doc,
        ]);

        doc::if_break(flat, broken)
    }).collect()
}

/// Check whether the `{` at position `brace_pos` is preceded by
/// a `struct` or `enum` keyword (possibly with intervening ident,
/// generics, where clause, etc.).
/// Format match arms with `=>` column alignment.
///
/// Splits the arm tokens at commas (respecting brace depth), then
/// aligns the `=>` across all simple (single-line) arms.
fn format_match_arms(tokens: &[Token], spec: &FormatSpec) -> Doc {
    // Split into individual arms at top-level commas.
    let arms = split_by_comma_depth(tokens);
    if arms.is_empty() {
        return doc::empty();
    }

    // Parse each arm: split prefix, find `=>`, split pattern/body.
    struct ArmParts {
        prefix:     Vec<Token>,
        pattern:    Vec<Token>,
        body:       Vec<Token>,
        has_arrow:  bool,
    }
    let mut parsed: Vec<ArmParts> = Vec::new();
    for arm_toks in &arms {
        let (prefix, content) = split_field_prefix(arm_toks);
        // Find `=>` at top-level depth.
        let mut arrow_pos = None;
        let mut depth = 0usize;
        for (j, t) in content.iter().enumerate() {
            match t.kind {
                TokenKind::Punct('(') | TokenKind::Punct('{') | TokenKind::Punct('[') => depth += 1,
                TokenKind::Punct(')') | TokenKind::Punct('}') | TokenKind::Punct(']') => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            if depth == 0 && matches!(t.kind, TokenKind::Operator(ref op) if op == "=>") {
                arrow_pos = Some(j);
                break;
            }
        }
        match arrow_pos {
            Some(ap) => {
                parsed.push(ArmParts {
                    prefix,
                    pattern:    content[..ap].to_vec(),
                    body:       content[ap + 1..].to_vec(),
                    has_arrow:  true,
                });
            }
            None => {
                parsed.push(ArmParts {
                    prefix,
                    pattern:    content,
                    body:       Vec::new(),
                    has_arrow:  false,
                });
            }
        }
    }

    // Compute pattern widths for alignment (prefix excluded).
    let pat_widths: Vec<usize> = parsed.iter().map(|a| {
        a.pattern.iter().map(|t| t.text.len()).sum::<usize>()
            + a.pattern.len().saturating_sub(1)
    }).collect();
    let max_pat = pat_widths.iter().copied().max().unwrap_or(0);
    let min_pat = pat_widths.iter().copied().min().unwrap_or(0);
    let align = (max_pat - min_pat) <= spec.field_align_threshold
        && parsed.iter().all(|a| a.has_arrow);

    // Build docs.
    let mut docs = Vec::new();
    for (i, (arm, &pat_w)) in parsed.iter().zip(pat_widths.iter()).enumerate() {
        if i > 0 {
            docs.push(doc::text(","));
            docs.push(doc::hardline());
        }

        emit_prefix(&mut docs, &arm.prefix);
        let pat_doc = doc::concat(smart_spaced_tokens(&arm.pattern));

        if !arm.has_arrow {
            docs.push(pat_doc);
            continue;
        }

        let body_doc = doc::concat(smart_spaced_tokens(&arm.body));

        if align {
            let pad = " ".repeat(max_pat - pat_w);
            docs.push(doc::concat(vec![
                pat_doc,
                doc::text(pad.as_str()),
                doc::text(" => "),
                body_doc,
            ]));
        } else {
            docs.push(doc::concat(vec![
                pat_doc,
                doc::text(" => "),
                body_doc,
            ]));
        }
    }
    // Trailing comma on last arm.
    if spec.match_trailing_comma && !parsed.is_empty() {
        docs.push(doc::text(","));
    }
    doc::concat(docs)
}

/// Split a token sequence at commas, respecting bracket depth.
/// Unlike `split_by_comma`, this tracks `{`, `(`, `[` depth so
/// commas inside braced blocks don't split arms.
fn split_by_comma_depth(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut groups: Vec<Vec<Token>> = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    let mut depth = 0usize;
    for tok in tokens {
        match tok.kind {
            TokenKind::Punct('(') | TokenKind::Punct('{') | TokenKind::Punct('[') => {
                depth += 1;
                current.push(tok.clone());
            }
            TokenKind::Punct(')') | TokenKind::Punct('}') | TokenKind::Punct(']') => {
                depth = depth.saturating_sub(1);
                current.push(tok.clone());
            }
            TokenKind::Punct(',') if depth == 0 => {
                if !current.is_empty() {
                    groups.push(current);
                    current = Vec::new();
                }
            }
            _ => {
                current.push(tok.clone());
            }
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn is_preceded_by_struct_or_enum(tokens: &[Token], brace_pos: usize) -> bool {
    // Scan backwards for the nearest keyword.
    for j in (0..brace_pos).rev() {
        match &tokens[j].kind {
            TokenKind::Keyword(k) if k == "struct" || k == "enum" => return true,
            TokenKind::Keyword(_) => return false,
            _ => continue,
        }
    }
    false
}

/// Format struct/enum fields with type-column alignment.
///
/// Splits the token sequence at commas, parses each field as
/// `name: Type`, and aligns the type column across all fields.
/// Format struct/enum fields with alignment.
///
/// Auto-detects the separator to align:
/// - `:` for struct fields (`name: Type`).
/// - `=` for enum discriminants (`Name = value`).
/// Falls back to no alignment if no separator is found.
/// Split a field's tokens into prefix (doc comments, attributes)
/// and content. Prefix items are emitted on separate lines.
fn split_field_prefix(tokens: &[Token]) -> (Vec<Token>, Vec<Token>) {
    let mut prefix = Vec::new();
    let mut content = Vec::new();
    let mut past = false;
    for tok in tokens {
        if !past && matches!(tok.kind, TokenKind::DocComment(_) | TokenKind::Attribute) {
            prefix.push(tok.clone());
        } else {
            past = true;
            content.push(tok.clone());
        }
    }
    (prefix, content)
}

/// Emit doc comments and attributes on their own lines.
fn emit_prefix(docs: &mut Vec<Doc>, prefix: &[Token]) {
    for tok in prefix {
        docs.push(doc::text(tok.text.as_str()));
        docs.push(doc::hardline());
    }
}

/// Find the first occurrence of `ch` as a `Punct` at bracket depth 0.
fn find_at_depth0(tokens: &[Token], ch: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        match tok.kind {
            TokenKind::Punct('(') | TokenKind::Punct('{') | TokenKind::Punct('[') => depth += 1,
            TokenKind::Punct(')') | TokenKind::Punct('}') | TokenKind::Punct(']') => {
                depth = depth.saturating_sub(1);
            }
            TokenKind::Punct(c) if c == ch && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn format_aligned_fields(tokens: &[Token], spec: &FormatSpec) -> Doc {
    let fields = split_by_comma_depth(tokens);
    if fields.len() < 2 {
        let mut docs = Vec::new();
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                docs.push(doc::text(","));
                docs.push(doc::hardline());
            }
            let (prefix, content) = split_field_prefix(field);
            emit_prefix(&mut docs, &prefix);
            docs.push(doc::concat(smart_spaced_tokens(&content)));
        }
        if !fields.is_empty() {
            docs.push(doc::text(","));
        }
        return doc::concat(docs);
    }

    // Split each field into prefix and content.
    let split: Vec<(Vec<Token>, Vec<Token>)> = fields.iter()
        .map(|f| split_field_prefix(f))
        .collect();

    // Detect separator at depth 0 only (ignore `:` inside `{...}`).
    let has_colon = split.iter().any(|(_, c)| find_at_depth0(c, ':').is_some());
    let has_eq = split.iter().any(|(_, c)| find_at_depth0(c, '=').is_some());

    let (sep_char, _sep_str) = if has_colon {
        (':', ":")
    } else if has_eq {
        ('=', " =")
    } else {
        // No separator — emit without alignment.
        let mut docs = Vec::new();
        for (i, (prefix, content)) in split.iter().enumerate() {
            if i > 0 {
                docs.push(doc::text(","));
                docs.push(doc::hardline());
            }
            emit_prefix(&mut docs, prefix);
            docs.push(doc::concat(smart_spaced_tokens(content)));
        }
        if !fields.is_empty() { docs.push(doc::text(",")); }
        return doc::concat(docs);
    };

    // Split each field's content at the depth-0 separator.
    let mut parsed: Vec<(Vec<Token>, Vec<Token>, Vec<Token>)> = Vec::new();
    for (prefix, content) in &split {
        let sep_pos = find_at_depth0(content, sep_char);
        match sep_pos {
            Some(sp) => {
                parsed.push((prefix.clone(), content[..sp].to_vec(), content[sp + 1..].to_vec()));
            }
            None => {
                parsed.push((prefix.clone(), content.clone(), Vec::new()));
            }
        }
    }

    // Compute name widths (content only, not prefix).
    let name_widths: Vec<usize> = parsed.iter().map(|(_, name_toks, _)| {
        name_toks.iter().map(|t| t.text.len()).sum::<usize>()
            + name_toks.len().saturating_sub(1)
    }).collect();
    let max_name = name_widths.iter().copied().max().unwrap_or(0);
    let min_name = name_widths.iter().copied().min().unwrap_or(0);
    let align = (max_name - min_name) <= spec.field_align_threshold;

    // Build docs.
    let mut docs = Vec::new();
    for (i, ((prefix, name_toks, val_toks), &name_w)) in parsed.iter().zip(name_widths.iter()).enumerate() {
        if i > 0 {
            docs.push(doc::text(","));
            docs.push(doc::hardline());
        }
        emit_prefix(&mut docs, prefix);
        let name_doc = doc::concat(smart_spaced_tokens(name_toks));
        if val_toks.is_empty() {
            docs.push(name_doc);
        } else {
            let val_doc = doc::concat(smart_spaced_tokens(val_toks));
            if align {
                let pad = " ".repeat(max_name - name_w);
                if sep_char == '=' {
                    docs.push(doc::concat(vec![
                        name_doc,
                        doc::text(pad.as_str()),
                        doc::text(" = "),
                        val_doc,
                    ]));
                } else {
                    docs.push(doc::concat(vec![
                        name_doc,
                        doc::text(":"),
                        doc::text(" "),
                        doc::text(pad.as_str()),
                        val_doc,
                    ]));
                }
            } else {
                let unpadded_sep = if sep_char == '=' { " = " } else { ": " };
                docs.push(doc::concat(vec![
                    name_doc,
                    doc::text(unpadded_sep),
                    val_doc,
                ]));
            }
        }
    }
    if !fields.is_empty() {
        docs.push(doc::text(","));
    }
    doc::concat(docs)
}

fn is_short_body(tokens: &[Token]) -> bool {
    // Must be a single expression — no semicolons, no let, no braces.
    let has_semi = tokens.iter().any(|t| matches!(t.kind, TokenKind::Punct(';')));
    if has_semi { return false; }
    let has_let = tokens.iter().any(|t| matches!(t.kind, TokenKind::Keyword(ref k) if k == "let"));
    if has_let { return false; }
    let has_brace = tokens.iter().any(|t| matches!(t.kind, TokenKind::Punct('{')));
    if has_brace { return false; }
    // Short enough (heuristic: total text width).
    let width: usize = tokens.iter().map(|t| t.text.len() + 1).sum();
    width < 40
}

/// Check whether a block contains a single expression (for fn_single_line).
fn is_single_expr_block(node: &CstNode) -> bool {
    let meaningful: Vec<&CstChild> = node.children.iter().filter(|c| {
        match c {
            CstChild::Token(tok) => !matches!(tok.kind,
                TokenKind::Punct('{') | TokenKind::Punct('}')),
            _ => true,
        }
    }).collect();
    // Count non-trivia tokens.
    let token_count = meaningful.iter().filter(|c| {
        match c {
            CstChild::Token(tok) => {
                // Skip tokens that are only whitespace/newline trivia.
                !tok.text.is_empty()
            }
            _ => true,
        }
    }).count();
    token_count <= 5 // Heuristic: short expressions have few tokens.
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmt::spec::FormatSpec;

    fn fmt(source: &str) -> String {
        let spec = FormatSpec::fe2o3();
        format_rust(source, &spec).expect("format failed")
    }

    fn fmt_with(source: &str, spec: &FormatSpec) -> String {
        format_rust(source, spec).expect("format failed")
    }

    #[test]
    fn test_format_empty_fn() {
        let out = fmt("fn main() {}");
        assert!(out.contains("fn"), "output: {}", out);
        assert!(out.contains("main"), "output: {}", out);
    }

    #[test]
    fn test_format_fn_with_body() {
        let out = fmt("fn main() { let x = 42; }");
        assert!(out.contains("fn main"), "output: {}", out);
    }

    #[test]
    fn test_format_struct() {
        let out = fmt("struct Foo { x: u32, y: u64, }");
        assert!(out.contains("struct Foo"), "output: {}", out);
    }

    #[test]
    fn test_format_use() {
        let out = fmt("use std::collections::HashMap;");
        assert!(out.contains("use"), "output: {}", out);
        assert!(out.contains("HashMap"), "output: {}", out);
    }

    #[test]
    fn test_format_impl() {
        let out = fmt("impl Foo { fn bar(&self) {} }");
        assert!(out.contains("impl Foo"), "output: {}", out);
    }

    #[test]
    fn test_format_preserves_comments() {
        let out = fmt("// A comment\nfn foo() {}");
        assert!(out.contains("// A comment"), "output: {:?}", out);
    }

    #[test]
    fn test_format_preserves_doc_comments() {
        let out = fmt("/// A doc comment.\nfn foo() {}");
        assert!(out.contains("/// A doc comment."), "output: {:?}", out);
    }

    #[test]
    fn test_format_multiple_items() {
        let out = fmt("use std::fmt;\n\nfn main() {}\n\nstruct Foo;");
        assert!(out.contains("use"), "output: {:?}", out);
        assert!(out.contains("fn main"), "output: {:?}", out);
        assert!(out.contains("struct Foo"), "output: {:?}", out);
    }

    #[test]
    fn test_format_fn_with_params() {
        let out = fmt("fn foo(x: u32, y: u64) -> bool { true }");
        assert!(out.contains("fn foo"), "output: {:?}", out);
        assert!(out.contains("x: u32"), "output: {:?}", out);
    }

    #[test]
    fn test_roundtrip_simple() {
        // Format twice — should be idempotent.
        let source = "fn main() {\n    let x = 42;\n}";
        let first = fmt(source);
        let second = fmt(&first);
        assert_eq!(first, second, "not idempotent:\nfirst:  {:?}\nsecond: {:?}", first, second);
    }
}
