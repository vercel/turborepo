//! The plugin-testkit conformance suite for the stub language.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_testkit::{Config, Escape, Wrapper, conformance, stub::StubLanguage};

const CONFIG: Config = Config {
    samples: &[
        "dep \"./a\"",
        "optional build asset \"./data/\" + ?name + \".txt\"",
        "dir \"./assets\"",
        "lazy dep \"pkg\"",
    ],
    comments: &[
        Wrapper::new("# ", "\n"),
        Wrapper::new("   #", "\n"),
        Wrapper::new("#", "\r\n"),
    ],
    // Strings only occur as arguments. An `asset` string is itself a reference, so the empty
    // wrapper yields one reference; a statement inside the string must not add another.
    // `unknown` lines are skipped as a whole.
    strings: &[
        Wrapper::new("asset \"", "\"\n").escape(Escape::Backslash(b"\"")),
        Wrapper::new("unknown \"", "\"\n").escape(Escape::Backslash(b"\"")),
    ],
    corpus: &[
        include_bytes!("../../turborepo-trace-testkit/fixtures/stub/basic/main.stub"),
        include_bytes!("../../turborepo-trace-testkit/fixtures/stub/basic/second.stub"),
        include_bytes!("../../turborepo-trace-testkit/fixtures/stub/basic/lib/index.stub"),
        include_bytes!(
            "../../turborepo-trace-testkit/fixtures/stub/basic/stub_modules/pkg/index.stub"
        ),
    ],
    tokens: &[
        b"dep",
        b"asset",
        b"dir",
        b"manifest",
        b"lib",
        b"optional",
        b"cond",
        b"lazy",
        b"type",
        b"build",
        b" + ",
        b"?x",
        b"\\u00",
        b"\\u{",
        b"#!stub",
    ],
    ..Config::new("/testkit/sample.stub")
};

conformance!(StubLanguage, CONFIG);
