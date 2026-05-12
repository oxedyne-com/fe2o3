//! Annealer -- the Hematite code formatter.
//!
//! A language-aware code formatter built on Wadler-style layout
//! algebra. The architecture separates three concerns:
//!
//! - **Lexing**: source text to token stream (per-language, data-driven).
//! - **Parsing**: token stream to concrete syntax tree (structural, keyword-aware).
//! - **Formatting**: CST to layout document to formatted text (universal algebra).
//!
//! The layout algebra (the `Doc` type) is a small set of combinators
//! that can express every formatting pattern across every language.
//! The renderer walks the document and makes optimal line-breaking
//! decisions within a given width.
//!
//! # Usage
//!
//! ```ignore
//! use oxedyne_fe2o3_text::fmt::{format_rust, format_source, spec::FormatSpec};
//!
//! let source = "fn main ( ) { println!(\"hello\") ; }";
//! let spec = FormatSpec::fe2o3();
//! let formatted = res!(format_rust(source, &spec));
//!
//! // Language-generic path.
//! let go_src = "func main() { fmt.Println(\"hello\") }";
//! let formatted = res!(format_source(go_src, "go", &spec));
//! ```
//!
pub mod doc;
pub mod cst;
pub mod lex;
pub mod parse;
pub mod spec;
pub mod render;
pub mod format;

use crate::fmt::spec::FormatSpec;

use oxedyne_fe2o3_core::prelude::*;


/// Format Rust source code according to the given specification.
pub fn format_rust(source: &str, spec: &FormatSpec) -> Outcome<String> {
    format::format_rust(source, spec)
}

/// Format source code in the named language.
///
/// Supported languages: `"rust"`, `"c"`, `"cpp"` (also `"c++"`),
/// `"csharp"` (also `"cs"`, `"c#"`), `"go"`, `"java"`,
/// `"js"` (also `"javascript"`, `"typescript"`, `"ts"`),
/// `"python"` (also `"py"`).
///
/// Rust goes through the structural parser for keyword-aware
/// formatting. Other languages use the generic token-stream
/// pipeline, which handles indentation, spacing, and comment
/// preservation.
pub fn format_source(source: &str, lang: &str, spec: &FormatSpec) -> Outcome<String> {
    match lang {
        "rust" | "rs" => format_rust(source, spec),
        _ => {
            let lang_tokens = res!(lang_tokens_for(lang));
            format::format_with_lang(source, &lang_tokens, spec)
        }
    }
}

/// Detect language from a file path's extension.
pub fn detect_language(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next();
    match ext {
        Some("rs")                                      => Some("rust"),
        Some("c")                                       => Some("c"),
        Some("cpp") | Some("cxx") | Some("cc")
            | Some("hpp") | Some("hxx") | Some("hh")   => Some("cpp"),
        Some("h")                                       => Some("c"),
        Some("cs")                                      => Some("csharp"),
        Some("go")                                      => Some("go"),
        Some("java")                                    => Some("java"),
        Some("js") | Some("jsx") | Some("mjs")
            | Some("ts") | Some("tsx")                  => Some("js"),
        Some("py") | Some("pyw")                        => Some("python"),
        _                                               => None,
    }
}

/// Detect language from source text content.
///
/// Examines the first 200 lines for shebang lines, keywords, and
/// syntactic patterns distinctive to each supported language.
/// Returns `None` if no language scores above the confidence
/// threshold.
pub fn detect_language_from_source(source: &str) -> Option<&'static str> {
    // Shebang check.
    if let Some(first) = source.lines().next() {
        if first.starts_with("#!") {
            let shebang = first.to_ascii_lowercase();
            if shebang.contains("python") || shebang.contains("python3") {
                return Some("python");
            }
            if shebang.contains("node") || shebang.contains("deno") || shebang.contains("bun") {
                return Some("js");
            }
            if shebang.contains("java") {
                return Some("java");
            }
        }
    }

    let mut rust:   i32 = 0;
    let mut c:      i32 = 0;
    let mut cpp:    i32 = 0;
    let mut csharp: i32 = 0;
    let mut go:     i32 = 0;
    let mut java:   i32 = 0;
    let mut js:     i32 = 0;
    let mut python: i32 = 0;

    let lines: Vec<&str> = source.lines().take(200).collect();

    for line in &lines {
        let trimmed = line.trim();

        // Rust.
        if trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub struct ")
            || trimmed.starts_with("pub enum ")
            || trimmed.starts_with("pub trait ")                { rust += 5; }
        if trimmed.starts_with("fn ")                           { rust += 3; }
        if trimmed.starts_with("let mut ")                      { rust += 4; }
        if trimmed.starts_with("mod ")                          { rust += 2; }
        if trimmed.starts_with("impl ")                         { rust += 5; }
        if trimmed.starts_with("use ") && trimmed.contains("::"){ rust += 4; }
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") { rust += 4; }
        if trimmed.starts_with("///") || trimmed.starts_with("//!") { rust += 3; }
        if trimmed.contains("-> ") && !trimmed.starts_with("//") { rust += 2; }
        if trimmed.contains("&self")                            { rust += 5; }
        if trimmed.contains("&str")                             { rust += 4; }
        if trimmed.contains("macro_rules!")                     { rust += 6; }
        if trimmed.contains("res!(") || trimmed.contains("ok!(") { rust += 3; }
        if trimmed.starts_with("match ")                        { rust += 2; }

        // C.
        if trimmed.starts_with("#include")                      { c += 6; }
        if trimmed.starts_with("#define")                       { c += 4; }
        if trimmed.starts_with("typedef ")                      { c += 5; }
        if trimmed.contains("sizeof(")                          { c += 3; }
        if trimmed.contains("malloc(") || trimmed.contains("free(") { c += 4; }
        if trimmed.contains("printf(") || trimmed.contains("fprintf(") { c += 3; }
        if trimmed.contains("NULL")                             { c += 2; }
        if trimmed.starts_with("void ")                         { c += 2; }
        if trimmed.contains("int main")                         { c += 3; }

        // C++.
        if trimmed.starts_with("#include <") && (
            trimmed.contains("<iostream>") || trimmed.contains("<vector>")
            || trimmed.contains("<string>") || trimmed.contains("<map>")
            || trimmed.contains("<memory>") || trimmed.contains("<algorithm>")
            || trimmed.contains("<functional>") || trimmed.contains("<array>")
            || trimmed.contains("<optional>") || trimmed.contains("<variant>"))
                                                                { cpp += 6; }
        if trimmed.starts_with("namespace ")                    { cpp += 5; }
        if trimmed.starts_with("class ") && trimmed.contains('{') { cpp += 2; }
        if trimmed.starts_with("template")                      { cpp += 6; }
        if trimmed.contains("std::")                             { cpp += 5; }
        if trimmed.contains("cout") || trimmed.contains("cerr") { cpp += 4; }
        if trimmed.contains("nullptr")                          { cpp += 5; }
        if trimmed.contains("auto ") && trimmed.contains("= ")
            && !trimmed.starts_with("#")                        { cpp += 3; }
        if trimmed.contains("const auto")                       { cpp += 4; }
        if trimmed.contains("unique_ptr") || trimmed.contains("shared_ptr")
            || trimmed.contains("make_unique") || trimmed.contains("make_shared")
                                                                { cpp += 6; }
        if trimmed.contains("static_cast") || trimmed.contains("dynamic_cast")
            || trimmed.contains("reinterpret_cast")             { cpp += 6; }
        if trimmed.starts_with("virtual ")                      { cpp += 4; }
        if trimmed.contains("constexpr ")                       { cpp += 5; }
        if trimmed.contains("noexcept")                         { cpp += 4; }
        if trimmed.starts_with("using namespace")               { cpp += 5; }
        if trimmed.starts_with("#include") && !trimmed.contains('<') { cpp += 2; }
        if trimmed.contains("override") && trimmed.contains('{') { cpp += 3; }

        // C#.
        if trimmed.starts_with("using System")                  { csharp += 6; }
        if trimmed.starts_with("namespace ")
            && !trimmed.contains("::")                          { csharp += 4; }
        if trimmed.contains("Console.Write")                    { csharp += 5; }
        if trimmed.starts_with("[") && (
            trimmed.contains("Serializable") || trimmed.contains("Attribute")
            || trimmed.contains("Test") || trimmed.contains("HttpGet")
            || trimmed.contains("HttpPost") || trimmed.contains("Required")
            || trimmed.contains("JsonProperty") || trimmed.contains("Obsolete"))
                                                                { csharp += 4; }
        if trimmed.contains("string.") || trimmed.contains("String.")
            && !trimmed.contains("import java.")                { csharp += 2; }
        if trimmed.starts_with("public class ") && !trimmed.contains("static void main")
                                                                { csharp += 2; }
        if trimmed.contains("async Task") || trimmed.contains("Task<") { csharp += 5; }
        if trimmed.contains("IEnumerable") || trimmed.contains("IList")
            || trimmed.contains("IDictionary")                  { csharp += 5; }
        if trimmed.starts_with("foreach ")                      { csharp += 5; }
        if trimmed.contains("var ") && trimmed.contains("= new ") { csharp += 3; }
        if trimmed.contains("get;") || trimmed.contains("set;") { csharp += 5; }
        if trimmed.contains("=> ") && trimmed.contains(';')
            && !trimmed.contains("===")                         { csharp += 2; }
        if trimmed.starts_with("internal ")                     { csharp += 5; }
        if trimmed.starts_with("sealed ")                       { csharp += 5; }
        if trimmed.starts_with("partial class")
            || trimmed.starts_with("partial struct")            { csharp += 6; }
        if trimmed.contains("nameof(")                          { csharp += 5; }
        if trimmed.contains(".Select(") || trimmed.contains(".Where(")
            || trimmed.contains(".OrderBy(")                    { csharp += 3; }
        if trimmed.starts_with("record ")                       { csharp += 5; }

        // Go.
        if trimmed.starts_with("package ")                      { go += 6; }
        if trimmed.starts_with("func ")                         { go += 5; }
        if trimmed.contains(":= ")                              { go += 4; }
        if trimmed.starts_with("import (")                      { go += 5; }
        if trimmed.starts_with("defer ")                        { go += 5; }
        if trimmed.contains("go func")                          { go += 5; }
        if trimmed.contains("chan ")                             { go += 4; }
        if trimmed.contains("fmt.")                             { go += 3; }
        if trimmed.starts_with("type ") && trimmed.contains("struct {") { go += 5; }
        if trimmed.starts_with("var ")                          { go += 1; }

        // Java.
        if trimmed.starts_with("public class ")                 { java += 6; }
        if trimmed.starts_with("private ")
            || trimmed.starts_with("protected ")                { java += 3; }
        if trimmed.contains("System.out")                       { java += 5; }
        if trimmed.starts_with("import java.")
            || trimmed.starts_with("import javax.")             { java += 6; }
        if trimmed.starts_with("@Override")
            || trimmed.starts_with("@Deprecated")               { java += 4; }
        if trimmed.contains("public static void main")          { java += 6; }
        if trimmed.contains("throws ")                          { java += 3; }
        if trimmed.starts_with("package ") && trimmed.contains(';') { java += 4; }

        // JavaScript / TypeScript.
        if trimmed.starts_with("const ") && trimmed.contains("= ") { js += 2; }
        if trimmed.starts_with("function ")                     { js += 3; }
        if trimmed.contains("=> {") || trimmed.contains("=> (") { js += 4; }
        if trimmed.contains("===") || trimmed.contains("!==")  { js += 5; }
        if trimmed.contains("require(")                         { js += 5; }
        if trimmed.contains("console.")                         { js += 4; }
        if trimmed.starts_with("export ")                       { js += 3; }
        if trimmed.starts_with("import ") && trimmed.contains("from ") { js += 5; }
        if trimmed.starts_with("interface ")                    { js += 3; }
        if trimmed.contains("document.") || trimmed.contains("window.") { js += 4; }
        if trimmed.starts_with("async function")                { js += 4; }

        // Python.
        if trimmed.starts_with("def ")                          { python += 4; }
        if trimmed.starts_with("class ") && trimmed.ends_with(':') { python += 5; }
        if trimmed.contains("self.")                            { python += 4; }
        if trimmed.starts_with("elif ")                         { python += 6; }
        if trimmed.starts_with("from ") && trimmed.contains("import ") { python += 5; }
        if trimmed.starts_with("import ") && !trimmed.contains('{')
            && !trimmed.contains("java.") && !trimmed.contains("from ") { python += 2; }
        if trimmed.starts_with("# ") && !trimmed.starts_with("#include")
            && !trimmed.starts_with("#define") && !trimmed.starts_with("#[") { python += 1; }
        if trimmed.contains("__init__") || trimmed.contains("__main__") { python += 6; }
        if trimmed.contains("None") && !trimmed.starts_with("//") { python += 1; }
        if trimmed.starts_with("@") && !trimmed.starts_with("@Override")
            && !trimmed.starts_with("@Deprecated")              { python += 1; }
        if (trimmed.starts_with("if ") || trimmed.starts_with("for ")
            || trimmed.starts_with("while ")) && trimmed.ends_with(':') { python += 2; }
    }

    let scores = [
        (rust,   "rust"),
        (c,      "c"),
        (cpp,    "cpp"),
        (csharp, "csharp"),
        (go,     "go"),
        (java,   "java"),
        (js,     "js"),
        (python, "python"),
    ];

    let (best_score, best_lang) = scores.iter()
        .fold((0, ""), |(bs, bl), &(s, l)| if s > bs { (s, l) } else { (bs, bl) });

    // Require a minimum confidence.
    if best_score >= 5 {
        Some(best_lang)
    } else {
        None
    }
}

/// Look up the `LangTokens` for a language name.
fn lang_tokens_for(lang: &str) -> Outcome<lex::LangTokens> {
    match lang {
        "c"                                             => Ok(lex::c_tokens()),
        "cpp" | "c++"                                   => Ok(lex::cpp_tokens()),
        "csharp" | "cs" | "c#"                          => Ok(lex::csharp_tokens()),
        "go"                                            => Ok(lex::go_tokens()),
        "java"                                          => Ok(lex::java_tokens()),
        "js" | "javascript" | "typescript" | "ts"       => Ok(lex::js_tokens()),
        "python" | "py"                                 => Ok(lex::python_tokens()),
        _ => Err(err!(
            "Unsupported language: '{}'. Supported: rust, c, cpp, csharp, go, java, js, python",
            lang;
            Invalid, Input
        )),
    }
}
