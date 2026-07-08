use std::str::FromStr;

use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;
use crate::Lockfile;

#[test]
fn test_global_change_version_mismatch() {
    let v0_contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
            }
        },
        "packages": {}
    }))
    .unwrap();

    let v1_contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
            }
        },
        "packages": {}
    }))
    .unwrap();

    let v0_lockfile = BunLockfile::from_str(&v0_contents).unwrap();
    let v1_lockfile = BunLockfile::from_str(&v1_contents).unwrap();

    // Version change should be detected
    assert!(v0_lockfile.global_change(&v1_lockfile));

    // Same version should not be a global change
    assert!(!v0_lockfile.global_change(&v0_lockfile));
    assert!(!v1_lockfile.global_change(&v1_lockfile));
}

#[test]
fn test_bun_global_change_function() {
    let v0_contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
            }
        },
        "packages": {}
    }))
    .unwrap();

    let v1_contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
            }
        },
        "packages": {}
    }))
    .unwrap();

    // Test the standalone function
    assert!(bun_global_change(v0_contents.as_bytes(), v1_contents.as_bytes()).unwrap());
    assert!(!bun_global_change(v0_contents.as_bytes(), v0_contents.as_bytes()).unwrap());
    assert!(!bun_global_change(v1_contents.as_bytes(), v1_contents.as_bytes()).unwrap());
}

#[test]
fn test_new_fields_parsing() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "foo": "^1.0.0"
                }
            }
        },
        "packages": {
            "foo": ["foo@1.0.0", {}, "sha512-hello"]
        },
        "overrides": {
            "foo": "1.0.0"
        },
        "catalog": {
            "react": "^18.0.0"
        },
        "catalogs": {
            "frontend": {
                "react": "^18.0.0",
                "next": "^14.0.0"
            }
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Check that new fields are parsed
    assert_eq!(lockfile.data.overrides.len(), 1);
    assert_eq!(
        lockfile.data.overrides.get("foo"),
        Some(&"1.0.0".to_string())
    );

    assert_eq!(lockfile.data.catalog.len(), 1);
    assert_eq!(
        lockfile.data.catalog.get("react"),
        Some(&"^18.0.0".to_string())
    );

    assert_eq!(lockfile.data.catalogs.len(), 1);
    let frontend_catalog = lockfile.data.catalogs.get("frontend").unwrap();
    assert_eq!(frontend_catalog.len(), 2);
    assert_eq!(frontend_catalog.get("react"), Some(&"^18.0.0".to_string()));
    assert_eq!(frontend_catalog.get("next"), Some(&"^14.0.0".to_string()));
}

#[test]
fn test_failure_if_mismatched_keys() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "foo": "^1.0.0",
                    "bar": "^1.0.0",
                }
            }
        },
        "packages": {
            "bar": ["bar@1.0.0", { "dependencies": { "shared": "^1.0.0" } }, "sha512-goodbye"],
            "bar/shared": ["shared@1.0.0", {}, "sha512-bar"],
            "foo": ["foo@1.0.0", { "dependencies": { "shared": "^1.0.0" } }, "sha512-hello"],
            "foo/shared": ["shared@1.0.0", { }, "sha512-foo"],
        }
    }))
    .unwrap();
    let lockfile = BunLockfile::from_str(&contents);
    assert!(lockfile.is_err(), "matching packages have differing shas");
}

#[test]
fn test_override_functionality_no_override() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "bar": "^1.0.0"
                }
            }
        },
        "packages": {
            "bar": ["bar@1.0.0", {}, "sha512-original"]
        },
        "overrides": {
            "foo": "2.0.0"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Resolve bar - should get original version since no override exists for bar
    let result = lockfile
        .resolve_package("", "bar", "^1.0.0")
        .unwrap()
        .unwrap();

    // Should resolve to original version (no override)
    assert_eq!(result.key, "bar@1.0.0");
    assert_eq!(result.version, "1.0.0");
}

#[test]
fn test_override_resolves_lower_version() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "parent": "^1.0.0"
                }
            }
        },
        "packages": {
            "parent": ["parent@1.0.0", "", {
                "dependencies": {
                    "dep": "1.5.0"
                }
            }, "sha512-parent"],
            "dep": ["dep@1.4.0", "", {}, "sha512-dep"]
        },
        "overrides": {
            "dep": "1.4.0"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    let result = lockfile.resolve_package("", "dep", "1.5.0").unwrap();

    assert!(
        result.is_some(),
        "Override to lower version (1.4.0) must still resolve even though 1.4.0 does not satisfy \
         ^1.5.0"
    );
    let pkg = result.unwrap();
    assert_eq!(pkg.key, "dep@1.4.0");
    assert_eq!(pkg.version, "1.4.0");
}

#[test]
fn test_override_lower_version_in_transitive_closure() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "parent": "^1.0.0"
                }
            }
        },
        "packages": {
            "parent": ["parent@1.0.0", "", {
                "dependencies": {
                    "dep": "1.5.0"
                }
            }, "sha512-parent"],
            "dep": ["dep@1.4.0", "", {}, "sha512-dep"]
        },
        "overrides": {
            "dep": "1.4.0"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    let unresolved_deps: std::collections::BTreeMap<String, String> =
        [("parent".to_string(), "^1.0.0".to_string())]
            .into_iter()
            .collect();
    let closure = crate::transitive_closure(&lockfile, "", unresolved_deps, false).unwrap();

    let dep_keys: Vec<String> = closure.iter().map(|p| p.key.clone()).collect();
    assert!(
        dep_keys.contains(&"dep@1.4.0".to_string()),
        "Overridden dep@1.4.0 must be in transitive closure even though parent declares \
         dep@1.5.0. Got: {:?}",
        dep_keys
    );
}

#[test]
fn test_subgraph_filters_overrides() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "foo": "^1.0.0",
                    "bar": "^1.0.0"
                }
            },
            "apps/web": {
                "name": "web",
                "dependencies": {
                    "foo": "^1.0.0"
                }
            }
        },
        "packages": {
            "foo": ["foo@1.0.0", {}, "sha512-foo"],
            "bar": ["bar@1.0.0", {}, "sha512-bar"]
        },
        "overrides": {
            "foo": "2.0.0",
            "bar": "2.0.0",
            "unused": "1.0.0"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Create subgraph with only foo package
    let subgraph = lockfile
        .subgraph(&["apps/web".into()], &["foo@1.0.0".into()])
        .unwrap();
    let subgraph_data = subgraph.lockfile().unwrap();

    // All overrides are preserved to stay in sync with the root
    // package.json that turbo prune copies as-is
    assert_eq!(subgraph_data.overrides.len(), 3);
    assert!(subgraph_data.overrides.contains_key("foo"));
    assert!(subgraph_data.overrides.contains_key("bar"));
    assert!(subgraph_data.overrides.contains_key("unused"));

    // Check that workspaces are correct
    assert_eq!(subgraph_data.workspaces.len(), 2);
    assert!(subgraph_data.workspaces.contains_key(""));
    assert!(subgraph_data.workspaces.contains_key("apps/web"));

    // Check that packages are correct
    assert_eq!(subgraph_data.packages.len(), 1);
    assert!(subgraph_data.packages.contains_key("foo"));
    assert!(!subgraph_data.packages.contains_key("bar"));
}

#[test]
fn test_override_with_patches() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "lodash": "^4.17.20"
                }
            }
        },
        "packages": {
            "lodash": ["lodash@4.17.20", {}, "sha512-original"],
            "lodash-override": ["lodash@4.17.21", {}, "sha512-override"]
        },
        "overrides": {
            "lodash": "4.17.21"
        },
        "patchedDependencies": {
            "lodash@4.17.21": "patches/lodash@4.17.21.patch"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Resolve lodash - should get override version with patch
    let result = lockfile
        .resolve_package("", "lodash", "^4.17.20")
        .unwrap()
        .unwrap();

    // Should resolve to overridden version with patch
    assert_eq!(result.key, "lodash@4.17.21");
    assert_eq!(result.version, "4.17.21+patches/lodash@4.17.21.patch");
}

#[test]
fn test_catalog_with_overrides() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 0,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "react": "catalog:"
                }
            }
        },
        "packages": {
            "react": ["react@18.2.0", {}, "sha512-react18"],
            "react-override": ["react@19.0.0", {}, "sha512-react19"]
        },
        "catalog": {
            "react": "^18.2.0"
        },
        "overrides": {
            "react": "19.0.0"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Resolve react - should get override version instead of catalog version
    let result = lockfile
        .resolve_package("", "react", "catalog:")
        .unwrap()
        .unwrap();

    // Should resolve to overridden version
    assert_eq!(result.key, "react@19.0.0");
    assert_eq!(result.version, "19.0.0");
}

#[test]
fn test_optional_dependencies_not_in_lockfile() {
    // Test that optional dependencies that are not present in the lockfile
    // don't cause errors when calculating transitive closures
    let lockfile_content = r#"{
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-app",
                    "dependencies": {
                        "@emnapi/runtime": "^1.0.0"
                    }
                }
            },
            "packages": {
                "@emnapi/runtime": [
                    "@emnapi/runtime@1.5.0",
                    "",
                    {
                        "dependencies": {
                            "tslib": "^2.4.0"
                        },
                        "optionalDependencies": {
                            "@emnapi/wasi-threads": "^1.0.0"
                        }
                    },
                    "sha512"
                ],
                "tslib": [
                    "tslib@2.8.1",
                    "",
                    {},
                    "sha512"
                ]
            }
        }"#;

    let lockfile = BunLockfile::from_str(lockfile_content).unwrap();

    // This should not error even though @emnapi/wasi-threads is not in the packages
    let deps = lockfile
        .all_dependencies("@emnapi/runtime@1.5.0")
        .unwrap()
        .unwrap();

    // Should only contain tslib, not @emnapi/wasi-threads
    assert_eq!(deps.len(), 1);
    let keys: Vec<_> = deps.keys().collect();
    assert_eq!(keys.len(), 1);
    assert!(keys[0].contains("tslib"));
    assert!(!deps.values().any(|v| v.contains("@emnapi/wasi-threads")));
}

#[test]
fn test_integration_v1_catalog_override_patch_combined() {
    // Test combining V1 format, catalogs, overrides, and patches
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "integration-test",
                "dependencies": {
                    "react": "catalog:ui",
                    "lodash": "catalog:"
                }
            },
            "packages/ui": {
                "name": "@repo/ui",
                "version": "1.0.0",
                "dependencies": {
                    "@repo/utils": "packages/utils",
                    "react": "catalog:ui"
                }
            },
            "packages/utils": {
                "name": "@repo/utils",
                "version": "2.0.0",
                "dependencies": {
                    "lodash": "catalog:"
                }
            }
        },
        "packages": {
            "react": ["react@18.0.0", {}, "sha512-react18"],
            "react-19": ["react@19.0.0", {}, "sha512-react19"],
            "lodash": ["lodash@4.17.20", {}, "sha512-lodash420"],
            "lodash-patched": ["lodash@4.17.21", {}, "sha512-lodash421"]
        },
        "catalog": {
            "lodash": "^4.17.20"
        },
        "catalogs": {
            "ui": {
                "react": "^18.0.0"
            }
        },
        "overrides": {
            "lodash": "4.17.21"
        },
        "patchedDependencies": {
            "lodash@4.17.21": "patches/lodash-security.patch"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Test catalog resolution with override
    let lodash_result = lockfile
        .resolve_package("", "lodash", "catalog:")
        .unwrap()
        .unwrap();
    // Should resolve catalog to 4.17.20, then override to 4.17.21, then apply patch
    assert_eq!(lodash_result.key, "lodash@4.17.21");
    assert_eq!(
        lodash_result.version,
        "4.17.21+patches/lodash-security.patch"
    );

    // Test V1 workspace dependency from packages/ui to packages/utils
    let utils_result = lockfile
        .resolve_package("packages/ui", "@repo/utils", "packages/utils")
        .unwrap()
        .unwrap();
    assert_eq!(utils_result.key, "@repo/utils@2.0.0");
    assert_eq!(utils_result.version, "2.0.0");

    // Test catalog resolution from named catalog
    let react_result = lockfile
        .resolve_package("packages/ui", "react", "catalog:ui")
        .unwrap()
        .unwrap();
    assert_eq!(react_result.key, "react@18.0.0");
    assert_eq!(react_result.version, "18.0.0");

    // Verify all fields are preserved
    assert_eq!(lockfile.data.lockfile_version, 1);
    assert_eq!(lockfile.data.overrides.len(), 1);
    assert_eq!(lockfile.data.catalog.len(), 1);
    assert_eq!(lockfile.data.catalogs.len(), 1);
    assert_eq!(lockfile.data.patched_dependencies.len(), 1);
}

#[test]
fn test_integration_complex_subgraph_filtering() {
    // Test subgraph filtering with all features enabled
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "complex-monorepo"
            },
            "apps/web": {
                "name": "web",
                "version": "1.0.0",
                "dependencies": {
                    "@repo/ui": "packages/ui",
                    "react": "catalog:frontend"
                }
            },
            "apps/api": {
                "name": "api",
                "version": "1.0.0",
                "dependencies": {
                    "@repo/shared": "packages/shared",
                    "express": "^4.18.0"
                }
            },
            "packages/ui": {
                "name": "@repo/ui",
                "version": "0.1.0",
                "dependencies": {
                    "@repo/shared": "packages/shared",
                    "react": "catalog:frontend"
                }
            },
            "packages/shared": {
                "name": "@repo/shared",
                "version": "0.2.0",
                "dependencies": {
                    "lodash": "catalog:"
                }
            }
        },
        "packages": {
            "react": ["react@18.0.0", {}, "sha512-react"],
            "react-19": ["react@19.0.0", {}, "sha512-react19"],
            "lodash": ["lodash@4.17.20", {}, "sha512-lodash"],
            "lodash-override": ["lodash@4.17.21", {}, "sha512-lodash21"],
            "express": ["express@4.18.0", {}, "sha512-express"]
        },
        "catalog": {
            "lodash": "^4.17.20"
        },
        "catalogs": {
            "frontend": {
                "react": "^18.0.0"
            }
        },
        "overrides": {
            "lodash": "4.17.21",
            "react": "19.0.0",
            "express": "4.18.0"
        },
        "patchedDependencies": {
            "lodash@4.17.21": "patches/lodash.patch",
            "express@4.18.0": "patches/express.patch"
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Create subgraph for web app only
    let subgraph = lockfile
        .subgraph(
            &[
                "apps/web".into(),
                "packages/ui".into(),
                "packages/shared".into(),
            ],
            &["react@19.0.0".into(), "lodash@4.17.21".into()],
        )
        .unwrap();
    let subgraph_data = subgraph.lockfile().unwrap();

    // Verify workspace filtering
    assert_eq!(subgraph_data.workspaces.len(), 4); // root + 3 specified
    assert!(subgraph_data.workspaces.contains_key(""));
    assert!(subgraph_data.workspaces.contains_key("apps/web"));
    assert!(subgraph_data.workspaces.contains_key("packages/ui"));
    assert!(subgraph_data.workspaces.contains_key("packages/shared"));
    assert!(!subgraph_data.workspaces.contains_key("apps/api"));

    // Verify package filtering
    assert_eq!(subgraph_data.packages.len(), 2);
    assert!(subgraph_data.packages.contains_key("react-19"));
    assert!(subgraph_data.packages.contains_key("lodash-override"));
    assert!(!subgraph_data.packages.contains_key("express"));

    // All overrides are preserved to stay in sync with root package.json
    assert_eq!(subgraph_data.overrides.len(), 3);
    assert!(subgraph_data.overrides.contains_key("react"));
    assert!(subgraph_data.overrides.contains_key("lodash"));
    assert!(subgraph_data.overrides.contains_key("express"));

    // Verify patches filtering
    assert_eq!(subgraph_data.patched_dependencies.len(), 1);
    assert!(
        subgraph_data
            .patched_dependencies
            .contains_key("lodash@4.17.21")
    );
    assert!(
        !subgraph_data
            .patched_dependencies
            .contains_key("express@4.18.0")
    );

    // Verify catalogs are preserved (they're kept for potential references)
    assert_eq!(subgraph_data.catalog.len(), 1);
    assert_eq!(subgraph_data.catalogs.len(), 1);
}

#[test]
fn test_subgraph_includes_transitive_dependencies() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test-root"
            },
            "apps/acme-client": {
                "name": "acme-client",
                "version": "0.0.1",
                "dependencies": {
                    "@hookform/resolvers": "^5.0.1"
                }
            }
        },
        "packages": {
            "@hookform/resolvers": ["@hookform/resolvers@5.2.2", "", {
                "dependencies": {
                    "@standard-schema/utils": "^0.3.0"
                },
                "peerDependencies": {
                    "react-hook-form": "^7.55.0"
                }
            }, "sha512-test"],
            "@standard-schema/utils": ["@standard-schema/utils@0.3.0", "", {}, "sha512-test2"],
            "react-hook-form": ["react-hook-form@7.62.0", "", {}, "sha512-test3"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Simulate what turbo prune would call: Get transitive closure first
    let unresolved_deps: std::collections::BTreeMap<String, String> =
        [("@hookform/resolvers".to_string(), "^5.0.1".to_string())]
            .into_iter()
            .collect();
    let closure =
        crate::transitive_closure(&lockfile, "apps/acme-client", unresolved_deps, false).unwrap();

    // Convert closure to idents
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

    // Create subgraph with the transitive closure
    let subgraph = lockfile
        .subgraph(&["apps/acme-client".into()], &package_idents)
        .unwrap();
    let subgraph_data = subgraph.lockfile().unwrap();

    // Verify @hookform/resolvers is included
    assert!(
        subgraph_data
            .packages
            .values()
            .any(|entry| entry.ident == "@hookform/resolvers@5.2.2"),
        "@hookform/resolvers should be in subgraph"
    );

    // Verify @standard-schema/utils is included (transitive dependency)
    assert!(
        subgraph_data
            .packages
            .values()
            .any(|entry| entry.ident == "@standard-schema/utils@0.3.0"),
        "@standard-schema/utils should be in subgraph as transitive dependency"
    );

    // Verify peer dependency is also included
    assert!(
        subgraph_data
            .packages
            .values()
            .any(|entry| entry.ident == "react-hook-form@7.62.0"),
        "react-hook-form should be in subgraph as peer dependency"
    );
}

/// Test that pruning a lockfile with GitHub dependencies doesn't corrupt
/// the format. GitHub packages should have 3 elements: [ident, info,
/// checksum] NOT 4 elements with an empty registry: [ident, "", info,
/// checksum]
#[test]
fn test_prune_github_package_format() {
    let lockfile_json = json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "root",
            },
            "packages/a": {
                "name": "pkg-a",
                "dependencies": {
                    "some-lib": "github:user/repo#abc123",
                },
            },
        },
        "packages": {
            "some-lib": [
                "some-lib@github:user/repo#abc123",
                { "dependencies": {} },
                "abc123"
            ],
        },
    });

    let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
    let subgraph = lockfile
        .subgraph(
            &["packages/a".into()],
            &["some-lib@github:user/repo#abc123".into()],
        )
        .unwrap();

    let encoded = String::from_utf8(subgraph.encode().unwrap()).unwrap();

    // Verify the GitHub package has exactly 3 elements (no empty string registry)
    // The output should contain the ident followed directly by the info object
    assert!(
        !encoded.contains(r#"["some-lib@github:user/repo#abc123", "", {"#),
        "GitHub package should NOT have empty string registry field"
    );
    assert!(
        encoded.contains(r#""some-lib": ["some-lib@github:user/repo#abc123", {"#),
        "GitHub package should have ident followed directly by info object"
    );
}

/// Test that metadata sections are preserved through encode round-trip.
/// Bun expects configVersion, trustedDependencies, overrides, and catalogs.
#[test]
fn test_encode_preserves_metadata_sections() {
    let lockfile_json = json!({
        "lockfileVersion": 1,
        "configVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
            },
        },
        "trustedDependencies": ["esbuild", "sharp"],
        "overrides": {
            "lodash": "4.17.21",
        },
        "catalog": {
            "react": "^18.0.0",
        },
        "packages": {},
    });

    let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
    let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

    // Verify configVersion is present
    assert!(
        encoded.contains(r#""configVersion": 1"#),
        "configVersion should be preserved in encoded output"
    );

    // Verify trustedDependencies section is present with its contents
    assert!(
        encoded.contains(r#""trustedDependencies""#),
        "trustedDependencies section should be present"
    );
    assert!(
        encoded.contains(r#""esbuild""#),
        "trustedDependencies should contain esbuild"
    );
    assert!(
        encoded.contains(r#""sharp""#),
        "trustedDependencies should contain sharp"
    );

    // Verify overrides section is present with its contents
    assert!(
        encoded.contains(r#""overrides""#),
        "overrides section should be present"
    );
    assert!(
        encoded.contains(r#""lodash""#),
        "overrides should contain lodash"
    );

    // Verify catalog section is present with its contents
    assert!(
        encoded.contains(r#""catalog""#),
        "catalog section should be present"
    );
    assert!(
        encoded.contains(r#""react""#),
        "catalog should contain react"
    );
}

/// Test that optionalPeers arrays use compact format without trailing
/// commas. Bun expects: ["react", "vue"] NOT [ "react", "vue", ]
#[test]
fn test_optional_peers_compact_format() {
    let lockfile_json = json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
            },
        },
        "packages": {
            "some-pkg": [
                "some-pkg@1.0.0",
                "",
                {
                    "peerDependencies": {
                        "react": "^18.0.0",
                        "vue": "^3.0.0",
                    },
                    "optionalPeers": ["react", "vue"],
                },
                "sha512-abc"
            ],
        },
    });

    let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
    let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

    // Verify optionalPeers uses compact format without leading/trailing spaces or
    // commas The array should be formatted as ["react", "vue"] or ["vue",
    // "react"] NOT as [ "react", "vue", ] or similar
    assert!(
        !encoded.contains(r#"[ ""#),
        "optionalPeers array should NOT have leading space after opening bracket"
    );
    assert!(
        !encoded.contains(r#", ]"#),
        "optionalPeers array should NOT have trailing comma before closing bracket"
    );

    // Verify the optionalPeers field exists and has content
    assert!(
        encoded.contains(r#""optionalPeers""#),
        "optionalPeers field should be present"
    );
}

/// Test that named catalogs (catalogs field) are preserved through encode.
/// This tests the plural "catalogs" field, not the singular "catalog"
/// field.
#[test]
fn test_encode_preserves_named_catalogs() {
    let lockfile_json = json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
            },
        },
        "catalog": {
            "lodash": "^4.17.0",
        },
        "catalogs": {
            "frontend": {
                "react": "^18.0.0",
                "vue": "^3.0.0",
            },
            "backend": {
                "express": "^4.18.0",
            },
        },
        "packages": {},
    });

    let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
    let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

    // Verify default catalog is present
    assert!(
        encoded.contains(r#""catalog""#),
        "default catalog section should be present"
    );
    assert!(
        encoded.contains(r#""lodash""#),
        "default catalog should contain lodash"
    );

    // Verify named catalogs section is present
    assert!(
        encoded.contains(r#""catalogs""#),
        "named catalogs section should be present"
    );

    // Verify frontend catalog entries
    assert!(
        encoded.contains(r#""frontend""#),
        "frontend catalog should be present"
    );
    assert!(
        encoded.contains(r#""react""#),
        "frontend catalog should contain react"
    );
    assert!(
        encoded.contains(r#""vue""#),
        "frontend catalog should contain vue"
    );

    // Verify backend catalog entries
    assert!(
        encoded.contains(r#""backend""#),
        "backend catalog should be present"
    );
    assert!(
        encoded.contains(r#""express""#),
        "backend catalog should contain express"
    );
}

/// Test that patched_dependencies are preserved through encode.
#[test]
fn test_encode_preserves_patched_dependencies() {
    let lockfile_json = json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "lodash": "^4.17.21",
                },
            },
        },
        "packages": {
            "lodash": [
                "lodash@4.17.21",
                "",
                {},
                "sha512-abc"
            ],
        },
        "patchedDependencies": {
            "lodash@4.17.21": "patches/lodash+4.17.21.patch",
        },
    });

    let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
    let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

    // Verify patchedDependencies section is present
    assert!(
        encoded.contains(r#""patchedDependencies""#),
        "patchedDependencies section should be present"
    );
    assert!(
        encoded.contains(r#""lodash@4.17.21""#),
        "patchedDependencies should contain lodash entry"
    );
    assert!(
        encoded.contains(r#"patches/lodash+4.17.21.patch"#),
        "patchedDependencies should contain patch path"
    );
}

/// Test that packages section is correctly encoded with proper format.
/// This verifies the packages field ordering and structure.
#[test]
fn test_encode_packages_structure() {
    let lockfile_json = json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test",
                "dependencies": {
                    "is-odd": "^3.0.0",
                },
            },
        },
        "packages": {
            "is-odd": [
                "is-odd@3.0.1",
                "",
                {
                    "dependencies": {
                        "is-number": "^6.0.0",
                    },
                },
                "sha512-def"
            ],
            "is-number": [
                "is-number@6.0.0",
                "",
                {},
                "sha512-ghi"
            ],
        },
    });

    let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
    let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

    // Verify packages section exists
    assert!(
        encoded.contains(r#""packages""#),
        "packages section should be present"
    );

    // Verify package entries are present with correct identifiers
    assert!(
        encoded.contains(r#""is-odd": ["is-odd@3.0.1""#),
        "is-odd package should be present with correct format"
    );
    assert!(
        encoded.contains(r#""is-number": ["is-number@6.0.0""#),
        "is-number package should be present with correct format"
    );

    // Verify packages have registry field (empty string for npm packages)
    assert!(
        encoded.contains(r#"["is-odd@3.0.1", "","#),
        "npm packages should have empty string registry field"
    );
}

/// Comprehensive test that all metadata sections are written in correct
/// order. Bun expects a specific ordering of top-level keys.
#[test]
fn test_encode_section_ordering() {
    let lockfile_json = json!({
        "lockfileVersion": 1,
        "configVersion": 1,
        "workspaces": {
            "": { "name": "test" },
        },
        "trustedDependencies": ["esbuild"],
        "overrides": { "lodash": "4.17.21" },
        "catalog": { "react": "^18.0.0" },
        "catalogs": {
            "frontend": { "vue": "^3.0.0" },
        },
        "packages": {
            "lodash": ["lodash@4.17.21", "", {}, "sha512-abc"],
        },
        "patchedDependencies": {
            "lodash@4.17.21": "patches/lodash.patch",
        },
    });

    let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
    let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

    // Find positions of each section to verify ordering
    let lockfile_version_pos = encoded.find(r#""lockfileVersion""#).unwrap();
    let config_version_pos = encoded.find(r#""configVersion""#).unwrap();
    let workspaces_pos = encoded.find(r#""workspaces""#).unwrap();
    let trusted_deps_pos = encoded.find(r#""trustedDependencies""#).unwrap();
    let overrides_pos = encoded.find(r#""overrides""#).unwrap();
    let catalog_pos = encoded.find(r#""catalog""#).unwrap();
    let catalogs_pos = encoded.find(r#""catalogs""#).unwrap();
    let packages_pos = encoded.find(r#""packages""#).unwrap();
    let patched_deps_pos = encoded.find(r#""patchedDependencies""#).unwrap();

    // Verify ordering: lockfileVersion < configVersion < workspaces <
    // trustedDependencies < overrides < catalog < catalogs < packages <
    // patchedDependencies
    assert!(
        lockfile_version_pos < config_version_pos,
        "lockfileVersion should come before configVersion"
    );
    assert!(
        config_version_pos < workspaces_pos,
        "configVersion should come before workspaces"
    );
    assert!(
        workspaces_pos < trusted_deps_pos,
        "workspaces should come before trustedDependencies"
    );
    assert!(
        trusted_deps_pos < overrides_pos,
        "trustedDependencies should come before overrides"
    );
    assert!(
        overrides_pos < catalog_pos,
        "overrides should come before catalog"
    );
    assert!(
        catalog_pos < catalogs_pos,
        "catalog should come before catalogs"
    );
    assert!(
        catalogs_pos < packages_pos,
        "catalogs should come before packages"
    );
    assert!(
        packages_pos < patched_deps_pos,
        "packages should come before patchedDependencies"
    );
}

#[test]
fn test_turbo_version_rejects_non_semver() {
    // Malicious version strings that could be used for RCE via npx should be
    // rejected
    let malicious_versions = [
        "file:./malicious.tgz",
        "https://evil.com/malicious.tgz",
        "git+https://github.com/evil/repo.git",
        "../../../etc/passwd",
        "1.0.0 && curl evil.com",
    ];

    for malicious_version in malicious_versions {
        let json = format!(
            r#"{{
  "lockfileVersion": 0,
  "workspaces": {{
    "": {{
      "name": "test"
    }}
  }},
  "packages": {{
    "turbo": ["turbo@{malicious_version}", "", {{}}, ""]
  }}
}}"#
        );
        let lockfile = BunLockfile::from_str(&json).unwrap();
        assert_eq!(
            lockfile.turbo_version(),
            None,
            "should reject malicious version: {}",
            malicious_version
        );
    }
}

// Regression test for https://github.com/vercel/turborepo/issues/11923
// When multiple workspaces depend on different versions of the same package
// (e.g., color-convert@3.x, color-convert@2.x, color-convert@1.x), pruning
// for a single workspace must preserve the nested key hierarchy so bun
// resolves the correct version for each parent.
#[test]
fn test_subgraph_multiple_versions_same_package() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "bug-repro",
                    "dependencies": {
                        "typescript": "^5.9.3"
                    }
                },
                "apps/app1": {
                    "name": "app1",
                    "dependencies": {
                        "color": "^5.0.3",
                        "express-winston": "4.2.0"
                    },
                    "peerDependencies": {
                        "typescript": "^5"
                    }
                },
                "apps/app2": {
                    "name": "app2",
                    "dependencies": {
                        "ansi-styles": "4.3.0"
                    },
                    "peerDependencies": {
                        "typescript": "^5"
                    }
                }
            },
            "packages": {
                "ansi-styles": ["ansi-styles@4.3.0", "", { "dependencies": { "color-convert": "^2.0.1" } }, "sha512-ansi"],
                "app1": ["app1@workspace:apps/app1"],
                "app2": ["app2@workspace:apps/app2"],
                "chalk": ["chalk@2.4.2", "", { "dependencies": { "ansi-styles": "^3.2.1", "escape-string-regexp": "^1.0.5", "supports-color": "^5.3.0" } }, "sha512-chalk"],
                "color": ["color@5.0.3", "", { "dependencies": { "color-convert": "^3.1.3", "color-string": "^2.1.3" } }, "sha512-color"],
                "color-convert": ["color-convert@3.1.3", "", { "dependencies": { "color-name": "^2.0.0" } }, "sha512-cc3"],
                "color-name": ["color-name@2.1.0", "", {}, "sha512-cn2"],
                "color-string": ["color-string@2.1.4", "", { "dependencies": { "color-name": "^2.0.0" } }, "sha512-cs"],
                "escape-string-regexp": ["escape-string-regexp@1.0.5", "", {}, "sha512-esr"],
                "express-winston": ["express-winston@4.2.0", "", { "dependencies": { "chalk": "^2.4.2", "lodash": "^4.17.21" } }, "sha512-ew"],
                "has-flag": ["has-flag@3.0.0", "", {}, "sha512-hf"],
                "lodash": ["lodash@4.17.23", "", {}, "sha512-lo"],
                "supports-color": ["supports-color@5.5.0", "", { "dependencies": { "has-flag": "^3.0.0" } }, "sha512-sc"],
                "typescript": ["typescript@5.9.3", "", {}, "sha512-ts"],
                "ansi-styles/color-convert": ["color-convert@2.0.1", "", { "dependencies": { "color-name": "~1.1.4" } }, "sha512-cc2"],
                "ansi-styles/color-convert/color-name": ["color-name@1.1.4", "", {}, "sha512-cn114"],
                "chalk/ansi-styles": ["ansi-styles@3.2.1", "", { "dependencies": { "color-convert": "^1.9.0" } }, "sha512-as3"],
                "chalk/ansi-styles/color-convert": ["color-convert@1.9.3", "", { "dependencies": { "color-name": "1.1.3" } }, "sha512-cc1"],
                "chalk/ansi-styles/color-convert/color-name": ["color-name@1.1.3", "", {}, "sha512-cn113"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Compute transitive closure for app1 (simulating turbo prune --scope=app1)
    let mut app1_deps = std::collections::BTreeMap::new();
    app1_deps.insert("color".to_string(), "^5.0.3".to_string());
    app1_deps.insert("express-winston".to_string(), "4.2.0".to_string());

    let closure = crate::transitive_closure(&lockfile, "apps/app1", app1_deps, false).unwrap();

    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

    // Create subgraph
    let subgraph = lockfile
        .subgraph(&["apps/app1".into()], &package_idents)
        .unwrap();

    // Verify the pruned lockfile round-trips correctly
    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();

    let reparsed =
        BunLockfile::from_str(&encoded_str).expect("pruned lockfile should be parseable");

    // app1 uses:
    //   color -> color-convert@3.1.3 -> color-name@2.1.0
    //   express-winston -> chalk -> chalk/ansi-styles@3.2.1
    //     -> chalk/ansi-styles/color-convert@1.9.3
    //     -> chalk/ansi-styles/color-convert/color-name@1.1.3

    let has_ident = |ident: &str| reparsed.data.packages.values().any(|e| e.ident == ident);

    assert!(
        has_ident("color-convert@3.1.3"),
        "color-convert@3.1.3 (for color@5) should be in subgraph"
    );

    assert!(
        has_ident("color-convert@1.9.3"),
        "color-convert@1.9.3 (for chalk/ansi-styles) should be in subgraph"
    );

    // color-convert@2.0.1 (used by ansi-styles@4.3.0, app2-only) should NOT be
    // present
    assert!(
        !has_ident("color-convert@2.0.1"),
        "color-convert@2.0.1 (app2 only) should NOT be in subgraph"
    );

    // ansi-styles@4.3.0 (app2-only) should NOT be present
    assert!(
        !has_ident("ansi-styles@4.3.0"),
        "ansi-styles@4.3.0 (app2 only) should NOT be in subgraph"
    );

    assert!(
        has_ident("color-name@1.1.3"),
        "color-name@1.1.3 (for chalk chain) should be in subgraph"
    );

    // color should resolve to 5.0.3
    let color_dep = reparsed
        .resolve_package("apps/app1", "color", "^5.0.3")
        .unwrap()
        .expect("should resolve color");
    assert_eq!(color_dep.key, "color@5.0.3");

    // chalk should resolve to 2.4.2
    let chalk_dep = reparsed
        .resolve_package("apps/app1", "chalk", "^2.4.2")
        .unwrap()
        .expect("should resolve chalk");
    assert_eq!(chalk_dep.key, "chalk@2.4.2");
}

// Regression test for https://github.com/vercel/turborepo/issues/12744
// Bun resolves nested package dependencies from ancestor scopes too. If
// `npm/@npmcli/arborist` depends on `@npmcli/metavuln-calculator`, the
// matching entry is `npm/@npmcli/metavuln-calculator`, not
// `npm/@npmcli/arborist/@npmcli/metavuln-calculator`.
#[test]
fn test_subgraph_preserves_ancestor_scoped_nested_dependencies() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-monorepo"
                },
                "apps/app": {
                    "name": "app",
                    "dependencies": {
                        "npm": "^11.0.0"
                    }
                }
            },
            "packages": {
                "app": ["app@workspace:apps/app"],
                "npm": ["npm@11.0.0", "", { "dependencies": { "@npmcli/arborist": "^8.0.0", "postcss-selector-parser": "^7.0.0" } }, "sha512-npm"],
                "npm/@npmcli/arborist": ["@npmcli/arborist@8.0.5", "", { "dependencies": { "@npmcli/metavuln-calculator": "^8.0.0" }, "bundled": true }, "sha512-arborist"],
                "npm/@npmcli/metavuln-calculator": ["@npmcli/metavuln-calculator@8.0.1", "", {}, "sha512-metavuln"],
                "npm/cssesc": ["cssesc@3.0.0", "", {}, "sha512-cssesc"],
                "npm/postcss-selector-parser": ["postcss-selector-parser@7.1.1", "", { "dependencies": { "cssesc": "^3.0.0" } }, "sha512-postcss"],
                "@oclif/plugin-plugins/npm/cssesc": ["cssesc@3.0.0", "", {}, "sha512-cssesc"],
                "@oclif/plugin-plugins/npm/postcss-selector-parser": ["postcss-selector-parser@7.1.1", "", { "dependencies": { "cssesc": "^3.0.0" } }, "sha512-postcss"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();
    let mut app_deps = std::collections::BTreeMap::new();
    app_deps.insert("npm".to_string(), "^11.0.0".to_string());

    let closure = crate::transitive_closure(&lockfile, "apps/app", app_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();
    let subgraph = lockfile
        .subgraph(&["apps/app".into()], &package_idents)
        .unwrap();
    let subgraph_data = subgraph.lockfile().unwrap();

    assert!(subgraph_data.packages.contains_key("npm/@npmcli/arborist"));
    assert!(
        subgraph_data
            .packages
            .contains_key("npm/@npmcli/metavuln-calculator"),
        "ancestor-scoped nested dependency should remain in pruned lockfile"
    );
    assert!(
        subgraph_data.packages.contains_key("npm/cssesc"),
        "exact source key should win when duplicate idents exist in other nested scopes"
    );
}

// Regression test for https://github.com/vercel/turborepo/issues/12156
// When pruning removes the hoisted version of a package but keeps a nested
// version, the nested entry must be promoted to top-level so that bun's
// --frozen-lockfile doesn't reject the structural change.
#[test]
fn test_nested_entries_promoted_when_hoisted_version_pruned() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-monorepo",
                    "dependencies": {
                        "typescript": "^5.0.0"
                    }
                },
                "apps/web": {
                    "name": "web",
                    "dependencies": {
                        "express": "^4.18.0"
                    },
                    "peerDependencies": {
                        "typescript": "^5"
                    }
                },
                "apps/admin": {
                    "name": "admin",
                    "dependencies": {
                        "compression": "^1.8.0"
                    },
                    "peerDependencies": {
                        "typescript": "^5"
                    }
                }
            },
            "packages": {
                "accepts": ["accepts@1.3.8", "", { "dependencies": { "mime-types": "~2.1.34", "negotiator": "0.6.3" } }, "sha512-accepts"],
                "admin": ["admin@workspace:apps/admin"],
                "body-parser": ["body-parser@1.20.3", "", { "dependencies": { "debug": "2.6.9", "depd": "2.0.0" } }, "sha512-bp"],
                "bytes": ["bytes@3.1.2", "", {}, "sha512-bytes"],
                "compressible": ["compressible@2.0.18", "", { "dependencies": { "mime-db": "~1.52.0" } }, "sha512-compress"],
                "compression": ["compression@1.8.1", "", { "dependencies": { "bytes": "3.1.2", "compressible": "~2.0.18", "debug": "2.6.9", "negotiator": "~0.6.4", "on-headers": "~1.1.0", "safe-buffer": "5.2.1", "vary": "~1.1.2" } }, "sha512-compression"],
                "debug": ["debug@2.6.9", "", { "dependencies": { "ms": "2.0.0" } }, "sha512-debug"],
                "depd": ["depd@2.0.0", "", {}, "sha512-depd"],
                "express": ["express@4.21.2", "", { "dependencies": { "accepts": "~1.3.8", "body-parser": "1.20.3" } }, "sha512-express"],
                "mime-db": ["mime-db@1.52.0", "", {}, "sha512-mimedb"],
                "mime-types": ["mime-types@2.1.35", "", { "dependencies": { "mime-db": "1.52.0" } }, "sha512-mimetypes"],
                "ms": ["ms@2.0.0", "", {}, "sha512-ms"],
                "negotiator": ["negotiator@0.6.4", "", {}, "sha512-neg064"],
                "on-headers": ["on-headers@1.1.0", "", {}, "sha512-onh"],
                "safe-buffer": ["safe-buffer@5.2.1", "", {}, "sha512-sb"],
                "typescript": ["typescript@5.9.3", "", {}, "sha512-ts"],
                "vary": ["vary@1.1.2", "", {}, "sha512-vary"],
                "web": ["web@workspace:apps/web"],
                "accepts/negotiator": ["negotiator@0.6.3", "", {}, "sha512-neg063"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Prune for apps/web only. The transitive closure includes
    // negotiator@0.6.3 (via accepts) but NOT negotiator@0.6.4 (only
    // needed by compression, which is in apps/admin).
    let mut web_deps = std::collections::BTreeMap::new();
    web_deps.insert("express".to_string(), "^4.18.0".to_string());
    let closure = crate::transitive_closure(&lockfile, "apps/web", web_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

    let subgraph = lockfile
        .subgraph(&["apps/web".into()], &package_idents)
        .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).expect("pruned lockfile should be parseable");

    // The nested entry "accepts/negotiator" should be promoted to
    // "negotiator" since the hoisted negotiator@0.6.4 was pruned.
    assert!(
        pruned.data.packages.contains_key("negotiator"),
        "negotiator should exist as a top-level entry after promotion"
    );
    assert!(
        !pruned.data.packages.contains_key("accepts/negotiator"),
        "nested accepts/negotiator should have been promoted to top-level"
    );
    assert_eq!(
        pruned.data.packages["negotiator"].ident, "negotiator@0.6.3",
        "promoted entry should have the correct ident"
    );
}

// Test nested key promotion with deeply nested chains
#[test]
fn test_nested_promotion_deep_chain() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "deep-chain-test",
                    "dependencies": {}
                },
                "apps/web": {
                    "name": "web",
                    "dependencies": {
                        "express-winston": "4.2.0"
                    }
                },
                "apps/other": {
                    "name": "other",
                    "dependencies": {
                        "ansi-styles": "4.3.0",
                        "color-convert": "2.0.1"
                    }
                }
            },
            "packages": {
                "ansi-styles": ["ansi-styles@4.3.0", "", { "dependencies": { "color-convert": "^2.0.1" } }, "sha512-as4"],
                "chalk": ["chalk@2.4.2", "", { "dependencies": { "ansi-styles": "^3.2.1" } }, "sha512-chalk"],
                "color-convert": ["color-convert@2.0.1", "", { "dependencies": { "color-name": "~1.1.4" } }, "sha512-cc2"],
                "color-name": ["color-name@2.1.0", "", {}, "sha512-cn2"],
                "express-winston": ["express-winston@4.2.0", "", { "dependencies": { "chalk": "^2.4.2" } }, "sha512-ew"],
                "other": ["other@workspace:apps/other"],
                "web": ["web@workspace:apps/web"],
                "chalk/ansi-styles": ["ansi-styles@3.2.1", "", { "dependencies": { "color-convert": "^1.9.0" } }, "sha512-as3"],
                "chalk/ansi-styles/color-convert": ["color-convert@1.9.3", "", { "dependencies": { "color-name": "1.1.3" } }, "sha512-cc1"],
                "chalk/ansi-styles/color-convert/color-name": ["color-name@1.1.3", "", {}, "sha512-cn113"],
                "color-convert/color-name": ["color-name@1.1.4", "", {}, "sha512-cn114"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Prune for apps/web. The transitive closure includes chalk ->
    // chalk/ansi-styles -> chalk/ansi-styles/color-convert ->
    // chalk/ansi-styles/color-convert/color-name but NOT the hoisted
    // ansi-styles@4.3.0, color-convert@2.0.1, or color-name@2.1.0.
    let mut web_deps = std::collections::BTreeMap::new();
    web_deps.insert("express-winston".to_string(), "4.2.0".to_string());
    let closure = crate::transitive_closure(&lockfile, "apps/web", web_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

    let subgraph = lockfile
        .subgraph(&["apps/web".into()], &package_idents)
        .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).expect("pruned lockfile should be parseable");

    // After promotion, deeply nested entries should be flattened.
    // "chalk/ansi-styles" → "ansi-styles" (promoted, children renamed)
    // "ansi-styles/color-convert" → "color-convert" (promoted, children renamed)
    // "color-convert/color-name" → "color-name" (promoted)
    assert!(
        pruned.data.packages.contains_key("ansi-styles"),
        "ansi-styles should be promoted to top-level"
    );
    assert!(
        pruned.data.packages.contains_key("color-convert"),
        "color-convert should be promoted to top-level"
    );
    assert!(
        pruned.data.packages.contains_key("color-name"),
        "color-name should be promoted to top-level"
    );

    // No nested keys should remain for these packages
    assert!(
        !pruned.data.packages.contains_key("chalk/ansi-styles"),
        "chalk/ansi-styles should have been promoted"
    );

    // Verify idents
    assert_eq!(
        pruned.data.packages["ansi-styles"].ident,
        "ansi-styles@3.2.1"
    );
    assert_eq!(
        pruned.data.packages["color-convert"].ident,
        "color-convert@1.9.3"
    );
    assert_eq!(pruned.data.packages["color-name"].ident, "color-name@1.1.3");
}

// Regression test for https://github.com/vercel/turborepo/issues/11701
// file: protocol dependencies should be serialized as 2-element arrays
// [ident, info], not corrupted into 4-element arrays [ident, "", info, ""]
#[test]
fn test_file_protocol_roundtrip() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "file-dep-fixture",
                },
                "apps/api": {
                    "name": "api",
                    "dependencies": {
                        "@api/sdk": "file:apps/api/.api/apis/sdk",
                        "lodash": "^4.17.21"
                    }
                },
                "packages/ui": {
                    "name": "@repo/ui",
                    "dependencies": {
                        "lodash": "^4.17.21"
                    }
                }
            },
            "packages": {
                "@api/sdk": ["@api/sdk@file:apps/api/.api/apis/sdk", { "dependencies": { "cross-fetch": "^3.1.5" } }],
                "@repo/ui": ["@repo/ui@workspace:packages/ui"],
                "api": ["api@workspace:apps/api"],
                "cross-fetch": ["cross-fetch@3.1.8", "", { "dependencies": { "node-fetch": "^2.6.12" } }, "sha512-crossfetch"],
                "lodash": ["lodash@4.17.23", "", {}, "sha512-lodash"],
                "node-fetch": ["node-fetch@2.7.0", "", {}, "sha512-nodefetch"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Verify the file: entry was parsed correctly
    let sdk_entry = &lockfile.data.packages["@api/sdk"];
    assert_eq!(sdk_entry.ident, "@api/sdk@file:apps/api/.api/apis/sdk");
    assert!(
        sdk_entry.registry.is_none(),
        "file: packages should not have registry"
    );
    assert!(
        sdk_entry.checksum.is_none(),
        "file: packages should not have checksum"
    );

    // Prune to just the api workspace
    let mut api_deps = std::collections::BTreeMap::new();
    api_deps.insert(
        "@api/sdk".to_string(),
        "file:apps/api/.api/apis/sdk".to_string(),
    );
    api_deps.insert("lodash".to_string(), "^4.17.21".to_string());

    let closure = crate::transitive_closure(&lockfile, "apps/api", api_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

    let subgraph = lockfile
        .subgraph(&["apps/api".into()], &package_idents)
        .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();

    // The encoded lockfile must be reparseable
    let reparsed = BunLockfile::from_str(&encoded_str)
        .expect("pruned lockfile with file: deps should be parseable");

    // Verify the file: entry survived the roundtrip as a 2-element array
    let sdk_reparsed = &reparsed.data.packages["@api/sdk"];
    assert_eq!(sdk_reparsed.ident, "@api/sdk@file:apps/api/.api/apis/sdk");
    assert!(
        sdk_reparsed.registry.is_none(),
        "file: packages should not gain a registry after roundtrip"
    );
    assert!(
        sdk_reparsed.checksum.is_none(),
        "file: packages should not gain a checksum after roundtrip"
    );

    // Verify the encoded string does NOT contain empty strings for the file: entry
    // A correct entry looks like: ["@api/sdk@file:apps/api/.api/apis/sdk", { ... }]
    // A corrupted entry looks like: ["@api/sdk@file:apps/api/.api/apis/sdk", "", {
    // ... }, ""]
    assert!(
        !encoded_str.contains(r#""@api/sdk@file:apps/api/.api/apis/sdk", """#),
        "file: entry should not have empty registry string inserted"
    );
}

// Regression test: link: protocol dependencies should also be 2-element arrays
#[test]
fn test_link_protocol_roundtrip() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "link-dep-fixture",
            },
            "apps/web": {
                "name": "web",
                "dependencies": {
                    "my-local-pkg": "link:../../local-pkg"
                }
            }
        },
        "packages": {
            "my-local-pkg": ["my-local-pkg@link:../../local-pkg", {}],
            "web": ["web@workspace:apps/web"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();
    let subgraph = lockfile
        .subgraph(
            &["apps/web".into()],
            &["my-local-pkg@link:../../local-pkg".into()],
        )
        .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();

    BunLockfile::from_str(&encoded_str)
        .expect("pruned lockfile with link: deps should be parseable");

    assert!(
        !encoded_str.contains(r#""my-local-pkg@link:../../local-pkg", """#),
        "link: entry should not have empty registry string inserted"
    );
}

// Regression test for https://github.com/vercel/turborepo/issues/12252
// When two workspaces have workspace-scoped entries for the same package
// at different versions, and a shared transitive dependency references that
// package with a wide semver range, the transitive closure must resolve
// to the correct workspace-scoped version for each workspace — not whichever
// version happened to be cached first during parallel processing.
#[test]
fn test_all_transitive_closures_deterministic_with_workspace_scoped_packages() {
    use std::collections::{BTreeMap, HashMap, HashSet};

    let lockfile_content = r#"{
            "lockfileVersion": 1,
            "workspaces": {
                "": { "name": "test-monorepo" },
                "apps/app-a": {
                    "name": "app-a",
                    "dependencies": { "shared": "^1.0.0" }
                },
                "apps/app-b": {
                    "name": "app-b",
                    "dependencies": { "shared": "^1.0.0" }
                }
            },
            "packages": {
                "app-a": ["app-a@workspace:apps/app-a"],
                "app-b": ["app-b@workspace:apps/app-b"],
                "shared": ["shared@1.0.0", "", {
                    "dependencies": { "lib": ">=1.0.0" }
                }, "sha512-shared"],
                "lib": ["lib@2.0.0", "", {}, "sha512-lib2"],
                "app-a/lib": ["lib@2.0.0", "", {}, "sha512-lib2"],
                "app-b/lib": ["lib@1.0.0", "", {}, "sha512-lib1"]
            }
        }"#;

    let lockfile = BunLockfile::from_str(lockfile_content).unwrap();

    let mut workspaces = HashMap::new();
    workspaces.insert(
        "apps/app-a".to_string(),
        BTreeMap::from([("shared".to_string(), "^1.0.0".to_string())]),
    );
    workspaces.insert(
        "apps/app-b".to_string(),
        BTreeMap::from([("shared".to_string(), "^1.0.0".to_string())]),
    );

    // Run multiple times to catch race-condition-driven non-determinism.
    // Before the fix, the result would vary depending on which workspace
    // populated the shared resolve cache first.
    let mut prev_app_a: Option<HashSet<crate::Package>> = None;
    let mut prev_app_b: Option<HashSet<crate::Package>> = None;

    for _ in 0..50 {
        let result = crate::all_transitive_closures(&lockfile, workspaces.clone(), false).unwrap();

        let app_a_pkgs = result.get("apps/app-a").unwrap();
        let app_b_pkgs = result.get("apps/app-b").unwrap();

        assert!(
            app_a_pkgs.iter().any(|p| p.key == "lib@2.0.0"),
            "app-a should have lib@2.0.0 via workspace-scoped resolution"
        );
        assert!(
            app_b_pkgs.iter().any(|p| p.key == "lib@1.0.0"),
            "app-b should have lib@1.0.0 via workspace-scoped resolution"
        );

        if let Some(ref prev) = prev_app_a {
            assert_eq!(app_a_pkgs, prev, "app-a closure changed between runs");
        }
        if let Some(ref prev) = prev_app_b {
            assert_eq!(app_b_pkgs, prev, "app-b closure changed between runs");
        }

        prev_app_a = Some(app_a_pkgs.clone());
        prev_app_b = Some(app_b_pkgs.clone());
    }
}

// Regression test for https://github.com/vercel/turborepo/issues/12816
// Duplicate alias keys that point at the same package need matching nested
// children. If pruning keeps `string-width` and `string-width-cjs` but only
// one `*/emoji-regex` child, Bun rewrites the pruned lockfile.
#[test]
fn test_subgraph_preserves_duplicate_alias_nested_children() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "monorepo",
                    "devDependencies": {
                        "root-tool": "1.0.0"
                    }
                },
                "apps/api": {
                    "name": "api",
                    "dependencies": {
                        "api-tool": "1.0.0"
                    }
                },
                "apps/front": {
                    "name": "front",
                    "dependencies": {
                        "front-tool": "1.0.0"
                    }
                }
            },
            "packages": {
                "api": ["api@workspace:apps/api"],
                "api-tool": ["api-tool@1.0.0", "", { "dependencies": { "string-width": "^4.2.0" } }, "sha512-api"],
                "emoji-regex": ["emoji-regex@9.2.2", "", {}, "sha512-emoji9"],
                "front": ["front@workspace:apps/front"],
                "front-tool": ["front-tool@1.0.0", "", {}, "sha512-front"],
                "root-tool": ["root-tool@1.0.0", "", { "dependencies": { "emoji-regex": "^9.2.2", "string-width-cjs": "npm:string-width@^4.2.0" } }, "sha512-root"],
                "string-width": ["string-width@4.2.3", "", { "dependencies": { "emoji-regex": "^8.0.0" } }, "sha512-string-width"],
                "string-width-cjs": ["string-width@4.2.3", "", { "dependencies": { "emoji-regex": "^8.0.0" } }, "sha512-string-width"],
                "string-width/emoji-regex": ["emoji-regex@8.0.0", "", {}, "sha512-emoji8"],
                "string-width-cjs/emoji-regex": ["emoji-regex@8.0.0", "", {}, "sha512-emoji8"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();
    let mut api_deps = std::collections::BTreeMap::new();
    api_deps.insert("api-tool".to_string(), "1.0.0".to_string());

    let closure = crate::transitive_closure(&lockfile, "apps/api", api_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();
    let subgraph = <BunLockfile as crate::Lockfile>::subgraph(
        &lockfile,
        &["apps/api".into()],
        &package_idents,
    )
    .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).unwrap();

    assert!(pruned.data.packages.contains_key("string-width"));
    assert!(pruned.data.packages.contains_key("string-width-cjs"));
    assert!(
        pruned
            .data
            .packages
            .contains_key("string-width/emoji-regex")
    );
    assert!(
        pruned
            .data
            .packages
            .contains_key("string-width-cjs/emoji-regex"),
        "alias-specific nested child should be restored when its sibling remains"
    );
}

#[test]
fn test_subgraph_preserves_nested_workspace_dependency_version() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "configVersion": 1,
        "workspaces": {
            "": {
                "name": "root"
            },
            "packages/a": {
                "name": "a",
                "version": "0.0.1",
                "dependencies": {
                    "b": "*"
                },
                "devDependencies": {
                    "b": "workspace:*",
                    "is-number": "^7.0.0"
                }
            },
            "packages/b": {
                "name": "b",
                "version": "0.0.1",
                "devDependencies": {
                    "is-number": "6.0.0"
                },
                "peerDependencies": {
                    "is-number": "6.0.0"
                }
            }
        },
        "packages": {
            "a": ["a@workspace:packages/a"],
            "b": ["b@workspace:packages/b"],
            "is-number": ["is-number@7.0.0", "", {}, "sha512-7"],
            "b/is-number": ["is-number@6.0.0", "", {}, "sha512-6"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();
    let subgraph = <BunLockfile as crate::Lockfile>::subgraph(
        &lockfile,
        &["packages/a".into(), "packages/b".into()],
        &["is-number@7.0.0".into()],
    )
    .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).unwrap();

    assert_eq!(pruned.data.packages["is-number"].ident, "is-number@7.0.0");
    assert_eq!(pruned.data.packages["b/is-number"].ident, "is-number@6.0.0");
}

// Regression test for https://github.com/vercel/turborepo/issues/12962
#[test]
fn test_subgraph_preserves_hoisted_transitive_version_with_nested_mismatch() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "turbo-prune-bun-nested-major-mismatch",
                    "dependencies": {
                        "bs58": "5.0.0"
                    }
                },
                "apps/app": {
                    "name": "@repro/app",
                    "version": "0.0.0",
                    "dependencies": {
                        "@repro/lib": "workspace:*",
                        "bs58": "6.0.0"
                    }
                },
                "packages/lib": {
                    "name": "@repro/lib",
                    "version": "0.0.0",
                    "dependencies": {
                        "bs58": "5.0.0"
                    }
                }
            },
            "packages": {
                "@repro/app": ["@repro/app@workspace:apps/app"],
                "@repro/lib": ["@repro/lib@workspace:packages/lib"],
                "base-x": ["base-x@4.0.1", "", {}, "sha512-base-x-4"],
                "bs58": ["bs58@5.0.0", "", { "dependencies": { "base-x": "^4.0.0" } }, "sha512-bs58-5"],
                "@repro/app/bs58": ["bs58@6.0.0", "", { "dependencies": { "base-x": "^5.0.0" } }, "sha512-bs58-6"],
                "@repro/app/bs58/base-x": ["base-x@5.0.1", "", {}, "sha512-base-x-5"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();
    let subgraph = <BunLockfile as crate::Lockfile>::subgraph(
        &lockfile,
        &["apps/app".into(), "packages/lib".into()],
        &[],
    )
    .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).unwrap();

    assert!(
        pruned.data.packages.contains_key("base-x"),
        "hoisted base-x@4 used by bs58@5 must remain in the pruned lockfile"
    );
    assert!(
        pruned.data.packages.contains_key("@repro/app/bs58/base-x"),
        "nested base-x@5 used by bs58@6 must remain in the pruned lockfile"
    );

    let lib_base_x = pruned
        .resolve_package("packages/lib", "base-x", "^4.0.0")
        .unwrap()
        .expect("bs58@5 should resolve base-x@4");
    assert_eq!(lib_base_x.key, "base-x@4.0.1");

    let app_base_x = pruned
        .resolve_package("apps/app", "base-x", "=5.0.1")
        .unwrap()
        .expect("bs58@6 should resolve base-x@5");
    assert_eq!(app_base_x.key, "base-x@5.0.1");
}

// Regression test for https://github.com/vercel/turborepo/issues/13101
// When a patched package exists at two versions in the prune closure, the
// patched hoisted version must be preserved and patchedDependencies must not
// be dropped.
#[test]
fn test_subgraph_preserves_patched_dependency_when_two_versions_exist() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "configVersion": 1,
        "workspaces": {
            "": {
                "name": "turbo-prune-patch-repro",
                "devDependencies": {
                    "turbo": "2.9.14"
                }
            },
            "apps/app-a": {
                "name": "app-a",
                "version": "0.0.0",
                "dependencies": {
                    "is-odd": "3.0.1",
                    "pkg-old": "workspace:*"
                }
            },
            "packages/pkg-old": {
                "name": "pkg-old",
                "version": "0.0.0",
                "dependencies": {
                    "is-odd": "2.0.0"
                }
            }
        },
        "patchedDependencies": {
            "is-odd@3.0.1": "patches/is-odd@3.0.1.patch"
        },
        "packages": {
            "app-a": ["app-a@workspace:apps/app-a"],
            "is-number": ["is-number@6.0.0", "", {}, "sha512-is-number-6"],
            "is-odd": ["is-odd@3.0.1", "", { "dependencies": { "is-number": "^6.0.0" } }, "sha512-is-odd-3"],
            "pkg-old": ["pkg-old@workspace:packages/pkg-old"],
            "pkg-old/is-odd": ["is-odd@2.0.0", "", { "dependencies": { "is-number": "^4.0.0" } }, "sha512-is-odd-2"],
            "pkg-old/is-odd/is-number": ["is-number@4.0.0", "", {}, "sha512-is-number-4"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Simulate lockfile_keys from `turbo prune` (external transitive deps only).
    let lockfile_keys: Vec<String> = [
        "is-number@4.0.0",
        "is-number@6.0.0",
        "is-odd@2.0.0",
        "is-odd@3.0.1",
        "turbo@2.9.14",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    let subgraph = Lockfile::subgraph(
        &lockfile,
        &["apps/app-a".into(), "packages/pkg-old".into()],
        &lockfile_keys,
    )
    .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).unwrap();

    assert!(
        pruned
            .data
            .patched_dependencies
            .contains_key("is-odd@3.0.1"),
        "patchedDependencies should retain is-odd@3.0.1, got {:?}",
        pruned.data.patched_dependencies
    );
    assert_eq!(
        pruned.data.packages.get("is-odd").map(|e| e.ident.as_str()),
        Some("is-odd@3.0.1"),
        "hoisted is-odd should remain the patched 3.0.1 version, got {:?}",
        pruned.data.packages.get("is-odd")
    );
    assert!(
        pruned.data.packages.contains_key("pkg-old/is-odd"),
        "nested is-odd@2.0.0 should remain under pkg-old, packages: {:?}",
        pruned
            .data
            .packages
            .iter()
            .filter(|(k, _)| k.contains("is-odd"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        pruned
            .data
            .packages
            .get("pkg-old/is-odd")
            .map(|e| e.ident.as_str()),
        Some("is-odd@2.0.0")
    );
}

// When the patched hoisted version is missing from lockfile_keys but still
// required by a workspace importer, prune must retain the patch and the
// patched version instead of promoting the nested alternative.
#[test]
fn test_subgraph_preserves_patch_when_patched_version_missing_from_lockfile_keys() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "configVersion": 1,
        "workspaces": {
            "": {
                "name": "turbo-prune-patch-repro",
                "devDependencies": {
                    "turbo": "2.9.14"
                }
            },
            "apps/app-a": {
                "name": "app-a",
                "version": "0.0.0",
                "dependencies": {
                    "is-odd": "^3.0.1",
                    "pkg-old": "workspace:*"
                }
            },
            "packages/pkg-old": {
                "name": "pkg-old",
                "version": "0.0.0",
                "dependencies": {
                    "is-odd": "2.0.0"
                }
            }
        },
        "patchedDependencies": {
            "is-odd@3.0.1": "patches/is-odd@3.0.1.patch"
        },
        "packages": {
            "app-a": ["app-a@workspace:apps/app-a"],
            "is-number": ["is-number@6.0.0", "", {}, "sha512-is-number-6"],
            "is-odd": ["is-odd@3.0.1", "", { "dependencies": { "is-number": "^6.0.0" } }, "sha512-is-odd-3"],
            "pkg-old": ["pkg-old@workspace:packages/pkg-old"],
            "pkg-old/is-odd": ["is-odd@2.0.0", "", { "dependencies": { "is-number": "^4.0.0" } }, "sha512-is-odd-2"],
            "pkg-old/is-odd/is-number": ["is-number@4.0.0", "", {}, "sha512-is-number-4"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Omit is-odd@3.0.1 from lockfile_keys, as can happen when only the nested
    // version appears in the package graph's transitive external dependencies.
    let lockfile_keys: Vec<String> = ["is-number@4.0.0", "is-odd@2.0.0", "turbo@2.9.14"]
        .into_iter()
        .map(String::from)
        .collect();

    let subgraph = Lockfile::subgraph(
        &lockfile,
        &["apps/app-a".into(), "packages/pkg-old".into()],
        &lockfile_keys,
    )
    .unwrap();

    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).unwrap();

    assert!(
        pruned
            .data
            .patched_dependencies
            .contains_key("is-odd@3.0.1"),
        "patchedDependencies should retain is-odd@3.0.1, got {:?}",
        pruned.data.patched_dependencies
    );
    assert_eq!(
        pruned.data.packages.get("is-odd").map(|e| e.ident.as_str()),
        Some("is-odd@3.0.1"),
        "hoisted is-odd should remain the patched 3.0.1 version, got {:?}",
        pruned.data.packages.get("is-odd")
    );
    assert!(
        pruned.data.packages.contains_key("pkg-old/is-odd"),
        "nested is-odd@2.0.0 should remain under pkg-old"
    );
}

#[test]
fn test_parses_v2_lockfile_and_resolves_workspace_and_external() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 2,
        "configVersion": 1,
        "workspaces": {
            "": {
                "name": "root",
                "devDependencies": { "is-odd": "3.0.1" },
            },
            "packages/a": {
                "name": "a",
                "version": "0.0.0",
                "dependencies": { "is-number": "^6.0.0" },
            },
        },
        "packages": {
            "a": ["a@workspace:packages/a"],
            "is-number": ["is-number@6.0.0", "", {}, "sha512-stub"],
            "is-odd": [
                "is-odd@3.0.1",
                "",
                { "dependencies": { "is-number": "^6.0.0" } },
                "sha512-stub",
            ],
        },
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).expect("Bun lockfileVersion 2 should parse");
    assert_eq!(lockfile.data.lockfile_version, 2);
    assert_eq!(lockfile.data.config_version, Some(1));

    // Workspace-direct resolution (the V1 optimization path) is gated on
    // `lockfile_version >= 1`, so it must keep firing for V2.
    // `VersionSpec::Workspace` is parsed from a bare workspace path; matches
    // how every other test exercises it.
    let workspace_dep = lockfile
        .resolve_package("", "a", "packages/a")
        .unwrap()
        .expect("workspace dep resolves on V2");
    assert_eq!(workspace_dep.key, "a@0.0.0");

    // Hoisted external resolution is shared with V1; spot-check one entry.
    let external = lockfile
        .resolve_package("packages/a", "is-number", "^6.0.0")
        .unwrap()
        .expect("hoisted external resolves on V2");
    assert_eq!(external.version, "6.0.0");
}

/// Regression for a 3-level nested version split that `turbo prune` drops.
///
/// `@vite-pwa/nuxt@1` depends on `pathe@^1` (direct) AND `@nuxt/kit@^3`, while
/// the workspace also depends on `@nuxt/kit@^4`. The hoisted `@nuxt/kit@4`
/// keeps `@nuxt/kit@3` nested under `@vite-pwa/nuxt`, and that nested
/// `@nuxt/kit@3` needs `pathe@^2` — recorded at the 3-level key
/// `@vite-pwa/nuxt/@nuxt/kit/pathe`. Prune must preserve that key, otherwise
/// bun resolves the nested `@nuxt/kit@3`'s pathe to the nearest ancestor
/// `@vite-pwa/nuxt/pathe@1.1.2` (wrong major).
#[test]
fn test_prune_preserves_three_level_nested_version() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": { "name": "root" },
            "apps/web": {
                "name": "web",
                "dependencies": {
                    "@nuxt/kit": "^4.4.8",
                    "@vite-pwa/nuxt": "1.1.1"
                }
            }
        },
        "packages": {
            "@nuxt/kit": ["@nuxt/kit@4.4.8", "", { "dependencies": { "pathe": "^2.0.3" } }, "sha512-a"],
            "@vite-pwa/nuxt": ["@vite-pwa/nuxt@1.1.1", "", { "dependencies": { "@nuxt/kit": "^3.9.0", "pathe": "^1.1.1" } }, "sha512-b"],
            "pathe": ["pathe@2.0.3", "", {}, "sha512-c"],
            "@vite-pwa/nuxt/@nuxt/kit": ["@nuxt/kit@3.21.8", "", { "dependencies": { "pathe": "^2.0.3" } }, "sha512-d"],
            "@vite-pwa/nuxt/pathe": ["pathe@1.1.2", "", {}, "sha512-e"],
            "@vite-pwa/nuxt/@nuxt/kit/pathe": ["pathe@2.0.3", "", {}, "sha512-c"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    let unresolved_deps: std::collections::BTreeMap<String, String> = [
        ("@nuxt/kit".to_string(), "^4.4.8".to_string()),
        ("@vite-pwa/nuxt".to_string(), "1.1.1".to_string()),
    ]
    .into_iter()
    .collect();
    let closure = crate::transitive_closure(&lockfile, "apps/web", unresolved_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

    let subgraph = lockfile
        .subgraph(&["apps/web".into()], &package_idents)
        .unwrap();
    let pruned = subgraph.lockfile().unwrap();

    assert!(
        pruned
            .packages
            .contains_key("@vite-pwa/nuxt/@nuxt/kit/pathe"),
        "3-level nested pathe@2.0.3 must be preserved so the nested @nuxt/kit@3 resolves pathe@2, \
         not the sibling @vite-pwa/nuxt/pathe@1.1.2. pruned pathe keys: {:?}",
        pruned
            .packages
            .keys()
            .filter(|k| k.ends_with("pathe"))
            .collect::<Vec<_>>()
    );
}

// https://github.com/vercel/turborepo/issues/13204
// A scoped package like "@types/webpack" whose unscoped name matches one of
// its own dependencies ("webpack") must not be treated as a nested entry for
// that dependency. The parent-chain walk previously split "@types/webpack" at
// the scope separator, found the "@types/webpack" entry under the bogus
// ancestor key "@types" + "/webpack", and pinned the webpack dependency to
// @types/webpack's version, dropping webpack from the pruned lockfile.
#[test]
fn test_scoped_package_name_not_mistaken_for_nested_entry() {
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "test-root"
            },
            "packages/app": {
                "name": "app",
                "version": "0.0.0",
                "dependencies": {
                    "@types/webpack": "5.28.5"
                }
            }
        },
        "packages": {
            "@types/webpack": ["@types/webpack@5.28.5", "", {
                "dependencies": {
                    "tapable": "^2.2.0",
                    "webpack": "^5"
                }
            }, "sha512-types-webpack"],
            "tapable": ["tapable@2.3.3", "", {}, "sha512-tapable"],
            "webpack": ["webpack@5.108.3", "", {
                "dependencies": {
                    "tapable": "^2.3.0"
                }
            }, "sha512-webpack"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // The webpack dependency of @types/webpack must keep its original spec
    // instead of being pinned to @types/webpack's own version.
    let deps = lockfile
        .all_dependencies("@types/webpack@5.28.5")
        .unwrap()
        .expect("@types/webpack should have dependencies");
    assert_eq!(deps.get("webpack"), Some(&"^5".to_string()));

    let unresolved_deps: std::collections::BTreeMap<String, String> =
        [("@types/webpack".to_string(), "5.28.5".to_string())]
            .into_iter()
            .collect();
    let closure =
        crate::transitive_closure(&lockfile, "packages/app", unresolved_deps, false).unwrap();

    assert!(
        closure.iter().any(|pkg| pkg.key == "webpack@5.108.3"),
        "webpack should be in the transitive closure of @types/webpack"
    );

    let subgraph = <BunLockfile as crate::Lockfile>::subgraph(
        &lockfile,
        &["packages/app".into()],
        &closure
            .iter()
            .map(|pkg| pkg.key.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let encoded = subgraph.encode().unwrap();
    let encoded_str = String::from_utf8(encoded).unwrap();
    let pruned = BunLockfile::from_str(&encoded_str).unwrap();

    assert!(
        pruned.data.packages.contains_key("webpack"),
        "webpack must remain in the pruned lockfile"
    );
}

// Regression test for https://github.com/vercel/turborepo/issues/13233
// When a dependency is resolved from an ancestor scope of its source key
// (e.g. `@headlessui/react/@floating-ui/react/@floating-ui/utils` providing
// `@floating-ui/utils` for a deeply nested `@floating-ui/dom`), the entry must
// not be copied into the pruned lockfile under that stale ancestor key once
// the dependent has been renamed by promotion/de-aliasing. Bun rejects the
// resulting lockfile because the dependent can no longer resolve the
// dependency.
#[test]
fn test_subgraph_relocates_ancestor_scoped_dep_for_renamed_dependent() {
    let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-monorepo"
                },
                "packages/app": {
                    "name": "app",
                    "dependencies": {
                        "@radix-ui/react-popper": "1.2.8"
                    }
                },
                "packages/other": {
                    "name": "other",
                    "dependencies": {
                        "@floating-ui/core": "1.7.5",
                        "@floating-ui/dom": "1.7.6",
                        "@floating-ui/react": "0.27.19",
                        "@floating-ui/react-dom": "2.1.8",
                        "@floating-ui/utils": "0.2.11",
                        "@headlessui/react": "2.2.0"
                    }
                }
            },
            "packages": {
                "@floating-ui/core": ["@floating-ui/core@1.7.5", "", { "dependencies": { "@floating-ui/utils": "^0.2.11" } }, "sha512-core175"],
                "@floating-ui/dom": ["@floating-ui/dom@1.7.6", "", { "dependencies": { "@floating-ui/core": "^1.7.5", "@floating-ui/utils": "^0.2.11" } }, "sha512-dom176"],
                "@floating-ui/react": ["@floating-ui/react@0.27.19", "", { "dependencies": { "@floating-ui/react-dom": "^2.1.8", "@floating-ui/utils": "^0.2.11" } }, "sha512-react02719"],
                "@floating-ui/react-dom": ["@floating-ui/react-dom@2.1.8", "", { "dependencies": { "@floating-ui/dom": "^1.7.6" } }, "sha512-reactdom218"],
                "@floating-ui/utils": ["@floating-ui/utils@0.2.11", "", {}, "sha512-utils0211"],
                "@headlessui/react": ["@headlessui/react@2.2.0", "", { "dependencies": { "@floating-ui/react": "^0.26.16" } }, "sha512-headlessui"],
                "@radix-ui/react-popper": ["@radix-ui/react-popper@1.2.8", "", { "dependencies": { "@floating-ui/react-dom": "^2.0.0" } }, "sha512-popper"],
                "app": ["app@workspace:packages/app"],
                "other": ["other@workspace:packages/other"],
                "@headlessui/react/@floating-ui/react": ["@floating-ui/react@0.26.16", "", { "dependencies": { "@floating-ui/react-dom": "^2.1.0", "@floating-ui/utils": "^0.2.0" } }, "sha512-react02616"],
                "@radix-ui/react-popper/@floating-ui/react-dom": ["@floating-ui/react-dom@2.1.7", "", { "dependencies": { "@floating-ui/dom": "^1.7.5" } }, "sha512-reactdom217"],
                "@headlessui/react/@floating-ui/react/@floating-ui/react-dom": ["@floating-ui/react-dom@2.1.7", "", { "dependencies": { "@floating-ui/dom": "^1.7.5" } }, "sha512-reactdom217"],
                "@headlessui/react/@floating-ui/react/@floating-ui/utils": ["@floating-ui/utils@0.2.10", "", {}, "sha512-utils0210"],
                "@radix-ui/react-popper/@floating-ui/react-dom/@floating-ui/dom": ["@floating-ui/dom@1.7.5", "", { "dependencies": { "@floating-ui/core": "^1.7.4", "@floating-ui/utils": "^0.2.10" } }, "sha512-dom175"],
                "@headlessui/react/@floating-ui/react/@floating-ui/react-dom/@floating-ui/dom": ["@floating-ui/dom@1.7.5", "", { "dependencies": { "@floating-ui/core": "^1.7.4", "@floating-ui/utils": "^0.2.10" } }, "sha512-dom175"],
                "@radix-ui/react-popper/@floating-ui/react-dom/@floating-ui/dom/@floating-ui/core": ["@floating-ui/core@1.7.4", "", { "dependencies": { "@floating-ui/utils": "^0.2.10" } }, "sha512-core174"],
                "@radix-ui/react-popper/@floating-ui/react-dom/@floating-ui/dom/@floating-ui/utils": ["@floating-ui/utils@0.2.10", "", {}, "sha512-utils0210"],
                "@headlessui/react/@floating-ui/react/@floating-ui/react-dom/@floating-ui/dom/@floating-ui/core": ["@floating-ui/core@1.7.4", "", { "dependencies": { "@floating-ui/utils": "^0.2.10" } }, "sha512-core174"]
            }
        }))
        .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();
    let mut app_deps = std::collections::BTreeMap::new();
    app_deps.insert("@radix-ui/react-popper".to_string(), "1.2.8".to_string());

    let closure = crate::transitive_closure(&lockfile, "packages/app", app_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();
    let subgraph = lockfile
        .subgraph(&["packages/app".into()], &package_idents)
        .unwrap();
    let data = subgraph.lockfile().unwrap();

    // Every nested key must still have its parent chain in the lockfile.
    for key in data.packages.keys() {
        if let Some(parent) = PackageKey::parse(key).parent() {
            assert!(
                data.packages.contains_key(&parent),
                "nested key {key} is unreachable: parent {parent} is missing"
            );
        }
    }

    // Every declared dependency must resolve the way bun does: direct nested
    // entry, then ancestor scopes, then top-level.
    for (key, entry) in &data.packages {
        let Some(info) = &entry.info else { continue };
        for dep_name in info.dependencies.keys() {
            let mut resolved = data.packages.contains_key(&format!("{key}/{dep_name}"));
            let mut scope = PackageKey::parse(key).parent();
            while !resolved && let Some(parent) = scope {
                resolved = data.packages.contains_key(&format!("{parent}/{dep_name}"));
                scope = PackageKey::parse(&parent).parent();
            }
            resolved = resolved || data.packages.contains_key(dep_name.as_str());
            assert!(
                resolved,
                "package {key} cannot resolve dependency {dep_name}"
            );
        }
    }
}

/// Regression test for https://github.com/vercel/turborepo/issues/13310
/// When pruning for workspace 'app', nested dependencies from the 'mobile'
/// workspace (e.g., @expo/cli/ora/chalk) should NOT appear in the pruned
/// lockfile. The `include_duplicate_alias_children` function previously
/// looked up child entries from the original (unpruned) lockfile, which
/// could re-introduce entries from non-target workspaces.
#[test]
fn test_subgraph_excludes_extraneous_nested_entries() {
    // Scenario: 'app' depends on 'chalk', 'mobile' depends on 'eas-cli'
    // which has nested '@expo/cli' -> '@expo/cli/ora' -> '@expo/cli/ora/chalk'.
    // Both 'chalk' (top-level) and '@expo/cli/ora/chalk' share the same ident
    // "chalk@5.4.1". After pruning for 'app', only the top-level 'chalk'
    // should remain — NOT '@expo/cli/ora/chalk' or any other entries from the
    // mobile workspace's dependency tree.
    let contents = serde_json::to_string(&json!({
        "lockfileVersion": 1,
        "workspaces": {
            "": {
                "name": "repro-root"
            },
            "apps/app": {
                "name": "app",
                "dependencies": {
                    "chalk": "^5.0.0"
                }
            },
            "apps/mobile": {
                "name": "mobile",
                "devDependencies": {
                    "eas-cli": "19.0.5"
                }
            }
        },
        "packages": {
            // Top-level chalk, used by 'app'
            "chalk": ["chalk@5.4.1", "", {}, "sha512-chalk"],
            // eas-cli from 'mobile' workspace
            "eas-cli": ["eas-cli@19.0.5", "", {
                "dependencies": {
                    "@expo/cli": "^1.0.0"
                }
            }, "sha512-eas"],
            // @expo/cli, a dependency of eas-cli
            "@expo/cli": ["@expo/cli@1.0.0", "", {
                "dependencies": {
                    "ora": "^5.0.0"
                }
            }, "sha512-expo-cli"],
            // ora nested under @expo/cli
            "@expo/cli/ora": ["ora@5.4.1", "", {
                "dependencies": {
                    "chalk": "^5.0.0"
                }
            }, "sha512-ora"],
            // chalk nested under @expo/cli/ora — SAME ident as top-level chalk
            "@expo/cli/ora/chalk": ["chalk@5.4.1", "", {}, "sha512-chalk"]
        }
    }))
    .unwrap();

    let lockfile = BunLockfile::from_str(&contents).unwrap();

    // Compute transitive closure for 'app' workspace
    let mut app_deps = std::collections::BTreeMap::new();
    app_deps.insert("chalk".to_string(), "^5.0.0".to_string());

    let closure =
        crate::transitive_closure(&lockfile, "apps/app", app_deps, false).unwrap();
    let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

    let subgraph = lockfile
        .subgraph(&["apps/app".into()], &package_idents)
        .unwrap();
    let data = subgraph.lockfile().unwrap();

    // 'app' only depends on chalk, so only chalk should be in the pruned lockfile
    assert!(
        data.packages.contains_key("chalk"),
        "top-level chalk must be present for 'app' workspace"
    );

    // None of the mobile/eas-cli dependency chain should be present
    assert!(
        !data.packages.contains_key("eas-cli"),
        "eas-cli from mobile workspace must NOT be in pruned lockfile"
    );
    assert!(
        !data.packages.contains_key("@expo/cli"),
        "@expo/cli from mobile workspace must NOT be in pruned lockfile"
    );
    assert!(
        !data.packages.contains_key("@expo/cli/ora"),
        "@expo/cli/ora from mobile workspace must NOT be in pruned lockfile"
    );
    assert!(
        !data.packages.contains_key("@expo/cli/ora/chalk"),
        "@expo/cli/ora/chalk must NOT be re-introduced from the original lockfile (issue #13310)"
    );

    // Verify workspace filtering
    assert!(
        data.workspaces.contains_key(""),
        "root workspace must be present"
    );
    assert!(
        data.workspaces.contains_key("apps/app"),
        "target workspace must be present"
    );
    assert!(
        !data.workspaces.contains_key("apps/mobile"),
        "mobile workspace must NOT be present"
    );
}
