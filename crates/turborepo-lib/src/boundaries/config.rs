use biome_deserialize_macros::Deserializable;
use serde::Serialize;
use struct_iterable::Iterable;
use turborepo_errors::Spanned;

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    allow: Option<Spanned<Vec<Spanned<String>>>>,
    deny: Option<Spanned<Vec<Spanned<String>>>>,
}

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoundariesConfig {
    tags: Option<Spanned<Vec<Spanned<String>>>>,
    dependencies: Option<Spanned<Permissions>>,
    dependents: Option<Spanned<Permissions>>,
}
