Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

# Save JSON to tmp file so we don't need to keep re-running the build
  $ ${TURBO} run build --dry=json --filter=main > tmpjson.log

  $ cat tmpjson.log | jq .packages
  []

# Save JSON to tmp file so we don't need to keep re-running the build
  $ ${TURBO} run build --experimental-rust-codepath --dry=json --filter=main > tmpjson.log

  $ cat tmpjson.log | jq .packages
  []
