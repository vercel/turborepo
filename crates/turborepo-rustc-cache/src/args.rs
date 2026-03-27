use std::path::PathBuf;

/// Crate types that produce cacheable artifacts.
/// Binary, cdylib, and proc-macro targets invoke the system linker,
/// which makes them non-deterministic across machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrateType {
    Lib,
    Rlib,
    Staticlib,
    Dylib,
    Cdylib,
    ProcMacro,
    Bin,
}

impl CrateType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "lib" => Some(Self::Lib),
            "rlib" => Some(Self::Rlib),
            "staticlib" => Some(Self::Staticlib),
            "dylib" => Some(Self::Dylib),
            "cdylib" => Some(Self::Cdylib),
            "proc-macro" => Some(Self::ProcMacro),
            "bin" => Some(Self::Bin),
            _ => None,
        }
    }

    pub fn is_cacheable(&self) -> bool {
        matches!(self, Self::Lib | Self::Rlib | Self::Staticlib)
    }
}

/// An extern crate dependency: `--extern name=path`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternCrate {
    pub name: String,
    pub path: Option<PathBuf>,
}

/// Parsed rustc invocation arguments relevant to cache key computation.
#[derive(Debug)]
pub struct ParsedArgs {
    pub rustc_path: PathBuf,
    pub crate_name: Option<String>,
    pub crate_types: Vec<CrateType>,
    pub out_dir: Option<PathBuf>,
    pub emit: Vec<String>,
    pub externs: Vec<ExternCrate>,
    pub source_file: Option<PathBuf>,
    /// Flags that affect compilation output, sorted for deterministic hashing.
    /// Excludes --out-dir, --extern, -L, --check-cfg, --error-format,
    /// --json, --diagnostic-width (output-formatting-only flags).
    pub hash_relevant_args: Vec<String>,
    /// Extra search paths (-L)
    pub search_paths: Vec<PathBuf>,
}

/// RUSTC_WRAPPER is invoked as: `<wrapper> <rustc> <args...>`
/// The first real argument is the path to rustc itself.
pub fn parse_rustc_args(args: &[String]) -> Option<ParsedArgs> {
    if args.len() < 2 {
        return None;
    }

    let rustc_path = PathBuf::from(&args[1]);
    let rustc_args = &args[2..];

    let mut crate_name = None;
    let mut crate_types = Vec::new();
    let mut out_dir = None;
    let mut emit = Vec::new();
    let mut externs = Vec::new();
    let mut source_file = None;
    let mut search_paths = Vec::new();
    let mut hash_relevant_args = Vec::new();

    let mut i = 0;
    while i < rustc_args.len() {
        let arg = &rustc_args[i];

        if arg == "--crate-name" {
            if let Some(val) = rustc_args.get(i + 1) {
                crate_name = Some(val.clone());
                hash_relevant_args.push(arg.clone());
                hash_relevant_args.push(val.clone());
                i += 2;
                continue;
            }
        } else if arg == "--crate-type" {
            if let Some(val) = rustc_args.get(i + 1) {
                if let Some(ct) = CrateType::from_str(val) {
                    crate_types.push(ct);
                }
                hash_relevant_args.push(arg.clone());
                hash_relevant_args.push(val.clone());
                i += 2;
                continue;
            }
        } else if arg == "--out-dir" {
            if let Some(val) = rustc_args.get(i + 1) {
                out_dir = Some(PathBuf::from(val));
                // Excluded from hash — location doesn't affect the artifact content
                i += 2;
                continue;
            }
        } else if arg == "--emit" {
            if let Some(val) = rustc_args.get(i + 1) {
                emit.extend(val.split(',').map(|s| s.to_string()));
                hash_relevant_args.push(arg.clone());
                hash_relevant_args.push(val.clone());
                i += 2;
                continue;
            }
        } else if let Some(val) = arg.strip_prefix("--emit=") {
            emit.extend(val.split(',').map(|s| s.to_string()));
            hash_relevant_args.push(format!("--emit={val}"));
            i += 1;
            continue;
        } else if arg == "--extern" {
            if let Some(val) = rustc_args.get(i + 1) {
                externs.push(parse_extern(val));
                // Extern names go into the hash, but paths are hashed by content
                i += 2;
                continue;
            }
        } else if arg.starts_with("-L") {
            let path = if arg == "-L" {
                if let Some(val) = rustc_args.get(i + 1) {
                    i += 2;
                    PathBuf::from(val)
                } else {
                    i += 1;
                    continue;
                }
            } else {
                i += 1;
                PathBuf::from(arg.strip_prefix("-L").unwrap())
            };
            search_paths.push(path);
            // Excluded from hash — search paths don't affect output content
            continue;
        } else if arg == "--check-cfg"
            || arg == "--error-format"
            || arg == "--json"
            || arg == "--diagnostic-width"
            || arg == "--color"
        {
            // Output-formatting-only flags, skip arg and its value
            i += 2;
            continue;
        } else if arg.starts_with("--check-cfg=")
            || arg.starts_with("--error-format=")
            || arg.starts_with("--json=")
            || arg.starts_with("--diagnostic-width=")
            || arg.starts_with("--color=")
        {
            i += 1;
            continue;
        } else if !arg.starts_with('-') && source_file.is_none() {
            // Positional argument: the source file
            source_file = Some(PathBuf::from(arg));
            // Source file path itself isn't in the hash; we hash its content
            i += 1;
            continue;
        } else {
            // All other flags go into the hash
            hash_relevant_args.push(arg.clone());
        }

        i += 1;
    }

    hash_relevant_args.sort();

    Some(ParsedArgs {
        rustc_path,
        crate_name,
        crate_types,
        out_dir,
        emit,
        externs,
        source_file,
        hash_relevant_args,
        search_paths,
    })
}

fn parse_extern(val: &str) -> ExternCrate {
    // Format: name=path or just name
    if let Some((name, path)) = val.split_once('=') {
        ExternCrate {
            name: name.to_string(),
            path: Some(PathBuf::from(path)),
        }
    } else {
        ExternCrate {
            name: val.to_string(),
            path: None,
        }
    }
}

impl ParsedArgs {
    pub fn is_cacheable(&self) -> bool {
        // Must have at least one crate type, and all must be cacheable
        if self.crate_types.is_empty() {
            return false;
        }
        self.crate_types.iter().all(|ct| ct.is_cacheable())
    }

    /// The emit types that produce output files we should cache.
    /// Filters out dep-info which is always regenerated.
    pub fn cacheable_emit_types(&self) -> Vec<&str> {
        self.emit
            .iter()
            .filter_map(|e| {
                // Strip file path suffixes like "link=foo"
                let base = e.split('=').next().unwrap_or(e);
                match base {
                    "dep-info" => None, // Always regenerated, not worth caching
                    _ => Some(base),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_lib_invocation() {
        let args: Vec<String> = [
            "turbo-rustc-cache",
            "/usr/bin/rustc",
            "--crate-name",
            "math_core",
            "--crate-type",
            "lib",
            "--emit=dep-info,metadata,link",
            "--out-dir",
            "/repo/target/debug/deps",
            "--extern",
            "utils=/repo/target/debug/deps/libutils-abc123.rlib",
            "-L",
            "dependency=/repo/target/debug/deps",
            "src/lib.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let parsed = parse_rustc_args(&args).unwrap();
        assert_eq!(parsed.crate_name.as_deref(), Some("math_core"));
        assert_eq!(parsed.crate_types, vec![CrateType::Lib]);
        assert!(parsed.is_cacheable());
        assert_eq!(
            parsed.out_dir,
            Some(PathBuf::from("/repo/target/debug/deps"))
        );
        assert_eq!(parsed.source_file, Some(PathBuf::from("src/lib.rs")));
        assert_eq!(parsed.externs.len(), 1);
        assert_eq!(parsed.externs[0].name, "utils");
        assert_eq!(parsed.emit, vec!["dep-info", "metadata", "link"]);
    }

    #[test]
    fn test_binary_not_cacheable() {
        let args: Vec<String> = [
            "turbo-rustc-cache",
            "/usr/bin/rustc",
            "--crate-name",
            "cli",
            "--crate-type",
            "bin",
            "--emit=dep-info,link",
            "src/main.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let parsed = parse_rustc_args(&args).unwrap();
        assert!(!parsed.is_cacheable());
    }

    #[test]
    fn test_proc_macro_not_cacheable() {
        let args: Vec<String> = [
            "turbo-rustc-cache",
            "/usr/bin/rustc",
            "--crate-name",
            "my_derive",
            "--crate-type",
            "proc-macro",
            "--emit=dep-info,link",
            "src/lib.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let parsed = parse_rustc_args(&args).unwrap();
        assert!(!parsed.is_cacheable());
    }

    #[test]
    fn test_hash_relevant_args_sorted() {
        let args: Vec<String> = [
            "turbo-rustc-cache",
            "/usr/bin/rustc",
            "--crate-name",
            "foo",
            "--crate-type",
            "lib",
            "--edition=2021",
            "-C",
            "opt-level=2",
            "--emit=link",
            "src/lib.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let parsed = parse_rustc_args(&args).unwrap();
        // Verify sorted
        let mut sorted = parsed.hash_relevant_args.clone();
        sorted.sort();
        assert_eq!(parsed.hash_relevant_args, sorted);
    }

    #[test]
    fn test_output_formatting_flags_excluded() {
        let args: Vec<String> = [
            "turbo-rustc-cache",
            "/usr/bin/rustc",
            "--crate-name",
            "foo",
            "--crate-type",
            "lib",
            "--error-format",
            "json",
            "--json",
            "diagnostic-rendered-ansi",
            "--diagnostic-width=120",
            "--color=always",
            "--emit=link",
            "src/lib.rs",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let parsed = parse_rustc_args(&args).unwrap();
        // None of the formatting flags should be in hash_relevant_args
        for arg in &parsed.hash_relevant_args {
            assert!(!arg.contains("error-format"));
            assert!(!arg.contains("diagnostic-width"));
            assert!(!arg.starts_with("--json"));
            assert!(!arg.starts_with("--color"));
        }
    }
}
