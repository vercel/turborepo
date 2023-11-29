Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/../../helpers/setup_monorepo.sh $(pwd) spaces_failure

Ensures that even when spaces fails, the build still succeeds.
  $ ${TURBO} run build --token foobarbaz --team bat --api https://example.com > /dev/null 2>&1
