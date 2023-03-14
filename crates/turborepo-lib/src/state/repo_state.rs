use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::local_turbo_state::LocalTurboState;
use crate::package_manager::{Globs, PackageManager};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoState {
    pub root: PathBuf,
    pub mode: RepoMode,
    pub local_turbo_state: Option<LocalTurboState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RepoMode {
    SinglePackage,
    MultiPackage,
}

#[derive(Debug)]
struct InferInfo {
    path: PathBuf,
    has_package_json: bool,
    has_turbo_json: bool,
    workspace_globs: Option<Globs>,
}

impl InferInfo {
    pub fn has_package_json(info: &'_ &InferInfo) -> bool {
        info.has_package_json
    }
    pub fn has_turbo_json(info: &'_ &InferInfo) -> bool {
        info.has_turbo_json
    }

    pub fn is_workspace_root_of(&self, target_path: &Path) -> bool {
        match &self.workspace_globs {
            Some(globs) => globs
                .test(self.path.to_path_buf(), target_path.to_path_buf())
                .unwrap_or(false),
            None => false,
        }
    }
}

impl RepoState {
    fn generate_potential_turbo_roots(reference_dir: &Path) -> Vec<InferInfo> {
        // Find all directories that contain a `package.json` or a `turbo.json`.
        // Gather a bit of additional metadata about them.
        let potential_turbo_roots = reference_dir
            .ancestors()
            .filter_map(|path| {
                let has_package_json = fs::metadata(path.join("package.json")).is_ok();
                let has_turbo_json = fs::metadata(path.join("turbo.json")).is_ok();

                if !has_package_json && !has_turbo_json {
                    return None;
                }

                // FIXME: This should be based upon detecting the pacakage manager.
                // However, we don't have that functionality implemented in Rust yet.
                // PackageManager::detect(path).get_workspace_globs().unwrap_or(None)
                let workspace_globs = PackageManager::Pnpm
                    .get_workspace_globs(path)
                    .unwrap_or_else(|_| {
                        PackageManager::Npm
                            .get_workspace_globs(path)
                            .unwrap_or(None)
                    });

                Some(InferInfo {
                    path: path.to_owned(),
                    has_package_json,
                    has_turbo_json,
                    workspace_globs,
                })
            })
            .collect();

        potential_turbo_roots
    }

    fn process_potential_turbo_roots(potential_turbo_roots: Vec<InferInfo>) -> Result<Self> {
        // Potential improvements:
        // - Detect invalid configuration where turbo.json isn't peer to package.json.
        // - There are a couple of possible early exits to prevent traversing all the
        //   way to root at significant code complexity increase.
        //
        //   1. [0].has_turbo_json && [0].workspace_globs.is_some()
        //   2. [0].has_turbo_json && [n].has_turbo_json && [n].is_workspace_root_of(0)
        //
        // If we elect to make any of the changes for early exits we need to expand test
        // suite which presently relies on the fact that the selection runs in a loop to
        // avoid creating those test cases.

        // We need to perform the same search strategy for _both_ turbo.json and _then_
        // package.json.
        let search_locations = [InferInfo::has_turbo_json, InferInfo::has_package_json];

        for check_set_comparator in search_locations {
            let mut check_roots = potential_turbo_roots
                .iter()
                .filter(check_set_comparator)
                .peekable();

            let current_option = check_roots.next();

            // No potential roots checking by this comparator.
            if current_option.is_none() {
                continue;
            }

            let current = current_option.unwrap();

            // If there is only one potential root, that's the winner.
            if check_roots.peek().is_none() {
                let local_turbo_state = LocalTurboState::infer(&current.path);
                return Ok(Self {
                    root: current.path.to_path_buf(),
                    mode: if current.workspace_globs.is_some() {
                        RepoMode::MultiPackage
                    } else {
                        RepoMode::SinglePackage
                    },
                    local_turbo_state,
                });

            // More than one potential root. See if we can stop at the first.
            // This is a performance optimization. We could remove this case,
            // and set the mode properly in the else and it would still work.
            } else if current.workspace_globs.is_some() {
                // If the closest one has workspaces then we stop there.
                let local_turbo_state = LocalTurboState::infer(&current.path);
                return Ok(Self {
                    root: current.path.to_path_buf(),
                    mode: RepoMode::MultiPackage,
                    local_turbo_state,
                });

            // More than one potential root.
            // Closest is not RepoMode::MultiPackage
            // We attempt to prove that the closest is a workspace of a parent.
            // Failing that we just choose the closest.
            } else {
                for ancestor_infer in check_roots {
                    if ancestor_infer.is_workspace_root_of(&current.path) {
                        let local_turbo_state = LocalTurboState::infer(&ancestor_infer.path);
                        return Ok(Self {
                            root: ancestor_infer.path.to_path_buf(),
                            mode: RepoMode::MultiPackage,
                            local_turbo_state,
                        });
                    }
                }

                // We have eliminated RepoMode::MultiPackage as an option.
                // We must exhaustively check before this becomes the answer.
                let local_turbo_state = LocalTurboState::infer(&current.path);
                return Ok(Self {
                    root: current.path.to_path_buf(),
                    mode: RepoMode::SinglePackage,
                    local_turbo_state,
                });
            }
        }

        // If we're here we didn't find a valid root.
        Err(anyhow!("Root could not be inferred."))
    }

    /// Infers `RepoState` from current directory.
    ///
    /// # Arguments
    ///
    /// * `current_dir`: Current working directory
    ///
    /// returns: Result<RepoState, Error>
    pub fn infer(reference_dir: &Path) -> Result<Self> {
        let potential_turbo_roots = RepoState::generate_potential_turbo_roots(reference_dir);
        RepoState::process_potential_turbo_roots(potential_turbo_roots)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_process_potential_turbo_roots() {
        struct TestCase {
            description: &'static str,
            infer_infos: Vec<InferInfo>,
            output: Result<PathBuf>,
        }

        let tests = [
            // Test for zero, exhaustive.
            TestCase {
                description: "No matches found.",
                infer_infos: vec![],
                output: Err(anyhow!("Root could not be inferred.")),
            },
            // Test for one, exhaustive.
            TestCase {
                description: "Only one, is monorepo with turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: true,
                    workspace_globs: Some(Globs {
                        inclusions: vec!["packages/*".to_string()],
                        exclusions: vec![],
                    }),
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Only one, is non-monorepo with turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: true,
                    workspace_globs: None,
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Only one, is monorepo without turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: false,
                    workspace_globs: Some(Globs {
                        inclusions: vec!["packages/*".to_string()],
                        exclusions: vec![],
                    }),
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Only one, is non-monorepo without turbo.json.",
                infer_infos: vec![InferInfo {
                    path: PathBuf::from("/path/to/root"),
                    has_package_json: true,
                    has_turbo_json: false,
                    workspace_globs: None,
                }],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            // Tests for how to choose what is closest.
            TestCase {
                description: "Execution in a workspace.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root/packages/ui-library"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Execution in a workspace, weird package layout.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root/packages/ui-library/css"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root/packages/ui-library"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            // This `**` is important:
                            inclusions: vec!["packages/**".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root")),
            },
            TestCase {
                description: "Nested disjoint monorepo roots.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one/root-two")),
            },
            TestCase {
                description: "Nested disjoint monorepo roots, execution in a workspace of the \
                              closer root.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from(
                            "/path/to/root-one/root-two/root-two-packages/ui-library",
                        ),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two/root-one-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one/root-two")),
            },
            TestCase {
                description: "Nested disjoint monorepo roots, execution in a workspace of the \
                              farther root.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from(
                            "/path/to/root-one/root-two/root-one-packages/ui-library",
                        ),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two/root-one-packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one")),
            },
            TestCase {
                description: "Disjoint package.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root/some-other-project"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root/some-other-project")),
            },
            TestCase {
                description: "Monorepo trying to point to a monorepo. We choose the closer one \
                              and ignore the problem.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one/root-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["packages/*".to_string()],
                            exclusions: vec![],
                        }),
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/root-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: Some(Globs {
                            inclusions: vec!["root-two".to_string()],
                            exclusions: vec![],
                        }),
                    },
                ],
                output: Ok(PathBuf::from("/path/to/root-one/root-two")),
            },
            TestCase {
                description: "Nested non-monorepo packages.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one/project-two"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                ],
                output: Ok(PathBuf::from("/path/to/project-one/project-two")),
            },
            // The below test ensures that we privilege a valid `turbo.json` structure prior to
            // evaluation of a valid `package.json` structure. If you include `turbo.json` you are
            // able to "skip" deeper into the resolution by disregarding anything that does _not_
            // have a `turbo.json`. This will matter _far_ more in a multi-language environment.

            // Just one example test proves that the entire alternative chain construction works.
            // The selection logic from within this set is identical. If we attempt to optimize the
            // number of file system reads by early-exiting for matching we should expand this test
            // set to mirror the above section.
            TestCase {
                description: "Nested non-monorepo packages, turbo.json primacy.",
                infer_infos: vec![
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one/project-two"),
                        has_package_json: true,
                        has_turbo_json: false,
                        workspace_globs: None,
                    },
                    InferInfo {
                        path: PathBuf::from("/path/to/project-one"),
                        has_package_json: true,
                        has_turbo_json: true,
                        workspace_globs: None,
                    },
                ],
                output: Ok(PathBuf::from("/path/to/project-one")),
            },
        ];

        for test in tests {
            match RepoState::process_potential_turbo_roots(test.infer_infos) {
                Ok(repo_state) => assert_eq!(
                    repo_state.root,
                    test.output.unwrap(),
                    "{}",
                    test.description
                ),
                Err(err) => assert_eq!(
                    err.to_string(),
                    test.output.unwrap_err().to_string(),
                    "{}",
                    test.description
                ),
            };
        }
    }
}
