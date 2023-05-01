use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<String>,
    #[serde(flatten)]
    pub other: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TurboJson {
    #[serde(flatten)]
    other: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_spaces: Option<SpacesJson>,
}
