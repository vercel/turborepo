use std::{fmt, str::FromStr as _};

use globwalk::{ValidatedGlob, fix_glob_pattern};
use itertools::Itertools as _;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError};
use wax::{Any, Glob, Program as _};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid workspace glob {fixed}: {err}")]
    Glob {
        fixed: String,
        #[source]
        err: Box<wax::BuildError>,
    },
    #[error("Invalid globwalk pattern {0}")]
    Globwalk(#[from] globwalk::GlobError),
    #[error(transparent)]
    WalkError(#[from] globwalk::WalkError),
}

// WorkspaceGlobs is suitable for finding package.json files via globwalk
#[derive(Clone)]
pub struct WorkspaceGlobs {
    directory_inclusions: Any<'static>,
    directory_exclusions: Any<'static>,
    package_json_inclusions: Vec<ValidatedGlob>,
    pub raw_inclusions: Vec<String>,
    pub raw_exclusions: Vec<String>,
    validated_exclusions: Vec<ValidatedGlob>,
}

impl Error {
    pub fn invalid_glob(fixed: String, err: wax::BuildError) -> Self {
        Self::Glob {
            fixed,
            err: Box::new(err),
        }
    }
}

impl fmt::Debug for WorkspaceGlobs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceGlobs")
            .field("inclusions", &self.raw_inclusions)
            .field("exclusions", &self.raw_exclusions)
            .finish()
    }
}

impl PartialEq for WorkspaceGlobs {
    fn eq(&self, other: &Self) -> bool {
        // Use the literals for comparison, not the compiled globs
        self.raw_inclusions == other.raw_inclusions && self.raw_exclusions == other.raw_exclusions
    }
}

impl Eq for WorkspaceGlobs {}

fn glob_with_contextual_error<S: AsRef<str>>(raw: S) -> Result<Glob<'static>, Error> {
    let raw = raw.as_ref();
    let fixed = fix_glob_pattern(raw);
    Glob::new(&fixed)
        .map(|g| g.into_owned())
        .map_err(|e| Error::invalid_glob(fixed, e))
}

fn any_with_contextual_error(
    precompiled: Vec<Glob<'static>>,
    text: Vec<String>,
) -> Result<wax::Any<'static>, Error> {
    wax::any(precompiled).map_err(|e| {
        let text = text.iter().join(",");
        Error::invalid_glob(text, e)
    })
}

impl WorkspaceGlobs {
    pub fn new<S: Into<String>>(inclusions: Vec<S>, exclusions: Vec<S>) -> Result<Self, Error> {
        // take ownership of the inputs
        let raw_inclusions: Vec<String> = inclusions
            .into_iter()
            .map(|s| s.into())
            .collect::<Vec<String>>();
        let package_json_inclusions = raw_inclusions
            .iter()
            .map(|s| {
                let mut s: String = s.clone();
                if s.ends_with('/') {
                    s.push_str("package.json");
                } else {
                    s.push_str("/package.json");
                }
                ValidatedGlob::from_str(&s)
            })
            .collect::<Result<Vec<ValidatedGlob>, _>>()?;
        let raw_exclusions: Vec<String> = exclusions
            .into_iter()
            .map(|s| s.into())
            .collect::<Vec<String>>();
        let inclusion_globs = raw_inclusions
            .iter()
            .map(glob_with_contextual_error)
            .collect::<Result<Vec<_>, _>>()?;
        let exclusion_globs = raw_exclusions
            .iter()
            .map(glob_with_contextual_error)
            .collect::<Result<Vec<_>, _>>()?;
        let validated_exclusions = raw_exclusions
            .iter()
            .map(|e| ValidatedGlob::from_str(e))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            directory_inclusions: any_with_contextual_error(
                inclusion_globs,
                raw_inclusions.clone(),
            )?,
            directory_exclusions: any_with_contextual_error(
                exclusion_globs,
                raw_exclusions.clone(),
            )?,
            package_json_inclusions,
            validated_exclusions,
            raw_exclusions,
            raw_inclusions,
        })
    }

    /// Checks if the given `target` matches this `WorkspaceGlobs`.
    ///
    /// Errors:
    /// This function returns an Err if `root` is not a valid anchor for
    /// `target`
    pub fn target_is_workspace(
        &self,
        root: &AbsoluteSystemPath,
        target: &AbsoluteSystemPath,
    ) -> Result<bool, PathError> {
        let search_value = root.anchor(target)?;

        let includes = self.directory_inclusions.is_match(&search_value);
        let excludes = self.directory_exclusions.is_match(&search_value);

        Ok(includes && !excludes)
    }

    pub fn get_package_jsons(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<impl Iterator<Item = AbsoluteSystemPathBuf> + use<>, Error> {
        let files = globwalk::globwalk(
            repo_root,
            &self.package_json_inclusions,
            &self.validated_exclusions,
            globwalk::WalkType::Files,
        )?;
        Ok(files.into_iter())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_workspace_globs_trailing_slash() {
        let globs =
            WorkspaceGlobs::new(vec!["scripts/", "packages/**"], vec!["package/template"]).unwrap();
        assert_eq!(
            &globs
                .package_json_inclusions
                .iter()
                .map(|i| i.as_str())
                .collect::<Vec<_>>(),
            &["scripts/package.json", "packages/**/package.json"]
        );
    }
}
