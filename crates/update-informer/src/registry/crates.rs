#[cfg(test)]
use mockito;
use serde::Deserialize;

use crate::{
    http_client::{GenericHttpClient, HttpClient},
    Package, Registry, Result,
};

#[cfg(not(test))]
const REGISTRY_URL: &str = "https://crates.io";

#[derive(Deserialize)]
struct Response {
    versions: Vec<VersionResponse>,
}

#[derive(Deserialize)]
struct VersionResponse {
    num: String,
}

/// The Rust community’s crate registry.
pub struct Crates;

#[cfg(not(test))]
fn get_base_url() -> String {
    format!("{REGISTRY_URL}/api/v1/crates")
}

#[cfg(test)]
fn get_base_url() -> String {
    format!("{}/api/v1/crates", &mockito::server_url())
}

impl Registry for Crates {
    const NAME: &'static str = "crates";

    fn get_latest_version<T: HttpClient>(
        http_client: GenericHttpClient<T>,
        pkg: &Package,
    ) -> Result<Option<String>> {
        let url = format!("{}/{}/versions", get_base_url(), pkg);
        let resp = http_client.get::<Response>(&url)?;

        if let Some(v) = resp.versions.first() {
            return Ok(Some(v.num.clone()));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{http_client, test_helper::mock_crates};

    const PKG_NAME: &str = "repo";
    const RAW_VERSION: &str = "0.1.0";
    const FIXTURES_PATH: &str = "tests/fixtures/registry/crates";
    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn failure_test() {
        let pkg = Package::new(PKG_NAME, RAW_VERSION).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/not_found.json", FIXTURES_PATH);
        let _mock = mock_crates(&pkg, 404, &data_path);
        let result = Crates::get_latest_version(client, &pkg);
        assert!(result.is_err());
    }

    #[test]
    fn success_test() {
        let pkg = Package::new(PKG_NAME, RAW_VERSION).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/versions.json", FIXTURES_PATH);
        let (_mock, data) = mock_crates(&pkg, 200, &data_path);

        let json: Response = serde_json::from_str(&data).expect("deserialize json");
        let latest_version = json
            .versions
            .first()
            .expect("get latest version")
            .num
            .clone();

        let result = Crates::get_latest_version(client, &pkg);

        assert!(result.is_ok());
        assert_eq!(result.expect("get result"), Some(latest_version));
    }
}
