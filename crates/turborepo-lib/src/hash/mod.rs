//! hash module
//!
//! This module contains the hash functions used by turborepo for certain
//! data-types. This is managed using capnproto for deterministic hashing across
//! languages and platforms.

mod traits;

use std::collections::HashMap;

use capnp::message::{Builder, HeapAllocator};
pub use traits::TurboHash;
use turborepo_env::{EnvironmentVariablePairs, ResolvedEnvMode};

use crate::{cli::EnvMode, task_graph::TaskOutputs};

mod proto_capnp {
    use turborepo_env::ResolvedEnvMode;

    use crate::cli::EnvMode;

    include!(concat!(env!("OUT_DIR"), "/src/hash/proto_capnp.rs"));

    impl From<EnvMode> for global_hashable::EnvMode {
        fn from(value: EnvMode) -> Self {
            match value {
                EnvMode::Infer => global_hashable::EnvMode::Infer,
                EnvMode::Loose => global_hashable::EnvMode::Loose,
                EnvMode::Strict => global_hashable::EnvMode::Strict,
            }
        }
    }

    impl From<ResolvedEnvMode> for task_hashable::EnvMode {
        fn from(value: ResolvedEnvMode) -> Self {
            match value {
                ResolvedEnvMode::Loose => task_hashable::EnvMode::Loose,
                ResolvedEnvMode::Strict => task_hashable::EnvMode::Strict,
            }
        }
    }
}

#[derive(Debug)]
pub struct TaskHashable<'a> {
    // hashes
    pub(crate) global_hash: &'a str,
    pub(crate) task_dependency_hashes: Vec<String>,
    pub(crate) hash_of_files: &'a str,
    pub(crate) external_deps_hash: Option<String>,

    // task
    pub(crate) package_dir: Option<turbopath::RelativeUnixPathBuf>,
    pub(crate) task: &'a str,
    pub(crate) outputs: TaskOutputs,
    pub(crate) pass_through_args: &'a [String],

    // env
    pub(crate) env: &'a [String],
    pub(crate) resolved_env_vars: EnvVarPairs,
    pub(crate) pass_through_env: &'a [String],
    pub(crate) env_mode: ResolvedEnvMode,
    pub(crate) dot_env: &'a [turbopath::RelativeUnixPathBuf],
}

#[derive(Debug, Clone)]
pub struct GlobalHashable<'a> {
    pub global_cache_key: &'static str,
    pub global_file_hash_map: &'a HashMap<turbopath::RelativeUnixPathBuf, String>,
    // This is None in single package mode
    pub root_external_dependencies_hash: Option<&'a str>,
    pub env: &'a [String],
    pub resolved_env_vars: EnvironmentVariablePairs,
    pub pass_through_env: &'a [String],
    pub env_mode: EnvMode,
    pub framework_inference: bool,
    pub dot_env: &'a [turbopath::RelativeUnixPathBuf],
}

pub struct LockFilePackages(pub Vec<turborepo_lockfiles::Package>);

#[derive(Debug, Clone)]
pub struct FileHashes(pub HashMap<turbopath::RelativeUnixPathBuf, String>);

impl From<TaskOutputs> for Builder<HeapAllocator> {
    fn from(value: TaskOutputs) -> Self {
        let mut message = ::capnp::message::TypedBuilder::<
            proto_capnp::task_outputs::Owned,
            HeapAllocator,
        >::new_default();
        let mut builder = message.init_root();

        {
            let mut inclusions = builder
                .reborrow()
                .init_inclusions(value.inclusions.len() as u32);
            for (i, inclusion) in value.inclusions.iter().enumerate() {
                inclusions.set(i as u32, inclusion);
            }
        }

        {
            let mut exclusions = builder
                .reborrow()
                .init_exclusions(value.exclusions.len() as u32);
            for (i, exclusion) in value.exclusions.iter().enumerate() {
                exclusions.set(i as u32, exclusion);
            }
        }

        message.into_inner()
    }
}

impl From<LockFilePackages> for Builder<HeapAllocator> {
    fn from(LockFilePackages(packages): LockFilePackages) -> Self {
        let mut message = ::capnp::message::TypedBuilder::<
            proto_capnp::lock_file_packages::Owned,
            HeapAllocator,
        >::new_default();
        let mut builder = message.init_root();

        {
            let mut packages_builder = builder.reborrow().init_packages(packages.len() as u32);
            for (i, turborepo_lockfiles::Package { key, version }) in packages.iter().enumerate() {
                let mut package = packages_builder.reborrow().get(i as u32);
                package.set_key(key);
                package.set_version(version);
                // we don't track this in rust, set it to true
                package.set_found(true);
            }
        }

        // We're okay to unwrap here because we haven't hit the nesting
        // limit and the message will not have cycles.
        let size = builder
            .total_size()
            .expect("unable to calculate total size")
            .word_count
            + 1; // + 1 to solve an off by one error inside capnp
        let mut canon_builder =
            Builder::new(HeapAllocator::default().first_segment_words(size as u32));
        canon_builder
            .set_root_canonical(builder.reborrow_as_reader())
            .expect("can't fail");

        canon_builder
    }
}

impl From<FileHashes> for Builder<HeapAllocator> {
    fn from(FileHashes(file_hashes): FileHashes) -> Self {
        let mut message = ::capnp::message::TypedBuilder::<
            proto_capnp::file_hashes::Owned,
            HeapAllocator,
        >::new_default();
        let mut builder = message.init_root();

        {
            let mut entries = builder
                .reborrow()
                .init_file_hashes(file_hashes.len() as u32);

            // get a sorted iterator over keys and values of the hashmap
            // and set the entries in the capnp message

            let mut hashable: Vec<_> = file_hashes.into_iter().collect();
            hashable.sort_by(|(path_a, _), (path_b, _)| path_a.cmp(path_b));

            for (i, (key, value)) in hashable.iter().enumerate() {
                let mut entry = entries.reborrow().get(i as u32);
                entry.set_key(key.as_str());
                entry.set_value(value);
            }
        }

        // We're okay to unwrap here because we haven't hit the nesting
        // limit and the message will not have cycles.
        let size = builder
            .total_size()
            .expect("unable to calculate total size")
            .word_count
            + 1; // + 1 to solve an off by one error inside capnp
        let mut canon_builder =
            Builder::new(HeapAllocator::default().first_segment_words(size as u32));
        canon_builder
            .set_root_canonical(builder.reborrow_as_reader())
            .expect("can't fail");

        canon_builder
    }
}

type EnvVarPairs = Vec<String>;

impl From<TaskHashable<'_>> for Builder<HeapAllocator> {
    fn from(task_hashable: TaskHashable) -> Self {
        let mut message =
            ::capnp::message::TypedBuilder::<proto_capnp::task_hashable::Owned>::new_default();
        let mut builder = message.init_root();

        builder.set_global_hash(task_hashable.global_hash);
        if let Some(package_dir) = task_hashable.package_dir {
            builder.set_package_dir(&package_dir.to_string());
        }

        builder.set_hash_of_files(task_hashable.hash_of_files);
        if let Some(external_deps_hash) = task_hashable.external_deps_hash {
            builder.set_external_deps_hash(&external_deps_hash);
        }

        builder.set_task(task_hashable.task);
        builder.set_env_mode(task_hashable.env_mode.into());

        {
            let output_builder: Builder<_> = task_hashable.outputs.into();
            builder
                .set_outputs(output_builder.get_root_as_reader().unwrap())
                .unwrap();
        }

        {
            let mut task_dependency_hashes_builder = builder
                .reborrow()
                .init_task_dependency_hashes(task_hashable.task_dependency_hashes.len() as u32);
            for (i, hash) in task_hashable.task_dependency_hashes.iter().enumerate() {
                task_dependency_hashes_builder.set(i as u32, hash);
            }
        }

        {
            let mut pass_through_args_builder = builder
                .reborrow()
                .init_pass_thru_args(task_hashable.pass_through_args.len() as u32);
            for (i, arg) in task_hashable.pass_through_args.iter().enumerate() {
                pass_through_args_builder.set(i as u32, arg);
            }
        }

        {
            let mut env_builder = builder.reborrow().init_env(task_hashable.env.len() as u32);
            for (i, env) in task_hashable.env.iter().enumerate() {
                env_builder.set(i as u32, env);
            }
        }

        {
            let mut pass_through_env_builder = builder
                .reborrow()
                .init_pass_thru_env(task_hashable.pass_through_env.len() as u32);
            for (i, env) in task_hashable.pass_through_env.iter().enumerate() {
                pass_through_env_builder.set(i as u32, env);
            }
        }

        {
            let mut dotenv_builder = builder
                .reborrow()
                .init_dot_env(task_hashable.dot_env.len() as u32);
            for (i, env) in task_hashable.dot_env.iter().enumerate() {
                dotenv_builder.set(i as u32, env.as_str());
            }
        }

        {
            let mut resolved_env_vars_builder = builder
                .reborrow()
                .init_resolved_env_vars(task_hashable.resolved_env_vars.len() as u32);
            for (i, env) in task_hashable.resolved_env_vars.iter().enumerate() {
                resolved_env_vars_builder.set(i as u32, env);
            }
        }

        // We're okay to unwrap here because we haven't hit the nesting
        // limit and the message will not have cycles.
        let size = builder
            .total_size()
            .expect("unable to calculate total size")
            .word_count
            + 1; // + 1 to solve an off by one error inside capnp
        let mut canon_builder =
            Builder::new(HeapAllocator::default().first_segment_words(size as u32));
        canon_builder
            .set_root_canonical(builder.reborrow_as_reader())
            .expect("can't fail");

        canon_builder
    }
}

impl From<GlobalHashable<'_>> for Builder<HeapAllocator> {
    fn from(hashable: GlobalHashable) -> Self {
        let mut message =
            ::capnp::message::TypedBuilder::<proto_capnp::global_hashable::Owned>::new_default();

        let mut builder = message.init_root();

        builder.set_global_cache_key(hashable.global_cache_key);

        {
            let mut entries = builder
                .reborrow()
                .init_global_file_hash_map(hashable.global_file_hash_map.len() as u32);

            // get a sorted iterator over keys and values of the hashmap
            // and set the entries in the capnp message

            let mut hashable: Vec<_> = hashable.global_file_hash_map.iter().collect();
            hashable.sort_by(|a, b| a.0.cmp(b.0));

            for (i, (key, value)) in hashable.iter().enumerate() {
                let mut entry = entries.reborrow().get(i as u32);
                entry.set_key(key.as_str());
                entry.set_value(value);
            }
        }

        if let Some(root_external_dependencies_hash) = hashable.root_external_dependencies_hash {
            builder.set_root_external_deps_hash(root_external_dependencies_hash);
        }

        {
            let mut entries = builder.reborrow().init_env(hashable.env.len() as u32);
            for (i, env) in hashable.env.iter().enumerate() {
                entries.set(i as u32, env);
            }
        }

        {
            let mut resolved_env_vars = builder
                .reborrow()
                .init_resolved_env_vars(hashable.resolved_env_vars.len() as u32);
            for (i, env) in hashable.resolved_env_vars.iter().enumerate() {
                resolved_env_vars.set(i as u32, env);
            }
        }

        {
            let mut pass_through_env = builder
                .reborrow()
                .init_pass_through_env(hashable.pass_through_env.len() as u32);
            for (i, env) in hashable.pass_through_env.iter().enumerate() {
                pass_through_env.set(i as u32, env);
            }
        }

        builder.set_env_mode(match hashable.env_mode {
            EnvMode::Infer => proto_capnp::global_hashable::EnvMode::Infer,
            EnvMode::Loose => proto_capnp::global_hashable::EnvMode::Loose,
            EnvMode::Strict => proto_capnp::global_hashable::EnvMode::Strict,
        });

        builder.set_framework_inference(hashable.framework_inference);

        {
            let mut dot_env = builder
                .reborrow()
                .init_dot_env(hashable.dot_env.len() as u32);
            for (i, env) in hashable.dot_env.iter().enumerate() {
                dot_env.set(i as u32, env.as_str());
            }
        }

        // We're okay to unwrap here because we haven't hit the nesting
        // limit and the message will not have cycles.
        let size = builder
            .total_size()
            .expect("unable to calculate total size")
            .word_count
            + 1; // + 1 to solve an off by one error inside capnp
        let mut canon_builder =
            Builder::new(HeapAllocator::default().first_segment_words(size as u32));
        canon_builder
            .set_root_canonical(builder.reborrow_as_reader())
            .expect("can't fail");

        canon_builder
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turborepo_env::ResolvedEnvMode;
    use turborepo_lockfiles::Package;

    use super::{
        FileHashes, GlobalHashable, LockFilePackages, TaskHashable, TaskOutputs, TurboHash,
    };
    use crate::cli::EnvMode;

    #[test]
    fn task_hashable() {
        let task_hashable = TaskHashable {
            global_hash: "global_hash",
            task_dependency_hashes: vec!["task_dependency_hash".to_string()],
            package_dir: Some(turbopath::RelativeUnixPathBuf::new("package_dir").unwrap()),
            hash_of_files: "hash_of_files",
            external_deps_hash: Some("external_deps_hash".to_string()),
            task: "task",
            outputs: TaskOutputs {
                inclusions: vec!["inclusions".to_string()],
                exclusions: vec!["exclusions".to_string()],
            },
            pass_through_args: &["pass_thru_args".to_string()],
            env: &["env".to_string()],
            resolved_env_vars: vec![],
            pass_through_env: &["pass_thru_env".to_string()],
            env_mode: ResolvedEnvMode::Loose,
            dot_env: &[turbopath::RelativeUnixPathBuf::new("dotenv".to_string()).unwrap()],
        };

        assert_eq!(task_hashable.hash(), "ff765ee2f83bc034");
    }

    #[test]
    fn global_hashable() {
        let global_file_hash_map = vec![(
            turbopath::RelativeUnixPathBuf::new("global_file_hash_map").unwrap(),
            "global_file_hash_map".to_string(),
        )]
        .into_iter()
        .collect();

        let global_hash = GlobalHashable {
            global_cache_key: "global_cache_key",
            global_file_hash_map: &global_file_hash_map,
            root_external_dependencies_hash: Some("0000000000000000"),
            env: &["env".to_string()],
            resolved_env_vars: vec![],
            pass_through_env: &["pass_through_env".to_string()],
            env_mode: EnvMode::Infer,
            framework_inference: true,

            dot_env: &[turbopath::RelativeUnixPathBuf::new("dotenv".to_string()).unwrap()],
        };

        assert_eq!(global_hash.hash(), "c0ddf8138bd686e8");
    }

    #[test_case(vec![], "459c029558afe716" ; "empty")]
    #[test_case(vec![Package {
        key: "key".to_string(),
        version: "version".to_string(),
    }], "1b266409f3ae154e" ; "non-empty")]
    #[test_case(vec![Package {
        key: "key".to_string(),
        version: "".to_string(),
    }], "bde280722f61644a" ; "empty version")]
    #[test_case(vec![Package {
        key: "key".to_string(),
        version: "version".to_string(),
    }, Package {
        key: "zey".to_string(),
        version: "version".to_string(),
    }], "6c0185544234b6dc" ; "multiple in-order")]
    #[test_case(vec![Package {
        key: "zey".to_string(),
        version: "version".to_string(),
    }, Package {
        key: "key".to_string(),
        version: "version".to_string(),
    }], "26a67c9beeb0d16f" ; "care about order")]
    fn lock_file_packages(vec: Vec<Package>, expected: &str) {
        let packages = LockFilePackages(vec);
        assert_eq!(packages.hash(), expected);
    }

    #[test]
    fn long_lock_file_packages() {
        let packages = (0..100).map(|i| Package {
            key: format!("key{}", i),
            version: format!("version{}", i),
        });

        lock_file_packages(packages.collect(), "4fd770c37194168e");
    }

    #[test_case(vec![], "459c029558afe716" ; "empty")]
    #[test_case(vec![
        ("a".to_string(), "b".to_string()),
        ("c".to_string(), "d".to_string()),
    ], "c9301c0bf1899c07" ; "non-empty")]
    #[test_case(vec![
        ("c".to_string(), "d".to_string()),
        ("a".to_string(), "b".to_string()),
    ], "c9301c0bf1899c07" ; "order resistant")]
    fn file_hashes(pairs: Vec<(String, String)>, expected: &str) {
        let file_hashes = FileHashes(
            pairs
                .into_iter()
                .map(|(a, b)| (turbopath::RelativeUnixPathBuf::new(a).unwrap(), b))
                .collect(),
        );
        assert_eq!(file_hashes.hash(), expected);
    }
}
