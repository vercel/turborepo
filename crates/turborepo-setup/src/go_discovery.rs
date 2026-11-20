//! Pure root `go.work`/`go.mod` toolchain declarations, following
//! <https://go.dev/doc/toolchain>: `go` is a mandatory minimum and `toolchain`
//! is a preferred exact release. A root `go.work` governs; a root module it
//! uses only constrains the minimum. No `go` subprocess, GOTOOLCHAIN/GOWORK
//! environment, module graph, or network.

use std::{fmt, io};

pub const GO_WORK: &str = "go.work";
pub const GO_MOD: &str = "go.mod";
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GoDiscoveryError {
    #[error("cannot read root {0}")]
    Read(&'static str),
    #[error("root {file} is malformed at line {line}: {reason}; repair it with `go {edit} edit`")]
    Malformed {
        file: &'static str,
        line: usize,
        reason: &'static str,
        edit: &'static str,
    },
    #[error("unsupported `{directive} {value}` in root {file}: {reason}; {help}")]
    Unsupported {
        file: &'static str,
        directive: &'static str,
        value: String,
        reason: &'static str,
        help: String,
    },
    #[error("conflicting Go declarations: {0}")]
    Conflict(String),
    #[error("no published Go release satisfies {0}")]
    NoMatchingRelease(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Stage {
    Language,
    Beta(u32),
    Candidate(u32),
    Release(u32),
}

/// Go version ordering: `1.21 < 1.21rc1 < 1.21.0 < 1.21.1`. Before Go 1.21
/// the initial release was `1.N`, which sorts after its candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GoVersion {
    minor: u32,
    stage: Stage,
}

impl GoVersion {
    pub fn parse(value: &str) -> Option<Self> {
        let rest = value.strip_prefix("1.")?;
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let minor = number(&rest[..end])?;
        let suffix = &rest[end..];
        let stage = if suffix.is_empty() {
            if minor < 21 {
                Stage::Release(0)
            } else {
                Stage::Language
            }
        } else if let Some(patch) = suffix.strip_prefix('.') {
            match number(patch)? {
                0 if minor < 21 => return None,
                patch => Stage::Release(patch),
            }
        } else if let Some(n) = suffix.strip_prefix("rc") {
            Stage::Candidate(number(n).filter(|n| *n > 0)?)
        } else {
            Stage::Beta(number(suffix.strip_prefix("beta")?).filter(|n| *n > 0)?)
        };
        Some(Self { minor, stage })
    }

    /// A release, candidate, or beta that names a downloadable toolchain.
    pub fn is_toolchain(self) -> bool {
        self.stage != Stage::Language
    }
    pub fn is_release(self) -> bool {
        matches!(self.stage, Stage::Release(_))
    }
}

impl fmt::Display for GoVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "1.{}", self.minor)?;
        match self.stage {
            Stage::Language => Ok(()),
            Stage::Release(0) if self.minor < 21 => Ok(()),
            Stage::Release(patch) => write!(f, ".{patch}"),
            Stage::Candidate(n) => write!(f, "rc{n}"),
            Stage::Beta(n) => write!(f, "beta{n}"),
        }
    }
}

fn number(value: &str) -> Option<u32> {
    let canonical = value == "0" || !value.starts_with('0');
    (canonical && (1..=9).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

/// Lock-ready provenance. `field` is the directive name; a `None` request is
/// Go's implicit `go` line for a file without one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoSource {
    pub file: &'static str,
    pub field: &'static str,
    pub request: Option<String>,
}

#[derive(Default)]
struct Directives {
    go: Option<String>,
    toolchain: Option<String>,
    uses_root: bool,
}

fn edit(file: &str) -> &'static str {
    if file == GO_WORK { "work" } else { "mod" }
}

// One line's tokens outside `//` comments; quoted strings stay whole.
fn tokens(line: &str) -> Option<Vec<&str>> {
    let bytes = line.as_bytes();
    let comment = |i: usize| bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/');
    let (mut out, mut i) = (Vec::new(), 0);
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b if b.is_ascii_whitespace() => {
                i += 1;
                continue;
            }
            _ if comment(i) => break,
            b'(' | b')' => i += 1,
            quote @ (b'"' | b'`') => loop {
                i += 1;
                match *bytes.get(i)? {
                    b'\\' if quote == b'"' => i += 1,
                    b if b == quote => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
            },
            _ => {
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !b"()\"`".contains(&bytes[i])
                    && !comment(i)
                {
                    i += 1;
                }
            }
        }
        out.push(&line[start..i]);
    }
    Some(out)
}

fn unquote(value: &str) -> Option<&str> {
    match value.as_bytes().first() {
        Some(b'"' | b'`') => value
            .get(1..value.len() - 1)
            .filter(|inner| !inner.contains(['\\', '"', '`'])),
        _ => Some(value),
    }
}

fn directives(file: &'static str, text: &str) -> Result<Directives, GoDiscoveryError> {
    let malformed = |line, reason| GoDiscoveryError::Malformed {
        file,
        line,
        reason,
        edit: edit(file),
    };
    let mut result = Directives::default();
    let mut block = None;
    let mut last = 0;
    for (index, raw) in text.lines().enumerate() {
        last = index + 1;
        let tokens = tokens(raw).ok_or_else(|| malformed(last, "unterminated string"))?;
        let Some((&verb, args)) = tokens.split_first() else {
            continue;
        };
        if let Some(open) = block {
            match (verb, args) {
                (")", []) => block = None,
                (")", _) => return Err(malformed(last, "text after block end")),
                (path, []) if open == "use" => result.uses_root |= root_path(path),
                _ => {}
            }
            continue;
        }
        if args.last() == Some(&"(") {
            if matches!(verb, "go" | "toolchain") || args.len() != 1 {
                return Err(malformed(last, "unexpected directive block"));
            }
            block = Some(verb);
            continue;
        }
        match (verb, args) {
            ("go" | "toolchain", [value]) => {
                let value = unquote(value).ok_or_else(|| malformed(last, "invalid string"))?;
                let slot = if verb == "go" {
                    &mut result.go
                } else {
                    &mut result.toolchain
                };
                if slot.replace(value.to_owned()).is_some() {
                    return Err(malformed(last, "repeated directive"));
                }
            }
            ("go" | "toolchain", _) => return Err(malformed(last, "expected exactly one value")),
            ("use", [path]) => result.uses_root |= root_path(path),
            _ => {}
        }
    }
    if block.is_some() {
        return Err(malformed(last, "unterminated directive block"));
    }
    Ok(result)
}

fn root_path(path: &str) -> bool {
    unquote(path).is_some_and(|path| matches!(path, "." | "./"))
}

fn minimum(file: &'static str, raw: &str) -> Result<GoVersion, GoDiscoveryError> {
    GoVersion::parse(raw).ok_or_else(|| GoDiscoveryError::Unsupported {
        file,
        directive: "go",
        value: raw.to_owned(),
        reason: "expected a Go version such as 1.22 or 1.22.3",
        help: format!("run `go {} edit -go=<version>`", edit(file)),
    })
}

fn toolchain(file: &'static str, raw: &str) -> Result<Option<GoVersion>, GoDiscoveryError> {
    if raw == "default" {
        // No preference beyond the default (managed) toolchain.
        return Ok(None);
    }
    let unsupported = |reason| GoDiscoveryError::Unsupported {
        file,
        directive: "toolchain",
        value: raw.to_owned(),
        reason,
        help: format!(
            "run `go {} edit -toolchain=go1.N.P` with an official release",
            edit(file)
        ),
    };
    let version = raw
        .strip_prefix("go")
        .ok_or_else(|| unsupported("expected a toolchain name such as go1.22.3"))?;
    if version.contains('-') {
        return Err(unsupported(
            "turbo setup provisions only official Go toolchains, not custom goV-suffix builds",
        ));
    }
    GoVersion::parse(version)
        .filter(|version| version.is_toolchain())
        .map(Some)
        .ok_or_else(|| unsupported("not an official toolchain name (go1.N.P or go1.NrcR)"))
}

/// Root toolchain requirement. Without a `toolchain` preference, `minimum`
/// resolves like Go's own toolchain switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoRequirements {
    minimum: GoVersion,
    preferred: Option<GoVersion>,
    sources: Vec<GoSource>,
}

impl GoRequirements {
    /// None when neither root file exists. `read` returns bounded bytes.
    pub fn read_with(
        mut read: impl FnMut(&'static str, usize) -> io::Result<Option<Vec<u8>>>,
    ) -> Result<Option<Self>, GoDiscoveryError> {
        let mut load = |file: &'static str| -> Result<Option<Directives>, GoDiscoveryError> {
            let bytes = read(file, MAX_MANIFEST_BYTES).map_err(|_| GoDiscoveryError::Read(file))?;
            let Some(bytes) = bytes else {
                return Ok(None);
            };
            let text = String::from_utf8(bytes).map_err(|_| GoDiscoveryError::Malformed {
                file,
                line: 0,
                reason: "invalid UTF-8",
                edit: edit(file),
            })?;
            directives(file, &text).map(Some)
        };
        let work = load(GO_WORK)?;
        let module = load(GO_MOD)?;
        let (file, governing) = match (&work, &module) {
            (Some(work), _) => (GO_WORK, work),
            (None, Some(module)) => (GO_MOD, module),
            (None, None) => return Ok(None),
        };
        let mut sources = Vec::new();
        let mut declared = |file: &'static str, raw: &Option<String>| {
            sources.push(GoSource {
                file,
                field: "go",
                request: raw.clone(),
            });
            // Go's implicit minimums for files without a `go` line.
            let implicit = if file == GO_WORK { "1.18" } else { "1.16" };
            minimum(file, raw.as_deref().unwrap_or(implicit))
        };
        let required = declared(file, &governing.go)?;
        let root = match (&work, &module) {
            (Some(work), Some(module)) if work.uses_root => Some(declared(GO_MOD, &module.go)?),
            _ => None,
        };
        let preferred = match &governing.toolchain {
            Some(raw) => {
                sources.push(GoSource {
                    file,
                    field: "toolchain",
                    request: Some(raw.clone()),
                });
                toolchain(file, raw)?
            }
            None => None,
        };
        if let Some(root) = root.filter(|root| *root > required) {
            return Err(GoDiscoveryError::Conflict(format!(
                "root go.mod requires go >= {root}, but go.work lists go {required}; run `go work \
                 use` to raise go.work"
            )));
        }
        if let Some(preferred) = preferred.filter(|preferred| *preferred < required) {
            return Err(GoDiscoveryError::Conflict(format!(
                "{file} toolchain go{preferred} is older than its go {required} minimum; run `go \
                 {} edit -toolchain=<go{required} or newer>`",
                edit(file)
            )));
        }
        Ok(Some(Self {
            minimum: required,
            preferred,
            sources,
        }))
    }

    pub fn sources(&self) -> &[GoSource] {
        &self.sources
    }

    /// An exact `toolchain` preference wins. Otherwise apply Go's switch rule
    /// for a minimum: the oldest of the latest patches of the previous and
    /// newest released language versions, or the newest candidate of an
    /// unreleased one, that satisfies it.
    pub fn select(&self, published: &[GoVersion]) -> Result<GoVersion, GoDiscoveryError> {
        let missing = || GoDiscoveryError::NoMatchingRelease(self.describe());
        if let Some(preferred) = self.preferred {
            return published
                .contains(&preferred)
                .then_some(preferred)
                .ok_or_else(missing);
        }
        let newest = published.iter().filter(|v| v.is_release()).max().copied();
        let previous = newest.and_then(|newest| {
            published
                .iter()
                .filter(|v| v.is_release() && v.minor + 1 == newest.minor)
                .max()
                .copied()
        });
        let candidate = published
            .iter()
            .filter(|v| matches!(v.stage, Stage::Candidate(_)))
            .filter(|v| newest.is_none_or(|newest| v.minor > newest.minor))
            .max()
            .copied();
        [previous, newest, candidate]
            .into_iter()
            .flatten()
            .filter(|v| *v >= self.minimum)
            .min()
            .ok_or_else(missing)
    }

    /// Frozen check of an existing selection against unchanged declarations.
    pub fn matches_locked(&self, version: GoVersion) -> bool {
        version.is_toolchain()
            && self
                .preferred
                .map_or(version >= self.minimum, |preferred| version == preferred)
    }

    fn describe(&self) -> String {
        self.sources
            .iter()
            .map(|s| {
                format!(
                    "{} {} {}",
                    s.file,
                    s.field,
                    s.request.as_deref().unwrap_or("(implicit)")
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests;
