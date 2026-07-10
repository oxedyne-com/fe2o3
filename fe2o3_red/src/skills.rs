//! Skills — named markdown instruction bundles the agent can invoke.
//!
//! A skill is a markdown file stored in the workspace at
//! `.red/skills/<name>.md` with a light YAML-ish frontmatter block:
//!
//! ```text
//! ---
//! name: review
//! description: Review a diff for bugs
//! ---
//! <the markdown instruction body...>
//! ```
//!
//! Skills are invoked from chat with an angle-tag directive
//! `<name args...>`, optionally closed with `</name>` or a bare `</>`.
//! Parsing is deliberately tolerant (plan D9): only the *opening* tag is
//! terminated by `>`, so a `>` inside the body — such as `Vec<T>` or
//! `->` — is safe and does not end the directive.  A missing `>` on the
//! opening tag recovers to end-of-line, and a missing closing tag
//! recovers to end-of-message.

use oxedyne_fe2o3_core::prelude::*;

use crate::workspace::Workspace;


/// A named markdown instruction bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Skill {
    /// The skill's invocation name (frontmatter `name`, or the file stem).
    pub name:        String,
    /// One-line description for autocomplete and listings.
    pub description: String,
    /// The markdown instruction body (everything after the frontmatter).
    pub body:        String,
}

/// A parsed chat invocation of a skill directive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillInvocation {
    /// The skill name from the opening tag.
    pub name: String,
    /// The remainder of the opening tag after the name, trimmed.
    pub args: String,
    /// The directive body between the opening and closing tags.
    pub body: String,
}


/// True if `c` is a legal character in a skill name / tag identifier.
fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The workspace-relative directory skills are stored in.
const SKILLS_DIR: &str = ".red/skills";


/// List every skill in the workspace's `.red/skills` directory.
///
/// Returns an empty vector (not an error) when the directory does not
/// exist.  Unreadable files are skipped.  Results are sorted by name.
pub fn list_skills(ws: &Workspace)
    -> Outcome<Vec<Skill>>
{
    let dir = res!(ws.resolve(SKILLS_DIR));
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let rd = res!(std::fs::read_dir(&dir)
        .map_err(|e| err!(e, "list_skills: cannot read '{}'.", SKILLS_DIR; IO, File, Read)));
    let mut out = Vec::new();
    for ent in rd.filter_map(|e| e.ok()) {
        let p = ent.path();
        // Only consider `*.md` files.
        if p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let stem = p.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // Skip files we cannot read as UTF-8 text.
        let text = match std::fs::read_to_string(&p) {
            Ok(t)  => t,
            Err(_) => continue,
        };
        out.push(parse_skill(&text, &stem));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Load a single skill by name, or `None` if no such skill exists.
pub fn load_skill(ws: &Workspace, name: &str)
    -> Outcome<Option<Skill>>
{
    let skills = res!(list_skills(ws));
    for s in skills {
        if s.name == name {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

/// Parse a skill file's text into a [`Skill`], using `stem` as the
/// fallback name when the frontmatter omits `name`.
fn parse_skill(text: &str, stem: &str) -> Skill {
    let mut name        = stem.to_string();
    let mut description = String::new();

    let lines: Vec<&str> = text.lines().collect();
    // Frontmatter must open with a `---` line at the very top.
    if !lines.is_empty() && lines[0].trim() == "---" {
        // Find the closing `---` line.
        let mut close = None;
        for (i, line) in lines.iter().enumerate().skip(1) {
            if line.trim() == "---" {
                close = Some(i);
                break;
            }
        }
        if let Some(j) = close {
            // Parse `key: value` pairs between the fences.
            for line in &lines[1..j] {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim();
                    let val = v.trim();
                    match key {
                        "name" => {
                            // Only override the stem when a value is present.
                            if !val.is_empty() {
                                name = val.to_string();
                            }
                        }
                        "description" => description = val.to_string(),
                        _             => {}
                    }
                }
            }
            let body = lines[j + 1..].join("\n").trim().to_string();
            return Skill { name, description, body };
        }
    }
    // No frontmatter — the whole file is the body.
    Skill { name, description, body: text.trim().to_string() }
}


/// Parse the first skill-directive opening tag in `input`.
///
/// Returns `None` when there is no plausible opening tag (a `<` followed
/// by an identifier character).  This is purely syntactic; matching the
/// name against real skills happens in [`expand`].
pub fn parse_invocation(input: &str) -> Option<SkillInvocation> {
    // Find the first `<` immediately followed by an identifier character.
    for (lt, _) in input.match_indices('<') {
        let name_start = lt + 1;
        let after = &input[name_start..];
        // The name is the leading run of identifier characters.
        let name_len = after
            .find(|c: char| !is_ident(c))
            .unwrap_or(after.len());
        if name_len == 0 {
            continue; // e.g. a closing `</...>` or a bare `<`.
        }
        let name = after[..name_len].to_string();
        let name_end = name_start + name_len;
        let rest = &input[name_end..]; // args, `>`, then the body.

        // Terminate the opening tag at the first `>`, unless a newline
        // comes first (a missing `>` recovers to end-of-line).
        let gt = rest.find('>');
        let nl = rest.find('\n');
        let (args, body_start) = match gt {
            Some(g) if nl.map_or(true, |n| g < n) => {
                // Normal case: opening tag closed by `>`.
                (rest[..g].trim().to_string(), name_end + g + 1)
            }
            _ => {
                // Missing `>`: recover to end-of-line (or end of input).
                match nl {
                    Some(n) => (rest[..n].trim().to_string(), name_end + n + 1),
                    None    => (rest.trim().to_string(), input.len()),
                }
            }
        };

        // The body runs to a matching `</name>` or bare `</>`, else to
        // the end of the input.
        let region = &input[body_start..];
        let close_named = fmt!("</{}>", name);
        let end_named   = region.find(&close_named);
        let end_bare    = region.find("</>");
        let end = match (end_named, end_bare) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None)    => a,
            (None, Some(b))    => b,
            (None, None)       => region.len(),
        };
        let body = region[..end].trim().to_string();

        return Some(SkillInvocation { name, args, body });
    }
    None
}

/// Expand a chat message, injecting a matching skill's instructions.
///
/// If the message opens with a skill directive whose name resolves to a
/// stored skill, the returned string is the skill's instruction body
/// followed by the user's supplied args/body.  Otherwise the input is
/// returned unchanged.
pub fn expand(input: &str, ws: &Workspace)
    -> Outcome<String>
{
    if let Some(inv) = parse_invocation(input) {
        if let Some(skill) = res!(load_skill(ws, &inv.name)) {
            // Combine the invocation's args and body into one request.
            let mut request = inv.args.clone();
            if !inv.body.is_empty() {
                if !request.is_empty() {
                    request.push('\n');
                }
                request.push_str(&inv.body);
            }
            let composed = fmt!("{}\n\nUser request: {}", skill.body, request);
            return Ok(composed);
        }
    }
    Ok(input.to_string())
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_ws() -> Workspace {
        let mut dir = std::env::temp_dir();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(fmt!("red_skills_test_{}", n));
        Workspace::new(dir).expect("workspace")
    }

    /// Write a skill file into the workspace's `.red/skills` directory.
    fn write_skill(ws: &Workspace, name: &str, content: &str) {
        let dir = ws.resolve(SKILLS_DIR).expect("resolve skills dir");
        std::fs::create_dir_all(&dir).expect("create skills dir");
        let path = dir.join(fmt!("{}.md", name));
        std::fs::write(&path, content).expect("write skill");
    }

    // ── parse_invocation ────────────────────────────────────────────

    #[test]
    fn test_parse_plain() {
        let inv = parse_invocation("<review>").expect("parse");
        assert_eq!(inv.name, "review");
        assert_eq!(inv.args, "");
        assert_eq!(inv.body, "");
    }

    #[test]
    fn test_parse_args_captured() {
        let inv = parse_invocation("<review focus=errors>").expect("parse");
        assert_eq!(inv.name, "review");
        assert_eq!(inv.args, "focus=errors");
        assert_eq!(inv.body, "");
    }

    #[test]
    fn test_parse_multiline_body_explicit_close() {
        let input = "<review>\nfirst line\nsecond line\n</review>";
        let inv = parse_invocation(input).expect("parse");
        assert_eq!(inv.name, "review");
        assert!(inv.body.contains("first line"));
        assert!(inv.body.contains("second line"));
        assert!(!inv.body.contains("</review>"));
    }

    #[test]
    fn test_parse_bare_close() {
        let inv = parse_invocation("<note>remember this</>").expect("parse");
        assert_eq!(inv.name, "note");
        assert_eq!(inv.body, "remember this");
    }

    #[test]
    fn test_parse_missing_close_body_to_end() {
        let inv = parse_invocation("<review>do the whole thing").expect("parse");
        assert_eq!(inv.name, "review");
        assert_eq!(inv.body, "do the whole thing");
    }

    #[test]
    fn test_parse_gt_inside_body() {
        // A `>` inside the body (Vec<T>, ->) must NOT end the body.
        let input = "<fix> convert Vec<T> -> Vec<U> </fix>";
        let inv = parse_invocation(input).expect("parse");
        assert_eq!(inv.name, "fix");
        assert!(inv.body.contains("Vec<T>"), "body was: {:?}", inv.body);
        assert!(inv.body.contains("->"),     "body was: {:?}", inv.body);
        assert!(inv.body.contains("Vec<U>"), "body was: {:?}", inv.body);
    }

    #[test]
    fn test_parse_missing_gt_recovers_to_eol() {
        // No `>` on the opening tag: recover to end-of-line; body follows.
        let input = "<review focus=bugs\nplease look here";
        let inv = parse_invocation(input).expect("parse");
        assert_eq!(inv.name, "review");
        assert_eq!(inv.args, "focus=bugs");
        assert_eq!(inv.body, "please look here");
    }

    #[test]
    fn test_parse_no_invocation() {
        assert!(parse_invocation("just some plain prose here").is_none());
        assert!(parse_invocation("no tags at all, only words").is_none());
        // A `<` not followed by an identifier is not an opening tag.
        assert!(parse_invocation("3 < 4 and 5 < 6").is_none());
        assert!(parse_invocation("closing only </review>").is_none());
    }

    #[test]
    fn test_parse_finds_first_tag() {
        let inv = parse_invocation("prefix text <run go> then more").expect("parse");
        assert_eq!(inv.name, "run");
        assert_eq!(inv.args, "go");
        assert_eq!(inv.body, "then more");
    }

    // ── frontmatter parsing ─────────────────────────────────────────

    #[test]
    fn test_parse_skill_frontmatter() {
        let text = "---\nname: review\ndescription: Review a diff for bugs\n---\nDo the review carefully.";
        let s = parse_skill(text, "review");
        assert_eq!(s.name, "review");
        assert_eq!(s.description, "Review a diff for bugs");
        assert_eq!(s.body, "Do the review carefully.");
    }

    #[test]
    fn test_parse_skill_name_falls_back_to_stem() {
        let text = "---\ndescription: no name here\n---\nbody text";
        let s = parse_skill(text, "myfile");
        assert_eq!(s.name, "myfile");
        assert_eq!(s.description, "no name here");
        assert_eq!(s.body, "body text");
    }

    #[test]
    fn test_parse_skill_no_frontmatter() {
        let text = "just a plain body, no frontmatter";
        let s = parse_skill(text, "plain");
        assert_eq!(s.name, "plain");
        assert_eq!(s.description, "");
        assert_eq!(s.body, "just a plain body, no frontmatter");
    }

    // ── list_skills / load_skill ────────────────────────────────────

    #[test]
    fn test_list_skills_missing_dir_is_empty() {
        let ws = tmp_ws();
        let skills = list_skills(&ws).expect("list");
        assert!(skills.is_empty());
    }

    #[test]
    fn test_list_skills_roundtrip() {
        let ws = tmp_ws();
        write_skill(&ws, "foo",
            "---\nname: foo\ndescription: The foo skill\n---\nfoo instructions");
        let skills = list_skills(&ws).expect("list");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "foo");
        assert_eq!(skills[0].description, "The foo skill");
        assert_eq!(skills[0].body, "foo instructions");
    }

    #[test]
    fn test_load_skill() {
        let ws = tmp_ws();
        write_skill(&ws, "review",
            "---\nname: review\ndescription: Review a diff\n---\nReview instructions here.");
        let found = load_skill(&ws, "review").expect("load");
        let skill = found.expect("some skill");
        assert_eq!(skill.name, "review");
        assert_eq!(skill.body, "Review instructions here.");
        assert!(load_skill(&ws, "absent").expect("load absent").is_none());
    }

    // ── expand ──────────────────────────────────────────────────────

    #[test]
    fn test_expand_with_matching_skill() {
        let ws = tmp_ws();
        write_skill(&ws, "review",
            "---\nname: review\ndescription: Review a diff\n---\nReview the diff for bugs.");
        let out = expand("<review focus=errors>look at handler.rs</review>", &ws)
            .expect("expand");
        assert!(out.contains("Review the diff for bugs."));
        assert!(out.contains("User request:"));
        assert!(out.contains("focus=errors"));
        assert!(out.contains("look at handler.rs"));
    }

    #[test]
    fn test_expand_without_matching_skill() {
        let ws = tmp_ws();
        // No skill file — the directive name does not resolve.
        let input = "<review>do it</review>";
        let out = expand(input, &ws).expect("expand");
        assert_eq!(out, input);
    }

    #[test]
    fn test_expand_plain_prose_unchanged() {
        let ws = tmp_ws();
        let input = "just chatting, no directive";
        let out = expand(input, &ws).expect("expand");
        assert_eq!(out, input);
    }
}
