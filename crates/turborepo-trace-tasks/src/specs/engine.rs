//! The interpreter: one [`Recognizer`] per [`ToolSpec`].

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;
use turborepo_trace_lang_js::extract::config::ConfigVal;

use super::format::{Default, Rule, RuleRef, Search, Seg, ToolSpec, ValueSpec, parse_path, pkg};
use crate::{
    recognizers::{
        codegen::config::{
            self, PathRole, add_location, as_str, dir_of, entries, get, output_pattern, str_list,
            unknown,
        },
        util::{Arg, Args, add_code_config, find_up},
    },
    registry::{Recognition, RecognizeCx, Recognizer, registry},
    scanners::{self, Target},
    spec::{Command, TaskSpec},
};

/// A recognizer interpreting a [`ToolSpec`].
pub struct SpecRecognizer {
    /// The spec.
    pub spec: ToolSpec,
    id: &'static str,
    programs: &'static [&'static str],
    value_flags: Vec<&'static str>,
}

impl SpecRecognizer {
    /// Parses and validates a TOML spec.
    pub fn parse(src: &str) -> Result<SpecRecognizer, String> {
        let spec: ToolSpec = toml::from_str(src).map_err(|e| e.to_string())?;
        spec.validate()?;
        let leak = |s: &str| -> &'static str { Box::leak(s.to_string().into_boxed_str()) };
        Ok(SpecRecognizer {
            id: leak(&spec.id),
            programs: Box::leak(spec.programs.iter().map(|p| leak(p)).collect()),
            value_flags: spec
                .flags
                .iter()
                .flat_map(|d| d.split('|'))
                .map(leak)
                .collect(),
            spec,
        })
    }
}

impl Recognizer for SpecRecognizer {
    fn id(&self) -> &'static str {
        self.id
    }

    fn programs(&self) -> &'static [&'static str] {
        self.programs
    }

    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        let s = &self.spec;
        for t in &s.toolchain {
            if let Ok((e, n)) = pkg(t) {
                out.tool(e, n);
            }
        }
        let mut flags = Vec::new();
        let mut pos = Vec::new();
        for a in Args::new(cmd, 0, &self.value_flags).collect_all() {
            match a {
                Arg::Opt(n, v) => flags.push((s.canonical(n).to_string(), v.map(str::to_string))),
                Arg::Pos(p) => pos.push(p.to_string()),
            }
        }
        let mut run = Run {
            spec: s,
            cmd,
            cx,
            flags,
            args: pos.clone(),
            sub: String::new(),
            cfg: None,
            cfg_failed: false,
            values: HashMap::new(),
        };
        // Select the rules.
        let (rules, load_config) = if !s.sub.is_empty() {
            let w0 = pos.first().map_or("", String::as_str);
            let w0 = match &s.subcommand_split {
                Some(sep) => w0.split(sep.as_str()).next().unwrap_or(w0),
                None => w0,
            };
            if s.ignore.iter().any(|i| i == w0) {
                return Recognition::Ignored;
            }
            let two = pos.get(1).map(|w1| format!("{w0} {w1}"));
            let found = two
                .as_deref()
                .and_then(|k| self.sub(k).map(|r| (r, 2)))
                .or_else(|| self.sub(w0).map(|r| (r, 1)));
            let Some((rules, n)) = found else {
                let depth = if s.sub.keys().any(|k| k.contains(' ')) {
                    2
                } else {
                    1
                };
                let mut words = vec![w0];
                words.extend(pos.iter().skip(1).take(depth - 1).map(String::as_str));
                let detail = format!("{} {}", self.id, words.join(" "));
                unknown(
                    out,
                    cx.package_root,
                    UncertaintyKind::UnknownCommand,
                    detail.trim().to_string(),
                );
                return finish(out, before);
            };
            run.sub = w0.to_string();
            run.args.drain(..n);
            (rules, true)
        } else {
            match s.variant.iter().find(|v| run.all(&v.when)) {
                Some(v) => (v.rules.as_slice(), v.config),
                None if s.variant.is_empty() => (&[][..], true),
                None => return Recognition::Ignored,
            }
        };
        out.env.extend(s.env.iter().cloned());
        if load_config {
            run.load_config(out);
        }
        for r in s.always.iter().chain(rules) {
            run.apply(r, out, None);
        }
        finish(out, before)
    }
}

impl SpecRecognizer {
    fn sub(&self, word: &str) -> Option<&[RuleRef]> {
        self.spec
            .sub
            .iter()
            .find(|(k, _)| k.split('|').any(|w| w == word))
            .map(|(_, v)| v.as_slice())
    }
}

/// `Full` unless uncertainties were added since `before`.
fn finish(out: &TaskSpec, before: usize) -> Recognition {
    if out.uncertainties.len() > before {
        Recognition::Partial
    } else {
        Recognition::Full
    }
}

/// A value item: a string, the directory it is relative to, and the config
/// objects that enclose it (outermost first), for `${key}` templates and
/// `base_field`.
#[derive(Clone, Debug)]
struct Item {
    text: String,
    base: PathBuf,
    scope: Vec<ConfigVal>,
}

impl Item {
    fn path(&self) -> PathBuf {
        fsx::join(&self.base, &self.text)
    }

    /// The string `key` of the nearest enclosing object that has it.
    fn lookup(&self, key: &str) -> Option<String> {
        self.scope
            .iter()
            .rev()
            .find_map(|o| get(o, key).as_ref().and_then(as_str))
    }
}

/// The located config.
struct Cfg {
    dir: PathBuf,
    value: Option<ConfigVal>,
}

/// One command being interpreted.
struct Run<'a> {
    spec: &'a ToolSpec,
    cmd: &'a Command,
    cx: &'a RecognizeCx<'a>,
    /// Flags (canonical name, value).
    flags: Vec<(String, Option<String>)>,
    /// Positionals after the subcommand.
    args: Vec<String>,
    /// The subcommand.
    sub: String,
    cfg: Option<Cfg>,
    /// A config-level uncertainty was reported (no further "missing field"
    /// reports).
    cfg_failed: bool,
    values: HashMap<String, Rc<Vec<Item>>>,
}

/// What walking a key path found.
#[derive(Default)]
struct Walk {
    items: Vec<Item>,
    /// Non-static parts: (concrete path, reason).
    unknown: Vec<(String, String)>,
    /// Missing keys of iterated elements (concrete paths).
    missing: Vec<String>,
}

impl Run<'_> {
    fn label(&self) -> &str {
        self.spec
            .config
            .as_ref()
            .map_or(self.spec.id.as_str(), |c| c.label.as_str())
    }

    /// The directory that becomes a membership when something is not statically
    /// known.
    fn ctx_dir(&self) -> PathBuf {
        self.cfg
            .as_ref()
            .map_or_else(|| self.cmd.cwd.clone(), |c| c.dir.clone())
    }

    fn flag_values(&self, f: &str) -> Vec<&str> {
        let c = self.spec.canonical(f);
        self.flags
            .iter()
            .filter(|(n, _)| n == c)
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }

    fn holds(&self, cond: &str) -> bool {
        cond.split('|').any(|c| match c {
            "@args" => !self.args.is_empty(),
            f => {
                let c = self.spec.canonical(f);
                self.flags.iter().any(|(n, _)| n == c)
            }
        })
    }

    fn all(&self, conds: &[String]) -> bool {
        conds.iter().all(|c| self.holds(c))
    }

    fn load_config(&mut self, out: &mut TaskSpec) {
        let Some(c) = &self.spec.config else { return };
        let cwd = &self.cmd.cwd;
        let search_dir = match c.search {
            Search::Package => self.cx.package_root.to_path_buf(),
            _ => cwd.clone(),
        };
        let located = match c
            .flag
            .as_deref()
            .and_then(|f| self.flag_values(f).first().copied())
        {
            Some(p) => Some(self.cmd.path(p)),
            None if c.search == Search::Up => {
                let mut d = Some(cwd.as_path());
                let mut found = None;
                while let Some(x) = d {
                    let names: Vec<&str> = c.names.iter().map(String::as_str).collect();
                    found = fsx::first_existing(x, &names);
                    if found.is_some() || x == self.cx.repo_root {
                        break;
                    }
                    d = x.parent();
                }
                found
            }
            None => {
                let names: Vec<&str> = c.names.iter().map(String::as_str).collect();
                fsx::first_existing(&search_dir, &names)
            }
        };
        let (dir, value) = match located {
            Some(p) => {
                let dir = dir_of(&p);
                let v = config::load(&p, out);
                if v.is_none()
                    && let Some(m) = &c.unreadable
                {
                    unknown(out, &dir, UncertaintyKind::ParseIncomplete, m.clone());
                    self.cfg_failed = true;
                }
                (dir, v)
            }
            None => {
                let from_pkg = c.package_key.as_ref().and_then(|k| {
                    let pj = find_up(cwd, "package.json", Some(self.cx.repo_root))?;
                    out.config(pj.clone());
                    let v = crate::sources::read_json(&pj)?
                        .get(k)
                        .map(config::from_json)?;
                    Some((dir_of(&pj), v))
                });
                match from_pkg {
                    Some((d, v)) => (d, Some(v)),
                    None => {
                        let n = if c.probe_all { c.names.len() } else { 1 };
                        for name in c.names.iter().take(n) {
                            out.config(search_dir.join(name));
                        }
                        if let Some(m) = &c.required {
                            unknown(out, cwd, UncertaintyKind::ParseIncomplete, m.clone());
                            self.cfg_failed = true;
                        }
                        return;
                    }
                }
            }
        };
        if let Some(v) = &value
            && !c.require_any.is_empty()
            && !c.require_any.iter().any(|k| get(v, k).is_some())
        {
            let keys: Vec<String> = c.require_any.iter().map(|k| format!("`{k}`")).collect();
            unknown(
                out,
                &dir,
                UncertaintyKind::ParseIncomplete,
                format!("{} config without {}", c.label, keys.join(" or ")),
            );
        }
        self.cfg = Some(Cfg { dir, value });
    }

    /// The items of value `name` (evaluated once).
    fn value(&mut self, name: &str, out: &mut TaskSpec) -> Rc<Vec<Item>> {
        if let Some(v) = self.values.get(name) {
            return v.clone();
        }
        let cwd = self.cmd.cwd.clone();
        let item = |text: &str, base: &Path| Item {
            text: text.to_string(),
            base: base.to_path_buf(),
            scope: Vec::new(),
        };
        let items = if name == "@args" {
            self.args.iter().map(|a| item(a, &cwd)).collect()
        } else if name.starts_with('-') {
            self.flag_values(name)
                .into_iter()
                .map(|v| item(v, &cwd))
                .collect()
        } else {
            let spec = self.spec;
            self.eval(name, &spec.values[name], out)
        };
        let v = Rc::new(items);
        self.values.insert(name.to_string(), v.clone());
        v
    }

    fn eval(&mut self, name: &str, vs: &ValueSpec, out: &mut TaskSpec) -> Vec<Item> {
        let cwd = self.cmd.cwd.clone();
        let ctx = self.ctx_dir();
        let mut items: Vec<Item> = vs
            .flag
            .iter()
            .flat_map(|f| self.flag_values(f))
            .map(|v| Item {
                text: v.to_string(),
                base: cwd.clone(),
                scope: Vec::new(),
            })
            .collect();
        let mut uncertain = false;
        let mut report = |w: Walk, items: &mut Vec<Item>, out: &mut TaskSpec, label: &str| {
            for (p, r) in w.unknown {
                uncertain = true;
                unknown(
                    out,
                    &ctx,
                    UncertaintyKind::EnvDependent,
                    format!("{label} `{p}`: {r}"),
                );
            }
            if !vs.optional {
                for p in w.missing {
                    uncertain = true;
                    unknown(
                        out,
                        &ctx,
                        UncertaintyKind::ParseIncomplete,
                        format!("{label} config without `{p}`"),
                    );
                }
            }
            items.extend(w.items);
        };
        let label = self.label().to_string();
        if items.is_empty()
            && let Some(Cfg {
                dir,
                value: Some(v),
            }) = &self.cfg
        {
            for f in &vs.field {
                let w = self.walk(v, f, dir, vs);
                report(w, &mut items, out, &label);
            }
        }
        // `package.json` is read (and recorded) even when an earlier source wins.
        if let Some(k) = &vs.pkg
            && let Some(pj) = find_up(&cwd, "package.json", Some(self.cx.repo_root))
        {
            out.config(pj.clone());
            if let Some(j) = crate::sources::read_json(&pj) {
                let w = self.walk(&config::from_json(&j), k, &dir_of(&pj), vs);
                if items.is_empty() {
                    report(w, &mut items, out, "package.json");
                }
            }
        }
        if items.is_empty() {
            let base = match &vs.default_base {
                Some(b) => {
                    let first = self.value(b, out).first().map(Item::path);
                    match first {
                        Some(p) if fsx::is_dir(&p) => p,
                        Some(p) => dir_of(&p),
                        None => cwd.clone(),
                    }
                }
                None => cwd.clone(),
            };
            let mut push = |t: &str| {
                items.push(Item {
                    text: t.to_string(),
                    base: base.clone(),
                    scope: Vec::new(),
                })
            };
            match &vs.default {
                Some(Default::One(d)) => push(d),
                Some(Default::Candidates(cs)) => {
                    let found = cs
                        .iter()
                        .position(|c| fsx::exists(&fsx::join(&base, c.trim_end_matches('/'))));
                    // Missing candidates before the one used are probes.
                    for c in &cs[..found.unwrap_or(cs.len())] {
                        let p = fsx::join(&base, c.trim_end_matches('/'));
                        if c.ends_with('/') {
                            let reason = vs.reason.clone().unwrap_or_else(|| name.to_string());
                            out.membership(p, vs.dir_glob.as_deref().unwrap_or("**"), reason);
                        } else {
                            out.input(p);
                        }
                    }
                    if let Some(c) = cs.get(found.unwrap_or(0)) {
                        push(c.trim_end_matches('/'));
                    }
                }
                None => {}
            }
        }
        if vs.base.is_some() {
            for i in &mut items {
                i.base = cwd.clone();
            }
        }
        if items.is_empty() && vs.required && !uncertain && !self.cfg_failed {
            let what = vs.field.first().or(vs.flag.first()).map_or(name, |s| s);
            unknown(
                out,
                &self.ctx_dir(),
                UncertaintyKind::ParseIncomplete,
                format!("{label} config without `{what}`"),
            );
        }
        items
    }

    /// Walks key path `path` of `v`; strings are relative to `base`.
    fn walk(&self, v: &ConfigVal, path: &str, base: &Path, vs: &ValueSpec) -> Walk {
        let mut w = Walk::default();
        let segs = parse_path(path).unwrap_or_default();
        let mut scope = Vec::new();
        self.walk_in(
            v,
            &segs,
            0,
            String::new(),
            &mut scope,
            false,
            base,
            vs,
            &mut w,
        );
        w
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_in(
        &self,
        v: &ConfigVal,
        segs: &[Seg],
        i: usize,
        disp: String,
        scope: &mut Vec<ConfigVal>,
        iterated: bool,
        base: &Path,
        vs: &ValueSpec,
        w: &mut Walk,
    ) {
        let join = |k: &str| {
            if disp.is_empty() {
                k.to_string()
            } else {
                format!("{disp}.{k}")
            }
        };
        let Some(seg) = segs.get(i) else {
            return self.leaf(v, disp, scope, iterated, base, vs, w);
        };
        match seg {
            Seg::Key(k) => match get(v, k) {
                Some(x) => {
                    scope.push(v.clone());
                    self.walk_in(&x, segs, i + 1, join(k), scope, iterated, base, vs, w);
                    scope.pop();
                }
                None if i + 1 == segs.len() && iterated => w.missing.push(join(k)),
                None => {}
            },
            Seg::Arr => match v {
                ConfigVal::Arr(a) => {
                    for (n, x) in a.iter().enumerate() {
                        let d = format!("{disp}[{n}]");
                        self.walk_in(x, segs, i + 1, d, scope, true, base, vs, w);
                    }
                }
                other => self.walk_in(other, segs, i + 1, disp, scope, true, base, vs, w),
            },
            Seg::Star(except) => match entries(v) {
                Ok(es) => {
                    let only: Vec<&str> = match &self.spec.config {
                        Some(c) if i == 0 => c
                            .key_flag
                            .iter()
                            .flat_map(|f| self.flag_values(f))
                            .collect(),
                        _ => Vec::new(),
                    };
                    scope.push(v.clone());
                    for (k, x) in es {
                        if except.contains(&k) || (!only.is_empty() && !only.contains(&&*k)) {
                            continue;
                        }
                        self.walk_in(&x, segs, i + 1, join(&k), scope, true, base, vs, w);
                    }
                    scope.pop();
                }
                Err(r) => w.unknown.push((join("*"), r)),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn leaf(
        &self,
        v: &ConfigVal,
        disp: String,
        scope: &[ConfigVal],
        iterated: bool,
        base: &Path,
        vs: &ValueSpec,
        w: &mut Walk,
    ) {
        let mut scope = scope.to_vec();
        let mut v = v.clone();
        if let Some(k) = &vs.or_key
            && matches!(v, ConfigVal::Obj(_))
        {
            match get(&v, k) {
                Some(x) => {
                    scope.push(v);
                    v = x;
                }
                None => {
                    if iterated || vs.required {
                        w.missing.push(format!("{disp}.{k}"));
                    }
                    return;
                }
            }
        }
        if vs.env_refs {
            let mut rs = Vec::new();
            v.unknown_reasons(&mut rs);
            for r in rs {
                for n in env_refs(&r) {
                    w.items.push(Item {
                        text: n,
                        base: base.to_path_buf(),
                        scope: Vec::new(),
                    });
                }
            }
            return;
        }
        let (strs, unk) = str_list(&v);
        for r in unk {
            w.unknown.push((disp.clone(), r));
        }
        for s in strs {
            let mut it = Item {
                text: s,
                base: base.to_path_buf(),
                scope: scope.clone(),
            };
            if let Some(b) = vs.base_field.as_deref().and_then(|f| it.lookup(f)) {
                it.base = fsx::join(base, &b);
            }
            w.items.push(it);
        }
    }

    fn apply(&mut self, r: &RuleRef, out: &mut TaskSpec, read_outputs: Option<&str>) {
        let r = match r {
            RuleRef::Group(g) => {
                let spec = self.spec;
                for r in &spec.groups[g] {
                    self.apply(r, out, read_outputs);
                }
                return;
            }
            RuleRef::Rule(r) => r,
        };
        if !self.all(&r.when) || r.unless.iter().any(|c| self.holds(c)) {
            return;
        }
        let spec = self.spec;
        let ctx = self.ctx_dir();
        let Some(&(role, arg)) = r.roles().first() else {
            return;
        };
        match role {
            "use" => {
                let ro = r.read_outputs.as_deref().or(read_outputs);
                for x in &spec.groups[arg] {
                    self.apply(x, out, ro);
                }
            }
            "tool" => {
                if let Ok((e, n)) = pkg(arg) {
                    out.tool(e, n);
                }
            }
            "uncertain" => {
                if let Some(k) = crate::spec::parse_kind(arg) {
                    let d = r.detail.as_deref().unwrap_or_default();
                    out.uncertain(k, d.replace("{sub}", &self.sub));
                }
            }
            "scan" | "toolchain" => {
                let items = self.value(r.on.as_deref().unwrap_or_default(), out);
                for it in items.iter() {
                    if role == "toolchain" {
                        out.tool(arg, it.text.clone());
                    } else if let Some(s) = scanners::get(arg) {
                        let t = Target {
                            text: &it.text,
                            base: &it.base,
                            ctx_dir: &ctx,
                        };
                        (s.scan)(self.cx, &t, out);
                    }
                }
            }
            _ => {
                let items = self.value(arg, out);
                for it in items.iter() {
                    self.apply_item(role, r, it, &ctx, read_outputs, out);
                }
            }
        }
    }

    fn apply_item(
        &self,
        role: &str,
        r: &Rule,
        it: &Item,
        ctx: &Path,
        read_outputs: Option<&str>,
        out: &mut TaskSpec,
    ) {
        let id = self.spec.id.as_str();
        let mut path = it.path();
        if let Some(j) = &r.join {
            path = path.join(j);
        }
        match role {
            "input" => {
                let role = if r.code {
                    PathRole::Code
                } else {
                    PathRole::Data
                };
                let reason = r.reason.clone().unwrap_or_else(|| format!("{id} input"));
                let glob = r.glob.as_deref().unwrap_or("**");
                add_location(&it.base, &it.text, role, glob, &reason, ctx, out);
            }
            "membership" => {
                let reason = r.reason.clone().unwrap_or_else(|| id.to_string());
                out.membership(path, r.glob.as_deref().unwrap_or("**"), reason);
            }
            "config" => add_code_config(&path, out),
            "env" => out.env.push(it.text.clone()),
            "run" => {
                let parsed = crate::shell::parse(&it.text, &it.base, self.cx.env);
                out.uncertainties.extend(parsed.uncertainties);
                for c in &parsed.commands {
                    registry().recognize(c, self.cx, out);
                }
            }
            "output" => {
                let glob = r.glob.as_deref().map(|g| render(g, it));
                if let Some(reason) = read_outputs {
                    out.membership(path, glob.as_deref().unwrap_or("**"), reason);
                    return;
                }
                if let Some(g) = glob {
                    output_pattern(out, path, &g, &[]);
                    return;
                }
                let dir_mode = r.dir_when.iter().flatten().any(|(k, vals)| {
                    let v = it.lookup(k).or_else(|| {
                        self.flag_values(&format!("--{k}"))
                            .first()
                            .map(|s| s.to_string())
                    });
                    v.is_some_and(|v| vals.contains(&v))
                });
                let has_ext = path.extension().is_some();
                if dir_mode {
                    out.output(if has_ext { dir_of(&path) } else { path });
                } else if r.siblings && has_ext {
                    let name = config::file_name(&path);
                    let stem = name.split('.').next().unwrap_or("");
                    let pattern = format!("{stem}.*");
                    output_pattern(out, dir_of(&path), &pattern, &[]);
                    out.output(path);
                } else {
                    out.output(path);
                }
            }
            _ => {}
        }
    }
}

/// Expands `${key}` / `${key:-default}` with the item's enclosing config
/// objects.
fn render(t: &str, it: &Item) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let inner = &rest[i + 2..i + j];
        let (key, def) = inner.split_once(":-").unwrap_or((inner, ""));
        out.push_str(&it.lookup(key).unwrap_or_else(|| def.to_string()));
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

/// `process.env.NAME` / `process.env["NAME"]` references in a source snippet.
pub fn env_refs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in s.match_indices("process.env") {
        let rest = &s[i + "process.env".len()..];
        let r = rest
            .strip_prefix('.')
            .or_else(|| rest.strip_prefix("[\""))
            .or_else(|| rest.strip_prefix("['"))
            .unwrap_or("");
        let name: String = r
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        let obj = config::parse_yaml("output_db_file_name: dbtx.go\n").unwrap();
        let it = Item {
            text: "gen".into(),
            base: PathBuf::from("/r"),
            scope: vec![obj],
        };
        assert_eq!(
            render("{${output_db_file_name:-db.go},*.sql${suffix}.go}", &it),
            "{dbtx.go,*.sql.go}"
        );
    }

    /// Engine features the fixture snapshots do not reach.
    #[test]
    fn interprets_a_spec() {
        let spec = SpecRecognizer::parse(
            r#"
id = "gen"
programs = ["gen"]
flags = ["--config|-c", "--project|-p", "--out|-o"]
ignore = [""]
[config]
label = "gen"
flag = "--config"
names = ["gen.json"]
key_flag = "--project"
[values]
schema = { default = ["schema.x", "schema/"], dir_glob = "**/*.x", reason = "gen schema" }
target = { field = "*.output", or_key = "target", base_field = "workspace" }
[groups]
write = [{ output = "target", siblings = true, dir_when = { mode = ["split"] } }]
[sub]
build = [{ input = "schema" }, "write", { output = "--out", unless = ["--dry"] }]
check = [{ use = "write", read_outputs = "gen outputs" }]
"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("tt-spec-engine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("gen.json"),
            r#"{"a": {"output": {"workspace": "ws", "target": "api.ts"}},
                "b": {"output": "b/out.ts"},
                "c": {"output": {"target": "c/x.ts", "mode": "split"}}}"#,
        )
        .unwrap();
        let run = |line: &str| {
            let env = std::collections::BTreeMap::new();
            let cmd = crate::shell::parse(line, &dir, &env).commands.remove(0);
            let cx = RecognizeCx {
                repo_root: &dir,
                package_root: &dir,
                env: &env,
                task_name: None,
                scripts: &[],
            };
            let mut out = TaskSpec::default();
            let r = spec.recognize(&cmd, &cx, &mut out);
            out.normalize();
            let rel = |p: &Path| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned();
            let outputs: Vec<String> = out
                .outputs
                .iter()
                .map(|o| format!("{} {}", rel(&o.path), o.pattern.as_deref().unwrap_or("")))
                .collect();
            (r, out, outputs)
        };
        let (r, out, outputs) = run("gen build");
        assert_eq!(r, Recognition::Full);
        // No default candidate exists: both are probes, the directory as a membership.
        assert_eq!(out.inputs, [dir.join("schema.x")]);
        assert_eq!(out.memberships[0].dir, dir.join("schema"));
        assert_eq!(out.memberships[0].pattern, "**/*.x");
        assert_eq!(
            outputs,
            ["b out.*", "b/out.ts ", "c ", "ws api.*", "ws/api.ts "]
        );
        let (_, _, outputs) = run("gen build -p b -o o --dry");
        assert_eq!(outputs, ["b out.*", "b/out.ts "]);
        let (_, out, outputs) = run("gen build -p b -o o");
        assert_eq!(outputs, ["b out.*", "b/out.ts ", "o "]);
        assert!(out.configs.contains(&dir.join("gen.json")));
        let (_, out, outputs) = run("gen check");
        assert!(outputs.is_empty());
        assert!(out.memberships.iter().any(|m| m.reason == "gen outputs"));
        let (r, out, _) = run("gen nope");
        assert_eq!(r, Recognition::Partial);
        assert_eq!(out.uncertainties[0].detail, "gen nope");
        assert_eq!(run("gen").0, Recognition::Ignored);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn env_names() {
        assert_eq!(
            env_refs("process.env.DATABASE_URL! + process.env['X_Y']"),
            ["DATABASE_URL", "X_Y"]
        );
    }
}
