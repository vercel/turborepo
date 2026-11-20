#![allow(clippy::unwrap_used)]

use super::*;

fn read(files: &[(&'static str, &str)]) -> Result<Option<GoRequirements>, GoDiscoveryError> {
    GoRequirements::read_with(|file, limit| {
        assert_eq!(limit, MAX_MANIFEST_BYTES);
        Ok(files
            .iter()
            .find(|(name, _)| *name == file)
            .map(|(_, text)| text.as_bytes().to_vec()))
    })
}
fn required(files: &[(&'static str, &str)]) -> GoRequirements {
    read(files).unwrap().unwrap()
}
fn v(value: &str) -> GoVersion {
    GoVersion::parse(value).unwrap()
}
fn source(file: &'static str, field: &'static str, request: &str) -> GoSource {
    GoSource {
        file,
        field,
        request: Some(request.into()),
    }
}
// Newest release 1.23.4, previous language 1.22.12, unreleased 1.24rc1.
fn published() -> Vec<GoVersion> {
    [
        "1.21.13", "1.22.0", "1.22.3", "1.22.12", "1.23.0", "1.23.4", "1.24rc1",
    ]
    .map(v)
    .to_vec()
}

#[test]
fn versions_follow_go_ordering_and_canonical_spelling() {
    let ordered = [
        "1.18beta1",
        "1.18rc1",
        "1.18",
        "1.18.1",
        "1.20rc1",
        "1.20",
        "1.20.1",
        "1.21",
        "1.21rc1",
        "1.21.0",
        "1.21.9",
        "1.22",
    ];
    for pair in ordered.windows(2) {
        assert!(v(pair[0]) < v(pair[1]), "{pair:?}");
    }
    for raw in ordered {
        assert_eq!(v(raw).to_string(), raw);
    }
    for invalid in [
        "", "1.", "1.20.0", "1.022", "2.0", "1.22.x", "1.22rc0", "1.22-pre", "go1.22",
    ] {
        assert_eq!(GoVersion::parse(invalid), None, "{invalid}");
    }
    assert!(!v("1.22").is_toolchain() && v("1.20").is_toolchain() && v("1.22rc1").is_toolchain());
}

#[test]
fn go_1_22_is_a_minimum_resolved_by_the_go_switch_rule() {
    let minimum = required(&[(GO_WORK, "go 1.22\n\nuse ./member\n")]);
    assert_eq!(minimum.sources(), [source(GO_WORK, "go", "1.22")]);
    // The oldest supported candidate satisfying the minimum, not 1.22.0.
    assert_eq!(minimum.select(&published()).unwrap(), v("1.22.12"));
    for (request, expected) in [
        ("1.21", "1.22.12"),
        ("1.22.12", "1.22.12"),
        ("1.23.1", "1.23.4"),
        ("1.24", "1.24rc1"),
    ] {
        let module = format!("module example.com/m\ngo {request}\n");
        let selected = required(&[(GO_MOD, &module)]).select(&published());
        assert_eq!(selected.unwrap(), v(expected), "{request}");
    }
    let too_new = required(&[(GO_MOD, "module m\ngo 1.25\n")]).select(&published());
    assert_eq!(
        too_new.unwrap_err().to_string(),
        "no published Go release satisfies go.mod go 1.25"
    );
    // Unchanged minimums accept any locked toolchain that satisfies them.
    assert!(minimum.matches_locked(v("1.23.4")) && minimum.matches_locked(v("1.22.0")));
    assert!(!minimum.matches_locked(v("1.21.13")) && !minimum.matches_locked(v("1.22")));
    // Go's implicit go.work minimum applies when the line is absent.
    let implicit = required(&[(GO_WORK, "use .\n")]);
    assert_eq!(implicit.sources()[0].request, None);
    assert_eq!(implicit.select(&published()).unwrap(), v("1.22.12"));
}

#[test]
fn exact_toolchain_is_the_preferred_selection() {
    let exact = required(&[(GO_WORK, "go 1.22 // minimum\ntoolchain go1.22.3\n")]);
    assert_eq!(
        exact.sources(),
        [
            source(GO_WORK, "go", "1.22"),
            source(GO_WORK, "toolchain", "go1.22.3")
        ]
    );
    assert_eq!(exact.select(&published()).unwrap(), v("1.22.3"));
    assert!(exact.matches_locked(v("1.22.3")) && !exact.matches_locked(v("1.22.12")));
    // An unpublished exact toolchain never falls back to another release.
    let missing = required(&[(GO_WORK, "go 1.22\ntoolchain go1.22.4\n")]);
    assert!(matches!(
        missing.select(&published()),
        Err(GoDiscoveryError::NoMatchingRelease(_))
    ));
    // `toolchain default` states no preference beyond the minimum.
    let default = required(&[(GO_MOD, "module m\ngo 1.23.0\ntoolchain default\n")]);
    assert_eq!(default.select(&published()).unwrap(), v("1.23.4"));
}

#[test]
fn go_work_governs_and_conflicts_name_files_and_fixes() {
    // A root module outside the workspace is ignored, as by the go command.
    let module = "module m\ngo 1.23\ntoolchain go1.23.4\n";
    let unused = required(&[(GO_WORK, "go 1.22\nuse ./member\n"), (GO_MOD, module)]);
    assert_eq!(unused.sources(), [source(GO_WORK, "go", "1.22")]);
    assert_eq!(unused.select(&published()).unwrap(), v("1.22.12"));
    // A used root module is recorded as a minimum; its toolchain is ignored.
    let used = required(&[
        (GO_WORK, "go 1.23.0\nuse (\n\t.\n\t./member\n)\n"),
        (GO_MOD, "module m\ngo 1.22\ntoolchain go1.22.3\n"),
    ]);
    assert_eq!(
        used.sources(),
        [
            source(GO_WORK, "go", "1.23.0"),
            source(GO_MOD, "go", "1.22")
        ]
    );
    assert_eq!(used.select(&published()).unwrap(), v("1.23.4"));
    let newer_module = read(&[(GO_WORK, "go 1.22\nuse .\n"), (GO_MOD, module)]);
    assert_eq!(
        newer_module.unwrap_err().to_string(),
        "conflicting Go declarations: root go.mod requires go >= 1.23, but go.work lists go 1.22; \
         run `go work use` to raise go.work"
    );
    let older_toolchain = read(&[(GO_MOD, "module m\ngo 1.23\ntoolchain go1.22.3\n")]);
    assert_eq!(
        older_toolchain.unwrap_err().to_string(),
        "conflicting Go declarations: go.mod toolchain go1.22.3 is older than its go 1.23 \
         minimum; run `go mod edit -toolchain=<go1.23 or newer>`"
    );
}

#[test]
fn unsupported_and_malformed_directives_are_actionable() {
    for (text, expected) in [
        (
            "go 1.22\ntoolchain go1.22.3-custom\n",
            "unsupported `toolchain go1.22.3-custom` in root go.work: turbo setup provisions only \
             official Go toolchains, not custom goV-suffix builds; run `go work edit \
             -toolchain=go1.N.P` with an official release",
        ),
        (
            "go 1.22\ntoolchain go1.22\n",
            "not an official toolchain name",
        ),
        ("go 1.22\ntoolchain local\n", "expected a toolchain name"),
        (
            "go >=1.22\n",
            "unsupported `go >=1.22` in root go.work: expected a Go version such as 1.22 or \
             1.22.3; run `go work edit -go=<version>`",
        ),
        (
            "go 1.22\ngo 1.23\n",
            "root go.work is malformed at line 2: repeated directive; repair it with `go work \
             edit`",
        ),
        ("go 1.22 1.23\n", "line 1: expected exactly one value"),
        ("go (\n1.22\n)\n", "line 1: unexpected directive block"),
        (
            "go 1.22\nuse (\n./a\n",
            "line 3: unterminated directive block",
        ),
        ("go \"1.22\n", "line 1: unterminated string"),
    ] {
        let error = read(&[(GO_WORK, text)]).unwrap_err().to_string();
        assert!(error.contains(expected), "{text:?}: {error}");
    }
    // Comments, quoting, and unrelated directives do not change declarations.
    let text = "// go 1.99\ngo \"1.22\" // note\nreplace example.com/x => \"./x//y\"\ngodebug \
                default=go1.21\n";
    assert_eq!(
        required(&[(GO_WORK, text)]).sources(),
        [source(GO_WORK, "go", "1.22")]
    );
    assert_eq!(read(&[]).unwrap(), None);
    let unreadable = GoRequirements::read_with(|_, _| Err(io::Error::other("denied")));
    assert_eq!(unreadable, Err(GoDiscoveryError::Read(GO_WORK)));
    let binary = GoRequirements::read_with(|file, _| Ok((file == GO_MOD).then(|| vec![0xff])));
    assert!(matches!(
        binary,
        Err(GoDiscoveryError::Malformed { file: GO_MOD, .. })
    ));
}
