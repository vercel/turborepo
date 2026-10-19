use std::{collections::BTreeMap, io::Write, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestResult {
    pub passed: u64,
}

#[derive(Debug, PartialEq)]
pub struct TestResults {
    pub hits: BTreeMap<String, TestResult>,
    pub source: crate::CacheSource,
}

pub(crate) const MAX_TEST_RESULTS: usize = 4096;

fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit())
}

pub(crate) fn validate_results(
    payload: serde_json::Value,
    keys: &[String],
) -> Option<BTreeMap<String, TestResult>> {
    let results: BTreeMap<String, TestResult> = serde_json::from_value(payload).ok()?;
    results
        .iter()
        .all(|(key, result)| valid_key(key) && keys.contains(key) && result.passed > 0)
        .then_some(results)
}

pub(crate) fn read_local(directory: &Path, keys: &[String]) -> BTreeMap<String, TestResult> {
    keys.iter()
        .filter(|key| valid_key(key))
        .filter_map(|key| {
            let bytes = std::fs::read(directory.join("test-results-v1").join(key)).ok()?;
            let result: TestResult = serde_json::from_slice(&bytes).ok()?;
            (result.passed > 0).then(|| (key.clone(), result))
        })
        .collect()
}

pub(crate) fn write_local(
    directory: &Path,
    results: &BTreeMap<String, TestResult>,
) -> std::io::Result<()> {
    let directory = directory.join("test-results-v1");
    std::fs::create_dir_all(&directory)?;
    for (key, result) in results {
        if !valid_key(key) || result.passed == 0 {
            continue;
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(&serde_json::to_vec(result)?)?;
        temporary.persist(directory.join(key))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bulk_remote_payloads_require_capability_and_never_fall_back_per_unit() {
        use turborepo_api_client::{APIAuth, APIClient};
        use turborepo_types::SecretString;

        use crate::{AsyncCache, CacheConfig, CacheOpts, LazyScmState};

        for supported in [false, true] {
            let server = httpmock::MockServer::start_async().await;
            let keys = vec!["a".repeat(64), "b".repeat(64)];
            let results = BTreeMap::from([(keys[0].clone(), TestResult { passed: 2 })]);
            let query = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::POST)
                        .path("/v8/artifacts/experimental-test-results")
                        .header("authorization", "Bearer test-token")
                        .header("x-turbo-test-cache", "1")
                        .json_body(serde_json::json!({"keys": keys}));
                    if supported {
                        then.status(200)
                            .header("x-turbo-test-cache", "1")
                            .json_body_obj(&results);
                    } else {
                        then.status(200).json_body_obj(&results);
                    }
                })
                .await;
            let upload = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::PUT)
                        .path("/v8/artifacts/experimental-test-results")
                        .json_body(serde_json::json!({"results": results}));
                    then.status(200)
                        .header("x-turbo-test-cache", "1")
                        .json_body(serde_json::json!({}));
                })
                .await;
            let per_unit = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::GET);
                    then.status(500);
                })
                .await;
            let per_unit_head = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::HEAD);
                    then.status(500);
                })
                .await;
            let directory = tempfile::tempdir().unwrap();
            let root = turbopath::AbsoluteSystemPathBuf::try_from(directory.path()).unwrap();
            let cache = AsyncCache::new(
                &CacheOpts {
                    cache_dir: ".turbo/cache".into(),
                    cache: CacheConfig::remote_only(),
                    workers: 1,
                    remote_cache_opts: None,
                    cache_max_age: None,
                    cache_max_size: None,
                },
                &root,
                Some(APIClient::new(server.base_url(), None, None, "test", false).unwrap()),
                Some(APIAuth {
                    team_id: None,
                    team_slug: None,
                    token: SecretString::new("test-token".into()),
                }),
                None,
                LazyScmState::resolved(None),
            )
            .unwrap();
            let fetched = cache.fetch_test_results(&keys, true).await.unwrap();
            assert_eq!(
                fetched,
                supported.then_some(TestResults {
                    hits: results.clone(),
                    source: crate::CacheSource::Remote,
                })
            );
            if supported {
                cache.save_test_results(results).await.unwrap();
            }
            cache.shutdown().await.unwrap();
            query.assert_calls_async(1).await;
            upload.assert_calls_async(usize::from(supported)).await;
            per_unit.assert_calls_async(0).await;
            per_unit_head.assert_calls_async(0).await;
        }
    }

    #[tokio::test]
    async fn configured_eviction_uses_the_ordinary_artifact_cache() {
        use crate::{AsyncCache, CacheActions, CacheConfig, CacheOpts, LazyScmState};
        for (age, size) in [
            (Some(std::time::Duration::from_secs(60)), None),
            (None, Some(1024)),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = turbopath::AbsoluteSystemPathBuf::try_from(directory.path()).unwrap();
            let cache = AsyncCache::new(
                &CacheOpts {
                    cache_dir: ".turbo/cache".into(),
                    cache: CacheConfig {
                        local: CacheActions::enabled(),
                        remote: CacheActions::disabled(),
                    },
                    workers: 1,
                    remote_cache_opts: None,
                    cache_max_age: age,
                    cache_max_size: size,
                },
                &root,
                None,
                None,
                None,
                LazyScmState::resolved(None),
            )
            .unwrap();
            assert!(
                cache
                    .fetch_test_results(&["a".repeat(64)], true)
                    .await
                    .unwrap()
                    .is_none()
            );
            cache.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn local_hits_only_probe_remote_and_missing_keys_are_batched() {
        use turborepo_api_client::{APIAuth, APIClient};
        use turborepo_types::SecretString;

        use crate::{AsyncCache, CacheConfig, CacheOpts, LazyScmState};
        let server = httpmock::MockServer::start_async().await;
        let directory = tempfile::tempdir().unwrap();
        let root = turbopath::AbsoluteSystemPathBuf::try_from(directory.path()).unwrap();
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let local = BTreeMap::from([(a.clone(), TestResult { passed: 1 })]);
        write_local(&directory.path().join(".turbo/cache"), &local).unwrap();
        let probe = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/v8/artifacts/experimental-test-results")
                    .json_body(serde_json::json!({"keys": []}));
                then.status(200)
                    .header("x-turbo-test-cache", "1")
                    .json_body(serde_json::json!({}));
            })
            .await;
        let missing = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/v8/artifacts/experimental-test-results")
                    .json_body(serde_json::json!({"keys": [b]}));
                then.status(200)
                    .header("x-turbo-test-cache", "1")
                    .json_body(serde_json::json!({b.clone(): {"passed": 2}}));
            })
            .await;
        let cache = AsyncCache::new(
            &CacheOpts {
                cache_dir: ".turbo/cache".into(),
                cache: CacheConfig::default(),
                workers: 1,
                remote_cache_opts: None,
                cache_max_age: None,
                cache_max_size: None,
            },
            &root,
            Some(APIClient::new(server.base_url(), None, None, "test", false).unwrap()),
            Some(APIAuth {
                team_id: None,
                team_slug: None,
                token: SecretString::new("token".into()),
            }),
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();
        assert_eq!(
            cache
                .fetch_test_results(std::slice::from_ref(&a), true)
                .await
                .unwrap(),
            Some(TestResults {
                hits: local,
                source: crate::CacheSource::Local,
            })
        );
        let mixed = cache
            .fetch_test_results(&[a.clone(), b.clone()], true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mixed.source, crate::CacheSource::Remote);
        assert_eq!(mixed.hits.len(), 2);
        assert!(
            cache
                .fetch_test_results(&vec![a; MAX_TEST_RESULTS + 1], true)
                .await
                .unwrap()
                .is_none()
        );
        probe.assert_calls_async(1).await;
        missing.assert_calls_async(1).await;
        cache.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn bulk_protocol_distinguishes_unsupported_errors_and_oversize_responses() {
        use turborepo_api_client::{APIAuth, APIClient};
        use turborepo_types::SecretString;
        for status in [404, 405, 501, 400, 401, 403] {
            let server = httpmock::MockServer::start_async().await;
            let request = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::POST)
                        .path("/v8/artifacts/experimental-test-results");
                    then.status(status).json_body(
                        serde_json::json!({"error": {"code": "forbidden", "message": "denied"}}),
                    );
                })
                .await;
            let client = APIClient::new(server.base_url(), None, None, "test", false).unwrap();
            let auth = APIAuth {
                team_id: None,
                team_slug: None,
                token: SecretString::new("token".into()),
            };
            let result = client
                .experimental_test_results(&auth, false, &serde_json::json!({"keys": []}))
                .await;
            if [404, 405, 501].contains(&status) {
                assert_eq!(result.unwrap(), None, "status {status}");
            } else {
                assert!(result.is_err(), "status {status}");
            }
            request.assert_calls_async(1).await;
        }
        let server = httpmock::MockServer::start_async().await;
        let request = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/v8/artifacts/experimental-test-results");
                then.status(200)
                    .header("x-turbo-test-cache", "1")
                    .body(" ".repeat(1024 * 1024 + 1));
            })
            .await;
        let client = APIClient::new(server.base_url(), None, None, "test", false).unwrap();
        let auth = APIAuth {
            team_id: None,
            team_slug: None,
            token: SecretString::new("token".into()),
        };
        assert!(
            client
                .experimental_test_results(&auth, false, &serde_json::json!({"keys": []}))
                .await
                .unwrap()
                .is_none()
        );
        request.assert_calls_async(1).await;
    }

    #[test]
    fn immutable_results_are_independent_and_invalid_records_miss() {
        let directory = tempfile::tempdir().unwrap();
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let results = BTreeMap::from([(a.clone(), TestResult { passed: 2 })]);
        write_local(directory.path(), &results).unwrap();
        assert_eq!(
            read_local(directory.path(), &[a.clone(), b.clone()]),
            results
        );
        std::fs::write(
            directory.path().join("test-results-v1").join(&b),
            b"{\"passed\":0}",
        )
        .unwrap();
        assert_eq!(read_local(directory.path(), &[a, b]), results);
        assert!(validate_results(serde_json::json!({"../bad": {"passed": 1}}), &[]).is_none());
    }
}
