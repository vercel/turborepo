use std::{
    collections::{btree_map, BTreeMap, HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    mem::take,
};

use anyhow::Result;
use indexmap::IndexMap;
use indoc::indoc;
use toml::{map::Entry, value::Array, Table, Value};

const NEXT_PATH: &str = "./next.js/packages/next-swc/";
const TURBO_PATH: &str = "./turbo-crates/";
const DEFAULT_TOML: &str = indoc!(
    r#"
    [workspace]
    resolver = "2"

    members = ["crates/*"]
"#
);

fn read_toml(file: &str) -> Table {
    let mut cargo_toml = String::new();
    let n = File::open(file)
        .unwrap_or_else(|_| panic!("failed to open {file}"))
        .read_to_string(&mut cargo_toml)
        .unwrap_or_else(|_| panic!("failed to read {file}"));
    cargo_toml.truncate(n);

    toml::from_str(&cargo_toml).unwrap_or_else(|_| panic!("failed to parse {file}"))
}

fn remap_path_dependencies(toml: &mut Table, path_prefix: &str) {
    for (_, value) in toml.iter_mut() {
        if let Some(attrs) = value.as_table_mut() {
            for (k, v) in attrs.iter_mut() {
                if k == "path" {
                    *v = Value::String(format!("{path_prefix}{}", v.as_str().unwrap()));
                }
            }
        }
    }
}

fn remap_git_dependencies(toml: &mut Table, git_repo: &str, path_prefix: &str) {
    for (key, value) in toml.iter_mut() {
        if let Some(attrs) = value.as_table_mut() {
            if let Some(git) = attrs.get("git") {
                if git.as_str().unwrap() == git_repo {
                    attrs.remove("git");
                    attrs.remove("rev");
                    attrs.remove("tag");
                    attrs.remove("branch");
                    attrs.insert(
                        "path".to_string(),
                        Value::String(format!("{path_prefix}{}", key)),
                    );
                }
            }
        }
    }
}

fn remap_lockfile_git_dependencies(lock_entries: &mut BTreeMap<String, Value>, git_repo: &str) {
    for value in lock_entries.values_mut() {
        if let Some(attrs) = value.as_table_mut() {
            if let Some(git) = attrs.get("source") {
                if git.as_str().unwrap().starts_with(git_repo) {
                    attrs.remove("source");
                }
            }
        }
    }
}

fn to_map(lock: Array) -> BTreeMap<String, Value> {
    let mut map = lock
        .into_iter()
        .map(|v| {
            let map = v.as_table().unwrap().clone();
            let name = map.get("name").unwrap().as_str().unwrap();
            let version = map.get("version").unwrap().as_str().unwrap();
            (format!("{name} {version}"), v)
        })
        .collect::<BTreeMap<_, _>>();
    let by_name: HashMap<String, String> = map
        .iter()
        .map(|(k, v)| {
            let map = v.as_table().unwrap();
            let name = map.get("name").unwrap().as_str().unwrap().to_string();
            (name, k.to_string())
        })
        .collect();
    for entry in map.values_mut() {
        let map = entry.as_table_mut().unwrap();
        if let Some(deps) = map.get_mut("dependencies") {
            for dep in deps.as_array_mut().unwrap() {
                let dep_str = dep.as_str().unwrap();
                if let Some(dep_with_version) = by_name.get(dep_str) {
                    *dep = Value::String(dep_with_version.clone());
                }
            }
        }
    }
    map
}

fn get_diff_only(a: &Value, b: &Value) -> Value {
    if let (Some(a), Some(b)) = (a.as_table(), b.as_table()) {
        Value::Table(get_table_diff_only(a, b))
    } else if let (Some(a), Some(b)) = (a.as_array(), b.as_array()) {
        Value::Array(get_array_diff_only(a, b))
    } else {
        a.clone()
    }
}

fn get_table_diff_only(a: &Table, b: &Table) -> Table {
    let mut diff = Table::new();
    for (key, value) in a {
        if let Some(b_value) = b.get(key) {
            if value != b_value {
                diff.insert(key.to_string(), get_diff_only(value, b_value));
            }
        } else {
            diff.insert(key.to_string(), value.clone());
        }
    }
    diff
}

fn get_array_diff_only(a: &Array, b: &Array) -> Array {
    let mut diff = Array::new();
    for value in a {
        if !b.iter().any(|b_value| value == b_value) {
            diff.push(value.clone());
        }
    }
    diff
}

fn take_dependencies(lock_entry: &mut Table) -> Array {
    if let Some(mut deps_value) = lock_entry.remove("dependencies") {
        take(deps_value.as_array_mut().unwrap())
    } else {
        Array::new()
    }
}

fn deps_to_map(deps: &Array) -> IndexMap<String, String> {
    deps.iter()
        .map(|v| {
            let item = v.as_str().unwrap();
            let (name, version) = item.split_once(' ').unwrap();
            (name.to_string(), version.to_string())
        })
        .collect::<IndexMap<_, _>>()
}

fn merge_dependencies(parent: &str, next_deps: Array, turbo_deps: Array) -> (Array, usize) {
    let next_map = deps_to_map(&next_deps);
    let turbo_map = deps_to_map(&turbo_deps);
    let mut conflicts = 0;
    let mut merged = Array::new();
    let mut handled = HashSet::new();
    for (name, next_version) in next_map {
        if let Some(turbo_version) = turbo_map.get(&name) {
            if next_version != *turbo_version {
                conflicts += 1;
                println!(
                    "confliction lockfile entry dependencies:\n  next:  {parent} -> {name} = \
                     {next_version}\n  turbo: {parent} -> {name} = {turbo_version}",
                );
            }
        }
        merged.push(Value::String(format!("{} {}", name, next_version)));
        handled.insert(name);
    }
    for (name, turbo_version) in turbo_map {
        if !handled.contains(&name) {
            merged.push(Value::String(format!("{} {}", name, turbo_version)));
        }
    }
    (merged, conflicts)
}

fn main() -> Result<()> {
    let conflicts_count = sync_cargo_toml()?;
    let lockfile_conflicts_count = sync_cargo_lock()?;
    if conflicts_count > 0 || lockfile_conflicts_count > 0 {
        println!(
            "\n### Merged with conflicts: {} conflicts in Cargo.toml, {} conflicts in Cargo.lock \
             ###\n",
            conflicts_count, lockfile_conflicts_count
        );
    }
    Ok(())
}

fn sync_cargo_toml() -> Result<usize> {
    println!("Synchronizing nextpack Cargo.toml with Turbo and Next.js…");

    let turbo_toml = read_toml("../Cargo.toml");
    let next_toml = read_toml(&[NEXT_PATH, "Cargo.toml"].concat());

    let mut cargo_toml: Table =
        toml::from_str(DEFAULT_TOML).expect("failed to parse default Cargo.toml");

    let members = cargo_toml["workspace"]["members"]
        .as_array_mut()
        .expect("cargo_toml[workspace][members]");
    members.extend(
        // Turbo crates are included in members, only Turbopack crates are in default-members.
        turbo_toml["workspace"]["default-members"]
            .as_array()
            .unwrap()
            .iter()
            // exclude xtask
            .filter(|member| member.as_str().unwrap().contains("crates/"))
            .map(|s| Value::String(format!("{TURBO_PATH}{}", s.as_str().unwrap()))),
    );
    members.extend(
        next_toml["workspace"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Value::String(format!("{NEXT_PATH}{}", s.as_str().unwrap()))),
    );

    let mut next_dependencies: Table = next_toml["workspace"]["dependencies"]
        .as_table()
        .unwrap()
        .clone();
    remap_path_dependencies(&mut next_dependencies, NEXT_PATH);
    remap_git_dependencies(
        &mut next_dependencies,
        "https://github.com/vercel/turbo.git",
        &format!("{TURBO_PATH}crates/"),
    );
    let mut turbo_dependencies: Table = turbo_toml["workspace"]["dependencies"]
        .as_table()
        .unwrap()
        .clone();
    remap_path_dependencies(&mut turbo_dependencies, TURBO_PATH);

    // Merge dependencies from Next.js and Turbo.
    let mut conflicts_count = 0;
    let mut dependencies = next_dependencies;
    for (key, turbo_value) in turbo_dependencies {
        match dependencies.entry(&key) {
            Entry::Occupied(mut e) => {
                let next_value = e.get_mut();
                if *next_value != turbo_value {
                    println!(
                        "conflicting dependency:\n  next:  {} = {}\n  turbo: {} = {}",
                        key,
                        get_diff_only(next_value, &turbo_value),
                        key,
                        get_diff_only(&turbo_value, next_value)
                    );
                    conflicts_count += 1;
                }
            }
            Entry::Vacant(e) => {
                e.insert(turbo_value);
            }
        }
    }

    // Ensure workspace path deps to members at least.
    for member in members.iter() {
        let dep = member.as_str().unwrap();
        if let Some((path, name)) = dep.rsplit_once('/') {
            if path == "crates" {
                // ignore the nexpack helper crates.
                continue;
            }
            if let Some(member_dep) = dependencies.get_mut(name).and_then(|d| d.as_table_mut()) {
                member_dep.insert("path".to_string(), member.clone());
            } else {
                let mut member_dep = Table::new();
                member_dep.insert("path".to_string(), member.clone());
                dependencies.insert(name.to_string(), Value::Table(member_dep));
            }
        }
    }

    // Turbo's workspace dependencies must be duplicated for any of the crates to
    // work, since they expect it to be in the workspace root. But we need to remap
    // any path dependencies to be relative to Turbo's Cargo.toml.
    cargo_toml["workspace"]
        .as_table_mut()
        .unwrap()
        .insert("dependencies".to_string(), Value::Table(dependencies));

    let mut toml_file = File::options()
        .write(true)
        .truncate(true)
        .open("Cargo.toml")?;
    writeln!(toml_file, "# THIS FILE IS AUTOGENERATED BY sync-workspace")?;
    writeln!(
        toml_file,
        "# Do NOT make changes to this file, instead change"
    )?;
    writeln!(toml_file, "# - ../Cargo.toml")?;
    writeln!(toml_file, "# - {}Cargo.toml", NEXT_PATH)?;
    writeln!(toml_file, "# and run `cargo run --bin sync-workspace`.")?;
    toml_file.write_all(toml::to_string_pretty(&cargo_toml)?.as_bytes())?;

    Ok(conflicts_count)
}

fn sync_cargo_lock() -> Result<usize> {
    println!("Synchronizing nextpack Cargo.lock with Turbo and Next.js…");

    let turbo_lock = read_toml("../Cargo.lock");
    let next_lock = read_toml(&[NEXT_PATH, "Cargo.lock"].concat());

    let mut next_lock_entries = to_map(next_lock["package"].as_array().unwrap().clone());
    remap_lockfile_git_dependencies(
        &mut next_lock_entries,
        "git+https://github.com/vercel/turbo.git",
    );
    let turbo_lock_entries = to_map(turbo_lock["package"].as_array().unwrap().clone());

    // Merge dependencies from Next.js and Turbo.
    let mut conflicts_count = 0;
    let mut lock_entries = next_lock_entries;
    for (key, mut turbo_value) in turbo_lock_entries {
        let turbo_deps = take_dependencies(turbo_value.as_table_mut().unwrap());
        match lock_entries.entry(key.clone()) {
            btree_map::Entry::Occupied(mut e) => {
                let next_value = e.get_mut();
                let next_deps = take_dependencies(next_value.as_table_mut().unwrap());
                if *next_value != turbo_value {
                    println!(
                        "conflicting lockfile entry:\n  next:  {} = {}\n  turbo: {} = {}",
                        key,
                        get_diff_only(next_value, &turbo_value),
                        key,
                        get_diff_only(&turbo_value, next_value)
                    );
                    conflicts_count += 1;
                }
                let (deps, conflicts) = merge_dependencies(&key, next_deps, turbo_deps);
                conflicts_count += conflicts;
                next_value
                    .as_table_mut()
                    .unwrap()
                    .insert("dependencies".to_string(), Value::Array(deps));
            }
            btree_map::Entry::Vacant(e) => {
                e.insert(turbo_value);
            }
        }
    }

    let mut cargo_lock = Table::new();
    cargo_lock.insert(
        "package".to_string(),
        Value::Array(lock_entries.into_values().collect()),
    );

    let mut lock_file = File::options()
        .write(true)
        .truncate(true)
        .open("Cargo.lock")?;
    writeln!(
        lock_file,
        "# This file is automatically @generated by Cargo."
    )?;
    writeln!(lock_file, "# It is not intended for manual editing.")?;
    writeln!(lock_file, "version = 3")?;
    writeln!(lock_file)?;
    lock_file.write_all(toml::to_string_pretty(&cargo_lock)?.as_bytes())?;

    Ok(conflicts_count)
}
