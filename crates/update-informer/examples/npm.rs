use std::time::Duration;

use update_informer::{registry, Check};

fn main() {
    let pkg_name = "turbo";
    let current_version = "1.6.2";

    let informer =
        update_informer::new(registry::Npm, pkg_name, current_version).interval(Duration::ZERO);

    if let Ok(Some(new_version)) = informer.check_version() {
        println!("A new release of {pkg_name} is available: v{current_version} -> {new_version}");
    }
}
