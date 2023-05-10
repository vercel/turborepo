use crate::Lockfile;

mod de;
mod ser;

struct Yarn1Lockfile {}

impl Lockfile for Yarn1Lockfile {
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        todo!()
    }

    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        todo!()
    }
}
