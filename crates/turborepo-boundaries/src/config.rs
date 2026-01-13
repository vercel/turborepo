use std::{collections::HashMap, sync::Arc};

use biome_deserialize_macros::Deserializable;
use schemars::JsonSchema;
use serde::Serialize;
use struct_iterable::Iterable;
use ts_rs::TS;
use turborepo_errors::{Spanned, WithMetadata};

/// Configuration for `turbo boundaries`.
///
/// Allows users to restrict a package's dependencies and dependents.
#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[schemars(rename_all = "camelCase")]
#[ts(export)]
pub struct BoundariesConfig {
    /// The boundaries rules for tags.
    ///
    /// Restricts which packages can import a tag and which packages a tag can
    /// import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Spanned<RulesMap>>,

    /// Declares any implicit dependencies, i.e. any dependency not declared in
    /// a `package.json`.
    ///
    /// These can include dependencies automatically injected by a framework or
    /// a testing library.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_dependencies: Option<Spanned<Vec<Spanned<String>>>>,

    /// Rules for a package's dependencies.
    ///
    /// Restricts which packages this package can import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Spanned<Permissions>>,

    /// Rules for a package's dependents.
    ///
    /// Restricts which packages can import this package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependents: Option<Spanned<Permissions>>,
}

/// A map of tag names to their boundary rules.
pub type RulesMap = HashMap<String, Spanned<Rule>>;

/// Boundary rules for a tag.
///
/// Restricts which packages a tag can import and which packages can import this
/// tag.
#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq, JsonSchema, TS)]
#[schemars(rename = "TagRules")]
#[ts(export, rename = "TagRules")]
pub struct Rule {
    /// Rules for a tag's dependencies.
    ///
    /// Restricts which packages a tag can import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Spanned<Permissions>>,

    /// Rules for a tag's dependents.
    ///
    /// Restricts which packages can import this tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependents: Option<Spanned<Permissions>>,
}

/// Permission rules for boundaries.
#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq, JsonSchema, TS)]
#[ts(export)]
pub struct Permissions {
    /// Lists which tags are allowed.
    ///
    /// Any tag not included will be banned. If omitted, all tags are permitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<Spanned<Vec<Spanned<String>>>>,

    /// Lists which tags are banned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny: Option<Spanned<Vec<Spanned<String>>>>,
}

impl WithMetadata for BoundariesConfig {
    fn add_text(&mut self, text: Arc<str>) {
        self.tags.add_text(text.clone());
        if let Some(tags) = &mut self.tags {
            for rule in tags.as_inner_mut().values_mut() {
                rule.add_text(text.clone());
                rule.value.add_text(text.clone());
            }
        }
        self.implicit_dependencies.add_text(text.clone());
        if let Some(implicit_dependencies) = &mut self.implicit_dependencies {
            for dep in implicit_dependencies.as_inner_mut() {
                dep.add_text(text.clone());
            }
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.tags.add_path(path.clone());
        if let Some(tags) = &mut self.tags {
            for rule in tags.as_inner_mut().values_mut() {
                rule.add_path(path.clone());
                rule.value.add_path(path.clone());
            }
        }
        self.implicit_dependencies.add_path(path.clone());
        if let Some(implicit_dependencies) = &mut self.implicit_dependencies {
            for dep in implicit_dependencies.as_inner_mut() {
                dep.add_path(path.clone());
            }
        }
    }
}

impl WithMetadata for Rule {
    fn add_text(&mut self, text: Arc<str>) {
        self.dependencies.add_text(text.clone());
        if let Some(dependencies) = &mut self.dependencies {
            dependencies.value.add_text(text.clone());
        }

        self.dependents.add_text(text.clone());
        if let Some(dependents) = &mut self.dependents {
            dependents.value.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.dependencies.add_path(path.clone());
        if let Some(dependencies) = &mut self.dependencies {
            dependencies.value.add_path(path.clone());
        }

        self.dependents.add_path(path.clone());
        if let Some(dependents) = &mut self.dependents {
            dependents.value.add_path(path);
        }
    }
}

impl WithMetadata for Permissions {
    fn add_text(&mut self, text: Arc<str>) {
        self.allow.add_text(text.clone());
        if let Some(allow) = &mut self.allow {
            allow.value.add_text(text.clone());
        }

        self.deny.add_text(text.clone());
        if let Some(deny) = &mut self.deny {
            deny.value.add_text(text.clone());
        }
    }

    fn add_path(&mut self, path: Arc<str>) {
        self.allow.add_path(path.clone());
        if let Some(allow) = &mut self.allow {
            allow.value.add_path(path.clone());
        }

        self.deny.add_path(path.clone());
        if let Some(deny) = &mut self.deny {
            deny.value.add_path(path);
        }
    }
}
