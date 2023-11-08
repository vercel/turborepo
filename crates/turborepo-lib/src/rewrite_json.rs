use jsonc_parser::{errors::ParseError, parse_to_ast};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RewriteError {
    #[error("The JSON document failed to parse. {0}")]
    ParseError(ParseError),
    #[error("The JSON document contains no root object.")]
    NoRoot,
}
/**
 * When generating replacement content it's one of two things.
 */
enum GenerateType {
    Object,
    Member,
}

/**
 * Range stores the primitive AST token details required for document
 * mutation.
 */
struct Range {
    pub start: usize,
    pub end: usize,
    pub replacement_char: String,
}

/**
 * Given a JSONC document, an object traversal path, and a _pre-serialized_
 * JSON value `set_path` will return a minimally-mutated JSONC document with
 * the path specified set to the JSON value.
 *
 * - If the path exists, it will clobber the existing contents.
 * - If the path does not exist, it will synthesize object members and
 *   objects to ensure it does exist.
 *
 * In the event that the key appears multiple times in the document the last
 * instance of the key will be updated and earlier instances will be
 * disregarded.
 */
pub fn set_path(
    json_document_string: &str,
    path: &[&str],
    json_value: &str,
) -> Result<String, RewriteError> {
    let root = get_root(json_document_string)?;

    // Find the token we'll be modifying and its path from the root.
    let current_path = &mut vec![];
    let (closest_path, closest_node) = get_closest_node(&root, path, current_path);

    // Pull the token metadata off of the token.
    let (property_count, range): (usize, jsonc_parser::common::Range) = match closest_node {
        jsonc_parser::ast::Value::Object(literal) => (literal.properties.len(), literal.range),
        jsonc_parser::ast::Value::StringLit(literal) => (0, literal.range),
        jsonc_parser::ast::Value::NumberLit(literal) => (0, literal.range),
        jsonc_parser::ast::Value::BooleanLit(literal) => (0, literal.range),
        jsonc_parser::ast::Value::Array(literal) => (0, literal.range),
        jsonc_parser::ast::Value::NullKeyword(literal) => (0, literal.range),
    };

    // Figure out what we should be generating:
    // - An object to be assigned to an existing member. ("object")
    // - A member to add to an existing object. ("member")
    let generate_type: GenerateType = if !closest_path.is_empty() {
        GenerateType::Object
    } else {
        GenerateType::Member
    };

    // Identify the token replacement metadata: start, end, and possible trailing
    // join character
    let (start, end, separator) = match generate_type {
        GenerateType::Object => {
            let start = range.start;
            let end = range.end;
            let separator = "";
            (start, end, separator)
        }
        GenerateType::Member => {
            let start = range.start + 1;
            let end = range.start + 1;
            let separator = if property_count > 0 { "," } else { "" };
            (start, end, separator)
        }
    };

    // Generate the serialized JSON to insert into the document.
    // We synthesize objects for missing path segments.
    let missing_path_segments = &path[closest_path.len()..];
    let computed_object = match generate_type {
        GenerateType::Object => generate_object(missing_path_segments, json_value),
        GenerateType::Member => generate_member(missing_path_segments, json_value, separator),
    };

    // Generate a new document!
    let mut output: String = json_document_string.to_owned();
    output.replace_range(start..end, &computed_object);

    Ok(output)
}

/**
 * get_root returns the document root, or information on the error
 * encountered with the input json_document_string.
 */
fn get_root(json_document_string: &str) -> Result<jsonc_parser::ast::Value, RewriteError> {
    let parse_result_result = parse_to_ast(
        json_document_string,
        &Default::default(),
        &Default::default(),
    );

    match parse_result_result {
        Ok(parse_result) => match parse_result.value {
            Some(root) => Ok(root),
            None => Err(RewriteError::NoRoot),
        },
        Err(parse_error) => Err(RewriteError::ParseError(parse_error)),
    }
}

/**
 * get_closest_node traverses the JSON document via recursive tail calls to
 * find the last node that exists with in this JSON key path.
 */
fn get_closest_node<'a>(
    current_node: &'a jsonc_parser::ast::Value<'a>,
    target_path: &[&str],
    current_path: &'a mut Vec<&'a str>,
) -> (&'a Vec<&'a str>, &'a jsonc_parser::ast::Value<'a>) {
    // No target_path? We've arrived.
    if target_path.is_empty() {
        return (current_path, current_node);
    }

    match current_node {
        // Only objects can have key paths.
        jsonc_parser::ast::Value::Object(obj) => {
            // Grab the last property (member) which matches the current target_path
            // element.
            let object_property = obj.properties.iter().rev().find(|property| {
                let current_property_name = property.name.as_str();
                target_path[0] == current_property_name
            });

            // See if we found a matching key. If so, recurse.
            match object_property {
                Some(property) => {
                    let next_current_node = &property.value;
                    let next_property_name = property.name.as_str();
                    let next_current_path = &mut *current_path;
                    next_current_path.push(next_property_name);
                    let next_target_path = &target_path[1..];

                    // Tail call!
                    get_closest_node(next_current_node, next_target_path, next_current_path)
                }
                None => (current_path, current_node),
            }
        }
        // All other node types are complete.
        _ => (current_path, current_node),
    }
}

/**
 * Given path segments, generate a JSON object.
 */
fn generate_object(path_segments: &[&str], value: &str) -> String {
    let mut output = String::new();
    let length = path_segments.len();

    for path in path_segments {
        output.push('{');
        output.push('\"');
        output.push_str(path);
        output.push('\"');
        output.push(':');
    }
    output.push_str(value);
    for _ in 0..length {
        output.push('}');
    }

    output
}

/**
 * Given path segments, generate a JSON object member with an optional
 * trailing separator.
 */
fn generate_member(path_segments: &[&str], value: &str, separator: &str) -> String {
    let (key, remainder) = path_segments.split_first().unwrap();
    let object = generate_object(remainder, value);
    format!("\"{key}\":{object}{separator}")
}

/**
 * Given a JSONC document and an object traversal path, `unset_path` will
 * return a minimally-mutated JSONC document with all occurrences of the
 * specified path removed.
 */
pub fn unset_path(
    json_document_string: &str,
    path: &[&str],
    match_case_sensitive: bool,
) -> Result<Option<String>, RewriteError> {
    let root = get_root(json_document_string)?;

    // The key path can appear multiple times. This a vec that contains each time it
    // occurs.
    let path_ranges = find_all_paths(&root, path, match_case_sensitive);

    if path_ranges.is_empty() {
        return Ok(None);
    }

    // We mutate this as we go.
    let mut output: String = json_document_string.to_owned();

    // We could either join overlapping ranges, or just carry it over.
    // This elects to carry it over.
    let mut last_start_position = None;

    // We iterate in reverse since we're mutating the string.
    path_ranges.iter().rev().for_each(|range| {
        let end = match last_start_position {
            Some(last_start_position) => {
                if range.end > last_start_position {
                    last_start_position
                } else {
                    range.end
                }
            }
            None => range.end,
        };

        let is_overlapping = end != range.end;
        let replacement_char = if is_overlapping {
            ""
        } else {
            &range.replacement_char
        };

        output.replace_range(range.start..end, replacement_char);
        last_start_position = Some(range.start);
    });

    Ok(Some(output))
}

/**
 * find_all_paths returns the list of ranges which define the specified
 * token.
 */
fn find_all_paths<'a>(
    current_node: &'a jsonc_parser::ast::Value<'a>,
    target_path: &[&str],
    match_case_sensitive: bool,
) -> Vec<Range> {
    let mut ranges: Vec<Range> = vec![];

    // Early exit when it's impossible to have matching ranges.
    if target_path.is_empty() {
        return ranges;
    }

    // We can only find paths on objects.
    if let jsonc_parser::ast::Value::Object(obj) = current_node {
        // We need a reference to the previous and next property to identify if we're
        // looking at the first or last node which need special handling.
        let mut properties_iterator = obj.properties.iter().peekable();

        let mut previous_property: Option<&jsonc_parser::ast::ObjectProp<'_>> = None;
        while let Some(property) = properties_iterator.next() {
            let current_property_name = property.name.as_str();

            let should_rewrite = if match_case_sensitive {
                target_path[0] == current_property_name
            } else {
                target_path[0].to_ascii_lowercase() == current_property_name.to_ascii_lowercase()
            };

            if should_rewrite {
                // target_path == 1? We've arrived at a node to remove.
                if target_path.len() == 1 {
                    let next_property = properties_iterator.peek();

                    // We calculate the range based off the adjacent nodes.
                    // This is required to ensure that we capture things like commas.

                    // If this property follows another property, remove starting from the end of
                    // that property.
                    let start = if let Some(previous_property) = previous_property {
                        previous_property.range.end
                    } else {
                        property.range.start
                    };

                    // If this property precedes another property, remove all the way to the start
                    // of that property.
                    let end = if let Some(next_property) = next_property {
                        next_property.range.start
                    } else {
                        property.range.end
                    };

                    // The comma will _always_ be in the range. If we're the first or last, we want
                    // to remove it. If we are somewhere in the middle, we need
                    // to insert a comma to replace the comma we ate.
                    let replacement_char = if previous_property.is_none() || next_property.is_none()
                    {
                        ""
                    } else {
                        ","
                    };

                    ranges.push(Range {
                        start,
                        end,
                        replacement_char: replacement_char.to_owned(),
                    });
                } else {
                    // We must recurse.
                    let next_current_node = &property.value;
                    let next_target_path = &target_path[1..];

                    let mut children_ranges =
                        find_all_paths(next_current_node, next_target_path, match_case_sensitive);
                    ranges.append(&mut children_ranges);
                }
            }
            previous_property = Some(property);
        }
    }

    ranges
}

#[cfg(test)]
mod test {
    use crate::rewrite_json::{set_path, unset_path};

    macro_rules! set_tests {
        ($($name:ident: $value:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let (json_document_string, expected) = $value;
                assert_eq!(expected, set_path(json_document_string, &["parent", "child"], "\"Junior\"").unwrap());
            }
        )*
        }
    }

    macro_rules! unset_tests {
        ($($name:ident: $value:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let (json_document_string, path, expected) = $value;
                let output_option = unset_path(json_document_string, path, false).unwrap();
                assert_eq!(output_option.as_deref(), expected);
            }
        )*
        }
    }

    set_tests! {
        empty_object: (
            "{}",
            "{\"parent\":{\"child\":\"Junior\"}}"
        ),
        populated_object: (
            "{ \"other\": \"thing\" }",
            "{\"parent\":{\"child\":\"Junior\"}, \"other\": \"thing\" }"
        ),
        trailing_comma: (
            "{ \"trailing\": \"comma\", }",
            "{\"parent\":{\"child\":\"Junior\"}, \"trailing\": \"comma\", }"
        ),
        existing_primitive: (
            "{ \"parent\": \"thing\" }",
            "{ \"parent\": {\"child\":\"Junior\"} }"
        ),
        existing_empty_object: (
            "{ \"parent\": {} }",
            "{ \"parent\": {\"child\":\"Junior\"} }"
        ),
        existing_matching_object: (
            "{ \"parent\": { \"child\": \"Jerry\" } }",
            "{ \"parent\": { \"child\": \"Junior\" } }"
        ),
        existing_bonus_child: (
            "{ \"parent\": { \"child\": { \"grandchild\": \"Morty\" } } }",
            "{ \"parent\": { \"child\": \"Junior\" } }"
        ),
    }

    unset_tests! {
        nonexistent_path: (
            r#"{ "before": {}, "experimentalSpaces": { "id": "one" }, "experimentalSpaces": { "id": "two" }, "after": {} }"#,
            &["experimentalSpaces", "id", "nope"],
            None
        ),
        leaf_node: (
            r#"{ "before": {}, "experimentalSpaces": { "id": "one" }, "experimentalSpaces": { "id": "two" }, "after": {} }"#,
            &["experimentalSpaces", "id"],
            Some("{ \"before\": {}, \"experimentalSpaces\": {  }, \"experimentalSpaces\": {  }, \"after\": {} }")
        ),
        adjacent_nodes: (
            r#"{ "before": {}, "experimentalSpaces": { "id": "one" }, "experimentalSpaces": { "id": "two" }, "after": {} }"#,
            &["experimentalSpaces"],
            Some("{ \"before\": {},\"after\": {} }")
        ),
        adjacent_nodes_trailing_comma: (
            r#"{ "before": {}, "experimentalSpaces": { "id": "one" }, "experimentalSpaces": { "id": "two" }, }"#,
            &["experimentalSpaces"],
            // If it had a trailing comma to start, it may continue to have one.
            Some("{ \"before\": {}, }")
        ),
        parent_node: (
            r#"{ "before": {}, "experimentalSpaces": { "id": "one" }, "middle": {}, "experimentalSpaces": { "id": "two" }, "after": {} }"#,
            &["experimentalSpaces"],
            Some("{ \"before\": {},\"middle\": {},\"after\": {} }")
        ),
        empty_path: (
            r#"{ "before": {}, "experimentalSpaces": { "id": "one" }, "experimentalSpaces": { "id": "two" }, "after": {} }"#,
            &[],
            None
        ),
    }
}
