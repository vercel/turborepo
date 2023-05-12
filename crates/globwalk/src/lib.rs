use std::{io::ErrorKind, path::Path};

use itertools::{
    FoldWhile::{Continue, Done},
    Itertools,
};
use path_slash::PathExt;
use turbopath::AbsoluteSystemPathBuf;
use wax::{Glob, Pattern};

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

fn glob_match(pattern: &str, path: &str) -> Option<bool> {
    let glob = match Glob::new(pattern) {
        Ok(glob) => glob,
        Err(e) => {
            println!("{}", e);
            return None;
        }
    };
    let result = glob.is_match(path);
    Some(result)
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
    // we enable following symlinks but only because without it they are ignored
    // completely (as opposed to yielded but not followed)

    let walker = walkdir::WalkDir::new(base_path.as_path()).follow_links(false);
    let mut iter = walker.into_iter();

    let include = include
        .into_iter()
        .filter_map(|s| collapse_path(s))
        .collect::<Vec<_>>();

    let exclude = exclude
        .into_iter()
        .filter_map(|g| {
            let split = collapse_path(g)?;
            if split.ends_with('/') {
                Some(Cow::Owned(format!("{}**", split)))
            } else {
                Some(split)
            }
        })
        .collect::<Vec<_>>();

    std::iter::from_fn(move || loop {
        let entry = iter.next()?;

        let (is_symlink, path) = match entry {
            Ok(entry) => (entry.path_is_symlink(), entry.into_path()),
            Err(err) => match (err.io_error(), err.path()) {
                // make sure to yield broken symlinks
                (Some(io_err), Some(path))
                    if io_err.kind() == ErrorKind::NotFound && path.is_symlink() =>
                {
                    (true, path.to_owned())
                }
                _ => return Some(Err(WalkError::WalkDir(err))),
            },
        };

        let relative_path = path.strip_prefix(&base_path).expect("it is a subdir");
        let is_directory = !path.is_symlink() && path.is_dir();

        let include = match do_match_directory(relative_path, &include, &exclude, is_directory) {
            Ok(include) => include,
            Err(glob) => return Some(Err(WalkError::BadPattern(glob.to_string()))),
        };

        if (include == MatchType::None || is_symlink) && is_directory {
            iter.skip_current_dir();
        }

        match include {
            // if it is a perfect match, and our walk_type allows it, then we should yield it
            MatchType::Match if walk_type.should_emit(is_directory) => {
                return Some(Ok(AbsoluteSystemPathBuf::new(path).expect("absolute")));
            }
            // we should yield potential matches if they are symlinks. we don't want to traverse
            // into them, but simply say 'hey this is a symlink that could match'
            // MatchType::PotentialMatch if is_symlink && walk_type.should_emit(is_directory) => {
            // return Some(Ok(AbsoluteSystemPathBuf::new(path).expect("absolute")));
            // }
            // just skip and continue on with the loop
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

fn do_match_directory<'a, S: AsRef<str>>(
    path: &Path,
    include: &'a [S],
    exclude: &'a [S],
    is_directory: bool,
) -> Result<MatchType, &'a S> {
    let path_unix = match path.to_slash() {
        Some(path) => path,
        None => return Ok(MatchType::None), // you can't match a path that isn't valid unicode
    };

    let first = do_match(&path_unix, include, exclude, is_directory);

    first
}

/// Executes a match against a relative path using the given include and exclude
/// globs. If the path is not valid unicode, then it is automatically not
/// matched.
///
/// If an evaluated glob is invalid, then this function returns an error with
/// the glob that failed.
fn do_match<'a, S: AsRef<str>>(
    path: &str,
    include: &'a [S],
    exclude: &'a [S],
    is_directory: bool,
) -> Result<MatchType, &'a S> {
    if include.is_empty() {
        return Ok(MatchType::Match);
    }

    let included = include
        .iter()
        .map(|glob| match_include(glob.as_ref(), path, is_directory).ok_or(glob))
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
        .map(|glob| glob_match(glob.as_ref(), path).ok_or(glob))
        .find_map(|res| match res {
            Ok(false) => None,            // no match, keep searching
            Ok(true) => Some(Ok(true)),   // match, stop searching
            Err(glob) => Some(Err(glob)), // invalid glob, stop searching
        })
        .unwrap_or(Ok(false));

    match (included, excluded) {
        // a match of the excludes always wins
        (_, Ok(true)) | (Ok(MatchType::None), Ok(false)) => Ok(MatchType::None),
        (Ok(match_type), Ok(false)) => Ok(match_type),
        (Err(glob), _) => Err(glob),
        (_, Err(glob)) => Err(glob),
    }
}

fn match_include(include: &str, path: &str, is_dir: bool) -> Option<MatchType> {
    if is_dir {
        potential_match(include, path)
    } else {
        // ensure that directories end with a slash
        // so that trailing star globs match

        match glob_match(include, path) {
            Some(true) => Some(MatchType::Match),
            Some(false) => Some(MatchType::None),
            None => None,
        }
    }
}

use std::borrow::Cow;

fn collapse_path(path: &str) -> Option<Cow<str>> {
    let mut stack: Vec<&str> = vec![];
    let mut changed = false;
    let is_root = path.starts_with("/");

    for segment in path.trim_start_matches('/').split('/') {
        match segment {
            ".." => {
                if let None = stack.pop() {
                    return None;
                }
                changed = true;
            }
            "." => {
                changed = true;
            }
            _ => stack.push(segment),
        }
    }

    if !changed {
        Some(Cow::Borrowed(path))
    } else {
        let string = if is_root {
            std::iter::once("").chain(stack.into_iter()).join("/")
        } else {
            stack.join("/")
        };

        Some(Cow::Owned(string))
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use itertools::Itertools;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::{collapse_path, MatchType, WalkError};

    #[test_case("a/./././b", "a/b" ; "test path with dot segments")]
    #[test_case("a/../b", "b" ; "test path with dotdot segments")]
    #[test_case("a/./../b", "b" ; "test path with mixed dot and dotdot segments")]
    #[test_case("./a/b", "a/b" ; "test path starting with dot segment")]
    #[test_case("a/b/..", "a" ; "test path ending with dotdot segment")]
    #[test_case("a/b/.", "a/b" ; "test path ending with dot segment")]
    #[test_case("a/.././b", "b" ; "test path with mixed and consecutive ./ and ../ segments")]
    #[test_case("/a/./././b", "/a/b" ; "test path with leading / and ./ segments")]
    #[test_case("/a/../b", "/b" ; "test path with leading / and dotdot segments")]
    #[test_case("/a/./../b", "/b" ; "test path with leading / and mixed dot and dotdot segments")]
    #[test_case("/./a/b", "/a/b" ; "test path with leading / and starting with dot segment")]
    #[test_case("/a/b/..", "/a" ; "test path with leading / and ending with dotdot segment")]
    #[test_case("/a/b/.", "/a/b" ; "test path with leading / and ending with dot segment")]
    #[test_case("/a/.././b", "/b" ; "test path with leading / and mixed and consecutive dot and dotdot segments")]
    fn test_collapse_path(glob: &str, expected: &str) {
        assert_eq!(collapse_path(glob).unwrap(), expected);
    }

    #[test_case("../a/b" ; "test path starting with ../ segment should return None")]
    #[test_case("/../a" ; "test path with leading dotdotdot segment should return None")]
    fn test_collapse_path_not(glob: &str) {
        assert_eq!(collapse_path(glob), None);
    }

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
            super::do_match_directory::<&str>(Path::new("/a/b/c/d"), &[], &[], false).unwrap(),
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
    #[test_case("*", None, 19, 15 ; "single star match")]
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
    // glob watch will not error on this, since it does not get far enough into the glob to see the
    // error
    #[test_case("ad[", None, 0, 0 ; "unclosed character class error after pattern 3")]
    #[test_case("*x", None, 4, 4 ; "star pattern match")]
    #[test_case("[abc]", None, 3, 3 ; "single character class match")]
    #[test_case("a/**", None, 7, 7 ; "a followed by double star match")]
    #[test_case("**/c", None, 4, 4 ; "double star and single subdirectory match")]
    #[test_case("a/**/b", None, 2, 2 ; "a followed by double star and single subdirectory match")]
    #[test_case("a/**/c", None, 2, 2 ; "a followed by double star and multiple subdirectories match 2")]
    #[test_case("a/**/d", None, 1, 1 ; "a followed by double star and multiple subdirectories with target match")]
    #[test_case("a/b/c", None, 1, 1 ; "a followed by subdirectories and double slash mismatch")]
    #[test_case("ab{c,d}", None, 1, 1 ; "pattern with curly braces match")]
    #[test_case("ab{c,d,*}", None, 5, 5 ; "pattern with curly braces and wildcard match")]
    #[test_case("ab{c,d}[", Some(WalkError::BadPattern("ab{c,d}[".into())), 0, 0)]
    // #[test_case("a{,bc}", None, 2, 2 ; "a followed by comma or b or c")]
    #[test_case("a{,bc}", Some(WalkError::BadPattern("a{,bc}".into())), 0, 0 ; "a followed by comma or b or c")]
    #[test_case("a/{b/c,c/b}", None, 2, 2)]
    #[test_case("{a/{b,c},abc}", None, 3, 3)]
    #[test_case("{a/ab*}", None, 1, 1)]
    #[test_case("a/*", None, 3, 3)]
    #[test_case("{a/*}", None, 3, 3 ; "curly braces with single star match")]
    #[test_case("{a/abc}", None, 1, 1)]
    #[test_case("{a/b,a/c}", None, 2, 2)]
    #[test_case("abc/**", None, 3, 3 ; "abc then doublestar")]
    #[test_case("**/abc", None, 2, 2)]
    #[test_case("**/*.txt", None, 1, 1)]
    #[test_case("**/【*", None, 1, 1)]
    // in the go implementation, broken-symlink is yielded,
    // however in symlink mode, walkdir yields broken symlinks as errors
    #[test_case("broken-symlink", None, 1, 1 ; "broken symlinks should be yielded")]
    // globs that match across a symlink should not follow the symlink
    #[test_case("working-symlink/c/*", None, 0, 0 ; "working symlink should not be followed")]
    #[test_case("working-sym*/*", None, 0, 0 ; "working symlink should not be followed 2")]
    #[test_case("b/**/f", None, 0, 0)]
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
    #[test_case("[-]", Some(WalkError::BadPattern("[-]".into())), 0 ; "bare dash in character class match")]
    #[test_case("[x-]", Some(WalkError::BadPattern("[x-]".into())), 0 ; "trailing dash in character class match 2")]
    #[test_case("[-x]", Some(WalkError::BadPattern("[-x]".into())), 0 ; "leading dash in character class match 2")]
    #[test_case("[a-b-d]", Some(WalkError::BadPattern("[a-b-d]".into())), 0 ; "dash within character class range match 3")]
    #[test_case("\\", Some(WalkError::BadPattern("\\".into())), 0 ; "single backslash error")]
    #[test_case("a/\\**", None, 0 ; "a followed by escaped double star and subdirectories mismatch")]
    #[test_case("a/\\[*\\]", None, 0 ; "a followed by escaped character class and pattern mismatch")]
    fn glob_walk_unix(pattern: &str, err_expected: Option<WalkError>, result_count: usize) {
        glob_walk_inner(pattern, err_expected, result_count)
    }

    fn glob_walk_inner(pattern: &str, err_expected: Option<WalkError>, result_count: usize) {
        let dir = setup();

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

    #[test_case(
        &["/test.txt"],
        "/",
        &["*.txt"],
        &[],
        &["/test.txt"],
        &["/test.txt"]
        ; "hello world"
    )]
    #[test_case(
        &["/test.txt", "/subdir/test.txt", "/other/test.txt"],
        "/",
        &["subdir/test.txt", "test.txt"],
        &[],
        &["/subdir/test.txt", "/test.txt"],
        &["/subdir/test.txt", "/test.txt"]
        ; "bullet files"
    )]
    #[test_case(&[
            "/external/file.txt",
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/bower_components/readline/package.json",
            "/repos/some-app/examples/package.json",
            "/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
            "/repos/some-app/node_modules/react/package.json",
            "/repos/some-app/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/test/mocks/kitchen-sink/package.json",
            "/repos/some-app/tests/mocks/kitchen-sink/package.json",
        ],
        "/repos/some-app/",
        &["packages/*/package.json", "apps/*/package.json"], &["**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"],
        &[
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
        ],
        &[
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
        ]
        ; "finding workspace package.json files"
    )]
    #[test_case(&[
            "/external/file.txt",
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/bower_components/readline/package.json",
            "/repos/some-app/examples/package.json",
            "/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
            "/repos/some-app/node_modules/react/package.json",
            "/repos/some-app/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/test/mocks/spanish-inquisition/package.json",
            "/repos/some-app/tests/mocks/spanish-inquisition/package.json",
        ],
        "/repos/some-app/",
        &["**/package.json"],
        &["**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"],
        &[
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/examples/package.json",
            "/repos/some-app/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
        ],
        &[
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/examples/package.json",
            "/repos/some-app/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
        ]
        ; "excludes unexpected workspace package.json files"
    )]
    #[test_case(&[
            "/external/file.txt",
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/bower_components/readline/package.json",
            "/repos/some-app/examples/package.json",
            "/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
            "/repos/some-app/node_modules/react/package.json",
            "/repos/some-app/package.json",
            "/repos/some-app/packages/xzibit/package.json",
            "/repos/some-app/packages/xzibit/node_modules/street-legal/package.json",
            "/repos/some-app/packages/xzibit/node_modules/paint-colors/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/meme/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/yo-dawg/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/test/mocks/spanish-inquisition/package.json",
            "/repos/some-app/tests/mocks/spanish-inquisition/package.json",
        ],
        "/repos/some-app/",
        &["packages/**/package.json"],
        &["**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"],
        &[
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/packages/xzibit/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
        ],
        &[
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/packages/xzibit/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
        ]
        ; "nested packages work")]
    #[test_case(&[
            "/external/file.txt",
            "/repos/some-app/apps/docs/package.json",
            "/repos/some-app/apps/web/package.json",
            "/repos/some-app/bower_components/readline/package.json",
            "/repos/some-app/examples/package.json",
            "/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
            "/repos/some-app/node_modules/react/package.json",
            "/repos/some-app/package.json",
            "/repos/some-app/packages/xzibit/package.json",
            "/repos/some-app/packages/xzibit/node_modules/street-legal/package.json",
            "/repos/some-app/packages/xzibit/node_modules/paint-colors/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/meme/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/yo-dawg/package.json",
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/test/mocks/spanish-inquisition/package.json",
            "/repos/some-app/tests/mocks/spanish-inquisition/package.json",
        ],
        "/repos/some-app/",
        &["packages/**/package.json", "tests/mocks/*/package.json"],
        &["**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"],
        &[
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/packages/xzibit/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
        ],
        &[
            "/repos/some-app/packages/colors/package.json",
            "/repos/some-app/packages/faker/package.json",
            "/repos/some-app/packages/left-pad/package.json",
            "/repos/some-app/packages/xzibit/package.json",
            "/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
        ]
        ; "includes do not override excludes")]
    #[test_case(&[
            "/external/file.txt",
            "/repos/some-app/src/index.js",
            "/repos/some-app/public/src/css/index.css",
            "/repos/some-app/.turbo/turbo-build.log",
            "/repos/some-app/.turbo/somebody-touched-this-file-into-existence.txt",
            "/repos/some-app/.next/log.txt",
            "/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
            "/repos/some-app/public/dist/css/index.css",
            "/repos/some-app/public/dist/images/rick_astley.jpg",
        ],
        "/repos/some-app/",
        &[".turbo/turbo-build.log", "dist/**", ".next/**", "public/dist/**"],
        &[],
        &[
            "/repos/some-app/.next",
            "/repos/some-app/.next/cache",
            "/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
            "/repos/some-app/.next/log.txt",
            "/repos/some-app/.turbo/turbo-build.log",
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules",
            "/repos/some-app/dist/js/node_modules/browserify.js",
            "/repos/some-app/public/dist",
            "/repos/some-app/public/dist/css",
            "/repos/some-app/public/dist/css/index.css",
            "/repos/some-app/public/dist/images",
            "/repos/some-app/public/dist/images/rick_astley.jpg",
        ],
        &[
            "/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
            "/repos/some-app/.next/log.txt",
            "/repos/some-app/.turbo/turbo-build.log",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
            "/repos/some-app/public/dist/css/index.css",
            "/repos/some-app/public/dist/images/rick_astley.jpg",
        ]
        ; "output globbing grabs the desired content"
    )]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], "/repos/some-app/",
        &["dist/**"],
        &[],
        &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        &[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ]
        ; "passing ** captures all children")]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["dist"],
        &[],
        &["/repos/some-app/dist"],
        &[]
        ; "passing just a directory captures no children")]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], "/repos/some-app/", &["**/*", "dist/**"], &[ ], &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], &[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ] ; "redundant includes do not duplicate")]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], "/repos/some-app/", &["**"], &["**"], &[ ], &[ ] ; "exclude everything, include everything")]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["dist/**"],
        &["dist/js"],
        &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
        ],
        &[
            "/repos/some-app/dist/index.html",
        ]
        ; "passing just a directory to exclude prevents capture of children")]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["dist/**"],
        &["dist/js/**"],
        &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
        ],
        &[
            "/repos/some-app/dist/index.html",
        ] ; "passing ** to exclude prevents capture of children")]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["**"],
        &["./"],
        &[],
        &[]
        ; "exclude everything with folder . applies at base path"
    )]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["**"],
        &["./dist"],
        &[],
        &[]
        ; "exclude everything with traversal applies at a non-base path"
    )]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["**"],
        &["dist/../"],
        &[],
        &[]
        ; "exclude everything with folder traversal (..) applies at base path"
    )]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], "/repos/some-app/", &["**/**/**"], &[], &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], &[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ]
        ; "how do globs even work bad glob microformat"
    )]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js"
        ],
        "/repos/some-app/",
        &["dist/js/../**"],
        &[],
        &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules",
            "/repos/some-app/dist/js/node_modules/browserify.js"],
        &[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ]
        ; "traversal works within base path"
    )]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["dist/./././**"],
        &[],
        &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        &[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ]
        ; "self references work (.)"
    )]
    #[test_case(&[
            "/repos/some-app/package.json",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], "/repos/some-app/", &["*"], &[ ], &[
            "/repos/some-app/dist",
            "/repos/some-app/package.json",
        ], &["/repos/some-app/package.json"] ; "depth of 1 includes handles folders properly")]
    #[test_case(&[
            "/repos/some-app/package.json",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ], "/repos/some-app/", &["**"], &["dist/*"], &[
            "/repos/some-app/dist",
            "/repos/some-app/package.json",
        ], &["/repos/some-app/package.json"] ; "depth of 1 excludes prevents capturing folders")]
    #[test_case(&[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app",
        &["dist/**"],
        &[],
        &[
            "/repos/some-app/dist",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        &[
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ]
        ; "No-trailing slash basePath works")]
    #[test_case(&[
            "/repos/some-app/included.txt",
            "/repos/some-app/excluded.txt",
        ], "/repos/some-app", &["*.txt"], &["excluded.txt"], &[
            "/repos/some-app/included.txt",
        ], &[
            "/repos/some-app/included.txt",
        ] ; "exclude single file")]
    #[test_case(&[
            "/repos/some-app/one/included.txt",
            "/repos/some-app/one/two/included.txt",
            "/repos/some-app/one/two/three/included.txt",
            "/repos/some-app/one/excluded.txt",
            "/repos/some-app/one/two/excluded.txt",
            "/repos/some-app/one/two/three/excluded.txt",
        ], "/repos/some-app", &["**"], &["**/excluded.txt"], &[
            "/repos/some-app/one/included.txt",
            "/repos/some-app/one/two/included.txt",
            "/repos/some-app/one/two/three/included.txt",
            "/repos/some-app/one",
            "/repos/some-app/one/two",
            "/repos/some-app/one/two/three",
        ], &[
            "/repos/some-app/one/included.txt",
            "/repos/some-app/one/two/included.txt",
            "/repos/some-app/one/two/three/included.txt",
        ] ; "exclude nested single file")]
    #[test_case(&[
            "/repos/some-app/one/included.txt",
            "/repos/some-app/one/two/included.txt",
            "/repos/some-app/one/two/three/included.txt",
            "/repos/some-app/one/excluded.txt",
            "/repos/some-app/one/two/excluded.txt",
            "/repos/some-app/one/two/three/excluded.txt",
        ], "/repos/some-app", &["**"], &["**"], &[], &[] ; "exclude everything")]
    #[test_case(&[
            "/repos/some-app/one/included.txt",
            "/repos/some-app/one/two/included.txt",
            "/repos/some-app/one/two/three/included.txt",
            "/repos/some-app/one/excluded.txt",
            "/repos/some-app/one/two/excluded.txt",
            "/repos/some-app/one/two/three/excluded.txt",
        ], "/repos/some-app", &["**"], &["**/"], &[], &[] ; "exclude everything with slash")]
    #[test_case(&[
            "/repos/some-app/foo/bar",
            "/repos/some-app/some-foo/bar",
            "/repos/some-app/included",
        ],
        "/repos/some-app",
        &["**"],
        &["**foo"],
        &[
            "/repos/some-app/included",
        ],
        &[
            "/repos/some-app/included",
        ]
        ; "exclude everything with leading **")]
    #[test_case(&[
            "/repos/some-app/foo/bar",
            "/repos/some-app/foo-file",
            "/repos/some-app/foo-dir/bar",
            "/repos/some-app/included",
        ], "/repos/some-app", &["**"], &["foo**"], &[
            "/repos/some-app/included",
        ], &[
            "/repos/some-app/included",
        ] ; "exclude everything with trailing **")]
    fn glob_walk_files(
        files: &[&str],
        base_path: &str,
        include: &[&str],
        exclude: &[&str],
        expected: &[&str],
        expected_files: &[&str],
    ) {
        let dir = setup_files(files);
        let base_path = base_path.trim_start_matches('/');
        let path = AbsoluteSystemPathBuf::new(dir.path().join(base_path)).unwrap();
        let include: Vec<_> = include.iter().map(|s| s.to_string()).collect();
        let exclude: Vec<_> = exclude.iter().map(|s| s.to_string()).collect();

        for (walk_type, expected) in [
            (crate::WalkType::Files, expected_files),
            (crate::WalkType::All, expected),
        ] {
            let (success, _): (Vec<AbsoluteSystemPathBuf>, Vec<_>) =
                super::globwalk(&path, &include, &exclude, walk_type).partition_result();

            let success = success
                .iter()
                .map(|p| {
                    p.as_path()
                        .strip_prefix(dir.path())
                        .unwrap()
                        .to_str()
                        .unwrap()
                })
                .sorted()
                .collect::<Vec<_>>();

            let expected = expected
                .iter()
                .map(|p| p.trim_start_matches('/'))
                .sorted()
                .collect::<Vec<_>>();

            assert_eq!(
                success, expected,
                "\n\nexpected \n{:#?} but got \n{:#?}",
                expected, success
            );
        }
    }

    #[test_case(&[
            "/repos/spanish-inquisition/index.html",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["../spanish-inquisition/**", "dist/**"],
        &[],
        &[],
        &[]
        ; "globs and traversal and globs do not cross base path"
    )]
    #[test_case(
        &[
            "/repos/spanish-inquisition/index.html",
            "/repos/some-app/dist/index.html",
            "/repos/some-app/dist/js/index.js",
            "/repos/some-app/dist/js/lib.js",
            "/repos/some-app/dist/js/node_modules/browserify.js",
        ],
        "/repos/some-app/",
        &["**/../../spanish-inquisition/**"],
        &[],
        &[],
        &[]
        ; "globs and traversal and globs do not cross base path doublestart up"
    )]
    fn glob_walk_err(
        files: &[&str],
        base_path: &str,
        include: &[&str],
        exclude: &[&str],
        expected: &[&str],
        expected_files: &[&str],
    ) {
        let dir = setup_files(files);
    }

    fn setup_files(files: &[&str]) -> tempdir::TempDir {
        let tmp = tempdir::TempDir::new("globwalk").unwrap();
        for file in files {
            let file = file.trim_start_matches('/');
            let path = tmp.path().join(file);
            let parent = path.parent().unwrap();
            std::fs::create_dir_all(parent)
                .expect(format!("failed to create {:?}", parent).as_str());
            std::fs::File::create(path).unwrap();
        }
        tmp
    }
}
