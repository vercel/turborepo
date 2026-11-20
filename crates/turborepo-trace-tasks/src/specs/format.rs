//! The tool spec format (TOML), as deserialized, and its validation. See
//! [`crate::specs`] for the reference.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

/// One tool.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    /// Recognizer id.
    pub id: String,
    /// Program base names.
    pub programs: Vec<String>,
    /// Toolchain packages (`<ecosystem>:<name>`), added for every command (also
    /// ignored ones).
    #[serde(default)]
    pub toolchain: Vec<String>,
    /// Env names the tool always reads.
    #[serde(default)]
    pub env: Vec<String>,
    /// Flags that take a value, `--long|-s` (the first spelling is the
    /// canonical name).
    #[serde(default)]
    pub flags: Vec<String>,
    /// Split the subcommand word at this separator and keep the first part
    /// (`generate:pg`).
    #[serde(default)]
    pub subcommand_split: Option<String>,
    /// Subcommands with no file effect (`""` = no subcommand).
    #[serde(default)]
    pub ignore: Vec<String>,
    /// The tool's config file.
    #[serde(default)]
    pub config: Option<ConfigSpec>,
    /// Named values.
    #[serde(default)]
    pub values: BTreeMap<String, ValueSpec>,
    /// Named rule lists.
    #[serde(default)]
    pub groups: BTreeMap<String, Vec<RuleRef>>,
    /// Rules of every recognized (not ignored, not unknown) command.
    #[serde(default)]
    pub always: Vec<RuleRef>,
    /// Rules per subcommand (`"migrate dev"`, `"up|drop"`).
    #[serde(default)]
    pub sub: BTreeMap<String, Vec<RuleRef>>,
    /// For tools without subcommands: the first variant whose `when` holds.
    #[serde(default)]
    pub variant: Vec<Variant>,
}

/// Config discovery.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSpec {
    /// Prefix of uncertainty details (`drizzle`, `prisma.config`).
    pub label: String,
    /// The flag naming the config file.
    #[serde(default)]
    pub flag: Option<String>,
    /// File names, first match wins.
    #[serde(default)]
    pub names: Vec<String>,
    /// Where `names` are looked up: `cwd` (default), `package` or `up` (cwd up
    /// to the repo).
    #[serde(default)]
    pub search: Search,
    /// A `package.json` key holding the config when no file is found.
    #[serde(default)]
    pub package_key: Option<String>,
    /// Without a config, record every name as a probe (default: the first).
    #[serde(default)]
    pub probe_all: bool,
    /// A missing config is an uncertainty with this detail.
    #[serde(default)]
    pub required: Option<String>,
    /// An unreadable config is an uncertainty with this detail.
    #[serde(default)]
    pub unreadable: Option<String>,
    /// A readable config without any of these keys is an uncertainty.
    #[serde(default)]
    pub require_any: Vec<String>,
    /// A flag selecting keys of a top-level `*` (`orval --project x`).
    #[serde(default)]
    pub key_flag: Option<String>,
}

/// Where config file names are looked up.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Search {
    /// The command's cwd.
    #[default]
    Cwd,
    /// The package root.
    Package,
    /// The cwd and its ancestors up to the repo root.
    Up,
}

/// A named value: a list of strings with the directory each is relative to.
/// Sources in precedence order: `flag`, `field`, `pkg`, `default`; the first
/// that yields anything wins.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSpec {
    /// Flags (every occurrence), relative to the cwd.
    #[serde(default, deserialize_with = "one_or_many")]
    pub flag: Vec<String>,
    /// Key paths into the config (relative to the config's directory).
    #[serde(default, deserialize_with = "one_or_many")]
    pub field: Vec<String>,
    /// A key path into the nearest `package.json` (relative to it).
    #[serde(default)]
    pub pkg: Option<String>,
    /// A default (relative to the cwd): one string, or candidates of which the
    /// first existing wins (a trailing `/` marks a directory). Missing
    /// candidates are probes.
    #[serde(default)]
    pub default: Option<Default>,
    /// `cwd`: resolve every source against the cwd.
    #[serde(default)]
    pub base: Option<String>,
    /// Resolve the default against the directory of this value's first item
    /// (the item itself when it is a directory).
    #[serde(default)]
    pub default_base: Option<String>,
    /// Empty is an uncertainty.
    #[serde(default)]
    pub required: bool,
    /// A missing key in an iterated element (`sql[].schema`) is fine.
    #[serde(default)]
    pub optional: bool,
    /// When the leaf is an object, use this key of it (`{ target: "…" }`).
    #[serde(default)]
    pub or_key: Option<String>,
    /// Resolve against this field of the nearest enclosing object, when
    /// present.
    #[serde(default)]
    pub base_field: Option<String>,
    /// Collect `process.env.X` names from the non-static parts instead of
    /// strings.
    #[serde(default)]
    pub env_refs: bool,
    /// Glob of a directory default candidate's probe membership.
    #[serde(default)]
    pub dir_glob: Option<String>,
    /// Reason of a directory default candidate's probe membership.
    #[serde(default)]
    pub reason: Option<String>,
}

/// A default value.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Default {
    /// Used whether it exists or not.
    One(String),
    /// The first existing.
    Candidates(Vec<String>),
}

/// A conditional rule list.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    /// Conditions (all must hold).
    #[serde(default)]
    pub when: Vec<String>,
    /// Whether the config is loaded.
    #[serde(default = "yes")]
    pub config: bool,
    /// Rules.
    pub rules: Vec<RuleRef>,
}

fn yes() -> bool {
    true
}

/// A rule, or the name of a group.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RuleRef {
    /// A group name.
    Group(String),
    /// A rule.
    Rule(Box<Rule>),
}

/// One rule: exactly one role (`input`, `output`, `membership`, `config`,
/// `env`, `run`, `scan`, `toolchain`, `tool`, `uncertain`, `use`) plus
/// modifiers.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Value → inputs (files, globs, directories).
    pub input: Option<String>,
    /// Value → outputs.
    pub output: Option<String>,
    /// Value → memberships.
    pub membership: Option<String>,
    /// Value → config files (code configs are also entries).
    pub config: Option<String>,
    /// Value → env names.
    pub env: Option<String>,
    /// Value → command lines, recognized with the registry (cwd: the value's
    /// directory).
    pub run: Option<String>,
    /// Scanner name, applied to the value `on`.
    pub scan: Option<String>,
    /// Ecosystem; the value `on` gives package names.
    pub toolchain: Option<String>,
    /// The value of `scan` / `toolchain`.
    pub on: Option<String>,
    /// A toolchain package `<ecosystem>:<name>`.
    pub tool: Option<String>,
    /// An uncertainty kind (`env_dependent`), with `detail` (`{sub}` = the
    /// subcommand).
    pub uncertain: Option<String>,
    /// Detail of `uncertain`.
    pub detail: Option<String>,
    /// A group name.
    #[serde(rename = "use")]
    pub use_: Option<String>,
    /// `input`: the files are JS/TS modules (also entries).
    #[serde(default)]
    pub code: bool,
    /// `input`/`membership`: glob for directories; `output`: glob template
    /// below the path (`${key:-default}` reads the enclosing config
    /// objects).
    pub glob: Option<String>,
    /// Membership reason.
    pub reason: Option<String>,
    /// `output`/`membership`: a path appended to each item.
    pub join: Option<String>,
    /// `output`: a file target also covers `<stem>.*` next to it, and a target
    /// without an extension is a directory.
    #[serde(default)]
    pub siblings: bool,
    /// `output`: `{ key = [values] }`: the target's directory is the output
    /// when `key` (enclosing config objects, else the flag `--key`) is one
    /// of `values`.
    pub dir_when: Option<BTreeMap<String, Vec<String>>>,
    /// `use`: outputs of the group are read (memberships with this reason), not
    /// written.
    pub read_outputs: Option<String>,
    /// Conditions (all must hold): `--flag` (present), `@args` (positionals),
    /// `a|b` (any).
    #[serde(default)]
    pub when: Vec<String>,
    /// Conditions of which none may hold.
    #[serde(default)]
    pub unless: Vec<String>,
}

impl Rule {
    /// The rule's roles, as `(role, argument)`.
    pub fn roles(&self) -> Vec<(&'static str, &str)> {
        [
            ("input", &self.input),
            ("output", &self.output),
            ("membership", &self.membership),
            ("config", &self.config),
            ("env", &self.env),
            ("run", &self.run),
            ("scan", &self.scan),
            ("toolchain", &self.toolchain),
            ("tool", &self.tool),
            ("uncertain", &self.uncertain),
            ("use", &self.use_),
        ]
        .into_iter()
        .filter_map(|(r, v)| v.as_deref().map(|v| (r, v)))
        .collect()
    }
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

/// One segment of a config key path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    /// `name`.
    Key(String),
    /// `[]`: every element of an array (a non-array is one element).
    Arr,
    /// `*` / `*!a,b`: every entry of an object, except the named keys.
    Star(Vec<String>),
}

/// Parses `sql[].gen.*!go.out`.
pub fn parse_path(s: &str) -> Result<Vec<Seg>, String> {
    let mut out = Vec::new();
    for part in s.split('.') {
        if let Some(ex) = part.strip_prefix('*') {
            let ex = match ex.strip_prefix('!') {
                Some(e) => e.split(',').map(str::to_string).collect(),
                None if ex.is_empty() => Vec::new(),
                None => return Err(format!("bad key path `{s}`")),
            };
            out.push(Seg::Star(ex));
            continue;
        }
        let (name, arr) = match part.strip_suffix("[]") {
            Some(n) => (n, true),
            None => (part, false),
        };
        if name.is_empty() && !arr {
            return Err(format!("bad key path `{s}`"));
        }
        if !name.is_empty() {
            out.push(Seg::Key(name.to_string()));
        }
        if arr {
            out.push(Seg::Arr);
        }
    }
    Ok(out)
}

impl ToolSpec {
    /// The canonical spelling of flag `f` (`-c` → `--config`).
    pub fn canonical<'a>(&'a self, f: &'a str) -> &'a str {
        for decl in &self.flags {
            let mut names = decl.split('|');
            let canon = names.next().unwrap_or("");
            if canon == f || names.any(|n| n == f) {
                return canon;
            }
        }
        f
    }

    /// Checks references and syntax. Every spec is validated when the registry
    /// is built (and by a unit test), so a broken spec fails the build's
    /// tests.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || self.programs.is_empty() {
            return Err("`id` and `programs` are required".into());
        }
        if !self.sub.is_empty() && !self.variant.is_empty() {
            return Err("use either `sub` or `variant`".into());
        }
        for t in &self.toolchain {
            pkg(t)?;
        }
        let mut words = std::collections::BTreeSet::new();
        for k in self.sub.keys() {
            for w in k.split('|') {
                if !words.insert(w) {
                    return Err(format!("subcommand `{w}` listed twice"));
                }
            }
        }
        for (name, v) in &self.values {
            for f in &v.flag {
                self.check_flag(f)?;
            }
            for f in v.field.iter().chain(&v.pkg) {
                parse_path(f)?;
            }
            if let Some(b) = &v.default_base
                && (b == name || !self.values.contains_key(b))
            {
                return Err(format!("value `{name}`: bad default_base `{b}`"));
            }
            if v.base.as_deref().is_some_and(|b| b != "cwd") {
                return Err(format!("value `{name}`: base must be `cwd`"));
            }
            if !v.field.is_empty() && self.config.is_none() {
                return Err(format!("value `{name}` reads a config, but there is none"));
            }
        }
        if let Some(c) = &self.config
            && let Some(f) = c.flag.iter().chain(&c.key_flag).next()
        {
            self.check_flag(f)?;
        }
        let lists = self
            .groups
            .values()
            .chain(self.sub.values())
            .chain(self.variant.iter().map(|v| &v.rules))
            .chain([&self.always]);
        for l in lists {
            for r in l {
                self.check_rule(r)?;
            }
        }
        for v in &self.variant {
            self.check_conds(&v.when)?;
        }
        Ok(())
    }

    fn check_flag(&self, f: &str) -> Result<(), String> {
        let c = self.canonical(f);
        if self.flags.iter().any(|d| d.split('|').next() == Some(c)) {
            Ok(())
        } else {
            Err(format!("`{f}` is not in `flags`"))
        }
    }

    fn check_value(&self, v: &str) -> Result<(), String> {
        if v == "@args" || self.values.contains_key(v) {
            Ok(())
        } else if v.starts_with('-') {
            self.check_flag(v)
        } else {
            Err(format!("unknown value `{v}`"))
        }
    }

    fn check_conds(&self, cs: &[String]) -> Result<(), String> {
        for c in cs.iter().flat_map(|c| c.split('|')) {
            if c != "@args" && !c.starts_with('-') {
                return Err(format!("bad condition `{c}`"));
            }
        }
        Ok(())
    }

    fn check_rule(&self, r: &RuleRef) -> Result<(), String> {
        let r = match r {
            RuleRef::Group(g) => {
                return if self.groups.contains_key(g) {
                    Ok(())
                } else {
                    Err(format!("unknown group `{g}`"))
                };
            }
            RuleRef::Rule(r) => r,
        };
        let roles = r.roles();
        let [(role, arg)] = roles.as_slice() else {
            return Err(format!("a rule needs exactly one role: {r:?}"));
        };
        self.check_conds(&r.when)?;
        self.check_conds(&r.unless)?;
        match *role {
            "scan" | "toolchain" => {
                if *role == "scan" && crate::scanners::get(arg).is_none() {
                    return Err(format!("unknown scanner `{arg}`"));
                }
                self.check_value(r.on.as_deref().ok_or(format!("`{role}` needs `on`"))?)?;
            }
            "tool" => {
                pkg(arg)?;
            }
            "uncertain" => {
                crate::spec::parse_kind(arg).ok_or(format!("unknown uncertainty `{arg}`"))?;
                r.detail.as_ref().ok_or("`uncertain` needs `detail`")?;
            }
            "use" => {
                if !self.groups.contains_key(*arg) {
                    return Err(format!("unknown group `{arg}`"));
                }
            }
            _ => self.check_value(arg)?,
        }
        if r.read_outputs.is_some() && *role != "use" {
            return Err("`read_outputs` goes with `use`".into());
        }
        Ok(())
    }
}

/// Splits `<ecosystem>:<name>`.
pub fn pkg(s: &str) -> Result<(&str, &str), String> {
    match s.split_once(':') {
        Some((e, n)) if !e.is_empty() && !n.is_empty() => Ok((e, n)),
        _ => Err(format!("`{s}` is not `<ecosystem>:<name>`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            parse_path("sql[].gen.*!go.out").unwrap(),
            [
                Seg::Key("sql".into()),
                Seg::Arr,
                Seg::Key("gen".into()),
                Seg::Star(vec!["go".into()]),
                Seg::Key("out".into())
            ]
        );
        assert!(parse_path("a..b").is_err());
    }
}
