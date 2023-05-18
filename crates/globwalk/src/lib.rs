mod empty_glob;

use std::{
    borrow::Cow,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use empty_glob::InclusiveEmptyAny;
use itertools::Itertools;
use path_slash::PathExt;
use turbopath::AbsoluteSystemPathBuf;
use wax::{Any, Glob, Pattern};

#[derive(Debug, PartialEq, Clone, Copy)]
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
    Exclude,
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
    #[error("bad pattern: {0}")]
    BadPattern(#[from] wax::BuildError),
    #[error("invalid path")]
    InvalidPath,
}

/// Performs a glob walk, yielding paths that _are_ included in the include list
/// (if it is nonempty) and _not_ included in the exclude list.
///
/// In the case of an empty include, then all files are included.
///
/// note: the rough algorithm to achieve this is as follows:
///       - prepend the slashified base_path to each include and exclude
///       - collapse the path, and calculate the new base_path, which defined as
///         the longest common prefix of all the includes
///       - traversing above the root of the base_path is not allowed
pub fn globwalk<'a>(
    base_path: &'a AbsoluteSystemPathBuf,
    include: &'a [String],
    exclude: &'a [String],
    walk_type: WalkType,
) -> Result<Vec<Result<AbsoluteSystemPathBuf, walkdir::Error>>, WalkError> {
    let (base_path_new, include_paths, exclude_paths) =
        preprocess_paths_and_globs(base_path, include, exclude)?;

    let inc_patterns = include_paths.iter().map(|g| g.as_ref());
    let include = InclusiveEmptyAny::new(inc_patterns)?;
    let ex_patterns = exclude_paths.iter().map(|g| g.as_ref());
    let exclude = wax::any(ex_patterns)?;

    // we enable following symlinks but only because without it they are ignored
    // completely (as opposed to yielded but not followed)
    let walker = walkdir::WalkDir::new(base_path_new.as_path()).follow_links(false);
    let mut iter = walker.into_iter();

    Ok(std::iter::from_fn(move || loop {
        let entry = iter.next()?;

        println!("{:?}", entry);

        let (is_symlink, path) = match entry {
            Ok(entry) => (entry.path_is_symlink(), entry.into_path()),
            Err(err) => match (err.io_error(), err.path()) {
                // make sure to yield broken symlinks
                (Some(io_err), Some(path))
                    if io_err.kind() == ErrorKind::NotFound && path.is_symlink() =>
                {
                    (true, path.to_owned())
                }
                _ => return Some(Err(err)),
            },
        };

        let relative_path = path.as_path(); // TODO
        let is_directory = !path.is_symlink() && path.is_dir();

        let match_type = do_match(relative_path, &include, &exclude);

        if (match_type == MatchType::Exclude || is_symlink) && is_directory {
            iter.skip_current_dir();
        }

        match match_type {
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
            MatchType::None | MatchType::PotentialMatch | MatchType::Match | MatchType::Exclude => {
            }
        }
    })
    .collect())
}

fn join_unix_like_paths(a: &str, b: &str) -> String {
    [a.trim_end_matches('/'), "/", b.trim_start_matches('/')].concat()
}

fn preprocess_paths_and_globs(
    base_path: &AbsoluteSystemPathBuf,
    include: &[String],
    exclude: &[String],
) -> Result<(PathBuf, Vec<String>, Vec<String>), WalkError> {
    let base_path_slash = base_path
        .as_path()
        .to_slash()
        .ok_or(WalkError::InvalidPath)?;
    let (include_paths, lowest_segment) = include
        .into_iter()
        .map(|s| join_unix_like_paths(&base_path_slash, s))
        .filter_map(|s| collapse_path(&s).map(|(s, v)| (s.to_string(), v)))
        .fold(
            (vec![], usize::MAX),
            |(mut vec, lowest_segment), (path, lowest_segment_next)| {
                let lowest_segment = std::cmp::min(lowest_segment, lowest_segment_next);
                vec.push(path.to_string()); // we stringify here due to lifetime issues
                (vec, lowest_segment)
            },
        );

    let base_path = base_path
        .components()
        .take(lowest_segment + 1)
        .collect::<PathBuf>();

    let exclude_paths = exclude
        .into_iter()
        .map(|s| join_unix_like_paths(&base_path_slash, s))
        .filter_map(|g| {
            let (split, _) = collapse_path(&g)?;
            let split = split.to_string();
            if split.ends_with('/') {
                Some(format!("{}**", split))
            } else {
                Some(split)
            }
        })
        .collect::<Vec<_>>();

    Ok((base_path, include_paths, exclude_paths))
}

fn do_match(path: &Path, include: &InclusiveEmptyAny, exclude: &Any) -> MatchType {
    let path_unix = match path.to_slash() {
        Some(path) => path,
        None => return MatchType::None, // you can't match a path that isn't valid unicode
    };

    let is_match = include.is_match(path_unix.as_ref());
    let is_match2 = exclude.is_match(path_unix.as_ref());
    match (is_match, is_match2) {
        (_, true) => MatchType::Exclude, // exclude takes precedence
        (true, false) => MatchType::Match,
        (false, false) => MatchType::None,
    }
}

/// collapse a path, returning a new path with all the dots and dotdots removed
///
/// also returns the position in the path of the first encountered collapse,
/// for the purposes of calculating the new base path
fn collapse_path(path: &str) -> Option<(Cow<str>, usize)> {
    let mut stack: Vec<&str> = vec![];
    let mut changed = false;
    let is_root = path.starts_with("/");

    // the index of the lowest segment that was collapsed
    // this is defined as the lowest stack size after a collapse
    let mut lowest_index = None;

    for segment in path.trim_start_matches('/').split('/') {
        match segment {
            ".." => {
                lowest_index.get_or_insert(stack.len());
                if let None = stack.pop() {
                    return None;
                }
                changed = true;
            }
            "." => {
                lowest_index.get_or_insert(stack.len());
                changed = true;
            }
            _ => stack.push(segment),
        }
        lowest_index.as_mut().map(|s| *s = stack.len().min(*s));
    }

    let lowest_index = lowest_index.unwrap_or(stack.len());
    if !changed {
        Some((Cow::Borrowed(path), lowest_index))
    } else {
        let string = if is_root {
            std::iter::once("").chain(stack.into_iter()).join("/")
        } else {
            stack.join("/")
        };

        Some((Cow::Owned(string), lowest_index))
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use itertools::Itertools;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;
    use wax::Glob;

    use crate::{collapse_path, empty_glob::InclusiveEmptyAny, MatchType, WalkError};

    #[test_case("a/./././b", "a/b", 1 ; "test path with dot segments")]
    #[test_case("a/../b", "b", 0 ; "test path with dotdot segments")]
    #[test_case("a/./../b", "b", 0 ; "test path with mixed dot and dotdot segments")]
    #[test_case("./a/b", "a/b", 0 ; "test path starting with dot segment")]
    #[test_case("a/b/..", "a", 1 ; "test path ending with dotdot segment")]
    #[test_case("a/b/.", "a/b", 2 ; "test path ending with dot segment")]
    #[test_case("a/.././b", "b", 0 ; "test path with mixed and consecutive ./ and ../ segments")]
    #[test_case("/a/./././b", "/a/b", 1 ; "test path with leading / and ./ segments")]
    #[test_case("/a/../b", "/b", 0 ; "test path with leading / and dotdot segments")]
    #[test_case("/a/./../b", "/b", 0 ; "test path with leading / and mixed dot and dotdot segments")]
    #[test_case("/./a/b", "/a/b", 0 ; "test path with leading / and starting with dot segment")]
    #[test_case("/a/b/..", "/a", 1 ; "test path with leading / and ending with dotdot segment")]
    #[test_case("/a/b/.", "/a/b", 2 ; "test path with leading / and ending with dot segment")]
    #[test_case("/a/.././b", "/b", 0 ; "test path with leading / and mixed and consecutive dot and dotdot segments")]
    #[test_case("/a/b/c/../../d/e/f/g/h/i/../j", "/a/d/e/f/g/h/j", 1 ; "leading collapse followed by shorter one")]
    fn test_collapse_path(glob: &str, expected: &str, earliest_collapsed_segement: usize) {
        let (glob, segment) = collapse_path(glob).unwrap();
        assert_eq!(glob, expected);
        assert_eq!(segment, earliest_collapsed_segement);
    }

    #[test_case("../a/b" ; "test path starting with ../ segment should return None")]
    #[test_case("/../a" ; "test path with leading dotdotdot segment should return None")]
    fn test_collapse_path_not(glob: &str) {
        assert_eq!(collapse_path(glob), None);
    }

    #[cfg(unix)]
    #[test_case("/a/b/c/d", &["/e/../../../f"], &[], "/a/b" ; "can traverse beyond the root")]
    #[test_case("/a/b/c/d/", &["/e/../../../f"], &[], "/a/b" ; "can handle slash-trailing base path")]
    #[test_case("/a/b/c/d/", &["e/../../../f"], &[], "/a/b" ; "can handle no slash on glob")]
    #[test_case("/a/b/c/d", &["e/../../../f"], &[], "/a/b" ; "can handle no slash on either")]
    #[test_case("/a/b/c/d", &["/e/f/../g"], &[], "/a/b/c/d" ; "can handle no collapse")]
    #[test_case("/a/b/c/d", &["./././../.."], &[], "/a/b" ; "can handle dot followed by dotdot")]
    fn preprocess_paths_and_globs(
        base_path: &str,
        include: &[&str],
        exclude: &[&str],
        expected: &str,
    ) {
        let base_path = AbsoluteSystemPathBuf::new(base_path).unwrap();
        let include = include.iter().map(|s| s.to_string()).collect_vec();
        let exclude = exclude.iter().map(|s| s.to_string()).collect_vec();

        let (base_expected, _, _) =
            super::preprocess_paths_and_globs(&base_path, &include, &exclude).unwrap();

        assert_eq!(base_expected.to_string_lossy(), expected);
    }

    #[cfg(unix)]
    #[test_case("/a/b/c", "dist/**", "dist/js/**")]
    fn exclude_prunes_subfolder(base_path: &str, include: &str, exclude: &str) {
        let base_path = AbsoluteSystemPathBuf::new(base_path).unwrap();
        let include = vec![include.to_string()];
        let exclude = vec![exclude.to_string()];

        let (_, include, exclude) =
            super::preprocess_paths_and_globs(&base_path, &include, &exclude).unwrap();

        let include_glob = InclusiveEmptyAny::new(include.iter().map(|s| s.as_ref())).unwrap();
        let exclude_glob = wax::any(exclude.iter().map(|s| s.as_ref())).unwrap();

        assert_eq!(
            super::do_match(
                Path::new("/a/b/c/dist/js/test.js"),
                &include_glob,
                &exclude_glob
            ),
            MatchType::Exclude
        );
    }

    #[test]
    fn do_match_empty_include() {
        let patterns: [&str; 0] = [];
        let any = wax::any(patterns).unwrap();
        let any_empty = InclusiveEmptyAny::new(patterns).unwrap();
        assert_eq!(
            super::do_match(Path::new("/a/b/c/d"), &any_empty, &any),
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
    // #[test_case("[a-b-d]", None, 3, 2 ; "dash within character class range match")]
    // #[test_case("[a-b-x]", None, 4, 3 ; "dash within character class range match 4")]
    // #[test_case("[", Some(WalkError::BadPattern("[".into())), 0, 0 ; "unclosed character class
    // error")] #[test_case("[^", Some(WalkError::BadPattern("[^".into())), 0, 0 ; "unclosed
    // negated character class error")] #[test_case("[^bc",
    // Some(WalkError::BadPattern("[^bc".into())), 0, 0 ; "unclosed negated character class error
    // 2")] #[test_case("a[", Some(WalkError::BadPattern("a[".into())), 0, 0 ; "unclosed
    // character class error after pattern")] glob watch will not error on this, since it does
    // not get far enough into the glob to see the error
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
    // #[test_case("ab{c,d}[", Some(WalkError::BadPattern("ab{c,d}[".into())), 0, 0)]
    // #[test_case("a{,bc}", None, 2, 2 ; "a followed by comma or b or c")]
    // #[test_case("a{,bc}", Some(WalkError::BadPattern("a{,bc}".into())), 0, 0 ; "a followed by
    // comma or b or c")]
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
    #[test_case("[x\\-]", None, 2 ; "escaped dash in character class match")]
    #[test_case("[\\-x]", None, 2 ; "escaped dash and character match")]
    // #[test_case("[-]", Some(WalkError::BadPattern("[-]".into())), 0 ; "bare dash in character
    // class match")] #[test_case("[x-]", Some(WalkError::BadPattern("[x-]".into())), 0 ;
    // "trailing dash in character class match 2")] #[test_case("[-x]",
    // Some(WalkError::BadPattern("[-x]".into())), 0 ; "leading dash in character class match 2")]
    // #[test_case("[a-b-d]", Some(WalkError::BadPattern("[a-b-d]".into())), 0 ; "dash within
    // character class range match 3")] #[test_case("\\",
    // Some(WalkError::BadPattern("\\".into())), 0 ; "single backslash error")]
    #[test_case("a/\\**", None, 0 ; "a followed by escaped double star and subdirectories mismatch")]
    #[test_case("a/\\[*\\]", None, 0 ; "a followed by escaped character class and pattern mismatch")]
    fn glob_walk_unix(pattern: &str, err_expected: Option<WalkError>, result_count: usize) {
        glob_walk_inner(pattern, err_expected, result_count)
    }

    fn glob_walk_inner(pattern: &str, err_expected: Option<WalkError>, result_count: usize) {
        let dir = setup();

        let path = AbsoluteSystemPathBuf::new(dir.path()).unwrap();
        let (success, error): (Vec<AbsoluteSystemPathBuf>, Vec<_>) =
            super::globwalk(&path, &[pattern.into()], &[], crate::WalkType::All)
                .unwrap()
                .into_iter()
                .partition_result();

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
            // "/repos/some-app/dist/js",
        ],
        &["/repos/some-app/dist/index.html",]
        ; "passing ** to exclude prevents capture of children")]
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
                super::globwalk(&path, &include, &exclude, walk_type)
                    .unwrap()
                    .into_iter()
                    .partition_result();

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
                "\n\n{:?}: expected \n{:#?} but got \n{:#?}",
                walk_type, expected, success
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
