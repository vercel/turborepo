//! The difference between two build fingerprints: which routes changed, and
//! why.
//!
//! - `changed`: routes whose own output differs, with the components that
//!   differ (`server`, `client`, `css`, `prerender`, `external`);
//! - `affected`: routes whose own output is the same but that invoke
//!   (transitively) a route that changed, was added or was removed;
//! - `added` / `removed` routes;
//! - `global`: inputs of every route that differ (`middleware`,
//!   `instrumentation`, `middleware_matchers`, `routing`, `route_table`,
//!   `config`, `images`);
//! - `rules` and `route_table`: the routing rules and route regexes that were
//!   added or removed, so a caller can test the paths a spec requested against
//!   them instead of treating every routing change as global;
//! - `public`: files under `public/` that were added, removed or modified.
//!
//! Stylesheets are compared by rule ([`super::css`]): with
//! [`CssMode::Utilities`] a stylesheet change that only adds or removes utility
//! classes does not make a route `changed`; such routes are listed in
//! `css_only_utilities`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    css::{self, CssVerdict},
    fingerprint::Fingerprint,
    invokes::reaches,
    manifest::{BuildRule, RouteRegex},
};

/// How stylesheet changes count.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CssMode {
    /// Utility-only changes do not count (assumes no class name is built at
    /// runtime).
    #[default]
    Utilities,
    /// Every stylesheet change counts.
    Strict,
}

/// Added and removed items.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta<T> {
    /// Only in the second fingerprint.
    pub added: Vec<T>,
    /// Only in the first.
    pub removed: Vec<T>,
}

impl<T> Default for Delta<T> {
    fn default() -> Self {
        Delta {
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
}

/// The difference between two fingerprints.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FingerprintDiff {
    /// Routes whose own output differs → the components that differ.
    pub changed: BTreeMap<String, Vec<String>>,
    /// Routes with unchanged own output → the changed, added or removed routes
    /// they invoke.
    pub affected: BTreeMap<String, Vec<String>>,
    /// Routes whose only difference is a utility-only stylesheet change.
    pub css_only_utilities: Vec<String>,
    /// Routes only in the second build.
    pub added: Vec<String>,
    /// Routes only in the first build.
    pub removed: Vec<String>,
    /// Global inputs that differ.
    pub global: Vec<String>,
    /// Routing rules added or removed.
    pub rules: Delta<BuildRule>,
    /// Route regexes added or removed.
    pub route_table: Delta<RouteRegex>,
    /// The stylesheet comparisons made, by `<base rule set>:<head rule set>`.
    pub css: BTreeMap<String, CssVerdict>,
    /// Files under `public/` that were added, removed or modified.
    #[serde(default)]
    pub public: Vec<String>,
}

impl FingerprintDiff {
    /// Whether nothing differs.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
            && self.affected.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.global.is_empty()
            && self.public.is_empty()
    }
}

/// Compares `base` with `head`. Fails if the two were not computed the same
/// way.
pub fn diff(
    base: &Fingerprint,
    head: &Fingerprint,
    css_mode: CssMode,
) -> Result<FingerprintDiff, String> {
    if base.version != head.version {
        return Err(format!(
            "fingerprint versions differ ({} and {}): recompute both",
            base.version, head.version
        ));
    }
    if base.per_module != head.per_module || base.volatile != head.volatile {
        return Err(
            "the fingerprints were computed with different options (per-module hashing or \
             volatile values): recompute both the same way"
                .to_string(),
        );
    }
    let mut d = FingerprintDiff::default();
    for p in head.routes.keys() {
        if !base.routes.contains_key(p) {
            d.added.push(p.clone());
        }
    }
    for (path, x) in &base.routes {
        let Some(y) = head.routes.get(path) else {
            d.removed.push(path.clone());
            continue;
        };
        let mut why: Vec<String> = Vec::new();
        for (name, a, b) in [
            ("server", &x.server.hash, &y.server.hash),
            ("client", &x.client.hash, &y.client.hash),
            ("prerender", &x.prerender.hash, &y.prerender.hash),
            ("external", &x.external, &y.external),
        ] {
            if a != b {
                why.push(name.to_string());
            }
        }
        if x.css.hash != y.css.hash {
            let verdict = match (&x.css_rules, &y.css_rules) {
                (Some(i), Some(j)) => {
                    let key = format!("{i}:{j}");
                    let v = d.css.entry(key).or_insert_with(|| {
                        match (base.css.get(i), head.css.get(j)) {
                            (Some(a), Some(b)) => css::compare(a, b),
                            _ => CssVerdict::default(),
                        }
                    });
                    v.utilities_only
                }
                _ => false,
            };
            if css_mode == CssMode::Strict || !verdict {
                why.push("css".to_string());
            } else if why.is_empty() {
                d.css_only_utilities.push(path.clone());
            }
        }
        if !why.is_empty() {
            d.changed.insert(path.clone(), why);
        }
    }

    // Routes that invoke something that changed.
    let moved: BTreeSet<&String> = d.changed.keys().chain(&d.added).chain(&d.removed).collect();
    if !moved.is_empty() {
        for path in base.routes.keys().filter(|p| head.routes.contains_key(*p)) {
            if d.changed.contains_key(path) {
                continue;
            }
            let mut via: BTreeSet<String> = reaches(base, path);
            via.extend(reaches(head, path));
            let hit: Vec<String> = via.into_iter().filter(|t| moved.contains(t)).collect();
            if !hit.is_empty() {
                d.affected.insert(path.clone(), hit);
            }
        }
    }

    let keys: BTreeSet<&String> = base.global.keys().chain(head.global.keys()).collect();
    for k in keys {
        if base.global.get(k) != head.global.get(k) {
            d.global.push(k.clone());
        }
    }
    let files: BTreeSet<&String> = base.public.keys().chain(head.public.keys()).collect();
    for f in files {
        if base.public.get(f) != head.public.get(f) {
            d.public.push(f.clone());
        }
    }
    let rule_key = |r: &BuildRule| format!("{} {}", r.phase, r.raw);
    let (a, b): (BTreeSet<String>, BTreeSet<String>) = (
        base.rules.iter().map(rule_key).collect(),
        head.rules.iter().map(rule_key).collect(),
    );
    d.rules.added = head
        .rules
        .iter()
        .filter(|r| !a.contains(&rule_key(r)))
        .cloned()
        .collect();
    d.rules.removed = base
        .rules
        .iter()
        .filter(|r| !b.contains(&rule_key(r)))
        .cloned()
        .collect();
    d.route_table.added = head
        .route_regexes
        .iter()
        .filter(|r| !base.route_regexes.contains(r))
        .cloned()
        .collect();
    d.route_table.removed = base
        .route_regexes
        .iter()
        .filter(|r| !head.route_regexes.contains(r))
        .cloned()
        .collect();
    Ok(d)
}
