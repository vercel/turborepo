use std::time::Duration;

use isahc::ReadResponseExt;
use serde::de::DeserializeOwned;
use update_informer::{
    http_client::{HeaderMap, HttpClient},
    registry, Check,
};

struct YourOwnHttpClient;

impl HttpClient for YourOwnHttpClient {
    fn get<T: DeserializeOwned>(
        url: &str,
        _timeout: Duration,
        _headers: HeaderMap,
    ) -> update_informer::Result<T> {
        let json = isahc::get(url)?.json()?;
        Ok(json)
    }
}

fn main() {
    let pkg_name = "update-informer";
    let current_version = "0.1.0";

    let informer = update_informer::new(registry::Crates, pkg_name, current_version)
        .interval(Duration::ZERO)
        .http_client(YourOwnHttpClient);

    if let Ok(Some(new_version)) = informer.check_version() {
        println!("A new release of {pkg_name} is available: v{current_version} -> {new_version}");
    }
}
