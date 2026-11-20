//! Property tests on random graphs against naive reference implementations.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]
// The reference implementations index by node number on purpose.
#![allow(clippy::needless_range_loop)]

use std::collections::{BTreeMap, VecDeque};

use proptest::prelude::*;
use turborepo_trace_graph::{
    AffectedOptions, ChangeKind, ChangeSet, EdgeSpec, Engine, GraphView, MemGraph, NodeId, NodeSet,
    Policy, RefFlags, RefKind, UncertaintyKind, Verdict, glob::Glob,
};

const DIRS: [&str; 4] = ["", "a", "a/b", "c"];
const NAMES: [&str; 4] = ["x.ts", "y.json", "z.ts", "w"];
const PATTERNS: [&str; 4] = ["*.ts", "*.json", "**/*.ts", "*"];

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

#[derive(Clone, Debug)]
struct Spec {
    n: usize,
    dirs: Vec<usize>,
    edges: Vec<(usize, Option<usize>, bool)>,
    uncertain: Vec<(usize, bool)>,
    external: Vec<bool>,
    probes: Vec<(usize, usize, usize)>,
    members: Vec<(usize, usize, usize)>,
}

fn spec() -> impl Strategy<Value = Spec> {
    (1usize..40).prop_flat_map(|n| {
        (
            Just(n),
            prop::collection::vec(0..DIRS.len(), n),
            prop::collection::vec(
                (0..n, prop::option::weighted(0.9, 0..n), any::<bool>()),
                0..n * 3,
            ),
            prop::collection::vec((0..n, any::<bool>()), 0..4),
            prop::collection::vec(prop::bool::weighted(0.15), n),
            prop::collection::vec((0..n, 0..DIRS.len(), 0..NAMES.len()), 0..6),
            prop::collection::vec((0..n, 0..DIRS.len(), 0..PATTERNS.len()), 0..4),
        )
            .prop_map(
                |(n, dirs, edges, uncertain, external, probes, members)| Spec {
                    n,
                    dirs,
                    edges,
                    uncertain,
                    external,
                    probes,
                    members,
                },
            )
    })
}

fn path_of(s: &Spec, i: usize) -> String {
    join(DIRS[s.dirs[i]], &format!("f{i}.ts"))
}

fn build(s: &Spec) -> MemGraph {
    let mut b = MemGraph::builder();
    let first = b.package("first", None, "");
    let ext = b.package("ext", Some("1.0.0"), "node_modules/ext");
    for i in 0..s.n {
        let pkg = if s.external[i] { ext } else { first };
        b.add_file(&path_of(s, i), pkg, Some("js"), s.external[i]);
    }
    for &(f, t, type_only) in &s.edges {
        let flags = if type_only {
            RefFlags::TYPE_ONLY
        } else {
            RefFlags::NONE
        };
        b.add_edge(
            NodeId(f as u32),
            EdgeSpec::new(t.map(|t| NodeId(t as u32)), RefKind::Import).flags(flags),
        );
    }
    for &(f, partial) in &s.uncertain {
        let kind = if partial {
            UncertaintyKind::PartialSpecifier
        } else {
            UncertaintyKind::EnvDependent
        };
        b.uncertainty(NodeId(f as u32), kind, 1, "x");
    }
    for &(f, d, name) in &s.probes {
        b.probe(NodeId(f as u32), &join(DIRS[d], NAMES[name]));
    }
    for &(f, d, p) in &s.members {
        b.membership(NodeId(f as u32), DIRS[d], PATTERNS[p]);
    }
    b.build().unwrap()
}

fn succ(s: &Spec, i: usize) -> Vec<usize> {
    s.edges
        .iter()
        .filter(|(f, _, _)| *f == i)
        .filter_map(|(_, t, _)| *t)
        .collect()
}

fn reach(s: &Spec, from: usize) -> Vec<bool> {
    let mut seen = vec![false; s.n];
    let mut stack = vec![from];
    seen[from] = true;
    while let Some(x) = stack.pop() {
        for t in succ(s, x) {
            if !seen[t] {
                seen[t] = true;
                stack.push(t);
            }
        }
    }
    seen
}

fn dist(s: &Spec, from: usize, targets: &[bool]) -> Option<usize> {
    let mut d = vec![usize::MAX; s.n];
    let mut q = VecDeque::from([from]);
    d[from] = 0;
    while let Some(x) = q.pop_front() {
        if targets[x] {
            return Some(d[x]);
        }
        for t in succ(s, x) {
            if d[t] == usize::MAX {
                d[t] = d[x] + 1;
                q.push_back(t);
            }
        }
    }
    None
}

fn set(n: usize, ids: &[usize]) -> NodeSet {
    NodeSet::from_iter_in(n as u32, ids.iter().map(|&i| NodeId(i as u32)))
}

fn has_edge(g: &MemGraph, from: NodeId, to: NodeId) -> bool {
    g.out_edges(from).any(|e| e.to == Some(to))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn rdeps_and_deps_match_naive(s in spec(), seeds in prop::collection::vec(0usize..40, 1..4), depth in prop::option::of(0u32..4)) {
        let g = build(&s);
        let e = Engine::new(&g);
        let seeds: Vec<usize> = seeds.into_iter().map(|x| x % s.n).collect();
        let r = e.rdeps(&set(s.n, &seeds), None);
        for v in 0..s.n {
            let naive = { let rv = reach(&s, v); seeds.iter().any(|&x| rv[x]) };
            prop_assert_eq!(r.contains(NodeId(v as u32)), naive, "rdeps node {}", v);
        }
        let d = e.deps(&set(s.n, &seeds), depth);
        let mut targets = vec![false; s.n];
        for v in 0..s.n {
            targets.fill(false);
            targets[v] = true;
            let naive = seeds.iter().filter_map(|&x| dist(&s, x, &targets)).min()
                .is_some_and(|k| depth.is_none_or(|dd| k <= dd as usize));
            prop_assert_eq!(d.contains(NodeId(v as u32)), naive, "deps node {}", v);
        }
    }

    #[test]
    fn somepath_is_a_shortest_valid_path(s in spec(), a in 0usize..40, b in 0usize..40) {
        let (a, b) = (a % s.n, b % s.n);
        let g = build(&s);
        let e = Engine::new(&g);
        let p = e.somepath(&set(s.n, &[a]), &set(s.n, &[b]));
        let mut t = vec![false; s.n];
        t[b] = true;
        match (p, dist(&s, a, &t)) {
            (None, None) => {}
            (Some(p), Some(k)) => {
                prop_assert_eq!(p.len(), k + 1);
                prop_assert_eq!(p[0], NodeId(a as u32));
                prop_assert_eq!(*p.last().unwrap(), NodeId(b as u32));
                for w in p.windows(2) {
                    prop_assert!(has_edge(&g, w[0], w[1]));
                }
            }
            (p, k) => prop_assert!(false, "somepath {:?} vs naive distance {:?}", p, k),
        }
        // allpaths = {v : a reaches v and v reaches b}.
        let all = e.allpaths(&set(s.n, &[a]), &set(s.n, &[b]));
        let ra = reach(&s, a);
        for v in 0..s.n {
            let naive = ra[v] && reach(&s, v)[b];
            prop_assert_eq!(all.contains(NodeId(v as u32)), naive);
        }
    }

    #[test]
    fn affected_matches_naive(
        s in spec(),
        modified in prop::collection::vec(0usize..40, 0..3),
        deleted in prop::collection::vec(0usize..40, 0..2),
        added in prop::collection::vec((0..DIRS.len(), 0..NAMES.len()), 0..3),
        deleted_missing in prop::collection::vec((0..DIRS.len(), 0..NAMES.len()), 0..2),
        entries in prop::collection::vec(0usize..40, 1..10),
        policy in prop_oneof![Just(Policy::Affected), Just(Policy::Unknown), Just(Policy::Ignore)],
    ) {
        let g = build(&s);
        let e = Engine::new(&g);
        let mut cs = ChangeSet::new();
        for &m in &modified { cs.push(ChangeKind::Modified, path_of(&s, m % s.n)); }
        for &d in &deleted { cs.push(ChangeKind::Deleted, path_of(&s, d % s.n)); }
        let added: Vec<String> = added.iter().map(|&(d, n)| join(DIRS[d], NAMES[n])).collect();
        for p in &added { cs.push(ChangeKind::Added, p.as_str()); }
        let gone: Vec<String> = deleted_missing.iter().map(|&(d, n)| join(DIRS[d], NAMES[n])).collect();
        for p in &gone { cs.push(ChangeKind::Deleted, p.as_str()); }
        let entries: Vec<usize> = entries.into_iter().map(|x| x % s.n).collect();
        let opts = AffectedOptions::default().policy(policy);
        let rep = e.affected(&cs, &set(s.n, &entries), &opts);

        // Naive seeds.
        let mut seed = vec![false; s.n];
        for &m in modified.iter().chain(&deleted) { seed[m % s.n] = true; }
        // Adds and deletes (of nodes or of missing paths) change directory listings.
        let deleted_paths: Vec<String> = deleted.iter().map(|&d| path_of(&s, d % s.n)).collect();
        let listing_changes: Vec<&String> =
            added.iter().chain(&gone).chain(&deleted_paths).collect();
        for &(f, d, name) in &s.probes {
            let probe = join(DIRS[d], NAMES[name]);
            if added.iter().any(|a| *a == probe || a.starts_with(&format!("{probe}/"))) {
                seed[f] = true;
            }
        }
        for &(f, d, p) in &s.members {
            let dir = DIRS[d];
            let glob = Glob::new(PATTERNS[p]).unwrap();
            for c in &listing_changes {
                let rest = if dir.is_empty() { Some(c.as_str()) } else { c.strip_prefix(&format!("{dir}/")) };
                if rest.is_some_and(|r| glob.matches(r)) { seed[f] = true; }
            }
        }
        let uncertain: Vec<bool> = (0..s.n).map(|i| !s.external[i] &&
            s.uncertain.iter().any(|&(f, partial)| f == i && !partial)).collect();

        let mut expect = BTreeMap::new();
        for &en in &entries {
            let closure = reach(&s, en);
            let v = if (0..s.n).any(|i| closure[i] && seed[i]) {
                Verdict::Affected
            } else if (0..s.n).any(|i| closure[i] && uncertain[i]) {
                match policy {
                    Policy::Affected => Verdict::Affected,
                    Policy::Unknown => Verdict::Unknown,
                    Policy::Ignore => Verdict::Unaffected,
                }
            } else {
                Verdict::Unaffected
            };
            if v != Verdict::Unaffected { expect.insert(en, v); }
        }
        let got: BTreeMap<usize, Verdict> =
            rep.entries.iter().map(|r| (r.entry.0 as usize, r.verdict)).collect();
        prop_assert_eq!(&got, &expect);

        // Witnesses: start at a seed (or an uncertain node), end at the entry, follow edges,
        // and are shortest.
        for r in &rep.entries {
            let w = &r.witness;
            prop_assert_eq!(*w.last().unwrap(), r.entry);
            for pair in w.windows(2) {
                prop_assert!(has_edge(&g, pair[1], pair[0]));
            }
            let start = w[0].0 as usize;
            let targets = if r.cause.is_some() { &seed } else { &uncertain };
            prop_assert!(targets[start]);
            prop_assert_eq!(Some(w.len() - 1), dist(&s, r.entry.0 as usize, targets));
        }

        // After-change mode (graph refreshed from the working tree): additionally, a deleted
        // path seeds the nodes whose probes it matches, exactly like an added path does.
        let rep2 = e.affected(&cs, &set(s.n, &entries), &opts.clone().after_change(true));
        let mut seed2 = seed.clone();
        for &(f, d, name) in &s.probes {
            let probe = join(DIRS[d], NAMES[name]);
            if gone.iter().chain(&deleted_paths).any(|a| *a == probe || a.starts_with(&format!("{probe}/"))) {
                seed2[f] = true;
            }
        }
        let mut expect2 = BTreeMap::new();
        for &en in &entries {
            let closure = reach(&s, en);
            let v = if (0..s.n).any(|i| closure[i] && seed2[i]) {
                Verdict::Affected
            } else {
                expect.get(&en).copied().unwrap_or(Verdict::Unaffected)
            };
            if v != Verdict::Unaffected { expect2.insert(en, v); }
        }
        let got2: BTreeMap<usize, Verdict> =
            rep2.entries.iter().map(|r| (r.entry.0 as usize, r.verdict)).collect();
        prop_assert_eq!(&got2, &expect2);
        for r in rep2.entries.iter().filter(|r| r.cause.is_some()) {
            prop_assert!(seed2[r.witness[0].0 as usize]);
            prop_assert_eq!(Some(r.witness.len() - 1), dist(&s, r.entry.0 as usize, &seed2));
        }
    }

    #[test]
    fn json_round_trips(s in spec()) {
        let g = build(&s);
        let j = g.to_json_string();
        let g2 = MemGraph::from_json_str(&j).unwrap();
        prop_assert_eq!(g2.to_json_string(), j);
    }
}
