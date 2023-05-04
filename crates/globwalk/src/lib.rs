use std::path::Path;

use glob_match::glob_match;
use itertools::{
    FoldWhile::{Continue, Done},
    Itertools,
};
use turbopath::AbsoluteSystemPathBuf;

pub enum WalkType {
    Files,
    Folders,
    All,
}

#[derive(Debug, PartialEq)]
pub enum MatchType {
    Match,
    PotentialMatch,
    None,
}

impl WalkType {
    fn should_emit(&self, is_dir: bool) -> bool {
        match self {
            WalkType::Files => !is_dir,
            WalkType::Folders => is_dir,
            WalkType::All => true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalkError {
    #[error("walkdir error: {0}")]
    WalkDir(walkdir::Error),
    #[error("bad pattern: {0}")]
    BadPattern(String),
}

/// Performs a glob walk, yielding paths that
/// _are_ included in the include list (if it is nonempty)
/// and _not_ included in the exclude list.
///
/// In the case of an empty include, then all
/// files are included.
pub fn globwalk<'a>(
    base_path: &'a AbsoluteSystemPathBuf,
    include: &'a [String],
    exclude: &'a [String],
    walk_type: WalkType,
) -> impl Iterator<Item = Result<AbsoluteSystemPathBuf, WalkError>> + 'a {
    let walker = walkdir::WalkDir::new(base_path.as_path()).follow_links(true);
    let mut iter = walker.into_iter();

    std::iter::from_fn(move || loop {
        let entry = match iter.next()?.map_err(WalkError::WalkDir) {
            Ok(entry) => entry,
            Err(err) => return Some(Err(err)),
        };

        let path = entry.path();
        let relative_path = path.strip_prefix(&base_path).expect("it is a subdir");
        let is_directory = path.is_dir();

        let include = match do_match(relative_path, include, exclude, is_directory) {
            Ok(include) => include,
            Err(glob) => return Some(Err(WalkError::BadPattern(glob.to_owned()))),
        };

        match include {
            MatchType::None if is_directory => {
                iter.skip_current_dir();
            }
            MatchType::Match if walk_type.should_emit(is_directory) => {
                return Some(Ok(AbsoluteSystemPathBuf::new(path).expect("absolute")));
            }
            // if it is not a much but not a directory, or a match but doesn't
            // match the walk type, then we just move to the next file.
            MatchType::None | MatchType::PotentialMatch | MatchType::Match => {}
        }
    })
}

/// Checks if a path is a partial match for a glob, meaning that a
/// subfolder could match.
fn potential_match(glob: &str, path: &str) -> Option<MatchType> {
    potential_match_inner(glob, path, true)
}

fn potential_match_inner(glob: &str, path: &str, top_level: bool) -> Option<MatchType> {
    let matches = glob_match(glob, path)?;

    // the emptry string is always a potential match
    if path == "" {
        return Some(MatchType::PotentialMatch);
    }

    if !matches {
        // pop last chunk from glob and try again.
        // if no more chunks, then there is no match.
        glob.rsplit_once('/')
            .map_or(Some(MatchType::None), |(prefix_glob, _)| {
                potential_match_inner(prefix_glob, path, false)
            })
    } else {
        if top_level {
            Some(MatchType::Match)
        } else {
            Some(MatchType::PotentialMatch)
        }
    }
}

/// Executes a match against a relative path using the given include and exclude
/// globs. If the path is not valid unicode, then it is automatically not
/// matched.
///
/// If an evaluated glob is invalid, then this function returns an error with
/// the glob that failed.
fn do_match<'a>(
    path: &Path,
    include: &'a [String],
    exclude: &'a [String],
    is_directory: bool,
) -> Result<MatchType, &'a String> {
    let path = match path.to_str() {
        Some(path) => path,
        None => return Ok(MatchType::None), // you can't match a path that isn't valid unicode
    };

    if include.is_empty() {
        return Ok(MatchType::Match);
    }

    println!("matching: {:?}", path);

    let included = include
        .iter()
        .map(|glob| match_include(glob, path, is_directory).ok_or(glob))
        // we want to stop searching if we find an exact match, but keep searching
        // if we find a potential match or an invalid glob
        .fold_while(Ok(MatchType::None), |acc, res| match (acc, res) {
            (_, Ok(MatchType::Match)) => Done(Ok(MatchType::Match)), // stop searching on an exact match
            (_, Err(glob)) => Done(Err(glob)), // stop searching on an invalid glob
            (_, Ok(MatchType::PotentialMatch)) => Continue(Ok(MatchType::PotentialMatch)), // keep searching on a potential match
            (Ok(match_type), Ok(MatchType::None)) => Continue(Ok(match_type)), // keep searching on a non-match
            (Err(_), _) => unreachable!("we stop searching on an error"),
        })
        .into_inner();

    let excluded = exclude
        .iter()
        .map(|glob| glob_match(glob, path).ok_or(glob))
        .find_map(|res| match res {
            Ok(false) => None,            // no match, keep searching
            Ok(true) => Some(Ok(true)),   // match, stop searching
            Err(glob) => Some(Err(glob)), // invalid glob, stop searching
        })
        .unwrap_or(Ok(false));

    println!("included: {:?}, excluded: {:?}", included, excluded);

    match (included, excluded) {
        // a match of the excludes always wins
        (_, Ok(true)) | (Ok(MatchType::None), Ok(false)) => Ok(MatchType::None),
        (Ok(match_type), Ok(false)) => Ok(match_type),
        (Err(glob), _) => Err(glob),
        (_, Err(glob)) => Err(glob),
    }
}

fn match_include(include: &str, path: &str, is_dir: bool) -> Option<MatchType> {
    println!(
        "matching include: {:?} against {:?}, is_dir: {}",
        include, path, is_dir
    );
    if is_dir {
        potential_match(include, path)
    } else {
        match glob_match(include, path) {
            Some(true) => Some(MatchType::Match),
            Some(false) => Some(MatchType::None),
            None => None,
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use itertools::Itertools;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::{MatchType, WalkError};

    #[test_case("/a/b/c/d", "/a/b/c/d", MatchType::Match; "exact match")]
    #[test_case("/a", "/a/b/c", MatchType::PotentialMatch; "minimal match")]
    #[test_case("/a/b/c/d", "**", MatchType::Match; "doublestar")]
    #[test_case("/a/b/c", "/b", MatchType::None; "no match")]
    #[test_case("a", "a/b/**", MatchType::PotentialMatch; "relative path")]
    #[test_case("a/b", "a/**/c/d", MatchType::PotentialMatch; "doublestar with later folders")]
    #[test_case("/a/b/c", "/a/*/c", MatchType::Match; "singlestar")]
    #[test_case("/a/b/c/d/e", "/a/**/d/e", MatchType::Match; "doublestar middle")]
    #[test_case("/a/b/c/d/e", "/a/**/e", MatchType::Match; "doublestar skip folders")]
    #[test_case("/a/b/c/d/e", "/a/**/*", MatchType::Match; "doublestar singlestar combination")]
    #[test_case("/a/b/c/d/e", "/a/*/*/d/*", MatchType::Match; "multiple singlestars")]
    #[test_case("/a/b/c/d/e", "/**/c/d/*", MatchType::Match; "leading doublestar")]
    #[test_case("/a/b/c/d/e", "/*/b/**", MatchType::Match; "leading singlestar and doublestar")]
    #[test_case("/a/b/c/d", "/a/b/c/?", MatchType::Match; "question mark match")]
    #[test_case("/a/b/c/d/e/f", "/a/b/**/e/?", MatchType::Match; "doublestar question mark combination")]
    #[test_case("/a/b/c/d/e/f", "/a/*/c/d/*/?", MatchType::Match; "singlestar doublestar question mark combination")]
    #[test_case("/a/b/c/d", "/a/b/c/?/e", MatchType::PotentialMatch; "question mark over match")]
    #[test_case("/a/b/c/d/e/f", "/a/b/*/e/f", MatchType::None; "singlestar no match")]
    #[test_case("/a/b/c/d/e", "/a/b/**/e/f/g", MatchType::PotentialMatch; "doublestar over match")]
    #[test_case("/a/b/c/d/e", "/a/b/*/d/z", MatchType::None; "multiple singlestars no match")]

    fn potential_match(path: &str, glob: &str, exp: MatchType) {
        assert_eq!(super::potential_match(glob, path), Some(exp));
    }

    #[test]
    fn do_match_empty_include() {
        assert_eq!(
            super::do_match(Path::new("/a/b/c/d"), &[], &[], false).unwrap(),
            MatchType::Match
        )
    }

    /// set up a globwalk test in a tempdir, returning the path to the tempdir
    fn setup() -> tempdir::TempDir {
        let tmp = tempdir::TempDir::new("globwalk").unwrap();

        let directories = ["a/b/c", "a/c", "abc", "axbxcxdxe/xxx", "axbxcxdxexxx", "b"];

        let files = [
            "a/abc",
            "a/b/c/d",
            "a/c/b",
            "abc/b",
            "abcd",
            "abcde",
            "abxbbxdbxebxczzx",
            "abxbbxdbxebxczzy",
            "axbxcxdxe/f",
            "axbxcxdxe/xxx/f",
            "axbxcxdxexxx/f",
            "axbxcxdxexxx/fff",
            "a☺b",
            "b/c",
            "c",
            "x",
            "xxx",
            "z",
            "α",
            "abc/【test】.txt",
        ];

        for dir in directories.iter() {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }

        for file in files.iter() {
            std::fs::File::create(tmp.path().join(file)).unwrap();
        }

        if cfg!(unix) {
            // these files/symlinks won't work on Windows
            std::fs::File::create(tmp.path().join("-")).unwrap();
            std::fs::File::create(tmp.path().join("]")).unwrap();

            std::os::unix::fs::symlink("../axbxcxdxe/", tmp.path().join("b/symlink-dir")).unwrap();
            std::os::unix::fs::symlink(
                "/tmp/nonexistant-file-20160902155705",
                tmp.path().join("broken-symlink"),
            )
            .unwrap();
            std::os::unix::fs::symlink("a/b", tmp.path().join("working-symlink")).unwrap();
        }

        // filesystem needs to propagate changes
        // std::thread::sleep(Duration::from_millis(100));

        tmp
    }

    #[test_case("abc", None, 1, 1 ; "exact match")]
    #[test_case("*", None, 18, 15 ; "single star match")]
    #[test_case("*c", None, 2, 2 ; "single star suffix match")]
    #[test_case("a*", None, 9, 9 ; "single star prefix match")]
    #[test_case("a*/b", None, 2, 2 ; "single star prefix with suffix match")]
    #[test_case("a*b*c*d*e*", None, 3, 3 ; "multiple single stars match")]
    #[test_case("a*b*c*d*e*/f", None, 2, 2 ; "single star and double star match")]
    #[test_case("a*b?c*x", None, 2, 2 ; "single star and question mark match")]
    #[test_case("ab[c]", None, 1, 1 ; "character class match")]
    #[test_case("ab[b-d]", None, 1, 1 ; "character class range match")]
    #[test_case("ab[e-g]", None, 0, 0 ; "character class range mismatch")]
    #[test_case("ab[^c]", None, 0, 0 ; "negated character class mismatch")]
    #[test_case("ab[^b-d]", None, 0, 0 ; "negated character class range mismatch")]
    #[test_case("ab[^e-g]", None, 1, 1 ; "negated character class range match")]
    #[test_case("a\\*b", None, 0, 0 ; "escaped star mismatch")]
    #[test_case("a?b", None, 1, 1 ; "question mark unicode match")]
    #[test_case("a[^a]b", None, 1, 1 ; "negated character class unicode match")]
    #[test_case("a[!a]b", None, 1, 1 ; "negated character class unicode match 2")]
    #[test_case("a???b", None, 0, 0 ; "insufficient question marks mismatch")]
    #[test_case("a[^a][^a][^a]b", None, 0, 0 ; "multiple negated character classes mismatch")]
    #[test_case("a?b", None, 1, 1 ; "question mark not matching slash")]
    #[test_case("a*b", None, 1, 1 ; "single star not matching slash 2")]
    #[test_case("[x-]", None, 2, 1 ; "trailing dash in character class match")]
    #[test_case("[-x]", None, 2, 1 ; "leading dash in character class match")]
    #[test_case("[a-b-d]", None, 3, 2 ; "dash within character class range match")]
    #[test_case("[a-b-x]", None, 4, 3 ; "dash within character class range match 4")]
    #[test_case("[", Some(WalkError::BadPattern("[".into())), 0, 0 ; "unclosed character class error")]
    #[test_case("[^", Some(WalkError::BadPattern("[^".into())), 0, 0 ; "unclosed negated character class error")]
    #[test_case("[^bc", Some(WalkError::BadPattern("[^bc".into())), 0, 0 ; "unclosed negated character class error 2")]
    #[test_case("a[", Some(WalkError::BadPattern("a[".into())), 0, 0 ; "unclosed character class error after pattern")]
    #[test_case("ad[", Some(WalkError::BadPattern("ad[".into())), 0, 0 ; "unclosed character class error after pattern 3")]
    #[test_case("*x", None, 4, 4 ; "star pattern match")]
    #[test_case("[abc]", None, 3, 3 ; "single character class match")]
    #[test_case("a/**", None, 6, 6 ; "a followed by double star match")]
    #[test_case("**/c", None, 5, 4 ; "double star and single subdirectory match")]
    #[test_case("a/**/b", None, 2, 2 ; "a followed by double star and single subdirectory match")]
    #[test_case("a/**/c", None, 2, 2 ; "a followed by double star and multiple subdirectories match 2")]
    #[test_case("a/**/d", None, 1, 1 ; "a followed by double star and multiple subdirectories with target match")]
    #[test_case("a/b/c", None, 1, 1 ; "a followed by subdirectories and double slash mismatch")]
    #[test_case("ab{c,d}", None, 1, 1 ; "pattern with curly braces match")]
    #[test_case("ab{c,d,*}", None, 5, 5 ; "pattern with curly braces and wildcard match")]
    #[test_case("ab{c,d}[", Some(WalkError::BadPattern("ab{c,d}[".into())), 0, 0)]
    // ; "pattern with curly braces and unclosed character class error"
    #[test_case("a{,bc}", None, 1, 1 ; "a followed by comma or b or c")]
    #[test_case("a/{b/c,c/b}", None, 2, 2)]
    #[test_case("{a/{b,c},abc}", None, 3, 3)]
    #[test_case("{a/ab*}", None, 1, 1)]
    #[test_case("{a/*}", None, 3, 3)]
    #[test_case("{a/abc}", None, 1, 1)]
    #[test_case("{a/b,a/c}", None, 2, 2)]
    #[test_case("abc/**", None, 2, 2 ; "abc then doublestar")]
    #[test_case("**/abc", None, 2, 2)]
    #[test_case("**/*.txt", None, 1, 1)]
    #[test_case("**/【*", None, 1, 1)]
    // broken symlinks will not appear
    #[test_case("broken-symlink", None, 0, 0)]
    // // We don't care about matching a particular file, we want to verify
    // // that we don't traverse the symlink
    #[test_case("working-symlink/c/*", None, 1, 1)]
    #[test_case("working-sym*/*", None, 1, 0)]
    #[test_case("b/**/f", None, 2, 0)]
    fn glob_walk(
        pattern: &str,
        err_expected: Option<WalkError>,
        result_count: usize,
        result_count_windows: usize,
    ) {
        glob_walk_inner(
            pattern,
            err_expected,
            if cfg!(windows) {
                result_count_windows
            } else {
                result_count
            },
        )
    }

    // these tests were configured to only run on unix, and not on windows
    #[cfg(unix)]
    #[test_case("[\\]a]", None, 2 ; "escaped bracket match")]
    #[test_case("[\\-]", None, 1 ; "escaped dash match")]
    #[test_case("[x\\-]", None, 2 ; "character class with escaped dash match")]
    #[test_case("[x\\-]", None, 2 ; "escaped dash in character class match")]
    #[test_case("[x\\-]", None, 2 ; "escaped dash in character class mismatch")]
    #[test_case("[\\-x]", None, 2 ; "escaped dash and character match")]
    #[test_case("[\\-x]", None, 2 ; "escaped dash and character match 2")]
    #[test_case("[\\-x]", None, 2 ; "escaped dash and character mismatch")]
    #[test_case("[-]", None, 1 ; "bare dash in character class match")]
    #[test_case("[x-]", None, 2 ; "trailing dash in character class match 2")]
    #[test_case("[-x]", None, 2 ; "leading dash in character class match 2")]
    #[test_case("[a-b-d]", None, 3 ; "dash within character class range match 3")]
    #[test_case("\\", Some(WalkError::BadPattern("\\".into())), 0 ; "single backslash error")]
    #[test_case("a/\\**", None, 0 ; "a followed by escaped double star and subdirectories mismatch")]
    #[test_case("a/\\[*\\]", None, 0 ; "a followed by escaped character class and pattern mismatch")]
    fn glob_walk_unix(pattern: &str, err_expected: Option<WalkError>, result_count: usize) {
        glob_walk_inner(pattern, err_expected, result_count)
    }

    fn glob_walk_inner(pattern: &str, err_expected: Option<WalkError>, result_count: usize) {
        let dir = setup();
        println!("running in {:?}", dir.path());

        let path = AbsoluteSystemPathBuf::new(dir.path()).unwrap();
        let (success, error): (Vec<AbsoluteSystemPathBuf>, Vec<_>) =
            super::globwalk(&path, &[pattern.into()], &[], crate::WalkType::All).partition_result();

        assert_eq!(
            success.len(),
            result_count,
            "{}: expected {} matches, but got {:#?}",
            pattern,
            result_count,
            success
        );

        if let Some(_) = err_expected {
            assert!(error.len() > 0); // todo: check the error
        }
    }
}

// func verifyGlobResults(t *testing.T, idx int, fn string, tt MatchTest, fsys
// fs.FS, matches []string, err error) {

// 	if inSlice(tt.testPath, matches) != tt.shouldMatch {
// 		if tt.shouldMatch {
// 			t.Errorf("#%v. %v(%#q) = %#v - doesn't contain %v, but should", idx, fn,
// tt.pattern, matches, tt.testPath) 		} else {
// 			t.Errorf("#%v. %v(%#q) = %#v - contains %v, but shouldn't", idx, fn,
// tt.pattern, matches, tt.testPath) 		}
// 	}
// 	if err != tt.expectedErr {
// 		t.Errorf("#%v. %v(%#q) has error %v, but should be %v", idx, fn, tt.pattern,
// err, tt.expectedErr) 	}

// 	if tt.isStandard {
// 		stdMatches, stdErr := fs.Glob(fsys, tt.pattern)
// 		if !compareSlices(matches, stdMatches) || !compareErrors(err, stdErr) {
// 			t.Errorf("#%v. %v(%#q) != fs.Glob(...). Got %#v, %v want %#v, %v", idx, fn,
// tt.pattern, matches, err, stdMatches, stdErr) 		}
// 	}
// }
