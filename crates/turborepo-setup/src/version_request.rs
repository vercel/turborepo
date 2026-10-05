//! Strict npm-style version requests for setup discovery, without runtime I/O.
//!
//! Supports exact/partial versions, trailing wildcards, comparators, caret,
//! tilde (including `~>`), standalone hyphen ranges, and `||`. Whitespace joins
//! comparators with AND; branches are OR. Impossible intersections remain
//! empty, never simplified into weaker constraints. Even `>=0.0.0` is retained:
//! stable-only equivalences must not erase bounds on prereleases. npm's default
//! prerelease gating and build-insensitive precedence apply. Numeric prerelease
//! identifiers compare exactly (no JavaScript rounding); wildcard OR branches
//! are not collapsed over explicit prerelease branches. Empty
//! requests/branches, aliases, loose syntax, non-trailing wildcards, suffixes
//! on partial versions, and non-ASCII syntax are rejected.

use semver::{Prerelease, Version};
use thiserror::Error;

pub const MAX_REQUEST_BYTES: usize = 4096;
pub const MAX_REQUEST_BRANCHES: usize = 32;
pub const MAX_BRANCH_TERMS: usize = 64;
const MAX_VERSION_BYTES: usize = 256;
const MAX_COMPONENT: u64 = 9_007_199_254_740_991; // npm's Number.MAX_SAFE_INTEGER

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum VersionRequestError {
    #[error("version request exceeds setup's byte, branch, or term limit")]
    TooComplex,
    #[error("malformed or unsupported npm version request")]
    InvalidSyntax,
    #[error("version component or expanded bound exceeds npm's safe integer limit")]
    ComponentOverflow,
}

/// Validated OR-of-AND constraints. No catalog lookup or alias resolution
/// occurs.
#[derive(Debug, Clone)]
pub struct VersionRequest {
    branches: Vec<Vec<Comparator>>,
    exact: Option<Version>,
}

impl VersionRequest {
    pub fn parse(input: &str) -> Result<Self, VersionRequestError> {
        if input.len() > MAX_REQUEST_BYTES {
            return Err(VersionRequestError::TooComplex);
        }
        if !input.is_ascii() || input.trim().is_empty() {
            return Err(VersionRequestError::InvalidSyntax);
        }
        let mut branches = Vec::new();
        let mut exact = None;
        for branch in input.split("||") {
            if branches.len() == MAX_REQUEST_BRANCHES {
                return Err(VersionRequestError::TooComplex);
            }
            let (comparators, terms) = parse_branch(branch)?;
            if branches.is_empty() && terms == 1 && comparators.len() == 1 {
                let comparator = &comparators[0];
                if comparator.op == Op::Eq {
                    exact = Some(comparator.version.clone());
                }
            } else {
                exact = None;
            }
            branches.push(comparators);
        }
        Ok(Self { branches, exact })
    }

    pub fn is_exact(&self) -> bool {
        self.exact.is_some()
    }

    /// Canonical exact request, including authored build metadata, if any.
    pub fn exact_version(&self) -> Option<&Version> {
        self.exact.as_ref()
    }

    /// Match an injected canonical release using npm precedence (ignores
    /// build). Non-npm candidates (unsafe components or more than 256
    /// bytes) never match.
    pub fn matches(&self, release: &Version) -> bool {
        self.matches_with_exact(release, |_, _| true)
    }

    /// Let discovery refine exact-release identity, e.g. `|a, b| a == b` to
    /// distinguish build metadata. The hook cannot widen npm precedence
    /// equality. Only a single authored exact request uses it; range
    /// comparators always retain npm semantics. The hook receives
    /// (requested, release).
    pub fn matches_with_exact(
        &self,
        release: &Version,
        exact_matches: impl FnOnce(&Version, &Version) -> bool,
    ) -> bool {
        let core = [release.major, release.minor, release.patch];
        let suffix = release.pre.len().saturating_add(release.build.len());
        if core.iter().any(|&n| n > MAX_COMPONENT) || suffix > MAX_VERSION_BYTES {
            return false;
        }
        // Count the canonical encoding without allocating an unbounded string.
        let bytes = core
            .iter()
            .map(|n| n.checked_ilog10().unwrap_or(0) as usize + 1)
            .sum::<usize>()
            + 2
            + suffix
            + usize::from(!release.pre.is_empty())
            + usize::from(!release.build.is_empty());
        if bytes > MAX_VERSION_BYTES {
            return false;
        }
        if let Some(exact) = &self.exact {
            return exact.cmp_precedence(release).is_eq() && exact_matches(exact, release);
        }
        self.branches.iter().any(|branch| {
            branch.iter().all(|c| c.matches(release))
                && (release.pre.is_empty()
                    || branch.iter().any(|c| {
                        !c.version.pre.is_empty()
                            && (c.version.major, c.version.minor, c.version.patch)
                                == (release.major, release.minor, release.patch)
                    }))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Gt,
    Ge,
    Lt,
    Le,
    Caret,
    Tilde,
}

#[derive(Debug, Clone)]
struct Comparator {
    op: Op,
    version: Version,
}

impl Comparator {
    fn matches(&self, release: &Version) -> bool {
        let order = release.cmp_precedence(&self.version);
        match self.op {
            Op::Eq => order.is_eq(),
            Op::Gt => order.is_gt(),
            Op::Ge => !order.is_lt(),
            Op::Lt => order.is_lt(),
            Op::Le => !order.is_gt(),
            Op::Caret | Op::Tilde => unreachable!("operators expanded at parse time"),
        }
    }
}

struct Partial {
    version: Version,
    precision: usize,
}

impl Partial {
    fn parse(input: &str) -> Result<Self, VersionRequestError> {
        if input.len() > MAX_VERSION_BYTES {
            return Err(VersionRequestError::TooComplex);
        }
        let input = input.strip_prefix('v').unwrap_or(input);
        let core_end = input.find(['-', '+']).unwrap_or(input.len());
        let mut numbers = [0; 3];
        let mut precision = 0;
        let mut wildcard = false;
        let mut components = 0;
        for (index, part) in input[..core_end].split('.').enumerate() {
            if index >= 3 {
                return Err(VersionRequestError::InvalidSyntax);
            }
            components += 1;
            if matches!(part, "x" | "X" | "*") {
                wildcard = true;
            } else {
                if wildcard
                    || part.is_empty()
                    || !part.bytes().all(|b| b.is_ascii_digit())
                    || (part.len() > 1 && part.starts_with('0'))
                {
                    return Err(VersionRequestError::InvalidSyntax);
                }
                let number = part
                    .parse::<u64>()
                    .map_err(|_| VersionRequestError::ComponentOverflow)?;
                if number > MAX_COMPONENT {
                    return Err(VersionRequestError::ComponentOverflow);
                }
                numbers[index] = number;
                precision += 1;
            }
        }
        let version = if precision == 3 {
            Version::parse(input).map_err(|_| VersionRequestError::InvalidSyntax)?
        } else {
            if core_end != input.len() || components == 0 {
                return Err(VersionRequestError::InvalidSyntax);
            }
            Version::new(numbers[0], numbers[1], numbers[2])
        };
        Ok(Self { version, precision })
    }

    // Every expansion adds at most two comparators. No intersection products.
    fn expand(self, op: Op, into: &mut Vec<Comparator>) -> Result<(), VersionRequestError> {
        let precision = self.precision;
        let mut version = self.version;
        if precision == 0 {
            if matches!(op, Op::Lt | Op::Gt) {
                version.pre =
                    Prerelease::new("0").map_err(|_| VersionRequestError::InvalidSyntax)?;
                into.push(Comparator {
                    op: Op::Lt,
                    version,
                });
            }
            return Ok(());
        }
        if matches!(op, Op::Caret | Op::Tilde) || (op == Op::Eq && precision < 3) {
            let index = match op {
                Op::Caret if version.major != 0 || precision == 1 => 0,
                Op::Caret if version.minor != 0 || precision == 2 => 1,
                Op::Caret => 2,
                _ => precision.min(2) - 1,
            };
            let upper = upper_bound(&version, index)?;
            into.push(Comparator {
                op: Op::Ge,
                version,
            });
            into.push(Comparator {
                op: Op::Lt,
                version: upper,
            });
        } else if precision < 3 {
            let op = match op {
                Op::Gt => {
                    version = upper_bound(&version, precision - 1)?;
                    version.pre = Prerelease::EMPTY;
                    Op::Ge
                }
                Op::Le => {
                    version = upper_bound(&version, precision - 1)?;
                    Op::Lt
                }
                Op::Lt => {
                    version.pre =
                        Prerelease::new("0").map_err(|_| VersionRequestError::InvalidSyntax)?;
                    Op::Lt
                }
                _ => op,
            };
            into.push(Comparator { op, version });
        } else {
            into.push(Comparator { op, version });
        }
        Ok(())
    }
}

fn upper_bound(version: &Version, index: usize) -> Result<Version, VersionRequestError> {
    let mut core = [version.major, version.minor, version.patch];
    if core[index] == MAX_COMPONENT {
        return Err(VersionRequestError::ComponentOverflow);
    }
    core[index] += 1;
    core[index + 1..].fill(0);
    let mut upper = Version::new(core[0], core[1], core[2]);
    upper.pre = Prerelease::new("0").map_err(|_| VersionRequestError::InvalidSyntax)?;
    Ok(upper)
}

fn parse_branch(input: &str) -> Result<(Vec<Comparator>, usize), VersionRequestError> {
    let tokens: Vec<_> = input.split_whitespace().collect();
    if tokens.is_empty() {
        return Err(VersionRequestError::InvalidSyntax);
    }
    let mut comparators = Vec::new();
    if tokens.len() == 3 && tokens[1] == "-" {
        let from = Partial::parse(tokens[0])?;
        let to = Partial::parse(tokens[2])?;
        from.expand(Op::Ge, &mut comparators)?;
        to.expand(Op::Le, &mut comparators)?;
        return Ok((comparators, 2));
    }
    let mut tokens = tokens.into_iter();
    let mut terms = 0;
    while let Some(token) = tokens.next() {
        if terms == MAX_BRANCH_TERMS {
            return Err(VersionRequestError::TooComplex);
        }
        terms += 1;
        let (op, mut version) = [
            (">=", Op::Ge),
            ("<=", Op::Le),
            ("~", Op::Tilde),
            ("^", Op::Caret),
            (">", Op::Gt),
            ("<", Op::Lt),
            ("=", Op::Eq),
        ]
        .into_iter()
        .find_map(|(prefix, op)| token.strip_prefix(prefix).map(|tail| (op, tail)))
        .unwrap_or((Op::Eq, token));
        if version.is_empty() {
            version = tokens.next().ok_or(VersionRequestError::InvalidSyntax)?;
        }
        if op == Op::Tilde {
            version = version.strip_prefix('>').unwrap_or(version);
            if version.is_empty() {
                version = tokens.next().ok_or(VersionRequestError::InvalidSyntax)?;
                if version == "=" {
                    return Err(VersionRequestError::InvalidSyntax);
                }
            }
        }
        if matches!(op, Op::Caret | Op::Tilde) {
            version = version.strip_prefix('=').unwrap_or(version);
            if version.is_empty() {
                version = tokens.next().ok_or(VersionRequestError::InvalidSyntax)?;
            }
        }
        Partial::parse(version)?.expand(op, &mut comparators)?;
    }
    Ok((comparators, terms))
}

#[cfg(test)]
mod tests;
