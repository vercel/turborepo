use std::fmt::{Display, Formatter};

use crate::Result;

/// A version representation.
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq)]
pub struct Version(semver::Version);

impl Version {
    pub(crate) fn parse<V: AsRef<str>>(value: V) -> Result<Self> {
        let value = value.as_ref();
        let v = value.strip_prefix('v').unwrap_or(value);
        let version = semver::Version::parse(v)?;

        Ok(Self(version))
    }

    /// Returns `semver::Version`.
    pub fn semver(&self) -> &semver::Version {
        &self.0
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use crate::Version;

    #[test]
    fn parse_str_version_test() {
        let version1 = Version::parse("0.1.0");
        let version2 = Version {
            0: semver::Version {
                major: 0,
                minor: 1,
                patch: 0,
                pre: Default::default(),
                build: Default::default(),
            },
        };

        assert!(version1.is_ok());
        assert_eq!(version1.unwrap(), version2);
    }

    #[test]
    fn parse_string_version_test() {
        let version1 = Version::parse(String::from("0.1.0"));
        let version2 = Version {
            0: semver::Version {
                major: 0,
                minor: 1,
                patch: 0,
                pre: Default::default(),
                build: Default::default(),
            },
        };

        assert!(version1.is_ok());
        assert_eq!(version1.unwrap(), version2);
    }

    #[test]
    fn parse_amp_string_version_test() {
        let version1 = Version::parse(&String::from("0.1.0"));
        let version2 = Version {
            0: semver::Version {
                major: 0,
                minor: 1,
                patch: 0,
                pre: Default::default(),
                build: Default::default(),
            },
        };

        assert!(version1.is_ok());
        assert_eq!(version1.unwrap(), version2);
    }

    #[test]
    fn parse_version_with_prefix_test() {
        let version1 = Version::parse("v0.1.0");
        let version2 = Version {
            0: semver::Version {
                major: 0,
                minor: 1,
                patch: 0,
                pre: Default::default(),
                build: Default::default(),
            },
        };

        assert!(version1.is_ok());
        assert_eq!(version1.unwrap(), version2);
    }

    #[test]
    fn fmt_test() {
        let version = Version::parse("0.1.0");
        assert!(version.is_ok());
        assert_eq!(String::from("v0.1.0"), format!("{}", version.unwrap()))
    }

    #[test]
    fn semver_test() {
        let version = Version::parse("0.1.0-canary");
        assert!(version.is_ok());

        let version = version.unwrap();
        assert_eq!(version.semver().pre.to_string(), "canary");
    }
}
