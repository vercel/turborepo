//! Inline suppressions, shared by every rule.
//!
//! ```js
//! // turbo-trace-ignore unconstrained-load: plugins are installed by the user, not in the repo
//! const plugin = require(name);
//! const other = require(name2); // turbo-trace-ignore unconstrained-load: same
//! ```
//!
//! The comment is on the finding's line or on the line directly above it. `//`,
//! `#` and a one-line `/* … */` open it; several rules are separated by commas;
//! the reason after the colon is mandatory.
//!
//! A suppression silences the check, nothing else: the selection still treats
//! the load as unbounded (it stays on the uncertainty floor), which is the safe
//! direction.
//!
//! Only the files that have findings are read. In those files:
//!
//! - a comment without a reason, without a rule or naming an unknown rule is an
//!   `invalid-suppression` finding (an error) and suppresses nothing;
//! - a comment that matches no finding of an enabled rule is an
//!   `unused-suppression` finding (a warning). A rule that is switched off is
//!   not judged.

use std::path::Path;

use super::{Finding, INVALID_SUPPRESSION, UNUSED_SUPPRESSION, rule_names};

/// The marker that starts a suppression inside a comment.
pub const MARKER: &str = "turbo-trace-ignore";

/// A suppression comment as written.
#[derive(Debug, PartialEq, Eq)]
pub struct Comment<'a> {
    /// The rules it names.
    pub rules: Vec<&'a str>,
    /// Why.
    pub reason: &'a str,
}

/// Parses one source line. `None`: the line has no suppression comment.
/// `Some(Err(why))`: it has one that is malformed.
pub fn parse_line(line: &str) -> Option<Result<Comment<'_>, &'static str>> {
    let at = line.find(MARKER)?;
    // The marker must open a comment.
    let before = line[..at].trim_end();
    let block = before.ends_with("/*");
    if !(block || before.ends_with("//") || before.ends_with('#')) {
        return None;
    }
    let mut rest = &line[at + MARKER.len()..];
    // `turbo-trace-ignored`, `turbo-trace-ignore-file`: another word.
    if rest
        .chars()
        .next()
        .is_some_and(|c| !(c.is_whitespace() || c == ':'))
    {
        return None;
    }
    if block && let Some(end) = rest.find("*/") {
        rest = &rest[..end];
    }
    const SYNTAX: &str = "expected `turbo-trace-ignore <rule>: <reason>`";
    let Some((rules, reason)) = rest.split_once(':') else {
        return Some(Err(
            "a suppression needs a reason: expected `turbo-trace-ignore <rule>: <reason>`"
        ));
    };
    let rules: Vec<&str> = rules.split(',').map(str::trim).collect();
    if rules.iter().any(|r| r.is_empty()) {
        return Some(Err(if rules.len() == 1 {
            "a suppression names the rule it silences: expected `turbo-trace-ignore <rule>: \
             <reason>`"
        } else {
            SYNTAX
        }));
    }
    let reason = reason.trim();
    if reason.is_empty() {
        return Some(Err(
            "a suppression needs a reason: expected `turbo-trace-ignore <rule>: <reason>`"
        ));
    }
    Some(Ok(Comment { rules, reason }))
}

/// The result of [`apply`].
pub struct Applied {
    /// Per finding, in the order given: the reason of the comment that
    /// suppresses it.
    pub reasons: Vec<Option<String>>,
    /// `invalid-suppression` and `unused-suppression` findings.
    pub meta: Vec<Finding>,
    /// Source files read.
    pub files_read: usize,
}

/// A valid comment of a file.
struct Valid {
    line: u32,
    rules: Vec<String>,
    reason: String,
    /// Per rule: whether a finding matched.
    used: Vec<bool>,
}

fn meta(rule: &'static str, file: &str, line: u32, message: String, fix: String) -> Finding {
    Finding {
        rule,
        file: file.to_owned(),
        line,
        key: String::new(),
        message,
        cost: None,
        fix,
    }
}

/// The comments of one file's text: the valid ones, and a finding per invalid
/// one. `enabled`: `None` for an unknown rule name.
fn comments(
    file: &str,
    text: &str,
    enabled: &dyn Fn(&str) -> Option<bool>,
    meta_out: &mut Vec<Finding>,
) -> Vec<Valid> {
    let mut valid = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line_no = i as u32 + 1;
        let (why, fix) = match parse_line(line) {
            None => continue,
            Some(Err(why)) => (
                why.to_owned(),
                "write `// turbo-trace-ignore <rule>: <reason>` on the finding's line or the line \
                 above it"
                    .to_owned(),
            ),
            Some(Ok(c)) => match c.rules.iter().find(|r| enabled(r).is_none()) {
                Some(unknown) => (
                    format!("a suppression names the unknown rule `{unknown}`"),
                    format!("the rules are {}", rule_names()),
                ),
                None => {
                    valid.push(Valid {
                        line: line_no,
                        used: vec![false; c.rules.len()],
                        rules: c.rules.iter().map(|r| (*r).to_owned()).collect(),
                        reason: c.reason.to_owned(),
                    });
                    continue;
                }
            },
        };
        meta_out.push(meta(INVALID_SUPPRESSION, file, line_no, why, fix));
    }
    valid
}

/// Applies the suppression comments of the files that have findings. `findings`
/// must be grouped by file. `enabled(rule)`: whether the rule ran; `None` for
/// an unknown name.
pub fn apply(root: &Path, findings: &[Finding], enabled: &dyn Fn(&str) -> Option<bool>) -> Applied {
    let mut out = Applied {
        reasons: vec![None; findings.len()],
        meta: Vec::new(),
        files_read: 0,
    };
    let mut start = 0;
    while start < findings.len() {
        let file = findings[start].file.as_str();
        let end = start
            + findings[start..]
                .iter()
                .take_while(|f| f.file == file)
                .count();
        let range = start..end;
        start = end;
        // A file that is gone or is not text has no comments.
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        out.files_read += 1;
        if !text.contains(MARKER) {
            continue;
        }
        let mut valid = comments(file, &text, enabled, &mut out.meta);
        for (f, reason) in findings[range.clone()].iter().zip(&mut out.reasons[range]) {
            if f.line == 0 {
                continue;
            }
            for c in valid
                .iter_mut()
                .filter(|c| c.line == f.line || c.line + 1 == f.line)
            {
                if let Some(k) = c.rules.iter().position(|r| r == f.rule) {
                    c.used[k] = true;
                    reason.get_or_insert_with(|| c.reason.clone());
                }
            }
        }
        for c in &valid {
            for (r, _) in (c.rules.iter().zip(&c.used))
                .filter(|(r, used)| !**used && enabled(r) == Some(true))
            {
                out.meta.push(meta(
                    UNUSED_SUPPRESSION,
                    file,
                    c.line,
                    format!("this suppression matches no `{r}` finding on its line or the next"),
                    "remove the comment".to_owned(),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(line: &str) -> (Vec<&str>, &str) {
        let c = parse_line(line)
            .unwrap_or_else(|| panic!("no comment in {line:?}"))
            .unwrap_or_else(|e| panic!("{line:?}: {e}"));
        (c.rules, c.reason)
    }

    #[test]
    fn parses_comments() {
        assert_eq!(
            ok("// turbo-trace-ignore unconstrained-load: plugins come from the user"),
            (vec!["unconstrained-load"], "plugins come from the user")
        );
        // Trailing, with several rules; the reason may contain colons.
        assert_eq!(
            ok("  require(x); //turbo-trace-ignore a, b-c :  see docs: https://example.com/x"),
            (vec!["a", "b-c"], "see docs: https://example.com/x")
        );
        // Python and shell.
        assert_eq!(ok("# turbo-trace-ignore a: why"), (vec!["a"], "why"));
        assert_eq!(
            ok("x = load(n)  # turbo-trace-ignore a: why"),
            (vec!["a"], "why")
        );
        // A block comment on one line, also in JSX.
        assert_eq!(ok("/* turbo-trace-ignore a: why */"), (vec!["a"], "why"));
        assert_eq!(ok("{/* turbo-trace-ignore a: why */}"), (vec!["a"], "why"));
        assert_eq!(
            ok("f(x) /* turbo-trace-ignore a: why */ + g(y)"),
            (vec!["a"], "why")
        );
    }

    #[test]
    fn rejects_comments_without_reason_or_rule() {
        let err = |line: &str| parse_line(line).expect("a comment").unwrap_err();
        assert!(err("// turbo-trace-ignore unconstrained-load").contains("needs a reason"));
        assert!(err("// turbo-trace-ignore unconstrained-load:").contains("needs a reason"));
        assert!(err("// turbo-trace-ignore unconstrained-load:   ").contains("needs a reason"));
        assert!(err("/* turbo-trace-ignore a: */").contains("needs a reason"));
        assert!(err("// turbo-trace-ignore").contains("needs a reason"));
        assert!(err("// turbo-trace-ignore : because").contains("names the rule"));
        assert!(err("// turbo-trace-ignore a,, b: because").contains("expected"));
    }

    #[test]
    fn ignores_text_that_is_not_a_comment() {
        // In a string, in an identifier, or another word.
        assert_eq!(parse_line("const s = 'turbo-trace-ignore a: b';"), None);
        assert_eq!(parse_line("x.turbo-trace-ignore a: b"), None);
        assert_eq!(parse_line("// turbo-trace-ignored a: b"), None);
        assert_eq!(parse_line("// turbo-trace-ignore-next a: b"), None);
        assert_eq!(parse_line("// turbo-trace-ignore-file a: b"), None);
        assert_eq!(parse_line("const a = 1;"), None);
    }

    fn finding(rule: &'static str, file: &str, line: u32) -> Finding {
        Finding {
            rule,
            file: file.to_owned(),
            line,
            key: "k".to_owned(),
            message: "m".to_owned(),
            cost: None,
            fix: String::new(),
        }
    }

    #[test]
    fn applies_to_the_line_and_the_line_below() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("a.js"),
            "// turbo-trace-ignore load: above\nrequire(a);\nrequire(b); // turbo-trace-ignore \
             load: same line\n\nrequire(c);\n// turbo-trace-ignore load: nothing below\n\n// \
             turbo-trace-ignore load\nrequire(d);\n// turbo-trace-ignore nope: unknown rule\n// \
             turbo-trace-ignore read: the rule is off\n// turbo-trace-ignore load, read: one of \
             two\nrequire(e);\n",
        )
        .unwrap();
        std::fs::write(d.path().join("b.js"), "require(x);\n").unwrap();
        let findings = [
            finding("load", "a.js", 2),
            finding("load", "a.js", 3),
            finding("load", "a.js", 5),
            finding("load", "a.js", 9),
            finding("load", "a.js", 13),
            finding("load", "b.js", 1),
            finding("load", "gone.js", 1),
        ];
        let enabled = |r: &str| match r {
            "load" => Some(true),
            "read" => Some(false),
            _ => None,
        };
        let a = apply(d.path(), &findings, &enabled);
        assert_eq!(a.files_read, 2);
        let reasons: Vec<Option<&str>> = a.reasons.iter().map(|r| r.as_deref()).collect();
        assert_eq!(
            reasons,
            [
                Some("above"),
                Some("same line"),
                None,
                None,
                Some("one of two"),
                None,
                None
            ]
        );
        let meta: Vec<(&str, u32)> = a.meta.iter().map(|f| (f.rule, f.line)).collect();
        assert_eq!(
            meta,
            [
                // Without a reason; an unknown rule.
                (INVALID_SUPPRESSION, 8),
                (INVALID_SUPPRESSION, 10),
                // Nothing on line 6 or 7. The comment for the rule that is off (line 11)
                // and the `read` half of line 12 are not judged.
                (UNUSED_SUPPRESSION, 6),
            ]
        );
        assert!(a.meta[1].message.contains("unknown rule `nope`"));
        assert!(a.meta.iter().all(|f| f.file == "a.js"));
    }
}
