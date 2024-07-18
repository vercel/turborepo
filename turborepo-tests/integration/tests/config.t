Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Run test run
  $ ${TURBO} config
  {
    "apiUrl": "https://vercel.com/api",
    "loginUrl": "https://vercel.com",
    "teamSlug": null,
    "teamId": null,
    "signature": false,
    "preflight": false,
    "timeout": 30,
    "uploadTimeout": 60,
    "enabled": true,
    "spacesId": null,
    "ui": false,
    "packageManager": "npm",
    "daemon": null
  }

Run test run with api overloaded
  $ ${TURBO} config --api http://localhost:8000 | jq .apiUrl
  "http://localhost:8000"

Run test run with team overloaded
  $ ${TURBO} config --team vercel | jq .teamSlug
  "vercel"

Run test run with team overloaded from both env and flag (flag should take precedence)
  $ TURBO_TEAM=vercel ${TURBO} config --team turbo | jq .teamSlug
  "turbo"

Run test run with remote cache timeout env variable set
  $ TURBO_REMOTE_CACHE_TIMEOUT=123 ${TURBO} config | jq .timeout
  123

Run test run with remote cache timeout from both env and flag (flag should take precedence)
  $ TURBO_REMOTE_CACHE_TIMEOUT=123 ${TURBO} config --remote-cache-timeout 456 | jq .timeout
  456

Use our custom turbo config with an invalid env var
  $ . ${TESTDIR}/../../helpers/replace_turbo_json.sh $(pwd) "invalid-env-var.json"

Run build with invalid env var
  $ ${TURBO} build
  invalid_env_prefix (https://turbo.build/messages/invalid-env-prefix)
  
    x Environment variables should not be prefixed with "$"
     ,-[turbo.json:6:1]
   6 |     "build": {
   7 |       "env": ["NODE_ENV", "$FOOBAR"],
     :                           ^^^^|^^^^
     :                               `-- variable with invalid prefix declared here
   8 |       "outputs": []
     `----
  
  [1]

