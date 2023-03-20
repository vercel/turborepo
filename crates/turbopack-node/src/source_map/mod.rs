use std::{
    borrow::Cow,
    fmt::Write,
    path::{Path, MAIN_SEPARATOR},
};

use anyhow::Result;
pub use content_source::{NextSourceMapTraceContentSource, NextSourceMapTraceContentSourceVc};
use once_cell::sync::Lazy;
use owo_colors::{OwoColorize, Style};
use regex::Regex;
pub use trace::{
    SourceMapTrace, SourceMapTraceVc, StackFrame, StackFrameVc, TraceResult, TraceResultVc,
};
use turbo_tasks_fs::{to_sys_path, FileSystemPathVc};
use turbopack_core::{asset::AssetVc, source_map::GenerateSourceMap};

use self::trace::TraceResultReadRef;
use crate::{internal_assets_for_source_mapping, AssetsForSourceMappingVc};

pub mod content_source;
pub mod trace;

pub async fn apply_source_mapping<'a>(
    text: &'a str,
    assets_for_source_mapping: AssetsForSourceMappingVc,
    root: FileSystemPathVc,
    ansi_colors: bool,
) -> Result<Cow<'a, str>> {
    static STACK_TRACE_LINE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\n    at (?:(.+) \()?(.+):(\d+):(\d+)\)?").unwrap());

    let mut it = STACK_TRACE_LINE.captures_iter(text).peekable();
    if it.peek().is_none() {
        return Ok(Cow::Borrowed(text));
    }
    let mut first_error = true;
    let mut new = String::with_capacity(text.len() * 2);
    let mut last_match = 0;
    for cap in it {
        // unwrap on 0 is OK because captures only reports matches
        let m = cap.get(0).unwrap();
        new.push_str(&text[last_match..m.start()]);
        let name = cap.get(1).map(|s| s.as_str());
        let file = cap.get(2).unwrap().as_str();
        let line = cap.get(3).unwrap().as_str();
        let column = cap.get(4).unwrap().as_str();
        let line = line.parse::<usize>()?;
        let column = column.parse::<usize>()?;
        let resolved =
            resolve_source_mapping(assets_for_source_mapping, root, name, file, line, column).await;
        write_resolved(
            &mut new,
            resolved,
            name,
            file,
            line,
            column,
            &mut first_error,
            ansi_colors,
        )?;
        last_match = m.end();
    }
    new.push_str(&text[last_match..]);
    Ok(Cow::Owned(new))
}

fn write_resolved(
    writable: &mut impl Write,
    resolved: Result<Option<TraceResultReadRef>>,
    original_name: Option<&str>,
    original_file: &str,
    original_line: usize,
    original_column: usize,
    first_error: &mut bool,
    ansi_colors: bool,
) -> Result<()> {
    let lowlight = if ansi_colors {
        Style::new().dimmed()
    } else {
        Style::new()
    };
    match resolved {
        Err(err) => {
            // There was an error resolving the source map
            write!(
                writable,
                "\n  at {} ({}:{}:{})",
                original_name.unwrap_or("(anonymous)"),
                original_file,
                original_line,
                original_column
            )?;
            if *first_error {
                write!(writable, "\n  (error resolving source map: {})", err)?;
                *first_error = false;
            } else {
                write!(writable, " (error resolving source map)")?;
            }
        }
        Ok(None) => {
            // There is no source map for this file
            write!(
                writable,
                "\n  {}",
                format_args!(
                    "at {} ({}:{}:{}) [no source map]",
                    original_name.unwrap_or("(anonymous)"),
                    original_file,
                    original_line,
                    original_column
                )
                .style(lowlight)
            )?;
        }
        Ok(Some(trace_result)) => {
            match &*trace_result {
                TraceResult::NotFound => {
                    // There is a source map for this file, but no mapping for the line
                    write!(
                        writable,
                        "\n  {}",
                        format_args!(
                            "at {} ({}:{}:{}) [unmapped]",
                            original_name.unwrap_or("(anonymous)"),
                            original_file,
                            original_line,
                            original_column
                        )
                        .style(lowlight)
                    )?;
                }
                TraceResult::Found(frame) => {
                    // There is a source map for this file, and it maps to an original location
                    write!(
                        writable,
                        "\n  at {} {}",
                        frame,
                        format_args!("[{}:{}:{}]", original_file, original_line, original_column)
                            .style(lowlight)
                    )?;
                }
            }
        }
    }
    Ok(())
}

async fn resolve_source_mapping(
    assets_for_source_mapping: AssetsForSourceMappingVc,
    root: FileSystemPathVc,
    name: Option<&str>,
    file: &str,
    line: usize,
    column: usize,
) -> Result<Option<TraceResultReadRef>> {
    let Some(root) = to_sys_path(root).await? else {
        return Ok(None);
    };
    let Ok(file) = Path::new(file).strip_prefix(root) else {
        return Ok(None);
    };
    let file = file.to_string_lossy();
    let file = if MAIN_SEPARATOR != '/' {
        Cow::Owned(file.replace(MAIN_SEPARATOR, "/"))
    } else {
        file
    };
    let map = assets_for_source_mapping.await?;
    println!("map: {:?}", map);
    let Some(generate_source_map) = map.get(file.as_ref()) else {
        return Ok(None);
    };
    let trace = SourceMapTraceVc::new(
        generate_source_map.generate_source_map(),
        line,
        column,
        name.map(|s| s.to_string()),
    )
    .trace()
    .await?;
    Ok(Some(trace))
}

#[turbo_tasks::value(shared)]
pub struct StructuredError {
    name: String,
    message: String,
    stack: Vec<StackFrame>,
}

impl StructuredError {
    pub async fn print(
        &self,
        assets_for_source_mapping: AssetsForSourceMappingVc,
        root: FileSystemPathVc,
        ansi_colors: bool,
    ) -> Result<String> {
        let mut message = String::new();

        writeln!(message, "{}: {}", self.name, self.message)?;

        let mut first_error = true;

        for frame in &self.stack {
            if let Some((line, column)) = frame.get_pos() {
                let resolved = resolve_source_mapping(
                    assets_for_source_mapping,
                    root,
                    frame.name.as_deref(),
                    frame.file.as_str(),
                    line,
                    column,
                )
                .await;
                write_resolved(
                    &mut message,
                    resolved,
                    frame.name.as_deref(),
                    frame.file.as_str(),
                    line,
                    column,
                    &mut first_error,
                    ansi_colors,
                )?;

                continue;
            }

            writeln!(message, "  at {}", frame)?;
        }
        Ok(message)
    }
}

pub async fn trace_stack(
    error: StructuredError,
    root_asset: AssetVc,
    output_path: FileSystemPathVc,
) -> Result<String> {
    let assets_for_source_mapping = internal_assets_for_source_mapping(root_asset, output_path);

    error
        .print(assets_for_source_mapping, output_path, false)
        .await
}
