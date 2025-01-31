use biome_deserialize_macros::Deserializable;
use serde::Serialize;
use struct_iterable::Iterable;
use turborepo_errors::Spanned;

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub allow: Option<Spanned<Vec<Spanned<String>>>>,
    pub deny: Option<Spanned<Vec<Spanned<String>>>>,
}

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoundariesConfig {
    pub tags: Option<Spanned<Vec<Spanned<String>>>>,
    pub dependencies: Option<Spanned<Permissions>>,
    pub dependents: Option<Spanned<Permissions>>,
}
