//! Entry-level differences of a `package.json`'s resolution fields (`exports`,
//! `imports`, `main`, `module`, `browser`, `types`, `typings`,
//! `typesVersions`).
//!
//! [`crate::configdiff`] asks, per request that an importer makes of the
//! package (`.`, `./feature`, `#internal`), whether the manifest alone answers
//! it differently at the two revisions ([`request_changed`]). The answer is
//! computed by looking the request up in both versions the way Node does (exact
//! subpath, else the best `*` pattern, with the match substituted) and
//! comparing the two target trees as written: condition order, array
//! order and `null` targets are part of the comparison, because each decides
//! what a resolver picks. Nothing is resolved against the file system: a file
//! that appears or disappears is a change of its own, matched by the probes of
//! the resolutions that looked for it.
//!
//! Two views are compared separately:
//!
//! - **runtime**: the target tree without `types` conditions; for `.` also
//!   `main`, `module` and a string `browser`;
//! - **types**: the whole target tree; for `.` also `types`, `typings` and all
//!   of `typesVersions`; for a subpath, what each `typesVersions` range maps it
//!   to.
//!
//! A difference only in the types view matters to entries that type-check, not
//! to tests whose runner transpiles file by file.
//!
//! Anything the model does not understand is an error ([`Shape`]), and the
//! caller keeps the whole-file treatment: `exports` or `imports` present at one
//! revision only (every deep import changes meaning), keys that mix subpaths
//! and conditions, a `browser` object (file remapping), leaves that are neither
//! strings, `null`, arrays nor condition objects.

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};

/// JSON with object keys in document order (condition order decides
/// resolution).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A number, as text.
    Num(String),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Json>),
    /// An object, in document order.
    Obj(Vec<(String, Json)>),
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Json, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Json;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_bool<E>(self, b: bool) -> Result<Json, E> {
                Ok(Json::Bool(b))
            }
            fn visit_i64<E>(self, n: i64) -> Result<Json, E> {
                Ok(Json::Num(n.to_string()))
            }
            fn visit_u64<E>(self, n: u64) -> Result<Json, E> {
                Ok(Json::Num(n.to_string()))
            }
            fn visit_f64<E>(self, n: f64) -> Result<Json, E> {
                Ok(Json::Num(n.to_string()))
            }
            fn visit_str<E>(self, s: &str) -> Result<Json, E> {
                Ok(Json::Str(s.to_owned()))
            }
            fn visit_string<E>(self, s: String) -> Result<Json, E> {
                Ok(Json::Str(s))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Json, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element()? {
                    v.push(x);
                }
                Ok(Json::Arr(v))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Json, A::Error> {
                let mut v = Vec::new();
                while let Some((k, x)) = a.next_entry::<String, Json>()? {
                    v.push((k, x));
                }
                Ok(Json::Obj(v))
            }
        }
        d.deserialize_any(V)
    }
}

impl Json {
    /// Parses a document; `None` unless it is a JSON object.
    pub fn parse_object(text: &str) -> Option<Json> {
        match serde_json::from_str::<Json>(text).ok()? {
            j @ Json::Obj(_) => Some(j),
            _ => None,
        }
    }

    /// The value of `key` (the last one, as `JSON.parse` keeps it).
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// The fields this module compares entry by entry.
pub const FIELDS: &[&str] = &[
    "exports",
    "imports",
    "main",
    "module",
    "browser",
    "types",
    "typings",
    "typesVersions",
];

/// A shape the model does not understand, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape(pub String);

/// Whether the answer to one request differs between two manifests.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// What a runtime resolver gets differs.
    pub runtime: bool,
    /// What type resolution gets differs.
    pub types: bool,
}

impl Outcome {
    /// Whether anything differs.
    pub fn any(self) -> bool {
        self.runtime || self.types
    }
}

/// Checks a target tree: strings, `null`, arrays of targets, condition objects.
fn check_target(t: &Json, field: &str) -> Result<(), Shape> {
    match t {
        Json::Str(_) | Json::Null => Ok(()),
        Json::Arr(a) => a.iter().try_for_each(|x| check_target(x, field)),
        Json::Obj(m) => m.iter().try_for_each(|(k, v)| {
            if k.starts_with('.') || k.starts_with('#') {
                return Err(Shape(format!(
                    "`{field}` nests the subpath `{k}` inside conditions"
                )));
            }
            check_target(v, field)
        }),
        _ => Err(Shape(format!("`{field}` has a target that is not a path"))),
    }
}

/// `exports` as a subpath map: sugar forms (`"./a.js"`, a condition object, an
/// array) map `.`.
fn subpath_map(exports: &Json) -> Result<Vec<(String, Json)>, Shape> {
    let map = match exports {
        Json::Obj(m) if m.iter().any(|(k, _)| k.starts_with('.')) => {
            if !m.iter().all(|(k, _)| k.starts_with('.')) {
                return Err(Shape("`exports` mixes subpaths and conditions".into()));
            }
            m.clone()
        }
        Json::Obj(_) | Json::Str(_) | Json::Arr(_) | Json::Null => {
            vec![(".".to_owned(), exports.clone())]
        }
        _ => return Err(Shape("`exports` is not a path, map or array".into())),
    };
    for (_, t) in &map {
        check_target(t, "exports")?;
    }
    Ok(map)
}

/// `imports` as a map of `#` specifiers.
fn imports_map(imports: &Json) -> Result<Vec<(String, Json)>, Shape> {
    let Json::Obj(m) = imports else {
        return Err(Shape("`imports` is not an object".into()));
    };
    for (k, t) in m {
        if !k.starts_with('#') {
            return Err(Shape(format!("`imports` has the key `{k}` without `#`")));
        }
        check_target(t, "imports")?;
    }
    Ok(m.clone())
}

/// Node's `PATTERN_KEY_COMPARE`: whether pattern key `a` wins over `b`.
fn pattern_wins(a: &str, b: &str) -> bool {
    let base = |k: &str| k.find('*').map_or(k.len(), |i| i + 1);
    (base(a), a.len()) > (base(b), b.len())
}

/// The key that answers `request` and the text its `*` matched: the exact key,
/// else the best pattern (a legacy `./dir/` key counts as `./dir/*`).
fn best_key<'a>(keys: impl Iterator<Item = &'a str>, request: &str) -> Option<(&'a str, String)> {
    let mut best: Option<(&'a str, String, String)> = None;
    for key in keys {
        if key == request && !key.contains('*') && !key.ends_with('/') {
            return Some((key, String::new()));
        }
        let pattern = if key.ends_with('/') {
            format!("{key}*")
        } else {
            key.to_owned()
        };
        let Some(star) = pattern.find('*') else {
            continue;
        };
        if pattern[star + 1..].contains('*') {
            continue;
        }
        let (base, trailer) = (&pattern[..star], &pattern[star + 1..]);
        if request.len() >= pattern.len()
            && request.starts_with(base)
            && request.ends_with(trailer)
            && best.as_ref().is_none_or(|b| pattern_wins(&pattern, &b.2))
        {
            let matched = request[base.len()..request.len() - trailer.len()].to_owned();
            best = Some((key, matched, pattern));
        }
    }
    best.map(|(k, m, _)| (k, m))
}

/// `tree` with `*` replaced by `matched` in every target.
fn substitute(tree: &Json, matched: &str) -> Json {
    match tree {
        Json::Str(s) => Json::Str(s.replace('*', matched)),
        Json::Arr(a) => Json::Arr(a.iter().map(|x| substitute(x, matched)).collect()),
        Json::Obj(m) => Json::Obj(
            m.iter()
                .map(|(k, v)| (k.clone(), substitute(v, matched)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `tree` without `types` conditions: what a runtime resolver can reach.
fn runtime_view(tree: &Json) -> Json {
    match tree {
        Json::Arr(a) => Json::Arr(a.iter().map(runtime_view).collect()),
        Json::Obj(m) => Json::Obj(
            m.iter()
                .filter(|(k, _)| k != "types")
                .map(|(k, v)| (k.clone(), runtime_view(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The target tree a map gives `request`, with the pattern match substituted.
fn lookup(map: &[(String, Json)], request: &str) -> Option<Json> {
    let (key, matched) = best_key(map.iter().map(|(k, _)| k.as_str()), request)?;
    let tree = map.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)?;
    Some(substitute(tree, &matched))
}

/// What each `typesVersions` range maps `request` (a subpath without `./`) to.
fn types_versions_lookup(tv: &Json, request: &str) -> Result<Vec<(String, Option<Json>)>, Shape> {
    let Json::Obj(ranges) = tv else {
        return Err(Shape("`typesVersions` is not an object".into()));
    };
    let mut out = Vec::new();
    for (range, paths) in ranges {
        let Json::Obj(paths) = paths else {
            return Err(Shape("a `typesVersions` range is not a path map".into()));
        };
        out.push((range.clone(), lookup(paths, request)));
    }
    Ok(out)
}

/// A field that has one of two forms on both sides, or is absent: `Ok(None)`
/// when absent on both, an error when present on one side only and `one_sided`
/// says so.
fn both<'a, T>(
    old: &'a Json,
    new: &'a Json,
    field: &str,
    read: impl Fn(&'a Json) -> Result<T, Shape>,
) -> Result<Option<(T, T)>, Shape> {
    match (old.get(field), new.get(field)) {
        (None, None) => Ok(None),
        (Some(a), Some(b)) => Ok(Some((read(a)?, read(b)?))),
        _ => Err(Shape(format!(
            "`{field}` exists at one revision only: every request of the package changes meaning"
        ))),
    }
}

/// Whether the manifests `old` and `new` answer `request` differently.
/// `request` is `.`, a subpath `./x`, or an `imports` specifier `#x`.
pub fn request_changed(old: &Json, new: &Json, request: &str) -> Result<Outcome, Shape> {
    let mut out = Outcome::default();
    if request.starts_with('#') {
        if let Some((a, b)) = both(old, new, "imports", imports_map)? {
            let (ta, tb) = (lookup(&a, request), lookup(&b, request));
            out.types = ta != tb;
            out.runtime = ta.as_ref().map(runtime_view) != tb.as_ref().map(runtime_view);
        }
        return Ok(out);
    }
    if let Some((a, b)) = both(old, new, "exports", subpath_map)? {
        let (ta, tb) = (lookup(&a, request), lookup(&b, request));
        out.types |= ta != tb;
        out.runtime |= ta.as_ref().map(runtime_view) != tb.as_ref().map(runtime_view);
    }
    if request == "." {
        for f in ["main", "module", "browser"] {
            let (a, b) = (old.get(f), new.get(f));
            if a == b {
                continue;
            }
            // A `browser` object remaps files: not an entry point.
            if !matches!(a, None | Some(Json::Str(_))) || !matches!(b, None | Some(Json::Str(_))) {
                return Err(Shape(format!("`{f}` is not a path")));
            }
            out.runtime = true;
            out.types = true;
        }
        for f in ["types", "typings", "typesVersions"] {
            out.types |= old.get(f) != new.get(f);
        }
    } else {
        // `main` and its relatives name the package root only. A `browser` object that
        // changed remaps any file.
        let (a, b) = (old.get("browser"), new.get("browser"));
        if a != b && (matches!(a, Some(Json::Obj(_))) || matches!(b, Some(Json::Obj(_)))) {
            return Err(Shape("`browser` remaps files".into()));
        }
        let sub = request.strip_prefix("./").unwrap_or(request);
        let tv = |m: &Json| match m.get("typesVersions") {
            Some(tv) => types_versions_lookup(tv, sub).map(Some),
            None => Ok(None),
        };
        out.types |= tv(old)? != tv(new)?;
    }
    Ok(out)
}

/// Whether any of [`FIELDS`] differs between the two manifests, as written (key
/// order included).
pub fn fields_differ(old: &Json, new: &Json) -> Vec<&'static str> {
    FIELDS
        .iter()
        .copied()
        .filter(|f| old.get(f) != new.get(f))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(old: &str, new: &str, request: &str) -> Result<Outcome, Shape> {
        request_changed(
            &Json::parse_object(old).unwrap(),
            &Json::parse_object(new).unwrap(),
            request,
        )
    }

    const SAME: Outcome = Outcome {
        runtime: false,
        types: false,
    };
    const BOTH: Outcome = Outcome {
        runtime: true,
        types: true,
    };
    const TYPES: Outcome = Outcome {
        runtime: false,
        types: true,
    };

    #[test]
    fn subpaths_change_alone() {
        let old = r#"{"exports":{".":"./src/index.js","./a":"./src/a.js","./b":"./src/b.js"}}"#;
        let new = r#"{"exports":{".":"./src/index.js","./a":"./src/a2.js","./c":"./src/c.js"}}"#;
        assert_eq!(changed(old, new, "."), Ok(SAME));
        assert_eq!(changed(old, new, "./a"), Ok(BOTH));
        // Removed and added entries.
        assert_eq!(changed(old, new, "./b"), Ok(BOTH));
        assert_eq!(changed(old, new, "./c"), Ok(BOTH));
        assert_eq!(changed(old, new, "./never"), Ok(SAME));
        // Sugar forms are the `.` entry.
        assert_eq!(
            changed(
                r#"{"exports":"./a.js"}"#,
                r#"{"exports":{".":"./a.js"}}"#,
                "."
            ),
            Ok(SAME)
        );
        assert_eq!(
            changed(
                r#"{"exports":"./a.js"}"#,
                r#"{"exports":{"default":"./a.js"}}"#,
                "."
            ),
            Ok(BOTH)
        );
    }

    #[test]
    fn patterns_capture_and_rank() {
        // An added pattern captures requests that nothing answered before.
        let old = r#"{"exports":{".":"./i.js","./x":"./x.js"}}"#;
        let new = r#"{"exports":{".":"./i.js","./x":"./x.js","./*":"./src/*.js"}}"#;
        assert_eq!(changed(old, new, "./x"), Ok(SAME));
        assert_eq!(changed(old, new, "./y"), Ok(BOTH));
        assert_eq!(changed(old, new, "./deep/z"), Ok(BOTH));
        assert_eq!(changed(old, new, "."), Ok(SAME));
        // A more specific pattern takes over only what it matches.
        let old = r#"{"exports":{"./*":"./src/*.js"}}"#;
        let new = r#"{"exports":{"./*":"./src/*.js","./lib/*":"./lib/*.js"}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(SAME));
        assert_eq!(changed(old, new, "./lib/a"), Ok(BOTH));
        // A changed pattern target: every request it answers.
        let new = r#"{"exports":{"./*":"./dist/*.js"}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(BOTH));
        // Trailers and `null` exclusions.
        let old = r#"{"exports":{"./f/*.js":"./f/*.js"}}"#;
        let new = r#"{"exports":{"./f/*.js":"./f/*.js","./f/internal/*":null}}"#;
        assert_eq!(changed(old, new, "./f/a.js"), Ok(SAME));
        assert_eq!(changed(old, new, "./f/internal/a.js"), Ok(BOTH));
        assert_eq!(
            best_key(["./*", "./a/*", "./a/*.js"].into_iter(), "./a/b.js")
                .unwrap()
                .0,
            "./a/*.js"
        );
        assert_eq!(
            best_key(["./*", "./a/*"].into_iter(), "./a/b").unwrap(),
            ("./a/*", "b".to_owned())
        );
        assert_eq!(
            best_key(["./dir/"].into_iter(), "./dir/x").unwrap(),
            ("./dir/", "x".to_owned())
        );
        assert!(best_key(["./a/*"].into_iter(), "./a/").is_none());
    }

    #[test]
    fn conditions_order_and_types() {
        // Condition order decides what matches first.
        let old = r#"{"exports":{".":{"import":"./a.mjs","default":"./a.js"}}}"#;
        let new = r#"{"exports":{".":{"default":"./a.js","import":"./a.mjs"}}}"#;
        assert_eq!(changed(old, new, "."), Ok(BOTH));
        // Nested conditions, arrays.
        let old = r#"{"exports":{"./a":{"node":{"import":"./n.mjs","require":"./n.cjs"},"default":["./x.js","./y.js"]}}}"#;
        let new = r#"{"exports":{"./a":{"node":{"import":"./n.mjs","require":"./n2.cjs"},"default":["./x.js","./y.js"]}}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(BOTH));
        let new = r#"{"exports":{"./a":{"node":{"import":"./n.mjs","require":"./n.cjs"},"default":["./y.js","./x.js"]}}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(BOTH));
        // Only a `types` condition: type resolution alone.
        let old = r#"{"exports":{"./a":{"types":"./a.d.ts","default":"./a.js"}}}"#;
        let new = r#"{"exports":{"./a":{"types":"./a2.d.ts","default":"./a.js"}}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(TYPES));
        let new = r#"{"exports":{"./a":{"default":"./a.js"}}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(TYPES));
    }

    #[test]
    fn entry_fields_are_the_root_request() {
        let old = r#"{"main":"a.js","types":"a.d.ts"}"#;
        assert_eq!(
            changed(old, r#"{"main":"b.js","types":"a.d.ts"}"#, "."),
            Ok(BOTH)
        );
        assert_eq!(
            changed(old, r#"{"main":"b.js","types":"a.d.ts"}"#, "./deep/x"),
            Ok(SAME)
        );
        assert_eq!(
            changed(old, r#"{"main":"a.js","types":"b.d.ts"}"#, "."),
            Ok(TYPES)
        );
        assert_eq!(
            changed(
                old,
                r#"{"main":"a.js","typings":"b.d.ts","types":"a.d.ts"}"#,
                "."
            ),
            Ok(TYPES)
        );
        assert_eq!(
            changed(
                old,
                r#"{"main":"a.js","types":"a.d.ts","module":"a.mjs"}"#,
                "."
            ),
            Ok(BOTH)
        );
        assert_eq!(
            changed(
                old,
                r#"{"main":"a.js","types":"a.d.ts","browser":"a.browser.js"}"#,
                "."
            ),
            Ok(BOTH)
        );
        // A `browser` object remaps files.
        assert!(
            changed(
                old,
                r#"{"main":"a.js","types":"a.d.ts","browser":{"./a.js":"./b.js"}}"#,
                "."
            )
            .is_err()
        );
        assert!(
            changed(
                old,
                r#"{"main":"a.js","types":"a.d.ts","browser":{"./a.js":"./b.js"}}"#,
                "./x"
            )
            .is_err()
        );
    }

    #[test]
    fn imports_and_types_versions() {
        let old = r##"{"imports":{"#a":"./a.js","#lib/*":"./lib/*.js"}}"##;
        let new =
            r##"{"imports":{"#a":"./a.js","#lib/*":"./src/lib/*.js","#b":{"node":"./b.js"}}}"##;
        assert_eq!(changed(old, new, "#a"), Ok(SAME));
        assert_eq!(changed(old, new, "#lib/x"), Ok(BOTH));
        assert_eq!(changed(old, new, "#b"), Ok(BOTH));
        assert_eq!(changed(old, new, "."), Ok(SAME));
        let old = r#"{"typesVersions":{"*":{"a":["dist/a.d.ts"],"x/*":["dist/x/*"]}}}"#;
        let new = r#"{"typesVersions":{"*":{"a":["dist/a.d.ts"],"x/*":["dist/x/*"],"b":["dist/b.d.ts"]}}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(SAME));
        assert_eq!(changed(old, new, "./x/y"), Ok(SAME));
        assert_eq!(changed(old, new, "./b"), Ok(TYPES));
        // The root request reads all of it.
        assert_eq!(changed(old, new, "."), Ok(TYPES));
        let new = r#"{"typesVersions":{">=5":{"a":["dist/a.d.ts"],"x/*":["dist/x/*"]}}}"#;
        assert_eq!(changed(old, new, "./a"), Ok(TYPES));
    }

    #[test]
    fn shapes_outside_the_model() {
        let with = r#"{"exports":{".":"./a.js"}}"#;
        // Present at one revision only.
        assert!(changed("{}", with, ".").is_err());
        assert!(changed(with, r#"{"main":"a.js"}"#, "./x").is_err());
        assert!(changed("{}", r##"{"imports":{"#a":"./a.js"}}"##, "#a").is_err());
        // Mixed keys, leaves that are not paths, subpaths inside conditions.
        assert!(changed(with, r#"{"exports":{".":"./a.js","import":"./b.js"}}"#, ".").is_err());
        assert!(changed(with, r#"{"exports":{".":1}}"#, ".").is_err());
        assert!(
            changed(
                with,
                r#"{"exports":{".":{"import":{"./x":"./a.js"}}}}"#,
                "."
            )
            .is_err()
        );
        assert!(changed(with, r#"{"exports":true}"#, ".").is_err());
        assert!(
            changed(
                r##"{"imports":{"#a":"./a.js"}}"##,
                r#"{"imports":{"a":"./a.js"}}"#,
                "#a"
            )
            .is_err()
        );
        assert!(
            changed(
                r#"{"typesVersions":{"*":{}}}"#,
                r#"{"typesVersions":[]}"#,
                "./a"
            )
            .is_err()
        );
        // `null` exports: nothing is exported, at both revisions the same.
        assert_eq!(
            changed(r#"{"exports":null}"#, r#"{"exports":null}"#, "."),
            Ok(SAME)
        );
    }

    #[test]
    fn field_order_is_seen() {
        let a = Json::parse_object(
            r#"{"exports":{".":{"import":"./a.mjs","default":"./a.js"}},"main":"a.js"}"#,
        )
        .unwrap();
        let b = Json::parse_object(
            r#"{"main":"a.js","exports":{".":{"default":"./a.js","import":"./a.mjs"}}}"#,
        )
        .unwrap();
        assert_eq!(fields_differ(&a, &b), ["exports"]);
        assert!(fields_differ(&a, &a).is_empty());
        assert!(Json::parse_object("[]").is_none());
        assert!(Json::parse_object("{").is_none());
    }
}
