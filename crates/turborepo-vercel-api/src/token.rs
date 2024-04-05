use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseTokenMetadata {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub token_type: String,
    pub origin: String,
    pub scopes: Vec<Scope>,
    #[serde(rename = "activeAt")]
    pub active_at: u128,
    #[serde(rename = "createdAt")]
    pub created_at: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    #[serde(rename = "type")]
    pub scope_type: String,
    pub origin: String,
    #[serde(rename = "createdAt")]
    pub created_at: u128,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<u128>,
    #[serde(rename = "teamId")]
    pub team_id: Option<String>,
}
