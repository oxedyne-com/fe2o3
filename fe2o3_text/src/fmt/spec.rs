//! Format specification.
//!
//! Defines the formatting rules that control how each syntactic
//! construct is laid out. The spec is a plain Rust struct that can
//! be loaded from a JDAT configuration file.
//!

use oxedyne_fe2o3_core::prelude::*;


/// How to break a parameter or argument list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListLayout {
    /// Keep everything on one line if it fits.
    Compressed,
    /// If any item doesn't fit, put all items on separate lines.
    Tall,
    /// Always put each item on its own line.
    Vertical,
}

impl Default for ListLayout {
    fn default() -> Self { Self::Tall }
}

/// Where to place the opening brace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BraceStyle {
    /// Always on the same line as the declaration.
    SameLine,
    /// On its own line when a where clause or generics are present.
    SameLineUnlessWhere,
    /// Always on its own line.
    NextLine,
}

impl Default for BraceStyle {
    fn default() -> Self { Self::SameLineUnlessWhere }
}

/// Where to break a binary operator continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinopBreak {
    /// Break before the operator.
    Before,
    /// Break after the operator.
    After,
}

impl Default for BinopBreak {
    fn default() -> Self { Self::Before }
}

/// Where to place the return type when params go vertical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReturnTypePlacement {
    /// On the same line as the closing parenthesis.
    SameLine,
    /// On its own indented line (the fe2o3 style).
    OwnLine,
}

impl Default for ReturnTypePlacement {
    fn default() -> Self { Self::OwnLine }
}

/// Complete formatting specification.
#[derive(Clone, Debug)]
pub struct FormatSpec {
    // ── Global ───────────────────────────────────────────────
    /// Indentation unit.
    pub indent_width:       u16,
    /// Use tabs for indentation.
    pub use_tabs:           bool,
    /// Maximum line width (soft limit).
    pub max_width:          usize,
    /// Maximum consecutive blank lines allowed.
    pub max_blank_lines:    usize,

    // ── Functions ────────────────────────────────────────────
    /// How to lay out function parameters.
    pub fn_params_layout:   ListLayout,
    /// Where to place the return type when params are vertical.
    pub fn_return_type:     ReturnTypePlacement,
    /// Allow single-expression functions on one line.
    pub fn_single_line:     bool,
    /// Maximum width for a single-line function.
    pub fn_single_line_max: usize,

    // ── Braces ───────────────────────────────────────────────
    /// Opening brace placement for items (fn, struct, impl, ...).
    pub brace_style:        BraceStyle,

    // ── Imports ──────────────────────────────────────────────
    /// How to lay out items within a use-tree.
    pub import_layout:      ListLayout,
    /// Reorder import items alphabetically.
    pub import_reorder:     bool,

    // ── Match ────────────────────────────────────────────────
    /// Trailing comma after block-based match arms.
    pub match_trailing_comma:   bool,

    // ── Alignment ────────────────────────────────────────────
    /// Align struct/enum fields whose name lengths differ by up
    /// to this threshold. 0 disables alignment.
    pub field_align_threshold:  usize,

    // ── Where clauses ───────────────────────────────────────
    /// Indent predicates inside a where clause.
    pub where_indent:           bool,

    // ── Binary expressions ───────────────────────────────────
    /// Where to break binary operators.
    pub binop_break:        BinopBreak,

    // ── Method chains ────────────────────────────────────────
    /// Maximum chain width before breaking.
    pub chain_max_width:    usize,

    // ── Trailing commas ──────────────────────────────────────
    /// Add trailing commas in vertical lists.
    pub trailing_comma:     bool,

    // ── Comments ─────────────────────────────────────────────
    /// Reflow comment text to fit max_width.
    pub reflow_comments:    bool,
}

impl Default for FormatSpec {
    fn default() -> Self {
        Self {
            indent_width:           4,
            use_tabs:               false,
            max_width:              100,
            max_blank_lines:        2,
            fn_params_layout:       ListLayout::Tall,
            fn_return_type:         ReturnTypePlacement::OwnLine,
            fn_single_line:         true,
            fn_single_line_max:     80,
            brace_style:            BraceStyle::SameLineUnlessWhere,
            import_layout:          ListLayout::Vertical,
            import_reorder:         false,
            match_trailing_comma:   true,
            field_align_threshold:  40,
            where_indent:           true,
            binop_break:            BinopBreak::Before,
            chain_max_width:        80,
            trailing_comma:         true,
            reflow_comments:        false,
        }
    }
}

impl FormatSpec {
    /// Specification that matches the Oxedyne coding style.
    pub fn fe2o3() -> Self {
        Self::default()
    }

    /// The indentation string for one level.
    pub fn indent_str(&self) -> String {
        if self.use_tabs {
            "\t".to_string()
        } else {
            " ".repeat(self.indent_width as usize)
        }
    }

    /// Parse a format specification from a simple text config.
    ///
    /// Each line is `key = value`. Blank lines and lines starting
    /// with `#` are ignored. Unknown keys produce an error.
    ///
    /// ```text
    /// # Oxedyne style
    /// indent_width = 4
    /// max_width = 100
    /// where_indent = true
    /// fn_return_type = own_line
    /// brace_style = same_line_unless_where
    /// ```
    pub fn from_config_str(s: &str) -> Outcome<Self> {
        let mut spec = Self::default();
        for (lineno, line) in s.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, val) = match line.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => return Err(err!(
                    "Line {}: expected 'key = value', got '{}'", lineno + 1, line;
                    Invalid, Input, Configuration)),
            };
            match key {
                "indent_width"          => spec.indent_width = res!(val.parse::<u16>()),
                "use_tabs"              => spec.use_tabs = val == "true",
                "max_width"             => spec.max_width = res!(val.parse::<usize>()),
                "max_blank_lines"       => spec.max_blank_lines = res!(val.parse::<usize>()),
                "fn_single_line"        => spec.fn_single_line = val == "true",
                "fn_single_line_max"    => spec.fn_single_line_max = res!(val.parse::<usize>()),
                "import_reorder"        => spec.import_reorder = val == "true",
                "match_trailing_comma"  => spec.match_trailing_comma = val == "true",
                "field_align_threshold" => spec.field_align_threshold = res!(val.parse::<usize>()),
                "where_indent"          => spec.where_indent = val == "true",
                "chain_max_width"       => spec.chain_max_width = res!(val.parse::<usize>()),
                "trailing_comma"        => spec.trailing_comma = val == "true",
                "reflow_comments"       => spec.reflow_comments = val == "true",
                "fn_params_layout" => {
                    spec.fn_params_layout = match val {
                        "compressed" => ListLayout::Compressed,
                        "tall"       => ListLayout::Tall,
                        "vertical"   => ListLayout::Vertical,
                        _ => return Err(err!(
                            "Unknown fn_params_layout: '{}'", val;
                            Invalid, Input, Configuration)),
                    };
                }
                "fn_return_type" => {
                    spec.fn_return_type = match val {
                        "same_line"  => ReturnTypePlacement::SameLine,
                        "own_line"   => ReturnTypePlacement::OwnLine,
                        _ => return Err(err!(
                            "Unknown fn_return_type: '{}'", val;
                            Invalid, Input, Configuration)),
                    };
                }
                "brace_style" => {
                    spec.brace_style = match val {
                        "same_line"              => BraceStyle::SameLine,
                        "same_line_unless_where"  => BraceStyle::SameLineUnlessWhere,
                        "next_line"              => BraceStyle::NextLine,
                        _ => return Err(err!(
                            "Unknown brace_style: '{}'", val;
                            Invalid, Input, Configuration)),
                    };
                }
                "import_layout" => {
                    spec.import_layout = match val {
                        "compressed" => ListLayout::Compressed,
                        "tall"       => ListLayout::Tall,
                        "vertical"   => ListLayout::Vertical,
                        _ => return Err(err!(
                            "Unknown import_layout: '{}'", val;
                            Invalid, Input, Configuration)),
                    };
                }
                "binop_break" => {
                    spec.binop_break = match val {
                        "before" => BinopBreak::Before,
                        "after"  => BinopBreak::After,
                        _ => return Err(err!(
                            "Unknown binop_break: '{}'", val;
                            Invalid, Input, Configuration)),
                    };
                }
                _ => return Err(err!(
                    "Unknown config key: '{}'", key;
                    Invalid, Input, Configuration)),
            }
        }
        Ok(spec)
    }
}
