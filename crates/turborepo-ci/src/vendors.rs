use std::{collections::HashMap, fmt::Debug, sync::OnceLock};

use crate::vendor_behavior::VendorBehavior;

#[derive(Clone, Debug, PartialEq)]
pub struct VendorEnvs {
    pub(crate) any: Vec<&'static str>,
    pub(crate) all: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub struct Vendor {
    pub(crate) name: &'static str,
    pub constant: &'static str,
    pub(crate) env: VendorEnvs,
    pub(crate) eval_env: Option<HashMap<&'static str, &'static str>>,
    pub sha_env_var: Option<&'static str>,
    pub branch_env_var: Option<&'static str>,
    pub username_env_var: Option<&'static str>,
    pub behavior: Option<VendorBehavior>,
}

static VENDORS: OnceLock<[Vendor; 45]> = OnceLock::new();

pub(crate) fn get_vendors() -> &'static [Vendor] {
    VENDORS
        .get_or_init(|| {
            [
                Vendor {
                    name: "Appcircle",
                    constant: "APPCIRCLE",
                    env: VendorEnvs {
                        any: vec!["AC_APPCIRCLE"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "AppVeyor",
                    constant: "APPVEYOR",
                    env: VendorEnvs {
                        any: vec!["APPVEYOR"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "AWS CodeBuild",
                    constant: "CODEBUILD",
                    env: VendorEnvs {
                        any: vec!["CODEBUILD_BUILD_ARN"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Azure Pipelines",
                    constant: "AZURE_PIPELINES",
                    env: VendorEnvs {
                        any: vec!["SYSTEM_TEAMFOUNDATIONCOLLECTIONURI"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: Some(VendorBehavior::new(
                        |group_name| format!("##[group]{group_name}\r\n"),
                        |_| String::from("##[endgroup]\r\n"),
                    )),
                },
                Vendor {
                    name: "Bamboo",
                    constant: "BAMBOO",
                    env: VendorEnvs {
                        any: vec!["bamboo_planKey"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Bitbucket Pipelines",
                    constant: "BITBUCKET",
                    env: VendorEnvs {
                        any: vec!["BITBUCKET_COMMIT"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Bitrise",
                    constant: "BITRISE",
                    env: VendorEnvs {
                        any: vec!["BITRISE_IO"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Buddy",
                    constant: "BUDDY",
                    env: VendorEnvs {
                        any: vec!["BUDDY_WORKSPACE_ID"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Buildkite",
                    constant: "BUILDKITE",
                    env: VendorEnvs {
                        any: vec!["BUILDKITE"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "CircleCI",
                    constant: "CIRCLE",
                    env: VendorEnvs {
                        any: vec!["CIRCLECI"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Cirrus CI",
                    constant: "CIRRUS",
                    env: VendorEnvs {
                        any: vec!["CIRRUS_CI"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Codefresh",
                    constant: "CODEFRESH",
                    env: VendorEnvs {
                        any: vec!["CF_BUILD_ID"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Codemagic",
                    constant: "CODEMAGIC",
                    env: VendorEnvs {
                        any: vec!["CM_BUILD_ID"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Codeship",
                    constant: "CODESHIP",
                    env: VendorEnvs {
                        any: vec![],
                        all: vec![],
                    },
                    eval_env: Some({
                        let mut map = HashMap::new();
                        map.insert("CI_NAME", "codeship");
                        map
                    }),
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Drone",
                    constant: "DRONE",
                    env: VendorEnvs {
                        any: vec!["DRONE"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "dsari",
                    constant: "DSARI",
                    env: VendorEnvs {
                        any: vec!["DSARI"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Expo Application Services",
                    constant: "EAS",
                    env: VendorEnvs {
                        any: vec!["EAS_BUILD"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "GitHub Actions",
                    constant: "GITHUB_ACTIONS",
                    env: VendorEnvs {
                        any: vec!["GITHUB_ACTIONS"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: Some("GITHUB_SHA"),
                    branch_env_var: Some("GITHUB_REF_NAME"),
                    username_env_var: Some("GITHUB_ACTOR"),
                    behavior: Some(
                        VendorBehavior::new(
                            |group_name| format!("::group::{group_name}\n"),
                            |_| String::from("::endgroup::\n"),
                        )
                        .with_error(
                            |group_name| format!("\x1B[;31m{group_name}\x1B[;0m\n"),
                            |_| String::new(),
                        ),
                    ),
                },
                Vendor {
                    name: "GitLab CI",
                    constant: "GITLAB",
                    env: VendorEnvs {
                        any: vec!["GITLAB_CI"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "GoCD",
                    constant: "GOCD",
                    env: VendorEnvs {
                        any: vec!["GO_PIPELINE_LABEL"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Google Cloud Build",
                    constant: "GOOGLE_CLOUD_BUILD",
                    env: VendorEnvs {
                        any: vec!["BUILDER_OUTPUT"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "LayerCI",
                    constant: "LAYERCI",
                    env: VendorEnvs {
                        any: vec!["LAYERCI"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Gerrit",
                    constant: "GERRIT",
                    env: VendorEnvs {
                        any: vec!["GERRIT_PROJECT"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Hudson",
                    constant: "HUDSON",
                    env: VendorEnvs {
                        any: vec!["HUDSON"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Jenkins",
                    constant: "JENKINS",
                    env: VendorEnvs {
                        any: vec![],
                        all: vec!["JENKINS_URL", "BUILD_ID"],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Magnum CI",
                    constant: "MAGNUM",
                    env: VendorEnvs {
                        any: vec!["MAGNUM"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Netlify CI",
                    constant: "NETLIFY",
                    env: VendorEnvs {
                        any: vec!["NETLIFY"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Nevercode",
                    constant: "NEVERCODE",
                    env: VendorEnvs {
                        any: vec!["NEVERCODE"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "ReleaseHub",
                    constant: "RELEASEHUB",
                    env: VendorEnvs {
                        any: vec!["RELEASE_BUILD_ID"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Render",
                    constant: "RENDER",
                    env: VendorEnvs {
                        any: vec!["RENDER"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Sail CI",
                    constant: "SAIL",
                    env: VendorEnvs {
                        any: vec!["SAILCI"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Screwdriver",
                    constant: "SCREWDRIVER",
                    env: VendorEnvs {
                        any: vec!["SCREWDRIVER"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Semaphore",
                    constant: "SEMAPHORE",
                    env: VendorEnvs {
                        any: vec!["SEMAPHORE"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Shippable",
                    constant: "SHIPPABLE",
                    env: VendorEnvs {
                        any: vec!["SHIPPABLE"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Solano CI",
                    constant: "SOLANO",
                    env: VendorEnvs {
                        any: vec!["TDDIUM"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Sourcehut",
                    constant: "SOURCEHUT",
                    env: VendorEnvs {
                        any: vec![],
                        all: vec![],
                    },
                    eval_env: Some({
                        let mut map = HashMap::new();
                        map.insert("CI_NAME", "sourcehut");
                        map
                    }),
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Strider CD",
                    constant: "STRIDER",
                    env: VendorEnvs {
                        any: vec!["STRIDER"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "TaskCluster",
                    constant: "TASKCLUSTER",
                    env: VendorEnvs {
                        any: vec!["TASK_ID", "RUN_ID"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "TeamCity",
                    constant: "TEAMCITY",
                    env: VendorEnvs {
                        any: vec!["TEAMCITY_VERSION"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: Some(VendorBehavior::new(
                        |group_name| format!("##teamcity[blockOpened name='{group_name}']"),
                        |group_name| format!("##teamcity[blockClosed name='{group_name}']"),
                    )),
                },
                Vendor {
                    name: "Travis CI",
                    constant: "TRAVIS",
                    env: VendorEnvs {
                        any: vec!["TRAVIS"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: Some(VendorBehavior::new(
                        |group_name| format!("travis_fold:start:{group_name}\r\n"),
                        |group_name| format!("travis_fold:end:{group_name}\r\n"),
                    )),
                },
                Vendor {
                    name: "Vercel",
                    constant: "VERCEL",
                    env: VendorEnvs {
                        any: vec!["NOW_BUILDER", "VERCEL"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: Some("VERCEL_GIT_COMMIT_SHA"),
                    branch_env_var: Some("VERCEL_GIT_COMMIT_REF"),
                    username_env_var: Some("VERCEL_GIT_COMMIT_AUTHOR_LOGIN"),
                    behavior: None,
                },
                Vendor {
                    name: "Visual Studio App Center",
                    constant: "APPCENTER",
                    env: VendorEnvs {
                        any: vec!["APPCENTER"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Woodpecker",
                    constant: "WOODPECKER",
                    env: VendorEnvs {
                        any: vec![],
                        all: vec![],
                    },
                    eval_env: Some({
                        let mut map = HashMap::new();
                        map.insert("CI", "woodpecker");
                        map
                    }),
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Xcode Cloud",
                    constant: "XCODE_CLOUD",
                    env: VendorEnvs {
                        any: vec!["CI_XCODE_PROJECT"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
                Vendor {
                    name: "Xcode Server",
                    constant: "XCODE_SERVER",
                    env: VendorEnvs {
                        any: vec!["XCS"],
                        all: vec![],
                    },
                    eval_env: None,
                    sha_env_var: None,
                    branch_env_var: None,
                    username_env_var: None,
                    behavior: None,
                },
            ]
        })
        .as_slice()
}
