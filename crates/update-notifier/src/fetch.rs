use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use crate::UpdateNotifierError;

#[derive(Deserialize)]
struct NpmVersionData {
    version: String,
}

const REGISTRY_URL: &str = "https://registry.npmjs.org";
const DEFAULT_TAG: &str = "latest";
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(800);

pub async fn get_latest_version(
    package: &str,
    tag: Option<&str>,
    timeout: Option<Duration>,
) -> Result<String, UpdateNotifierError> {
    let tag = tag.unwrap_or(DEFAULT_TAG);
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let client: Client = reqwest::Client::new();
    let url = format!("{}/{}/{}", REGISTRY_URL, package, tag);

    // send request
    log::debug!("fetching {:?}", url);
    let resp = client.get(url).timeout(timeout).send().await?;

    let json_result = resp.json::<NpmVersionData>().await?;
    Ok(json_result.version)
}
