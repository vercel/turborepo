use std::str::FromStr;

use regex::Regex;
use thiserror::Error;
use turbopath::AnchoredSystemPathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct TargetSelector {
    pub include_dependencies: bool,
    pub match_dependencies: bool,
    pub include_dependents: bool,
    pub exclude: bool,
    pub exclude_self: bool,
    pub follow_prod_deps_only: bool,
    pub parent_dir: AnchoredSystemPathBuf,
    pub name_pattern: String,
    pub from_ref: String,
    pub to_ref_override: String,
    pub raw: String,
}

impl TargetSelector {
    pub fn to_ref(&self) -> &str {
        if self.to_ref_override.is_empty() {
            "HEAD"
        } else {
            &self.to_ref_override
        }
    }

    #[allow(dead_code)]
    pub fn is_valid(&self) -> bool {
        !self.from_ref.is_empty()
            || self.parent_dir != AnchoredSystemPathBuf::default()
            || !self.name_pattern.is_empty()
    }
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

        let re = Regex::new(r"^(?P<name>[^.](?:[^{}\[\]]*[^{}\[\].])?)?(\{(?P<directory>[^}]*)})?(?P<commits>(?:\.{3})?\[[^\]]+\])?$").expect("valid");
        let captures = re.captures(selector);

        let captures = match captures {
            Some(captures) => captures,
            None => {
                return if let Some(relative_path) = is_selector_by_location(selector) {
                    Ok(TargetSelector {
                        exclude,
                        include_dependencies,
                        include_dependents,
                        parent_dir: relative_path?,
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

        let mut parent_dir = AnchoredSystemPathBuf::default();

        if let Some(directory) = captures.name("directory") {
            let directory = directory.as_str().to_string();
            if directory.is_empty() {
                return Err(InvalidSelectorError::EmptyPathSpecification);
            } else {
                parent_dir = AnchoredSystemPathBuf::try_from(directory.as_str())
                    .map_err(|_| InvalidSelectorError::InvalidAnchoredPath(directory))?;
            }
        }

        let (from_ref, to_ref_override) = if let Some(commits) = captures.name("commits") {
            let commits_str = if let Some(commits) = commits.as_str().strip_prefix("...") {
                if parent_dir == AnchoredSystemPathBuf::default() && name_pattern.is_empty() {
                    return Err(InvalidSelectorError::CantMatchDependencies);
                }
                pre_add_dependencies = true;
                commits
            } else {
                commits.as_str()
            };

            // strip the square brackets
            let inner_str = commits_str
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'));

            if let Some(commits_str) = inner_str {
                if let Some((a, b)) = commits_str.split_once("...") {
                    (a.to_string(), b.to_string())
                } else {
                    (commits_str.to_string(), String::new())
                }
            } else {
                (commits_str.to_string(), String::new())
            }
        } else {
            Default::default()
        };

        Ok(TargetSelector {
            from_ref,
            to_ref_override,
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
        Some(
            AnchoredSystemPathBuf::try_from(raw_selector)
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

    #[test_case("foo", TargetSelector { name_pattern: "foo".to_string(), raw: "foo".to_string(), ..Default::default() }; "foo")]
    #[test_case("foo...", TargetSelector { name_pattern: "foo".to_string(), raw: "foo...".to_string(), include_dependencies: true, ..Default::default() }; "foo dot dot dot")]
    #[test_case("...foo", TargetSelector { name_pattern: "foo".to_string(), raw: "...foo".to_string(), include_dependents: true, ..Default::default() }; "dot dot dot foo")]
    #[test_case("...foo...", TargetSelector { name_pattern: "foo".to_string(), raw: "...foo...".to_string(), include_dependents: true, include_dependencies: true, ..Default::default() }; "dot dot dot foo dot dot dot")]
    #[test_case("foo^...", TargetSelector { name_pattern: "foo".to_string(), raw: "foo^...".to_string(), include_dependencies: true, exclude_self: true, ..Default::default() }; "foo caret dot dot dot")]
    #[test_case("...^foo", TargetSelector { name_pattern: "foo".to_string(), raw: "...^foo".to_string(), include_dependents: true, exclude_self: true, ..Default::default() }; "dot dot dot caret foo")]
    #[test_case("./foo", TargetSelector { raw: "./foo".to_string(), parent_dir: AnchoredSystemPathBuf::try_from("./foo").unwrap(), ..Default::default() }; "dot slash foo")]
    #[test_case("../foo", TargetSelector { raw: "../foo".to_string(), parent_dir: AnchoredSystemPathBuf::try_from("../foo").unwrap(), ..Default::default() }; "dot dot slash foo")]
    #[test_case("...{./foo}", TargetSelector { raw: "...{./foo}".to_string(), parent_dir: AnchoredSystemPathBuf::try_from("./foo").unwrap(), include_dependents: true, ..Default::default() }; "dot dot dot curly bracket foo")]
    #[test_case(".", TargetSelector { raw: ".".to_string(), parent_dir: AnchoredSystemPathBuf::try_from(".").unwrap(), ..Default::default() }; "parent dir dot")]
    #[test_case("..", TargetSelector { raw: "..".to_string(), parent_dir: AnchoredSystemPathBuf::try_from("..").unwrap(), ..Default::default() }; "parent dir dot dot")]
    #[test_case("[master]", TargetSelector { raw: "[master]".to_string(), from_ref: "master".to_string(), ..Default::default() }; "square brackets master")]
    #[test_case("[from...to]", TargetSelector { raw: "[from...to]".to_string(), from_ref: "from".to_string(), to_ref_override: "to".to_string(), ..Default::default() }; "[from...to]")]
    #[test_case("{foo}[master]", TargetSelector { raw: "{foo}[master]".to_string(), from_ref: "master".to_string(), parent_dir: AnchoredSystemPathBuf::try_from("foo").unwrap(), ..Default::default() }; "{foo}[master]")]
    #[test_case("pattern{foo}[master]", TargetSelector { raw: "pattern{foo}[master]".to_string(), from_ref: "master".to_string(), parent_dir: AnchoredSystemPathBuf::try_from("foo").unwrap(), name_pattern: "pattern".to_string(), ..Default::default() }; "pattern{foo}[master]")]
    #[test_case("[master]...", TargetSelector { raw: "[master]...".to_string(), from_ref: "master".to_string(), include_dependencies: true, ..Default::default() }; "square brackets master dot dot dot")]
    #[test_case("...[master]", TargetSelector { raw: "...[master]".to_string(), from_ref: "master".to_string(), include_dependents: true, ..Default::default() }; "dot dot dot master square brackets")]
    #[test_case("...[master]...", TargetSelector { raw: "...[master]...".to_string(), from_ref: "master".to_string(), include_dependencies: true, include_dependents: true, ..Default::default() }; "dot dot dot master square brackets dot dot dot")]
    #[test_case("...[from...to]...", TargetSelector { raw: "...[from...to]...".to_string(), from_ref: "from".to_string(), to_ref_override: "to".to_string(), include_dependencies: true, include_dependents: true, ..Default::default() }; "dot dot dot [from...to] dot dot dot")]
    #[test_case("foo...[master]", TargetSelector { raw: "foo...[master]".to_string(), from_ref: "master".to_string(), name_pattern: "foo".to_string(), match_dependencies: true, ..Default::default() }; "foo...[master]")]
    #[test_case("foo...[master]...", TargetSelector { raw: "foo...[master]...".to_string(), from_ref: "master".to_string(), name_pattern: "foo".to_string(), match_dependencies: true, include_dependencies: true, ..Default::default() }; "foo...[master] dot dot dot")]
    #[test_case("{foo}...[master]", TargetSelector { raw: "{foo}...[master]".to_string(), from_ref: "master".to_string(), parent_dir: AnchoredSystemPathBuf::try_from("foo").unwrap(), match_dependencies: true, ..Default::default() }; "curly brackets foo...[master]")]
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
