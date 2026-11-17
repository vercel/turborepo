//! The view of a `package.json` that module resolution gets.
//!
//! A manifest is read by many: the resolver (which package is this, what does a
//! request map to), the task layer (scripts, workspace dependencies), test
//! frameworks (an embedded config). Most edits of a manifest touch what
//! resolution never reads: a dependency entry, a script, the description. A
//! graph build that keyed every file that resolved through a manifest by the
//! manifest's whole contents traced all of them again for such an edit: on
//! real repositories, nine in ten files re-traced between two
//! neighbouring commits read nothing else that changed.
//!
//! So resolution does not get the file. It gets [`view`]: the manifest without
//! its [`INERT_FIELDS`]. Whatever resolution computes is then a function of the
//! view **by construction** (there is nothing else it could read), and a
//! reader's record of the manifest ([`crate::record::view_read`]) is keyed by
//! the view's bytes. A reader that needs one of those fields (the task layer, a
//! framework adapter) reads the file itself ([`crate::DirCache::read_cached`])
//! and is keyed by its contents, as before.
//!
//! The projection is textual: the kept members are copied byte for byte, in
//! their order (the order of the conditions of an `exports` map is its
//! meaning), so a reader that searches the bytes sees what a parser sees.

/// Top-level fields of a `package.json` that resolution does not get: which
/// packages are installed is a question for the lockfile and the `node_modules`
/// layout, never for the requester's or the target's dependency lists, and the
/// rest describes the package to people and to the package manager.
///
/// Not here, because resolution reads them: `name`, `version`, `type`, `main`,
/// `module`, `browser`, `exports`, `imports`, `types`, `typings`,
/// `typesVersions`, `bin`, `workspaces` (the workspace root),
/// `optionalDependencies` (platform packages of native addons), and every field
/// this list does not name.
pub const INERT_FIELDS: &[&str] = &[
    "author",
    "bugs",
    "contributors",
    "dependencies",
    "description",
    "devDependencies",
    "funding",
    "homepage",
    "keywords",
    "license",
    "peerDependencies",
    "peerDependenciesMeta",
    "repository",
    "scripts",
];

/// `bytes` without the [`INERT_FIELDS`] of its top-level object: the other
/// members byte for byte and in order, separated by commas. `None` when `bytes`
/// is not a JSON object: such a manifest has no view, and a reader depends on
/// the file as it is.
pub fn view(bytes: &[u8]) -> Option<Vec<u8>> {
    // A key written with escapes is not recognized, and kept: the safe side.
    project(bytes, |key| {
        !INERT_FIELDS.iter().any(|f| f.as_bytes() == key)
    })
}

/// The top-level fields of a `package.json` that the loader of a
/// `tsconfig.json` gets: what the resolution of an `extends` entry can read of
/// a manifest.
///
/// An `extends` that names a package is resolved like a module request with the
/// loader's own options (`oxc_resolver`'s `tsconfig_extends_resolver`: the
/// conditions `node` and `import`, the extension `.json`, the index name
/// `tsconfig`): against the manifest of the nearest package scope for a
/// self-reference (`name`, then `exports`) and for a subpath
/// import (`imports`), and against the manifest of the package directory it
/// finds (`exports`, else its `tsconfig` field, else `main`: TypeScript and the
/// loader resolve a package named by `extends` through `tsconfig`;
/// `typesVersions` is kept because it can redirect such a request for
/// TypeScript). Nothing else of a manifest reaches a config load.
///
/// The resolution view ([`view`]) keeps every field it does not know, and the
/// nearest manifest of a config at the repository's root is the root manifest:
/// with that view, a root `tsconfig.base.json` that extends a package made
/// `packageManager`, `pnpm.overrides` or `engines` of the root manifest an
/// input of every file below it.
pub const LOADER_FIELDS: &[&str] = &[
    "name",
    "exports",
    "imports",
    "main",
    "tsconfig",
    "typesVersions",
];

/// The [`LOADER_FIELDS`] of the top-level object of `bytes`, byte for byte and
/// in order (like [`view`], which holds every byte of this one). `None` when
/// `bytes` is not a JSON object.
pub fn loader_view(bytes: &[u8]) -> Option<Vec<u8>> {
    // A key written with escapes may spell one of the fields: kept, the safe side.
    project(bytes, |key| {
        key.contains(&b'\\') || LOADER_FIELDS.iter().any(|f| f.as_bytes() == key)
    })
}

/// The members of the top-level object of `bytes` whose key `keep` accepts,
/// byte for byte and in order, separated by commas. `None` when `bytes` is not
/// a JSON object.
fn project(bytes: &[u8], keep: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    serde_json::from_slice::<serde::de::IgnoredAny>(bytes).ok()?;
    let b = bytes;
    let mut i = skip_ws(b, 0);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let mut out = Vec::with_capacity(b.len());
    out.push(b'{');
    let mut first = true;
    loop {
        i = skip_ws(b, i);
        match *b.get(i)? {
            b'}' => break,
            b',' => {
                i += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let key_end = string_end(b, i)?;
        let key = &b[i + 1..key_end - 1];
        let key_start = i;
        i = skip_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i = skip_ws(b, i + 1);
        let value_start = i;
        i = value_end(b, i)?;
        if !keep(key) {
            continue;
        }
        if !first {
            out.push(b',');
        }
        first = false;
        out.extend_from_slice(&b[key_start..key_end]);
        out.push(b':');
        out.extend_from_slice(&b[value_start..i]);
    }
    out.push(b'}');
    Some(out)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while matches!(b.get(i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        i += 1;
    }
    i
}

/// The index after the closing quote of the string that opens at `i`.
fn string_end(b: &[u8], mut i: usize) -> Option<usize> {
    i += 1;
    loop {
        match *b.get(i)? {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
}

/// The index after the JSON value that starts at `i`.
fn value_end(b: &[u8], mut i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => string_end(b, i),
        b'{' | b'[' => {
            let mut depth = 0usize;
            loop {
                match *b.get(i)? {
                    b'"' => {
                        i = string_end(b, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        _ => {
            while !matches!(
                b.get(i),
                None | Some(b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
            ) {
                i += 1;
            }
            Some(i)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{loader_view, view};

    #[test]
    fn the_loader_view_holds_what_an_extends_can_read() {
        let l = |s: &str| loader_view(s.as_bytes()).map(|b| String::from_utf8(b).unwrap());
        let m = r##"{
  "name": "root",
  "private": true,
  "packageManager": "pnpm@9.1.0",
  "main": "./index.js",
  "pnpm": { "overrides": { "exports": "1" } },
  "imports": { "#cfg": "./configs/base.json" },
  "engines": { "node": ">=20" },
  "exports": { "./base": "./configs/base.json" },
  "workspaces": ["packages/*"]
}"##;
        assert_eq!(
            l(m).unwrap(),
            r##"{"name":"root","main":"./index.js","imports":{ "#cfg": "./configs/base.json" },"exports":{ "./base": "./configs/base.json" }}"##
        );
        // It does not move with a field outside it,
        let other = m
            .replace("pnpm@9.1.0", "pnpm@10.0.0")
            .replace("\"1\"", "\"2\"");
        assert_ne!(v(m), v(&other));
        assert_eq!(l(m), l(&other));
        // and moves with each field of its own.
        for (a, b) in [
            ("\"root\"", "\"other\""),
            ("./index.js", "./main.js"),
            ("\"#cfg\"", "\"#config\""),
            ("\"./base\":", "\"./lib\":"),
        ] {
            assert_ne!(l(m), l(&m.replace(a, b)), "{a}");
        }
        // A key written with an escape may be one of them: kept.
        let escaped = ["{\"na", "\\u006d", "e\":\"x\",\"private\":true}"].concat();
        assert_eq!(
            l(&escaped).unwrap(),
            ["{\"na", "\\u006d", "e\":\"x\"}"].concat()
        );
        assert_eq!(l("[1]"), None);
        assert_eq!(l("{\"scripts\":{}}").as_deref(), Some("{}"));
    }

    fn v(s: &str) -> Option<String> {
        view(s.as_bytes()).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn inert_fields_are_left_out_and_the_rest_is_kept_as_written() {
        let m = r#"{
  "name": "@scope/pkg",
  "version": "1.2.3",
  "scripts": { "build": "tsc -p .", "x": "echo \"}\" ," },
  "exports": { ".": { "import": "./a.mjs",   "default": "./a.js" }, "./x": [ "./x.js", null ] },
  "dependencies": { "left-pad": "^1", "exports": "1" },
  "type": "module",
  "devDependencies": {},
  "optionalDependencies": { "fsevents": "*" },
  "private": true
}"#;
        assert_eq!(
            v(m).unwrap(),
            r#"{"name":"@scope/pkg","version":"1.2.3","exports":{ ".": { "import": "./a.mjs",   "default": "./a.js" }, "./x": [ "./x.js", null ] },"type":"module","optionalDependencies":{ "fsevents": "*" },"private":true}"#
        );
    }

    #[test]
    fn the_view_does_not_move_when_only_inert_fields_do() {
        let a = r#"{"name":"a","main":"x.js","dependencies":{"b":"1.0.0"},"scripts":{"t":"jest"}}"#;
        let b = r#"{"name":"a",
            "dependencies":{"b":"2.0.0","c":"1.0.0"},"main":"x.js","description":"more"}"#;
        assert_eq!(v(a), v(b));
        let c = r#"{"name":"a","main":"y.js","dependencies":{"b":"1.0.0"}}"#;
        assert_ne!(v(a), v(c));
        // Member order is kept: it is the meaning of a conditions object.
        assert_ne!(
            v(r#"{"exports":{"import":"./a","default":"./b"}}"#),
            v(r#"{"exports":{"default":"./b","import":"./a"}}"#)
        );
    }

    #[test]
    fn what_is_not_an_object_has_no_view() {
        assert_eq!(v("[1]"), None);
        assert_eq!(v("{\"name\": }"), None);
        assert_eq!(v(""), None);
        assert_eq!(v("{}").as_deref(), Some("{}"));
        assert_eq!(v(" {\"scripts\":{}} ").as_deref(), Some("{}"));
        // A key written with an escape is kept (not recognized as inert).
        let escaped = ["{\"scri", "\\u0070", "ts\":1}"].concat();
        assert_eq!(v(&escaped).as_deref(), Some(escaped.as_str()));
    }
}
