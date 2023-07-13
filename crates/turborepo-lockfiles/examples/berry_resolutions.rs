use turborepo_lockfiles::{BerryLockfile, BerryManifest, Lockfile, LockfileData};

fn main() {
    let manifest = generate_manifest("foobar", 100);
    let lockfile_bytes = include_bytes!("yarn.lock");
    let data = LockfileData::from_bytes(lockfile_bytes.as_slice()).unwrap();
    let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();
    let key = "debug@npm:3.2.7";
    println!(
        "Dependencies of {key}: {}",
        lockfile
            .all_dependencies(key)
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|(k, v)| format!("{k}@{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn generate_manifest(prefix: &str, size: usize) -> BerryManifest {
    let mut count = 0;
    BerryManifest::with_resolutions(std::iter::from_fn(move || {
        let cont = count < size;
        count += 1;
        if cont {
            Some((format!("{prefix}{count}"), "1.0.0".to_string()))
        } else {
            None
        }
    }))
}
