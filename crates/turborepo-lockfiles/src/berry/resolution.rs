use std::{fmt, sync::OnceLock};

use pest::{iterators::Pair, Parser};
use pest_derive::Parser;
use regex::Regex;
use semver::Version;
use thiserror::Error;

use super::identifiers::{Descriptor, Ident, Locator};

fn tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[^v][a-z0-9._-]*$").unwrap())
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("unable to parse: {0}")]
    // Boxed due to this enum variant being much larger than the others
    Pest(#[from] Box<pest::error::Error<Rule>>),
    #[error("unexpected end of input")]
    UnexpectedEOI,
    #[error("unexpected token")]
    UnexpectedToken(Rule),
    #[error("invalid identifier used as specifier: {0}")]
    InvalidSpecifier(#[from] super::identifiers::Error),
}

/// A resolution that can appear in the resolutions field of the top level
/// package.json
#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Hash)]
pub struct Resolution {
    from: Option<Specifier>,
    descriptor: Specifier,
}

// This is essentially an Ident with an optional semver range
#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord, Hash)]
struct Specifier {
    full_name: String,
    description: Option<String>,
    ident: Ident<'static>,
}

#[derive(Parser)]
#[grammar = "src/berry/resolution.pest"]
struct ResolutionParser;

pub fn parse_resolution(resolution: &str) -> Result<Resolution, Error> {
    let resolution = ResolutionParser::parse(Rule::resolution, resolution)
        .map_err(Box::new)?
        .next()
        .ok_or(Error::UnexpectedEOI)?;

    match resolution.as_rule() {
        Rule::resolution => {
            let mut specifiers = resolution.into_inner();
            let s1 = parse_specifier(specifiers.next().ok_or(Error::UnexpectedEOI)?)?
                .ok_or(Error::UnexpectedEOI)?;
            let s2 = specifiers
                .next()
                .map(parse_specifier)
                .transpose()?
                .flatten();
            Ok(if let Some(s2) = s2 {
                Resolution {
                    from: Some(s1),
                    descriptor: s2,
                }
            } else {
                Resolution {
                    from: None,
                    descriptor: s1,
                }
            })
        }
        Rule::EOI => Err(Error::UnexpectedEOI),
        _ => Err(Error::UnexpectedToken(resolution.as_rule())),
    }
}

fn parse_specifier(specifier: Pair<'_, Rule>) -> Result<Option<Specifier>, Error> {
    match specifier.as_rule() {
        Rule::specifier => {
            let mut parts = specifier.into_inner();
            let full_name = parts.next().ok_or(Error::UnexpectedEOI)?.as_str();
            let description = parts.next().map(|p| p.as_str());
            let spec = Specifier::new(full_name, description)?;
            Ok(Some(spec))
        }
        Rule::EOI => Ok(None),
        _ => Err(Error::UnexpectedToken(specifier.as_rule())),
    }
}

impl Resolution {
    /// Returns a new descriptor if an override is applicable
    // reference: version that this resolution resolves to
    // locator: package that depends on the dependency
    // dependency: package that we are considering overriding
    pub fn reduce_dependency<'b>(
        &self,
        reference: &str,
        dependency: &Descriptor<'b>,
        locator: &Locator,
    ) -> Option<Descriptor<'b>> {
        // if the ref is patch locator then it will have a suffix of
        // ::locator={ROOT_WORKSPACE}@workspace:. ( with @ and : escaped)
        if let Some(from) = &self.from {
            let from_ident = from.ident();
            // If the from doesn't match the locator we skip
            if from_ident != &locator.ident {
                return None;
            }

            // Since we have already checked the ident portion of the locator for equality
            // we can avoid an allocation caused by constructing a locator by just checking
            // the reference portion.
            if let Some(desc) = &from.description {
                if !Self::eq_with_protocol(&locator.reference, desc, "npm:") {
                    return None;
                }
            }
        }

        // Note: berry parses this as a locator even though it's an ident
        let resolution_ident = self.descriptor.ident();
        if resolution_ident != &dependency.ident {
            return None;
        }

        if let Some(resolution_range) = &self.descriptor.description {
            if resolution_range != &dependency.range
                // Yarn4 encodes the default npm protocol in yarn.lock, but not in resolutions field of package.json
                // We check if the ranges match when we add `npm:` to range coming from resolutions.
                && !Self::eq_with_protocol(&dependency.range, resolution_range, "npm:")
            {
                return None;
            }
        }

        // We have a match an we now override the dependency
        let mut dependency_override = dependency.clone();
        dependency_override.range = reference.to_string().into();
        if Version::parse(reference).is_ok() || tag_regex().is_match(reference) {
            dependency_override.range.to_mut().insert_str(0, "npm:")
        }

        // Patch references aren't complete in the resolutions field so we
        // instead resolve to the package getting patched.
        // The patch still gets picked up as we include patches for any
        // packages in the pruned lockfile if the package is a member.
        if matches!(dependency_override.protocol(), Some("patch")) {
            return Some(
                Descriptor::from(
                    Locator::from_patch_reference(reference)
                        .expect("expected patch reference to contain locator"),
                )
                .into_owned(),
            );
        }

        Some(dependency_override)
    }

    // Checks if two references are equal with a default protocol
    // Avoids any allocations
    fn eq_with_protocol(
        reference: &str,
        incomplete_reference: &str,
        default_protocol: &str,
    ) -> bool {
        match Version::parse(incomplete_reference).is_ok()
            || tag_regex().is_match(incomplete_reference)
        {
            // We need to inject a protocol
            true => {
                if let Some(stripped_reference) = reference.strip_prefix(default_protocol) {
                    stripped_reference == incomplete_reference
                } else {
                    // The reference doesn't use the default protocol so adding it to the incomplete
                    // reference would result in the references being different
                    false
                }
            }
            // The protocol is already present
            false => reference == incomplete_reference,
        }
    }
}

impl Specifier {
    pub fn new(full_name: &str, description: Option<&str>) -> Result<Specifier, Error> {
        let ident = Ident::try_from(full_name)?.to_owned();

        Ok(Specifier {
            full_name: full_name.to_string(),
            description: description.map(|s| s.to_string()),
            ident,
        })
    }

    pub fn ident(&self) -> &Ident<'static> {
        &self.ident
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(from) = &self.from {
            f.write_fmt(format_args!("{from}/"))?;
        }
        f.write_fmt(format_args!("{}", self.descriptor))?;
        Ok(())
    }
}

impl fmt::Display for Specifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.full_name)?;
        if let Some(descriptor) = &self.description {
            f.write_fmt(format_args!("@{descriptor}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    #[test]
    fn test_full_name() {
        assert_eq!(
            ResolutionParser::parse(Rule::fullName, "relay-compiler")
                .unwrap()
                .as_str(),
            "relay-compiler"
        );
        assert_eq!(
            ResolutionParser::parse(Rule::fullName, "@babel/types")
                .unwrap()
                .as_str(),
            "@babel/types"
        );
    }

    #[test]
    fn test_specifier() {
        assert_eq!(
            ResolutionParser::parse(Rule::specifier, "replay-compiler")
                .unwrap()
                .as_str(),
            "replay-compiler"
        );
        assert_eq!(
            ResolutionParser::parse(Rule::specifier, "replay-compiler@npm:1.0.0")
                .unwrap()
                .as_str(),
            "replay-compiler@npm:1.0.0"
        );
    }

    #[test]
    fn test_descriptor_only() {
        assert_eq!(
            parse_resolution("relay-compiler").unwrap(),
            Resolution {
                from: None,
                descriptor: Specifier::new("relay-compiler", None).unwrap()
            }
        )
    }

    #[test]
    fn test_descriptor_with_scope() {
        assert_eq!(
            parse_resolution("@babel/core").unwrap(),
            Resolution {
                from: None,
                descriptor: Specifier::new("@babel/core", None,).unwrap()
            }
        )
    }

    #[test]
    fn test_from_and_descriptor_only() {
        assert_eq!(
            parse_resolution("webpack/memory-fs").unwrap(),
            Resolution {
                from: Some(Specifier::new("webpack", None).unwrap()),
                descriptor: Specifier::new("memory-fs", None).unwrap()
            }
        );

        assert_eq!(
            parse_resolution("is-even/is-odd").unwrap(),
            Resolution {
                from: Some(Specifier::new("is-even", None).unwrap()),
                descriptor: Specifier::new("is-odd", None).unwrap()
            }
        );
    }

    #[test]
    fn test_descriptor_with_version() {
        assert_eq!(
            parse_resolution("@babel/core@npm:7.0.0/@babel/generator").unwrap(),
            Resolution {
                from: Some(Specifier::new("@babel/core", Some("npm:7.0.0"),).unwrap()),
                descriptor: Specifier::new("@babel/generator", None).unwrap()
            }
        )
    }

    #[test]
    fn test_patch_resolution() {
        let resolution = parse_resolution("lodash@^4.17.21").unwrap();
        let dependency = resolution.reduce_dependency(
            "patch:lodash@npm%3A4.17.21#./.yarn/patches/lodash-npm-4.17.21-6382451519.patch",
            &Descriptor::try_from("lodash@^4.17.21").unwrap(),
            &Locator::try_from("test@workspace:.").unwrap(),
        );
        assert_eq!(
            dependency,
            Some(Descriptor::try_from("lodash@npm:4.17.21").unwrap())
        );
    }

    #[test_case("proto:1.0.0", "proto:1.0.0", true ; "identical")]
    #[test_case("proto:1.0.0", "1.0.0", true ; "use default")]
    #[test_case("proto:1.0.0", "other:1.0.0", false ; "different protocols")]
    #[test_case("other:1.0.0", "1.0.0", false ; "non-default protocols")]
    #[test_case("proto:1.0.0", "proto:1.2.3", false ; "mismatched ref")]
    #[test_case("proto:1.0.0", "1.2.3", false ; "mismatched ref with protocol")]
    fn test_reference_eq(a: &str, b: &str, expected: bool) {
        assert_eq!(Resolution::eq_with_protocol(a, b, "proto:"), expected);
    }
}
