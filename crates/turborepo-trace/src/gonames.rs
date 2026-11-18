//! Go files in the names layer ([`crate::names`]): what the query side knows
//! about a Go file from its path. The names themselves are a fact the Go plugin
//! records when the file is traced (`turborepo_trace_lang_go::names::names`:
//! the strings of the file by the predicate every language shares,
//! `runtime.Caller` as its location, the working directory for a test file and
//! for `os.Getwd`).

/// How far above its directory the location of a Go file with `ups` parent
/// steps reaches. `runtime.Caller` gives the file, not its directory: the first
/// parent step (`filepath.Dir(file)`, or the first `..` joined to it) only
/// removes the file name.
pub(crate) fn location_ups(ups: u32) -> u32 {
    ups.saturating_sub(1)
}

/// The directory `go test` runs the test file `path` in: its package directory.
pub(crate) fn test_cwd(path: &str) -> Option<&str> {
    path.ends_with("_test.go")
        .then(|| path.rsplit_once('/').map_or("", |x| x.0))
}
