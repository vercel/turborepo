//! Stylesheets as rule sets, and the rule-level comparison of two of them.
//!
//! A generated stylesheet (Tailwind) is one file for many routes: a new utility
//! class used by one page changes the file for all of them. Comparing rules
//! instead of bytes separates two cases:
//!
//! - only utility-class rules were added or removed (a selector that is one
//!   class, with optional pseudo-classes), plus custom properties and
//!   `@property` rules that no unchanged rule references: a route whose own
//!   output is unchanged renders the same, provided no class name is built at
//!   runtime (a route that used the class already had the rule; a route that
//!   starts using it changed itself);
//! - anything else (a changed declaration, an element selector, `:root` values
//!   that unchanged rules read, keyframes): every route loading the stylesheet
//!   is affected.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The rules of a set of stylesheets: `"<at-rule context> | <selector>"` →
/// declaration blocks (several if the selector occurs more than once).
/// Statements without a block (`@import`, `@layer a, b;`) have an empty block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CssRules(pub BTreeMap<String, Vec<String>>);

fn skip_string(s: &[u8], mut i: usize) -> usize {
    let q = s[i];
    i += 1;
    while i < s.len() && s[i] != q {
        if s[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    i
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn is_container(head: &str, body: &str) -> bool {
    const AT: &[&str] = &[
        "@media",
        "@supports",
        "@layer",
        "@container",
        "@scope",
        "@starting-style",
    ];
    AT.iter().any(|a| {
        head.strip_prefix(a)
            .is_some_and(|r| !r.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '-'))
    }) || (!head.starts_with('@') && body.contains('{'))
}

fn parse(s: &str, ctx: &str, out: &mut CssRules) {
    let b = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let key = |head: &str| format!("{ctx} | {head}");
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => i = skip_string(b, i),
            b';' => {
                let st = s[start..i].trim();
                if !st.is_empty() {
                    out.0.entry(key(st)).or_default().push(String::new());
                }
                start = i + 1;
            }
            b'{' => {
                let head = s[start..i].trim();
                let mut depth = 1;
                let mut j = i + 1;
                while j < b.len() && depth > 0 {
                    match b[j] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        b'"' | b'\'' => j = skip_string(b, j),
                        _ => {}
                    }
                    j += 1;
                }
                let body_end = if depth == 0 { j - 1 } else { j.min(b.len()) };
                let body = &s[(i + 1).min(body_end)..body_end];
                if is_container(head, body) {
                    let inner = if ctx.is_empty() {
                        head.to_string()
                    } else {
                        format!("{ctx} / {head}")
                    };
                    parse(body, &inner, out);
                } else {
                    out.0
                        .entry(key(head))
                        .or_default()
                        .push(body.trim().to_string());
                }
                i = j.saturating_sub(1);
                start = j.min(b.len());
            }
            _ => {}
        }
        i += 1;
    }
    let tail = s[start.min(s.len())..].trim();
    if !tail.is_empty() && !tail.contains('{') {
        out.0.entry(key(tail)).or_default().push(String::new());
    }
}

/// Adds the rules of stylesheet `text` to `out`.
pub fn parse_into(text: &str, out: &mut CssRules) {
    parse(&strip_comments(text), "", out);
}

/// The result of comparing two rule sets.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CssVerdict {
    /// Only utility rules and unreferenced custom properties differ.
    pub utilities_only: bool,
    /// Utility selectors only in the second set.
    pub added: Vec<String>,
    /// Utility selectors only in the first set.
    pub removed: Vec<String>,
    /// The other differences (rule keys, or the reason), at most 16.
    pub other: Vec<String>,
    /// Custom properties added or removed that no unchanged rule references.
    pub custom_properties: usize,
}

/// Whether `sel` is one class with optional pseudo-classes or pseudo-elements
/// (`.underline`, `.hover\:bg-\[\#bada55\]:hover`, `.a::before`).
fn is_utility(sel: &str) -> bool {
    let b = sel.as_bytes();
    if b.first() != Some(&b'.') {
        return false;
    }
    let mut i = 1;
    let class_start = i;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c >= 0x80 => i += 1,
            _ => break,
        }
    }
    if i == class_start || i > b.len() {
        return false;
    }
    // Pseudo suffixes: `:name` or `::name`, each with an optional `(...)` without
    // nesting.
    while i < b.len() {
        if b[i] != b':' {
            return false;
        }
        i += 1;
        if i < b.len() && b[i] == b':' {
            i += 1;
        }
        let name = i;
        while i < b.len() && (b[i].is_ascii_lowercase() || b[i] == b'-') {
            i += 1;
        }
        if i == name {
            return false;
        }
        if i < b.len() && b[i] == b'(' {
            match sel[i..].find(')') {
                Some(j) if !sel[i + 1..i + j].contains('(') => i += j + 1,
                _ => return false,
            }
        }
    }
    true
}

fn declarations(blocks: &[String]) -> BTreeSet<&str> {
    blocks
        .iter()
        .flat_map(|b| b.split(';'))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .collect()
}

/// The name of a custom property declaration (`--x: 1` → `--x`).
fn custom_property(decl: &str) -> Option<&str> {
    let (name, _) = decl.split_once(':')?;
    let name = name.trim();
    (name.starts_with("--")
        && name[2..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'))
    .then_some(name)
}

/// Compares two rule sets.
pub fn compare(a: &CssRules, b: &CssRules) -> CssVerdict {
    let mut v = CssVerdict::default();
    let mut vars: BTreeSet<String> = BTreeSet::new();
    let mut unchanged: Vec<&str> = Vec::new();
    let keys: BTreeSet<&String> = a.0.keys().chain(b.0.keys()).collect();
    for key in keys {
        let (x, y) = (a.0.get(key), b.0.get(key));
        if x == y {
            unchanged.extend(y.into_iter().flatten().map(String::as_str));
            continue;
        }
        let sel = key.split_once(" | ").map_or(key.as_str(), |(_, s)| s);
        match (x, y) {
            (Some(_), None) | (None, Some(_)) if is_utility(sel) => {
                if x.is_some() {
                    v.removed.push(sel.to_string());
                } else {
                    v.added.push(sel.to_string());
                }
            }
            (Some(_), None) | (None, Some(_)) if sel.starts_with("@property --") => {
                vars.insert(sel["@property ".len()..].trim().to_string());
            }
            (Some(x), Some(y)) => {
                let (dx, dy) = (declarations(x), declarations(y));
                let diff: Vec<&str> = dx.symmetric_difference(&dy).copied().collect();
                if diff.iter().all(|d| custom_property(d).is_some()) {
                    vars.extend(
                        diff.iter()
                            .filter_map(|d| custom_property(d))
                            .map(String::from),
                    );
                    unchanged.extend(dx.intersection(&dy));
                } else {
                    v.other.push(key.clone());
                }
            }
            _ => v.other.push(key.clone()),
        }
    }
    // A custom property that an unchanged rule reads changes that rule's rendering.
    for name in &vars {
        let (plain, with_fallback) = (format!("var({name})"), format!("var({name},"));
        if unchanged
            .iter()
            .any(|t| t.contains(&plain) || t.contains(&with_fallback))
        {
            v.other
                .push(format!("custom property {name} is read by unchanged rules"));
        }
    }
    v.custom_properties = vars.len();
    v.utilities_only = v.other.is_empty();
    v.other.truncate(16);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(css: &str) -> CssRules {
        let mut r = CssRules::default();
        parse_into(css, &mut r);
        r
    }

    const BASE: &str = "@layer theme{:root,:host{--text-xl:1.25rem;--font:ui}}@layer \
                        base{body{font-family:var(--font)}}@layer \
                        utilities{.text-xl{font-size:var(--text-xl)}.p-4{padding:1rem}@media \
                        (hover:hover){.hover\\:x:hover{color:red}}}.legacy-box{border:1px solid}";

    #[test]
    fn parses_nested_rules_with_context() {
        let r = rules(BASE);
        assert_eq!(
            r.0["@layer utilities | .text-xl"],
            vec!["font-size:var(--text-xl)"]
        );
        assert!(r.0.contains_key("@layer theme | :root,:host"));
        assert!(r.0.contains_key("@layer utilities / @media (hover:hover) | .hover\\:x:hover"));
        assert!(r.0.contains_key(" | .legacy-box"));
        let r = rules("@import \"x.css\";/* c */@layer a,b;a{b:\"}\";c:d}");
        assert!(r.0.contains_key(" | @import \"x.css\""));
        assert!(r.0.contains_key(" | @layer a,b"));
        assert_eq!(r.0[" | a"], vec!["b:\"}\";c:d"]);
    }

    #[test]
    fn utility_selectors() {
        for s in [
            ".underline",
            ".hover\\:bg-\\[\\#bada55\\]:hover",
            ".a::before",
            ".x:nth-child(2)",
        ] {
            assert!(is_utility(s), "{s}");
        }
        for s in [
            "body",
            ".a .b",
            ".a,.b",
            ":root",
            ".a>p",
            "@keyframes x",
            ".a:is(.b(c))",
        ] {
            assert!(!is_utility(s), "{s}");
        }
    }

    #[test]
    fn new_utility_with_its_theme_variable_is_utilities_only() {
        let head = BASE
            .replace("--font:ui}", "--font:ui;--text-2xl:1.5rem}")
            .replace(
                ".p-4{",
                ".text-2xl{font-size:var(--text-2xl)}.underline{text-decoration-line:underline}.\
                 p-4{",
            );
        let v = compare(&rules(BASE), &rules(&head));
        assert!(v.utilities_only, "{v:?}");
        assert_eq!(v.added, vec![".text-2xl", ".underline"]);
        assert_eq!(v.custom_properties, 1);
        let back = compare(&rules(&head), &rules(BASE));
        assert!(back.utilities_only);
        assert_eq!(back.removed, vec![".text-2xl", ".underline"]);
    }

    #[test]
    fn other_changes_affect_every_route() {
        let base = rules(BASE);
        // A changed declaration of an existing rule.
        assert!(
            !compare(&base, &rules(&BASE.replace("padding:1rem", "padding:2rem"))).utilities_only
        );
        // An element selector.
        assert!(!compare(&base, &rules(&format!("{BASE}h1{{margin:0}}"))).utilities_only);
        // A custom property that an unchanged rule reads.
        let v = compare(&base, &rules(&BASE.replace("--font:ui", "--font:serif")));
        assert!(!v.utilities_only);
        assert!(v.other[0].contains("--font"), "{v:?}");
        // The same set.
        let same = compare(&base, &base);
        assert!(same.utilities_only && same.added.is_empty() && same.removed.is_empty());
    }
}
