//! Concrete syntax tree types.
//!
//! The CST preserves every byte of the source: tokens, whitespace,
//! and comments. This is essential for round-trip formatting.
//!

/// Byte range in the source text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Span {
    /// Start byte offset (inclusive).
    pub start:  usize,
    /// End byte offset (exclusive).
    pub end:    usize,
}

impl Span {
    /// Length in bytes.
    pub fn len(&self) -> usize { self.end - self.start }
    /// Whether the span is empty.
    pub fn is_empty(&self) -> bool { self.start == self.end }
}

/// Trivia — whitespace and comments that appear between tokens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Trivia {
    /// Contiguous whitespace (spaces, tabs).
    Whitespace(String),
    /// A newline sequence (LF or CRLF).
    Newline,
    /// A line comment (e.g. `// ...`), including the delimiter.
    LineComment(String),
    /// A block comment (e.g. `/* ... */`), including delimiters.
    BlockComment(String),
}

/// A token in the source text, with attached trivia.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    /// The syntactic kind of this token.
    pub kind:           TokenKind,
    /// The literal text of the token.
    pub text:           String,
    /// Trivia (whitespace, comments) that appeared before this token.
    pub leading_trivia: Vec<Trivia>,
    /// Byte span in the source.
    pub span:           Span,
}

/// Broad token categories. Language-specific token kinds are mapped
/// into these categories by the lexer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenKind {
    /// A keyword in the language (e.g. `fn`, `let`, `if`).
    Keyword(String),
    /// An identifier.
    Ident,
    /// A numeric literal.
    Number,
    /// A string literal (including delimiters).
    StringLit,
    /// A character literal.
    CharLit,
    /// A single punctuation character (e.g. `(`, `,`, `;`).
    Punct(char),
    /// A multi-character operator (e.g. `->`, `=>`, `::`, `<=`).
    Operator(String),
    /// A doc comment (e.g. `///` or `//!`).
    DocComment(String),
    /// An attribute (e.g. `#[derive(...)]`).
    Attribute,
    /// A lifetime (e.g. `'a`).
    Lifetime,
    /// A macro invocation name (e.g. `vec` in `vec![...]`).
    MacroName,
    /// End of file.
    Eof,
}

/// A node in the concrete syntax tree.
#[derive(Clone, Debug)]
pub struct CstNode {
    /// What kind of syntactic construct this node represents.
    pub kind:       NodeKind,
    /// The children of this node (tokens and sub-nodes), in source order.
    pub children:   Vec<CstChild>,
    /// Byte span covering the entire node in the source.
    pub span:       Span,
}

/// What a CST child is.
#[derive(Clone, Debug)]
pub enum CstChild {
    /// A leaf token.
    Token(Token),
    /// A sub-tree with a labelled role.
    Node {
        role: ChildRole,
        node: CstNode,
    },
}

/// The syntactic kind of a CST node. These are broad categories
/// that apply across brace-delimited languages. Language-specific
/// refinements add detail but the formatter operates on these.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NodeKind {
    /// The root node of a source file.
    SourceFile,
    /// A function or method definition.
    FnDef,
    /// A parameter list (parenthesised).
    ParamList,
    /// A single parameter.
    Param,
    /// A struct definition.
    StructDef,
    /// A field list (braced).
    FieldList,
    /// A single field.
    Field,
    /// An enum definition.
    EnumDef,
    /// A variant list (braced).
    VariantList,
    /// A single enum variant.
    Variant,
    /// A trait definition.
    TraitDef,
    /// An impl block.
    ImplBlock,
    /// A type alias.
    TypeAlias,
    /// A use/import declaration.
    UseDecl,
    /// A use tree (the braced contents of a use declaration).
    UseTree,
    /// A match/switch expression.
    MatchExpr,
    /// A single match arm.
    MatchArm,
    /// A block (braced sequence of statements).
    Block,
    /// An if/else expression or statement.
    IfExpr,
    /// A for loop.
    ForLoop,
    /// A while loop.
    WhileLoop,
    /// A loop expression.
    LoopExpr,
    /// A let binding.
    LetBinding,
    /// A return expression.
    ReturnExpr,
    /// A closure/lambda.
    Closure,
    /// A function call or macro invocation.
    CallExpr,
    /// An argument list (parenthesised, in a call).
    ArgList,
    /// A method chain (a.b().c()).
    ChainExpr,
    /// A binary expression.
    BinaryExpr,
    /// A where clause.
    WhereClause,
    /// A where predicate.
    WherePredicate,
    /// A generic parameter list (angle-bracketed).
    GenericParams,
    /// A type expression.
    TypeExpr,
    /// An attribute (e.g. `#[derive(...)]`).
    Attribute,
    /// A struct literal (e.g. `Foo { x: 1, y: 2 }`).
    StructLit,
    /// A tuple expression.
    TupleExpr,
    /// An array expression.
    ArrayExpr,
    /// A module declaration.
    ModDecl,
    /// A constant or static item.
    ConstItem,
    /// A macro definition (`macro_rules!`).
    MacroDef,
    /// An expression statement.
    ExprStmt,
    /// A token sequence that the parser did not refine further.
    /// The formatter passes it through unchanged.
    Verbatim,
    /// A comment group (consecutive line comments or a block comment).
    CommentGroup,
}

/// The role a child plays within its parent node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChildRole {
    /// The name of the item (fn name, struct name, etc.).
    Name,
    /// Visibility qualifier (pub, pub(crate), etc.).
    Visibility,
    /// Generic parameters.
    Generics,
    /// The parameter list of a function.
    Params,
    /// The return type of a function.
    ReturnType,
    /// A where clause.
    Where,
    /// The body of a function, struct, enum, impl, etc.
    Body,
    /// A condition (if, while).
    Condition,
    /// The "else" branch.
    Else,
    /// A pattern (in let, match arm).
    Pattern,
    /// The initialiser / value.
    Value,
    /// A type annotation.
    Type,
    /// An attribute.
    Attr,
    /// The separator (comma, semicolon).
    Separator,
    /// The operator in a binary expression.
    Op,
    /// Left-hand side.
    Lhs,
    /// Right-hand side.
    Rhs,
    /// Receiver (self argument or method chain base).
    Receiver,
    /// The iterator in a for loop.
    Iterator,
    /// A module path.
    Path,
    /// Unspecified / fallback.
    Misc,
}
