//! The per-file relation of changed exports: what [`Module::taint_members`]
//! answers, worked out once per file and written as one string ([`fact`],
//! [`TAG`]).
//!
//! `affected` walks from a changed export through the files that import it. At
//! every importer it asks two things: which names of the import does the file
//! use (the module shape, `crate::extract::exports`), and which of the file's
//! own exports, or its load-time code, read them ([`super`]). Both are
//! functions of the importer's text alone, so they are computed once per file,
//! the first time a walk of the query gets to it, and every later question
//! about the file is answered from the string. The relation is not traced
//! with the file: it takes a lex and a parse of its own, which every build
//! would pay for every file, and a query asks about the files between a test
//! and a change.
//!
//! # What is stored
//!
//! - **The shape**: the names each import, `require` and re-export uses, keyed
//!   by the byte offset of its keyword (the start of the graph edge's span; the
//!   specifier is the edge's), and the export names of a file whose top level
//!   is free of side effects.
//! - **One row per import binding** ([`Seed`]): the file's exports whose
//!   definition reads the binding as a value, directly or through other
//!   top-level declarations; whether any statement reads it at all; and the
//!   load-time statements that read it ([`Load`]). A load-time statement either
//!   always counts, or counts unless every question of a short list holds
//!   ([`Atom`]): the statement is a chain of calls that may only build a value,
//!   and whether it does depends on other files (`super`, "Calls that only
//!   build a value"). The questions are recorded with the row and asked when
//!   the walk gets there, because their answers are facts of other files.
//! - For a binding that some statement only reads members of (`Resource.Team`,
//!   `ns.a`): one row for the statements that use the binding as a whole, and
//!   one per member name. A change of some members of an enum or object table
//!   taints the first row and the rows of those members ([`super`], "Members of
//!   enums and object tables").
//!
//! # Why a row per binding is the same as the joint computation
//!
//! [`Module::taint_members`] taints a set of import bindings at once and closes
//! over the statements. A statement is tainted when *some* reference of it
//! reads a tainted binding, so the closure of a set is the union of the
//! closures of its members, and so are the tainted exports. A load-time
//! statement counts unless its chain only builds a value and no root of the
//! chain is tainted; a statement whose chain is rooted in a binding reads
//! that binding as a whole here (never member by member), so the roots a row
//! taints are the ones any set containing the row's binding taints through that
//! statement. The unit tests compare the two computations on every subset of
//! every import of their modules.
//!
//! # Format
//!
//! One string, a pure function of the file's name and text: [`TAG`], a version
//! and flags, then lines of tab-separated fields (lists inside a field are
//! separated by U+001F, U+001E and U+001D). A name with a control character
//! cannot be written: such a file has no rows, or no shape (its edges use
//! everything). See [`encode`] for the lines. The walk reads a fact in place:
//! [`Fact::at`] finds what one import uses, [`Fact::taint`] the rows of the
//! bindings that are asked about; nothing is decoded that is not asked for.

use std::{cell::RefCell, collections::BTreeMap, path::Path};

use super::{Asked, Callee, Cond, Export, FUEL, Kind, Module, Oracle, Prop, Step, Taint};
use crate::{
    Dialect,
    extract::exports::{self as ex, Names, scan_shape},
};

/// The tag of the fact string.
pub const TAG: &str = "exports:";

/// The version of the format after the tag. Bump on any change of [`encode`] or
/// of what [`Module::rows`] computes.
const VERSION: &str = "1";

/// Files larger than this have a shape and no rows: a bundle that is checked in
/// is not worth a full parse, and nobody narrows a change through it.
const MAX_PARSED: usize = 512 * 1024;

/// A member-level row is kept for at most this many member names of one
/// binding; a binding with more is read as a whole.
const MAX_MEMBERS: usize = 64;

const LIST: char = '\u{1f}';
const SUB: char = '\u{1e}';
const SUBSUB: char = '\u{1d}';

/// A question about another file or a package, recorded while a load-time chain
/// is judged ([`Oracle`]): the chain only builds a value when every question of
/// its list holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Atom {
    /// [`Oracle::builds`].
    Builds {
        /// The specifier of the import.
        spec: String,
        /// The imported name (`default`, `*`).
        name: String,
        /// The chain.
        steps: Vec<Step>,
        /// Every argument is a literal.
        literal_args: bool,
    },
    /// [`Oracle::builds_with`].
    BuildsWith {
        /// The specifier of the import.
        spec: String,
        /// The imported name.
        name: String,
        /// The chain.
        steps: Vec<Step>,
        /// Every argument is a literal.
        literal_args: bool,
        /// The properties of the object literal the last call is handed.
        arg: Vec<(String, OwnedProp)>,
    },
    /// [`Oracle::continues`].
    Continues {
        /// The specifier of the import.
        spec: String,
        /// The imported name.
        name: String,
        /// The chain that made the value.
        made: Vec<Step>,
        /// The chain on that value.
        steps: Vec<Step>,
    },
    /// [`Oracle::prim`].
    Prim {
        /// The specifier of the import.
        spec: String,
        /// The imported name.
        name: String,
    },
}

/// [`Prop`] with owned strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnedProp {
    /// A function.
    Function,
    /// A chain on an import: specifier, imported name, steps.
    Chain(String, String, Vec<Step>),
    /// A spread of a parameter.
    Spread,
    /// Anything else.
    Other,
}

impl Atom {
    /// Asks `oracle` the recorded question.
    pub fn holds(&self, oracle: &dyn Oracle) -> bool {
        match self {
            Atom::Builds {
                spec,
                name,
                steps,
                literal_args,
            } => oracle.builds(Callee { spec, name }, steps, *literal_args),
            Atom::BuildsWith {
                spec,
                name,
                steps,
                literal_args,
                arg,
            } => {
                let props: Vec<(String, Prop<'_>)> = arg
                    .iter()
                    .map(|(key, p)| {
                        let p = match p {
                            OwnedProp::Function => Prop::Function,
                            OwnedProp::Spread => Prop::Spread,
                            OwnedProp::Other => Prop::Other,
                            OwnedProp::Chain(spec, name, steps) => {
                                Prop::Chain(Callee { spec, name }, steps.clone())
                            }
                        };
                        (key.clone(), p)
                    })
                    .collect();
                oracle.builds_with(Callee { spec, name }, steps, *literal_args, &props)
            }
            Atom::Continues {
                spec,
                name,
                made,
                steps,
            } => oracle.continues(Callee { spec, name }, made, steps),
            Atom::Prim { spec, name } => oracle.prim(Callee { spec, name }),
        }
    }
}

/// An [`Oracle`] that says yes to everything and keeps the questions: a
/// condition that fails with it fails with every oracle, and one that holds
/// with it holds exactly when the recorded questions do (a condition is a
/// conjunction of local facts and questions).
#[derive(Default)]
struct Recorder(RefCell<Vec<Atom>>);

impl Oracle for Recorder {
    fn builds(&self, c: Callee<'_>, steps: &[Step], literal_args: bool) -> bool {
        self.0.borrow_mut().push(Atom::Builds {
            spec: c.spec.to_owned(),
            name: c.name.to_owned(),
            steps: steps.to_vec(),
            literal_args,
        });
        true
    }

    fn continues(&self, c: Callee<'_>, made: &[Step], steps: &[Step]) -> bool {
        self.0.borrow_mut().push(Atom::Continues {
            spec: c.spec.to_owned(),
            name: c.name.to_owned(),
            made: made.to_vec(),
            steps: steps.to_vec(),
        });
        true
    }

    fn builds_with(
        &self,
        c: Callee<'_>,
        steps: &[Step],
        literal_args: bool,
        arg: &[(String, Prop<'_>)],
    ) -> bool {
        self.0.borrow_mut().push(Atom::BuildsWith {
            spec: c.spec.to_owned(),
            name: c.name.to_owned(),
            steps: steps.to_vec(),
            literal_args,
            arg: arg
                .iter()
                .map(|(key, p)| {
                    let p = match p {
                        Prop::Function => OwnedProp::Function,
                        Prop::Spread => OwnedProp::Spread,
                        Prop::Other => OwnedProp::Other,
                        Prop::Chain(c, steps) => {
                            OwnedProp::Chain(c.spec.to_owned(), c.name.to_owned(), steps.clone())
                        }
                    };
                    (key.clone(), p)
                })
                .collect(),
        });
        true
    }

    fn prim(&self, c: Callee<'_>) -> bool {
        self.0.borrow_mut().push(Atom::Prim {
            spec: c.spec.to_owned(),
            name: c.name.to_owned(),
        });
        true
    }
}

/// A load-time statement that reads what a row stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    /// It always counts: the reason (an index into [`Rows::whys`]).
    Always(u32),
    /// It counts unless every question of a list holds: the reason, and the
    /// list (an index into [`Rows::conds`]).
    Unless(u32, u32),
}

/// What a file derives from one tainted import binding, or from one way of
/// reading it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Some statement reads it as a value.
    pub any: bool,
    /// The exports whose definition reads it (indexes into [`Rows::exports`]),
    /// ascending.
    pub exports: Vec<u32>,
    /// The load-time statements that read it, in source order, up to the first
    /// that always counts.
    pub loads: Vec<Load>,
}

/// How a [`Seed`]'s name is imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeedKind {
    /// `import {name as local}`.
    Name,
    /// `import local from`.
    Default,
    /// `import * as local from`.
    Namespace,
}

/// The rows of one imported name of one import declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    /// The byte offset of the `import` keyword.
    pub start: u32,
    /// How the name is imported.
    pub kind: SeedKind,
    /// The imported name (`default` for a default import, empty for a namespace
    /// import).
    pub name: String,
    /// Some binding of the name is not type-only: linking the file reads the
    /// name.
    pub value: bool,
    /// Every use of the binding.
    pub all: Row,
    /// When some statement only reads members of the binding: the statements
    /// that use it as a whole, and the statements that read each member.
    /// Their union is `all`.
    pub parts: Option<(Row, Vec<(String, Row)>)>,
}

/// The rows of a module ([`Module::rows`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    /// The export names rows refer to.
    pub exports: Vec<String>,
    /// The reasons load-time statements count.
    pub whys: Vec<String>,
    /// The question lists of conditional load-time statements.
    pub conds: Vec<Vec<Atom>>,
    /// One per imported name of each import declaration, by start, then name.
    pub seeds: Vec<Seed>,
}

/// Which references to the seed's bindings a row follows.
enum Mode<'a> {
    All,
    Whole,
    Member(&'a str),
}

impl Module {
    /// The rows of the module: [`Module::taint_members`] for each import
    /// binding on its own. See the module documentation.
    pub fn rows(&self) -> Rows {
        let mut out = Rows {
            exports: self.exports.keys().cloned().collect(),
            ..Rows::default()
        };
        // Per statement with a conditional chain: its roots, and the questions on which
        // it only builds a value (`None`: it never does).
        let mut roots: Vec<Vec<u32>> = vec![Vec::new(); self.stmts.len()];
        let mut reduced: Vec<Option<Vec<Atom>>> = vec![None; self.stmts.len()];
        for (i, s) in self.stmts.iter().enumerate() {
            let Some(e) = s.evaluated.as_ref().filter(|e| !e.unless.is_empty()) else {
                continue;
            };
            roots[i] = e
                .unless
                .iter()
                .map(|c| match c {
                    Cond::Builds { root, .. } => *root,
                    Cond::Prim { binding } => *binding,
                })
                .collect();
            let rec = Recorder::default();
            if e.unless.iter().all(|c| self.holds(c, None, &rec, FUEL)) {
                reduced[i] = Some(rec.0.into_inner());
            }
        }
        // The bindings of each imported name of each import declaration.
        let mut groups: BTreeMap<(u32, SeedKind, String), (Vec<u32>, bool)> = BTreeMap::new();
        for (b, binding) in self.bindings.iter().enumerate() {
            let Some(i) = &binding.import else { continue };
            let (kind, name) = match &i.imported {
                super::Imported::Name(n) => (SeedKind::Name, n.clone()),
                super::Imported::Default => (SeedKind::Default, "default".to_owned()),
                super::Imported::Namespace => (SeedKind::Namespace, String::new()),
            };
            let g = groups.entry((i.start, kind, name)).or_default();
            g.0.push(b as u32);
            g.1 |= !i.type_only;
        }
        let mut cond_index: BTreeMap<usize, u32> = BTreeMap::new();
        for ((start, kind, name), (bindings, value)) in groups {
            let all = self.row(
                &bindings,
                &Mode::All,
                &roots,
                &reduced,
                &mut cond_index,
                &mut out,
            );
            // Member names that some statement reads without using the binding otherwise.
            let mut members: Vec<&str> = Vec::new();
            for (i, s) in self.stmts.iter().enumerate() {
                for r in s
                    .refs
                    .iter()
                    .filter(|r| r.value && bindings.contains(&r.binding))
                {
                    if roots[i].contains(&r.binding) {
                        continue;
                    }
                    for m in r.members.iter().flatten() {
                        if !members.contains(&m.as_str()) {
                            members.push(m);
                        }
                    }
                }
            }
            members.sort_unstable();
            let parts = (!members.is_empty() && members.len() <= MAX_MEMBERS).then(|| {
                let whole = self.row(
                    &bindings,
                    &Mode::Whole,
                    &roots,
                    &reduced,
                    &mut cond_index,
                    &mut out,
                );
                let rows = members
                    .iter()
                    .map(|m| {
                        let row = self.row(
                            &bindings,
                            &Mode::Member(m),
                            &roots,
                            &reduced,
                            &mut cond_index,
                            &mut out,
                        );
                        ((*m).to_owned(), row)
                    })
                    .collect();
                (whole, rows)
            });
            out.seeds.push(Seed {
                start,
                kind,
                name,
                value,
                all,
                parts,
            });
        }
        out
    }

    /// What the module derives from the import bindings `seed`, following the
    /// references to them that `mode` selects.
    fn row(
        &self,
        seed: &[u32],
        mode: &Mode<'_>,
        roots: &[Vec<u32>],
        reduced: &[Option<Vec<Atom>>],
        cond_index: &mut BTreeMap<usize, u32>,
        out: &mut Rows,
    ) -> Row {
        let mut tainted = vec![false; self.bindings.len()];
        for &b in seed {
            tainted[b as usize] = true;
        }
        // Whether statement `i` reads the seed through reference `r`. A statement whose
        // conditional chain is rooted in the binding uses it as a whole.
        let direct = |i: usize, r: &super::Ref| {
            let whole = r.members.is_none() || roots[i].contains(&r.binding);
            match mode {
                Mode::All => true,
                Mode::Whole => whole,
                Mode::Member(m) => {
                    !whole
                        && r.members
                            .as_ref()
                            .is_some_and(|ms| ms.iter().any(|x| x == m))
                }
            }
        };
        let mut stmt_tainted = vec![false; self.stmts.len()];
        let mut changed = true;
        while changed {
            changed = false;
            for (i, s) in self.stmts.iter().enumerate() {
                if stmt_tainted[i]
                    || !s.refs.iter().any(|r| {
                        r.value
                            && tainted[r.binding as usize]
                            && (!seed.contains(&r.binding) || direct(i, r))
                    })
                {
                    continue;
                }
                stmt_tainted[i] = true;
                changed = true;
                for &b in &s.declares {
                    if self.bindings[b as usize].import.is_none() {
                        tainted[b as usize] = true;
                    }
                }
            }
        }
        fn why(text: &str, out: &mut Rows) -> u32 {
            match out.whys.iter().position(|w| w == text) {
                Some(at) => at as u32,
                None => {
                    out.whys.push(text.to_owned());
                    out.whys.len() as u32 - 1
                }
            }
        }
        let mut loads: Vec<Load> = Vec::new();
        'stmts: for (i, s) in self.stmts.iter().enumerate() {
            if !stmt_tainted[i] {
                continue;
            }
            // A class that extends a tainted value, other than a class or a plain function
            // of this module (those stay constructors whatever they read).
            if s.extends.iter().any(|&b| {
                tainted[b as usize]
                    && !matches!(self.bindings[b as usize].kind, Kind::Class | Kind::Function)
            }) {
                loads.push(Load::Always(why(
                    "a class that extends a changed value",
                    out,
                )));
                break 'stmts;
            }
            let Some(e) = &s.evaluated else { continue };
            let always = e.unless.is_empty() || roots[i].iter().any(|&r| tainted[r as usize]);
            match (&reduced[i], always) {
                (Some(atoms), false) => {
                    let at = *cond_index.entry(i).or_insert_with(|| {
                        out.conds.push(atoms.clone());
                        out.conds.len() as u32 - 1
                    });
                    loads.push(Load::Unless(why(&e.why, out), at));
                }
                _ => {
                    loads.push(Load::Always(why(&e.why, out)));
                    break 'stmts;
                }
            }
        }
        let exports = self
            .exports
            .values()
            .enumerate()
            .filter(|(_, def)| match def {
                Export::Binding(b) => tainted[*b as usize],
                Export::Stmt(i) => stmt_tainted[*i as usize],
                Export::From(_) => false,
            })
            .map(|(i, _)| i as u32)
            .collect();
        Row {
            any: stmt_tainted.contains(&true),
            exports,
            loads,
        }
    }
}

/// Whether `s` can be written as a field.
fn plain(s: &str) -> bool {
    !s.chars().any(|c| c < ' ')
}

fn steps_text(steps: &[Step], out: &mut String) {
    for s in steps {
        match s {
            Step::Call => out.push_str("()"),
            Step::Member(m) => {
                out.push('.');
                out.push_str(m);
            }
        }
    }
}

fn parse_steps(s: &str) -> Option<Vec<Step>> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("()") {
            out.push(Step::Call);
            rest = r;
        } else {
            let r = rest.strip_prefix('.')?;
            let end = r.find(['.', '(']).unwrap_or(r.len());
            if end == 0 {
                return None;
            }
            out.push(Step::Member(r[..end].to_owned()));
            rest = &r[end..];
        }
    }
    Some(out)
}

/// Whether a member name survives [`steps_text`].
fn plain_steps(steps: &[Step]) -> bool {
    steps.iter().all(|s| match s {
        Step::Call => true,
        Step::Member(m) => plain(m) && !m.is_empty() && !m.contains(['.', '(']),
    })
}

fn atom_text(a: &Atom, out: &mut String) -> bool {
    let head = |kind: char, spec: &str, name: &str, out: &mut String| {
        out.push(kind);
        out.push(LIST);
        out.push_str(spec);
        out.push(LIST);
        out.push_str(name);
        out.push(LIST);
        plain(spec) && plain(name)
    };
    match a {
        Atom::Builds {
            spec,
            name,
            steps,
            literal_args,
        } => {
            let ok = head('b', spec, name, out) && plain_steps(steps);
            steps_text(steps, out);
            out.push(LIST);
            out.push(if *literal_args { '1' } else { '0' });
            ok
        }
        Atom::BuildsWith {
            spec,
            name,
            steps,
            literal_args,
            arg,
        } => {
            let mut ok = head('w', spec, name, out) && plain_steps(steps);
            steps_text(steps, out);
            out.push(LIST);
            out.push(if *literal_args { '1' } else { '0' });
            out.push(LIST);
            for (i, (key, p)) in arg.iter().enumerate() {
                if i > 0 {
                    out.push(SUB);
                }
                ok &= plain(key);
                out.push_str(key);
                out.push(SUBSUB);
                match p {
                    OwnedProp::Function => out.push('f'),
                    OwnedProp::Spread => out.push('s'),
                    OwnedProp::Other => out.push('o'),
                    OwnedProp::Chain(spec, name, steps) => {
                        ok &= plain(spec) && plain(name) && plain_steps(steps);
                        out.push('c');
                        out.push(SUBSUB);
                        out.push_str(spec);
                        out.push(SUBSUB);
                        out.push_str(name);
                        out.push(SUBSUB);
                        steps_text(steps, out);
                    }
                }
            }
            ok
        }
        Atom::Continues {
            spec,
            name,
            made,
            steps,
        } => {
            let ok = head('c', spec, name, out) && plain_steps(made) && plain_steps(steps);
            steps_text(made, out);
            out.push(LIST);
            steps_text(steps, out);
            ok
        }
        Atom::Prim { spec, name } => {
            let ok = head('p', spec, name, out);
            out.pop();
            ok
        }
    }
}

fn parse_atom(s: &str) -> Option<Atom> {
    let mut f = s.split(LIST);
    let kind = f.next()?;
    let spec = f.next()?.to_owned();
    let name = f.next()?.to_owned();
    Some(match kind {
        "b" => Atom::Builds {
            spec,
            name,
            steps: parse_steps(f.next()?)?,
            literal_args: f.next()? == "1",
        },
        "w" => {
            let steps = parse_steps(f.next()?)?;
            let literal_args = f.next()? == "1";
            let props = f.next()?;
            let mut arg = Vec::new();
            for p in props.split(SUB).filter(|p| !p.is_empty()) {
                let mut q = p.split(SUBSUB);
                let key = q.next()?.to_owned();
                let prop = match q.next()? {
                    "f" => OwnedProp::Function,
                    "s" => OwnedProp::Spread,
                    "o" => OwnedProp::Other,
                    "c" => OwnedProp::Chain(
                        q.next()?.to_owned(),
                        q.next()?.to_owned(),
                        parse_steps(q.next()?)?,
                    ),
                    _ => return None,
                };
                arg.push((key, prop));
            }
            Atom::BuildsWith {
                spec,
                name,
                steps,
                literal_args,
                arg,
            }
        }
        "c" => Atom::Continues {
            spec,
            name,
            made: parse_steps(f.next()?)?,
            steps: parse_steps(f.next()?)?,
        },
        "p" => Atom::Prim { spec, name },
        _ => return None,
    })
}

fn row_text(r: &Row, out: &mut String) {
    out.push(if r.any { '+' } else { '~' });
    for (i, e) in r.exports.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(&e.to_string());
    }
    if r.loads.is_empty() {
        return;
    }
    out.push(';');
    for (i, l) in r.loads.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        match l {
            Load::Always(why) => {
                out.push('a');
                out.push_str(&why.to_string());
            }
            Load::Unless(why, cond) => {
                out.push('u');
                out.push_str(&why.to_string());
                out.push('-');
                out.push_str(&cond.to_string());
            }
        }
    }
}

/// Reads a row; the empty string is the row of a binding nothing reads.
fn parse_row(s: &str) -> Option<Row> {
    let mut row = Row::default();
    let Some((flag, rest)) = s.split_at_checked(1) else {
        return Some(row);
    };
    row.any = match flag {
        "+" => true,
        "~" => false,
        _ => return None,
    };
    let (exports, loads) = rest.split_once(';').unwrap_or((rest, ""));
    for e in exports.split('.').filter(|e| !e.is_empty()) {
        row.exports.push(e.parse().ok()?);
    }
    for l in loads.split('.').filter(|l| !l.is_empty()) {
        row.loads.push(match l.split_at_checked(1)? {
            ("a", why) => Load::Always(why.parse().ok()?),
            ("u", rest) => {
                let (why, cond) = rest.split_once('-')?;
                Load::Unless(why.parse().ok()?, cond.parse().ok()?)
            }
            _ => return None,
        });
    }
    Some(row)
}

/// Whether a row is the one of a binding nothing reads (written as nothing).
fn blank(r: &Row) -> bool {
    !r.any && r.exports.is_empty() && r.loads.is_empty()
}

/// Writes the fact of a file from its shape and rows (`Err`: why it has none).
///
/// ```text
/// exports:1<TAB>[m][t][u][a]    an ES module with facts; an export hands out a function
///                                that reads `this`; understood; every edge uses everything
/// P<TAB>name...                  the exports of a file with a side-effect-free top level
/// U<TAB>start<TAB>a              an import or require that uses everything
/// U<TAB>start<TAB>n<TAB>name...  ... that uses these names
/// R<TAB>start<TAB>s              export * from
/// R<TAB>start<TAB>n<TAB>ns       export * as ns from
/// R<TAB>start<TAB>p<TAB>a<US>b...  export {a as b} from
/// N<TAB>reason                   no rows: why
/// E<TAB>name...                  the exports the rows refer to
/// Y<TAB>reason...                the reasons load-time statements count
/// C<TAB>question...              one line per question list
/// B<TAB>start<TAB>seed...        the bindings of one import declaration
/// ```
///
/// A seed is `[n|d|s][v|t]name` (named, default or namespace import; some
/// binding is a value, or all are type-only), then, separated by U+001F, its
/// row, and for a binding that is read member by member the row of its uses as
/// a whole and `member`, `row` pairs. A row is `+` or `~` (some statement reads
/// the binding, or none does), the exports as indexes separated by `.`, and
/// after `;` the loads as `a<reason>` or `u<reason>-<question list>`; the row
/// of a binding that nothing reads and no export hands on is left out.
///
/// The `U` line of an import declaration is left out when it says what the `B`
/// line says: the names it uses are the names of the declaration's named and
/// default bindings.
fn encode(shape: &ex::Shape<'_>, rows: Result<&Rows, &str>, flags: Flags) -> String {
    let mut out = String::from(TAG);
    out.push_str(VERSION);
    out.push('\t');
    // These two say something about the file whatever its shape is.
    if flags.es_module {
        out.push('m');
    }
    if flags.reads_this {
        out.push('t');
    }
    let names_ok = shape.exports.iter().all(|n| plain(n))
        && shape.uses.iter().all(|u| match &u.names {
            Names::All => true,
            Names::Some(v) => v.iter().all(|n| plain(n)),
        })
        && shape.reexports.iter().all(|r| match &r.kind {
            ex::ReExportKind::Star => true,
            ex::ReExportKind::Namespace(n) => plain(n),
            ex::ReExportKind::Named(p) => p.iter().all(|(a, b)| plain(a) && plain(b)),
        });
    if !shape.understood || !names_ok {
        return out;
    }
    out.push('u');
    if shape.uses_all {
        out.push('a');
    }
    if shape.side_effect_free && shape.module_syntax && !shape.exports_open {
        out.push_str("\nP");
        for n in &shape.exports {
            out.push('\t');
            out.push_str(n);
        }
    }
    // Whether the seeds can be written: every string is plain.
    let seeds: &[Seed] = match rows {
        Ok(rows)
            if rows.exports.iter().all(|n| plain(n))
                && rows.whys.iter().all(|w| plain(w))
                && rows.seeds.iter().all(|s| {
                    plain(&s.name)
                        && !s.name.contains(LIST)
                        && s.parts.iter().all(|(_, m)| m.iter().all(|(n, _)| plain(n)))
                }) =>
        {
            &rows.seeds
        }
        _ => &[],
    };
    // The names of the named and default bindings of the declaration at `start`,
    // sorted; `None` when it has a namespace binding (what that uses is the
    // shape's to say).
    let bound = |start: u32| -> Option<Vec<&str>> {
        let mut names = Vec::new();
        for s in seeds.iter().filter(|s| s.start == start) {
            if s.kind == SeedKind::Namespace {
                return None;
            }
            names.push(s.name.as_str());
        }
        names.sort_unstable();
        (!names.is_empty()).then_some(names)
    };
    for u in &shape.uses {
        if let Names::Some(v) = &u.names
            && bound(u.start).is_some_and(|b| b == *v)
        {
            continue;
        }
        out.push_str("\nU\t");
        out.push_str(&u.start.to_string());
        match &u.names {
            Names::All => out.push_str("\ta"),
            Names::Some(v) => {
                out.push_str("\tn");
                for n in v {
                    out.push('\t');
                    out.push_str(n);
                }
            }
        }
    }
    for r in &shape.reexports {
        out.push_str("\nR\t");
        out.push_str(&r.start.to_string());
        match &r.kind {
            ex::ReExportKind::Star => out.push_str("\ts"),
            ex::ReExportKind::Namespace(n) => {
                out.push_str("\tn\t");
                out.push_str(n);
            }
            ex::ReExportKind::Named(p) => {
                out.push_str("\tp");
                for (a, b) in p {
                    out.push('\t');
                    out.push_str(a);
                    out.push(LIST);
                    out.push_str(b);
                }
            }
        }
    }
    let rows = match rows {
        Ok(rows) => rows,
        Err(why) => {
            out.push_str("\nN\t");
            out.push_str(why);
            return out;
        }
    };
    if rows.seeds.is_empty() {
        return out;
    }
    if seeds.is_empty() {
        out.push_str("\nN\ta name with a control character");
        return out;
    }
    let mut ok = true;
    // The exports are listed only when a row refers to one.
    if rows.seeds.iter().any(|s| {
        !s.all.exports.is_empty()
            || s.parts.as_ref().is_some_and(|(w, m)| {
                !w.exports.is_empty() || m.iter().any(|x| !x.1.exports.is_empty())
            })
    }) {
        out.push_str("\nE");
        for n in &rows.exports {
            out.push('\t');
            out.push_str(n);
        }
    }
    if !rows.whys.is_empty() {
        out.push_str("\nY");
        for w in &rows.whys {
            out.push('\t');
            out.push_str(w);
        }
    }
    for c in &rows.conds {
        out.push_str("\nC");
        for a in c {
            out.push('\t');
            ok &= atom_text(a, &mut out);
        }
    }
    let mut last = None;
    for s in &rows.seeds {
        if last != Some(s.start) {
            out.push_str("\nB\t");
            out.push_str(&s.start.to_string());
            last = Some(s.start);
        }
        out.push('\t');
        out.push(match s.kind {
            SeedKind::Name => 'n',
            SeedKind::Default => 'd',
            SeedKind::Namespace => 's',
        });
        out.push(if s.value { 'v' } else { 't' });
        out.push_str(&s.name);
        if s.parts.is_none() && blank(&s.all) {
            continue;
        }
        out.push(LIST);
        row_text(&s.all, &mut out);
        if let Some((whole, members)) = &s.parts {
            out.push(LIST);
            row_text(whole, &mut out);
            for (m, row) in members {
                ok &= !m.contains(LIST);
                out.push(LIST);
                out.push_str(m);
                out.push(LIST);
                row_text(row, &mut out);
            }
        }
    }
    if !ok {
        // A question with a control character: the shape's lines for the declarations
        // were left out on the word of seeds that are not written after all, so the
        // file has no shape either.
        let mut out = String::from(TAG);
        out.push_str(VERSION);
        out.push('\t');
        if flags.reads_this {
            out.push('t');
        }
        return out;
    }
    out
}

/// What a fact says about a file beyond its shape and rows.
#[derive(Clone, Copy, Debug, Default)]
struct Flags {
    /// The file is an ES module with facts ([`Module::read`] succeeded):
    /// reading one of its exports runs no code of it (no getter that
    /// someone wrote).
    es_module: bool,
    /// An export of the file hands out a function that reads `this`, or the
    /// file was not read and has the word.
    reads_this: bool,
}

/// Whether `src` has the word `this` in it: without it, the fact of the file
/// does not say that it reads `this` ([`Fact::reads_this`]).
pub fn has_this(src: &[u8]) -> bool {
    memchr::memmem::find(src, b"this").is_some()
}

/// The fact of the file `path` with contents `src`: its shape, and for an ES
/// module its rows. `None` for a file with nothing to say (a file that is not
/// valid UTF-8).
pub fn fact(path: &Path, src: &[u8]) -> Option<String> {
    let shape = scan_shape(src, Dialect::from_path(path));
    let has_this = || has_this(src);
    let mut flags = Flags::default();
    let rows = if shape.understood && shape.module_syntax {
        let text = std::str::from_utf8(src).ok()?;
        if src.len() > MAX_PARSED {
            flags.reads_this = has_this();
            Err("a file too large to parse at every trace")
        } else if !shape.mocked.is_empty() {
            // A mock call names `jest` or `vi`: the parse would end with this answer.
            flags.reads_this = has_this();
            Err("test-runner mocks")
        } else {
            match Module::read(&path.to_string_lossy(), text, false) {
                Ok(m) => {
                    flags.es_module = true;
                    flags.reads_this = m.reads_this();
                    Ok(m.rows())
                }
                Err(why) => {
                    flags.reads_this = has_this();
                    Err(why)
                }
            }
        }
    } else {
        // A script or a CommonJS module: `require('./x').f()` calls `f` on its exports.
        flags.reads_this = has_this();
        Ok(Rows::default())
    };
    Some(encode(&shape, rows.as_ref().map_err(|e| *e), flags))
}

/// What a re-export forwards ([`Fact::reexports`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Forward<'a> {
    /// `export * from`.
    Star,
    /// `export * as ns from`.
    Namespace(&'a str),
    /// `export {a as b} from`: `(name in the source, exported name)`.
    Named(Vec<(&'a str, &'a str)>),
}

/// What the import, `require` or re-export that starts at some offset uses
/// ([`Fact::at`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Used<'a> {
    /// Everything.
    All,
    /// These names.
    Names(Vec<&'a str>),
    /// It is a re-export.
    Forward(Forward<'a>),
}

/// One seed of a `B` line, read in place.
struct SeedText<'a> {
    /// `n`, `d` or `s`.
    kind: u8,
    /// Some binding of the name is a value.
    value: bool,
    name: &'a str,
    /// The row of every use (empty: nothing reads the binding).
    all: &'a str,
    /// The row of the uses as a whole and the member rows, when the binding is
    /// read member by member: `whole US member US row US member US row
    /// ...`.
    parts: Option<&'a str>,
}

impl<'a> SeedText<'a> {
    fn parse(s: &'a str) -> Option<SeedText<'a>> {
        let b = s.as_bytes();
        let (kind, value) = (*b.first()?, *b.get(1)? == b'v');
        let rest = s.get(2..)?;
        let (name, rest) = rest.split_once(LIST).unwrap_or((rest, ""));
        let (all, parts) = match rest.split_once(LIST) {
            Some((all, parts)) => (all, Some(parts)),
            None => (rest, None),
        };
        Some(SeedText {
            kind,
            value,
            name,
            all,
            parts,
        })
    }

    /// The member rows: `(member, row)`.
    fn members(&self) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        let mut f = self.parts.unwrap_or("").split(LIST).skip(1);
        std::iter::from_fn(move || Some((f.next()?, f.next()?)))
    }

    /// The row of the uses of the binding as a whole.
    fn whole(&self) -> Option<&'a str> {
        self.parts.map(|p| p.split(LIST).next().unwrap_or(""))
    }
}

/// A fact string, read in place.
#[derive(Clone, Copy, Debug)]
pub struct Fact<'a> {
    flags: &'a str,
    body: &'a str,
}

impl<'a> Fact<'a> {
    /// Reads a fact string. `None` for facts of other kinds or versions.
    pub fn parse(fact: &'a str) -> Option<Fact<'a>> {
        let rest = fact.strip_prefix(TAG)?.strip_prefix(VERSION)?;
        let rest = rest.strip_prefix('\t')?;
        let (flags, body) = rest.split_once('\n').unwrap_or((rest, ""));
        Some(Fact { flags, body })
    }

    /// The file is an ES module with facts: reading an export of it is reading
    /// a binding, never a getter that someone wrote.
    pub fn es_module(&self) -> bool {
        self.flags.contains('m')
    }

    /// An export of the file hands out a function that reads `this` (or may:
    /// the file was not read): called as a member of a namespace, the
    /// function reads what that namespace exports.
    pub fn reads_this(&self) -> bool {
        self.flags.contains('t')
    }

    /// Whether the import or `require` at `start` may hand the module's
    /// namespace to a function as `this`: a namespace import that is used
    /// as a whole somewhere (a call through a member, `ns.f()`, is such a
    /// use), or something that is not an import declaration with rows (a
    /// `require`).
    pub fn namespace_as_this(&self, start: u32) -> bool {
        let mut needle = start.to_string();
        needle.push('\t');
        for l in self.lines('B') {
            let Some(seeds) = l.strip_prefix(needle.as_str()) else {
                continue;
            };
            return seeds.split('\t').any(|s| {
                let Some(s) = SeedText::parse(s) else {
                    return true;
                };
                let read = |row: &str| !row.is_empty() && row != "~";
                s.kind == b's'
                    && match s.whole() {
                        Some(whole) => read(whole),
                        None => read(s.all),
                    }
            });
        }
        true
    }

    /// False: nothing about the file's imports can be relied on.
    pub fn understood(&self) -> bool {
        self.flags.contains('u')
    }

    /// Every edge of the file uses everything.
    pub fn uses_all(&self) -> bool {
        self.flags.contains('a')
    }

    fn lines(&self, kind: char) -> impl Iterator<Item = &'a str> + 'a {
        self.body.split('\n').filter_map(move |l| {
            let rest = l.strip_prefix(kind)?;
            Some(rest.strip_prefix('\t').unwrap_or(rest))
        })
    }

    /// The export names of a file whose top level is free of side effects.
    pub fn pure_exports(&self) -> Option<Vec<&'a str>> {
        let l = self.lines('P').next()?;
        Some(l.split('\t').filter(|n| !n.is_empty()).collect())
    }

    /// The names of the named and default bindings of a `B` line (after its
    /// start), sorted.
    fn bound(seeds: &'a str) -> Option<Vec<&'a str>> {
        let mut names = Vec::new();
        for s in seeds.split('\t') {
            let s = SeedText::parse(s)?;
            if s.kind == b's' {
                return None;
            }
            names.push(s.name);
        }
        names.sort_unstable();
        Some(names)
    }

    /// What the import, `require` or re-export that starts at `start` uses.
    /// `None`: the file has none there.
    pub fn at(&self, start: u32) -> Option<Used<'a>> {
        self.at_line(start).map(|u| u.0)
    }

    /// [`Fact::at`], and whether the answer has a line of its own: `false` for
    /// an import declaration whose names are the names of its named and
    /// default bindings (it has no namespace binding, and is not a
    /// `require`).
    pub fn at_line(&self, start: u32) -> Option<(Used<'a>, bool)> {
        let mut needle = start.to_string();
        needle.push('\t');
        let mut bound = None;
        for l in self.body.split('\n') {
            let Some(rest) = l.get(2..).and_then(|r| r.strip_prefix(needle.as_str())) else {
                // A line that ends with the start: `U<TAB>start` never does (a kind follows),
                // a `B` line always has a seed.
                continue;
            };
            match l.as_bytes()[0] {
                b'U' => {
                    let mut f = rest.split('\t');
                    return match f.next()? {
                        "a" => Some((Used::All, true)),
                        "n" => Some((Used::Names(f.collect()), true)),
                        _ => None,
                    };
                }
                b'R' => {
                    let mut f = rest.split('\t');
                    let forward = match f.next()? {
                        "s" => Forward::Star,
                        "n" => Forward::Namespace(f.next()?),
                        "p" => Forward::Named(
                            f.map(|p| p.split_once(LIST)).collect::<Option<Vec<_>>>()?,
                        ),
                        _ => return None,
                    };
                    return Some((Used::Forward(forward), true));
                }
                // The names of the declaration's bindings, unless a `U` line says otherwise.
                b'B' => bound = Self::bound(rest),
                _ => {}
            }
        }
        bound.map(|names| (Used::Names(names), false))
    }

    /// The imports and requires: the start of each and the names it uses
    /// (`None`: everything).
    pub fn uses(&self) -> Vec<(u32, Option<Vec<&'a str>>)> {
        let mut out: Vec<(u32, Option<Vec<&'a str>>)> = Vec::new();
        for l in self.lines('U') {
            let mut f = l.split('\t');
            let Some(start) = f.next().and_then(|s| s.parse().ok()) else {
                continue;
            };
            match f.next() {
                Some("a") => out.push((start, None)),
                Some("n") => out.push((start, Some(f.collect()))),
                _ => {}
            }
        }
        // The declarations whose `U` line was left out.
        for l in self.lines('B') {
            let Some((start, seeds)) = l.split_once('\t') else {
                continue;
            };
            let Ok(start) = start.parse() else { continue };
            if !out.iter().any(|u| u.0 == start)
                && let Some(names) = Self::bound(seeds)
            {
                out.push((start, Some(names)));
            }
        }
        out.sort_by_key(|u| u.0);
        out
    }

    /// The re-exports: the start of each and what it forwards.
    pub fn reexports(&self) -> impl Iterator<Item = (u32, Forward<'a>)> + 'a {
        self.lines('R').filter_map(|l| {
            let mut f = l.split('\t');
            let start = f.next()?.parse().ok()?;
            let kind = match f.next()? {
                "s" => Forward::Star,
                "n" => Forward::Namespace(f.next()?),
                "p" => Forward::Named(f.map(|p| p.split_once(LIST)).collect::<Option<Vec<_>>>()?),
                _ => return None,
            };
            Some((start, kind))
        })
    }

    /// Why the file has no rows (`None`: it has them, or no import
    /// declaration).
    pub fn no_rows(&self) -> Option<&'a str> {
        self.lines('N').next()
    }

    /// The rows, decoded (for tests and tools; [`Fact::taint`] reads them in
    /// place).
    pub fn rows(&self) -> Option<Rows> {
        let mut out = Rows::default();
        if let Some(l) = self.lines('E').next() {
            out.exports = l.split('\t').map(str::to_owned).collect();
        }
        if let Some(l) = self.lines('Y').next() {
            out.whys = l.split('\t').map(str::to_owned).collect();
        }
        for l in self.lines('C') {
            out.conds.push(
                l.split('\t')
                    .filter(|a| !a.is_empty())
                    .map(parse_atom)
                    .collect::<Option<Vec<_>>>()?,
            );
        }
        for l in self.lines('B') {
            let (start, seeds) = l.split_once('\t')?;
            let start = start.parse().ok()?;
            for s in seeds.split('\t') {
                let s = SeedText::parse(s)?;
                let parts = match s.whole() {
                    None => None,
                    Some(whole) => {
                        let mut members = Vec::new();
                        for (name, row) in s.members() {
                            members.push((name.to_owned(), parse_row(row)?));
                        }
                        Some((parse_row(whole)?, members))
                    }
                };
                out.seeds.push(Seed {
                    start,
                    kind: match s.kind {
                        b'n' => SeedKind::Name,
                        b'd' => SeedKind::Default,
                        b's' => SeedKind::Namespace,
                        _ => return None,
                    },
                    name: s.name.to_owned(),
                    value: s.value,
                    all: parse_row(s.all)?,
                    parts,
                });
            }
        }
        Some(out)
    }

    /// [`Module::taint_members`] from the rows: what the file derives from the
    /// imports `asked`, with `members` mapping a changed name to its
    /// changed members. `None` when the file has no rows, or one of the
    /// imports is not an import declaration of it.
    pub fn taint(
        &self,
        asked: &[Asked<'_>],
        members: &BTreeMap<String, Vec<String>>,
        oracle: &dyn Oracle,
    ) -> Option<Taint> {
        // The rows that carry the change.
        let mut rows: Vec<&'a str> = Vec::new();
        let mut relinked = false;
        let mut found = 0usize;
        for l in self.body.split('\n') {
            let Some(l) = l.strip_prefix("B\t") else {
                if l.starts_with('N') {
                    return None;
                }
                continue;
            };
            let (start, seeds) = l.split_once('\t')?;
            let start: u32 = start.parse().ok()?;
            for a in asked.iter().filter(|a| a.start == start) {
                found += 1;
                for s in seeds.split('\t') {
                    let s = SeedText::parse(s)?;
                    // A namespace import reads every name; a default import reads `default`.
                    let named = s.kind != b's';
                    let hit = match (named, a.names) {
                        (_, None) | (false, _) => true,
                        (true, Some(names)) => names.iter().any(|n| n == s.name),
                    };
                    relinked |= named && s.value && a.rebound.iter().any(|r| r == s.name);
                    if !hit {
                        continue;
                    }
                    // Only some members of the name changed, and the file reads some of
                    // it member by member: the statements that use it as a whole, and the
                    // ones that read a changed member.
                    let only = (s.kind == b'n' && a.names.is_some())
                        .then(|| members.get(s.name))
                        .flatten();
                    match (only, s.whole()) {
                        (Some(changed), Some(whole)) => {
                            rows.push(whole);
                            for (member, row) in s.members() {
                                if changed.iter().any(|c| c == member) {
                                    rows.push(row);
                                }
                            }
                        }
                        _ => rows.push(s.all),
                    }
                }
            }
        }
        if found != asked.len() {
            // `import 'x'` binds nothing and reads nothing; anything else is not an import
            // declaration this file has.
            return None;
        }
        let mut out = Taint {
            evaluation: relinked
                .then(|| "an import of a name that was added, removed or redeclared".to_owned()),
            any: relinked,
            ..Taint::default()
        };
        let mut exports: Vec<u32> = Vec::new();
        // Question lists already asked: the index and the answer.
        let mut asked_conds: Vec<(u32, bool)> = Vec::new();
        let (mut whys, mut conds): (Option<Vec<&str>>, Option<Vec<&str>>) = (None, None);
        for r in rows {
            if r.is_empty() {
                continue;
            }
            let row = parse_row(r)?;
            out.any |= row.any;
            for e in row.exports {
                if !exports.contains(&e) {
                    exports.push(e);
                }
            }
            for l in row.loads {
                let (why, holds) = match l {
                    Load::Always(why) => (why, false),
                    Load::Unless(why, cond) => {
                        let known = asked_conds.iter().find(|c| c.0 == cond).map(|c| c.1);
                        let holds = match known {
                            Some(h) => h,
                            None => {
                                let conds = conds.get_or_insert_with(|| self.lines('C').collect());
                                let mut holds = true;
                                for a in conds
                                    .get(cond as usize)?
                                    .split('\t')
                                    .filter(|a| !a.is_empty())
                                {
                                    if !parse_atom(a)?.holds(oracle) {
                                        holds = false;
                                        break;
                                    }
                                }
                                asked_conds.push((cond, holds));
                                if holds {
                                    out.pure_calls += 1;
                                }
                                holds
                            }
                        };
                        (why, holds)
                    }
                };
                if !holds && out.evaluation.is_none() {
                    let whys = whys.get_or_insert_with(|| {
                        self.lines('Y')
                            .next()
                            .map(|l| l.split('\t').collect())
                            .unwrap_or_default()
                    });
                    out.evaluation = Some((*whys.get(why as usize)?).to_owned());
                }
            }
        }
        if !exports.is_empty() {
            let names: Vec<&str> = self.lines('E').next()?.split('\t').collect();
            exports.sort_unstable();
            for e in exports {
                out.exports.push((*names.get(e as usize)?).to_owned());
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{super::Alone, *};

    /// An oracle that vouches for imports whose name starts with `build`, and
    /// says an import named `PRIM` is a primitive.
    struct Named;

    impl Oracle for Named {
        fn builds(&self, c: Callee<'_>, steps: &[Step], _: bool) -> bool {
            c.name.starts_with("build") && steps.iter().all(|s| *s == Step::Call)
        }
        fn prim(&self, c: Callee<'_>) -> bool {
            c.name == "PRIM"
        }
    }

    /// The fact of `src`, read back.
    fn fact_of(name: &str, src: &str) -> String {
        fact(Path::new(name), src.as_bytes()).expect("a fact")
    }

    /// Compares [`Fact::taint`] with [`Module::taint_members`] for every
    /// non-empty subset of the names of every import declaration of `src`
    /// (and "every name"), alone and together with each other declaration,
    /// with and without member-level changes.
    fn same_as_joint(name: &str, src: &str, oracle: &dyn Oracle, exact: bool) -> usize {
        let m = Module::read(name, src, false).expect("facts");
        let text = fact_of(name, src);
        let f = Fact::parse(&text).expect("parses");
        let (mut decoded, rows) = (f.rows().expect("rows"), m.rows());
        if decoded.exports.is_empty() {
            // The exports are listed only when a row refers to one.
            decoded.exports = rows.exports.clone();
        }
        assert_eq!(decoded, rows, "the rows survive the encoding");
        // Per import declaration: its start and imported names.
        let mut decls: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
        for b in &m.bindings {
            if let Some(i) = &b.import {
                let names = decls.entry(i.start).or_default();
                match &i.imported {
                    super::super::Imported::Name(n) => {
                        names.insert(n.clone());
                    }
                    super::super::Imported::Default => {
                        names.insert("default".to_owned());
                    }
                    super::super::Imported::Namespace => {}
                }
            }
        }
        // Member names read anywhere, as candidates for member-level changes.
        let mut member_names: BTreeSet<String> = BTreeSet::new();
        for s in &m.stmts {
            for r in &s.refs {
                member_names.extend(r.members.iter().flatten().cloned());
            }
        }
        let member_names: Vec<String> = member_names.into_iter().take(4).collect();
        // The questions: each declaration with each subset of its names (at most 6
        // names).
        let mut questions: Vec<(u32, Option<Vec<String>>)> = Vec::new();
        for (start, names) in &decls {
            questions.push((*start, None));
            let names: Vec<&String> = names.iter().take(6).collect();
            for mask in 1u32..(1 << names.len()) {
                let set: Vec<String> = names
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, n)| (*n).clone())
                    .collect();
                questions.push((*start, Some(set)));
            }
        }
        let mut compared = 0;
        let mut check = |asked: &[Asked<'_>], members: &BTreeMap<String, Vec<String>>| {
            let joint = m.taint_members(asked, members, oracle);
            let rows = f.taint(asked, members, oracle);
            match (joint, rows) {
                (None, None) => {}
                (Some(j), Some(r)) => {
                    let same = j.exports == r.exports
                        && j.any == r.any
                        && j.evaluation.is_some() == r.evaluation.is_some();
                    // The rows may say more, never less.
                    let covers = j.exports.iter().all(|e| r.exports.contains(e))
                        && (!j.any || r.any)
                        && (j.evaluation.is_none() || r.evaluation.is_some());
                    assert!(
                        if exact { same } else { covers },
                        "{asked:?} {members:?}: joint {j:?}, rows {r:?}\n{src}"
                    );
                }
                (j, r) => panic!("{asked:?}: joint {j:?}, rows {r:?}\n{src}"),
            }
            compared += 1;
        };
        let none = BTreeMap::new();
        for (i, (start, names)) in questions.iter().enumerate() {
            let one = [Asked {
                start: *start,
                names: names.as_deref(),
                rebound: &[],
            }];
            check(&one, &none);
            // With each name's members narrowed to each candidate member.
            for n in names.iter().flatten() {
                for member in &member_names {
                    let narrowed: BTreeMap<String, Vec<String>> =
                        [(n.clone(), vec![member.clone()])].into_iter().collect();
                    check(&one, &narrowed);
                }
            }
            // Rebound names.
            if let Some(n) = names.as_ref().and_then(|n| n.iter().next()) {
                let rebound = [n.clone()];
                check(
                    &[Asked {
                        start: *start,
                        names: names.as_deref(),
                        rebound: &rebound,
                    }],
                    &none,
                );
            }
            // Together with a question about another declaration.
            for (other, other_names) in questions.iter().skip(i + 1) {
                if other == start {
                    continue;
                }
                let two = [
                    one[0],
                    Asked {
                        start: *other,
                        names: other_names.as_deref(),
                        rebound: &[],
                    },
                ];
                check(&two, &none);
                for n in names.iter().flatten().chain(other_names.iter().flatten()) {
                    for member in &member_names {
                        let narrowed: BTreeMap<String, Vec<String>> =
                            [(n.clone(), vec![member.clone()])].into_iter().collect();
                        check(&two, &narrowed);
                    }
                }
            }
        }
        compared
    }

    const SAMPLES: &[(&str, &str)] = &[
        (
            "a.ts",
            "import { x, y, type T } from './f';\nimport z from './g';\nconst local = x + \
             1;\nexport const a = () => local;\nexport const b = y;\nexport function c(v: T) { \
             return z(v); }\nexport const d = 1;\n",
        ),
        (
            "load.ts",
            "import { x, y } from './f';\nimport { reg } from './r';\nreg(x);\nexport const a = \
             y;\nconsole.log('loaded');\n",
        ),
        (
            "table.ts",
            "import { Resource, LIMITS } from './f';\nimport * as ns from './g';\nexport const a \
             = Resource.Team;\nexport const b = [Resource.User, LIMITS.free];\nexport const c = \
             Object.keys(Resource);\nexport const d = () => ns.a + ns.b;\nexport const e = () => \
             Resource.Team + LIMITS.pro;\nexport const f = ns;\n",
        ),
        (
            "builders.ts",
            "import { build, buildOther, PRIM, plain } from './b';\nimport { x } from \
             './f';\nexport const a = build(x);\nexport const b = buildOther({ k: x });\nexport \
             const c = `${PRIM}-c`;\nexport const d = plain(x);\nconst made = build(1);\nexport \
             const e = made;\n",
        ),
        (
            "chain.ts",
            "import { T, build } from './f';\nexport const a = T.make.build(1);\nexport const b = \
             T.other;\nexport const c = build(T.other);\n",
        ),
        (
            "class.ts",
            "import { Base, mixin } from './f';\nimport { helper } from './h';\nexport class A \
             extends Base { m() { return helper(); } }\nexport class B { static s = helper; n() { \
             return mixin; } }\nexport default class extends B {}\n",
        ),
        (
            "jsx.tsx",
            "import { h } from './h';\nimport { Box, label } from './ui';\n/** @jsx h */\nexport \
             const A = () => <Box title={label} />;\nexport const B = () => \
             <div>{label}</div>;\nexport const c = 1;\n",
        ),
        (
            "reexport.ts",
            "import { x } from './f';\nexport { y } from './f';\nexport * from './g';\nexport { x \
             };\nexport const z = () => x;\n",
        ),
        (
            "this.ts",
            "import { x, y } from './f';\nexport const a = x;\nexport const b = y;\nexport \
             function f() { return this.a; }\n",
        ),
        // A binding that no statement reads, exported again: the export hands it on.
        (
            "again.ts",
            "import { x, y, type T } from './f';\nimport d from './g';\nexport { x };\nexport \
             type { T };\nexport { d as default };\nexport const z = 1;\n",
        ),
        (
            "typeonly.ts",
            "import type { T } from './f';\nimport { type U, v } from './f';\nexport const a: T = \
             v;\nexport type W = U;\nexport const b = 1;\n",
        ),
    ];

    #[test]
    fn rows_answer_as_the_joint_computation() {
        let mut compared = 0;
        for (name, src) in SAMPLES {
            // A chain rooted in a binding reads the binding as a whole in the rows; the
            // joint computation reads it member by member until the root is tainted.
            let exact = *name != "chain.ts";
            compared += same_as_joint(name, src, &Alone, exact);
            compared += same_as_joint(name, src, &Named, exact);
        }
        assert!(compared > 400, "{compared} comparisons");
    }

    #[test]
    fn the_fact_carries_the_shape() {
        let text = fact_of(
            "a.ts",
            "import { x, y as z } from './f';\nimport * as ns from './g';\nimport \
             './side';\nexport * from './h';\nexport * as all from './i';\nexport { p as q, r } \
             from './j';\nexport const a = () => x + ns.one;\n",
        );
        let f = Fact::parse(&text).expect("parses");
        assert!(f.understood() && !f.uses_all());
        let uses: Vec<(u32, Option<Vec<&str>>)> = f.uses();
        assert_eq!(uses[0].1, Some(vec!["x", "y"]));
        assert_eq!(uses[1].1, Some(vec!["one"]));
        assert_eq!(uses[2].1, Some(vec![]));
        // One import at a time, as the walk asks.
        for (start, used) in &uses {
            assert_eq!(
                f.at(*start),
                Some(match used {
                    Some(names) => Used::Names(names.clone()),
                    None => Used::All,
                })
            );
        }
        for (start, forward) in f.reexports() {
            assert_eq!(f.at(start), Some(Used::Forward(forward)));
        }
        assert_eq!(f.at(3), None);
        // The names of a declaration with bindings are written once, with the bindings.
        assert!(!text.contains("\nU\t0\t"), "{text:?}");
        assert!(text.contains("\nB\t0\t"), "{text:?}");
        let forwards: Vec<Forward<'_>> = f.reexports().map(|r| r.1).collect();
        assert_eq!(
            forwards,
            vec![
                Forward::Star,
                Forward::Namespace("all"),
                Forward::Named(vec![("p", "q"), ("r", "r")]),
            ]
        );
        // `export *` leaves the exports open: not a pure file.
        assert_eq!(f.pure_exports(), None);
        let pure = fact_of("b.ts", "export const a = 1;\nexport function f() {}\n");
        assert_eq!(
            Fact::parse(&pure).expect("parses").pure_exports(),
            Some(vec!["a", "f"])
        );
    }

    #[test]
    fn files_without_rows_say_why() {
        for (src, why) in [
            (
                "import { a } from './a';\nmodule.exports = a;\n",
                "CommonJS",
            ),
            (
                "import { a } from './a';\njest.mock('./a');\nexport const b = a;\n",
                "test-runner mocks",
            ),
            (
                "import { a } from './a';\nexport const b = eval('a');\n",
                "a call of `eval`",
            ),
        ] {
            let text = fact_of("a.ts", src);
            let f = Fact::parse(&text).expect("parses");
            assert_eq!(f.no_rows(), Some(why), "{src}");
            assert_eq!(f.taint(&[], &BTreeMap::new(), &Alone), None);
        }
        // No import declaration: no rows, and nothing to say about that.
        let text = fact_of("a.ts", "export const a = 1;\n");
        let f = Fact::parse(&text).expect("parses");
        assert_eq!(f.no_rows(), None);
        assert!(f.es_module() && !f.reads_this());
        // A script is not an ES module, and may hand `this` to anything.
        let text = fact_of("a.js", "module.exports = function () { return this; };\n");
        let f = Fact::parse(&text).expect("parses");
        assert!(!f.es_module() && f.reads_this());
    }

    #[test]
    fn a_function_that_reads_this_marks_the_file() {
        for (src, reads) in [
            ("export function f() { return this.a; }\n", true),
            (
                "function impl() { return this; }\nexport const f = impl;\n",
                true,
            ),
            (
                "function impl() { return this; }\nconst g = impl;\nexport { g as f };\n",
                true,
            ),
            // A method's `this` is its object, and a function no export reaches is not
            // handed out.
            ("export class C { m() { return this; } }\n", false),
            (
                "function local() { return this; }\nexport const a = 1;\n",
                false,
            ),
            ("export const o = { m() { return this; } };\n", false),
        ] {
            let text = fact_of("a.ts", src);
            assert_eq!(
                Fact::parse(&text).expect("parses").reads_this(),
                reads,
                "{src}"
            );
        }
        // A namespace import that is called through hands the namespace over; one that
        // is only read member by member does not.
        let text = fact_of(
            "a.ts",
            "import * as called from './f';\nimport * as read from './g';\nimport { x } from \
             './h';\nexport const a = () => called.f();\nexport const b = () => read.v + x;\n",
        );
        let f = Fact::parse(&text).expect("parses");
        assert!(f.namespace_as_this(0));
        assert!(!f.namespace_as_this(31));
        assert!(!f.namespace_as_this(60));
        // Not an import declaration of the file.
        assert!(f.namespace_as_this(7));
    }

    #[test]
    fn an_import_that_is_not_a_declaration_has_no_answer() {
        let text = fact_of("a.ts", "import { a } from './a';\nexport const b = a;\n");
        let f = Fact::parse(&text).expect("parses");
        let names = ["a".to_owned()];
        let ask = |start| {
            f.taint(
                &[Asked {
                    start,
                    names: Some(&names),
                    rebound: &[],
                }],
                &BTreeMap::new(),
                &Alone,
            )
        };
        assert_eq!(ask(0).map(|t| t.exports), Some(vec!["b".to_owned()]));
        assert_eq!(ask(7), None);
    }

    #[test]
    fn questions_survive_the_encoding() {
        let atoms = vec![
            Atom::Builds {
                spec: "./b".to_owned(),
                name: "default".to_owned(),
                steps: vec![Step::Member("object".to_owned()), Step::Call, Step::Call],
                literal_args: true,
            },
            Atom::BuildsWith {
                spec: "flags/next".to_owned(),
                name: "flag".to_owned(),
                steps: vec![Step::Call],
                literal_args: false,
                arg: vec![
                    ("key".to_owned(), OwnedProp::Other),
                    ("decide".to_owned(), OwnedProp::Function),
                    ("...".to_owned(), OwnedProp::Spread),
                    (
                        "adapter".to_owned(),
                        OwnedProp::Chain(
                            "@flags-sdk/vercel".to_owned(),
                            "vercelAdapter".to_owned(),
                            vec![Step::Call],
                        ),
                    ),
                ],
            },
            Atom::Continues {
                spec: "./c".to_owned(),
                name: "*".to_owned(),
                made: vec![Step::Call],
                steps: vec![],
            },
            Atom::Prim {
                spec: "./p".to_owned(),
                name: "P".to_owned(),
            },
        ];
        for a in atoms {
            let mut text = String::new();
            assert!(atom_text(&a, &mut text));
            assert_eq!(parse_atom(&text), Some(a), "{text:?}");
        }
    }
}
