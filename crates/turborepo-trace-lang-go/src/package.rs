//! The files of a package directory for a build context (`go/build`'s
//! `Import`): which `.go` files (and `_test.go` files) build, plus the
//! package's cgo, assembly and object files.
//!
//! Per file: names starting with `.` or `_` are ignored; the `_GOOS`/`_GOARCH`
//! name rule and the header's build constraint must hold; Go files that `import
//! "C"` need cgo. With [`BuildContext::all_targets`], files failing these
//! checks are kept as [`PackageFiles::conditional`] instead of dropped.

use std::path::{Path, PathBuf};

use turborepo_trace_core::{
    DirCache,
    fs::{EntryKind, FileKind},
    os::OsStrExt,
};

use crate::{
    constraint::{BuildContext, header_constraints},
    header::{GoHeader, scan_go_header},
};

/// Extensions of non-Go files that belong to a package (`go list`'s `CFiles`,
/// `CXXFiles`, `HFiles`, `SFiles`, `MFiles`, `FFiles`, `SwigFiles`,
/// `SwigCXXFiles`, `SysoFiles`).
pub const OTHER_EXTS: &[&str] = &[
    "c", "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx", "s", "S", "sx", "m", "f", "F", "for", "f90",
    "swig", "swigcxx", "syso",
];

/// How many bytes of a file are read for its header first.
const HEAD_LEN: usize = 16 * 1024;

/// What kind of package file.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileClass {
    /// A non-test `.go` file.
    Go,
    /// A `_test.go` file (of the package or its external `_test` package).
    Test,
    /// A cgo/assembly/object file.
    Other,
}

/// One file excluded for the build context but kept by
/// [`BuildContext::all_targets`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionalFile {
    /// The file.
    pub path: PathBuf,
    /// Its class.
    pub class: FileClass,
    /// Why it does not build for the configured target.
    pub why: String,
}

/// The files of one package directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageFiles {
    /// Non-test Go files that build, sorted.
    pub go: Vec<PathBuf>,
    /// `_test.go` files that build (both the package and its external test
    /// package).
    pub tests: Vec<PathBuf>,
    /// cgo, assembly and object files that build.
    pub other: Vec<PathBuf>,
    /// Files that only build for other targets ([`BuildContext::all_targets`]
    /// only).
    pub conditional: Vec<ConditionalFile>,
    /// Files whose contents decided their exclusion (constraint or cgo). Their
    /// contents are resolution inputs.
    pub read_excluded: Vec<PathBuf>,
    /// Whether the directory has any `.go` file at all (before filtering).
    pub any_go: bool,
    /// Imports of the selected `_test.go` files (deduplicated, sorted).
    pub test_imports: Vec<String>,
    /// Package names of the selected files.
    pub package_names: Vec<String>,
}

impl PackageFiles {
    /// Whether no non-test Go file builds.
    pub fn is_empty(&self) -> bool {
        self.go.is_empty()
    }
}

fn read_head(path: &Path) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; HEAD_LEN];
    let mut n = 0;
    while n < buf.len() {
        let r = f.read(&mut buf[n..])?;
        if r == 0 {
            buf.truncate(n);
            return Ok((buf, true));
        }
        n += r;
    }
    Ok((buf, false))
}

/// Reads a Go file's header, rereading the whole file if the import section is
/// longer than the first chunk.
pub fn read_go_header(path: &Path) -> Option<GoHeader> {
    let (head, whole) = read_head(path).ok()?;
    let h = scan_go_header(&head);
    if h.complete || whole {
        return Some(h);
    }
    let all = std::fs::read(path).ok()?;
    Some(scan_go_header(&all))
}

/// Lists the package in `dir` for `cx`. Uses the dir cache for the listing
/// (callers record the membership themselves) and plain reads for file headers.
pub fn package_files(fs: &DirCache, dir: &Path, cx: &BuildContext) -> PackageFiles {
    let mut out = PackageFiles::default();
    let Some(listing) = turborepo_trace_core::record::pause_listings(|| fs.list(dir)) else {
        return out;
    };
    for e in listing.iter() {
        let name_b = e.name_bytes();
        if name_b.first().is_some_and(|b| *b == b'.' || *b == b'_') {
            continue;
        }
        let Ok(name) = std::str::from_utf8(name_b) else {
            continue;
        };
        let path = dir.join(std::ffi::OsStr::from_bytes(name_b));
        let regular = match e.kind() {
            EntryKind::File => true,
            EntryKind::Symlink => matches!(fs.stat(&path).map(|s| s.kind), Some(FileKind::File)),
            _ => false,
        };
        if !regular {
            continue;
        }
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let class = if ext == "go" {
            out.any_go = true;
            if name.ends_with("_test.go") {
                FileClass::Test
            } else {
                FileClass::Go
            }
        } else if OTHER_EXTS.contains(&ext) {
            FileClass::Other
        } else {
            continue;
        };
        let mut why: Option<String> = None;
        if !cx.file_name_ok(name) {
            why = Some(format!(
                "file name {name} is for another target than {}/{}",
                cx.platform.go_os(),
                cx.platform.go_arch()
            ));
        }
        let mut content_decided = false;
        let mut header: Option<GoHeader> = None;
        if why.is_none() {
            match class {
                FileClass::Go | FileClass::Test => {
                    header = read_go_header(&path);
                    if let Some(h) = &header {
                        if h.constraints.is_constrained() && !h.constraints.matches(cx) {
                            why = Some(format!("//go:build {}", h.constraints.display()));
                            content_decided = true;
                        } else if h.imports_c() && !cx.cgo {
                            why = Some("uses cgo (import \"C\") and cgo is disabled".into());
                            content_decided = true;
                        }
                    }
                }
                FileClass::Other if ext != "syso" => {
                    if let Ok((head, _)) = read_head(&path) {
                        let hc = header_constraints(&head);
                        if hc.is_constrained() && !hc.matches(cx) {
                            why = Some(format!("//go:build {}", hc.display()));
                            content_decided = true;
                        }
                    }
                }
                FileClass::Other => {}
            }
        }
        match why {
            None => {
                if let Some(h) = &header {
                    if let Some(p) = &h.package
                        && !out.package_names.contains(p)
                    {
                        out.package_names.push(p.clone());
                    }
                    if class == FileClass::Test {
                        out.test_imports.extend(h.imports.iter().cloned());
                    }
                }
                match class {
                    FileClass::Go => out.go.push(path),
                    FileClass::Test => out.tests.push(path),
                    FileClass::Other => out.other.push(path),
                }
            }
            Some(why) => {
                if content_decided {
                    out.read_excluded.push(path.clone());
                }
                if cx.all_targets {
                    out.conditional.push(ConditionalFile { path, class, why });
                }
            }
        }
    }
    out.test_imports.sort();
    out.test_imports.dedup();
    out.package_names.sort();
    out
}

#[cfg(test)]
mod tests {
    use turborepo_trace_core::Platform;

    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        for (f, c) in files {
            let p = t.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        t
    }

    fn names(v: &[PathBuf]) -> Vec<String> {
        v.iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn selects_files_for_target() {
        let t = tree(&[
            ("a.go", "package p\n"),
            ("a_linux.go", "package p\n"),
            ("a_windows.go", "package p\n"),
            ("b.go", "//go:build ignore\n\npackage main\n"),
            ("c.go", "//go:build linux && amd64\n\npackage p\n"),
            ("cgo.go", "package p\n// #include <x.h>\nimport \"C\"\n"),
            ("a_test.go", "package p\nimport \"testing\"\n"),
            (
                "x_test.go",
                "package p_test\nimport (\n\t\"testing\"\n\t\"example.com/p\"\n)\n",
            ),
            ("asm_amd64.s", "#include \"textflag.h\"\n"),
            ("asm_arm64.s", "\n"),
            ("gen.s", "//go:build arm64\n\nTEXT x\n"),
            ("x.c", "int x;\n"),
            ("x.h", "\n"),
            ("_skip.go", "package p\n"),
            (".hidden.go", "package p\n"),
            ("README.md", "x"),
            ("sub/d.go", "package sub\n"),
        ]);
        let fs = DirCache::new();
        let cx = BuildContext::new(Platform::parse("linux/amd64").unwrap());
        let p = package_files(&fs, t.path(), &cx);
        assert_eq!(names(&p.go), ["a.go", "a_linux.go", "c.go", "cgo.go"]);
        assert_eq!(names(&p.tests), ["a_test.go", "x_test.go"]);
        assert_eq!(names(&p.other), ["asm_amd64.s", "x.c", "x.h"]);
        assert_eq!(names(&p.read_excluded), ["b.go", "gen.s"]);
        assert_eq!(p.test_imports, ["example.com/p", "testing"]);
        assert_eq!(p.package_names, ["p", "p_test"]);
        assert!(p.conditional.is_empty());

        let mut nocgo = cx.clone();
        nocgo.cgo = false;
        let p = package_files(&fs, t.path(), &nocgo);
        assert_eq!(names(&p.go), ["a.go", "a_linux.go", "c.go"]);

        let mut all = BuildContext::new(Platform::parse("darwin/arm64").unwrap());
        all.all_targets = true;
        let p = package_files(&fs, t.path(), &all);
        assert_eq!(names(&p.go), ["a.go", "cgo.go"]);
        let cond: Vec<_> = p
            .conditional
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            cond,
            ["a_linux.go", "a_windows.go", "asm_amd64.s", "b.go", "c.go"]
        );
    }
}
