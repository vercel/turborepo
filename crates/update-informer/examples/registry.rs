use std::time::Duration;

use update_informer::{
    http_client::{GenericHttpClient, HttpClient},
    Check, Package, Registry, Result,
};

#[derive(serde::Deserialize)]
struct Response {
    version: String,
}

struct YourOwnRegistry;
impl Registry for YourOwnRegistry {
    const NAME: &'static str = "your_own_registry";

    fn get_latest_version<T: HttpClient>(
        http_client: GenericHttpClient<T>,
        _pkg: &Package,
    ) -> Result<Option<String>> {
        let url = "https://turbo.build/api/binaries/version";
        let resp = http_client.get::<Response>(&url)?;

        Ok(Some(resp.version))
    }
}

fn main() {
    let pkg_name = "turbo";
    let current_version = "1.6.2";

    let informer =
        update_informer::new(YourOwnRegistry, pkg_name, current_version).interval(Duration::ZERO);

    if let Ok(Some(new_version)) = informer.check_version() {
        println!("A new release of {pkg_name} is available: v{current_version} -> {new_version}");
    }
}
