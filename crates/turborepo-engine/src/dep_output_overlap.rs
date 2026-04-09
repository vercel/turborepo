//! Detects tasks whose inputs overlap with their dependencies' outputs.
//!
//! When task B depends on task A and B's `inputs` include a file that A
//! declares as an `output`, turbo must defer B's file hashing until after
//! A has executed, so the output files exist on disk and hash correctly.

use std::collections::HashSet;

use turborepo_task_id::TaskId;
use turborepo_types::TaskDefinition;
use wax::Program as _;

use crate::{Built, Engine, TaskNode};

/// A detected overlap between a task's inputs and a dependency's outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepOutputOverlap {
    /// The task whose inputs overlap with a dependency's outputs.
    pub task_id: TaskId<'static>,
    /// The dependency task that produces the overlapping outputs.
    pub dep_task_id: TaskId<'static>,
    /// The patterns that overlap (from the dependency's outputs).
    pub overlapping_patterns: Vec<String>,
}

/// Analyze the engine's task graph to detect tasks whose inputs include
/// files produced by their dependencies' outputs.
///
/// Only considers same-package dependencies since cross-package tasks
/// write to different package directories.
pub fn detect_dep_output_overlaps(engine: &Engine<Built, TaskDefinition>) -> Vec<DepOutputOverlap> {
    let mut overlaps = Vec::new();

    for task_id in engine.task_ids() {
        let task_def = match engine.task_definition(task_id) {
            Some(def) => def,
            None => continue,
        };

        let input_globs = &task_def.inputs.globs;
        if input_globs.is_empty() {
            continue;
        }

        // Walk all transitive dependencies to catch chains like
        // prepare -> transform -> build.
        let transitive_deps = engine.transitive_dependencies(task_id);

        for dep_node in transitive_deps {
            let TaskNode::Task(dep_id) = dep_node else {
                continue;
            };

            // Only same-package deps can have overlapping paths.
            if dep_id.package() != task_id.package() {
                continue;
            }

            let dep_def = match engine.task_definition(dep_id) {
                Some(def) => def,
                None => continue,
            };

            let dep_outputs = &dep_def.outputs.inclusions;
            if dep_outputs.is_empty() {
                continue;
            }

            let matching = find_overlapping_patterns(input_globs, dep_outputs);
            if !matching.is_empty() {
                overlaps.push(DepOutputOverlap {
                    task_id: task_id.clone(),
                    dep_task_id: dep_id.clone(),
                    overlapping_patterns: matching,
                });
            }
        }
    }

    // Sort for deterministic output.
    overlaps.sort_by(|a, b| {
        a.task_id
            .cmp(&b.task_id)
            .then_with(|| a.dep_task_id.cmp(&b.dep_task_id))
    });

    overlaps
}

/// Build a set of task IDs that need deferred file hashing.
pub fn deferred_hash_tasks(overlaps: &[DepOutputOverlap]) -> HashSet<TaskId<'static>> {
    overlaps.iter().map(|o| o.task_id.clone()).collect()
}

/// A config-level overlap, deduplicated by task name (package stripped).
/// Represents a pattern in the turbo.json task definitions, not a specific
/// package instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOverlap {
    /// The task name whose inputs overlap (e.g. "bundle").
    pub task_name: String,
    /// The dependency task name that produces the overlapping outputs (e.g.
    /// "fetch").
    pub dep_task_name: String,
    /// The patterns that overlap.
    pub overlapping_patterns: Vec<String>,
}

/// Deduplicate per-task overlaps into config-level overlaps by stripping
/// the package prefix. This produces one warning per turbo.json task
/// pattern regardless of how many packages inherit it.
pub fn config_level_overlaps(overlaps: &[DepOutputOverlap]) -> Vec<ConfigOverlap> {
    use std::collections::BTreeSet;

    // Key: (task_name, dep_task_name) → set of patterns
    let mut seen: std::collections::BTreeMap<(String, String), BTreeSet<String>> =
        std::collections::BTreeMap::new();

    for overlap in overlaps {
        let key = (
            overlap.task_id.task().to_string(),
            overlap.dep_task_id.task().to_string(),
        );
        let patterns = seen.entry(key).or_default();
        for pat in &overlap.overlapping_patterns {
            patterns.insert(pat.clone());
        }
    }

    seen.into_iter()
        .map(|((task_name, dep_task_name), patterns)| ConfigOverlap {
            task_name,
            dep_task_name,
            overlapping_patterns: patterns.into_iter().collect(),
        })
        .collect()
}

/// Check if any input glob matches any output pattern.
///
/// Uses exact string matching first, then glob-based matching where one
/// pattern could match the other (e.g., output `dist/**` matches input
/// `dist/bundle.js`).
fn find_overlapping_patterns(input_globs: &[String], output_patterns: &[String]) -> Vec<String> {
    let mut matched = Vec::new();

    for output in output_patterns {
        for input in input_globs {
            // Skip negated inputs (exclusions).
            if input.starts_with('!') {
                continue;
            }

            // Exact match.
            if input == output {
                if !matched.contains(output) {
                    matched.push(output.clone());
                }
                continue;
            }

            // Try: does the output glob match the input as a literal path?
            // e.g., output "dist/**" matches input "dist/bundle.js"
            if let Ok(output_glob) = wax::Glob::new(output)
                && output_glob.is_match(input.as_str())
                && !matched.contains(output)
            {
                matched.push(output.clone());
                continue;
            }

            // Try: does the input glob match the output as a literal path?
            // e.g., input "generated.*" matches output "generated.txt"
            if let Ok(input_glob) = wax::Glob::new(input)
                && input_glob.is_match(output.as_str())
                && !matched.contains(output)
            {
                matched.push(output.clone());
                continue;
            }

            // Directory containment: input "dir" should match output "dir/**"
            // because turbo treats a bare directory name as "everything in it".
            let input_dir = input.trim_end_matches('/');
            let output_dir = output.trim_end_matches("/**").trim_end_matches("/*");
            if (input_dir == output_dir
                || output.starts_with(&format!("{input_dir}/"))
                || input.starts_with(&format!("{output_dir}/")))
                && !matched.contains(output)
            {
                matched.push(output.clone());
            }
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use turborepo_types::{TaskInputs, TaskOutputs};

    use super::*;

    fn make_task_def(
        inputs: Vec<&str>,
        outputs: Vec<&str>,
        task_deps: Vec<&str>,
    ) -> TaskDefinition {
        TaskDefinition {
            inputs: TaskInputs {
                globs: inputs.into_iter().map(|s| s.to_string()).collect(),
                default: false,
            },
            outputs: TaskOutputs {
                inclusions: outputs.into_iter().map(|s| s.to_string()).collect(),
                exclusions: vec![],
            },
            task_dependencies: task_deps
                .into_iter()
                .map(|s| {
                    turborepo_errors::Spanned::new(
                        turborepo_task_id::TaskName::from(s.to_string()).into_owned(),
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_exact_match_detection() {
        let overlaps =
            find_overlapping_patterns(&["generated.txt".into()], &["generated.txt".into()]);
        assert_eq!(overlaps, vec!["generated.txt"]);
    }

    #[test]
    fn test_glob_output_matches_literal_input() {
        let overlaps = find_overlapping_patterns(&["dist/bundle.js".into()], &["dist/**".into()]);
        assert_eq!(overlaps, vec!["dist/**"]);
    }

    #[test]
    fn test_glob_input_matches_literal_output() {
        let overlaps =
            find_overlapping_patterns(&["generated.*".into()], &["generated.txt".into()]);
        assert_eq!(overlaps, vec!["generated.txt"]);
    }

    #[test]
    fn test_no_overlap() {
        let overlaps = find_overlapping_patterns(&["src/**".into()], &["dist/**".into()]);
        assert!(overlaps.is_empty());
    }

    #[test]
    fn test_negated_input_ignored() {
        let overlaps =
            find_overlapping_patterns(&["!generated.txt".into()], &["generated.txt".into()]);
        assert!(overlaps.is_empty());
    }

    #[test]
    fn test_directory_input_matches_glob_output() {
        // turbo treats bare "dir" as "everything in dir"
        let overlaps =
            find_overlapping_patterns(&["target-schemas".into()], &["target-schemas/**".into()]);
        assert_eq!(overlaps, vec!["target-schemas/**"]);
    }

    #[test]
    fn test_directory_output_matches_subpath_input() {
        let overlaps = find_overlapping_patterns(&["dist/bundle.js".into()], &["dist/**".into()]);
        assert_eq!(overlaps, vec!["dist/**"]);
    }

    // --- Engine-level tests for detect_dep_output_overlaps ---

    fn make_engine(
        tasks: &[(TaskId<'static>, TaskDefinition)],
        edges: &[(TaskId<'static>, TaskId<'static>)],
    ) -> Engine<crate::Built, TaskDefinition> {
        use crate::Building;
        let mut engine: Engine<Building, TaskDefinition> = Engine::new();
        let mut indices = std::collections::HashMap::new();
        for (task_id, def) in tasks {
            let idx = engine.get_index(task_id);
            engine.add_definition(task_id.clone(), def.clone());
            indices.insert(task_id.clone(), idx);
        }
        for (from, to) in edges {
            let from_idx = indices[from];
            let to_idx = indices[to];
            // Edge points dependent → dependency (Outgoing = dependencies)
            engine.task_graph_mut().add_edge(from_idx, to_idx, ());
        }
        // Connect leaf nodes to root
        for (task_id, _) in tasks {
            let has_deps = edges.iter().any(|(_, to)| to == task_id);
            if !has_deps {
                engine.connect_to_root(task_id);
            }
        }
        engine.seal()
    }

    fn tid(pkg: &str, task: &str) -> TaskId<'static> {
        TaskId::new(pkg, task).into_owned()
    }

    #[test]
    fn test_engine_simple_overlap() {
        let prepare = tid("pkg", "prepare");
        let build = tid("pkg", "build");

        let engine = make_engine(
            &[
                (
                    prepare.clone(),
                    make_task_def(vec!["package.json"], vec!["generated.txt"], vec![]),
                ),
                (
                    build.clone(),
                    make_task_def(vec!["generated.txt"], vec![], vec!["prepare"]),
                ),
            ],
            &[(build.clone(), prepare.clone())],
        );

        let overlaps = detect_dep_output_overlaps(&engine);
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].task_id, build);
        assert_eq!(overlaps[0].dep_task_id, prepare);
        assert_eq!(overlaps[0].overlapping_patterns, vec!["generated.txt"]);
    }

    #[test]
    fn test_engine_transitive_overlap() {
        // prepare -> transform -> build
        // prepare outputs generated.txt
        // build inputs generated.txt (transitive dep, not direct)
        let prepare = tid("pkg", "prepare");
        let transform = tid("pkg", "transform");
        let build = tid("pkg", "build");

        let engine = make_engine(
            &[
                (
                    prepare.clone(),
                    make_task_def(vec!["package.json"], vec!["generated.txt"], vec![]),
                ),
                (
                    transform.clone(),
                    make_task_def(
                        vec!["generated.txt"],
                        vec!["transformed.txt"],
                        vec!["prepare"],
                    ),
                ),
                (
                    build.clone(),
                    make_task_def(
                        vec!["generated.txt", "transformed.txt"],
                        vec![],
                        vec!["transform"],
                    ),
                ),
            ],
            &[
                (transform.clone(), prepare.clone()),
                (build.clone(), transform.clone()),
            ],
        );

        let overlaps = detect_dep_output_overlaps(&engine);

        // build should detect overlap with prepare (generated.txt) and transform
        // (transformed.txt) transform should detect overlap with prepare
        // (generated.txt)
        let build_overlaps: Vec<_> = overlaps.iter().filter(|o| o.task_id == build).collect();
        let transform_overlaps: Vec<_> =
            overlaps.iter().filter(|o| o.task_id == transform).collect();

        assert_eq!(
            build_overlaps.len(),
            2,
            "build should overlap with both prepare and transform"
        );
        assert_eq!(
            transform_overlaps.len(),
            1,
            "transform should overlap with prepare"
        );

        // Check specific patterns
        let build_prepare = build_overlaps
            .iter()
            .find(|o| o.dep_task_id == prepare)
            .unwrap();
        assert_eq!(build_prepare.overlapping_patterns, vec!["generated.txt"]);

        let build_transform = build_overlaps
            .iter()
            .find(|o| o.dep_task_id == transform)
            .unwrap();
        assert_eq!(
            build_transform.overlapping_patterns,
            vec!["transformed.txt"]
        );
    }

    #[test]
    fn test_engine_no_overlap_different_packages() {
        // Cross-package deps should NOT be detected (different package dirs)
        let lib_build = tid("lib", "build");
        let app_build = tid("app", "build");

        let engine = make_engine(
            &[
                (
                    lib_build.clone(),
                    make_task_def(vec!["src/**"], vec!["dist/**"], vec![]),
                ),
                (
                    app_build.clone(),
                    make_task_def(vec!["dist/**"], vec![], vec![]),
                ),
            ],
            &[(app_build.clone(), lib_build.clone())],
        );

        let overlaps = detect_dep_output_overlaps(&engine);
        assert!(
            overlaps.is_empty(),
            "cross-package deps should not trigger overlap detection"
        );
    }

    #[test]
    fn test_engine_no_overlap_unrelated_patterns() {
        let prepare = tid("pkg", "prepare");
        let build = tid("pkg", "build");

        let engine = make_engine(
            &[
                (
                    prepare.clone(),
                    make_task_def(vec!["package.json"], vec!["generated.txt"], vec![]),
                ),
                (
                    build.clone(),
                    make_task_def(vec!["src/**"], vec![], vec!["prepare"]),
                ),
            ],
            &[(build.clone(), prepare.clone())],
        );

        let overlaps = detect_dep_output_overlaps(&engine);
        assert!(
            overlaps.is_empty(),
            "non-overlapping patterns should not trigger detection"
        );
    }

    #[test]
    fn test_engine_directory_containment() {
        // schemas outputs "target-schemas/**", contain inputs "target-schemas" (bare
        // dir)
        let schemas = tid("pkg", "schemas");
        let contain = tid("pkg", "contain");

        let engine = make_engine(
            &[
                (
                    schemas.clone(),
                    make_task_def(vec!["src/**"], vec!["target-schemas/**"], vec![]),
                ),
                (
                    contain.clone(),
                    make_task_def(vec!["target-schemas"], vec![], vec!["schemas"]),
                ),
            ],
            &[(contain.clone(), schemas.clone())],
        );

        let overlaps = detect_dep_output_overlaps(&engine);
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].task_id, contain);
        assert_eq!(overlaps[0].dep_task_id, schemas);
        assert_eq!(overlaps[0].overlapping_patterns, vec!["target-schemas/**"]);
    }

    // --- Config-level deduplication tests ---

    #[test]
    fn test_config_level_deduplicates_across_packages() {
        // Same task pattern in multiple packages should produce one config overlap
        let overlaps = vec![
            DepOutputOverlap {
                task_id: tid("app-a", "bundle"),
                dep_task_id: tid("app-a", "fetch"),
                overlapping_patterns: vec!["target-fetched/**".into()],
            },
            DepOutputOverlap {
                task_id: tid("app-b", "bundle"),
                dep_task_id: tid("app-b", "fetch"),
                overlapping_patterns: vec!["target-fetched/**".into()],
            },
            DepOutputOverlap {
                task_id: tid("lib-c", "bundle"),
                dep_task_id: tid("lib-c", "fetch"),
                overlapping_patterns: vec!["target-fetched/**".into()],
            },
        ];

        let config = config_level_overlaps(&overlaps);
        assert_eq!(config.len(), 1, "should deduplicate to one config overlap");
        assert_eq!(config[0].task_name, "bundle");
        assert_eq!(config[0].dep_task_name, "fetch");
        assert_eq!(config[0].overlapping_patterns, vec!["target-fetched/**"]);
    }

    #[test]
    fn test_config_level_preserves_distinct_patterns() {
        // Different task pairs should remain separate
        let overlaps = vec![
            DepOutputOverlap {
                task_id: tid("pkg", "bundle"),
                dep_task_id: tid("pkg", "fetch"),
                overlapping_patterns: vec!["target-fetched/**".into()],
            },
            DepOutputOverlap {
                task_id: tid("pkg", "contain"),
                dep_task_id: tid("pkg", "schemas"),
                overlapping_patterns: vec!["target-schemas/**".into()],
            },
        ];

        let config = config_level_overlaps(&overlaps);
        assert_eq!(config.len(), 2);
        assert_eq!(config[0].task_name, "bundle");
        assert_eq!(config[0].dep_task_name, "fetch");
        assert_eq!(config[1].task_name, "contain");
        assert_eq!(config[1].dep_task_name, "schemas");
    }

    #[test]
    fn test_config_level_merges_patterns_from_different_packages() {
        // If different packages contribute different patterns for the same
        // task pair, they should be merged
        let overlaps = vec![
            DepOutputOverlap {
                task_id: tid("app-a", "bundle"),
                dep_task_id: tid("app-a", "fetch"),
                overlapping_patterns: vec!["target-fetched/**".into()],
            },
            DepOutputOverlap {
                task_id: tid("app-b", "bundle"),
                dep_task_id: tid("app-b", "fetch"),
                overlapping_patterns: vec!["target-fetched/**".into(), "data/**".into()],
            },
        ];

        let config = config_level_overlaps(&overlaps);
        assert_eq!(config.len(), 1);
        assert_eq!(
            config[0].overlapping_patterns,
            vec!["data/**", "target-fetched/**"]
        );
    }
}
