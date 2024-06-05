#[cfg(test)]
use mockito;
use serde::Deserialize;

use crate::{
    http_client::{GenericHttpClient, HttpClient},
    Package, Registry, Result,
};

#[cfg(not(test))]
const REGISTRY_URL: &str = "https://pypi.org";

#[derive(Deserialize, Debug)]
struct Response {
    info: Info,
}

#[derive(Deserialize, Debug)]
struct Info {
    version: String,
    yanked: bool,
}

/// The Python community’s package registry.
pub struct PyPI;

#[cfg(not(test))]
fn get_base_url() -> String {
    format!("{REGISTRY_URL}/pypi")
}

#[cfg(test)]
fn get_base_url() -> String {
    format!("{}/pypi", &mockito::server_url())
}

impl Registry for PyPI {
    const NAME: &'static str = "pypi";

    fn get_latest_version<T: HttpClient>(
        http_client: GenericHttpClient<T>,
        pkg: &Package,
    ) -> Result<Option<String>> {
        let url = format!("{}/{}/json", get_base_url(), pkg);
        let resp = http_client.get::<Response>(&url)?;

        if !resp.info.yanked {
            return Ok(Some(resp.info.version));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{http_client, test_helper::mock_pypi};

    const PKG_NAME: &str = "filprofiler";
    const RAW_VERSION: &str = "0.1.0";
    const FIXTURES_PATH: &str = "tests/fixtures/registry/pypi";
    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn failure_test() {
        let pkg = Package::new(PKG_NAME, RAW_VERSION).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/not_found.html", FIXTURES_PATH);
        let _mock = mock_pypi(&pkg, 404, &data_path);

        let result = PyPI::get_latest_version(client, &pkg);
        assert!(result.is_err());
    }

    #[test]
    fn success_test() {
        let pkg = Package::new(PKG_NAME, RAW_VERSION).unwrap();
        let client = http_client::new(http_client::DefaultHttpClient {}, TIMEOUT);
        let data_path = format!("{}/release.json", FIXTURES_PATH);
        let (_mock, _data) = mock_pypi(&pkg, 200, &data_path);

        let latest_version = "2022.1.1".to_string();
        let result = PyPI::get_latest_version(client, &pkg);

        assert!(result.is_ok());
        assert_eq!(result.expect("get result"), Some(latest_version));
    }
}
