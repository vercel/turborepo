//! Lexer unit tests.

use super::*;

fn lex_all(src: &str, dialect: Dialect) -> (Vec<(TokKind, String)>, Lexer<'_>) {
    let mut lx = Lexer::new(src.as_bytes(), dialect);
    let mut out = Vec::new();
    for _ in 0..100_000 {
        let t = lx.next_token();
        if t.kind == TokKind::Eof {
            break;
        }
        out.push((
            t.kind,
            String::from_utf8_lossy(t.text(src.as_bytes())).into_owned(),
        ));
    }
    (out, lx)
}

fn toks(src: &str, dialect: Dialect) -> Vec<(TokKind, String)> {
    lex_all(src, dialect).0
}

/// Kinds of all tokens.
fn kinds(src: &str, dialect: Dialect) -> Vec<TokKind> {
    toks(src, dialect).into_iter().map(|(k, _)| k).collect()
}

/// Texts of tokens of `kind`.
fn of(src: &str, dialect: Dialect, kind: TokKind) -> Vec<String> {
    toks(src, dialect)
        .into_iter()
        .filter(|(k, _)| *k == kind)
        .map(|(_, s)| s)
        .collect()
}

fn regexes(src: &str) -> Vec<String> {
    of(src, Dialect::JS, TokKind::Regex)
}

fn regexes_ts(src: &str) -> Vec<String> {
    of(src, Dialect::TS, TokKind::Regex)
}

fn strings(src: &str, dialect: Dialect) -> Vec<String> {
    of(src, dialect, TokKind::Str)
}

fn no_error(src: &str, dialect: Dialect) {
    let (_, mut lx) = lex_all(src, dialect);
    assert_eq!(lx.take_error(), None, "unexpected error for {src:?}");
    assert!(!lx.jsx_failed(), "jsx failed for {src:?}");
}

use TokKind as K;

#[test]
fn basic_tokens() {
    assert_eq!(
        toks("import x from 'a';", Dialect::JS),
        vec![
            (K::Ident, "import".into()),
            (K::Ident, "x".into()),
            (K::Ident, "from".into()),
            (K::Str, "'a'".into()),
            (K::Semi, ";".into()),
        ]
    );
    assert_eq!(
        kinds("a?.b ?? c ... => === !== ++ -- **= >>>= ?.5", Dialect::JS),
        vec![
            K::Ident,
            K::OptChain,
            K::Ident,
            K::Op,
            K::Ident,
            K::Ellipsis,
            K::Arrow,
            K::Op,
            K::Op,
            K::IncDec,
            K::IncDec,
            K::Op,
            K::Gt,
            K::Gt,
            K::Gt,
            K::Assign,
            K::Question,
            K::Number,
        ]
    );
}

#[test]
fn keywords_after_dot_are_identifiers() {
    let mut lx = Lexer::new(b"a.import x.require import", Dialect::JS);
    let mut kws = Vec::new();
    loop {
        let t = lx.next_token();
        if t.kind == K::Eof {
            break;
        }
        kws.push(t.kw);
    }
    assert_eq!(
        kws,
        [
            Kw::None,
            Kw::None,
            Kw::None,
            Kw::None,
            Kw::None,
            Kw::None,
            Kw::Import
        ]
    );
}

#[test]
fn comments_and_hashbang() {
    assert_eq!(
        kinds(
            "#!/usr/bin/env node\n// c\n/* d */ x /** e\n */ y",
            Dialect::JS
        ),
        vec![K::Ident, K::Ident]
    );
    // BOM then hashbang.
    assert_eq!(
        kinds("\u{feff}#!/usr/bin/env node\nx", Dialect::JS),
        vec![K::Ident]
    );
    // `#!` later is not a hashbang.
    assert_eq!(kinds("x\n#!y", Dialect::JS)[0], K::Ident);
    // Comments ended by a lone CR.
    assert_eq!(kinds("// c\rx", Dialect::JS), vec![K::Ident]);
}

#[test]
fn triple_slash_only_before_code() {
    let src = "/// <reference path=\"a\" />\n// x\n/// <reference types=\"b\" />\nx\n/// \
               <reference path=\"c\" />";
    assert_eq!(
        kinds(src, Dialect::TS),
        vec![K::TripleSlash, K::TripleSlash, K::Ident]
    );
}

#[test]
fn strings_with_escapes() {
    assert_eq!(
        strings(r#"'a\'b' "c\"d" 'e\\' "f'g""#, Dialect::JS),
        vec![r"'a\'b'", r#""c\"d""#, r"'e\\'", r#""f'g""#]
    );
    // Line continuation.
    assert_eq!(strings("'a\\\nb' x", Dialect::JS), vec!["'a\\\nb'"]);
    assert_eq!(strings("'a\\\r\nb' x", Dialect::JS), vec!["'a\\\r\nb'"]);
}

#[test]
fn unterminated_string_recovers_at_newline_once() {
    let (t, mut lx) = lex_all("x = 'abc\ny = 'def\nimport 'z'", Dialect::JS);
    assert_eq!(t.last().unwrap(), &(K::Str, "'z'".into()));
    let e = lx.take_error().unwrap();
    assert_eq!(e.kind, UncertaintyKind::ParseIncomplete);
    assert_eq!(e.span, Span::new(4, 8));
    assert_eq!(lx.take_error(), None);
}

#[test]
fn template_literals() {
    assert_eq!(
        toks("`a${b}c${`d${e}`}f` g", Dialect::JS),
        vec![
            (K::TemplateHead, "`a${".into()),
            (K::Ident, "b".into()),
            (K::TemplateMiddle, "}c${".into()),
            (K::TemplateHead, "`d${".into()),
            (K::Ident, "e".into()),
            (K::TemplateTail, "}`".into()),
            (K::TemplateTail, "}f`".into()),
            (K::Ident, "g".into()),
        ]
    );
    // Objects and blocks inside substitutions, strings with backticks and braces.
    assert_eq!(
        of(
            "`${ {a: '}`'} }${ function() { return '`' } }` x",
            Dialect::JS,
            K::Ident
        ),
        vec!["a", "function", "return", "x"]
    );
    // Escapes, `$` without `{`, multi-line.
    assert_eq!(
        kinds("`\\` $ \\${a}\n` x", Dialect::JS),
        vec![K::Template, K::Ident]
    );
    no_error("`${`${`${a}`}`}`", Dialect::JS);
    // Regex after a substitution start.
    assert_eq!(regexes("`${/}/.source}`"), vec!["/}/"]);
}

#[test]
fn unterminated_template() {
    let (t, mut lx) = lex_all("x = `abc ${ y", Dialect::JS);
    assert_eq!(t.last().unwrap().0, K::Ident);
    assert!(lx.take_error().is_some());
    let (_, mut lx) = lex_all("x = `abc", Dialect::JS);
    assert!(lx.take_error().is_some());
}

#[test]
fn regex_vs_division_basic() {
    assert_eq!(regexes("a / b / c"), Vec::<String>::new());
    assert_eq!(regexes("x = /ab+c/gi.test(s)"), vec!["/ab+c/gi"]);
    assert_eq!(regexes("f(/a/, /b/)"), vec!["/a/", "/b/"]);
    assert_eq!(regexes("[/a/]"), vec!["/a/"]);
    assert_eq!(regexes("a[0] / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("(a) / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("1 / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("'a' / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("a++ / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("a-- / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("x = a ? /b/ : /c/"), vec!["/b/", "/c/"]);
    assert_eq!(regexes("!/a/.test(b)"), vec!["/a/"]);
    assert_eq!(regexes("a && /b/.test(c)"), vec!["/b/"]);
    assert_eq!(regexes("x = y /= 2"), Vec::<String>::new());
    assert_eq!(regexes("x = /=/"), vec!["/=/"]);
    assert_eq!(regexes("this / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("#priv / 2 / 3"), Vec::<String>::new());
}

#[test]
fn regex_after_keywords() {
    for kw in [
        "return",
        "typeof",
        "case",
        "yield",
        "await",
        "void",
        "delete",
        "instanceof",
        "in",
        "of",
        "new",
        "throw",
        "else",
        "do",
        "extends",
        "default",
    ] {
        assert_eq!(regexes(&format!("{kw} /a'/g")), vec!["/a'/g"], "after {kw}");
        // As property names they are identifiers.
        assert_eq!(
            regexes(&format!("x.{kw} / 2 / 3")),
            Vec::<String>::new(),
            "after .{kw}"
        );
    }
}

#[test]
fn regex_after_paren_and_brace() {
    assert_eq!(regexes("if (a) /b'/.test(c)"), vec!["/b'/"]);
    assert_eq!(regexes("while (a) /b/.exec(c)"), vec!["/b/"]);
    assert_eq!(regexes("for (;;) /b/.exec(c)"), vec!["/b/"]);
    assert_eq!(regexes("with (a) /b/.exec(c)"), vec!["/b/"]);
    assert_eq!(regexes("for await (x of y) /b/.exec(c)"), vec!["/b/"]);
    assert_eq!(regexes("if ((a)) /b/.exec(c)"), vec!["/b/"]);
    assert_eq!(regexes("f(a) / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("x.if(a) / 2 / 3"), Vec::<String>::new());
    // `}` closing a block vs an object literal / template substitution.
    assert_eq!(regexes("function f() {}\n/a'/.test(b)"), vec!["/a'/"]);
    assert_eq!(regexes("{}\n/a/.test(b)"), vec!["/a/"]);
    assert_eq!(regexes("if (x) {} /a/.test(b)"), vec!["/a/"]);
    assert_eq!(regexes("try {} finally {} /a/.test(b)"), vec!["/a/"]);
    assert_eq!(regexes("x = {} / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("x = {a: {}} / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("f({}) / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("return {} / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("x = a ? {} / 2 / 3 : b"), Vec::<String>::new());
    assert_eq!(
        regexes("switch (a) { case 1: {} /b/.test(c) }"),
        vec!["/b/"]
    );
    assert_eq!(regexes("label: {} /b/.test(c)"), vec!["/b/"]);
    assert_eq!(regexes("x = () => {}\n/b/.test(c)"), vec!["/b/"]);
    assert_eq!(regexes("`${a}` / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes("export default {} / 2 / 3"), Vec::<String>::new());
}

#[test]
fn regex_bodies() {
    // Classes may contain unescaped `/`; escapes; quotes and backticks are plain
    // bytes.
    assert_eq!(regexes("x = /[/]'\"`/ + 1"), vec!["/[/]'\"`/"]);
    assert_eq!(regexes(r"x = /a\/b/ + 1"), vec![r"/a\/b/"]);
    assert_eq!(regexes(r"x = /[\]/]/ + 1"), vec![r"/[\]/]/"]);
    assert_eq!(regexes("x = /a/dgimsuyv.x"), vec!["/a/dgimsuyv"]);
    // Comments are not regexes.
    assert_eq!(regexes("x = 1 // a / b\n/* / */ y"), Vec::<String>::new());
}

#[test]
fn unterminated_regex_recovers_at_newline() {
    let (t, mut lx) = lex_all("x = /abc\nimport 'a'", Dialect::JS);
    assert_eq!(t.last().unwrap(), &(K::Str, "'a'".into()));
    assert!(lx.take_error().is_some());
}

#[test]
fn ts_non_null_assertion() {
    assert_eq!(regexes_ts("a! / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes_ts("a()! / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes_ts("a[0]!! / 2 / 3"), Vec::<String>::new());
    assert_eq!(regexes_ts("x = !/a/.test(b)"), vec!["/a/"]);
    assert_eq!(regexes_ts("x = a !/b/"), vec!["/b/"]);
    assert_eq!(regexes_ts("return!/a/.test(b)"), vec!["/a/"]);
    // In JS `!` is always prefix.
    assert_eq!(regexes("a\n!/b/.test(c)"), vec!["/b/"]);
}

#[test]
fn ts_braces_after_types() {
    // Return type annotations: the body is a block, a regex may follow it.
    assert_eq!(
        regexes_ts("function f(): void {}\n/a/.test(b)"),
        vec!["/a/"]
    );
    assert_eq!(
        regexes_ts("function f(): Promise<T> {}\n/a/.test(b)"),
        vec!["/a/"]
    );
    assert_eq!(regexes_ts("function f(): T[] {}\n/a/.test(b)"), vec!["/a/"]);
    no_error(
        "function f<T extends {a: string}>(x: {b: T}): {c: T} { return {c: x.b} }",
        Dialect::TS,
    );
    // Optional parameters and conditional types keep `?`/`:` balanced.
    assert_eq!(
        regexes_ts("function f(a?: string, b?: {x: number}) { return a ? /x/ : {} }"),
        vec!["/x/"]
    );
}

#[test]
fn numbers() {
    assert_eq!(
        of(
            "1 1.5 .5 1e10 1E-5 1e+5 0x1F 0b10 0o17 1_000_000 10n 0xFFn 1..toString",
            Dialect::JS,
            K::Number
        ),
        vec![
            "1",
            "1.5",
            ".5",
            "1e10",
            "1E-5",
            "1e+5",
            "0x1F",
            "0b10",
            "0o17",
            "1_000_000",
            "10n",
            "0xFFn",
            "1."
        ]
    );
    // `0x1e-5` is hex `0x1e` minus 5.
    assert_eq!(of("0x1e-5", Dialect::JS, K::Number), vec!["0x1e", "5"]);
    assert_eq!(regexes("1 / 2 / 3"), Vec::<String>::new());
}

#[test]
fn identifiers_non_ascii() {
    assert_eq!(
        of("const café = ŝ; ünïcödé_$1 \\u0061b", Dialect::JS, K::Ident),
        vec!["const", "café", "ŝ", "ünïcödé_$1", "\\u0061b"]
    );
    // Non-ASCII whitespace separates tokens.
    assert_eq!(
        of(
            "import\u{a0}x\u{2028}from\u{feff}y\u{3000}z",
            Dialect::JS,
            K::Ident
        ),
        vec!["import", "x", "from", "y", "z"]
    );
    // Non-ASCII identifiers before `/` are values.
    assert_eq!(regexes("ü / 2 / 3"), Vec::<String>::new());
}

#[test]
fn private_names_and_decorators() {
    assert_eq!(
        kinds("class A { #x = 1; @dec m() { this.#x } }", Dialect::TS)
            .into_iter()
            .filter(|k| matches!(k, K::PrivateName | K::At))
            .count(),
        3
    );
}

#[test]
fn jsx_text_and_attributes_are_skipped() {
    let src = r#"x = <div a="it's" b='"' c={'str'} {...rest}>don't `stop` /* no */ {y}</div>; z"#;
    assert_eq!(
        toks(src, Dialect::JSX),
        vec![
            (K::Ident, "x".into()),
            (K::Assign, "=".into()),
            (K::LBrace, "{".into()),
            (K::Str, "'str'".into()),
            (K::RBrace, "}".into()),
            (K::LBrace, "{".into()),
            (K::Ellipsis, "...".into()),
            (K::Ident, "rest".into()),
            (K::RBrace, "}".into()),
            (K::LBrace, "{".into()),
            (K::Ident, "y".into()),
            (K::RBrace, "}".into()),
            (K::Jsx, "</div>".into()),
            (K::Semi, ";".into()),
            (K::Ident, "z".into()),
        ]
    );
    no_error(src, Dialect::JSX);
}

#[test]
fn jsx_nesting_fragments_and_self_closing() {
    let src = "f(<><A.B x:y='1' /><c-d>{cond && <e>{`t${<f/>}`}</e>}text</c-d></>, 2) / 2 / 3";
    let t = toks(src, Dialect::JSX);
    assert_eq!(of(src, Dialect::JSX, K::Regex), Vec::<String>::new());
    assert_eq!(
        t.iter().filter(|(k, _)| *k == K::Jsx).count(),
        3,
        "the outer fragment, <e> and <f/> (both inside containers): {t:?}"
    );
    no_error(src, Dialect::JSX);
    // Element as attribute value; comment in a tag.
    no_error("x = <a b=<c/> /* c */ // d\n d />", Dialect::JSX);
}

#[test]
fn jsx_regex_and_arrows_in_containers() {
    let src = "x = <a onClick={() => /'/.test(s)}>{a / b}</a>";
    assert_eq!(of(src, Dialect::JSX, K::Regex), vec!["/'/"]);
    no_error(src, Dialect::JSX);
}

#[test]
fn jsx_vs_less_than() {
    // Operators: no JSX.
    assert!(of("a < b; c << d; e <= f; x = y < z > w", Dialect::JSX, K::Jsx).is_empty());
    assert_eq!(of("a < b", Dialect::JSX, K::Lt), vec!["<"]);
    // `<` followed by a non-identifier in expression position is not JSX.
    assert!(of("x = < 1", Dialect::JSX, K::Jsx).is_empty());
    // After `return`, `(`, `=>`, `?`, `:`, `&&` JSX starts.
    for prefix in [
        "return ", "f(", "() => ", "a ? ", "a ? b : ", "a && ", "[", "x = ",
    ] {
        let src = format!("{prefix}<p>don't</p>");
        assert_eq!(of(&src, Dialect::JSX, K::Jsx).len(), 1, "{src}");
    }
}

#[test]
fn tsx_generics_vs_jsx() {
    for src in [
        "const f = <T,>(x: T) => x; y / 2 / 3",
        "const f = <T extends object>(x: T) => x; y / 2 / 3",
        "const f = <const T,>(x: T) => x",
        "const f = <T extends unknown,>(x: T) => x",
        // A default: `<T = D` is a type parameter list, as in TypeScript's own lookahead.
        "const f = <A = any,>(x: A) => x; y / 2 / 3",
        "const f = <A = any>(x: A) => x; y / 2 / 3",
        "const f = <const A = any,>(x: A) => x",
    ] {
        assert!(of(src, Dialect::TSX, K::Jsx).is_empty(), "{src}");
        assert!(of(src, Dialect::TSX, K::Regex).is_empty(), "{src}");
        no_error(src, Dialect::TSX);
    }
    // `extends` as a JSX attribute.
    for src in [
        "x = <T extends>it's</T>",
        "x = <T extends='a' />",
        "x = <T extends/>",
    ] {
        assert_eq!(of(src, Dialect::TSX, K::Jsx).len(), 1, "{src}");
    }
    // `<T>` in .tsx is JSX; in .ts it is a type assertion.
    assert_eq!(of("x = <T>y</T>", Dialect::TSX, K::Jsx).len(), 1);
    assert!(of("x = <T>y; z / 2 / 3", Dialect::TS, K::Jsx).is_empty());
    assert!(of("x = <T>y; z / 2 / 3", Dialect::TS, K::Regex).is_empty());
}

/// `<T>(...) => R` in a `.tsx` file is a generic function type, wherever it
/// stands: JSX text cannot contain a raw `>`.
#[test]
fn tsx_generic_function_types_are_not_jsx() {
    for src in [
        // An interface member, a type-literal member, an annotation, an alias, a union.
        "interface S { sync: <T>(refresh: () => Promise<T>) => Promise<T>; }\nconst c = make<S | \
         null>(null); y / 2 / 3",
        "type S = { run?: <T>(\n  fn: (signal?: Sig) => Promise<T>,\n  o?: O,\n) => \
         Promise<R<T>>; };\ny / 2 / 3",
        "const toValue: <T>(v: MV<T> | T) => T = (v) => v; y / 2 / 3",
        "type F = <T>(x: T) => T; y / 2 / 3",
        "type G = string | (<T>(x: T) => T); y / 2 / 3",
        "type H = <T> (opts: { a: T }) => void; y / 2 / 3",
    ] {
        assert!(of(src, Dialect::TSX, K::Jsx).is_empty(), "{src}");
        assert!(of(src, Dialect::TSX, K::Regex).is_empty(), "{src}");
        no_error(src, Dialect::TSX);
    }
    // Elements whose text starts with a parenthesis stay elements.
    for src in [
        "x = <b>(optional)</b>; y / 2",
        "x = <b>(don't {v})</b>; y / 2",
        "x = <p>(see <a href=\"u\">docs</a>)</p>; y / 2",
        "x = <b>({n}) {items.map((i) => i)}</b>; y / 2",
        "x = <T>(a) =&gt; b</T>; y / 2",
        "g = (x = <b>(</b>) => x; y / 2",
    ] {
        assert_eq!(of(src, Dialect::TSX, K::Jsx).len(), 1, "{src}");
        assert!(of(src, Dialect::TSX, K::Regex).is_empty(), "{src}");
        no_error(src, Dialect::TSX);
    }
}

/// Type arguments of a JSX element (`<Tag<A, B> ...>`, TypeScript only) produce
/// no tokens: the element lexes exactly like the same element without them.
#[test]
fn tsx_element_type_arguments() {
    for (generic, plain) in [
        (
            "x = <Field<TIn, K>\n {...(props as P<typeof Field<TIn, K>>[0])}\n />; y / 2",
            "x = <Field\n {...(props as P<typeof Field<TIn, K>>[0])}\n />; y / 2",
        ),
        (
            "x = <Show<TIn> {...props} />; y / 2",
            "x = <Show {...props} />; y / 2",
        ),
        (
            "x = <Select<Opt, true> a=\"1\">it's {v}</Select>; y / 2",
            "x = <Select a=\"1\">it's {v}</Select>; y / 2",
        ),
        (
            "x = <Filter<Filters['status']> name=\"s\" />; y / 2",
            "x = <Filter name=\"s\" />; y / 2",
        ),
        (
            "x = <A.B<(e: E<F>) => void> on={f} />; y / 2",
            "x = <A.B on={f} />; y / 2",
        ),
        (
            "x = <A<{ a: 'q>'; b: `t>${C}`; c: [D, E] }> />; y / 2",
            "x = <A />; y / 2",
        ),
        ("x = <A<B<C>>>{v}</A>; y / 2", "x = <A>{v}</A>; y / 2"),
        (
            "x = <A</* c> */ B, // d>\n C>>{v}</A>; y / 2",
            "x = <A>{v}</A>; y / 2",
        ),
        (
            "x = <Table<\n  Row,\n  Ctx\n> data={d} />; y / 2",
            "x = <Table data={d} />; y / 2",
        ),
        // In children, and as an attribute value.
        (
            "x = <div a=<I<T> /> ><Ctl<V, typeof k> name={k} /></div>; y / 2",
            "x = <div a=<I /> ><Ctl name={k} /></div>; y / 2",
        ),
        // The shape of a test file: the element is a call argument, its closer is followed
        // by `,` and `)`.
        (
            "f(\n  <Field<V> name=\"a\">\n    {(f) => <In field={f} />}\n  </Field>,\n)",
            "f(\n  <Field name=\"a\">\n    {(f) => <In field={f} />}\n  </Field>,\n)",
        ),
    ] {
        let strip = |src: &str| -> Vec<(TokKind, String)> {
            toks(src, Dialect::TSX)
                .into_iter()
                .filter(|(k, _)| *k != K::Jsx)
                .collect()
        };
        assert_eq!(strip(generic), strip(plain), "{generic}");
        assert_eq!(
            of(generic, Dialect::TSX, K::Jsx).len(),
            of(plain, Dialect::TSX, K::Jsx).len(),
            "{generic}"
        );
        assert!(of(generic, Dialect::TSX, K::Regex).is_empty(), "{generic}");
        no_error(generic, Dialect::TSX);
    }
    // Not TypeScript: a `<` after the tag name is not a type argument list.
    let (_, mut lx) = lex_all("x = <A<B> c />; y", Dialect::JSX);
    assert!(lx.take_error().is_some());
    // Unbalanced: still an error, and the scan is not repeated for later tags.
    let (_, mut lx) = lex_all("x = <A<B c />; y = <D<E c />; z", Dialect::TSX);
    assert!(lx.take_error().is_some());
}

/// The two TSX lookaheads stay linear on input built to repeat them: a type
/// argument scan that fails is not tried again, and the parameter-list
/// lookahead is bounded. A quadratic scan of these 2 MB inputs would run for
/// minutes.
#[test]
fn tsx_lookaheads_stay_linear() {
    for unit in [
        "x = <A<B ",
        "x = <A<B\n",
        "x = <a>(</a>, ",
        "x = <a>( ",
        "x = <a>((</a>, y(",
    ] {
        let src = unit.repeat(2_000_000 / unit.len());
        let start = std::time::Instant::now();
        let mut lx = Lexer::new(src.as_bytes(), Dialect::TSX);
        while lx.next_token().kind != K::Eof {}
        let mut lx = Lexer::new(src.as_bytes(), Dialect::TSX);
        while lx.next_matching(0).kind != K::Eof {}
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "{unit:?}: {:?}",
            start.elapsed()
        );
    }
}

#[test]
fn ts_never_lexes_jsx() {
    let src = "let a = <any>b; let c = f<string>('d')";
    assert!(of(src, Dialect::TS, K::Jsx).is_empty());
    assert_eq!(strings(src, Dialect::TS), vec!["'d'"]);
}

#[test]
fn js_jsx_heuristic() {
    // JSX in a .js file.
    let src = "export default () => <div className='x'>it's {name}</div>; y / 2";
    assert_eq!(of(src, Dialect::JS, K::Jsx).len(), 1);
    no_error(src, Dialect::JS);
    // Flow-style generic arrow: JSX never closes, the heuristic reports failure.
    let (_, lx) = lex_all("const f = <T>(x: T): T => x;\nimport 'a'", Dialect::JS);
    assert!(lx.jsx_failed());
    // With JSX off, it lexes as plain tokens.
    let off = Dialect {
        jsx: Jsx::Off,
        ..Dialect::JS
    };
    assert_eq!(
        strings("const f = <T>(x: T): T => x;\nimport 'a'", off),
        vec!["'a'"]
    );
}

#[test]
fn strict_jsx_unterminated_reports_error() {
    let (_, mut lx) = lex_all("x = <div>abc", Dialect::JSX);
    assert!(!lx.jsx_failed());
    assert!(lx.take_error().is_some());
}

#[test]
fn depth_and_try_flags() {
    let src = "a(b[c{d}]) try { e; function f() { g } } catch { h } i";
    let mut lx = Lexer::new(src.as_bytes(), Dialect::JS);
    let mut v = Vec::new();
    loop {
        let t = lx.next_token();
        if t.kind == K::Eof {
            break;
        }
        v.push((
            String::from_utf8_lossy(t.text(src.as_bytes())).into_owned(),
            t.depth,
            t.in_try(),
        ));
    }
    let get = |s: &str| v.iter().find(|(x, _, _)| x == s).cloned().unwrap();
    assert_eq!(get("a").1, 0);
    assert_eq!(get("(").1, 0);
    assert_eq!(get(")").1, 0);
    assert_eq!(get("d").1, 3);
    assert!(get("e").2 && get("g").2);
    assert!(!get("h").2 && !get("i").2 && !get("a").2);
}

#[test]
fn unbalanced_delimiters_recover() {
    for src in [
        "}}} import 'a'",
        ")) import 'a'",
        "f(a]; import 'a'",
        "{ ( } import 'a'",
        "((((((((",
        "`${",
        "`${}}}",
        "<div>{",
    ] {
        let mut lx = Lexer::new(src.as_bytes(), Dialect::JSX);
        for _ in 0..1000 {
            if lx.next_token().kind == K::Eof {
                break;
            }
        }
        assert!(lx.take_error().is_some(), "{src}");
    }
    assert_eq!(strings("}}} import 'a'", Dialect::JS), vec!["'a'"]);
    assert_eq!(strings("f(a]; import 'a'", Dialect::JS), vec!["'a'"]);
}

#[test]
fn eof_is_sticky() {
    let mut lx = Lexer::new(b"a", Dialect::JS);
    assert_eq!(lx.next_token().kind, K::Ident);
    for _ in 0..3 {
        assert_eq!(lx.next_token().kind, K::Eof);
    }
}

#[test]
fn dialect_from_path() {
    let d = |p: &str| Dialect::from_path(Path::new(p));
    assert_eq!(d("/a/b.js"), Dialect::JS);
    assert_eq!(d("/a/b.mjs"), Dialect::JS);
    assert_eq!(d("/a/b.cjs"), Dialect::JS);
    assert_eq!(d("/a/b.jsx"), Dialect::JSX);
    assert_eq!(d("/a/b.ts"), Dialect::TS);
    assert_eq!(d("/a/b.mts"), Dialect::TS);
    assert_eq!(d("/a/b.cts"), Dialect::TS);
    assert_eq!(d("/a/b.tsx"), Dialect::TSX);
    assert_eq!(d("/a/b.d.ts"), Dialect::DTS);
    assert_eq!(d("/a/b.d.mts"), Dialect::DTS);
    assert_eq!(d("/a/bin"), Dialect::JS);
    assert_eq!(d("/a/.ts"), Dialect::JS);
}

#[test]
fn arbitrary_bytes_terminate() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    for len in 0..300 {
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let table = b"/'\"`${}()[]<>=!?:;.\n\\ab0#*+-x";
                table[(seed % table.len() as u64) as usize]
            })
            .collect();
        for d in [Dialect::JS, Dialect::TS, Dialect::TSX, Dialect::JSX] {
            let mut lx = Lexer::new(&bytes, d);
            let mut n = 0;
            loop {
                let t = lx.next_token();
                assert!(t.start <= t.end && t.end as usize <= bytes.len());
                if t.kind == K::Eof {
                    break;
                }
                n += 1;
                assert!(
                    n <= bytes.len() + 1,
                    "no progress on {:?}",
                    bytes.escape_ascii()
                );
            }
        }
    }
}

#[test]
fn block_comment_edges() {
    assert_eq!(kinds("/**/x", Dialect::JS), vec![K::Ident]);
    assert_eq!(kinds("/***/x", Dialect::JS), vec![K::Ident]);
    assert_eq!(kinds("/*/ x */y", Dialect::JS), vec![K::Ident]);
    assert_eq!(kinds("/* ** * */y", Dialect::JS), vec![K::Ident]);
    assert_eq!(kinds("a/*#__PURE__*/(b)", Dialect::JS).len(), 4);
    let (t, mut lx) = lex_all("x /*/", Dialect::JS);
    assert_eq!(t.len(), 1);
    assert!(lx.take_error().is_some());
}

// ---- skim (`next_matching_top`) equivalence ----

/// The token predicate `next_matching_top(mask, top_mask)` must implement.
fn reference_match(lx: &Lexer<'_>, t: &Token, mask: u64, top_mask: u64) -> bool {
    match t.kind {
        TokKind::Ident => {
            let bit = 1u64 << t.kw as u8;
            (t.kw != Kw::None && mask & bit != 0)
                || (t.kw != Kw::None && top_mask & bit != 0 && t.depth == 0)
                || lx.is_dyn_trigger(t)
        }
        TokKind::Eof | TokKind::TripleSlash => true,
        _ => false,
    }
}

/// Interleaves `next_token` and `next_matching_top` (as the recognizer does,
/// choosing by `seed`) and checks every returned token against the full token
/// stream.
fn check_skim(src: &[u8], dialect: Dialect, seed: u64, dyn_words: &[&str]) {
    let mask = (1u64 << Kw::Import as u8)
        | (1u64 << Kw::Require as u8)
        | (1u64 << Kw::New as u8)
        | (1u64 << Kw::Type as u8);
    let top_mask = (1u64 << Kw::Const as u8) | (1u64 << Kw::Var as u8);
    let mut reference = Lexer::new(src, dialect);
    for w in dyn_words {
        reference.add_trigger_word(w.as_bytes());
    }
    let mut all = Vec::new();
    loop {
        let t = reference.next_token();
        all.push(t);
        if t.kind == TokKind::Eof {
            break;
        }
    }
    // The block skim (`lx`) and the per-token reference skim (`rf`) run in
    // lockstep.
    let mut lx = Lexer::new(src, dialect);
    lx.set_reference_skim(false);
    let mut rf = Lexer::new(src, dialect);
    rf.set_reference_skim(true);
    for w in dyn_words {
        lx.add_trigger_word(w.as_bytes());
        rf.add_trigger_word(w.as_bytes());
    }
    let mut state = seed | 1;
    let mut i = 0;
    loop {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let t = if state.is_multiple_of(4) {
            let t = lx.next_token();
            assert_eq!(t, all[i], "next_token at {i}: {}", src.escape_ascii());
            assert_eq!(rf.next_token(), t);
            i += 1;
            t
        } else {
            lx.reset_low_water();
            rf.reset_low_water();
            let t = lx.next_matching_top(mask, top_mask);
            while !reference_match(&reference, &all[i], mask, top_mask) {
                i += 1;
            }
            assert_eq!(t, all[i], "next_matching at {i}: {}", src.escape_ascii());
            assert_eq!(rf.next_matching_top(mask, top_mask), t);
            assert_eq!(
                lx.low_water(),
                rf.low_water(),
                "low water at {i}: {}",
                src.escape_ascii()
            );
            i += 1;
            t
        };
        if t.kind == TokKind::Eof {
            break;
        }
        // The recognizer registers dynamic words as it goes.
        if state & 0xff == 7 {
            lx.add_trigger_word(b"x1");
            rf.add_trigger_word(b"x1");
            reference.add_trigger_word(b"x1");
        }
    }
    assert_eq!(lx.jsx_failed(), reference.jsx_failed());
    assert_eq!(rf.jsx_failed(), reference.jsx_failed());
    let err = reference.take_error();
    assert_eq!(lx.take_error(), err, "error: {}", src.escape_ascii());
    assert_eq!(rf.take_error(), err);
}

fn fixture_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            fixture_files(&p, out);
        } else if p.extension().is_some_and(|x| {
            ["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"]
                .contains(&x.to_str().unwrap_or(""))
        }) {
            out.push(p);
        }
    }
}

#[test]
fn skim_matches_full_lexer_on_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/js");
    let mut files = Vec::new();
    fixture_files(&root, &mut files);
    assert!(!files.is_empty());
    for (n, f) in files.iter().enumerate() {
        let src = std::fs::read(f).unwrap();
        let d = Dialect::from_path(f);
        check_skim(&src, d, n as u64 + 1, &[]);
        check_skim(&src, d, n as u64 + 7, &["fs", "path", "r", "readFileSync"]);
    }
}

#[test]
fn skim_matches_full_lexer_on_generated_input() {
    const PIECES: &[&str] = &[
        "a", "fs", "r", "path", "x1", "import", "require", "new", "type", "const", "var", "return",
        "typeof", "case", "default", "if", "for", "await", "while", "else", "in", "(", ")", "[",
        "]", "{", "}", ".", "..", "...", ",", ";", ":", "?", "?.", "!", "=", "=>", "+", "++", "-",
        "*", "/", "/=", "<", ">", "<div>", "</div>", "&&", "||", "1", ".5", "'s'", "\"d\"", "`t`",
        "`a${", "}b`", "/re/g", "// c\n", "/* c */", "/*\n*/", "\n", " ", "\t", "\u{a0}", "é",
        "#p", "@d", "\\u0061", "'\\''",
    ];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for case in 0..3000 {
        let mut s = String::new();
        let n = 1 + (case % 60);
        for _ in 0..n {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            s.push_str(PIECES[(state % PIECES.len() as u64) as usize]);
            if state & 0x100 != 0 {
                s.push(' ');
            }
        }
        for d in [Dialect::JS, Dialect::TS, Dialect::TSX, Dialect::JSX] {
            check_skim(s.as_bytes(), d, case as u64 + 3, &["fs", "r"]);
        }
    }
}

// ---- block skim vs per-token skim: random, mutated and real inputs ----

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Fragments that exercise the skim's reconstruction of the previous token.
const TRICKY: &[&str] = &[
    "1.", "1.e", "1e+", "1e-", "0x1", ".5", "...", "..", "?.", "?.5", "#", "#a", "a.", "x. ", "\n",
    "\r\n", " ", "\t", "\u{2028}", "\u{a0}", "é", "π", "$", "_", "\\u0061", "if", "for", "await",
    "while", "with", "case", "default", "typeof", "return", "import", "require", "new", "export",
    "module", "type", "const", "let", "var", "eval", "fs", "r", "x1", "try", "void", "(", ")", "[",
    "]", "{", "}", "${", "`", "'", "\"", "/", "//", "/*", "*/", "/=", "!", "!=", "<", "<<", ">",
    "=>", "=", "==", "+", "++", "-", "--", "*", "**", "&&", "||", "??", "?", ":", ";", ",", "@",
    "~", "%", "^", "|", "&", "\0", "<div>", "</div>", "/>", "a", "b0", "9",
];

fn random_source(rng: &mut Rng, max_pieces: usize) -> Vec<u8> {
    let mut s = Vec::new();
    for _ in 0..1 + rng.below(max_pieces) {
        match rng.below(10) {
            0 => s.push(rng.next() as u8),
            1 => s.extend(std::iter::repeat_n(b' ', rng.below(70))),
            2 => s.extend(std::iter::repeat_n(b'a', 1 + rng.below(80))),
            _ => s.extend_from_slice(TRICKY[rng.below(TRICKY.len())].as_bytes()),
        }
        if rng.next() & 3 == 0 {
            s.push(b' ');
        }
    }
    s
}

/// Byte-level mutations of a real file: flips, insertions of tricky fragments,
/// deletions and duplications of ranges.
fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut s = src.to_vec();
    for _ in 0..1 + rng.below(8) {
        let at = rng.below(s.len() + 1);
        match rng.below(5) {
            0 if at < s.len() => s[at] = rng.next() as u8,
            1 => {
                let frag = TRICKY[rng.below(TRICKY.len())].as_bytes();
                s.splice(at..at, frag.iter().copied());
            }
            2 if at < s.len() => {
                let end = (at + 1 + rng.below(40)).min(s.len());
                s.drain(at..end);
            }
            3 if at < s.len() => {
                let end = (at + 1 + rng.below(200)).min(s.len());
                let dup = s[at..end].to_vec();
                s.splice(at..at, dup);
            }
            _ => s.insert(at.min(s.len()), b"(){}[]/'\"`?:"[rng.below(12)]),
        }
    }
    s
}

const DIALECTS: [Dialect; 5] = [
    Dialect::JS,
    Dialect::TS,
    Dialect::TSX,
    Dialect::JSX,
    Dialect::DTS,
];

/// `scan` with the block skim and with the per-token skim produce identical
/// extractions.
fn check_scan(src: &[u8], dialect: Dialect) {
    let fast = crate::scan(
        src,
        dialect,
        crate::ScanOptions {
            bindings: true,
            ..Default::default()
        },
    );
    REFERENCE_SKIM.with(|c| c.set(true));
    let slow = crate::scan(
        src,
        dialect,
        crate::ScanOptions {
            bindings: true,
            ..Default::default()
        },
    );
    REFERENCE_SKIM.with(|c| c.set(false));
    assert_eq!(fast, slow, "{dialect:?}: {}", src.escape_ascii());
}

#[test]
fn block_skim_matches_on_random_input() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for case in 0..6000 {
        let s = random_source(&mut rng, 1 + case % 120);
        for d in DIALECTS {
            check_skim(&s, d, case as u64 + 11, &["fs", "r", "x1"]);
        }
        if case % 4 == 0 {
            check_scan(&s, DIALECTS[case % DIALECTS.len()]);
        }
    }
}

#[test]
fn block_skim_matches_on_random_bytes() {
    let mut rng = Rng(0x94D0_49BB_1331_11EB);
    for len in 0..2000 {
        let s: Vec<u8> = (0..len % 300).map(|_| rng.next() as u8).collect();
        for d in DIALECTS {
            check_skim(&s, d, len as u64 + 5, &["fs"]);
        }
    }
}

#[test]
fn block_skim_matches_on_mutated_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/js");
    let mut files = Vec::new();
    fixture_files(&root, &mut files);
    files.sort();
    let mut rng = Rng(0xBF58_476D_1CE4_E5B9);
    for (n, f) in files.iter().enumerate() {
        let src = std::fs::read(f).unwrap();
        let d = Dialect::from_path(f);
        check_scan(&src, d);
        for k in 0..6 {
            let m = mutate(&mut rng, &src);
            check_skim(&m, d, (n * 8 + k) as u64 + 1, &["fs", "path", "r"]);
            check_scan(&m, DIALECTS[(n + k) % DIALECTS.len()]);
        }
    }
}

/// Real trees: `TURBO_TRACE_DEBUG_SKIM_DIR=<node_modules> cargo test --release
/// -p turborepo-trace-lang-js block_skim_matches_on_real_tree -- --ignored
/// --nocapture`. Every file is checked as is
/// and with `TURBO_TRACE_DEBUG_SKIM_MUTATIONS` (default 2) mutations.
#[test]
#[ignore = "needs TURBO_TRACE_DEBUG_SKIM_DIR"]
fn block_skim_matches_on_real_tree() {
    let Some(dir) = turborepo_trace_core::debug::var_os("SKIM_DIR") else {
        return;
    };
    let mutations: usize = turborepo_trace_core::debug::var("SKIM_MUTATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let mut files = Vec::new();
    fixture_files(std::path::Path::new(&dir), &mut files);
    files.sort();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let (mut bytes, mut n) = (0usize, 0usize);
    for (i, f) in files.iter().enumerate() {
        let Ok(src) = std::fs::read(f) else { continue };
        let d = Dialect::from_path(f);
        check_skim(&src, d, i as u64 + 1, &[]);
        check_skim(&src, d, i as u64 + 2, &["fs", "path", "e", "t", "r"]);
        check_scan(&src, d);
        for k in 0..mutations {
            let m = mutate(&mut rng, &src);
            check_skim(&m, d, (i * 16 + k) as u64 + 3, &["fs", "r"]);
            check_scan(&m, d);
        }
        bytes += src.len();
        n += 1;
    }
    eprintln!("block skim: {n} files, {bytes} bytes, {mutations} mutations each: identical");
}
