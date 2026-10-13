use super::*;

fn record(value: &str) -> Record {
    Record::new(value.as_bytes().to_vec()).unwrap()
}

fn select(store: &mut Store, desired: &[Tool], record: &Record, force: bool) -> Outcome {
    let expected = store.generation().unwrap();
    store
        .reconcile_recorded_checked(desired, record, &expected, force, populate, || Ok(()))
        .unwrap()
}

#[test]
fn coherent_metadata_only_noop_force_and_legacy_selection() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    assert_eq!(
        store.repository_root().unwrap(),
        fs::canonicalize(repo.path()).unwrap()
    );
    let desired = [tool("node"), tool("pnpm")];
    store.reconcile(&desired, populate).unwrap();
    assert!(
        store
            .generation()
            .unwrap()
            .current()
            .unwrap()
            .record
            .is_none()
    );
    assert!(
        !String::from_utf8(manifest(&store))
            .unwrap()
            .contains("record_sha256")
    );
    let a = record("canonical selection A");
    let b = record("canonical selection B");
    for data in [&a, &a, &b] {
        let before = manifest(&store);
        let old = store.current().unwrap().unwrap();
        let expected = store.generation().unwrap();
        let noop = old.record.as_ref() == Some(data);
        let mut checked = false;
        assert_eq!(
            store
                .reconcile_recorded_checked(
                    &desired,
                    data,
                    &expected,
                    false,
                    |_, _| panic!("reuse full healthy trees"),
                    || {
                        checked = true;
                        Ok(())
                    }
                )
                .unwrap(),
            if noop {
                Outcome::Unchanged
            } else {
                Outcome::Replaced
            }
        );
        assert!(checked);
        let current = Store::inspect(repo.path()).unwrap().unwrap();
        assert!(current.record.as_ref() == Some(data));
        assert_eq!(manifest(&store) == before, noop);
        assert_eq!(current.bin == old.bin, noop);
        for tool in &desired {
            assert!(store.can_reuse(tool).unwrap());
            assert_eq!(
                fs::read(current.tool_tree(tool).unwrap().join("resource")).unwrap(),
                b"adjacent resource"
            );
        }
        assert_eq!(fs::read(old.bin.join("node")).unwrap(), b"1.2.3");
    }
    let before = manifest(&store);
    let expected = store.generation().unwrap();
    let mut calls = Vec::new();
    store
        .reconcile_recorded_checked(
            &desired,
            &b,
            &expected,
            true,
            |tool, tree| {
                calls.push(tool.id.clone());
                populate(tool, tree)
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(calls, ["node", "pnpm"]);
    assert_ne!(manifest(&store), before);
    for tool in &desired {
        assert!(store.can_reuse(tool).unwrap());
    }
    store
        .reconcile(&desired, |_, _| {
            panic!("ordinary metadata removal reuses trees")
        })
        .unwrap();
    assert!(store.current().unwrap().unwrap().record.is_none());
    assert_eq!(
        store.reconcile(&desired, |_, _| panic!()).unwrap(),
        Outcome::Unchanged
    );
    select(&mut store, &desired, &a, false);
    store
        .force_reconcile_checked(&desired, populate, || Ok(()))
        .unwrap();
    assert!(store.current().unwrap().unwrap().record.is_none());
}

#[test]
fn recorded_failures_never_select_partial_tools_or_metadata() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [tool("node")];
    let a = record("old selection");
    let b = record("new selection");
    select(&mut store, &desired, &a, false);
    let before = manifest(&store);
    let old = store.current().unwrap().unwrap().bin;
    for data in [&a, &b] {
        let expected = store.generation().unwrap();
        assert!(
            store
                .reconcile_recorded_checked(
                    &desired,
                    data,
                    &expected,
                    false,
                    |_, _| panic!(),
                    || Err(io::Error::other("source guard changed").into())
                )
                .is_err()
        );
        assert_eq!(manifest(&store), before);
    }
    for failure in 0..8 {
        let expected = store.generation().unwrap();
        let root = store.root.clone();
        let mut staged = None;
        let result = store.reconcile_recorded_checked(
            &desired,
            &b,
            &expected,
            true,
            |tool, tree| {
                staged = Some(tree.parent().unwrap().parent().unwrap().to_path_buf());
                if failure == 0 {
                    return Err(io::Error::other("prepare failed").into());
                }
                if failure == 1 {
                    return Ok(());
                } // Invalid full tree.
                populate(tool, tree)?;
                let generation = staged.as_ref().unwrap();
                match failure {
                    2 => fs::create_dir(generation.join("record.json"))?, // create_new failure.
                    3 => fs::write(generation.join("record.json"), b"foreign")?,
                    4 => fs::write(generation.join("con"), b"unflushable managed name")?,
                    5 => fs::set_permissions(&root, fs::Permissions::from_mode(0o500))?,
                    _ => {}
                }
                Ok(())
            },
            || {
                if failure == 6 {
                    return Err(io::Error::other("final failure").into());
                }
                if failure == 7 {
                    fs::set_permissions(&root, fs::Permissions::from_mode(0o500))?;
                    return Ok(()); // Persist fails after all final checks.
                }
                panic!("must fail before final check")
            },
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err(), "failure {failure}");
        if failure == 7 {
            assert!(matches!(result, Err(Error::Io(_))));
        }
        assert_eq!(manifest(&store), before);
        assert!(store.current().unwrap().unwrap().record.as_ref() == Some(&a));
        assert_eq!(
            fs::read(old.join("../tools/node/resource")).unwrap(),
            b"adjacent resource"
        );
    }
}

#[test]
fn readonly_records_are_bounded_hash_checked_and_not_health_stamps() {
    assert!(Record::new(Vec::new()).is_err());
    assert!(Record::new(vec![0; RECORD_LIMIT as usize + 1]).is_err());
    let maximum = Record::new(vec![0; RECORD_LIMIT as usize]).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    select(&mut store, &[], &maximum, false);
    assert!(
        Store::inspect(repo.path())
            .unwrap()
            .unwrap()
            .record
            .as_ref()
            == Some(&maximum)
    );
    for damage in 0..12 {
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        let desired = [tool("node"), tool("pnpm")];
        let data = record("canonical opaque bytes");
        select(&mut store, &desired, &data, false);
        let current = store.current().unwrap().unwrap();
        let path = current.bin.parent().unwrap().join("record.json");
        let mut wire: Inventory = serde_json::from_slice(&manifest(&store)).unwrap();
        match damage {
            0 => fs::write(&path, b"corrupt").unwrap(),
            1 => fs::remove_file(&path).unwrap(),
            2 => {
                fs::remove_file(&path).unwrap();
                symlink("tools/node/resource", &path).unwrap();
            }
            3 => File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(RECORD_LIMIT + 1)
                .unwrap(),
            4 => wire.record_sha256 = Some("invalid".into()),
            5 => wire.record_sha256 = None,
            6 => fs::write(current.bin.join("pnpm"), b"damaged sibling").unwrap(),
            7 => fs::hard_link(&path, path.with_extension("alias")).unwrap(),
            8 => fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap(),
            9 => {
                fs::remove_file(&path).unwrap();
                fs::create_dir(&path).unwrap();
            }
            10 => fs::write(&path, b"").unwrap(),
            _ => wire.record_sha256 = Some(record("different generation").hash()),
        }
        fs::write(
            store.root.join("manifest.json"),
            serde_json::to_vec(&wire).unwrap(),
        )
        .unwrap();
        let before = manifest(&store);
        assert!(Store::inspect(repo.path()).is_err(), "damage {damage}");
        assert!(store.current().is_err());
        assert!(store.generation().is_err());
        assert!(!store.can_reuse(&desired[0]).unwrap_or(false));
        assert!(!store.is_current(&desired).unwrap_or(false));
        assert_eq!(manifest(&store), before);
        assert!(
            store
                .reconcile(&desired, |_, _| Err(
                    io::Error::other("no verified repair").into()
                ))
                .is_err()
        );
        assert_eq!(manifest(&store), before);
    }
}

#[test]
fn absent_tokens_are_distinct_from_invalid_or_missing_generations() {
    let repo = tempfile::tempdir().unwrap();
    assert!(Store::inspect(repo.path()).unwrap().is_none());
    assert!(!repo.path().join(".turbo").exists());
    let mut store = Store::open(repo.path()).unwrap();
    let absent = store.generation().unwrap();
    assert!(absent.current().is_none());
    let desired = [tool("node")];
    let data = record("first selection");
    select(&mut store, &desired, &data, false);
    assert!(store.check_generation(&absent).is_err());
    let current = store.current().unwrap().unwrap();
    let before = manifest(&store);
    fs::remove_dir_all(current.bin.parent().unwrap()).unwrap();
    assert!(store.generation().is_err());
    assert!(store.current().unwrap().is_none()); // Generic repair remains supported.
    assert!(!store.can_reuse(&desired[0]).unwrap());
    store.reconcile(&desired, populate).unwrap();
    assert_ne!(manifest(&store), before);
    fs::write(
        store.root.join("manifest.json"),
        b"invalid present manifest",
    )
    .unwrap();
    assert!(store.generation().is_err());
    fs::write(
        store.root.join("manifest.json"),
        vec![b' '; RECORD_LIMIT as usize + 1],
    )
    .unwrap();
    assert!(store.generation().is_err());
}

#[test]
fn tokens_bind_guard_root_exact_manifest_and_full_resources_after_waits() {
    let repo = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let mut foreign = Store::open(other.path()).unwrap();
    let desired = [tool("node")];
    let data = record("same metadata");
    let absent = store.generation().unwrap();
    assert!(foreign.check_generation(&absent).is_err());
    select(&mut store, &desired, &data, false);
    select(&mut foreign, &desired, &data, false);
    let expected = store.generation().unwrap();
    assert!(
        foreign
            .reconcile_recorded_checked(
                &desired,
                &data,
                &expected,
                false,
                |_, _| panic!(),
                || panic!()
            )
            .is_err()
    );
    let before = manifest(&store);
    fs::write(
        store.root.join("manifest.json"),
        [before.as_slice(), b" "].concat(),
    )
    .unwrap();
    assert!(store.current().is_ok()); // Same parsed manifest, different actual bytes.
    assert!(store.check_generation(&expected).is_err());
    fs::write(store.root.join("manifest.json"), &before).unwrap();
    let tree = expected.current().unwrap().tool_tree(&desired[0]).unwrap();
    fs::write(tree.join("resource"), b"resource drift during wait").unwrap();
    assert!(store.check_generation(&expected).is_err());
    fs::write(tree.join("resource"), b"adjacent resource").unwrap();
    select(&mut store, &desired, &data, true);
    assert!(store.check_generation(&expected).is_err());
    let reopened = store.generation().unwrap();
    drop(store);
    let store = Store::open(repo.path()).unwrap();
    assert!(store.check_generation(&reopened).is_err());
}

#[test]
fn final_pointer_record_resource_and_root_races_reject_noop_and_changed_paths() {
    for changed in [false, true] {
        for race in 0..7 {
            let repo = tempfile::tempdir().unwrap();
            let mut store = Store::open(repo.path()).unwrap();
            let desired = [tool("node")];
            let a = record("old selection");
            let b = record("new selection");
            select(&mut store, &desired, &a, false);
            let expected = store.generation().unwrap();
            let before = manifest(&store);
            let root = store.root.clone();
            let old_generation = expected
                .current()
                .unwrap()
                .bin
                .parent()
                .unwrap()
                .to_path_buf();
            let data = if changed { &b } else { &a };
            assert!(
                store
                    .reconcile_recorded_checked(
                        &desired,
                        data,
                        &expected,
                        false,
                        |_, _| panic!("copy healthy tree"),
                        || {
                            match race {
                                0 => fs::write(
                                    root.join("manifest.json"),
                                    [before.as_slice(), b" "].concat(),
                                )?,
                                1 => fs::write(old_generation.join("record.json"), b"corrupt")?,
                                2 => fs::remove_file(root.join("manifest.json"))?,
                                3 => fs::write(
                                    old_generation.join("tools/node/resource"),
                                    b"corrupt resource",
                                )?,
                                4 => fs::remove_dir_all(&old_generation)?,
                                5 => {
                                    fs::rename(&root, root.with_extension("held"))?;
                                    fs::create_dir(&root)?;
                                    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
                                }
                                _ if changed => {
                                    for entry in fs::read_dir(&root)? {
                                        let path = entry?.path();
                                        if path != old_generation && path.is_dir() {
                                            fs::write(
                                                path.join("record.json"),
                                                b"corrupt staged record",
                                            )?;
                                        }
                                    }
                                }
                                _ => fs::write(old_generation.join("record.json"), b"corrupt")?,
                            }
                            Ok(())
                        }
                    )
                    .is_err(),
                "changed {changed}, race {race}"
            );
            // Only the deliberate racing writer's state remains, never our new selector.
            match race {
                0 => assert_eq!(manifest(&store), [before.as_slice(), b" "].concat()),
                2 | 5 => assert!(!root.join("manifest.json").exists()),
                _ => assert_eq!(manifest(&store), before),
            }
        }
    }
}

#[test]
fn replaced_repository_cannot_rebind_the_same_storage_inode() {
    let owner = tempfile::tempdir().unwrap();
    let repo = owner.path().join("repo");
    fs::create_dir(&repo).unwrap();
    let mut store = Store::open(&repo).unwrap();
    let data = record("original repository");
    select(&mut store, &[], &data, false);
    let expected = store.generation().unwrap();
    let moved = owner.path().join("moved");
    fs::rename(&repo, &moved).unwrap();
    fs::create_dir(&repo).unwrap();
    fs::rename(moved.join(".turbo"), repo.join(".turbo")).unwrap();
    assert!(store.check_generation(&expected).is_err());
    assert!(
        store
            .reconcile_recorded_checked(&[], &data, &expected, false, |_, _| panic!(), || panic!())
            .is_err()
    );
}

#[test]
fn replaced_root_rejected_at_entry_even_with_identical_manifest_bytes() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let data = record("root bound");
    let desired = [tool("node")];
    select(&mut store, &desired, &data, false);
    let expected = store.generation().unwrap();
    let held = store.root.with_extension("held");
    fs::rename(&store.root, &held).unwrap();
    fs::create_dir(&store.root).unwrap();
    fs::set_permissions(&store.root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::copy(held.join("manifest.json"), store.root.join("manifest.json")).unwrap();
    assert!(store.check_generation(&expected).is_err());
    assert!(store.can_reuse(&desired[0]).is_err());
    assert!(
        store
            .reconcile_recorded_checked(
                &desired,
                &data,
                &expected,
                true,
                |_, _| panic!(),
                || panic!()
            )
            .is_err()
    );
}
