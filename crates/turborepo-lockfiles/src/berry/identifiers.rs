use lazy_static::lazy_static;
use regex::Regex;
use thiserror::Error;

lazy_static! {
    static ref IDENT: Regex = Regex::new(r"^(?:@([^/]+?)/)?([^@/]+)$").unwrap();
    static ref DESCRIPTOR: Regex = Regex::new(r"^(?:@([^/]+?)/)?([^@/]+?)(?:@(.+))$").unwrap();
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid descriptor ({0})")]
    Ident(String),
    #[error("Invalid descriptor ({0})")]
    Descriptor(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ident<'a> {
    scope: Option<&'a str>,
    name: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Descriptor<'a> {
    ident: Ident<'a>,
    range: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Locator<'a> {
    ident: Ident<'a>,
    reference: &'a str,
}

impl<'a> TryFrom<&'a str> for Ident<'a> {
    type Error = Error;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let make_err = || Error::Ident(value.to_string());
        let captures = IDENT.captures(value).ok_or_else(make_err)?;
        let scope = captures.get(1).map(|m| m.as_str());
        let name = captures.get(2).map(|m| m.as_str()).ok_or_else(make_err)?;
        Ok(Self { scope, name })
    }
}

impl<'a> TryFrom<&'a str> for Descriptor<'a> {
    type Error = Error;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let make_err = || Error::Descriptor(value.to_string());
        let captures = DESCRIPTOR.captures(value).ok_or_else(make_err)?;
        let scope = captures.get(1).map(|m| m.as_str());
        let name = captures.get(2).map(|m| m.as_str()).ok_or_else(make_err)?;
        let range = captures.get(3).map(|m| m.as_str()).ok_or_else(make_err)?;
        let ident = Ident { scope, name };
        Ok(Descriptor { ident, range })
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_parse_ident_with_scope() {
        assert_eq!(
            Ident::try_from("@babel/parser").unwrap(),
            Ident {
                scope: Some("babel"),
                name: "parser"
            }
        )
    }

    #[test]
    fn test_parse_ident_without_scope() {
        assert_eq!(
            Ident::try_from("turbo").unwrap(),
            Ident {
                scope: None,
                name: "turbo"
            }
        )
    }

    #[test]
    fn test_parse_descriptor() {
        assert_eq!(
            Descriptor::try_from("@babel/code-frame@npm:7.12.11").unwrap(),
            Descriptor {
                ident: Ident {
                    scope: Some("babel"),
                    name: "code-frame"
                },
                range: "npm:7.12.11",
            }
        )
    }
}
