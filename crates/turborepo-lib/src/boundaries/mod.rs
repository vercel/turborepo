//! Package boundaries checking for turborepo.
//!
//! This module provides integration between turborepo-lib and the
//! turborepo-boundaries crate, implementing the necessary traits.

use std::collections::HashMap;

pub use turborepo_boundaries::{BoundariesConfig, Error};
use turborepo_boundaries::TurboJsonProvider;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::PackageName;

use crate::turbo_json::UnifiedTurboJsonLoader;

pub struct RunTurboJsonProvider<'a> {
    turbo_json_loader: &'a UnifiedTurboJsonLoader,
}

impl<'a> RunTurboJsonProvider<'a> {
    pub fn new(turbo_json_loader: &'a UnifiedTurboJsonLoader) -> Self {
        Self { turbo_json_loader }
    }
}

impl<'a> TurboJsonProvider for RunTurboJsonProvider<'a> {
    fn has_turbo_json(&self, pkg: &PackageName) -> bool {
        self.turbo_json_loader.load(pkg).is_ok()
    }

    fn boundaries_config(&self, pkg: &PackageName) -> Option<&BoundariesConfig> {
        self.turbo_json_loader
            .load(pkg)
            .ok()
            .and_then(|turbo_json| turbo_json.boundaries.as_ref())
            .map(|spanned| spanned.as_inner())
    }

    fn package_tags(&self, pkg: &PackageName) -> Option<&Spanned<Vec<Spanned<String>>>> {
        self.turbo_json_loader
            .load(pkg)
            .ok()
            .and_then(|turbo_json| turbo_json.tags.as_ref())
    }

    fn implicit_dependencies(&self, pkg: &PackageName) -> HashMap<String, Spanned<()>> {
        self.turbo_json_loader
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
