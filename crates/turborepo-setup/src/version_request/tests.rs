use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn npm_reference_matrix() -> TestResult {
    let mut lines = include_str!("../../tests/version-requests.tsv").lines();
    assert_eq!(
        lines.next(),
        Some("# npm semver 7.5.2; default strict options")
    );
    let header = lines.next().ok_or("missing releases")?;
    let releases: Vec<Version> = header
        .strip_prefix("# releases\t")
        .ok_or("missing releases prefix")?
        .split(' ')
        .map(Version::parse)
        .collect::<Result<_, _>>()?;
    for line in lines {
        let (input, expected) = line.split_once('\t').ok_or("missing result")?;
        if expected == "invalid" {
            assert!(VersionRequest::parse(input).is_err(), "{input:?}");
            continue;
        }
        let request = VersionRequest::parse(input)?;
        assert_eq!(expected.len(), releases.len(), "{input:?}");
        for (release, result) in releases.iter().zip(expected.bytes()) {
            assert_eq!(
                request.matches(release),
                result == b'1',
                "{input:?}: {release}"
            );
        }
    }
    Ok(())
}

#[test]
fn every_fragment_is_validated_even_in_dead_or_universal_branches() {
    for fragment in [
        "garbage",
        "@20",
        "!=1.2.3",
        "==1.2.3",
        "=>1.2.3",
        "^",
        ">=",
        "-",
        "1..2",
        "1.2.",
        "v",
        "1.2.3-",
        "1.2.3-01",
        "1.2.3+a..b",
        "1.2.3+a/b",
        "1.2.3.4",
        "1.2.3,",
        "1.2.3\\evil",
        "1.2.3\0",
        "1.2.3#evil",
        "∞",
        "||",
        // npm accepts some of these by ignoring numeric components or suffixes.
        // Reject them rather than erasing authored information at the boundary.
        "1.x.3",
        "x.1",
        "*.2.3",
        "1.2.x-alpha",
        "1.x.x+build",
        "1.2-alpha",
        "1+build",
        "vv1.2.3",
        "^=1.2.3junk",
        "~=1.2.3junk",
        "~>=>1.2.3",
        "~ >>1.2.3",
        "^= >=1.2.3",
        "~> = 1.2.3",
        "latest",
        "node",
        "lts/*",
    ] {
        for input in [
            fragment.to_string(),
            format!(">=1.0.0 {fragment}"),
            format!("* || {fragment}"),
            format!(">2 <1 {fragment} || *"),
            format!("{fragment} || 1.2.3"),
        ] {
            assert!(VersionRequest::parse(&input).is_err(), "{input:?}");
        }
    }
    for input in [
        "",
        " \n\t ",
        "|| 1",
        "1 ||",
        "1 || || 2",
        "1 | 2",
        "1 && 2",
        "1 - 2 <3",
    ] {
        assert!(VersionRequest::parse(input).is_err(), "{input:?}");
    }
}

#[test]
fn exact_release_hook_is_canonical_and_not_used_for_ranges() -> TestResult {
    let exact = VersionRequest::parse(" = v1.2.3+one ")?;
    let same = Version::parse("1.2.3+one")?;
    let other_build = Version::parse("1.2.3+two")?;
    assert!(exact.is_exact());
    assert_eq!(exact.exact_version(), Some(&same));
    assert!(exact.matches(&other_build));
    assert!(exact.matches_with_exact(&same, |a, b| a == b));
    assert!(!exact.matches_with_exact(&other_build, |a, b| a == b));
    assert!(!exact.matches_with_exact(&Version::parse("2.0.0")?, |_, _| true));
    for input in ["^1.2.3", "1.2.3 *", "1.2.3 || 2.0.0", "1.2.3 1.2.3"] {
        let request = VersionRequest::parse(input)?;
        assert!(!request.is_exact());
        assert!(request.matches_with_exact(&same, |_, _| panic!("range called exact hook")));
    }
    Ok(())
}

#[test]
fn invalid_candidates_never_match_or_reach_the_hook() -> TestResult {
    for separator in ["-", "+"] {
        let release = Version::parse(&format!("1.2.3{separator}{}", "b".repeat(251)))?;
        for input in ["*", "1.2.3", ">=1.2.3-a"] {
            let request = VersionRequest::parse(input)?;
            assert!(!request.matches(&release));
            assert!(!request.matches_with_exact(&release, |_, _| panic!("invalid candidate")));
        }
    }
    let unsafe_release = Version::new(MAX_COMPONENT + 1, 0, 0);
    assert!(!VersionRequest::parse("*")?.matches_with_exact(&unsafe_release, |_, _| true));
    Ok(())
}

#[test]
fn contradictions_never_become_weaker_constraints() -> TestResult {
    for input in [
        ">2 <1",
        "1.x 2.x",
        "1.2.3 2.3.4",
        "2 - 1",
        ">1 <=1",
        ">*",
        "<*",
    ] {
        let request = VersionRequest::parse(input)?;
        for major in 0..4 {
            for minor in 0..4 {
                for patch in 0..5 {
                    let release = Version::new(major, minor, patch);
                    assert!(!request.matches(&release), "{input}: {release}");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn prerelease_opt_in_is_per_branch_not_per_term_or_request() -> TestResult {
    let release = Version::parse("1.2.3-beta")?;
    assert!(VersionRequest::parse("\u{000b}>=\t1.2.3-alpha\r\n<\u{000c}2 ")?.matches(&release));
    assert!(VersionRequest::parse(">=1.2.3-alpha <2")?.matches(&release));
    assert!(!VersionRequest::parse(">=1.2.3-alpha <1.2.3-beta")?.matches(&release));
    assert!(!VersionRequest::parse(">=1.2.3-alpha <1 || >=1 <2")?.matches(&release));
    assert!(!VersionRequest::parse("^1.2.3-alpha")?.matches(&Version::parse("1.2.4-alpha")?));
    // Numeric prerelease ordering is exact, not lossy JavaScript Number ordering.
    assert!(
        !VersionRequest::parse("=1.2.3-9007199254740992")?
            .matches(&Version::parse("1.2.3-9007199254740993")?)
    );
    Ok(())
}

#[test]
fn stable_zero_lower_bounds_are_not_dropped_after_prerelease_opt_in() -> TestResult {
    // npm removes >=0.0.0 as a stable-only tautology, which weakens these
    // intersections once the upper comparator opts into 0.0.0 prereleases.
    for lower in [">=0.0.0", "^0.0.0+meta", "~0.0.0", "0", "0.0", "0.x"] {
        for upper in ["<=0.0.0-alpha", "<0.0.0-alpha.1+meta"] {
            let request = VersionRequest::parse(&format!("{lower} {upper}"))?;
            for version in ["0.0.0-0", "0.0.0-alpha", "0.0.0", "0.0.1"] {
                assert!(
                    !request.matches(&Version::parse(version)?),
                    "{lower} {upper}: {version}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn constraints_obey_boolean_algebra() -> TestResult {
    let terms = "* >* 1 1.2.3 ^0.0.1 ~1.2.3 >=1.2.3-alpha <2";
    for a in terms.split_whitespace() {
        for b in terms.split_whitespace() {
            let a_request = VersionRequest::parse(a)?;
            let b_request = VersionRequest::parse(b)?;
            let and = VersionRequest::parse(&format!("{a} {b}"))?;
            let or = VersionRequest::parse(&format!("{a} || {b}"))?;
            for version in [
                "0.0.0",
                "0.0.1",
                "1.2.3-alpha",
                "1.2.3",
                "1.2.4-beta",
                "2.0.0",
            ] {
                let release = Version::parse(version)?;
                assert_eq!(
                    or.matches(&release),
                    a_request.matches(&release) || b_request.matches(&release),
                    "{a} || {b}: {release}"
                );
                // Prerelease opt-in belongs to the whole conjunction, not each
                // separate request; stable releases have ordinary AND semantics.
                if release.pre.is_empty() {
                    assert_eq!(
                        and.matches(&release),
                        a_request.matches(&release) && b_request.matches(&release),
                        "{a} {b}: {release}"
                    );
                }
            }
        }
    }
    // Unlike npm's wildcard-union shortcut, retain the explicit prerelease arm.
    assert!(VersionRequest::parse("* || >=1.2.3-alpha")?.matches(&Version::parse("1.2.3-beta")?));
    let bounded = VersionRequest::parse(&vec!["^1"; MAX_BRANCH_TERMS].join(" "))?;
    assert_eq!(bounded.branches[0].len(), 2 * MAX_BRANCH_TERMS);
    Ok(())
}

#[test]
fn parser_limits_are_inclusive_and_overflow_never_wraps() -> TestResult {
    let at_byte_limit = format!("{}*", " ".repeat(MAX_REQUEST_BYTES - 1));
    assert!(VersionRequest::parse(&at_byte_limit).is_ok());
    assert_eq!(
        VersionRequest::parse(&format!(" {at_byte_limit}")).err(),
        Some(VersionRequestError::TooComplex)
    );
    for (count, valid) in [
        (MAX_REQUEST_BRANCHES, true),
        (MAX_REQUEST_BRANCHES + 1, false),
    ] {
        let input = vec!["*"; count].join(" || ");
        assert_eq!(VersionRequest::parse(&input).is_ok(), valid);
    }
    for (count, valid) in [(MAX_BRANCH_TERMS, true), (MAX_BRANCH_TERMS + 1, false)] {
        let input = vec![">= 1"; count].join(" ");
        assert_eq!(VersionRequest::parse(&input).is_ok(), valid);
    }
    for (length, valid) in [(MAX_VERSION_BYTES, true), (MAX_VERSION_BYTES + 1, false)] {
        let input = format!("1.2.3+{}", "a".repeat(length - 6));
        assert_eq!(VersionRequest::parse(&input).is_ok(), valid);
    }
    assert!(VersionRequest::parse("9007199254740991.0.0").is_ok());
    for input in [
        "9007199254740992.0.0",
        "18446744073709551616",
        "^9007199254740991",
        "~1.9007199254740991",
        "^0.0.9007199254740991",
    ] {
        assert_eq!(
            VersionRequest::parse(input).err(),
            Some(VersionRequestError::ComponentOverflow),
            "{input}"
        );
    }
    let full = vec![vec!["^1"; MAX_BRANCH_TERMS].join(" "); MAX_REQUEST_BRANCHES].join("||");
    // Bytes bound the combined work even when each branch/term count is allowed.
    assert_eq!(
        VersionRequest::parse(&full).err(),
        Some(VersionRequestError::TooComplex)
    );
    Ok(())
}
