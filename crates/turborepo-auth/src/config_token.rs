/**
 * This whole file will hopefully go away in the future when we stop writing
 * tokens to `config.json`.
 */

#[derive(serde::Deserialize, serde::Serialize)]
/// ConfigToken describes the legacy token format. It should only be used as a
/// way to store the underlying token as a Token trait, and then converted to an
/// AuthToken.
pub struct ConfigToken {
    pub token: String,
}
