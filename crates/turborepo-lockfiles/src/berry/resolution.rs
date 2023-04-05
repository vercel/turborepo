use std::fmt;

use pest::{iterators::Pair, Parser};
use pest_derive::Parser;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unable to parse")]
    ParseError(#[from] pest::error::Error<Rule>),
    #[error("unexpected end of input")]
    UnexpectedEOI,
    #[error("unexpected token")]
    UnexpectedToken(Rule),
}

#[derive(Debug, PartialEq, Clone, Copy, Eq, Default)]
pub struct Resolution<'a> {
    from: Option<Specifier<'a>>,
    descriptor: Specifier<'a>,
}

#[derive(Debug, PartialEq, Clone, Copy, Eq, Default)]
struct Specifier<'a> {
    full_name: &'a str,
    description: Option<&'a str>,
}

#[derive(Parser)]
#[grammar = "src/berry/resolution.pest"]
struct ResolutionParser;

pub fn parse_resolution(resolution: &str) -> Result<Resolution, Error> {
    let resolution = ResolutionParser::parse(Rule::resolution, resolution)?
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

impl fmt::Display for Resolution<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(from) = &self.from {
            f.write_fmt(format_args!("{from}/"))?;
        }
        f.write_fmt(format_args!("{}", self.descriptor))?;
        todo!()
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
        )
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
}
