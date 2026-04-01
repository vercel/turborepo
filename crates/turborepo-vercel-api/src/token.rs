use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseTokenMetadata {
    pub scopes: Vec<Scope>,
    #[serde(rename = "activeAt")]
    pub active_at: u128,
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
