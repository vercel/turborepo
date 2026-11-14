//! Registry-wide recognizer properties.
//!
//! Every test iterates `registry().recognizers()`: a newly registered
//! recognizer, hand written or generated from a declarative tool spec, is
//! covered without editing this file. Commands are generated per declared
//! program with a seeded RNG (see `testkit` for the knobs) and run
//! against a small synthetic repository in a temporary directory.
//!
//! | test | invariant |
//! |---|---|
//! | `random_argv_never_panics` | arbitrary flags, subcommands, paths, unicode, control characters and empty words never panic |
//! | `regression_characters_never_panic` | vertical tab, form feed, NBSP and U+2028 inside any word never panic |
//! | `huge_args_never_panic_and_stay_fast` | a 256 KiB word or 3000 words: no panic, within the per-call time budget |
//! | `output_is_deterministic` | the same command twice gives an identical `TaskSpec` and report |
//! | `paths_are_absolute_and_inside_the_repo` | every emitted path is absolute and normalized, and under the repo root unless flagged `outside_base` |
//! | `outside_paths_are_absolute_and_normalized` | commands that name paths outside the repo still produce absolute normalized paths |
//! | `full_never_has_unknown_command` | `Recognition::Full` never comes with an `unknown_command` uncertainty |
//! | `recognizers_claim_only_declared_programs` | `matches` accepts only programs listed in `programs()` |
//! | `calls_are_time_bounded` | no single call exceeds the time budget |
//! | `every_recognizer_is_reachable` | each recognizer wins dispatch for at least one generated command (not shadowed) |
//! | `specs_are_in_the_registry` | declarative tool specs are registered, so all of the above covers them |

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[macro_use]
mod testkit;

use std::collections::BTreeSet;

use testkit::{Gen, Mode, Prop, SyntheticRepo, assert_no_failures, iters, run_registry};
use turborepo_trace_tasks::registry::registry;

fn only(prop: Prop, mode: Mode, per_program: usize) {
    let (fails, n) = run_registry(mode, per_program);
    let fails: Vec<_> = fails.into_iter().filter(|f| f.prop == prop).collect();
    assert_no_failures(&format!("{prop:?} ({mode:?})"), &fails, n);
}

#[test]
fn registry_is_not_empty_and_ids_are_unique() {
    let rs = registry().recognizers();
    assert!(rs.len() > 20, "only {} recognizers registered", rs.len());
    let mut seen = BTreeSet::new();
    let dups: Vec<&str> = rs
        .iter()
        .map(|r| r.id())
        .filter(|id| !seen.insert(*id))
        .collect();
    assert!(
        dups.is_empty(),
        "duplicate recognizer ids (coverage per id would merge them): {dups:?}"
    );
    for r in rs {
        assert!(
            !r.programs().is_empty(),
            "`{}` declares no programs",
            r.id()
        );
    }
}

/// Declarative tool specs are recognizers in the registry, so every property
/// covers them.
#[test]
fn specs_are_in_the_registry() {
    let registered: BTreeSet<&str> = registry().recognizers().iter().map(|r| r.id()).collect();
    let specs = turborepo_trace_tasks::specs::recognizers();
    assert!(!specs.is_empty(), "no declarative specs");
    for s in specs {
        assert!(
            registered.contains(s.id()),
            "spec `{}` is not in the registry",
            s.id()
        );
    }
}

#[test]
fn random_argv_never_panics() {
    only(Prop::NoPanic, Mode::Confined, iters(12));
}

#[test]
fn regression_characters_never_panic() {
    let (fails, n) = run_registry(Mode::Regression, iters(6));
    let fails: Vec<_> = fails
        .into_iter()
        .filter(|f| {
            matches!(
                f.prop,
                Prop::NoPanic | Prop::BoundedTime | Prop::Deterministic
            )
        })
        .collect();
    assert_no_failures("regression characters", &fails, n);
}

#[test]
fn huge_args_never_panic_and_stay_fast() {
    let (fails, n) = run_registry(Mode::Huge, iters(12).div_ceil(6));
    let fails: Vec<_> = fails
        .into_iter()
        .filter(|f| matches!(f.prop, Prop::NoPanic | Prop::BoundedTime))
        .collect();
    assert_no_failures("huge args", &fails, n);
}

#[test]
fn output_is_deterministic() {
    only(Prop::Deterministic, Mode::Confined, iters(12));
}

#[test]
fn paths_are_absolute_and_inside_the_repo() {
    let (fails, n) = run_registry(Mode::Confined, iters(12));
    let fails: Vec<_> = fails
        .into_iter()
        .filter(|f| matches!(f.prop, Prop::AbsoluteNormalized | Prop::InsideRepo))
        .collect();
    assert_no_failures("paths", &fails, n);
}

#[test]
fn outside_paths_are_absolute_and_normalized() {
    let (fails, n) = run_registry(Mode::Outside, iters(12));
    let fails: Vec<_> = fails
        .into_iter()
        .filter(|f| matches!(f.prop, Prop::AbsoluteNormalized | Prop::NoPanic))
        .collect();
    assert_no_failures("outside paths", &fails, n);
}

#[test]
fn full_never_has_unknown_command() {
    only(Prop::FullNotUnknown, Mode::Confined, iters(12));
}

#[test]
fn recognizers_claim_only_declared_programs() {
    only(Prop::DeclaredPrograms, Mode::Confined, iters(12));
}

#[test]
fn calls_are_time_bounded() {
    only(Prop::BoundedTime, Mode::Confined, iters(12));
}

/// A recognizer that never wins dispatch is dead weight (or shadowed by an
/// earlier one).
#[test]
fn every_recognizer_is_reachable() {
    let repo = SyntheticRepo::new();
    let mut g = Gen::new(&repo, testkit::seed());
    let mut unreachable = Vec::new();
    for r in registry().recognizers() {
        let mut hit = false;
        'programs: for program in r.programs() {
            for _ in 0..iters(12) * 4 {
                let case = g.case(program, Mode::Confined);
                if registry()
                    .find(&case.cmd)
                    .is_some_and(|w| std::ptr::addr_eq(w, *r))
                {
                    hit = true;
                    break 'programs;
                }
            }
        }
        if !hit {
            unreachable.push(format!("{} (programs {:?})", r.id(), r.programs()));
        }
    }
    assert!(
        unreachable.is_empty(),
        "recognizers that never win dispatch: {unreachable:#?}"
    );
}

/// The regression characters, on their own and glued to flags, for every
/// declared program.
#[test]
fn regression_characters_as_whole_words() {
    let repo = SyntheticRepo::new();
    let mut fails = Vec::new();
    let mut n = 0;
    for r in registry().recognizers() {
        for program in r.programs() {
            for c in testkit::REGRESSION_CHARS {
                for args in [
                    vec![c.to_string()],
                    vec!["-p".to_string(), c.to_string()],
                    vec![format!("--config={c}")],
                    vec!["run".to_string(), c.to_string(), c.to_string()],
                    vec![format!("{c}src/index.ts{c}")],
                    vec![String::new(), c.to_string(), String::new()],
                ] {
                    let mut argv = vec![turborepo_trace_tasks::spec::Word::lit(*program)];
                    argv.extend(args.into_iter().map(turborepo_trace_tasks::spec::Word::lit));
                    let case = testkit::Case {
                        cmd: turborepo_trace_tasks::spec::Command {
                            argv,
                            cwd: repo.root.clone(),
                            text: format!("{program} …"),
                            ..Default::default()
                        },
                        package_root: repo.root.clone(),
                        mode: Mode::Regression,
                    };
                    n += 1;
                    fails.extend(
                        testkit::check_case(*r, &case, &repo)
                            .into_iter()
                            .filter(|f| {
                                matches!(
                                    f.prop,
                                    Prop::NoPanic | Prop::BoundedTime | Prop::Deterministic
                                )
                            }),
                    );
                }
            }
        }
    }
    assert_no_failures("regression characters as whole words", &fails, n);
}

// ---- Known violations (found by the generator; owners outside this test)
// -----------------------

/// Found with `TURBO_TRACE_DEBUG_TESTKIT_SEED=3
/// TURBO_TRACE_DEBUG_TESTKIT_ITERS=40`.
#[test]
#[ignore = "bug in recognizers/codegen/golang.rs: stringer/enumer build the default output with \
            `dir.join(format!(\"{t}{suffix}\"))` instead of `fsx::join`, so a `-type` with `..` or \
            `./` yields a non-normalized output path"]
fn stringer_type_with_dotdot_gives_normalized_output() {
    let repo = SyntheticRepo::new();
    let stringer = registry()
        .recognizers()
        .iter()
        .find(|r| r.id() == "stringer")
        .expect("stringer recognizer");
    for ty in ["src/../../x", "./x"] {
        let case = testkit::Case {
            cmd: turborepo_trace_tasks::spec::Command {
                argv: ["stringer", "-type", ty]
                    .into_iter()
                    .map(turborepo_trace_tasks::spec::Word::lit)
                    .collect(),
                cwd: repo.root.join("apps/web"),
                text: format!("stringer -type {ty}"),
                ..Default::default()
            },
            package_root: repo.root.join("apps/web"),
            mode: Mode::Outside,
        };
        let fails: Vec<_> = testkit::check_case(*stringer, &case, &repo)
            .into_iter()
            .filter(|f| f.prop == Prop::AbsoluteNormalized)
            .collect();
        assert_no_failures("stringer -type", &fails, 1);
    }
}

// ---- The checks themselves catch broken recognizers
// --------------------------------------------

mod broken {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use turborepo_trace_core::UncertaintyKind;
    use turborepo_trace_tasks::{
        registry::{Recognition, RecognizeCx, Recognizer},
        spec::{Command, TaskSpec},
    };

    /// Violates every invariant, depending on its first argument.
    pub struct Broken;

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    impl Recognizer for Broken {
        fn id(&self) -> &'static str {
            "broken"
        }
        fn programs(&self) -> &'static [&'static str] {
            &["broken"]
        }
        fn matches(&self, cmd: &Command) -> bool {
            matches!(cmd.program_name(), Some("broken" | "undeclared"))
        }
        fn recognize(
            &self,
            cmd: &Command,
            cx: &RecognizeCx<'_>,
            out: &mut TaskSpec,
        ) -> Recognition {
            match cmd.args().next() {
                Some("panic") => panic!("boom"),
                Some("relative") => out.input("rel.txt"),
                Some("dotdot") => out.config(cx.repo_root.join("a/../b")),
                Some("outside") => out.output("/outside/x"),
                Some("unknown") => out.uncertain(UncertaintyKind::UnknownCommand, "inner"),
                Some("random") => out.input(
                    cx.repo_root
                        .join(format!("n{}", CALLS.fetch_add(1, Ordering::Relaxed))),
                ),
                _ => {}
            }
            Recognition::Full
        }
    }
}

#[test]
fn checks_catch_a_broken_recognizer() {
    let repo = SyntheticRepo::new();
    let case = |program: &str, arg: &str| testkit::Case {
        cmd: turborepo_trace_tasks::spec::Command {
            argv: vec![
                turborepo_trace_tasks::spec::Word::lit(program),
                turborepo_trace_tasks::spec::Word::lit(arg),
            ],
            cwd: repo.root.clone(),
            text: format!("{program} {arg}"),
            ..Default::default()
        },
        package_root: repo.root.clone(),
        mode: Mode::Confined,
    };
    for (program, arg, prop) in [
        ("broken", "panic", Prop::NoPanic),
        ("broken", "relative", Prop::AbsoluteNormalized),
        ("broken", "dotdot", Prop::AbsoluteNormalized),
        ("broken", "outside", Prop::InsideRepo),
        ("broken", "unknown", Prop::FullNotUnknown),
        ("broken", "random", Prop::Deterministic),
        ("undeclared", "x", Prop::DeclaredPrograms),
    ] {
        let fails = testkit::check_case(&broken::Broken, &case(program, arg), &repo);
        assert!(
            fails
                .iter()
                .any(|f| f.prop == prop && f.recognizer == "broken"),
            "`{program} {arg}` should violate {prop:?}; got {:?}",
            fails
                .iter()
                .map(|f| (f.prop, f.recognizer.clone()))
                .collect::<Vec<_>>()
        );
    }
    // A well-behaved command passes.
    assert!(testkit::check_case(&broken::Broken, &case("broken", "fine"), &repo).is_empty());
}

// ---- Per-tool fixture macro, exercised on existing fixtures
// ------------------------------------

recognizer_fixture!(
    fixture_macro_inline,
    "fixtures/tasks/recognizers/ts",
    [
        "tsc -p tsconfig.json",
        "npx tsc --noEmit && echo done",
        "unknown-tool --x"
    ]
);
