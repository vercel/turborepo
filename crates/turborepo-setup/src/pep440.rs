//! Bounded release-only PEP 440 version specifiers for uv and Python requests.
//! Candidates are final releases. Epoch, pre-, post-, dev- and local segments
//! are rejected rather than approximated.

use std::cmp::Ordering;

pub const MAX_SPECIFIER_BYTES: usize = 4096;
const MAX_CLAUSES: usize = 32;
const MAX_SEGMENTS: usize = 8;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("version specifier exceeds {MAX_SPECIFIER_BYTES} bytes or {MAX_CLAUSES} clauses")]
    Limit,
    #[error(
        "unsupported version specifier {0:?}; use release-only PEP 440 clauses such as >=0.5, \
         ==0.5.*, or ~=0.5.1"
    )]
    Unsupported(String),
}

/// Final release segments, compared with PEP 440 zero padding (`0.5 == 0.5.0`).
#[derive(Clone, Debug)]
pub struct Release(Vec<u64>);

impl Release {
    /// Dotted decimal segments only: no sign, whitespace, `v` prefix or suffix.
    pub fn parse(value: &str) -> Option<Self> {
        let segments = value
            .split('.')
            .map(|part| {
                (!part.is_empty() && part.len() <= 19 && part.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| part.parse().ok())
                    .flatten()
            })
            .collect::<Option<Vec<u64>>>()?;
        (segments.len() <= MAX_SEGMENTS).then_some(Self(segments))
    }

    pub fn segments(&self) -> &[u64] {
        &self.0
    }

    fn starts_with(&self, prefix: &[u64]) -> bool {
        prefix
            .iter()
            .enumerate()
            .all(|(i, part)| self.0.get(i).copied().unwrap_or(0) == *part)
    }
}

/// Release segments only; callers reject prerelease/build identities first.
impl From<&semver::Version> for Release {
    fn from(version: &semver::Version) -> Self {
        Self(vec![version.major, version.minor, version.patch])
    }
}

impl Ord for Release {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.0.len().max(other.0.len());
        (0..len)
            .map(|i| {
                let a = self.0.get(i).copied().unwrap_or(0);
                a.cmp(&other.0.get(i).copied().unwrap_or(0))
            })
            .find(|order| order.is_ne())
            .unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for Release {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Release {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Release {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Equal,
    EqualPrefix,
    NotEqual,
    NotEqualPrefix,
    GreaterEqual,
    LessEqual,
    Greater,
    Less,
    Compatible,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Clause {
    operator: Operator,
    release: Release,
}

impl Clause {
    fn contains(&self, candidate: &Release) -> bool {
        let release = &self.release;
        match self.operator {
            Operator::Equal => candidate == release,
            Operator::EqualPrefix => candidate.starts_with(&release.0),
            Operator::NotEqual => candidate != release,
            Operator::NotEqualPrefix => !candidate.starts_with(&release.0),
            Operator::GreaterEqual => candidate >= release,
            Operator::LessEqual => candidate <= release,
            Operator::Greater => candidate > release,
            Operator::Less => candidate < release,
            Operator::Compatible => {
                candidate >= release && candidate.starts_with(&release.0[..release.0.len() - 1])
            }
        }
    }
}

/// Comma-separated clauses, all of which must match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Specifiers(Vec<Clause>);

impl Specifiers {
    pub fn parse(input: &str) -> Result<Self, Error> {
        if input.len() > MAX_SPECIFIER_BYTES {
            return Err(Error::Limit);
        }
        let unsupported = || Error::Unsupported(input.to_owned());
        let raw: Vec<_> = input.split(',').collect();
        if raw.len() > MAX_CLAUSES {
            return Err(Error::Limit);
        }
        let mut clauses = Vec::with_capacity(raw.len());
        for clause in raw {
            let clause = clause.trim_matches(|c: char| c == ' ' || c == '\t');
            // Longest operators first; `===` is arbitrary string equality.
            let (operator, rest) = [
                ("~=", Operator::Compatible),
                ("==", Operator::Equal),
                ("!=", Operator::NotEqual),
                (">=", Operator::GreaterEqual),
                ("<=", Operator::LessEqual),
                (">", Operator::Greater),
                ("<", Operator::Less),
            ]
            .into_iter()
            .find_map(|(spelling, op)| clause.strip_prefix(spelling).map(|rest| (op, rest)))
            .ok_or_else(unsupported)?;
            let rest = rest.trim_start_matches([' ', '\t']);
            let (operator, version) = match (operator, rest.strip_suffix(".*")) {
                (Operator::Equal, Some(prefix)) => (Operator::EqualPrefix, prefix),
                (Operator::NotEqual, Some(prefix)) => (Operator::NotEqualPrefix, prefix),
                (_, Some(_)) => return Err(unsupported()),
                (operator, None) => (operator, rest),
            };
            let release = Release::parse(version).ok_or_else(unsupported)?;
            if operator == Operator::Compatible && release.0.len() < 2 {
                return Err(unsupported());
            }
            clauses.push(Clause { operator, release });
        }
        Ok(Self(clauses))
    }

    pub fn exact(release: Release) -> Self {
        Self(vec![Clause {
            operator: Operator::Equal,
            release,
        }])
    }

    pub fn contains(&self, candidate: &Release) -> bool {
        self.0.iter().all(|clause| clause.contains(candidate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(specifiers: &str, version: &str) -> bool {
        Specifiers::parse(specifiers)
            .unwrap()
            .contains(&Release::parse(version).unwrap())
    }

    #[test]
    fn release_only_operators_follow_pep_440_padding() {
        for (specifiers, version, expected) in [
            ("==0.5", "0.5.0", true),
            ("==0.5.1", "0.5.0", false),
            ("==0.5.*", "0.5.9", true),
            ("==0.5.*", "0.6.0", false),
            ("!=0.5.*", "0.6.0", true),
            ("!=0.5.0", "0.5", false),
            (">=0.5, <0.6", "0.5.7", true),
            (" >= 0.5 ,< 0.6 ", "0.6.0", false),
            (">0.5", "0.5.0", false),
            ("<=0.5", "0.5.0", true),
            ("~=0.5.1", "0.5.9", true),
            ("~=0.5.1", "0.6.0", false),
            ("~=0.5", "0.9.0", true),
            ("~=0.5", "1.0.0", false),
            ("==1.*", "1.20.3", true),
        ] {
            assert_eq!(
                matches(specifiers, version),
                expected,
                "{specifiers} {version}"
            );
        }
    }

    #[test]
    fn unsupported_components_fail_closed() {
        for input in [
            "",
            ",",
            "0.5",
            "===0.5",
            "~=1",
            ">=0.5.*",
            "==0.5.0rc1",
            ">=1!2.0",
            "==0.5+local",
            ">=v0.5",
            "==0.5.0.post1",
            "=0.5",
            ">=0.5;",
            ">=-1",
            "==0..5",
        ] {
            assert!(
                matches!(Specifiers::parse(input), Err(Error::Unsupported(_))),
                "{input}"
            );
        }
        assert_eq!(
            Specifiers::parse(&format!(">={}", "1".repeat(MAX_SPECIFIER_BYTES))),
            Err(Error::Limit)
        );
        assert_eq!(Specifiers::parse(&[">=1"; 33].join(",")), Err(Error::Limit));
        assert!(Release::parse("1.2.3.4.5.6.7.8.9").is_none());
        assert!(Release::parse("99999999999999999999").is_none());
    }
}
