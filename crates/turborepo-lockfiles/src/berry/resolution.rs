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
    #[error("unable to parse")]
    // Boxed due to this enum variant being much larger than the others
    Pest(#[from] Box<pest::error::Error<Rule>>),
    #[error("unexpected end of input")]
    UnexpectedEOI,
    #[error("unexpected token")]
    UnexpectedToken(Rule),
}

/// A resolution that can appear in the resolutions field of the top level
/// package.json
#[derive(Debug, PartialEq, Clone, Copy, Eq, Default, PartialOrd, Ord, Hash)]
pub struct Resolution<'a> {
    from: Option<Specifier<'a>>,
    descriptor: Specifier<'a>,
}

// This is essentially an Ident with an optional semver range
#[derive(Debug, PartialEq, Clone, Copy, Eq, Default, PartialOrd, Ord, Hash)]
struct Specifier<'a> {
    full_name: &'a str,
    description: Option<&'a str>,
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
            Ok(Some(Specifier {
                full_name,
                description,
            }))
        }
        Rule::EOI => Ok(None),
        _ => Err(Error::UnexpectedToken(specifier.as_rule())),
    }
}

impl<'a> Resolution<'a> {
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
            if from_ident != locator.ident {
                return None;
            }

            let mut from_locator = Locator {
                ident: from_ident,
                reference: from
                    .description
                    .map_or_else(|| locator.reference.to_string(), |desc| desc.to_string())
                    .into(),
            };

            // we now insert the default protocol if one isn't present
            if Version::parse(&from_locator.reference).is_ok()
                || tag_regex().is_match(&from_locator.reference)
            {
                let reference = from_locator.reference.to_mut();
                reference.insert_str(0, "npm:");
            }

            // If the normalized from locator doesn't match the package we're currently
            // processing, we skip
            if &from_locator != locator {
                return None;
            }
        }

        // Note: berry parses this as a locator even though it's an ident
        let resolution_ident = self.descriptor.ident();
        if resolution_ident != dependency.ident {
            return None;
        }

        let resolution_descriptor = Descriptor {
            ident: resolution_ident,
            range: self
                .descriptor
                .description
                .map_or_else(|| dependency.range.to_string(), |range| range.to_string())
                .into(),
        };

        if &resolution_descriptor != dependency {
            return None;
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
}

impl<'a> Specifier<'a> {
    fn ident(&self) -> Ident<'a> {
        Ident::try_from(self.full_name).expect("Invalid identifier in resolution")
    }
}

impl fmt::Display for Resolution<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(from) = &self.from {
            f.write_fmt(format_args!("{from}/"))?;
        }
        f.write_fmt(format_args!("{}", self.descriptor))?;
        Ok(())
    }
}

impl fmt::Display for Specifier<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.full_name)?;
        if let Some(descriptor) = self.description {
            f.write_fmt(format_args!("@{descriptor}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
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
                descriptor: Specifier {
                    full_name: "relay-compiler",
                    description: None
                }
            }
        )
    }

    #[test]
    fn test_descriptor_with_scope() {
        assert_eq!(
            parse_resolution("@babel/core").unwrap(),
            Resolution {
                from: None,
                descriptor: Specifier {
                    full_name: "@babel/core",
                    description: None
                }
            }
        )
    }

    #[test]
    fn test_from_and_descriptor_only() {
        assert_eq!(
            parse_resolution("webpack/memory-fs").unwrap(),
            Resolution {
                from: Some(Specifier {
                    full_name: "webpack",
                    description: None
                }),
                descriptor: Specifier {
                    full_name: "memory-fs",
                    description: None
                }
            }
        );

        assert_eq!(
            parse_resolution("is-even/is-odd").unwrap(),
            Resolution {
                from: Some(Specifier {
                    full_name: "is-even",
                    description: None
                }),
                descriptor: Specifier {
                    full_name: "is-odd",
                    description: None
                }
            }
        );
    }

    #[test]
    fn test_descriptor_with_version() {
        assert_eq!(
            parse_resolution("@babel/core@npm:7.0.0/@babel/generator").unwrap(),
            Resolution {
                from: Some(Specifier {
                    full_name: "@babel/core",
                    description: Some("npm:7.0.0"),
                }),
                descriptor: Specifier {
                    full_name: "@babel/generator",
                    description: None
                }
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
}
