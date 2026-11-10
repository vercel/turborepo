//! Parser tests: structure (via the canonical `Display` form) and error
//! messages.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_graph::{ExprKind, parse};

fn canon(src: &str) -> String {
    parse(src)
        .unwrap_or_else(|e| panic!("{}", e.render(src)))
        .to_string()
}

fn err(src: &str) -> String {
    parse(src).expect_err("should fail").render(src)
}

#[test]
fn literals_and_calls() {
    assert_eq!(canon("deps(src/a.ts)"), "deps(src/a.ts)");
    assert_eq!(canon("  deps( src/a.ts , 2 ) "), "deps(src/a.ts, 2)");
    assert_eq!(canon("src/*.{ts,tsx}"), "src/*.{ts,tsx}");
    assert_eq!(canon("src/[ab]*.ts"), "src/[ab]*.ts");
    assert_eq!(canon("\"dir with space/x.ts\""), "\"dir with space/x.ts\"");
    assert_eq!(canon("'it\\'s.ts'"), "\"it's.ts\"");
    assert_eq!(canon("pkg:@vercel/foo@1.2.3"), "pkg:@vercel/foo@1.2.3");
    assert_eq!(
        canon("kind(import|require, **)"),
        "kind(import|require, **)"
    );
    assert_eq!(canon("my-file.ts"), "my-file.ts");
    assert_eq!(canon("f()"), "f()");
    assert_eq!(canon("$changes"), "$changes");
    assert_eq!(canon("ünïcode/ß.ts"), "ünïcode/ß.ts");
}

#[test]
fn operators_are_left_associative_with_equal_precedence() {
    assert_eq!(canon("a + b - c ^ d"), "(((a + b) - c) ^ d)");
    assert_eq!(
        canon("a union b except c intersect d"),
        "(((a + b) - c) ^ d)"
    );
    assert_eq!(canon("a + (b - c)"), "(a + (b - c))");
    assert_eq!(
        canon("my-file.ts - other-file.ts"),
        "(my-file.ts - other-file.ts)"
    );
    assert_eq!(canon("a -b"), "(a - b)");
    assert_eq!(canon("a+b"), "(a + b)");
}

#[test]
fn let_bindings() {
    assert_eq!(
        canon("let x = deps(a) in $x ^ b"),
        "(let x = deps(a) in ($x ^ b))"
    );
    assert_eq!(
        canon("let x = a + b in let y = $x in $y"),
        "(let x = (a + b) in (let y = $x in $y))"
    );
    assert_eq!(canon("a + let x = b in $x"), "(a + (let x = b in $x))");
}

#[test]
fn keywords_are_contextual() {
    // `in`, `let`, `union` are literals where an expression is expected.
    assert_eq!(canon("deps(in)"), "deps(in)");
    assert_eq!(canon("let"), "let");
    assert_eq!(canon("union"), "union");
    assert_eq!(canon("let + in"), "(let + in)");
}

#[test]
fn spans() {
    let e = parse("deps(src/a.ts) + b").unwrap();
    assert_eq!((e.span.start, e.span.end), (0, 18));
    let ExprKind::Binary { lhs, .. } = &e.kind else {
        panic!()
    };
    assert_eq!((lhs.span.start, lhs.span.end), (0, 14));
}

#[test]
fn error_unclosed_call_renders_caret() {
    assert_eq!(
        err("deps(src/a.ts, 2"),
        "error: expected `,` or `)`, found end of input\n --> 1:17\n  |\n1 | deps(src/a.ts, 2\n  \
         |                 ^\n  = hint: the `(` of `deps` at byte 4 is not closed"
    );
}

#[test]
fn error_messages() {
    let cases: &[(&str, &str, &str)] = &[
        ("", "expected an expression, found end of input", "^"),
        (
            "deps(a))",
            "expected end of input, found `)`",
            "has no matching `(`",
        ),
        (
            "a b",
            "expected end of input, found `b`",
            "combine sets with",
        ),
        ("+ a", "expected an expression, found `+`", "^"),
        ("deps(a,)", "expected an expression, found `)`", "^"),
        ("a & b", "unexpected character `&`", "quote literals"),
        (
            "-x.ts",
            "expected an expression, found `-`",
            "cannot start with `-`",
        ),
        ("\"abc", "unterminated string", "close it with \""),
        (
            "let x = a b",
            "expected `in` after the `let` value, found `b`",
            "^",
        ),
        ("src/{a,b", "unclosed `{` in word", "quote the pattern"),
        (
            "(a + b",
            "expected `)`, found end of input",
            "the `(` at byte 0",
        ),
        ("src/x.ts(a)", "`src/x.ts` is not a function name", "^"),
        ("$", "expected a variable name after `$`", "^"),
    ];
    for (src, msg, extra) in cases {
        let e = parse(src).expect_err(src);
        assert_eq!(e.message, *msg, "for {src:?}");
        let r = e.render(src);
        assert!(r.contains(extra), "for {src:?}:\n{r}");
    }
}

#[test]
fn error_position_on_second_line() {
    let src = "deps(a)\n  + )";
    let r = err(src);
    assert!(r.contains("--> 2:5"), "{r}");
    assert!(r.contains("2 |   + )\n  |     ^"), "{r}");
}

#[test]
fn caret_width_covers_token() {
    let src = "a foo.ts";
    let r = err(src);
    assert!(
        r.ends_with("  |   ^^^^^^\n  = hint: combine sets with `+`, `-` or `^`"),
        "{r}"
    );
}
