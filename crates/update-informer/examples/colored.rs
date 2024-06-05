use colored::*;
use update_informer::{registry, Check};

fn main() {
    let pkg_name = "update-informer";
    let current_version = "0.1.0";

    let informer = update_informer::new(registry::Crates, pkg_name, current_version);
    if let Some(version) = informer.check_version().ok().flatten() {
        let msg = format!(
            "A new release of {pkg_name} is available: v{current_version} -> {new_version}",
            pkg_name = pkg_name.italic().cyan(),
            current_version = current_version,
            new_version = version.to_string().green()
        );

        let release_url =
            format!("https://github.com/{pkg_name}/{pkg_name}/releases/tag/{version}").yellow();

        println!("\n{msg}\n{url}", msg = msg, url = release_url);
    }
}
