use anyhow::Result;

use crate::{commands::CommandBase, package_json::PackageJson, package_manager::PackageManager};

pub fn run(base: &mut CommandBase) -> Result<()> {
    let root_package_json = PackageJson::load(&base.repo_root.join_component("package.json")).ok();

    let package_manager =
        PackageManager::get_package_manager(&base.repo_root, root_package_json.as_ref())?;

    let mut package_jsons: Vec<_> = package_manager
        .get_package_jsons(&base.repo_root)?
        .collect();
    package_jsons.sort();

    for package_json_path in package_jsons {
        let mut relative_package_json_path = base.repo_root.anchor(package_json_path)?;
        relative_package_json_path.pop();
        println!("{}", relative_package_json_path);
    }

    Ok(())
}
