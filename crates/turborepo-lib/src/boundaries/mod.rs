//! Package boundaries checking for turborepo.
//!
//! This module provides integration between turborepo-lib and the
//! turborepo-boundaries crate, implementing the necessary traits.

use std::collections::HashMap;

use turborepo_boundaries::{BoundariesChecker, BoundariesContext, TurboJsonProvider};
pub use turborepo_boundaries::{BoundariesConfig, BoundariesDiagnostic, BoundariesResult, Error};
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::PackageName;

use crate::run::Run;

/// Implementation of TurboJsonProvider for the Run context
pub struct RunTurboJsonProvider<'a> {
    run: &'a Run,
}

impl<'a> RunTurboJsonProvider<'a> {
    pub fn new(run: &'a Run) -> Self {
        Self { run }
    }
}

impl<'a> TurboJsonProvider for RunTurboJsonProvider<'a> {
    fn has_turbo_json(&self, pkg: &PackageName) -> bool {
        self.run.turbo_json_loader().load(pkg).is_ok()
    }

    fn boundaries_config(&self, pkg: &PackageName) -> Option<&BoundariesConfig> {
        self.run
            .turbo_json_loader()
            .load(pkg)
            .ok()
            .and_then(|turbo_json| turbo_json.boundaries.as_ref())
            .map(|spanned| spanned.as_inner())
    }

    fn package_tags(&self, pkg: &PackageName) -> Option<&Spanned<Vec<Spanned<String>>>> {
        self.run
            .turbo_json_loader()
            .load(pkg)
            .ok()
            .and_then(|turbo_json| turbo_json.tags.as_ref())
    }

    fn implicit_dependencies(&self, pkg: &PackageName) -> HashMap<String, Spanned<()>> {
        self.run
            .turbo_json_loader()
            .load(pkg)
            .ok()
            .and_then(|turbo_json| turbo_json.boundaries.as_ref())
            .map(|spanned| spanned.as_inner())
            .and_then(|boundaries| boundaries.implicit_dependencies.as_ref())
            .into_iter()
            .flatten()
            .flatten()
            .map(|dep| dep.clone().split())
            .collect::<HashMap<_, _>>()
    }
}

impl Run {
    /// Check package boundaries for all filtered packages
    pub async fn check_boundaries(&self, show_progress: bool) -> Result<BoundariesResult, Error> {
        let turbo_json_provider = RunTurboJsonProvider::new(self);
        let root_boundaries_config = self
            .root_turbo_json()
            .boundaries
            .as_ref()
            .map(|spanned| spanned.as_inner());
        let ctx = BoundariesContext {
            repo_root: self.repo_root(),
            pkg_dep_graph: self.pkg_dep_graph(),
            turbo_json_provider: &turbo_json_provider,
            root_boundaries_config,
            filtered_pkgs: self.filtered_pkgs(),
        };

        BoundariesChecker::check_boundaries(&ctx, show_progress).await
    }

    /// Patch a file with @boundaries-ignore comments
    pub fn patch_file(
        &self,
        file_path: &turbopath::AbsoluteSystemPath,
        file_patches: Vec<(miette::SourceSpan, String)>,
    ) -> Result<(), Error> {
        BoundariesChecker::patch_file(file_path, file_patches)
    }
}
