//! Extraction tests: every recognized construct, cfg handling, and the lexer
//! edge cases that must not produce references.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use turborepo_trace_core::{Base, ExtractCx, Language, PathInterner, RefSink, TraceOptions};
use turborepo_trace_lang_rust::RustLanguage;

fn options() -> TraceOptions {
    TraceOptions {
        target: Some("x86_64-unknown-linux-gnu".into()),
        ..Default::default()
    }
}

/// `kind base spec [flags] phase uncertainty` per reference.
fn refs_at(path: &str, src: &str) -> Vec<String> {
    let options = options();
    let interner = PathInterner::new();
    let p = Path::new(path);
    let cx = ExtractCx {
        file: interner.intern(p),
        path: p,
        options: &options,
    };
    let mut sink = RefSink::new();
    RustLanguage::new().extract(src.as_bytes(), &cx, &mut sink);
    for r in sink.refs() {
        assert!(r.span.end as usize <= src.len() && r.span.start <= r.span.end);
    }
    sink.refs()
        .iter()
        .map(|r| {
            let base = match &r.base {
                Base::FileDir => "file_dir".to_owned(),
                Base::Cwd => "cwd".to_owned(),
                Base::ProjectRoot => "project".to_owned(),
                Base::Dir(_) => "dir".to_owned(),
                Base::Language(l) => (*l).to_owned(),
            };
            let flags: Vec<_> = r.flags.names().collect();
            let u = r
                .uncertainty
                .as_ref()
                .map(|u| format!(" !{}", u.kind))
                .unwrap_or_default();
            format!(
                "{} {} {} [{}] {}{}",
                r.kind,
                base,
                r.spec,
                flags.join(","),
                r.phase.as_str(),
                u
            )
        })
        .collect()
}

fn refs(src: &str) -> Vec<String> {
    refs_at("/p/src/lib.rs", src)
}

/// An inner-attribute embed followed by an item whose outer `cfg` is false for
/// the target (cpufeatures' `lib.rs`): the embed belongs to the crate, not to
/// the item.
#[test]
fn inner_doc_embed_before_cfg_item() {
    assert_eq!(
        refs(
            "#![no_std]\n#![doc = include_str!(\"../README.md\")]\n#![doc(html_logo_url = \
             \"x\")]\n\n#[cfg(not(miri))]\n#[cfg(target_arch = \"aarch64\")]\n#[doc(hidden)]\npub \
             mod aarch64;\n"
        ),
        ["embed file_dir ../README.md [] build"]
    );
}

#[test]
fn module_declarations() {
    assert_eq!(
        refs("mod a; pub mod b; pub(crate) mod r#c; mod d { fn f() {} }"),
        [
            "mod rust_mod a [] build",
            "mod rust_mod b [] build",
            "mod rust_mod c [] build",
        ]
    );
}

#[test]
fn inline_modules_and_path_attributes() {
    let src = r#"
mod outer {
    mod inner;
    #[path = "p.rs"] mod q;
    pub mod deep { mod x; }
}
#[path = "zz"]
mod t { mod u; }
#[path = "../shared/s.rs"]
mod s;
"#;
    assert_eq!(
        refs(src),
        [
            "mod rust_mod outer::inner [] build",
            "mod rust_mod_path outer::p.rs [] build",
            "mod rust_mod outer/deep::x [] build",
            "mod rust_mod zz::u [] build",
            "mod rust_mod_path ../shared/s.rs [] build",
        ]
    );
}

#[test]
fn cfg_on_modules_and_blocks() {
    let src = r#"
#[cfg(windows)] mod w;
#[cfg(unix)] mod u;
#[cfg(test)] mod tests;
#[cfg(feature = "x")] mod f;
#[cfg(all(target_os = "linux", not(target_env = "musl")))] mod gnu;
#[cfg(windows)]
mod win { mod inner; }
#[cfg(windows)]
fn g() { let _ = include_str!("w.txt"); }
#[cfg(any(unix, feature = "y"))]
fn h() { let _ = include_str!("h.txt"); }
#[derive(Debug)]
#[cfg(target_os = "macos")]
struct S { a: u8 }
mod after;
"#;
    assert_eq!(
        refs(src),
        [
            "mod rust_mod u [] build",
            "mod rust_mod tests [conditional] build",
            "mod rust_mod f [conditional] build",
            "mod rust_mod gnu [] build",
            "embed file_dir h.txt [] build",
            "mod rust_mod after [] build",
        ]
    );
}

#[test]
fn cfg_attr_paths() {
    let src = r#"
#[cfg_attr(unix, path = "unix.rs")] mod sys;
#[cfg_attr(windows, path = "w.rs")] mod sys2;
#[cfg_attr(feature = "x", path = "x.rs")] mod sys3;
#[cfg_attr(test, cfg(windows))] mod only_non_test_or_windows;
"#;
    assert_eq!(
        refs(src),
        [
            "mod rust_mod_path unix.rs [] build",
            "mod rust_mod sys2 [] build",
            "mod rust_mod sys3 [optional,conditional] build",
            "mod rust_mod_path x.rs [optional,conditional] build",
            "mod rust_mod only_non_test_or_windows [conditional] build",
        ]
    );
}

#[test]
fn cfg_if_chains() {
    let src = r#"
cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod w;
    } else if #[cfg(unix)] {
        mod u;
    } else {
        mod other;
    }
}
cfg_if! {
    if #[cfg(libc_align)] {
        mod align;
    } else {
        mod no_align;
    }
}
cfg_if! {
    if #[cfg(target_os = "macos")] {
        mod mac;
    } else if #[cfg(feature = "x")] {
        mod x;
    } else {
        mod fallback;
    }
}
fn f() { if a { mod_like(); } else { mod after; } }
"#;
    assert_eq!(
        refs(src),
        [
            "mod rust_mod u [] build",
            "mod rust_mod align [conditional] build",
            "mod rust_mod no_align [conditional] build",
            "mod rust_mod x [conditional] build",
            "mod rust_mod fallback [conditional] build",
            "mod rust_mod after [] build",
        ]
    );
}

#[test]
fn inner_cfg_attribute_drops_the_file() {
    assert!(refs("#![cfg(windows)]\nmod a;\n").is_empty());
    assert_eq!(
        refs("#![cfg(test)]\nmod a;\n"),
        ["mod rust_mod a [conditional] build"]
    );
    assert_eq!(
        refs("mod m {\n#![cfg(windows)]\nmod a;\n}\nmod b;"),
        ["mod rust_mod b [] build"]
    );
}

#[test]
fn includes_and_anchors() {
    let src = r#"
const A: &str = include_str!("a.txt");
static B: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/b.bin"));
include!(concat!(env!("OUT_DIR"), "/gen.rs"));
const C: &str = include_str!(concat!("x/", env!("PROFILE"), ".txt"));
const D: &str = std::include_str!("../d.txt");
const E: &str = include_str!(concat!(file!(), ".snap"));
include!("parts/generated_by_hand.rs");
"#;
    assert_eq!(
        refs(src),
        [
            "embed file_dir a.txt [] build",
            "embed crate_root data/b.bin [] build",
            "embed out_dir $OUT_DIR/gen.rs [] build !generated_file",
            "embed file_dir x/*.txt [] build !env_dependent",
            "embed file_dir ../d.txt [] build",
            "embed file_dir lib.rs.snap [] build",
            "embed file_dir parts/generated_by_hand.rs [] build",
            "mod file_dir parts/generated_by_hand.rs [] build",
        ]
    );
}

#[test]
fn comments_strings_chars_and_lifetimes() {
    let src = r####"
// mod no1;
/* mod no2; /* nested mod no3; */ include_str!("no4") */
/// mod no5; include_str!("no6")
//! mod no7;
const S: &str = "mod no8; include_str!(\"no9\")";
const R: &str = r#"mod no10; "include_str!("no11")" "#;
const B: &[u8] = br##"mod no12;"#"##;
const CS: &core::ffi::CStr = c"mod no13;";
fn f<'a>(x: &'a str) -> char { '"' }
mod after1;
fn g() -> char { '\'' }
mod after2;
fn h<'b>() { 'outer: loop { break 'outer; } }
const Q: char = '{';
mod after3;
"####;
    assert_eq!(
        refs(src),
        [
            "mod rust_mod after1 [] build",
            "mod rust_mod after2 [] build",
            "mod rust_mod after3 [] build",
        ]
    );
}

#[test]
fn runtime_reads() {
    let src = r#"
use std::fs::read_to_string as rts;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
const CFG: &str = "conf/app.toml";
fn main() {
    let _ = std::fs::read_to_string("cfg.toml");
    let _ = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("x.json"));
    let _ = File::open(format!("{}/y", env!("CARGO_MANIFEST_DIR")));
    let _ = rts("z");
    let _ = fs::read_dir("assets");
    let _ = fs::read(CFG);
    let _ = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("a").join("b.txt"));
    let _ = fs::metadata("maybe");
    let _ = tokio::fs::read(name);
    let _ = reader.read(&mut buf);
}
fn read(x: u8) {}
"#;
    assert_eq!(
        refs(src),
        [
            "fs_read cwd cfg.toml [] runtime !cwd_dependent",
            "fs_read crate_root x.json [] runtime",
            "fs_read crate_root y [] runtime",
            "fs_read cwd z [] runtime !cwd_dependent",
            "fs_read_dir cwd assets [] runtime !cwd_dependent",
            "fs_read cwd conf/app.toml [] runtime !cwd_dependent",
            "fs_read crate_root a/b.txt [] runtime",
            "fs_read cwd maybe [optional] runtime !cwd_dependent",
            "fs_read cwd <unknown> [] runtime !computed_specifier",
        ]
    );
}

#[test]
fn tests_and_build_scripts_run_in_the_crate_root() {
    assert_eq!(
        refs_at(
            "/p/tests/it.rs",
            r#"fn t() { std::fs::read("fixtures/a.json"); }"#
        ),
        ["fs_read crate_root fixtures/a.json [] runtime"]
    );
    assert_eq!(
        refs(r#"#[test] fn t() { std::fs::read("f.txt"); } fn u() { std::fs::read("g.txt"); }"#),
        [
            "fs_read crate_root f.txt [] runtime",
            "fs_read cwd g.txt [] runtime !cwd_dependent",
        ]
    );
    assert_eq!(
        refs(
            r#"#[cfg(test)] mod tests { #[tokio::test] async fn t() { std::fs::read("f.txt"); } }"#
        ),
        ["fs_read crate_root f.txt [conditional] runtime"]
    );
}

#[test]
fn build_script_inputs() {
    let src = r#"
fn main() {
    println!("cargo:rerun-if-changed=src/x.c");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rustc-link-lib=z\ncargo:rerun-if-changed=proto/");
    cc::Build::new().file("src/a.c").include("inc").compile("a");
    let s = std::fs::read_to_string("version.txt");
}
"#;
    assert_eq!(
        refs_at("/p/build.rs", src),
        [
            "fs_read crate_root src/x.c [] build",
            "fs_read crate_root build.rs [] build",
            "fs_read crate_root <unknown> [] build !computed_specifier",
            "shared_lib file_dir <unknown> [] build !env_dependent",
            "fs_read crate_root proto/ [] build",
            "fs_read crate_root src/a.c [] build",
            "fs_read_dir crate_root inc [] build",
            "fs_read crate_root version.txt [] runtime",
        ]
    );
    // `.file()` outside build scripts is not cc.
    assert!(refs(r#"fn f() { b.file("x"); }"#).is_empty());
}

#[test]
fn macro_and_attribute_table() {
    let src = r#"
static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");
static M: Migrator = sqlx::migrate!();
static M2: Migrator = sqlx::migrate!("db/migrations");
#[derive(RustEmbed)]
#[folder = "public/"]
struct Public;
#[derive(Template)]
#[template(path = "index.html", escape = "none")]
struct Index;
#[cfg(windows)]
#[derive(RustEmbed)]
#[folder = "win/"]
struct Win;
"#;
    assert_eq!(
        refs(src),
        [
            "embed crate_root assets [] build",
            "embed crate_root migrations [] build",
            "embed crate_root db/migrations [] build",
            "embed crate_root public/ [] build",
            "embed askama_templates index.html [] build",
        ]
    );
}

#[test]
fn doc_includes_in_attributes() {
    let src = r#"
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, doc = include_str!("../docs/extra.md"))]
#![cfg_attr(windows, doc = include_str!("../docs/win.md"))]
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo.md"))]
pub struct Demo;
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
#[doc = include_str!("../WIN.md")]
#[cfg(windows)]
pub struct WinDoc;
#[doc = concat!("```\n", include_str!("../examples/readme.rs"), "```\n")]
pub trait Traits {}
"#;
    assert_eq!(
        refs(src),
        [
            "embed file_dir ../README.md [] build",
            "embed file_dir ../docs/extra.md [conditional] build",
            "embed crate_root examples/demo.md [] build",
            "embed file_dir ../README.md [conditional] build",
            "embed file_dir ../examples/readme.rs [] build",
        ]
    );
}

#[test]
fn insta_snapshots() {
    let src = r#"
#[cfg(test)]
mod tests {
    #[test] fn a() { insta::assert_snapshot!(x); }
    #[test] fn b() { insta::assert_debug_snapshot!(y); assert_json_snapshot!(z); }
}
"#;
    assert_eq!(
        refs_at("/p/src/lib.rs", src),
        ["fs_read file_dir snapshots/*.snap [conditional] runtime"]
    );
}

#[test]
fn macro_rules_bodies_are_uncertain() {
    assert_eq!(
        refs("macro_rules! m { ($p:expr) => { include_str!($p) }; }"),
        ["embed file_dir <unknown> [] build !computed_specifier"]
    );
}

#[test]
fn prefilter_skips_files_without_triggers() {
    assert!(refs("fn main() { println!(\"hi\"); }").is_empty());
}

#[test]
fn unterminated_input_reports_parse_incomplete() {
    let options = options();
    let interner = PathInterner::new();
    let p = Path::new("/p/src/lib.rs");
    let cx = ExtractCx {
        file: interner.intern(p),
        path: p,
        options: &options,
    };
    let mut sink = RefSink::new();
    RustLanguage::new().extract(b"mod a; /* mod b;", &cx, &mut sink);
    assert_eq!(sink.refs().len(), 1);
    assert_eq!(sink.diagnostics().len(), 1);
}
