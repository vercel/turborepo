#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

#[tokio::test(flavor = "multi_thread")]
async fn standard_cli_protects_saved_vercel_credentials_but_preserves_generic_cache_tokens() {
    for source in ["auth", "legacy", "global", "env", "cli", "coexisting"] {
        for api_source in ["repo", "cli", "env", "global"] {
            let server = httpmock::MockServer::start_async().await;
            let status = server
                .mock_async(|when, then| {
                    when.path("/v8/artifacts/status")
                        .header("authorization", "Bearer synthetic-cli-sentinel");
                    then.status(200)
                        .json_body(serde_json::json!({"status":"enabled"}));
                })
                .await;
            let artifacts = server
                .mock_async(|when, then| {
                    when.header("authorization", "Bearer synthetic-cli-sentinel");
                    then.status(404);
                })
                .await;
            let unexpected = server
                .mock_async(|when, then| {
                    when.any_request();
                    then.status(500);
                })
                .await;
            let repo = tempfile::tempdir().unwrap();
            let config = tempfile::tempdir().unwrap();
            let legacy = tempfile::tempdir().unwrap();
            std::fs::write(repo.path().join("package.json"), r#"{"name":"fixture","version":"0.0.0","packageManager":"npm@10.5.0","scripts":{"build":"node -e \"\""}}"#).unwrap();
            std::fs::write(repo.path().join("package-lock.json"), r#"{"name":"fixture","lockfileVersion":3,"packages":{"":{"name":"fixture","version":"0.0.0"}}}"#).unwrap();
            std::fs::write(repo.path().join("turbo.json"), serde_json::json!({"tasks":{"build":{}},"remoteCache":{"apiUrl":server.base_url(),"teamId":"team_test"}}).to_string()).unwrap();
            std::fs::create_dir_all(config.path().join("turborepo")).unwrap();
            std::fs::create_dir_all(legacy.path().join("com.vercel.cli")).unwrap();
            let document = r#"{"token":"synthetic-cli-sentinel"}"#;
            match source {
                "auth" => {
                    std::fs::write(config.path().join("turborepo/auth.json"), document).unwrap()
                }
                "legacy" => {
                    std::fs::write(legacy.path().join("com.vercel.cli/auth.json"), document)
                        .unwrap()
                }
                "global" => {
                    std::fs::write(config.path().join("turborepo/config.json"), document).unwrap()
                }
                "coexisting" | "env" | "cli" => {
                    std::fs::write(
                        config.path().join("turborepo/config.json"),
                        r#"{"token":"synthetic-retained-sentinel"}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        config.path().join("turborepo/auth.json"),
                        r#"{"token":"synthetic-protected-sentinel"}"#,
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            if api_source == "global" {
                let path = config.path().join("turborepo/config.json");
                let mut global = if path.exists() {
                    serde_json::from_str::<serde_json::Value>(
                        &std::fs::read_to_string(&path).unwrap(),
                    )
                    .unwrap()
                } else {
                    serde_json::json!({})
                };
                global["apiUrl"] = server.base_url().into();
                std::fs::write(path, global.to_string()).unwrap();
            }
            let mut command = common::turbo_command(repo.path());
            command
                .env("TURBO_CONFIG_DIR_PATH", config.path())
                .env("VERCEL_CONFIG_DIR_PATH", legacy.path());
            if source == "env" {
                command.env("TURBO_TOKEN", "synthetic-cli-sentinel");
            }
            if source == "cli" {
                command.args(["--token", "synthetic-cli-sentinel"]);
            }
            match api_source {
                "cli" => {
                    command.args(["--api", &server.base_url()]);
                }
                "env" => {
                    command.env("TURBO_API", server.base_url());
                }
                _ => {}
            }
            let output = command
                .args(["run", "build", "--cache=remote:r", "--ui=stream"])
                .output()
                .unwrap();
            let text = common::combined_output(&output);
            assert!(
                output.status.success(),
                "{source}, api_source={api_source}: {text}"
            );
            unexpected.assert_calls_async(0).await;
            let calls = status.calls_async().await + artifacts.calls_async().await;
            if matches!(source, "auth" | "legacy" | "coexisting") {
                assert_eq!(calls, 0, "{source}, api_source={api_source}: {text}");
                assert!(
                    text.contains("Refusing to send saved Vercel credentials"),
                    "{text}"
                );
            } else {
                assert!(
                    calls > 0,
                    "generic credential never reached the mock: {source}: {text}"
                );
            }
            assert!(!text.contains("synthetic-cli-sentinel"), "{text}");
            assert!(!text.contains("synthetic-protected-sentinel"), "{text}");
            assert!(!text.contains("synthetic-retained-sentinel"), "{text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn custom_api_logout_without_invalidation_clears_mixed_stores_without_requests() {
    let server = httpmock::MockServer::start_async().await;
    let requests = server
        .mock_async(|when, then| {
            when.any_request();
            then.status(500);
        })
        .await;
    let repo = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(config.path().join("turborepo")).unwrap();
    std::fs::create_dir_all(legacy.path().join("com.vercel.cli")).unwrap();
    let paths = [
        config.path().join("turborepo/auth.json"),
        config.path().join("turborepo/config.json"),
        legacy.path().join("com.vercel.cli/auth.json"),
    ];
    for (path, token) in
        paths
            .iter()
            .zip(["synthetic-saved", "synthetic-generic", "synthetic-legacy"])
    {
        std::fs::write(
            path,
            serde_json::json!({"token": token, "other": true}).to_string(),
        )
        .unwrap();
    }
    let output = common::turbo_command(repo.path())
        .env("TURBO_CONFIG_DIR_PATH", config.path())
        .env("VERCEL_CONFIG_DIR_PATH", legacy.path())
        .args(["--api", &server.base_url(), "logout", "--invalidate=false"])
        .output()
        .unwrap();
    let text = common::combined_output(&output);
    assert!(output.status.success(), "{text}");
    requests.assert_calls_async(0).await;
    for path in paths {
        let document: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(document, serde_json::json!({"other": true}));
    }
    assert!(!text.contains("synthetic-"), "{text}");
}
