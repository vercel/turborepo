Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

Run test run
  $ ${TURBO} run build --__test-run | jq .api_client_config
  {
    "token": null,
    "team_id": null,
    "team_slug": null,
    "api_url": "https://vercel.com/api",
    "use_preflight": false,
    "timeout": 20
  }

Run test run with api overloaded
  $ ${TURBO} run build --__test-run --api http://localhost:8000 | jq .api_client_config.api_url
  "http://localhost:8000"

Run test run with token overloaded
  $ ${TURBO} run build --__test-run --token 1234567890 | jq .api_client_config.token
  "1234567890"

Run test run with token overloaded from both TURBO_TOKEN and VERCEL_ARTIFACTS_TOKEN
  $ TURBO_TOKEN=turbo VERCEL_ARTIFACTS_TOKEN=vercel ${TURBO} run build --__test-run | jq .api_client_config.token
  "vercel"

Run test run with team overloaded
  $ ${TURBO} run build --__test-run --team vercel | jq .api_client_config.team_slug
  "vercel"

Run test run with team overloaded from both env and flag (flag should take precedence)
  $ TURBO_TEAM=vercel ${TURBO} run build --__test-run --team turbo | jq .api_client_config.team_slug
  "turbo"

Run test run with remote cache timeout env variable set
  $ TURBO_REMOTE_CACHE_TIMEOUT=123 ${TURBO} run build --__test-run | jq .api_client_config.timeout
  123

Run test run with remote cache timeout from both env and flag (flag should take precedence)
  $ TURBO_REMOTE_CACHE_TIMEOUT=123 ${TURBO} run build --__test-run --remote-cache-timeout 456 | jq .api_client_config.timeout
  456
