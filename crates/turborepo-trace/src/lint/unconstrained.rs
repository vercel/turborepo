//! The rules `unconstrained-load` and `unconstrained-read`: references whose
//! target the trace cannot bound.
//!
//! Both report the open uncertainty records of first-party files, as the floor
//! report lists them ([`turborepo_trace_graph::FloorReport::sites`]): records
//! that no scope, probe, name bound or hint covers, so that every test that
//! reaches the file is selected on every change. `unconstrained-load` takes the
//! records whose reference loads its target as code ([`Effect::Load`]:
//! `import()`, `require`, workers, child processes); `unconstrained-read` those
//! whose reference reads a file's contents ([`Effect::Read`]: file reads,
//! directory listings, asset paths). Records that no reference raised
//! ([`Effect::None`]: parse errors, task commands) belong to neither.
//!
//! `unconstrained-read` is off by default: in JavaScript and TypeScript a read
//! with a computed path is bounded by other layers (the path-like strings of
//! the code loaded with it, the package it runs for) and is not open; where it
//! is open (other languages, `TURBO_TRACE_DEBUG_NAMES=0`) such reads are far
//! more numerous than loads.
//!
//! The key of a finding is `<kind>/<origin>: <detail>`: the record without its
//! line.
//!
//! "Cannot bound" is about the site. A layer that bounds an open load from the
//! outside does not make the site constrained: in a workspace whose packages
//! resolve only what they declare, a module load with an unknown target is held
//! to the package of its file and of each test and to what those packages
//! declare (`turborepo_trace_graph::loads`, [`Disposition::Declared`]; with
//! `TURBO_TRACE_DEBUG_LOADS_STEP=0` to everything the test's package depends
//! on) and is not on the floor. `unconstrained-load` keeps reporting
//! these loads ([`bounded_loads`]): they are the loads an edit at the site
//! would tighten. The finding has the same key as the open record it would be
//! without that bound (a baseline written with the bound off, or in a hoisted
//! workspace, stays valid), its message says what bounds it, and its cost is
//! the tests that reach the site.

use std::collections::HashMap;

use anyhow::Result;
use turborepo_trace_graph::{
    BoundedSite, Disposition, Effect, FloorRecord, FloorReport, GraphView, NodeId, Origin,
    PackageId, UncertaintyKind,
};

use super::{Cost, Ctx, Finding};

const LOAD: &str = "unconstrained-load";

fn is_load(r: &FloorRecord) -> bool {
    r.class.effect == Effect::Load && !unresolved(r)
}

/// A record of what the test runner is configured with (plugins that are not
/// modelled, a custom resolver: the detail starts with the configuration
/// field), not of a specifier in the code.
fn runner_level(r: &FloorRecord) -> bool {
    r.class.effect == Effect::Load
        && r.class.kind == UncertaintyKind::ConditionalResolution
        && !r.detail.starts_with("pattern ")
        && !r.detail.starts_with('\'')
}

/// `unconstrained-load`.
pub fn loads(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>> {
    let mut out = open_records(ctx, ctx.floor(), LOAD, is_load);
    if ctx.options.loads_declared {
        out.extend(bounded_loads(ctx));
    }
    Ok(at_configs(out))
}

/// A record of the framework pass about what a test runner may load (plugins
/// that are not modelled, a custom resolver) sits on every test of the project,
/// with line 0 and a detail that starts with the place in the runner's
/// configuration that causes it (`<config>:<line> <field>: …`). Reported per
/// test it is one finding per test file for one line of configuration; this
/// makes it one finding at that line, whose key is the detail without the place
/// (a key carries no line number) and whose cost adds up the tests that only it
/// holds.
fn at_configs(findings: Vec<Finding>) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for mut f in findings {
        let place = (f.line == 0)
            .then(|| f.key.split_once(": "))
            .flatten()
            .and_then(|(class, detail)| {
                let (at, rest) = detail.split_once(' ')?;
                let (file, line) = at.rsplit_once(':')?;
                let line: u32 = line.parse().ok()?;
                // `<field>: <text>` follows the place.
                rest.split_once(": ")?;
                (line > 0 && file.contains('.')).then(|| {
                    (
                        file.to_owned(),
                        line,
                        format!("{class}: {rest}"),
                        format!("{at} "),
                    )
                })
            });
        let Some((file, line, key, at)) = place else {
            out.push(f);
            continue;
        };
        match out
            .iter_mut()
            .find(|o| o.rule == f.rule && o.file == file && o.line == line && o.key == key)
        {
            Some(o) => {
                // The tests only this record holds add up (each is its own); the tests
                // that reach any of them are at least the most one of them has.
                if let (Some(a), Some(b)) = (&mut o.cost, f.cost) {
                    a.exclusive += b.exclusive;
                    a.reached = a.reached.max(b.reached).max(a.exclusive);
                }
            }
            None => {
                f.message = f.message.replacen(&at, "", 1);
                f.file = file;
                f.line = line;
                f.key = key;
                out.push(f);
            }
        }
    }
    out
}

/// `unconstrained-read`.
pub fn reads(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>> {
    Ok(open_records(ctx, ctx.floor(), "unconstrained-read", |r| {
        r.class.effect == Effect::Read && !unresolved(r)
    }))
}

/// `unresolved-reference`: a literal path or module name that resolves to
/// nothing and that nothing stands for. It is not a computed target (the code
/// says what it means), so it is a rule of its own; in JavaScript such a
/// reference is covered by the paths its resolution probed and never open.
pub fn unresolved_references(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>> {
    Ok(open_records(
        ctx,
        ctx.floor(),
        "unresolved-reference",
        |r| r.class.effect != Effect::None && unresolved(r),
    ))
}

fn unresolved(r: &FloorRecord) -> bool {
    r.class.kind == UncertaintyKind::UnresolvedSpecifier
}

/// The key of a record's finding: the record without its line.
fn key(rec: &FloorRecord) -> String {
    format!("{}/{}: {}", rec.class.kind, rec.class.origin, rec.detail)
}

/// One finding per open record of `floor` that `keep` accepts, in files (a
/// record on a task or on a directory listing has no line of code to point at).
fn open_records(
    ctx: &Ctx<'_, '_>,
    floor: &FloorReport,
    rule: &'static str,
    keep: fn(&FloorRecord) -> bool,
) -> Vec<Finding> {
    let g = ctx.derived;
    let mut out = Vec::new();
    for site in &floor.sites {
        // The JavaScript extractor writes details as `<source text>; <label>
        // <payload>`.
        let lang = g.lang(site.node);
        if lang.is_none() {
            continue;
        }
        let js = lang == Some("js");
        let cost = Cost {
            reached: site.reached,
            exclusive: site.exclusive,
            bounded: false,
        };
        for rec in site.records.iter().filter(|r| keep(r)) {
            out.push(Finding {
                rule,
                file: g.path(site.node).to_owned(),
                line: rec.line,
                key: key(rec),
                message: message(rec, js),
                cost: Some(cost),
                fix: fix_of(rec, lang),
            });
        }
    }
    out
}

/// The module loads with an unknown target that are not on the floor because
/// the declared packages bound them ([`Disposition::Declared`]): one finding
/// per record, as [`open_records`] reports it where that bound does not apply,
/// with the bound in the message. The cost is taken from the floor without the
/// bound ([`Ctx::floor_without_bounds`]): the tests that reach the site, and of
/// those the ones that reach no other such site and no open one.
fn bounded_loads(ctx: &Ctx<'_, '_>) -> Vec<Finding> {
    let g = ctx.derived;
    let mut out = Vec::new();
    // The packages of the tests that reach each site (walked only if there is a
    // finding).
    let mut by_node: Option<HashMap<NodeId, &BoundedSite>> = None;
    for site in &ctx.floor_without_bounds().sites {
        let lang = g.lang(site.node);
        if lang.is_none() {
            continue;
        }
        let js = lang == Some("js");
        let declared = |rec: &FloorRecord| {
            g.uncertainties(site.node).any(|u| {
                u.line == rec.line
                    && u.kind == rec.class.kind
                    && u.detail == rec.detail
                    && ctx.engine.disposition_of(site.node, &u, ctx.options)
                        == Disposition::Declared
            })
        };
        let cost = Cost {
            reached: site.reached,
            exclusive: site.exclusive,
            bounded: true,
        };
        for rec in site.records.iter().filter(|r| is_load(r) && declared(r)) {
            let sites = by_node.get_or_insert_with(|| {
                let b = ctx.bounded_loads();
                b.sites.iter().map(|s| (s.node, s)).collect()
            });
            let bound = if ctx.options.loads_step {
                // One resolution step from where the load happens.
                let mut loaders: Vec<PackageId> = Vec::new();
                for f in ctx.engine.loading_files(site.node, ctx.options) {
                    if !loaders.contains(&g.package(f)) {
                        loaders.push(g.package(f));
                    }
                }
                let mut reached: Vec<u32> = loaders
                    .iter()
                    .flat_map(|&p| ctx.reach().of(p).iter().copied())
                    .collect();
                reached.sort_unstable();
                reached.dedup();
                step_text(
                    &loaders
                        .iter()
                        .map(|&p| package_label(g, p))
                        .collect::<Vec<_>>(),
                    reached.len(),
                    ctx.bounded_loads().packages,
                    sites.contains_key(&site.node),
                )
            } else {
                bound_text(
                    sites.get(&site.node).copied(),
                    ctx.bounded_loads().packages,
                    |p| package_label(g, p),
                )
            };
            out.push(Finding {
                rule: LOAD,
                file: g.path(site.node).to_owned(),
                line: rec.line,
                key: key(rec),
                message: format!("{}; {bound}", message(rec, js)),
                cost: Some(cost),
                fix: fix_of(rec, lang),
            });
        }
    }
    out
}

/// `packages/a/`, or `the root package`.
fn package_label<G: GraphView>(g: &G, p: PackageId) -> String {
    let root = g.package_info(p).root.trim_matches('/');
    if root.is_empty() || root == "." {
        "the root package".to_owned()
    } else {
        format!("{root}/")
    }
}

/// What bounds a load from outside its site, for the message: what one
/// resolution step reaches from the packages of the files where the load
/// happens (`loaders`, as labels; `reached` packages of `packages` in the
/// repository), and from the package of each test. `tested`: a test reaches the
/// site.
fn step_text(loaders: &[String], reached: usize, packages: usize, tested: bool) -> String {
    let (who, declares) = match loaders {
        [one] => (one.clone(), "it declares"),
        many => (many.join(", "), "they declare"),
    };
    format!(
        "bounded to {who} and the packages {declares} ({reached} of {packages}), and to the \
         package of each test and what that declares{}",
        if tested {
            ""
        } else {
            " (no test reaches this file)"
        }
    )
}

/// [`step_text`] for the declared closure (`TURBO_TRACE_DEBUG_LOADS_STEP=0`):
/// the packages that the package of each test that reaches the site depends on.
/// `site`: the tests that reach it (`None`: no test does); `packages`: the
/// first-party packages of the repository.
fn bound_text(
    site: Option<&BoundedSite>,
    packages: usize,
    label: impl Fn(PackageId) -> String,
) -> String {
    let Some(s) = site else {
        return "bounded to the packages that the package of a test depends on (no test reaches \
                this file)"
            .to_owned();
    };
    let root = if s.root_entries {
        "; tests of the root package reach it too, and the root depends on every package"
    } else {
        ""
    };
    match (s.entry_package, s.entry_packages) {
        (Some(p), _) => format!(
            "bounded to the packages {} depends on ({} of {packages})",
            label(p),
            s.covers
        ),
        (None, 0) => {
            "bounded to the packages that the root package depends on: every package".to_owned()
        }
        (None, n) => format!(
            "bounded to the packages each test's package depends on (tests of {n} packages reach \
             it, which depend on {} of {packages} packages){root}",
            s.covers
        ),
    }
}

/// The payload a detail carries after `label` (`…; parameter exports.load#0`).
fn payload<'a>(detail: &'a str, label: &str) -> Option<&'a str> {
    detail
        .split("; ")
        .skip(1)
        .find_map(|p| p.strip_prefix(label)?.strip_prefix(' '))
        .filter(|p| !p.is_empty())
}

/// The first `max` characters of `s`, with `…` if something was cut.
fn short(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

/// What makes the record unbounded, in one line. `js`: the detail starts with
/// the source text of the specifier.
fn message(rec: &FloorRecord, js: bool) -> String {
    let load = rec.class.effect == Effect::Load;
    let (what, anything, bound) = if load {
        (
            "specifier",
            "any module can be loaded here",
            "what is loaded here",
        )
    } else {
        ("path", "any file can be read here", "what is read here")
    };
    let detail = rec.detail.as_str();
    match rec.class.kind {
        UncertaintyKind::ComputedSpecifier
        | UncertaintyKind::EnvDependent
        | UncertaintyKind::CwdDependent => {}
        UncertaintyKind::AliasedLoader => {
            return format!(
                "{}: a load through an alias is not analyzed, so {anything}",
                short(detail, 100)
            );
        }
        UncertaintyKind::UnresolvedSpecifier => {
            return format!(
                "{}: the reference names nothing turbo trace can find, and nothing stands for it",
                short(detail, 120)
            );
        }
        // What the test runner is configured with: plugins that are not modelled, a custom
        // resolver (the detail starts with the configuration field).
        UncertaintyKind::ConditionalResolution if runner_level(rec) => {
            return format!(
                "{}: the test runner may load another module than the one turbo trace sees",
                short(detail, 160)
            );
        }
        // A prefix or a check was recognized, but what it names is not expanded: it matches
        // too many files, or a module mapper may apply to it.
        UncertaintyKind::PatternTooBroad | UncertaintyKind::ConditionalResolution if load => {
            return format!(
                "the load is held to a pattern that is not expanded ({}): the prefix names too \
                 much to bound it",
                short(detail, 120)
            );
        }
        kind => {
            return format!(
                "{} ({kind}): turbo trace cannot bound {bound}",
                short(detail, 100)
            );
        }
    }
    let expr = detail.split("; ").next().unwrap_or(detail);
    let subject = if js {
        format!("the {what} `{}`", short(expr, 60))
    } else if load {
        "the target of this load".to_owned()
    } else {
        "the path".to_owned()
    };
    // Details of other languages are prose: quoted after the phrase.
    let quoted = if js {
        String::new()
    } else {
        format!(" ({})", short(detail, 100))
    };
    let mut consequence = anything;
    let predicate = match rec.class.origin {
        Origin::Param => {
            match payload(detail, "parameter").map(|p| p.split('#').next().unwrap_or(p)) {
                Some(func) if js && is_identifier(expr) => {
                    return format!("the {what} is the parameter `{expr}` of `{func}`: {anything}");
                }
                Some(func) => format!("comes from a parameter of `{func}`"),
                None => "comes from a function parameter".to_owned(),
            }
        }
        Origin::Env => match payload(detail, "env") {
            Some(var) => format!("comes from the environment variable `{var}`"),
            None => "comes from the environment".to_owned(),
        },
        Origin::Cwd => {
            consequence = if load {
                "what it loads depends on where the process is started"
            } else {
                "what it reads depends on where the process is started"
            };
            "is relative to the working directory".to_owned()
        }
        Origin::Argv => "comes from the command line".to_owned(),
        Origin::Imported => match payload(detail, "imported").and_then(|p| p.rsplit_once('#')) {
            Some((module, name)) if name == expr => format!("is imported from `{module}`"),
            Some((module, name)) => format!("comes from `{name}`, imported from `{module}`"),
            None => "comes from a binding imported from another module".to_owned(),
        },
        Origin::Call => match payload(detail, "call") {
            Some(callee) => {
                format!("is the result of calling `{callee}`, which turbo trace cannot evaluate")
            }
            None => "is the result of a call turbo trace cannot evaluate".to_owned(),
        },
        Origin::Data => "comes from data known only at run time".to_owned(),
        Origin::Unknown => "is computed from values turbo trace cannot follow".to_owned(),
    };
    format!("{subject} {predicate}{quoted}: {consequence}")
}

const LOAD_PREFIX: &str = "give the specifier a literal prefix: `import(`./handlers/${name}`)` or \
                           `require(path.join(__dirname, 'handlers', name))`";
const LOAD_CHECK: &str = "check it before the load: `if (!name.startsWith('./handlers/')) throw \
                          …` (turbo trace then bounds the load to that directory)";
const LOAD_TABLE: &str = "index a literal table: `const HANDLERS = { a: () => \
                          import('./handlers/a') }; HANDLERS[name]()`";
const LOAD_NARROW: &str = "name one directory in the prefix or the check, close to the modules \
                           that are loaded (a pattern is expanded up to `--max-glob` matches, \
                           1000 by default)";
const UNRESOLVED: &str = "name a file that exists (a path relative to the script, not to where it \
                          happens to be started), or remove the reference";
const LAST_RESORT: &str =
    "if none fits: an inline suppression with a reason (`turbo-trace-ignore <rule>: <reason>`)";
const READ_PREFIX: &str =
    "give the path a literal directory: `fs.readFileSync(path.join(__dirname, 'fixtures', name))`";
const PY_LOAD_PREFIX: &str =
    "give the module name a literal prefix: `importlib.import_module(f\"handlers.{name}\")`";
const PY_READ_PREFIX: &str = "give the path a literal directory: \
                              `open(os.path.join(os.path.dirname(__file__), \"fixtures\", name))`";
const ANY_LOAD_PREFIX: &str =
    "give the loaded path a literal prefix, so that the load stays in one directory";
const ANY_READ_PREFIX: &str =
    "give the path a literal prefix, so that the read stays in one directory";
const RUNNER_DECLARE: &str = "no specifier to narrow here: declare in the project's package.json \
                              every package whose modules the plugin or resolver may load (the \
                              bound is the project's package and the packages it declares)";
const RUNNER_REPLACE: &str = "or configure it where turbo trace reads it (an alias or a module \
                              mapper with literal targets), or add an inline suppression with a \
                              reason";

/// The fix of a record: for what the test runner is configured with there is no
/// specifier in the code to change ([`runner_level`]); otherwise [`fix`].
fn fix_of(rec: &FloorRecord, lang: Option<&str>) -> String {
    if runner_level(rec) {
        return format!("{RUNNER_DECLARE}\n{RUNNER_REPLACE}");
    }
    fix(rec.class.effect, rec.class.kind, rec.class.origin, lang)
}

/// The fix patterns that fit the origin best, most preferred first: two lines.
/// The four patterns for loads, in their general order of preference: a literal
/// prefix, a check before the load, a literal table, and as the last resort a
/// hint or a suppression. The check and the table are what the JavaScript
/// analysis recognizes; other languages get the prefix in their own syntax.
fn fix(effect: Effect, kind: UncertaintyKind, origin: Origin, lang: Option<&str>) -> String {
    let load = effect == Effect::Load;
    if kind == UncertaintyKind::UnresolvedSpecifier {
        return format!("{UNRESOLVED}\n{LAST_RESORT}");
    }
    match (lang, load) {
        // The site already has a prefix: it is the prefix that has to change.
        (Some("js"), true)
            if matches!(
                kind,
                UncertaintyKind::PatternTooBroad | UncertaintyKind::ConditionalResolution
            ) =>
        {
            return format!("{LOAD_NARROW}\nor {LOAD_TABLE}");
        }
        (Some("js"), true) => {}
        (Some("js"), false) => return format!("{READ_PREFIX}\n{LAST_RESORT}"),
        (Some("python"), true) => return format!("{PY_LOAD_PREFIX}\n{LAST_RESORT}"),
        (Some("python"), false) => return format!("{PY_READ_PREFIX}\n{LAST_RESORT}"),
        (_, true) => return format!("{ANY_LOAD_PREFIX}\n{LAST_RESORT}"),
        (_, false) => return format!("{ANY_READ_PREFIX}\n{LAST_RESORT}"),
    }
    match origin {
        // A value of the code itself: keep it, and say where it points.
        Origin::Param | Origin::Imported | Origin::Call => {
            format!("{LOAD_PREFIX}\nor {LOAD_CHECK}")
        }
        // A value from outside the code selects one of the modules the code knows.
        Origin::Env | Origin::Argv | Origin::Data => format!("{LOAD_TABLE}\nor {LOAD_PREFIX}"),
        Origin::Cwd | Origin::Unknown => format!("{LOAD_PREFIX}\n{LAST_RESORT}"),
    }
}

#[cfg(test)]
mod tests {
    use turborepo_trace_graph::UncertaintyClass;

    use super::*;

    fn rec(kind: UncertaintyKind, effect: Effect, origin: Origin, detail: &str) -> FloorRecord {
        FloorRecord {
            line: 1,
            class: UncertaintyClass {
                kind,
                effect,
                origin,
            },
            detail: detail.to_owned(),
        }
    }

    #[test]
    fn one_phrase_per_origin() {
        let computed = UncertaintyKind::ComputedSpecifier;
        let load =
            |origin, detail: &str| message(&rec(computed, Effect::Load, origin, detail), true);
        assert_eq!(
            load(Origin::Param, "spec; parameter exports.normalize#0"),
            "the specifier is the parameter `spec` of `exports.normalize`: any module can be \
             loaded here"
        );
        assert_eq!(
            load(Origin::Param, "dir + '/' + name; parameter load#1"),
            "the specifier `dir + '/' + name` comes from a parameter of `load`: any module can be \
             loaded here"
        );
        assert_eq!(
            load(Origin::Imported, "target; imported ./cfg#target"),
            "the specifier `target` is imported from `./cfg`: any module can be loaded here"
        );
        assert_eq!(
            load(Origin::Imported, "dir + target; imported ./cfg#target"),
            "the specifier `dir + target` comes from `target`, imported from `./cfg`: any module \
             can be loaded here"
        );
        assert_eq!(
            load(Origin::Argv, "process.argv[2]"),
            "the specifier `process.argv[2]` comes from the command line: any module can be \
             loaded here"
        );
        assert_eq!(
            load(Origin::Call, "pick()"),
            "the specifier `pick()` is the result of a call turbo trace cannot evaluate: any \
             module can be loaded here"
        );
        assert_eq!(
            load(Origin::Data, "cfg.module"),
            "the specifier `cfg.module` comes from data known only at run time: any module can be \
             loaded here"
        );
        assert_eq!(
            load(Origin::Unknown, "x"),
            "the specifier `x` is computed from values turbo trace cannot follow: any module can \
             be loaded here"
        );
        assert_eq!(
            message(
                &rec(
                    UncertaintyKind::EnvDependent,
                    Effect::Load,
                    Origin::Env,
                    "process.env.PLUGIN; env PLUGIN; the repository never assigns PLUGIN"
                ),
                true
            ),
            "the specifier `process.env.PLUGIN` comes from the environment variable `PLUGIN`: any \
             module can be loaded here"
        );
        assert_eq!(
            message(
                &rec(
                    UncertaintyKind::CwdDependent,
                    Effect::Load,
                    Origin::Cwd,
                    "process.cwd() + '/src/routes'"
                ),
                true
            ),
            "the specifier `process.cwd() + '/src/routes'` is relative to the working directory: \
             what it loads depends on where the process is started"
        );
        // A read; another language (the detail is prose); another kind.
        assert_eq!(
            message(
                &rec(
                    computed,
                    Effect::Read,
                    Origin::Param,
                    "p; parameter exports.read#0"
                ),
                true
            ),
            "the path is the parameter `p` of `exports.read`: any file can be read here"
        );
        assert_eq!(
            message(
                &rec(
                    computed,
                    Effect::Load,
                    Origin::Unknown,
                    "computed path or module in importlib.import_module(name)"
                ),
                false
            ),
            "the target of this load is computed from values turbo trace cannot follow (computed \
             path or module in importlib.import_module(name)): any module can be loaded here"
        );
        assert_eq!(
            message(
                &rec(
                    UncertaintyKind::AliasedLoader,
                    Effect::Load,
                    Origin::Unknown,
                    "`r` is an alias of `require`"
                ),
                true
            ),
            "`r` is an alias of `require`: a load through an alias is not analyzed, so any module \
             can be loaded here"
        );
        assert_eq!(
            message(
                &rec(
                    UncertaintyKind::PatternTooBroad,
                    Effect::Load,
                    Origin::Unknown,
                    "./**/*.js matches 20000 files"
                ),
                true
            ),
            "the load is held to a pattern that is not expanded (./**/*.js matches 20000 files): \
             the prefix names too much to bound it"
        );
        assert!(
            fix(
                Effect::Load,
                UncertaintyKind::PatternTooBroad,
                Origin::Unknown,
                Some("js")
            )
            .starts_with("name one directory in the prefix or the check")
        );
    }

    #[test]
    fn fixes_by_origin() {
        let lines = |origin| {
            let f = fix(
                Effect::Load,
                UncertaintyKind::ComputedSpecifier,
                origin,
                Some("js"),
            );
            f.lines().map(str::to_owned).collect::<Vec<_>>()
        };
        for origin in Origin::ALL {
            assert_eq!(lines(*origin).len(), 2, "{origin}");
        }
        assert!(lines(Origin::Param)[0].starts_with("give the specifier a literal prefix"));
        assert!(lines(Origin::Param)[1].starts_with("or check it before the load"));
        assert!(lines(Origin::Env)[0].starts_with("index a literal table"));
        assert!(lines(Origin::Unknown)[1].contains("an inline suppression"));
        let first = |effect, lang| {
            let f = fix(
                effect,
                UncertaintyKind::ComputedSpecifier,
                Origin::Param,
                lang,
            );
            assert_eq!(f.lines().count(), 2);
            f.lines().next().unwrap().to_owned()
        };
        assert!(first(Effect::Read, Some("js")).contains("fs.readFileSync(path.join(__dirname"));
        assert!(
            first(Effect::Load, Some("python")).contains("importlib.import_module(f\"handlers.")
        );
        assert!(first(Effect::Read, Some("python")).contains("os.path.dirname(__file__)"));
        assert!(
            first(Effect::Load, Some("shell")).starts_with("give the loaded path a literal prefix")
        );
        assert!(first(Effect::Read, None).starts_with("give the path a literal prefix"));
        // What the runner is configured with has no specifier to change; a pattern in
        // the code that a mapper may apply to has.
        let conditional = UncertaintyKind::ConditionalResolution;
        let of = |detail: &str| {
            let f = fix_of(
                &rec(conditional, Effect::Load, Origin::Unknown, detail),
                Some("js"),
            );
            assert_eq!(f.lines().count(), 2);
            f.lines().next().unwrap().to_owned()
        };
        assert!(
            of("plugins: vite plugins are not modelled").starts_with("no specifier to narrow here")
        );
        assert!(of("resolver: a custom resolver").starts_with("no specifier to narrow here"));
        assert!(
            of("pattern ./handlers/*: a module mapper may apply")
                .starts_with("name one directory in the prefix")
        );
    }

    #[test]
    fn shortens_long_source_text() {
        assert_eq!(short("abcdef", 3), "abc…");
        assert_eq!(short("abc", 3), "abc");
        assert_eq!(payload("x; env A; more", "env"), Some("A"));
        assert_eq!(payload("x; environment", "env"), None);
        assert_eq!(
            payload("env A", "env"),
            None,
            "the first part is the source text"
        );
    }
}
