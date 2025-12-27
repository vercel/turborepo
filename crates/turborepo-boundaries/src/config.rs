use std::{collections::HashMap, sync::Arc};

use biome_deserialize_macros::Deserializable;
use serde::Serialize;
use struct_iterable::Iterable;
use turborepo_errors::{Spanned, WithMetadata};

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
pub struct BoundariesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Spanned<RulesMap>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_dependencies: Option<Spanned<Vec<Spanned<String>>>>,
    /// If in a package `turbo.json`, the following two keys define
    /// boundaries rules for that package
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Spanned<Permissions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependents: Option<Spanned<Permissions>>,
}

pub type RulesMap = HashMap<String, Spanned<Rule>>;

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
pub struct Rule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Spanned<Permissions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependents: Option<Spanned<Permissions>>,
}

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
pub struct Permissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<Spanned<Vec<Spanned<String>>>>,
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
