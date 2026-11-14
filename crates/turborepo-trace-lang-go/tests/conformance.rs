//! The plugin-testkit conformance suite for Go. Every language must
//! pass it.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_lang_go::GoLanguage;
use turborepo_trace_testkit::{Config, Escape, Wrapper, conformance};

// Samples are single lines so they can sit inside a `//` comment; the cgo
// preamble (which needs a line comment directly before `import "C"`) is covered
// by the extractor's unit tests.
const CONFIG: Config = Config {
    samples: &[
        "import \"fmt\"",
        "import x \"example.com/x\"",
        "import (_ \"embed\"; . \"strings\")",
        "//go:embed static/* `b.txt`",
        "import \"os\"; func f() { os.ReadFile(\"a.txt\") }",
        "import \"html/template\"; var t = template.Must(template.ParseFiles(\"a.html\", \
         \"b.html\"))",
        "import \"net/http\"; var h = http.FileServer(http.Dir(\"public\"))",
    ],
    comments: &[Wrapper::new("// ", "\n"), Wrapper::new("/* ", " */")],
    strings: &[
        Wrapper::new("x = \"", "\"\n").escape(Escape::Custom(go_interpreted)),
        Wrapper::new("x = `", "`\n").escape(Escape::Custom(go_raw)),
        Wrapper::new("x = '", "'\n").escape(Escape::Custom(go_rune)),
    ],
    corpus: &[
        include_bytes!("../../turborepo-trace-testkit/fixtures/go/basic/main.go"),
        include_bytes!("../../turborepo-trace-testkit/fixtures/go/basic/internal/cgo/cgo.go"),
        include_bytes!(
            "../../turborepo-trace-testkit/fixtures/go/basic/internal/util/export_test.go"
        ),
        include_bytes!("../../turborepo-trace-testkit/fixtures/go/basic/internal/util/tagged.go"),
        include_bytes!("../../turborepo-trace-testkit/fixtures/go/work/shared/pkg/pkg.go"),
    ],
    tokens: &[
        b"import",
        b"package",
        b"//go:embed ",
        b"//go:build ",
        b"\"C\"",
        b"#include",
        b"os.ReadFile",
        b"template.ParseFiles",
        b"(",
        b")",
        b"\"",
        b"`",
        b"'",
        b"//",
        b"/*",
        b"*/",
        b"\n",
    ],
    ..Config::new("/testkit/sample.go")
};

/// An interpreted string: escape `\`, `"` and newlines (which end the literal).
fn go_interpreted(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// A raw string cannot contain a backtick: split it out with concatenation
/// outside the literal is not possible inside one literal, so replace it by a
/// quote (the sample stays a reference statement for the check's purpose: it
/// must not produce references).
fn go_raw(s: &str) -> String {
    s.replace('`', "'")
}

/// A rune literal holds one character; longer content is invalid Go but must
/// still lex as one (unterminated-free) literal without references, so escape
/// quotes and newlines.
fn go_rune(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
}

conformance!(GoLanguage::default(), CONFIG);
