Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Use our custom turbo config with syntax errors
  $ . ${TESTDIR}/../../helpers/replace_turbo_config.sh $(pwd) "syntax-error.json"

Run build with invalid env var
  $ EXPERIMENTAL_RUST_CODEPATH=true ${TURBO} build
  ERROR  run failed: Environment variables should not be prefixed with "$"
turbo::config::invalid_env_prefix

  x Environment variables should not be prefixed with "$"
  ,-[6:1]
6 |     "build": {
  7 |       "env": ["NODE_ENV", "$FOOBAR"],
    :                           ^^^^|^^^^
     :                               `-- variable with invalid prefix declared here
   8 |       "outputs": []
     `----

  [1]

