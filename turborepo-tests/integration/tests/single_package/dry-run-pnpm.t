Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package pnpm@8.0.0

We only care about this running sucessfully and not the json output
  $ ${TURBO} run build --dry=json > /dev/null
