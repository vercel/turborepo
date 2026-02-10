//! Benchmarks for turborepo-engine operations, focused on subgraph creation
//! which is on the hot path in watch mode.

use std::collections::HashSet;

use codspeed_criterion_compat::{Criterion, black_box, criterion_group, criterion_main};
use turborepo_engine::{Building, Engine, TaskInfo};
use turborepo_repository::package_graph::PackageName;
use turborepo_task_id::TaskId;

/// Build a linear dependency chain: pkg0:build <- pkg1:build <- ... <- pkgN:build
/// Each package also has a :test task with no cross-package dependencies.
fn build_linear_engine(num_packages: usize) -> Engine {
    let mut engine: Engine<Building, TaskInfo> = Engine::new();
    let mut prev_build_idx = None;

    for i in 0..num_packages {
        let pkg = format!("pkg{i}");
        let build_task = TaskId::from_static(pkg.clone(), "build".into());
        let test_task = TaskId::from_static(pkg, "test".into());

        let build_idx = engine.get_index(&build_task);
        engine.get_index(&test_task);
        engine.add_definition(build_task, TaskInfo::default());
        engine.add_definition(test_task, TaskInfo::default());

        if let Some(prev) = prev_build_idx {
            engine.task_graph_mut().add_edge(build_idx, prev, ());
        }
        prev_build_idx = Some(build_idx);
    }

    engine.seal()
}

/// Build a fan-out graph: pkg0:build is depended on by all other packages' build
/// tasks. Each package also has a :test task.
fn build_fan_out_engine(num_packages: usize) -> Engine {
    let mut engine: Engine<Building, TaskInfo> = Engine::new();

    let root_build = TaskId::from_static("pkg0".into(), "build".into());
    let root_test = TaskId::from_static("pkg0".into(), "test".into());
    let root_idx = engine.get_index(&root_build);
    engine.get_index(&root_test);
    engine.add_definition(root_build, TaskInfo::default());
    engine.add_definition(root_test, TaskInfo::default());

    for i in 1..num_packages {
        let pkg = format!("pkg{i}");
        let build_task = TaskId::from_static(pkg.clone(), "build".into());
        let test_task = TaskId::from_static(pkg, "test".into());

        let build_idx = engine.get_index(&build_task);
        engine.get_index(&test_task);
        engine.add_definition(build_task, TaskInfo::default());
        engine.add_definition(test_task, TaskInfo::default());

        engine.task_graph_mut().add_edge(build_idx, root_idx, ());
    }

    engine.seal()
}

/// Build a diamond/layered graph simulating a realistic monorepo:
/// Layer 0 (core libs) <- Layer 1 (mid-level) <- Layer 2 (apps)
/// Each layer package depends on all packages in the previous layer.
fn build_diamond_engine(packages_per_layer: usize, num_layers: usize) -> Engine {
    let mut engine: Engine<Building, TaskInfo> = Engine::new();
    let mut prev_layer_indices = Vec::new();

    for layer in 0..num_layers {
        let mut current_layer_indices = Vec::new();

        for i in 0..packages_per_layer {
            let pkg = format!("l{layer}p{i}");
            let build_task = TaskId::from_static(pkg.clone(), "build".into());
            let test_task = TaskId::from_static(pkg.clone(), "test".into());
            let lint_task = TaskId::from_static(pkg, "lint".into());

            let build_idx = engine.get_index(&build_task);
            engine.get_index(&test_task);
            engine.get_index(&lint_task);
            engine.add_definition(build_task, TaskInfo::default());
            engine.add_definition(test_task, TaskInfo::default());
            engine.add_definition(lint_task, TaskInfo::default());

            // Depend on all packages in the previous layer
            for &prev_idx in &prev_layer_indices {
                engine.task_graph_mut().add_edge(build_idx, prev_idx, ());
            }

            current_layer_indices.push(build_idx);
        }

        prev_layer_indices = current_layer_indices;
    }

    engine.seal()
}

fn bench_subgraph_linear_small(c: &mut Criterion) {
    let engine = build_linear_engine(20);
    let changed: HashSet<_> = [PackageName::from("pkg0")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_linear_20", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

fn bench_subgraph_linear_medium(c: &mut Criterion) {
    let engine = build_linear_engine(100);
    let changed: HashSet<_> = [PackageName::from("pkg0")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_linear_100", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

fn bench_subgraph_linear_large(c: &mut Criterion) {
    let engine = build_linear_engine(500);
    let changed: HashSet<_> = [PackageName::from("pkg0")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_linear_500", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

fn bench_subgraph_fan_out_medium(c: &mut Criterion) {
    let engine = build_fan_out_engine(100);
    let changed: HashSet<_> = [PackageName::from("pkg0")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_fan_out_100", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

fn bench_subgraph_fan_out_large(c: &mut Criterion) {
    let engine = build_fan_out_engine(500);
    let changed: HashSet<_> = [PackageName::from("pkg0")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_fan_out_500", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

fn bench_subgraph_diamond(c: &mut Criterion) {
    // 5 layers x 20 packages = 100 packages, 300 tasks, realistic monorepo shape
    let engine = build_diamond_engine(20, 5);
    let changed: HashSet<_> = [PackageName::from("l0p0")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_diamond_100pkg", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

fn bench_subgraph_diamond_large(c: &mut Criterion) {
    // 5 layers x 50 packages = 250 packages, 750 tasks
    let engine = build_diamond_engine(50, 5);
    let changed: HashSet<_> = [PackageName::from("l0p0")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_diamond_250pkg", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

fn bench_subgraph_leaf_change(c: &mut Criterion) {
    // Benchmark when the changed package is a leaf (no dependents),
    // which should be the fastest case.
    let engine = build_linear_engine(500);
    let changed: HashSet<_> = [PackageName::from("pkg499")].into_iter().collect();

    c.bench_function("create_engine_for_subgraph_leaf_change_500", |b| {
        b.iter(|| black_box(engine.create_engine_for_subgraph(black_box(&changed))))
    });
}

criterion_group!(
    benches,
    bench_subgraph_linear_small,
    bench_subgraph_linear_medium,
    bench_subgraph_linear_large,
    bench_subgraph_fan_out_medium,
    bench_subgraph_fan_out_large,
    bench_subgraph_diamond,
    bench_subgraph_diamond_large,
    bench_subgraph_leaf_change,
);

criterion_main!(benches);
