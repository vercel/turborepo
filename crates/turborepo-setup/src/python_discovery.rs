//! Root-only CPython interpreter requests. A root `.python-version` wins, as in
//! uv; otherwise `pyproject.toml` `[project] requires-python` applies. Only
//! CPython release requests are supported: `3.12`, `3.12.7`, `cpython@3.12`,
//! `cpython-3.12` and release-only PEP 440 specifiers. PyPy, GraalPy,
//! free-threaded, prerelease and path requests fail closed. No binaries,
//! network or lock writes.

use std::io;

use crate::{
    lock::Declaration,
    pep440::{self, Release, Specifiers},
    uv_discovery::MAX_CONFIG_BYTES,
};

pub const SOURCES: [&str; 2] = [".python-version", "pyproject.toml"];

#[derive(Debug, thiserror::Error)]
pub enum PythonDiscoveryError {
    #[error("cannot read {0}: {1}")]
    Read(&'static str, #[source] io::Error),
    #[error("{0} exceeds the {MAX_CONFIG_BYTES}-byte discovery limit")]
    TooLarge(&'static str),
    #[error("invalid {file}: {reason}")]
    Invalid { file: &'static str, reason: String },
    #[error(
        "unsupported Python request {request:?} in {file}; use a CPython release such as 3.12, \
         3.12.7, cpython@3.12, or a PEP 440 range"
    )]
    Unsupported { file: &'static str, request: String },
}

#[derive(Clone, Debug)]
pub struct PythonRequirement {
    file: &'static str,
    field: Option<&'static str>,
    request: String,
    specifiers: Specifiers,
}

impl PythonRequirement {
    /// `read` returns raw root file bytes (or None when absent); limits are
    /// enforced here so callers may supply captured snapshot bytes.
    pub fn read_with(
        mut read: impl FnMut(&'static str, usize) -> io::Result<Option<Vec<u8>>>,
    ) -> Result<Option<Self>, PythonDiscoveryError> {
        for file in SOURCES {
            let Some(bytes) = read(file, MAX_CONFIG_BYTES)
                .map_err(|error| PythonDiscoveryError::Read(file, error))?
            else {
                continue;
            };
            if bytes.len() > MAX_CONFIG_BYTES {
                return Err(PythonDiscoveryError::TooLarge(file));
            }
            let invalid = |reason: &str| PythonDiscoveryError::Invalid {
                file,
                reason: reason.into(),
            };
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid("expected UTF-8"))?;
            let (field, request) = if file == ".python-version" {
                let mut lines = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'));
                let Some(request) = lines.next() else {
                    return Err(invalid("expected one Python request"));
                };
                if lines.next().is_some() {
                    return Err(invalid("expected exactly one Python request"));
                }
                (None, request.to_owned())
            } else {
                // Do not echo parser diagnostics; they can quote unrelated values.
                let table: toml::Table =
                    text.parse().map_err(|_| invalid("expected valid TOML"))?;
                let project = match table.get("project") {
                    None => return Ok(None),
                    Some(project) => project
                        .as_table()
                        .ok_or_else(|| invalid("`project` table"))?,
                };
                let Some(value) = project.get("requires-python") else {
                    return Ok(None);
                };
                let request = value
                    .as_str()
                    .ok_or_else(|| invalid("`requires-python` must be a string"))?;
                (Some("project.requires-python"), request.trim().to_owned())
            };
            let specifiers = parse(&request).ok_or_else(|| PythonDiscoveryError::Unsupported {
                file,
                request: request.clone(),
            })?;
            return Ok(Some(Self {
                file,
                field,
                request,
                specifiers,
            }));
        }
        Ok(None)
    }

    pub fn declaration(&self) -> Declaration {
        Declaration {
            file: self.file.into(),
            field: self.field.map(Into::into),
            request: Some(self.request.clone()),
        }
    }

    pub fn file(&self) -> &'static str {
        self.file
    }

    pub fn request(&self) -> &str {
        &self.request
    }

    /// Whether an exact final CPython release satisfies the request.
    pub fn matches(&self, version: &semver::Version) -> bool {
        version.pre.is_empty()
            && version.build.is_empty()
            && self.specifiers.contains(&Release::from(version))
    }
}

/// A bare version is a release prefix (`3.12` means any `3.12.x`), matching
/// uv's interpretation of `.python-version`.
fn parse(request: &str) -> Option<Specifiers> {
    if request.len() > pep440::MAX_SPECIFIER_BYTES {
        return None;
    }
    let version = request
        .strip_prefix("cpython@")
        .or_else(|| request.strip_prefix("cpython-"))
        .unwrap_or(request);
    match Release::parse(version) {
        Some(release) if release.segments().len() == 3 => Some(Specifiers::exact(release)),
        Some(release) if release.segments().len() < 3 => {
            Specifiers::parse(&format!("=={version}.*")).ok()
        }
        Some(_) => None,
        // Ranges are not combined with an implementation prefix.
        None if version == request => Specifiers::parse(request).ok(),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semver::Version;

    use super::*;

    fn discover(
        files: &[(&'static str, &str)],
    ) -> Result<Option<PythonRequirement>, PythonDiscoveryError> {
        let files: BTreeMap<_, _> = files.iter().copied().collect();
        PythonRequirement::read_with(|file, _| Ok(files.get(file).map(|t| t.as_bytes().to_vec())))
    }
    fn matches(files: &[(&'static str, &str)], version: &str) -> bool {
        discover(files)
            .unwrap()
            .unwrap()
            .matches(&Version::parse(version).unwrap())
    }

    #[test]
    fn python_version_wins_and_bare_versions_are_prefixes() {
        let pyproject = (
            "pyproject.toml",
            "[project]\nrequires-python = \">=3.13\"\n",
        );
        for (request, version, expected) in [
            ("3.12", "3.12.12", true),
            ("3.12", "3.13.0", false),
            ("3", "3.14.1", true),
            ("3.12.7", "3.12.7", true),
            ("3.12.7", "3.12.8", false),
            ("cpython@3.12", "3.12.1", true),
            ("cpython-3.11.4", "3.11.4", true),
            (">=3.11,<3.13", "3.12.0", true),
        ] {
            let text = format!("# pinned\n\n {request} \n");
            assert_eq!(
                matches(&[(".python-version", &text), pyproject], version),
                expected,
                "{request} {version}"
            );
        }
        let found = discover(&[pyproject]).unwrap().unwrap();
        assert_eq!(
            found.declaration(),
            Declaration {
                file: "pyproject.toml".into(),
                field: Some("project.requires-python".into()),
                request: Some(">=3.13".into()),
            }
        );
        assert!(found.matches(&Version::new(3, 14, 0)));
        assert!(!found.matches(&Version::parse("3.14.0-rc.1").unwrap()));
    }

    #[test]
    fn unsupported_and_invalid_requests_fail_closed() {
        for request in [
            "pypy@3.10",
            "3.13t",
            "3.14.0rc1",
            "system",
            "/usr/bin/python3",
            "graalpy",
            "3.1.2.3",
            "cpython@>=3.12",
        ] {
            assert!(
                matches!(
                    discover(&[(".python-version", request)]),
                    Err(PythonDiscoveryError::Unsupported {
                        file: ".python-version",
                        ..
                    })
                ),
                "{request}"
            );
        }
        for (file, text) in [
            (".python-version", "# only a comment\n"),
            (".python-version", "3.12\n3.13\n"),
            ("pyproject.toml", "[project]\nrequires-python = 3\n"),
            ("pyproject.toml", "project = \"secret\"\n"),
            ("pyproject.toml", "[broken"),
        ] {
            let error = discover(&[(file, text)]).unwrap_err();
            assert!(
                matches!(&error, PythonDiscoveryError::Invalid { file: found, .. } if *found == file),
                "{text}"
            );
            assert!(!error.to_string().contains("secret"));
        }
        for files in [
            vec![],
            vec![("pyproject.toml", "[project]\nname = \"app\"\n")],
        ] {
            assert!(discover(&files).unwrap().is_none());
        }
    }
}
