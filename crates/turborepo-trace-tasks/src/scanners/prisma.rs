//! Prisma schema scanners: schema folders and the `generator` / `datasource`
//! blocks.
//!
//! Each scanner takes the main schema location (a file or a multi-file folder):
//!
//! | Scanner | Adds |
//! |---|---|
//! | `prisma-schema` | the schema as an input. A folder, a file in a folder named `schema`, or a file using the `prismaSchemaFolder` preview feature makes every `*.prisma` below the folder an input (a membership) |
//! | `prisma-generators` | generator outputs: `output = "…"` relative to the schema file; `prisma-client-js` without `output` writes the packages `@prisma/client` and `.prisma/client` ([`TaskSpec::output_package`]); providers as toolchain, `node ./gen.js` providers as entries; other generators without an `output` are uncertain |
//! | `prisma-env` | `env("X")` names in `datasource` blocks, as env inputs |
//! | `prisma-format` | the schema files as outputs (`prisma format`, `prisma db pull`) |

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use super::Target;
use crate::{
    recognizers::codegen::config::{dir_of, unknown},
    registry::RecognizeCx,
    spec::{EntryLang, TaskSpec},
};

/// `prisma-schema`.
pub fn scan_schema(_cx: &RecognizeCx<'_>, t: &Target<'_>, out: &mut TaskSpec) {
    schema_files(&t.path(), Some(out));
}

/// `prisma-generators`.
pub fn scan_generators(cx: &RecognizeCx<'_>, t: &Target<'_>, out: &mut TaskSpec) {
    let main = t.path();
    let mut any = false;
    for f in schema_files(&main, None) {
        let Ok(text) = fsx::read_to_string(&f) else {
            continue;
        };
        let dir = dir_of(&f);
        for b in blocks(&text).into_iter().filter(|b| b.kind == "generator") {
            any = true;
            generator(cx, &dir, &b, out);
        }
    }
    if !any {
        // A missing schema or one without generators: the default client location.
        let dir = if fsx::is_dir(&main) {
            main.clone()
        } else {
            dir_of(&main)
        };
        default_client(cx, &dir, out);
    }
}

/// `prisma-env`.
pub fn scan_env(_cx: &RecognizeCx<'_>, t: &Target<'_>, out: &mut TaskSpec) {
    for f in schema_files(&t.path(), None) {
        let Ok(text) = fsx::read_to_string(&f) else {
            continue;
        };
        for b in blocks(&text).into_iter().filter(|b| b.kind == "datasource") {
            for (_, v) in &b.fields {
                if let Some(n) = env_name(v) {
                    out.env.push(n.to_string());
                }
            }
        }
    }
}

/// `prisma-format`.
pub fn scan_format(_cx: &RecognizeCx<'_>, t: &Target<'_>, out: &mut TaskSpec) {
    for f in schema_files(&t.path(), None) {
        out.output(f);
    }
}

/// The schema files of the main schema location `main`; with `out`, also adds
/// the schema inputs (the file, and a `**/*.prisma` membership for a folder).
fn schema_files(main: &Path, mut out: Option<&mut TaskSpec>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if fsx::is_dir(main) {
        folder(main, out, &mut files);
        return files;
    }
    if let Some(o) = out.as_deref_mut() {
        o.input(main.to_path_buf());
    }
    let text = fsx::read_to_string(main).unwrap_or_default();
    let dir = dir_of(main);
    if dir.file_name().is_some_and(|n| n == "schema") || text.contains("prismaSchemaFolder") {
        folder(&dir, out, &mut files);
        if !files.iter().any(|f| f == main) {
            files.push(main.to_path_buf());
        }
    } else {
        files.push(main.to_path_buf());
    }
    files
}

/// Every `*.prisma` file below `dir` (except in `migrations`), and the
/// membership.
fn folder(dir: &Path, out: Option<&mut TaskSpec>, files: &mut Vec<PathBuf>) {
    if let Some(o) = out {
        o.membership(dir.to_path_buf(), "**/*.prisma", "prisma schema folder");
    }
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for (p, is_dir) in fsx::read_dir(&d).unwrap_or_default() {
            if is_dir {
                if depth < 8 && p.file_name().is_some_and(|n| n != "migrations") {
                    stack.push((p, depth + 1));
                }
            } else if p.extension().is_some_and(|e| e == "prisma") {
                files.push(p);
            }
        }
    }
    files.sort();
}

fn generator(cx: &RecognizeCx<'_>, dir: &Path, b: &Block, out: &mut TaskSpec) {
    let provider = b
        .get("provider")
        .map(|p| string_value(p).unwrap_or(p).to_string())
        .unwrap_or_default();
    // A generator run as a command (`provider = "node ./gen.js"`).
    if let Some(script) = provider.strip_prefix("node ") {
        out.entry(fsx::join(dir, script.trim()), EntryLang::Js);
    } else if !provider.is_empty()
        && !provider.contains(' ')
        && !matches!(provider.as_str(), "prisma-client-js" | "prisma-client")
    {
        out.tool("npm", provider.clone());
    }
    match b.get("output") {
        Some(o) => match string_value(o) {
            Some(o) => out.output(fsx::join(dir, o)),
            None => unknown(
                out,
                dir,
                UncertaintyKind::EnvDependent,
                format!("prisma generator `{}` output `{o}`", b.name),
            ),
        },
        None if provider == "prisma-client-js" || provider.is_empty() => {
            default_client(cx, dir, out)
        }
        None => unknown(
            out,
            dir,
            UncertaintyKind::GeneratedFile,
            format!(
                "prisma generator `{}` (`{provider}`) has no output and no known default",
                b.name
            ),
        ),
    }
}

/// The default client: the packages `@prisma/client` and `.prisma/client`, as
/// resolved from the task's package, and from the nearest directory above the
/// schema where `@prisma/client` is installed when that is not on the package's
/// resolution path.
fn default_client(cx: &RecognizeCx<'_>, schema_dir: &Path, out: &mut TaskSpec) {
    let mut installed = None;
    let mut d = Some(schema_dir);
    while let Some(x) = d {
        if fsx::is_dir(&x.join("node_modules/@prisma/client")) {
            installed = Some(x.to_path_buf());
            break;
        }
        if x == cx.repo_root {
            break;
        }
        d = x.parent();
    }
    for name in [".prisma/client", "@prisma/client"] {
        out.output_package("npm", name);
        if let Some(i) = &installed
            && !cx.package_root.starts_with(i)
        {
            out.output_package_in("npm", name, i.clone());
        }
    }
}

/// One `generator` or `datasource` block.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// `generator` or `datasource`.
    pub kind: String,
    /// Block name.
    pub name: String,
    /// `key = value` assignments (raw values, quotes kept).
    pub fields: Vec<(String, String)>,
}

impl Block {
    /// The raw value of `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// The `generator` and `datasource` blocks of a Prisma schema.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut cur: Option<Block> = None;
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        match &mut cur {
            None => {
                let mut it = line.split_whitespace();
                let kw = it.next().unwrap_or("");
                if matches!(kw, "generator" | "datasource") && line.ends_with('{') {
                    cur = Some(Block {
                        kind: kw.to_string(),
                        name: it.next().unwrap_or("").trim_end_matches('{').to_string(),
                        fields: Vec::new(),
                    });
                }
            }
            Some(b) => {
                if line.starts_with('}') {
                    out.push(cur.take().unwrap_or_default());
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    b.fields.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    // `//` outside a string.
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => in_str = !in_str,
            b'\\' if in_str => i += 1,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// A quoted string value (`"x"` → `x`); `None` for `env("X")` and other
/// expressions.
fn string_value(v: &str) -> Option<&str> {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .filter(|s| !s.contains('"'))
}

/// `env("X")` → `X`.
fn env_name(v: &str) -> Option<&str> {
    let r = v.trim().strip_prefix("env(")?.strip_suffix(')')?;
    string_value(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks() {
        let s = r#"
// comment generator x {
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL") // the url
}
generator client {
  provider = "prisma-client-js"
  output   = "../src/generated/client" // "x"
}
model User { id Int @id }
"#;
        let b = blocks(s);
        assert_eq!(b.len(), 2);
        assert_eq!(env_name(b[0].get("url").unwrap()), Some("DATABASE_URL"));
        assert_eq!(
            string_value(b[1].get("output").unwrap()),
            Some("../src/generated/client")
        );
    }
}
