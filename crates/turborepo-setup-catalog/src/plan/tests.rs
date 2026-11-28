use turborepo_turbo_json::ArtifactFormat::{Binary, TarGz, TarXz, Zip};

use super::*;
use crate::tests::request;

const NAMES: &str = concat!(
    "# Fixture names.\n",
    "prefixed\texample/prefixed\n",
    "http-tool\texample/http-tool\n",
    "raw\texample/raw\n",
);

pub(crate) const REGISTRY: &str = r#"
packages:
  - type: github_release
    repo_owner: example
    repo_name: prefixed
    version_prefix: tool-
    asset: prefixed-{{.SemVer}}-{{.OS}}-{{.Arch}}.{{.Format}}
    format: tar.gz
    replacements:
      amd64: x86_64
      darwin: macos
    files:
      - name: prefixed
        src: "{{.AssetWithoutExt}}/bin/prefixed"
      - name: prefixed-helper
        src: "{{.AssetWithoutExt}}/libexec/helper"
    overrides:
      - goos: windows
        format: zip
  - type: http
    repo_owner: example
    repo_name: http-tool
    url: https://downloads.example.test/http-tool/{{trimV .Version}}/http-tool_{{.OS}}_{{.Arch}}.zip
    supported_envs:
      - darwin
      - linux/amd64
  - repo_owner: example
    repo_name: raw
    asset: raw-{{.OS}}-{{.Arch}}
    format: raw
  - repo_owner: other
    repo_name: raw
    asset: raw.tar.gz
  - repo_owner: example
    repo_name: xz
    asset: xz-{{.Version}}-{{.OS}}-{{.Arch}}.tar.xz
    overrides:
      - goos: windows
        asset: xz-{{.Version}}-{{.OS}}-{{.Arch}}.zip
  - repo_owner: example
    repo_name: zst
    asset: zst-{{.Version}}-{{.OS}}-{{.Arch}}.tar.zst
    overrides:
      - goos: windows
        asset: zst-{{.Version}}-{{.OS}}-{{.Arch}}.zip
  - type: go_install
    repo_owner: example
    repo_name: from-source
  - repo_owner: example
    repo_name: versions
    asset: versions_{{.OS}}_{{.Arch}}.tar.gz
    version_constraint: semver(">= 2.0.0")
    version_overrides:
      - version_constraint: semver(">= 1.0.0")
        asset: versions-{{.Version}}.tgz
        files:
          - name: versions
            src: bin/versions
      - version_constraint: semver(">= 0.5.0")
        error_message: versions before 1.0.0 are built from source
      - version_constraint: "true"
        no_asset: true
  - repo_owner: example
    repo_name: renamed
    version_constraint: "false"
    version_overrides:
      - version_constraint: semver("< 2.0.0")
        version_prefix: cli/v
        asset: renamed-{{.OS}}-{{.Arch}}
        format: raw
        complete_windows_ext: false
      - version_constraint: "true"
        version_prefix: "@example/renamed@"
        asset: renamed-{{.SemVer}}-{{.OS}}-{{.Arch}}.zip
  - repo_owner: example
    repo_name: moved
    asset: moved-{{.OS}}-{{.Arch}}
    format: raw
    aliases:
      - name: elsewhere/old-name
  - type: http
    name: example/suite/cli
    repo_owner: example
    repo_name: suite-monorepo
    url: https://downloads.example.test/suite-{{.OS}}.tar.gz
  - type: http
    repo_owner: example
    repo_name: unsafe
    url: http://downloads.example.test/unsafe-{{.OS}}.tar.gz
    files:
      - name: unsafe
        src: ../unsafe
    overrides:
      - goos: darwin
        url: https://downloads.example.test/unsafe.tar.gz
      - goos: windows
        url: https://downloads.example.test/unsafe.tar.gz
        files:
          - name: npm
"#;

fn catalog() -> Catalog {
    Catalog::for_tests(NAMES, REGISTRY)
}

fn plan(id: &str, version: &str, from: Option<&str>) -> Plan {
    catalog().plan(&request(id, version, from)).unwrap()
}

fn first(plan: &Plan, platform: Platform) -> Result<PlannedArtifact, Unavailable> {
    plan.releases[0].platforms[&platform].clone()
}

fn executables(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, path)| ((*name).to_owned(), (*path).to_owned()))
        .collect()
}

#[test]
fn release_names_try_the_recipe_prefix_then_v_then_the_bare_version() {
    let names =
        |plan: &Plan| -> Vec<String> { plan.releases.iter().map(|r| r.name.clone()).collect() };
    assert_eq!(
        names(&plan("prefixed", "1.2.3", None)),
        ["tool-1.2.3", "tool-v1.2.3", "v1.2.3", "1.2.3"]
    );
    assert_eq!(names(&plan("raw", "26.1", None)), ["v26.1", "26.1"]);
    let renamed = plan("renamed", "2.3.0", Some("aqua:example/renamed"));
    assert_eq!(
        names(&renamed),
        [
            "cli/v2.3.0",
            "@example/renamed@2.3.0",
            "@example/renamed@v2.3.0",
            "v2.3.0",
            "2.3.0"
        ]
    );
    let linux =
        |release: usize| renamed.releases[release].platforms[&Platform::LinuxX64Gnu].clone();
    assert!(
        matches!(linux(0), Err(Unavailable::Recipe(_))),
        "{:?}",
        linux(0)
    );
    assert_eq!(
        linux(1).unwrap().url,
        "https://github.com/example/renamed/releases/download/@example/renamed@2.3.0/\
         renamed-2.3.0-linux-amd64.zip"
    );
    let planned = renamed.releases.iter();
    let planned = planned.filter(|release| release.platforms[&Platform::LinuxX64Gnu].is_ok());
    assert_eq!(
        planned.map(|release| &release.name).collect::<Vec<_>>(),
        ["@example/renamed@2.3.0", "@example/renamed@v2.3.0"]
    );
    let old = plan("renamed", "1.9.4", Some("aqua:example/renamed"));
    assert_eq!(
        first(&old, Platform::LinuxX64Gnu).unwrap().url,
        "https://github.com/example/renamed/releases/download/cli/v1.9.4/renamed-linux-amd64"
    );
    assert_eq!(
        first(&old, Platform::WindowsX64),
        Ok(PlannedArtifact {
            url: "https://github.com/example/renamed/releases/download/cli/v1.9.4/\
                  renamed-windows-amd64"
                .into(),
            format: Binary,
            executables: executables(&[("renamed", "renamed.exe")]),
        })
    );
    for release in plan("prefixed", "1.2.3", None).releases {
        assert_eq!(
            release.platforms.keys().copied().collect::<Vec<_>>(),
            [
                Platform::MacosX64,
                Platform::MacosArm64,
                Platform::LinuxX64Gnu,
                Platform::LinuxArm64Gnu,
                Platform::WindowsX64
            ]
        );
    }
}

#[test]
fn github_releases_plan_their_asset_and_declared_files() {
    let plan = plan("prefixed", "1.2.3", None);
    assert_eq!(plan.source, "aqua:example/prefixed");
    assert_eq!(
        first(&plan, Platform::LinuxX64Gnu),
        Ok(PlannedArtifact {
            url: "https://github.com/example/prefixed/releases/download/tool-1.2.3/\
                  prefixed-1.2.3-linux-x86_64.tar.gz"
                .into(),
            format: TarGz,
            executables: executables(&[
                ("prefixed", "prefixed-1.2.3-linux-x86_64/bin/prefixed"),
                ("prefixed-helper", "prefixed-1.2.3-linux-x86_64/libexec/helper"),
            ]),
        })
    );
    assert_eq!(
        first(&plan, Platform::MacosArm64).unwrap().url,
        "https://github.com/example/prefixed/releases/download/tool-1.2.3/\
         prefixed-1.2.3-macos-arm64.tar.gz"
    );
    assert_eq!(
        first(&plan, Platform::WindowsX64),
        Ok(PlannedArtifact {
            url: "https://github.com/example/prefixed/releases/download/tool-1.2.3/\
                  prefixed-1.2.3-windows-x86_64.zip"
                .into(),
            format: Zip,
            executables: executables(&[
                ("prefixed", "prefixed-1.2.3-windows-x86_64/bin/prefixed.exe"),
                (
                    "prefixed-helper",
                    "prefixed-1.2.3-windows-x86_64/libexec/helper.exe"
                ),
            ]),
        })
    );
    let v = plan.releases[2].platforms[&Platform::LinuxX64Gnu]
        .clone()
        .unwrap();
    assert_eq!(
        v.url,
        "https://github.com/example/prefixed/releases/download/v1.2.3/\
         prefixed-v1.2.3-linux-x86_64.tar.gz"
    );
}

#[test]
fn http_recipes_plan_their_url_default_file_and_supported_platforms() {
    let plan = plan("http-tool", "2.0.0", None);
    assert_eq!(plan.source, "aqua:example/http-tool");
    assert_eq!(
        first(&plan, Platform::LinuxX64Gnu),
        Ok(PlannedArtifact {
            url: "https://downloads.example.test/http-tool/2.0.0/http-tool_linux_amd64.zip".into(),
            format: Zip,
            executables: executables(&[("http-tool", "http-tool")]),
        })
    );
    assert!(first(&plan, Platform::MacosX64).is_ok());
    assert!(first(&plan, Platform::MacosArm64).is_ok());
    for unsupported in [Platform::LinuxArm64Gnu, Platform::WindowsX64] {
        assert_eq!(first(&plan, unsupported), Err(Unavailable::Platform));
    }
}

#[test]
fn the_default_command_is_named_after_the_recipe_not_the_declared_id() {
    let command = |from: &str| {
        let planned = first(&plan("tool", "1.0.0", Some(from)), Platform::LinuxX64Gnu);
        planned.unwrap().executables
    };
    assert_eq!(
        command("aqua:example/moved"),
        executables(&[("moved", "moved")])
    );
    assert_eq!(
        command("aqua:elsewhere/old-name"),
        command("aqua:example/moved")
    );
    assert_eq!(
        command("aqua:example/suite/cli"),
        executables(&[("cli", "cli")])
    );
}

#[test]
fn raw_downloads_are_one_binary_installed_under_its_name() {
    let plan = plan("raw", "26.1", None);
    assert_eq!(
        first(&plan, Platform::MacosArm64),
        Ok(PlannedArtifact {
            url: "https://github.com/example/raw/releases/download/v26.1/raw-darwin-arm64".into(),
            format: Binary,
            executables: executables(&[("raw", "raw")]),
        })
    );
    assert_eq!(
        first(&plan, Platform::WindowsX64),
        Ok(PlannedArtifact {
            url: "https://github.com/example/raw/releases/download/v26.1/raw-windows-amd64.exe"
                .into(),
            format: Binary,
            executables: executables(&[("raw", "raw.exe")]),
        })
    );
}

#[test]
fn platforms_a_recipe_cannot_install_are_left_out_with_a_reason() {
    let xz = plan("xz", "5.8.1", Some("aqua:example/xz"));
    assert_eq!(first(&xz, Platform::LinuxX64Gnu).unwrap().format, TarXz);
    assert_eq!(first(&xz, Platform::WindowsX64).unwrap().format, Zip);

    let zst = plan("zst", "5.8.1", Some("aqua:example/zst"));
    assert_eq!(
        first(&zst, Platform::LinuxX64Gnu),
        Err(Unavailable::Format("tar.zst".into()))
    );
    assert!(
        Unavailable::Format("tar.zst".into())
            .to_string()
            .contains("tar.zst")
    );
    assert_eq!(first(&zst, Platform::WindowsX64).unwrap().format, Zip);

    let source = plan("from-source", "1.0.0", Some("aqua:example/from-source"));
    for planned in source.releases[0].platforms.values() {
        assert_eq!(planned, &Err(Unavailable::Kind("go_install".into())));
    }

    let unsafe_recipe = plan("unsafe", "1.0.0", Some("aqua:example/unsafe"));
    for platform in [
        Platform::LinuxX64Gnu,
        Platform::MacosArm64,
        Platform::WindowsX64,
    ] {
        let planned = first(&unsafe_recipe, platform);
        assert!(
            matches!(planned, Err(Unavailable::Recipe(_))),
            "{platform:?}: {planned:?}"
        );
    }
}

#[test]
fn version_overrides_select_the_recipe_for_the_requested_version() {
    let from = Some("aqua:example/versions");
    let current = plan("versions", "2.1.0", from);
    assert_eq!(
        first(&current, Platform::LinuxArm64Gnu).unwrap().url,
        "https://github.com/example/versions/releases/download/v2.1.0/versions_linux_arm64.tar.gz"
    );
    assert_eq!(
        first(&plan("versions", "1.4.0", from), Platform::LinuxArm64Gnu),
        Ok(PlannedArtifact {
            url: "https://github.com/example/versions/releases/download/v1.4.0/versions-v1.4.0.tgz"
                .into(),
            format: TarGz,
            executables: executables(&[("versions", "bin/versions")]),
        })
    );
    assert_eq!(
        first(&plan("versions", "0.9.0", from), Platform::LinuxArm64Gnu),
        Err(Unavailable::Message(
            "versions before 1.0.0 are built from source".into()
        ))
    );
    assert_eq!(
        first(&plan("versions", "0.1.0", from), Platform::LinuxArm64Gnu),
        Err(Unavailable::NoAsset)
    );
}

#[test]
fn an_unknown_name_lists_the_catalog_entries_it_could_mean() {
    let catalog = catalog();
    let error = catalog.plan(&request("xz", "5.8.1", None)).unwrap_err();
    assert!(
        matches!(&error, Error::UnknownName { tool, candidates, .. }
            if tool == "xz" && candidates == &["example/xz".to_owned()]),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(message.contains("\"xz\" is not a tool name"), "{message}");
    assert!(message.contains("\"aqua:example/xz\""), "{message}");
    assert!(
        message.contains("{ \"version\": \"5.8.1\", \"from\": \"aqua:<package ID>\" }"),
        "{message}"
    );
    assert!(message.contains("\"artifacts\""), "{message}");

    assert_eq!(
        catalog.plan(&request("raw", "1.0.0", None)).unwrap().source,
        "aqua:example/raw"
    );
    let unnamed = Catalog::for_tests("", REGISTRY);
    let error = unnamed.plan(&request("raw", "1.0.0", None)).unwrap_err();
    assert!(
        matches!(&error, Error::UnknownName { candidates, .. }
            if candidates == &["example/raw".to_owned(), "other/raw".to_owned()]),
        "{error:?}"
    );
    let error = catalog
        .plan(&request("nothing", "1.0.0", None))
        .unwrap_err();
    assert!(
        matches!(&error, Error::UnknownName { candidates, .. } if candidates.is_empty()),
        "{error:?}"
    );
    assert!(!error.to_string().contains("The catalog has"));

    let error = catalog
        .plan(&request("xz", "5.8.1", Some("aqua:example/missing")))
        .unwrap_err();
    assert!(
        matches!(&error, Error::UnknownPackage { package, .. } if package == "example/missing"),
        "{error:?}"
    );
}
