use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseTokenMetadata {
    id: String,
    name: String,
    #[serde(rename = "type")]
    token_type: String,
    origin: String,
    scopes: Vec<Scope>,
    #[serde(rename = "activeAt")]
    active_at: u64,
    #[serde(rename = "createdAt")]
    created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    #[serde(rename = "type")]
    scope_type: String,
    origin: String,
    #[serde(rename = "createdAt")]
    created_at: u64,
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
    #[serde(rename = "teamId")]
    team_id: Option<String>,
}
