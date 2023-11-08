use std::sync::OnceLock;

use nom::{
    branch::alt,
    bytes::complete::{escaped_transform, is_not, tag, take_till},
    character::complete::{anychar, char as nom_char, crlf, newline, none_of, satisfy, space1},
    combinator::{all_consuming, map, not, opt, peek, recognize, value},
    multi::{count, many0, many1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use regex::Regex;
use serde_json::Value;

// regex for trimming spaces from start and end
fn pseudostring_replace() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ *| *$").unwrap())
}

pub fn parse_syml(input: &str) -> Result<Value, super::Error> {
    match all_consuming(property_statements(0))(input) {
        Ok((_, value)) => Ok(value),
        Err(e) => Err(super::Error::SymlParse(e.to_string())),
    }
}

// Array and map types
fn item_statements(level: usize) -> impl Fn(&str) -> IResult<&str, Value> {
    move |i: &str| map(many0(item_statement(level)), Value::Array)(i)
}

fn item_statement(level: usize) -> impl Fn(&str) -> IResult<&str, Value> {
    move |i: &str| {
        let (i, _) = indent(level)(i)?;
        let (i, _) = nom_char('-')(i)?;
        let (i, _) = blankspace(i)?;
        expression(level)(i)
    }
}

fn property_statements(level: usize) -> impl Fn(&str) -> IResult<&str, Value> {
    move |i: &str| {
        let (i, properties) = many0(property_statement(level))(i)?;
        let mut map = serde_json::Map::new();
        for (key, value) in properties.into_iter().flatten() {
            map.insert(key, value);
        }
        Ok((i, Value::Object(map)))
    }
}

fn property_statement(level: usize) -> impl Fn(&str) -> IResult<&str, Vec<(String, Value)>> {
    move |i: &str| {
        alt((
            value(
                vec![],
                tuple((
                    opt(blankspace),
                    opt(pair(nom_char('#'), many1(pair(not(eol), anychar)))),
                    many1(eol_any),
                )),
            ),
            map(
                preceded(
                    indent(level),
                    separated_pair(name, wrapped_colon, expression(level)),
                ),
                |entry| vec![entry],
            ),
            // legacy names
            map(
                preceded(
                    indent(level),
                    separated_pair(legacy_name, wrapped_colon, expression(level)),
                ),
                |entry| vec![entry],
            ),
            // legacy prop without colon
            map(
                preceded(
                    indent(level),
                    separated_pair(
                        legacy_name,
                        blankspace,
                        terminated(legacy_literal, many1(eol_any)),
                    ),
                ),
                |entry| vec![entry],
            ),
            multikey_property_statement(level),
        ))(i)
    }
}

fn multikey_property_statement(
    level: usize,
) -> impl Fn(&str) -> IResult<&str, Vec<(String, Value)>> {
    move |i: &str| {
        let (i, ()) = indent(level)(i)?;
        let (i, property) = legacy_name(i)?;
        let (i, others) = many1(preceded(
            delimited(opt(blankspace), nom_char(','), opt(blankspace)),
            legacy_name,
        ))(i)?;
        let (i, _) = wrapped_colon(i)?;
        let (i, value) = expression(level)(i)?;

        Ok((
            i,
            std::iter::once(property)
                .chain(others)
                .map(|key| (key, value.clone()))
                .collect(),
        ))
    }
}

fn wrapped_colon(i: &str) -> IResult<&str, char> {
    delimited(opt(blankspace), nom_char(':'), opt(blankspace))(i)
}

fn expression(level: usize) -> impl Fn(&str) -> IResult<&str, Value> {
    move |i: &str| {
        alt((
            preceded(
                tuple((
                    peek(tuple((eol, indent(level + 1), nom_char('-'), blankspace))),
                    eol_any,
                )),
                item_statements(level + 1),
            ),
            preceded(eol, property_statements(level + 1)),
            terminated(literal, many1(eol_any)),
        ))(i)
    }
}

fn indent(level: usize) -> impl Fn(&str) -> IResult<&str, ()> {
    move |i: &str| {
        let (i, _) = count(nom_char(' '), level * 2)(i)?;
        Ok((i, ()))
    }
}

// Simple types

fn name(i: &str) -> IResult<&str, String> {
    alt((string, pseudostring))(i)
}

fn legacy_name(i: &str) -> IResult<&str, String> {
    alt((
        string,
        map(recognize(many1(pseudostring_legacy)), |s| s.to_string()),
    ))(i)
}

fn literal(i: &str) -> IResult<&str, Value> {
    alt((
        value(Value::Null, null),
        map(boolean, Value::Bool),
        map(string, Value::String),
        map(pseudostring, Value::String),
    ))(i)
}

fn legacy_literal(i: &str) -> IResult<&str, Value> {
    alt((
        value(Value::Null, null),
        map(string, Value::String),
        map(pseudostring_legacy, Value::String),
    ))(i)
}

fn pseudostring(i: &str) -> IResult<&str, String> {
    let (i, pseudo) = recognize(pseudostring_inner)(i)?;
    Ok((
        i,
        pseudostring_replace().replace_all(pseudo, "").into_owned(),
    ))
}

fn pseudostring_inner(i: &str) -> IResult<&str, ()> {
    let (i, _) = none_of("\r\n\t ?:,][{}#&*!|>'\"%@`-")(i)?;
    let (i, _) = many0(tuple((opt(blankspace), none_of("\r\n\t ,][{}:#\"'"))))(i)?;
    Ok((i, ()))
}

fn pseudostring_legacy(i: &str) -> IResult<&str, String> {
    let (i, pseudo) = recognize(pseudostring_legacy_inner)(i)?;
    let replaced = pseudostring_replace().replace_all(pseudo, "");
    Ok((i, replaced.to_string()))
}

fn pseudostring_legacy_inner(i: &str) -> IResult<&str, ()> {
    let (i, _) = opt(tag("--"))(i)?;
    let (i, _) = satisfy(|c| c.is_ascii_alphanumeric() || c == '/')(i)?;
    let (i, _) = take_till(|c| "\r\n\t :,".contains(c))(i)?;
    Ok((i, ()))
}

// String parsing

fn null(i: &str) -> IResult<&str, &str> {
    tag("null")(i)
}

fn boolean(i: &str) -> IResult<&str, bool> {
    alt((value(true, tag("true")), value(false, tag("false"))))(i)
}

fn string(i: &str) -> IResult<&str, String> {
    alt((empty_string, delimited(tag("\""), syml_chars, tag("\""))))(i)
}

fn empty_string(i: &str) -> IResult<&str, String> {
    let (i, _) = tag(r#""""#)(i)?;
    Ok((i, "".to_string()))
}

fn syml_chars(i: &str) -> IResult<&str, String> {
    // The SYML grammar provided by Yarn2+ includes escape sequences that weren't
    // supported by the yarn1 parser. We diverge from the Yarn2+ provided
    // grammar to match the actual parser used by yarn1.
    escaped_transform(
        is_not("\"\\"),
        '\\',
        alt((
            value("\"", tag("\"")),
            value("\\", tag("\\")),
            value("/", tag("/")),
            value("\n", tag("n")),
            value("\r", tag("r")),
            value("\t", tag("t")),
        )),
    )(i)
}

// Spaces
fn blankspace(i: &str) -> IResult<&str, &str> {
    space1(i)
}

fn eol_any(i: &str) -> IResult<&str, &str> {
    recognize(tuple((eol, many0(tuple((opt(blankspace), eol))))))(i)
}

fn eol(i: &str) -> IResult<&str, &str> {
    alt((crlf, value("\n", newline), value("\r", nom_char('\r'))))(i)
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    #[test_case("null", Value::Null ; "null")]
    #[test_case("false", Value::Bool(false) ; "literal false")]
    #[test_case("true", Value::Bool(true) ; "literal true")]
    #[test_case("\"\"", Value::String("".into()) ; "empty string literal")]
    #[test_case("\"foo\"", Value::String("foo".into()) ; "quoted string literal")]
    #[test_case("foo", Value::String("foo".into()) ; "unquoted string literal")]
    fn test_literal(input: &str, expected: Value) {
        let (_, actual) = literal(input).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case("name: foo", "name" ; "basic")]
    #[test_case("technically a name: foo", "technically a name" ; "multiword name")]
    fn test_name(input: &str, expected: &str) {
        let (_, actual) = name(input).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case("foo@1:", "foo@1" ; "name with colon terminator")]
    #[test_case("\"foo@1\":", "foo@1" ; "qutoed name with colon terminator")]
    #[test_case("name foo", "name" ; "name without colon terminator")]
    fn test_legacy_name(input: &str, expected: &str) {
        let (_, actual) = legacy_name(input).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case("null\n", Value::Null ; "null")]
    #[test_case("\"foo\"\n", json!("foo") ; "basic string")]
    #[test_case("\n  name: foo\n", json!({ "name": "foo" }) ; "basic object")]
    fn test_expression(input: &str, expected: Value) {
        let (_, actual) = expression(0)(input).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case("# a comment\n", vec![] ; "comment")]
    #[test_case("foo: null\n", vec![("foo".into(), Value::Null)] ; "single property")]
    #[test_case("name foo\n", vec![("name".into(), json!("foo"))] ; "legacy property")]
    fn test_property_statement(input: &str, expected: Vec<(String, Value)>) {
        let (_, actual) = property_statement(0)(input).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case("name: foo\n", json!({"name": "foo"}) ; "single property object")]
    #[test_case("\"name\": foo\n", json!({"name": "foo"}) ; "single quoted property object")]
    #[test_case("name foo\n", json!({"name": "foo"}) ; "single property without colon object")]
    #[test_case("# comment\nname: foo\n", json!({"name": "foo"}) ; "comment doesn't affect object")]
    #[test_case("name foo\nversion \"1.2.3\"\n", json!({"name": "foo", "version": "1.2.3"}) ; "multi-property object")]
    #[test_case("foo:\n  version \"1.2.3\"\n", json!({"foo": {"version": "1.2.3"}}) ; "nested object")]
    #[test_case("foo, bar, baz:\n  version \"1.2.3\"\n", json!({
        "foo": {"version": "1.2.3"},
        "bar": {"version": "1.2.3"},
        "baz": {"version": "1.2.3"},
    }) ; "multi-key object")]
    fn test_property_statements(input: &str, expected: Value) {
        let (_, actual) = property_statements(0)(input).unwrap();
        assert_eq!(actual, expected);
    }
}
