#[cfg(test)]
use mockito;
use serde::Deserialize;

use crate::{
    http_client::{GenericHttpClient, HttpClient},
    Package, Registry, Result,
};

#[cfg(not(test))]
const REGISTRY_URL: &str = "https://api.github.com";

#[derive(Deserialize)]
struct Response {
    tag_name: String,
}

/// The most popular and largest project hosting.
pub struct GitHub;

#[cfg(not(test))]
fn get_base_url() -> String {
    format!("{REGISTRY_URL}/repos")
}

#[cfg(test)]
fn get_base_url() -> String {
    format!("{}/repos", &mockito::server_url())
}

impl Registry for GitHub {
    const NAME: &'static str = "github";

    fn get_latest_version<T: HttpClient>(
        http_client: GenericHttpClient<T>,
        pkg: &Package,
    ) -> Result<Option<String>> {
        let url = format!("{}/{}/releases/latest", get_base_url(), pkg);
        let resp = http_client
            .add_header("Accept", "application/vnd.github.v3+json")
            .add_header("User-Agent", "update-informer")
            .get::<Response>(&url)?;

        if resp.tag_name.starts_with('v') {
            return Ok(Some(resp.tag_name[1..].to_string()));
        }

        Ok(Some(resp.tag_name))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{http_client, test_helper::mock_github};

    const PKG_NAME: &str = "owner/repo";
    const FIXTURES_PATH: &str = "tests/fixtures/registry/github";
    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn failure_test() {
        let raw_version = "0.1.0";
        let pkg = Package::new(PKG_NAME, raw_version).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/not_found.json", FIXTURES_PATH);
        let _mock = mock_github(&pkg, 404, &data_path);

        let result = GitHub::get_latest_version(client, &pkg);
        assert!(result.is_err());
    }

    #[test]
    fn success_test() {
        let raw_version = "1.6.3-canary.0";
        let pkg = Package::new(PKG_NAME, raw_version).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/release.json", FIXTURES_PATH);
        let (_mock, data) = mock_github(&pkg, 200, &data_path);

        let json: Response = serde_json::from_str(&data).expect("deserialize json");
        let latest_version = json.tag_name[1..].to_string();

        let result = GitHub::get_latest_version(client, &pkg);

        assert!(result.is_ok());
        assert_eq!(result.expect("get result"), Some(latest_version));
    }
}
