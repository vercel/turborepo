Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Save JSON to tmp file so we don't need to keep re-running the build
  $ ${TURBO} run build --dry=json --filter=main > tmpjson.log

  $ cat tmpjson.log | jq .packages
  []

# Save JSON to tmp file so we don't need to keep re-running the build
  $ EXPERIMENTAL_RUST_CODEPATH=true ${TURBO} run build --dry=json --filter=main > tmpjson.log

  $ cat tmpjson.log | jq .packages
  []
