//! Replaying a recorded binding run ([`trace`]): whenever a replay answers, its
//! answer is what binding from nothing gives in that world; and it answers when
//! no fact the run looked at changed.

use super::{
    super::{
        bind::{Bound, World, bind},
        show,
        trace::{self, Recorder, Trace, replay},
    },
    LOADS, ROUTES, Repo,
};

const HELPER: &str = "helpers/setup-endpoint.ts";
const INDEX: (&str, &str) = ("helpers/index.ts", "export * from './setup-endpoint';\n");
const A: &str = "import { setupEndpoint } from '@scope/helpers';\nconst { client } = \
                 setupEndpoint({ extraRoutes: ['svc-a'] });\n";
const B: &str = "import { setupEndpoint } from '@scope/helpers';\nsetupEndpoint();\n";

fn repo(files: &[(&str, &str)]) -> Repo {
    let mut r = Repo::new(files);
    r.aliases.push(("@scope/helpers", "helpers/index.ts"));
    r
}

fn base() -> Vec<(&'static str, &'static str)> {
    vec![
        (HELPER, ROUTES),
        ("pkg/a.test.ts", A),
        ("pkg/b.test.ts", B),
        INDEX,
    ]
}

fn ids(r: &Repo, roots: &[&str]) -> Vec<u32> {
    roots.iter().filter_map(|x| r.id(x)).collect()
}

/// Binds `r` behind a recorder: the result and the trace, which went through
/// its stored form.
fn record(r: &Repo, roots: &[&str]) -> (Bound, Trace) {
    let roots = ids(r, roots);
    let rec = Recorder::new(r);
    let b = bind(&rec, &roots, LOADS);
    assert_eq!(b, bind(r, &roots, LOADS), "a recorder changes nothing");
    let t = rec.finish(&roots, &b).expect("a trace");
    let stored = Trace::from_bytes(&t.to_bytes()).expect("the stored form");
    assert_eq!(stored, t);
    (b, stored)
}

/// The replay of `t` in `r`, shown. A replay that answers must answer what
/// binding gives.
fn replayed(t: &Trace, r: &Repo, roots: &[&str]) -> Option<String> {
    let roots = ids(r, roots);
    let mut t = t.clone();
    let out = replay(&mut t, r, &roots)?;
    assert_eq!(
        out.bound,
        bind(r, &roots, LOADS),
        "a replayed result is the result of binding"
    );
    Some(show::bound(&out.bound, &|f| r.path(f)))
}

#[test]
fn an_unchanged_world_replays() {
    let r = repo(&base());
    let (b, t) = record(&r, &[HELPER]);
    assert!(t.questions() > 4 && t.files() >= 4, "{t:?}");
    assert_eq!(
        replayed(&t, &r, &[HELPER]),
        Some(show::bound(&b, &|f| r.path(f)))
    );
    // Nothing about the trace changed: there is nothing to store again.
    let mut again = t.clone();
    let out = replay(&mut again, &r, &ids(&r, &[HELPER])).expect("replays");
    assert!(!out.updated && again == t);
}

#[test]
fn a_file_the_run_never_looked_at_changes_nothing() {
    let (_, t) = record(&repo(&base()), &[HELPER]);
    // Added at the end, and in front of everything (every id moves).
    let mut files = base();
    files.push(("docs/notes.ts", "export const n = 1;\n"));
    assert!(replayed(&t, &repo(&files), &[HELPER]).is_some());
    let mut files = base();
    files.insert(0, ("a/first.ts", "export const n = 1;\n"));
    let out = replayed(&t, &repo(&files), &[HELPER]).expect("replays");
    assert!(out.contains("dep pkg/a.test.ts:2"), "{out}");
}

#[test]
fn a_comment_in_a_caller_changes_nothing() {
    let (_, t) = record(&repo(&base()), &[HELPER]);
    let mut files = base();
    let edited = format!("{A}\n// a comment at the end\n");
    files[1].1 = Box::leak(edited.into_boxed_str());
    assert!(replayed(&t, &repo(&files), &[HELPER]).is_some());
}

#[test]
fn a_changed_argument_of_a_caller_is_bound_again() {
    let (_, t) = record(&repo(&base()), &[HELPER]);
    let mut files = base();
    files[1].1 = "import { setupEndpoint } from '@scope/helpers';\nconst { client } = \
                  setupEndpoint({ extraRoutes: ['svc-b'] });\n";
    assert_eq!(replayed(&t, &repo(&files), &[HELPER]), None);
}

#[test]
fn a_call_that_moved_to_another_line_is_bound_again() {
    // The line is part of the result (where the supplier passes the value).
    let (_, t) = record(&repo(&base()), &[HELPER]);
    let mut files = base();
    files[1].1 = "import { setupEndpoint } from '@scope/helpers';\n\nconst { client } = \
                  setupEndpoint({ extraRoutes: ['svc-a'] });\n";
    let r = repo(&files);
    assert_eq!(replayed(&t, &r, &[HELPER]), None);
    let (b, _) = record(&r, &[HELPER]);
    assert!(
        show::bound(&b, &|f| r.path(f)).contains("dep pkg/a.test.ts:3"),
        "{b:?}"
    );
}

#[test]
fn a_new_caller_and_a_deleted_caller_are_bound_again() {
    let (_, t) = record(&repo(&base()), &[HELPER]);
    let mut files = base();
    files.push((
        "pkg/c.test.ts",
        "import { setupEndpoint } from '@scope/helpers';\nsetupEndpoint({ extraRoutes: ['svc-b'] \
         });\n",
    ));
    let more = repo(&files);
    assert_eq!(replayed(&t, &more, &[HELPER]), None);
    // And back: the run recorded with three callers does not apply to two.
    let (_, t3) = record(&more, &[HELPER]);
    assert_eq!(replayed(&t3, &repo(&base()), &[HELPER]), None);
    let mut files = base();
    files.remove(2);
    assert_eq!(replayed(&t, &repo(&files), &[HELPER]), None);
}

#[test]
fn a_changed_callee_is_bound_again() {
    let (_, t) = record(&repo(&base()), &[HELPER]);
    let mut files = base();
    let edited = ROUTES.replace("services/svc-a/src/routes", "services/svc-a/src/other");
    files[0].1 = Box::leak(edited.into_boxed_str());
    assert_eq!(replayed(&t, &repo(&files), &[HELPER]), None);
}

#[test]
fn other_roots_and_another_order_of_files_are_bound_again() {
    let r = repo(&base());
    let (_, t) = record(&r, &[HELPER]);
    assert_eq!(replayed(&t, &r, &[HELPER, "pkg/a.test.ts"]), None);
    assert_eq!(replayed(&t, &r, &[]), None);
    // The same files with other ids relative to each other: the run binds and sorts
    // in the order of the ids.
    let mut files = base();
    files.swap(1, 2);
    assert_eq!(replayed(&t, &repo(&files), &[HELPER]), None);
}

#[test]
fn a_changed_working_directory_of_the_tests_is_bound_again() {
    let mut r = super::harness_repo();
    let roots: Vec<String> = (0..r.files.len() as u32)
        .filter(|f| r.facts(*f).is_some_and(|x| !x.sinks.is_empty()))
        .map(|f| r.path(f))
        .collect();
    let roots: Vec<&str> = roots.iter().map(String::as_str).collect();
    assert!(!roots.is_empty());
    let (_, t) = record(&r, &roots);
    assert!(replayed(&t, &r, &roots).is_some());
    r.cwds.push("services/another");
    assert_eq!(replayed(&t, &r, &roots), None);
}

#[test]
fn a_damaged_trace_is_no_trace() {
    let (_, t) = record(&repo(&base()), &[HELPER]);
    let bytes = t.to_bytes();
    for cut in [0, 7, 8, bytes.len() / 2, bytes.len() - 1] {
        assert_eq!(Trace::from_bytes(&bytes[..cut]), None, "cut at {cut}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(Trace::from_bytes(&longer), None);
}

#[test]
fn facts_have_one_digest() {
    let r = repo(&base());
    let f = r.facts(0);
    let d = trace::facts_digest(f.as_deref());
    assert_ne!(d, 0);
    assert_eq!(d, trace::facts_digest(r.facts(0).as_deref()));
    assert_ne!(d, trace::facts_digest(r.facts(1).as_deref()));
    assert_eq!(trace::facts_digest(None), 0);
}

/// Edits applied one after the other in every order of a few steps: after each,
/// either the last recorded run replays to what binding gives, or it is
/// recorded again.
#[test]
fn sequences_of_edits_replay_or_rebind() {
    type Edit = fn(&mut Vec<(&'static str, &'static str)>);
    let edits: [(&str, Edit); 8] = [
        ("comment in a caller", |f| {
            let i = f.iter().position(|x| x.0 == "pkg/a.test.ts");
            if let Some(i) = i {
                f[i].1 = Box::leak(format!("{}// more\n", f[i].1).into_boxed_str());
            }
        }),
        ("argument of a caller", |f| {
            if let Some(i) = f.iter().position(|x| x.0 == "pkg/b.test.ts") {
                f[i].1 = "import { setupEndpoint } from '@scope/helpers';\nsetupEndpoint({ \
                          extraRoutes: ['svc-b'] });\n";
            }
        }),
        ("a new caller", |f| {
            if !f.iter().any(|x| x.0 == "pkg/c.test.ts") {
                f.push((
                    "pkg/c.test.ts",
                    "import { loadRoutes } from \
                     '@scope/helpers';\nloadRoutes([process.argv[2]]);\n",
                ));
            }
        }),
        ("a caller deleted", |f| f.retain(|x| x.0 != "pkg/a.test.ts")),
        ("the callee", |f| {
            let edited = f[0].1.replace(
                "'svc-b': 'services/svc-b/src/routes.ts'",
                "'svc-b': 'services/svc-b/src/v2'",
            );
            f[0].1 = Box::leak(edited.into_boxed_str());
        }),
        ("an unrelated file", |f| {
            if !f.iter().any(|x| x.0 == "lib/unrelated.ts") {
                f.push(("lib/unrelated.ts", "export const z = 1;\n"));
            }
        }),
        ("a file in front", |f| {
            if !f.iter().any(|x| x.0 == "a/first.ts") {
                f.insert(1, ("a/first.ts", "export const n = 1;\n"));
            }
        }),
        ("the caller comes back", |f| {
            if !f.iter().any(|x| x.0 == "pkg/a.test.ts") {
                f.push(("pkg/a.test.ts", A));
            }
        }),
    ];
    let (mut replays, mut rebinds) = (0, 0);
    // Every ordered choice of three edits out of the eight.
    for a in 0..edits.len() {
        for b in 0..edits.len() {
            for c in 0..edits.len() {
                if a == b || a == c || b == c {
                    continue;
                }
                let mut files = base();
                let (_, mut t) = record(&repo(&files), &[HELPER]);
                for step in [a, b, c] {
                    (edits[step].1)(&mut files);
                    let r = repo(&files);
                    // `replayed` checks the answer against binding from nothing.
                    match replayed(&t, &r, &[HELPER]) {
                        Some(_) => replays += 1,
                        None => {
                            rebinds += 1;
                            t = record(&r, &[HELPER]).1;
                        }
                    }
                }
            }
        }
    }
    assert!(
        replays > 150 && rebinds > 150,
        "{replays} replays, {rebinds} rebinds"
    );
}
