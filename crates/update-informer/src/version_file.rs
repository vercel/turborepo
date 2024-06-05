use std::{
    fs,
    io::{self, ErrorKind},
    path::PathBuf,
    time::Duration,
};

use crate::{Package, Result};

#[derive(Debug, PartialEq)]
pub(crate) struct VersionFile<'a> {
    path: PathBuf,
    version: &'a str,
}

impl<'a> VersionFile<'a> {
    pub(crate) fn new(registry: &str, pkg: &Package, version: &'a str) -> Result<Self> {
        let file_name = format!("{}-{}", registry, pkg.name());
        let path = cache_path()?.join(file_name);

        Ok(Self { path, version })
    }

    pub(crate) fn last_modified(&self) -> Result<Duration> {
        let metadata = match fs::metadata(&self.path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                self.write_version(self.version)?;
                return Ok(Duration::ZERO);
            }
            Err(e) => return Err(e.into()),
        };

        let last_modified = metadata.modified()?.elapsed();
        Ok(last_modified.unwrap_or_default())
    }

    pub(crate) fn recreate_file(&self) -> io::Result<()> {
        fs::remove_file(&self.path)?;
        self.write_version(self.version)
    }

    pub(crate) fn write_version<V: AsRef<str>>(&self, version: V) -> io::Result<()> {
        fs::write(&self.path, version.as_ref())
    }

    pub(crate) fn get_version(&self) -> io::Result<String> {
        fs::read_to_string(&self.path)
    }
}

#[cfg(not(test))]
fn cache_path() -> Result<PathBuf> {
    let project_dir = directories::ProjectDirs::from("", "", "update-informer-rs")
        .ok_or("Unable to find cache directory")?;
    let directory = project_dir.cache_dir().to_path_buf();
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

#[cfg(test)]
fn cache_path() -> Result<PathBuf> {
    Ok(std::env::temp_dir().join("update-informer-test"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helper::within_test_dir;

    #[test]
    fn new_test() {
        let version = "0.1.0";
        let pkg = Package::new("repo", version).unwrap();
        let version_file1 = VersionFile::new("myreg", &pkg, version).unwrap();
        let version_file2 = VersionFile {
            path: cache_path().unwrap().join("myreg-repo"),
            version: "0.1.0",
        };

        assert_eq!(version_file1, version_file2);
    }

    #[test]
    fn create_version_file_twice_test() {
        let version = "0.1.0";
        let pkg = Package::new("repo", version).unwrap();
        let version_file1 = VersionFile::new("reg", &pkg, version).expect("create version file");
        let version_file2 = VersionFile::new("reg", &pkg, version).expect("create version file");
        assert_eq!(version_file1, version_file2);
    }

    #[test]
    fn last_modified_file_exists_test() {
        within_test_dir(|path| {
            fs::write(&path, "0.1.0").expect("creates test file");

            let version_file = VersionFile {
                path,
                version: "0.1.0",
            };

            let last_modified = version_file.last_modified();
            assert!(last_modified.is_ok());
            assert!(!last_modified.unwrap().is_zero());
        });
    }

    #[test]
    fn last_modified_file_not_exists_test() {
        within_test_dir(|path| {
            let version_file = VersionFile {
                path: path.clone(),
                version: "0.1.0",
            };

            let last_modified = version_file.last_modified();
            assert!(last_modified.is_ok());
            assert!(last_modified.unwrap().is_zero());

            let version = fs::read_to_string(&path).expect("read test file");
            assert_eq!(version, "0.1.0");
        });
    }

    #[test]
    fn recreate_file_test() {
        within_test_dir(|path| {
            fs::write(&path, "0.1.0").expect("creates test file");

            let version_file = VersionFile {
                path: path.clone(),
                version: "1.0.0",
            };

            let result = version_file.recreate_file();
            assert!(result.is_ok());

            let version = fs::read_to_string(&path).expect("read test file");
            assert_eq!(version, "1.0.0");
        });
    }

    #[test]
    fn write_version_test() {
        within_test_dir(|path| {
            fs::write(&path, "1.0.0").expect("creates test file");

            let version_file = VersionFile {
                path: path.clone(),
                version: "1.0.0",
            };

            let result = version_file.write_version("2.0.0");
            assert!(result.is_ok());

            let version = fs::read_to_string(&path).expect("read test file");
            assert_eq!(version, "2.0.0");
        });
    }

    #[test]
    fn get_version_file_exists_test() {
        within_test_dir(|path| {
            fs::write(&path, "1.0.0").expect("creates test file");

            let version_file = VersionFile {
                path: path.clone(),
                version: "1.0.0",
            };

            let result = version_file.get_version();
            assert!(result.is_ok());

            let version = fs::read_to_string(&path).expect("read test file");
            assert_eq!(version, "1.0.0");
        });
    }

    #[test]
    fn get_version_file_not_exists_test() {
        within_test_dir(|path| {
            let version_file = VersionFile {
                path: path.clone(),
                version: "1.0.0",
            };

            let result = version_file.get_version();
            assert!(result.is_err());
        });
    }
}
