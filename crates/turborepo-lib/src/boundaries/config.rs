use std::collections::HashMap;

use biome_deserialize_macros::Deserializable;
use serde::Serialize;
use struct_iterable::Iterable;
use turborepo_errors::Spanned;

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
pub struct RootBoundariesConfig {
    pub tags: Option<Spanned<RulesMap>>,
}
pub type RulesMap = HashMap<String, Spanned<Rule>>;

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
pub struct Rule {
    pub dependencies: Option<Spanned<Permissions>>,
    pub dependents: Option<Spanned<Permissions>>,
}

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
pub struct Permissions {
    pub allow: Option<Spanned<Vec<Spanned<String>>>>,
    pub deny: Option<Spanned<Vec<Spanned<String>>>>,
}
