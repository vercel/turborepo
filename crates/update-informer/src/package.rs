use std::fmt::{Display, Formatter};

use crate::{Result, Version};

/// A package representation.
#[derive(Debug, PartialEq, Eq)]
pub struct Package<'a> {
    owner: Option<&'a str>,
    name: &'a str,
    version: Version,
}

impl<'a> Package<'a> {
    pub(crate) fn new(name: &'a str, version: &'a str) -> Result<Self> {
        let version = Version::parse(version)?;

        let pkg = if !name.contains('/') {
            Self {
                owner: None,
                name,
                version,
            }
        } else {
            let parts = name.split('/').collect::<Vec<_>>();

            Self {
                owner: Some(parts[0]),
                name: parts[1],
                version,
            }
        };

        Ok(pkg)
    }

    /// Returns a name suitable for storing on filesystem, that will include
    /// owner if it is set.
    pub(crate) fn name(&self) -> String {
        let owner = self.owner.map(|s| format!("{s}-")).unwrap_or_default();
        format!("{}{}", owner, self.name)
    }

    /// Returns the parsed version of the package
    pub fn version(&self) -> &Version {
        &self.version
    }
}

impl Display for Package<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.owner.is_none() {
            return write!(f, "{}", self.name);
        }

        write!(f, "{}/{}", self.owner.as_ref().unwrap(), self.name)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Package, Version};

    const RAW_VERSION: &str = "1.0.0";

    #[test]
    fn new_with_name_test() {
        let version = Version::parse(RAW_VERSION).expect("parse version");
        let pkg1 = Package::new("repo", RAW_VERSION).unwrap();
        let pkg2 = Package {
            owner: None,
            name: "repo",
            version: version.clone(),
        };

        assert_eq!(pkg1, pkg2);
        assert_eq!(pkg1.name(), "repo".to_string());
        assert_eq!(pkg1.version(), &version);
    }

    #[test]
    fn new_with_owner_and_name_test() {
        let version = Version::parse(RAW_VERSION).expect("parse version");
        let pkg1 = Package::new("owner/repo", RAW_VERSION).unwrap();
        let pkg2 = Package {
            owner: Some("owner"),
            name: "repo",
            version,
        };

        assert_eq!(pkg1, pkg2);
        assert_eq!(pkg1.name(), "owner-repo".to_string());
    }

    #[test]
    fn name_fmt_test() {
        let pkg = Package::new("repo", RAW_VERSION).unwrap();
        assert_eq!(String::from("repo"), format!("{}", pkg))
    }

    #[test]
    fn name_with_owner_fmt_test() {
        let pkg = Package::new("owner/repo", RAW_VERSION).unwrap();
        assert_eq!(String::from("owner/repo"), format!("{}", pkg))
    }
}
