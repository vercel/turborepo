// Allow unused_assignments for fields used by miette's Diagnostic derive macro
// These fields are accessed via the derive macro attributes (#[label],
// #[source_code], error message interpolation) but clippy doesn't recognize
// this usage pattern
#![allow(unused_assignments)]

use convert_case::{Case, Casing};
use miette::{Diagnostic, NamedSource, SourceSpan};
use turborepo_errors::TURBO_SITE;

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum MissingTaskError {
    #[error("Could not find task `{name}` in project")]
    MissingTaskDefinition {
        name: String,
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Could not find package `{name}` in project")]
    MissingPackage { name: String },
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Could not find \"{task_id}\" in root turbo.json or \"{task_name}\" in package")]
pub struct MissingPackageTaskError {
    #[label]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
    pub task_id: String,
    pub task_name: String,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Could not find package \"{package}\" referenced by task \"{task_id}\" in project")]
pub struct MissingPackageFromTaskError {
    #[label]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
    pub package: String,
    pub task_id: String,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Invalid task name: {reason}")]
pub struct InvalidTaskNameError {
    #[label]
    span: Option<SourceSpan>,
    #[source_code]
    text: NamedSource<String>,
    task_name: String,
    reason: String,
}

impl InvalidTaskNameError {
    /// Creates a new InvalidTaskNameError.
    pub fn new(
        span: Option<SourceSpan>,
        text: NamedSource<String>,
        task_name: String,
        reason: String,
    ) -> Self {
        Self {
            span,
            text,
            task_name,
            reason,
        }
    }
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error(
    "{task_id} requires an entry in turbo.json before it can be depended on because it is a task \
     declared in the root package.json"
)]
#[diagnostic(
    code(missing_root_task_in_turbo_json),
    url(
            "{}/messages/{}",
            TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab)
    )
)]
pub struct MissingRootTaskInTurboJsonError {
    pub task_id: String,
    #[label("Add an entry in turbo.json for this task")]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Cannot extend from '{package_name}' without a package 'turbo.json'.")]
pub struct MissingTurboJsonExtends {
    pub package_name: String,
    #[label("Extended from here")]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Cyclic \"extends\" detected: {}", cycle.join(" -> "))]
pub struct CyclicExtends {
    pub cycle: Vec<String>,
    #[label("Cycle detected here")]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
}
