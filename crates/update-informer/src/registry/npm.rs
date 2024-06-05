use serde::Deserialize;

use crate::{
    http_client::{GenericHttpClient, HttpClient},
    Package, Registry, Result,
};

#[cfg(not(test))]
const REGISTRY_URL: &str = "https://registry.npmjs.org";

#[derive(Deserialize)]
struct Response {
    version: String,
}

/// The NPM package registry.
pub struct Npm;

#[cfg(not(test))]
fn get_base_url() -> String {
    REGISTRY_URL.to_string()
}

#[cfg(test)]
fn get_base_url() -> String {
    mockito::server_url()
}

impl Registry for Npm {
    const NAME: &'static str = "npm";

    fn get_latest_version<T: HttpClient>(
        http_client: GenericHttpClient<T>,
        pkg: &Package,
    ) -> Result<Option<String>> {
        let url = format!("{}/{}/latest", get_base_url(), pkg);
        let resp = http_client.get::<Response>(&url)?;

        Ok(Some(resp.version))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{http_client, test_helper::mock_npm};

    const PKG_NAME: &str = "turbo";
    const FIXTURES_PATH: &str = "tests/fixtures/registry/npm";
    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn failure_test() {
        let raw_version = "0.1.0";
        let pkg = Package::new(PKG_NAME, raw_version).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/not_found.html", FIXTURES_PATH);
        let _mock = mock_npm(&pkg, 404, &data_path);

        let result = Npm::get_latest_version(client, &pkg);
        assert!(result.is_err());
    }

    #[test]
    fn success_test() {
        let raw_version = "1.6.2";
        let pkg = Package::new(PKG_NAME, raw_version).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/latest.json", FIXTURES_PATH);
        let (_mock, _data) = mock_npm(&pkg, 200, &data_path);

        let latest_version = "1.6.3".to_string();
        let result = Npm::get_latest_version(client, &pkg);

        assert!(result.is_ok());
        assert_eq!(result.expect("get result"), Some(latest_version));
    }
}
