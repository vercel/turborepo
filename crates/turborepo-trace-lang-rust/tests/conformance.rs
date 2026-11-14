//! The plugin-testkit conformance suite for Rust. Every language
//! must pass it.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_lang_rust::RustLanguage;
use turborepo_trace_testkit::{Config, Escape, Wrapper, conformance};

/// Raw strings need no escaping as long as the content has no `"#`.
fn raw_hash(s: &str) -> String {
    s.replace("\"#", "\" #")
}

const CONFIG: Config = Config {
    samples: &[
        "mod a;",
        "#[path = \"p.rs\"] mod b;",
        "mod outer { mod inner; }",
        "const S: &str = include_str!(\"a.txt\");",
        "static B: &[u8] = include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/b.bin\"));",
        "include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));",
        "fn f() { let _ = std::fs::read_to_string(\"cfg.toml\"); }",
        "fn g() { let _ = File::open(\"x\"); }",
        "static D: Dir = include_dir!(\"assets\");",
        "#[derive(RustEmbed)] #[folder = \"public/\"] struct P;",
        "#[template(path = \"index.html\")] struct T;",
        "fn main() { println!(\"cargo:rerun-if-changed=build.rs\"); }",
        "static M: Migrator = sqlx::migrate!();",
    ],
    comments: &[
        Wrapper::new("// ", "\n"),
        Wrapper::new("/// ", "\n"),
        Wrapper::new("//! ", "\n"),
        Wrapper::new("/* ", " */"),
        Wrapper::new("/* /* nested */ ", " /* again */ */"),
        Wrapper::new("/** ", " */"),
    ],
    strings: &[
        Wrapper::new("const X: &str = \"", "\";\n").escape(Escape::Backslash(b"\"")),
        Wrapper::new("const X: &[u8] = b\"", "\";\n").escape(Escape::Backslash(b"\"")),
        Wrapper::new("const X: &CStr = c\"", "\";\n").escape(Escape::Backslash(b"\"")),
        Wrapper::new("const X: &str = r#\"", "\"#;\n").escape(Escape::Custom(raw_hash)),
        Wrapper::new("const X: &[u8] = br##\"", "\"##;\n"),
        Wrapper::new(
            "fn f<'a>(x: &'a str) -> char { let _ = '\"'; let s = \"",
            "\"; 'x' }\n",
        )
        .escape(Escape::Backslash(b"\"")),
    ],
    corpus: &[
        include_bytes!("../../turborepo-trace-testkit/fixtures/rust/basic/app/src/main.rs"),
        include_bytes!("../../turborepo-trace-testkit/fixtures/rust/basic/app/src/net/mod.rs"),
        include_bytes!("../../turborepo-trace-testkit/fixtures/rust/basic/app/build.rs"),
        include_bytes!("../../turborepo-trace-testkit/fixtures/rust/basic/util/src/lib.rs"),
        include_bytes!("../src/extract.rs"),
        include_bytes!("../src/lexer.rs"),
    ],
    tokens: &[
        b"mod",
        b"#[",
        b"#![",
        b"cfg(",
        b"path",
        b"=",
        b"include_str!",
        b"include!",
        b"concat!(",
        b"env!(",
        b"\"",
        b"r#\"",
        b"\"#",
        b"b'",
        b"'",
        b"'a",
        b"/*",
        b"*/",
        b"//",
        b"{",
        b"}",
        b"(",
        b")",
        b";",
        b"::",
        b"fs::read(",
        b"println!(",
    ],
    ..Config::new("/testkit/src/sample.rs")
};

conformance!(RustLanguage::new(), CONFIG);
