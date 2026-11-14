//! Reading what package selection needs from a Go file without extracting it:
//! the build constraints, the package clause and the import paths (`import "C"`
//! decides cgo files).

use crate::{
    constraint::{HeaderConstraints, header_constraints},
    lex::{Kind, Lexer, string_value},
};

/// The header of a Go source file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoHeader {
    /// Build constraints.
    pub constraints: HeaderConstraints,
    /// The package name, if the package clause was found.
    pub package: Option<String>,
    /// Import paths in order (including `"C"`).
    pub imports: Vec<String>,
    /// Whether the scan reached the first declaration after the imports.
    /// `false` means the input ended inside the import section; if it was a
    /// truncated prefix, rescan the whole file.
    pub complete: bool,
}

impl GoHeader {
    /// Whether the file uses cgo.
    pub fn imports_c(&self) -> bool {
        self.imports.iter().any(|i| i == "C")
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum State {
    Start,
    PkgName,
    Decls,
    Import,
    ImportName,
    Block,
    BlockName,
}

/// Scans the header of Go source `src`.
pub fn scan_go_header(src: &[u8]) -> GoHeader {
    let mut h = GoHeader {
        constraints: header_constraints(src),
        ..Default::default()
    };
    let mut st = State::Start;
    for t in Lexer::new(src) {
        let Ok(t) = t else {
            continue;
        };
        if t.is_comment() {
            continue;
        }
        let semi = t.is_punct(src, b';');
        st = match st {
            State::Start if t.is_ident(src, b"package") => State::PkgName,
            State::Start => {
                h.complete = true;
                return h;
            }
            State::PkgName => {
                if t.kind == Kind::Ident {
                    h.package = Some(String::from_utf8_lossy(t.text(src)).into_owned());
                }
                State::Decls
            }
            State::Decls if semi => State::Decls,
            State::Decls if t.is_ident(src, b"import") => State::Import,
            State::Decls => {
                h.complete = true;
                return h;
            }
            State::Import | State::ImportName if t.is_string() => {
                h.imports
                    .extend(string_value(src, &t).map(|s| s.into_owned()));
                State::Decls
            }
            State::Import if t.is_punct(src, b'(') => State::Block,
            State::Import if t.kind == Kind::Ident || t.is_punct(src, b'.') => State::ImportName,
            State::Block | State::BlockName if t.is_string() => {
                h.imports
                    .extend(string_value(src, &t).map(|s| s.into_owned()));
                State::Block
            }
            State::Block if t.is_punct(src, b')') => State::Decls,
            State::Block if semi => State::Block,
            State::Block if t.kind == Kind::Ident || t.is_punct(src, b'.') => State::BlockName,
            // Malformed import section: stop.
            _ => {
                h.complete = true;
                return h;
            }
        };
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_package_and_imports() {
        let src = b"// doc\n//go:build linux\n\npackage foo // c\n\nimport \"fmt\"\nimport (\n\tx \"os\"\n\t. \"strings\"\n\t_ \"embed\"\n)\n/*\n#include <a.h>\n*/\nimport \"C\"\n\nfunc main() { import_ := 1 }\n";
        let h = scan_go_header(src);
        assert_eq!(h.package.as_deref(), Some("foo"));
        assert_eq!(h.imports, ["fmt", "os", "strings", "embed", "C"]);
        assert!(h.imports_c());
        assert!(h.complete);
        assert!(h.constraints.is_constrained());
    }

    #[test]
    fn truncated_prefix_is_incomplete() {
        let h = scan_go_header(b"package a\nimport (\n\t\"fmt\"\n");
        assert!(!h.complete);
        assert_eq!(h.imports, ["fmt"]);
        let h = scan_go_header(b"package a\nvar x = 1\n");
        assert!(h.complete);
        assert!(h.imports.is_empty());
    }
}
