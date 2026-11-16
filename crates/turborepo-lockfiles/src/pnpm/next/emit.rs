//! Port of pnpm `yaml_emit/flow.rs` and `yaml_emit/nesting.rs`: the v13 main
//! document as tab-indented JSON with a trailing comma after every entry of a
//! multi-line collection.

use serde_yaml_ng::{Mapping, Value};

use super::wire::{DOCUMENT_SEPARATOR, DOCUMENT_START};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Lines,
    OneLine,
}

pub(super) fn encode(env: Option<&str>, tree: &Value) -> String {
    let mut out = String::new();
    if let Some(env) = env {
        out.push_str(DOCUMENT_START);
        out.push_str(env);
        out.push_str(DOCUMENT_SEPARATOR);
    }
    write_value(&mut out, tree, 0, Layout::Lines, None);
    out.push('\n');
    out
}

fn write_value(out: &mut String, value: &Value, level: usize, layout: Layout, key: Option<&str>) {
    match value {
        Value::Mapping(map) => write_map(out, map, level, layout, key),
        Value::Sequence(items) => write_list(out, items, level, layout, key),
        Value::String(text) => push_string(out, text),
        Value::Tagged(tagged) => write_value(out, &tagged.value, level, layout, key),
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
    }
}

fn write_map(out: &mut String, map: &Mapping, level: usize, layout: Layout, key: Option<&str>) {
    if map.is_empty() {
        out.push_str("{}");
        return;
    }
    if layout == Layout::OneLine || level == 3 && is_single_line_map(key, map) {
        write_one_line_map(out, map, level);
        return;
    }
    let blank_lines = level == 0 || level == 1 && matches!(key, Some("projects" | "packages"));
    out.push('{');
    for (index, (entry_key, entry)) in map.iter().enumerate() {
        if index > 0 && blank_lines {
            out.push('\n');
        }
        next_line(out, level + 1);
        let entry_key = key_text(entry_key);
        push_string(out, &entry_key);
        out.push_str(": ");
        write_value(out, entry, level + 1, Layout::Lines, Some(&entry_key));
        out.push(',');
    }
    next_line(out, level);
    out.push('}');
}

fn write_one_line_map(out: &mut String, map: &Mapping, level: usize) {
    out.push('{');
    for (index, (entry_key, entry)) in map.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_string(out, &key_text(entry_key));
        out.push_str(": ");
        write_value(out, entry, level, Layout::OneLine, None);
    }
    out.push('}');
}

fn write_list(out: &mut String, items: &[Value], level: usize, layout: Layout, key: Option<&str>) {
    let one_line = items.is_empty()
        || layout == Layout::OneLine
        || level == 3 && key.is_some_and(is_single_line_key);
    out.push('[');
    for (index, item) in items.iter().enumerate() {
        if one_line {
            if index > 0 {
                out.push_str(", ");
            }
            write_value(out, item, level, Layout::OneLine, None);
        } else {
            next_line(out, level + 1);
            write_value(out, item, level + 1, Layout::Lines, None);
            out.push(',');
        }
    }
    if !one_line {
        next_line(out, level);
    }
    out.push(']');
}

fn key_text(key: &Value) -> String {
    match key {
        Value::String(text) => text.clone(),
        other => {
            let mut text = String::new();
            write_value(&mut text, other, 0, Layout::OneLine, None);
            text
        }
    }
}

const SINGLE_LINE_KEYS: [&str; 4] = ["cpu", "engines", "os", "libc"];

fn is_single_line_key(key: &str) -> bool {
    SINGLE_LINE_KEYS.contains(&key)
}

fn is_single_line_map(key: Option<&str>, map: &Mapping) -> bool {
    match key {
        Some(key) if is_single_line_key(key) => true,
        Some("resolution") => !matches!(
            map.get("type").and_then(Value::as_str),
            Some("variations" | "binary")
        ),
        _ => false,
    }
}

fn next_line(out: &mut String, level: usize) {
    out.push('\n');
    out.extend(std::iter::repeat_n('\t', level));
}

fn push_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            control if control < ' ' => {
                use std::fmt::Write;
                let _ = write!(out, r"\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}
