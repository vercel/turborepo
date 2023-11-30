Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh $(pwd) spaces_failure
  $ . ${TESTDIR}/../../helpers/replace_turbo_config.sh $(pwd) spaces-failure.json

Ensures that even when spaces fails, the build still succeeds.
  $ ${TURBO} run build --token foobarbaz --team bat --api https://example.com > /dev/null 2>&1
