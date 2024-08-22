use std::str::FromStr;

use regex::Regex;
use thiserror::Error;
use turbopath::AnchoredSystemPathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct GitRange {
    pub from_ref: Option<String>,
    pub to_ref: Option<String>,
    pub include_uncommitted: bool,
    // Allow unknown objects to be included in the range, without returning an error.
    // this is useful for shallow clones where objects may not exist.
    // When this happens, we assume that everything has changed.
    pub allow_unknown_objects: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct TargetSelector {
    pub include_dependencies: bool,
    pub match_dependencies: bool,
    pub include_dependents: bool,
    pub exclude: bool,
    pub exclude_self: bool,
    pub follow_prod_deps_only: bool,
    pub parent_dir: Option<AnchoredSystemPathBuf>,
    pub name_pattern: String,
    pub git_range: Option<GitRange>,
    pub raw: String,
}

impl FromStr for TargetSelector {
    type Err = InvalidSelectorError;

    fn from_str(raw_selector: &str) -> Result<Self, Self::Err> {
        let selector = raw_selector.strip_prefix('!');
        let (exclude, selector) = match selector {
            Some(selector) => (true, selector),
            None => (false, raw_selector),
        };

        let mut exclude_self = false;
        let include_dependencies = selector.strip_suffix("...");

        let (include_dependencies, selector) = if let Some(selector) = include_dependencies {
            (
                true,
                if let Some(selector) = selector.strip_suffix('^') {
                    exclude_self = true;
                    selector
                } else {
                    selector
                },
            )
        } else {
            (false, selector)
        };

        let include_dependents = selector.strip_prefix("...");
        let (include_dependents, selector) = if let Some(selector) = include_dependents {
            (
                true,
                if let Some(selector) = selector.strip_prefix('^') {
                    exclude_self = true;
                    selector
                } else {
                    selector
                },
            )
        } else {
            (false, selector)
        };

        // We explicitly allow empty git ranges so we can return a more targeted error
        // below
        let re = Regex::new(r"^(?P<name>[^.](?:[^{}\[\]]*[^{}\[\].])?)?(\{(?P<directory>[^}]*)})?(?P<commits>(?:\.{3})?\[[^\]]*\])?$").expect("valid");
        let captures = re.captures(selector);

        let captures = match captures {
            Some(captures) => captures,
            None => {
                return if let Some(relative_path) = is_selector_by_location(selector) {
                    Ok(TargetSelector {
                        exclude,
                        include_dependencies,
                        include_dependents,
                        parent_dir: Some(relative_path?),
                        raw: raw_selector.to_string(),
                        ..Default::default()
                    })
                } else {
                    Ok(TargetSelector {
                        exclude,
                        exclude_self,
                        include_dependencies,
                        include_dependents,
                        name_pattern: selector.to_string(),
                        raw: raw_selector.to_string(),
                        ..Default::default()
                    })
                }
            }
        };

        let mut pre_add_dependencies = false;

        let name_pattern = captures
            .name("name")
            .map_or(String::new(), |m| m.as_str().to_string());

        let mut parent_dir = None;

        if let Some(directory) = captures.name("directory") {
            let directory = directory.as_str().to_string();
            if directory.is_empty() {
                return Err(InvalidSelectorError::EmptyPathSpecification);
            } else {
                let clean_directory = path_clean::clean(std::path::Path::new(directory.as_str()))
                    .into_os_string()
                    .into_string()
                    .expect("directory was valid utf8 before cleaning");
                parent_dir = Some(
                    AnchoredSystemPathBuf::try_from(clean_directory.as_str())
                        .map_err(|_| InvalidSelectorError::InvalidAnchoredPath(directory))?,
                );
            }
        }

        let git_range = if let Some(commits) = captures.name("commits") {
            let commits_str = if let Some(commits) = commits.as_str().strip_prefix("...") {
                if parent_dir.is_none() && name_pattern.is_empty() {
                    return Err(InvalidSelectorError::CantMatchDependencies);
                }
                pre_add_dependencies = true;
                commits
            } else {
                commits.as_str()
            };

            // strip the square brackets
            let commits_str = commits_str
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .expect("regex guarantees square brackets");
            if commits_str.is_empty() {
                return Err(InvalidSelectorError::InvalidGitRange(
                    commits_str.to_string(),
                ));
            }

            let git_range = if let Some((a, b)) = commits_str.split_once("...") {
                if a.is_empty() || b.is_empty() {
                    return Err(InvalidSelectorError::InvalidGitRange(
                        commits_str.to_string(),
                    ));
                }
                GitRange {
                    from_ref: Some(a.to_string()),
                    to_ref: Some(b.to_string()),
                    include_uncommitted: false,
                    allow_unknown_objects: false,
                }
            } else {
                // If only the start of the range is specified, we assume that
                // we want to include uncommitted changes
                GitRange {
                    from_ref: Some(commits_str.to_string()),
                    to_ref: None,
                    include_uncommitted: true,
                    allow_unknown_objects: false,
                }
            };
            Some(git_range)
        } else {
            None
        };

        Ok(TargetSelector {
            git_range,
            exclude,
            exclude_self,
            include_dependencies,
            include_dependents,
            match_dependencies: pre_add_dependencies,
            name_pattern,
            parent_dir,
            raw: raw_selector.to_string(),
            ..Default::default()
        })
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum InvalidSelectorError {
    #[error("cannot use match dependencies without specifying either a directory or package")]
    CantMatchDependencies,
    #[error("invalid anchored path: {0}")]
    InvalidAnchoredPath(String),
    #[error("empty path specification")]
    EmptyPathSpecification,
    #[error("invalid git range selector: {0}")]
    InvalidGitRange(String),

    #[error("selector \"{0}\" must have a reference, directory, or name pattern")]
    InvalidSelector(String),
}

/// checks if the selector is a filesystem path
pub fn is_selector_by_location(
    raw_selector: &str,
) -> Option<Result<AnchoredSystemPathBuf, InvalidSelectorError>> {
    let exact_matches = [".", ".."];
    let prefixes = ["./", ".\\", "../", "..\\"];

    if exact_matches.contains(&raw_selector)
        || prefixes
            .iter()
            .any(|prefix| raw_selector.starts_with(prefix))
    {
        let cleaned_selector = path_clean::clean(std::path::Path::new(raw_selector))
            .into_os_string()
            .into_string()
            .expect("raw selector was valid utf8");
        Some(
            AnchoredSystemPathBuf::try_from(cleaned_selector.as_str())
                .map_err(|_| InvalidSelectorError::InvalidAnchoredPath(raw_selector.to_string())),
        )
    } else {
        None
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use test_case::test_case;
    use turbopath::AnchoredSystemPathBuf;

    use super::TargetSelector;
    use crate::run::scope::target_selector::GitRange;

    #[test_case("foo", TargetSelector { name_pattern: "foo".to_string(), raw: "foo".to_string(), ..Default::default() }; "foo")]
    #[test_case("foo...", TargetSelector { name_pattern: "foo".to_string(), raw: "foo...".to_string(), include_dependencies: true, ..Default::default() }; "foo dot dot dot")]
    #[test_case("...foo", TargetSelector { name_pattern: "foo".to_string(), raw: "...foo".to_string(), include_dependents: true, ..Default::default() }; "dot dot dot foo")]
    #[test_case("...foo...", TargetSelector { name_pattern: "foo".to_string(), raw: "...foo...".to_string(), include_dependents: true, include_dependencies: true, ..Default::default() }; "dot dot dot foo dot dot dot")]
    #[test_case("foo^...", TargetSelector { name_pattern: "foo".to_string(), raw: "foo^...".to_string(), include_dependencies: true, exclude_self: true, ..Default::default() }; "foo caret dot dot dot")]
    #[test_case("...^foo", TargetSelector { name_pattern: "foo".to_string(), raw: "...^foo".to_string(), include_dependents: true, exclude_self: true, ..Default::default() }; "dot dot dot caret foo")]
    #[test_case("../foo", TargetSelector { raw: "../foo".to_string(), parent_dir: Some(AnchoredSystemPathBuf::try_from(if cfg!(windows) { "..\\foo" } else { "../foo" }).unwrap()), ..Default::default() }; "dot dot slash foo")]
    #[test_case("./foo", TargetSelector { raw: "./foo".to_string(), parent_dir: Some(AnchoredSystemPathBuf::try_from("foo").unwrap()), ..Default::default() }; "dot slash foo")]
    #[test_case("./foo/*", TargetSelector { raw: "./foo/*".to_string(), parent_dir: Some(AnchoredSystemPathBuf::try_from(if cfg!(windows) { "foo\\*" } else { "foo/*" }).unwrap()), ..Default::default() }; "dot slash foo star")]
    #[test_case("...{./foo}", TargetSelector { raw: "...{./foo}".to_string(), parent_dir: Some(AnchoredSystemPathBuf::try_from("foo").unwrap()), include_dependents: true, ..Default::default() }; "dot dot dot curly bracket foo")]
    #[test_case(".", TargetSelector { raw: ".".to_string(), parent_dir: Some(AnchoredSystemPathBuf::try_from(".").unwrap()), ..Default::default() }; "parent dir dot")]
    #[test_case("..", TargetSelector { raw: "..".to_string(), parent_dir: Some(AnchoredSystemPathBuf::try_from("..").unwrap()), ..Default::default() }; "parent dir dot dot")]
    #[test_case("[master]", TargetSelector { raw: "[master]".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), ..Default::default() }; "square brackets master")]
    #[test_case("[from...to]", TargetSelector { raw: "[from...to]".to_string(), git_range: Some(GitRange { from_ref: Some("from".to_string()), to_ref: Some("to".to_string()), ..Default::default() }), ..Default::default() }; "[from...to]")]
    #[test_case("{foo}[master]", TargetSelector { raw: "{foo}[master]".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), parent_dir: Some(AnchoredSystemPathBuf::try_from("foo").unwrap()), ..Default::default() }; "{foo}[master]")]
    #[test_case("pattern{foo}[master]", TargetSelector { raw: "pattern{foo}[master]".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), parent_dir: Some(AnchoredSystemPathBuf::try_from("foo").unwrap()), name_pattern: "pattern".to_string(), ..Default::default() }; "pattern{foo}[master]")]
    #[test_case("[master]...", TargetSelector { raw: "[master]...".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), include_dependencies: true, ..Default::default() }; "square brackets master dot dot dot")]
    #[test_case("...[master]", TargetSelector { raw: "...[master]".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), include_dependents: true, ..Default::default() }; "dot dot dot master square brackets")]
    #[test_case("...[master]...", TargetSelector { raw: "...[master]...".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), include_dependencies: true, include_dependents: true, ..Default::default() }; "dot dot dot master square brackets dot dot dot")]
    #[test_case("...[from...to]...", TargetSelector { raw: "...[from...to]...".to_string(), git_range: Some(GitRange { from_ref: Some("from".to_string()), to_ref: Some("to".to_string()), ..Default::default() }), include_dependencies: true, include_dependents: true, ..Default::default() }; "dot dot dot [from...to] dot dot dot")]
    #[test_case("foo...[master]", TargetSelector { raw: "foo...[master]".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), name_pattern: "foo".to_string(), match_dependencies: true, ..Default::default() }; "foo...[master]")]
    #[test_case("foo...[master]...", TargetSelector { raw: "foo...[master]...".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), name_pattern: "foo".to_string(), match_dependencies: true, include_dependencies: true, ..Default::default() }; "foo...[master] dot dot dot")]
    #[test_case("{foo}...[master]", TargetSelector { raw: "{foo}...[master]".to_string(), git_range: Some(GitRange { from_ref: Some("master".to_string()), to_ref: None, include_uncommitted: true, ..Default::default() }), parent_dir: Some(AnchoredSystemPathBuf::try_from("foo").unwrap()), match_dependencies: true, ..Default::default() }; " curly brackets foo...[master]")]
    fn parse_target_selector(raw_selector: &str, want: TargetSelector) {
        let result = TargetSelector::from_str(raw_selector);

        match result {
            Ok(got) => {
                assert_eq!(
                    got, want,
                    "ParseTargetSelector() = {:?}, want {:?}",
                    got, want
                );
            }
            Err(e) => {
                panic!("ParseTargetSelector() error = {:?}", e)
            }
        }
    }

    #[test_case("{}" ; "curly brackets")]
    #[test_case("......[master]" ; "......[master]")]
    #[test_case("[]" ; "empty git range")]
    #[test_case("[...some-ref]" ; "missing git range start")]
    #[test_case("[some-ref...]" ; "missing git range end")]
    #[test_case("[...]" ; "missing entire git range")]
    fn parse_target_selector_invalid(raw_selector: &str) {
        let result = TargetSelector::from_str(raw_selector);

        match result {
            Ok(_got) => {
                panic!("expected error when parsing {}", raw_selector);
            }
            Err(e) => {
                println!("{:?}", e);
            }
        }
    }
}
