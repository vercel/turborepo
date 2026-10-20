use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseTokenMetadata {
    pub scopes: Vec<Scope>,
    #[serde(rename = "activeAt")]
    pub active_at: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    #[serde(rename = "type")]
    pub scope_type: String,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<u128>,
    #[serde(rename = "teamId")]
    pub team_id: Option<String>,
}
