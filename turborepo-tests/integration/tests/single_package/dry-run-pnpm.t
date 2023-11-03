Setup
  $ . ${TESTDIR}/../_helpers/setup.sh single_package pnpm@8.0.0

We only care about this running sucessfully and not the json output
  $ ${TURBO} run build --dry=json > /dev/null
