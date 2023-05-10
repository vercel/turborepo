use pest::{
    iterators::{Pair, Pairs},
    Parser,
};
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "./yarn1/syml.pest"]
pub struct SymlParser;

#[derive(Debug)]
pub struct Resolution<'a> {
    pub names: Vec<&'a str>,
    pub version: &'a str,
    pub metadata: Vec<&'a str>,
}

/// Syml is a simplified version of yaml that is used in yarn v1.
enum Syml {
    Bool(bool),
    String(String),
    Object(Vec<(String, Syml)>),
}

impl Resolution<'_> {
    pub fn new<'a>(name: &'a str, version: &'a str, metadata: Vec<&'a str>) -> Resolution<'a> {
        Resolution {
            names: vec![name],
            version,
            metadata,
        }
    }

    fn parse<'a>(pair: Pair<'a, Rule>) -> Resolution<'a> {
        match pair.as_rule() {
            Rule::samedent_property => {
                let mut inner = pair.into_inner();
                let _samedent = inner.next().unwrap(); // todo(arlyon): intendation
                let name = inner.next().unwrap().as_str();
                let mut expression = inner.next().unwrap().into_inner();
                let version = Self::parse_metadata(&mut expression);
                Resolution::new(name, version, vec![])
            }
            Rule::legacy_property => {
                let mut inner = pair.into_inner();
                let _samedent = inner.next().unwrap(); // todo: indentation
                let name = Resolution::parse_legacy_name(&mut inner);
                let mut expression = inner.next().unwrap().into_inner();
                let version = Self::parse_metadata(&mut expression);

                Resolution::new(name, version, vec![])
            }
            Rule::legacy_multiple_property => {
                let mut inner = pair.into_inner();
                let _samedent = inner.next().unwrap(); // todo(arlyon): intendation

                let mut names = vec![];
                let mut expression = loop {
                    match inner.peek().unwrap().as_rule() {
                        Rule::legacy_name => {
                            names.push(Resolution::parse_legacy_name(&mut inner));
                        }
                        Rule::expression => break inner.next().unwrap().into_inner(),
                        _ => unreachable!(),
                    }
                };

                let version = Self::parse_metadata(&mut expression);

                Resolution {
                    names,
                    version,
                    metadata: vec![],
                }
            }
            _ => unreachable!(),
        }
    }

    /// in the case of an expression it is either a version,
    /// or an object with a number of fields
    fn parse_metadata<'a>(pairs: &mut Pairs<'a, Rule>) -> &'a str {
        match (pairs.next(), pairs.next()) {
            (Some(r1), Some(r2))
                if r1.as_rule() == Rule::eol && r2.as_rule() == Rule::extradent =>
            {
                Self::parse_item_statements(&mut pairs.next().unwrap().into_inner());
                ""
            }
            (Some(r1), Some(r2)) if r1.as_rule() == Rule::literal && r2.as_rule() == Rule::eol => {
                r1.as_str()
            }
            (Some(r1), Some(r2))
                if r1.as_rule() == Rule::eol && r2.as_rule() == Rule::property_statements =>
            {
                let (version, _, _) =
                    Self::parse_metadata_from_property_statements(&mut r2.into_inner());
                version.unwrap()
            }
            _ => unreachable!(),
        }
    }

    fn parse_item_statements<'a>(pairs: &mut Pairs<'a, Rule>) {
        println!("{:#?}", pairs);
    }

    fn parse_metadata_from_property_statements<'a>(
        pairs: &mut Pairs<'a, Rule>,
    ) -> (Option<&'a str>, Option<&'a str>, Option<&'a str>) {
        let mut version = None;
        let mut resolved = None;
        let mut integrity = None;

        for pair in pairs {
            let pair = pair.into_inner().next().unwrap();
            match pair.as_rule() {
                Rule::samedent_property => {
                    let mut inner = pair.into_inner();
                    let _samedent = inner.next().unwrap(); // todo(arlyon): intendation
                    let name = inner.next().unwrap().as_str();
                    let mut expression = inner.next().unwrap().into_inner();
                    println!("{:#?}", expression);
                    let value = match expression.next() {
                        Some(r) if r.as_rule() == Rule::literal => r.as_str(),
                        _ => panic!(),
                    };

                    match name {
                        "version" => version.replace(value),
                        "resolved" => resolved.replace(value),
                        "integrity" => integrity.replace(value),
                        _ => panic!(),
                    };
                }
                Rule::legacy_property | Rule::legacy_multiple_property => {} // ignored
                Rule::comment_line | Rule::version_spec => {}
                r => unreachable!("{:?}", r),
            };
        }

        (version, resolved, integrity)
    }

    fn parse_legacy_name<'a>(inner: &mut Pairs<'a, Rule>) -> &'a str {
        let name = inner.next().unwrap().into_inner().next().unwrap();
        match name.as_rule() {
            Rule::string => {
                let name = name.as_str();
                // this is guaranteed to have quotes
                &name[1..name.len() - 1]
            }
            Rule::legacy_pseudostring => name.as_str(),
            x => unreachable!("{:?}", x),
        }
    }
}

fn get_packages<'a>(lockfile: &'a str) -> Result<Vec<Resolution<'a>>, pest::error::Error<Rule>> {
    let mut property_statements = SymlParser::parse(Rule::grammar, lockfile)?
        .next()
        .expect("one grammar")
        .into_inner()
        .next()
        .expect("one set of property statements")
        .into_inner();

    let version = loop {
        match property_statements
            .peek()
            .map(|r| r.into_inner().next().expect("exactly one inner").as_rule())
        {
            Some(Rule::comment_line) => {
                property_statements.next();
            }
            Some(Rule::version_spec) => {
                property_statements.next();
                break Some(1);
            }
            // if it is not a comment line or a version spec, or there are no lines,
            // then we are done
            _ => break None,
        }
    }
    .ok_or(pest::error::Error::new_from_span(
        pest::error::ErrorVariant::CustomError {
            message: "Unsupported yarn version".to_string(),
        },
        property_statements.peek().unwrap().as_span(),
    ))?;

    Ok(property_statements
        .flat_map(|r| r.into_inner()) // a property statement is a comment line or a property
        .filter_map(|record| match record.as_rule() {
            Rule::samedent_property | Rule::legacy_property | Rule::legacy_multiple_property => {
                Some(Resolution::parse(record))
            }
            Rule::comment_line => None,
            Rule::version_spec => None,
            _ => unreachable!(),
        })
        .collect())
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    #[test]
    fn test_semver() {
        let input = "# yarn lockfile v1
foo: 1.2.3
bar: 2
baz: latest
";

        let packages = get_packages(input).unwrap();

        assert_eq!(packages.len(), 3);
        assert_eq!(
            packages.iter().map(|p| p.version).collect::<Vec<_>>(),
            vec!["1.2.3", "2", "latest"]
        );
    }

    #[test]
    fn test_metadata() {
        let input = r#"# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
# yarn lockfile v1
package-1@^1.0.0:
  version "1.0.3"
  resolved "https://registry.npmjs.org/package-1/-/package-1-1.0.3.tgz#a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8s9t0"
package-2@^2.0.0:
  version "2.0.1"
  resolved "https://registry.npmjs.org/package-2/-/package-2-2.0.1.tgz#a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8s9t0"
  dependencies:
    package-4 "^4.0.0"
package-3@^3.0.0:
  version "3.1.9"
  resolved "https://registry.npmjs.org/package-3/-/package-3-3.1.9.tgz#a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8s9t0"
  dependencies:
    package-4 "^4.5.0"
"#;

        let packages = get_packages(input);
        println!("{:#?}", packages);
        assert_eq!(packages.unwrap().len(), 4);
    }

    #[test_case(r#"# yarn lockfile v1
package-4@^4.0.0, package-4@^4.5.0:
  version "4.6.3"
"# ; "comma separated key")]
    #[test_case(r#"# yarn lockfile v1
lodash@^4.17.21:
  version "4.17.21"
  resolved "https://registry.yarnpkg.com/lodash/-/lodash-4.17.21.tgz#679591c564c3bffaae8454cf0b3df370c3d6911c"
  integrity sha512-v2kDEe57lecTulaDIuNTPy3Ry4gLGJ6Z1O3vE1krgXZNrsQ+LFTGHVxVjcXPs17LhbZVGedAJv8XZ1tvj5FvSg==
"# ; "basic yarn1 lockfile")]
    #[test_case(r#"# yarn lockfile v1
"@babel/code-frame@7.8.3", "@babel/code-frame@^7.0.0", "@babel/code-frame@^7.8.3":
  version "7.8.3"
  resolved "https://registry.yarnpkg.com/@babel/code-frame/-/code-frame-7.8.3.tgz#33e25903d7481181534e12ec0a25f16b6fcf419e"
  integrity sha512-a9gxpmdXtZEInkCSHUJDLHZVBgb1QS0jhss4cPP93EW7s+uC5bikET2twEF3KV+7rDblJcmNvTR7VJejqd2C2g==
  dependencies:
    "@babel/highlight" "^7.8.3"
"# ; "comma separated quoted key")]
    fn parse(input: &str) {
        let packages = get_packages(input);
        println!("{:#?}", packages);
        assert_eq!(packages.unwrap().len(), 1);
    }
}
