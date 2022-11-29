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
const TIMEOUT: Duration = Duration::from_millis(800);

pub async fn get_latest_version(
    package: &str,
    tag: Option<&str>,
    timeout: Option<Duration>,
) -> Result<String, UpdateNotifierError> {
    log::debug!("fetching latest version");
    let tag = tag.unwrap_or_else(|| DEFAULT_TAG);
    let timeout = timeout.unwrap_or_else(|| TIMEOUT);
    let client: Client = reqwest::Client::new();
    let url = format!("{}/{}/{}", REGISTRY_URL, package, tag);
    log::debug!("fetching {:?}", url);
    let resp = client.get(url).timeout(timeout).send().await;
    match resp {
        Ok(r) => {
            let json_result = r.json::<NpmVersionData>().await;
            match json_result {
                Ok(v) => Ok(v.version.to_string()),
                Err(err) => Err(UpdateNotifierError::FetchError(err)),
            }
        }
        Err(err) => {
            log::error!("failed to fetch latest version {:?}", err);
            Err(UpdateNotifierError::FetchError(err))
        }
    }
}
